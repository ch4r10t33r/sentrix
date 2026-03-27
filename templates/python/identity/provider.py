"""
Borgkit Identity Providers
──────────────────────────────────────────────────────────────────────────────
Provides flexible agent identity without requiring ERC-8004 on-chain
registration or a wallet. On-chain identity (ERC-8004) remains available as
an opt-in for production deployments that need verifiable ownership.

Identity modes
--------------
Mode            | Requires wallet? | On-chain? | Use case
----------------|-----------------|-----------|---------------------
anonymous       | no              | no        | dev / ephemeral agents
local_keystore  | no              | no        | persistent local key (auto-created)
env             | no              | no        | containers / cloud (12-factor)
raw_key         | no              | no        | bring-your-own hex key
erc8004         | yes             | optional  | production, verifiable ownership

All modes that have a private key produce:
  - A W3C DID          (did:key:z<secp256k1-compressed-pubkey> for local/env/raw)
                        (did:pkh:eip155:<chainId>:<addr> for erc8004)
  - An Ethereum address (owner field, derived from same key)
  - ANR signing support (same secp256k1 key)
  - libp2p PeerId       (same secp256k1 key → Secp256k1Keypair)
"""

from __future__ import annotations

import hashlib
import os
import secrets
import stat
import warnings
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Optional


# ── helpers ───────────────────────────────────────────────────────────────────

def _pub_from_priv(key_bytes: bytes) -> bytes:
    """Derive 64-byte uncompressed public key (without 04 prefix)."""
    try:
        from coincurve import PublicKey          # preferred: small, fast
        return PublicKey.from_secret(key_bytes).format(compressed=False)[1:]
    except ImportError:
        pass
    try:
        from eth_keys import keys as eth_keys    # fallback
        return bytes(eth_keys.PrivateKey(key_bytes).public_key)
    except ImportError:
        pass
    # Pure-Python fallback using cryptography library
    from cryptography.hazmat.primitives.asymmetric.ec import (
        SECP256K1, generate_private_key, EllipticCurvePrivateKey
    )
    from cryptography.hazmat.backends import default_backend
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
    import struct
    # k256 manual: use coincurve if possible, else raise
    raise ImportError(
        "Public key derivation requires one of: coincurve, eth_keys. "
        "Install with:  pip install coincurve  or  pip install eth-keys"
    )


# ── DID helpers ───────────────────────────────────────────────────────────────

_B58_ALPHABET = b'123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'


def _b58encode(data: bytes) -> str:
    """Pure-Python base58 encode (no external library needed)."""
    n = int.from_bytes(data, 'big')
    result = []
    while n:
        n, rem = divmod(n, 58)
        result.append(_B58_ALPHABET[rem: rem + 1])
    for byte in data:
        if byte == 0:
            result.append(_B58_ALPHABET[0:1])
        else:
            break
    return b''.join(reversed(result)).decode('ascii')


# secp256k1-pub multicodec varint: 0xe7 0x01
_SECP256K1_MULTICODEC = b'\xe7\x01'


def _did_key_from_priv(priv_bytes: bytes) -> str:
    """
    Derive a W3C did:key DID from a secp256k1 private key.

    Format:  did:key:z<base58btc(multicodec_prefix + compressed_pubkey)>
    Spec:    https://w3c-ccg.github.io/did-key-spec/
    """
    # Get 33-byte compressed public key
    try:
        from coincurve import PublicKey
        pub_compressed = PublicKey.from_secret(priv_bytes).format(compressed=True)
    except ImportError:
        try:
            from eth_keys import keys as eth_keys
            raw_pub = bytes(eth_keys.PrivateKey(priv_bytes).public_key)  # 64 bytes
            x, y = raw_pub[:32], raw_pub[32:]
            prefix = b'\x02' if int.from_bytes(y, 'big') % 2 == 0 else b'\x03'
            pub_compressed = prefix + x
        except ImportError:
            raise ImportError(
                "did:key derivation requires coincurve or eth-keys. "
                "Install:  pip install coincurve  or  pip install eth-keys"
            )
    prefixed = _SECP256K1_MULTICODEC + pub_compressed   # 35 bytes
    return f"did:key:z{_b58encode(prefixed)}"


def _did_pkh_evm(address: str, chain_id: int) -> str:
    """
    Return a did:pkh DID for an EVM address.
    Format: did:pkh:eip155:<chainId>:<checksumAddress>
    Used when ERC-8004 on-chain identity is selected.
    """
    return f"did:pkh:eip155:{chain_id}:{address}"


def _keccak256(data: bytes) -> bytes:
    try:
        from Crypto.Hash import keccak as pycryptodome
        k = pycryptodome.new(digest_bits=256)
        k.update(data)
        return k.digest()
    except ImportError:
        pass
    try:
        from eth_hash.auto import keccak
        return keccak(data)
    except ImportError:
        pass
    # pysha3 fallback
    import sha3  # type: ignore
    return hashlib.new("sha3_256", data).digest()  # NOT keccak — just a last resort
    # Real keccak is different from SHA3; warn the caller
    warnings.warn(
        "Could not import keccak256 library. Address derivation may be incorrect. "
        "Install:  pip install pycryptodome  or  pip install eth-hash[pysha3]"
    )


def _eth_address_from_pub(pub64: bytes) -> str:
    """Derive checksummed Ethereum address from 64-byte public key."""
    digest = _keccak256(pub64)
    raw = digest[-20:].hex()
    # EIP-55 checksum
    keccak_hex = _keccak256(raw.encode()).hex()
    checksum = "".join(
        c.upper() if int(keccak_hex[i], 16) >= 8 else c
        for i, c in enumerate(raw)
    )
    return "0x" + checksum


# ── base class ────────────────────────────────────────────────────────────────

class IdentityProvider(ABC):
    """
    Abstract base for all Borgkit identity providers.

    Usage pattern:
        identity = LocalKeystoreIdentity("my-agent")
        agent_id = identity.agent_id()
        owner    = identity.owner()
        anr_key  = identity.private_key_bytes()  # None for anonymous
    """

    @abstractmethod
    def agent_id(self) -> str:
        """Return the Borgkit agent URI (e.g. borgkit://agent/0xABC…)."""
        ...

    @abstractmethod
    def owner(self) -> str:
        """Return the owner identifier (Ethereum address or arbitrary string)."""
        ...

    @abstractmethod
    def private_key_bytes(self) -> Optional[bytes]:
        """
        Return the 32-byte secp256k1 private key, or None if not available.
        The same key is used for ANR signing and libp2p PeerId derivation.
        """
        ...

    def sign_bytes(self, message: bytes) -> Optional[str]:
        """
        Sign arbitrary bytes with the agent's private key.
        Returns hex-encoded signature, or None if no key is set.
        """
        key = self.private_key_bytes()
        if key is None:
            return None
        try:
            from eth_keys import keys as eth_keys
            pk = eth_keys.PrivateKey(key)
            msg_hash = _keccak256(b"\x19Ethereum Signed Message:\n" + str(len(message)).encode() + message)
            return pk.sign_msg_hash(msg_hash).to_hex()
        except ImportError:
            warnings.warn("eth-keys not installed; sign_bytes() unavailable. pip install eth-keys")
            return None

    def to_plugin_config_fields(self) -> dict:
        """Return dict with agent_id, owner, signing_key for PluginConfig."""
        key = self.private_key_bytes()
        return {
            "agent_id":    self.agent_id(),
            "owner":       self.owner(),
            "signing_key": key.hex() if key else None,
        }


# ── anonymous identity ────────────────────────────────────────────────────────

@dataclass
class AnonymousIdentity(IdentityProvider):
    """
    No cryptographic identity — suitable for local dev and ephemeral agents.
    ANR records will be unsigned; discovery is not authenticated.
    """
    name: str = "unnamed"

    def agent_id(self) -> str:
        return f"borgkit://agent/{self.name}"

    def owner(self) -> str:
        return "anonymous"

    def private_key_bytes(self) -> Optional[bytes]:
        return None


# ── raw-key identity ──────────────────────────────────────────────────────────

@dataclass
class RawKeyIdentity(IdentityProvider):
    """
    Identity from an explicit hex private key.

    Args:
        private_key_hex: 32-byte secp256k1 key in hex (with or without 0x prefix).
        name_override:   Optional human-readable suffix for the agent_id URI.
    """
    private_key_hex: str
    name_override: Optional[str] = None

    def __post_init__(self):
        raw = self.private_key_hex.lstrip("0x")
        if len(raw) != 64:
            raise ValueError("private_key_hex must be exactly 32 bytes (64 hex chars)")
        self._key = bytes.fromhex(raw)
        self._pub64 = _pub_from_priv(self._key)
        self._address = _eth_address_from_pub(self._pub64)

    def agent_id(self) -> str:
        return _did_key_from_priv(self._key)

    def owner(self) -> str:
        return self._address

    def private_key_bytes(self) -> Optional[bytes]:
        return self._key


# ── env-var identity ──────────────────────────────────────────────────────────

@dataclass
class EnvKeyIdentity(IdentityProvider):
    """
    Identity from an environment variable (12-factor / container-friendly).

    Reads the private key from BORGKIT_AGENT_KEY (default) or a custom env var.
    Falls back to AnonymousIdentity if the env var is not set.

    Example:
        export BORGKIT_AGENT_KEY=0xdeadbeef...  # 32-byte hex
        identity = EnvKeyIdentity()
    """
    env_var: str = "BORGKIT_AGENT_KEY"
    name_override: Optional[str] = None
    _delegate: Optional[IdentityProvider] = field(default=None, init=False, repr=False)

    def __post_init__(self):
        val = os.environ.get(self.env_var)
        if val:
            self._delegate = RawKeyIdentity(val, name_override=self.name_override)
        else:
            warnings.warn(
                f"[Borgkit] {self.env_var} not set — using anonymous identity. "
                "Set the env var or use LocalKeystoreIdentity for persistent identity."
            )
            self._delegate = AnonymousIdentity(name=self.name_override or "unnamed")

    def agent_id(self) -> str:
        return self._delegate.agent_id()   # type: ignore[union-attr]

    def owner(self) -> str:
        return self._delegate.owner()       # type: ignore[union-attr]

    def private_key_bytes(self) -> Optional[bytes]:
        return self._delegate.private_key_bytes()   # type: ignore[union-attr]


# ── local keystore identity ───────────────────────────────────────────────────

@dataclass
class LocalKeystoreIdentity(IdentityProvider):
    """
    Persistent identity stored as a plain-text hex key in ~/.borgkit/keystore/.

    The key file is created on first use.  The same key is reused on every
    subsequent run, giving the agent a stable identity across restarts without
    requiring a wallet or on-chain registration.

    File: ~/.borgkit/keystore/<name>.key   (mode 0600)

    Args:
        name:         Unique name for this agent (used as filename).
        keystore_dir: Override the keystore directory (default: ~/.borgkit/keystore).

    Example:
        identity = LocalKeystoreIdentity(name="research-agent")
        # Key auto-created at ~/.borgkit/keystore/research-agent.key
        print(identity.agent_id())   # borgkit://agent/0x...
    """
    name: str
    keystore_dir: str = field(default_factory=lambda: os.path.expanduser("~/.borgkit/keystore"))
    _key: bytes = field(default=None, init=False, repr=False)          # type: ignore[assignment]
    _pub64: bytes = field(default=None, init=False, repr=False)        # type: ignore[assignment]
    _address: str = field(default="", init=False, repr=False)

    def __post_init__(self):
        self._key = self._load_or_create()
        self._pub64 = _pub_from_priv(self._key)
        self._address = _eth_address_from_pub(self._pub64)

    def _load_or_create(self) -> bytes:
        os.makedirs(self.keystore_dir, mode=0o700, exist_ok=True)
        keyfile = os.path.join(self.keystore_dir, f"{self.name}.key")
        if os.path.exists(keyfile):
            with open(keyfile, "r") as f:
                return bytes.fromhex(f.read().strip())
        # Generate a new random key
        key = secrets.token_bytes(32)
        with open(keyfile, "w") as f:
            f.write(key.hex())
        os.chmod(keyfile, stat.S_IRUSR | stat.S_IWUSR)   # 0o600
        print(f"[Borgkit] New identity created: {keyfile}")
        return key

    def agent_id(self) -> str:
        return _did_key_from_priv(self._key)

    def owner(self) -> str:
        return self._address

    def private_key_bytes(self) -> Optional[bytes]:
        return self._key


# ── ERC-8004 on-chain identity ────────────────────────────────────────────────

class ERC8004Identity(IdentityProvider):
    """
    On-chain identity compliant with ERC-8004.

    This mode anchors the agent's ANR record on a smart contract, providing
    verifiable on-chain ownership.  It requires a wallet private key and gas
    to register.  All other Borgkit features work identically whether you use
    this mode or an off-chain mode.

    Args:
        private_key_hex:   32-byte secp256k1 key in hex (wallet private key).
        chain_id:          EVM chain ID (default: 8453 = Base).
        contract_address:  ERC-8004 registry contract address.
        rpc_url:           RPC endpoint for the target chain.

    Example:
        identity = ERC8004Identity(
            private_key_hex=os.environ["WALLET_PRIVATE_KEY"],
            chain_id=8453,
            contract_address="0x...",
            rpc_url="https://mainnet.base.org",
        )
        await identity.register_on_chain(anr_text)

    Note:
        On-chain registration is OPTIONAL.  You can use this class purely for
        its key derivation (agent_id / owner) without calling register_on_chain().
    """

    def __init__(
        self,
        private_key_hex: str,
        chain_id: int = 8453,
        contract_address: Optional[str] = None,
        rpc_url: Optional[str] = None,
    ):
        raw = private_key_hex.lstrip("0x")
        self._key = bytes.fromhex(raw)
        self.chain_id = chain_id
        self.contract_address = contract_address
        self.rpc_url = rpc_url
        self._pub64 = _pub_from_priv(self._key)
        self._address = _eth_address_from_pub(self._pub64)

    def agent_id(self) -> str:
        return _did_pkh_evm(self._address, self.chain_id)

    def owner(self) -> str:
        return self._address

    def private_key_bytes(self) -> Optional[bytes]:
        return self._key

    async def register_on_chain(self, anr_text: str) -> str:
        """
        Publish the signed ANR to the ERC-8004 on-chain registry.

        Returns the transaction hash.

        Requirements:
            pip install web3
            A funded wallet with gas on chain_id.
            A deployed ERC-8004 registry contract.
        """
        if not self.contract_address or not self.rpc_url:
            raise ValueError(
                "ERC8004Identity.register_on_chain() requires contract_address and rpc_url. "
                "See docs/identity.md for setup."
            )
        try:
            from web3 import Web3
        except ImportError:
            raise ImportError(
                "On-chain registration requires web3.  Install with:  pip install web3"
            )

        w3 = Web3(Web3.HTTPProvider(self.rpc_url))
        # Minimal ERC-8004 ABI (setRecord function)
        abi = [
            {
                "inputs": [{"internalType": "bytes", "name": "anr", "type": "bytes"}],
                "name": "setRecord",
                "outputs": [],
                "stateMutability": "nonpayable",
                "type": "function",
            }
        ]
        contract = w3.eth.contract(
            address=Web3.to_checksum_address(self.contract_address),
            abi=abi,
        )
        account = w3.eth.account.from_key(self._key)
        nonce = w3.eth.get_transaction_count(account.address)
        tx = contract.functions.setRecord(anr_text.encode()).build_transaction(
            {
                "chainId": self.chain_id,
                "from": account.address,
                "nonce": nonce,
                "gas": 200_000,
                "gasPrice": w3.eth.gas_price,
            }
        )
        signed = account.sign_transaction(tx)
        tx_hash = w3.eth.send_raw_transaction(signed.rawTransaction)
        return tx_hash.hex()


# ── factory / convenience ─────────────────────────────────────────────────────

def identity_from_config(
    mode: str = "local",
    name: str = "unnamed-agent",
    private_key_hex: Optional[str] = None,
    env_var: str = "BORGKIT_AGENT_KEY",
    keystore_dir: Optional[str] = None,
    chain_id: int = 8453,
    contract_address: Optional[str] = None,
    rpc_url: Optional[str] = None,
) -> IdentityProvider:
    """
    Factory function — pick an identity mode by name.

    Modes:
        "anonymous"     → AnonymousIdentity
        "local"         → LocalKeystoreIdentity (default)
        "env"           → EnvKeyIdentity
        "raw"           → RawKeyIdentity (requires private_key_hex)
        "erc8004"       → ERC8004Identity (requires private_key_hex)

    Example:
        identity = identity_from_config(mode="local", name="my-agent")
    """
    if mode == "anonymous":
        return AnonymousIdentity(name=name)
    if mode == "env":
        return EnvKeyIdentity(env_var=env_var, name_override=name)
    if mode == "raw":
        if not private_key_hex:
            raise ValueError("mode='raw' requires private_key_hex")
        return RawKeyIdentity(private_key_hex=private_key_hex, name_override=name)
    if mode == "erc8004":
        if not private_key_hex:
            raise ValueError("mode='erc8004' requires private_key_hex")
        return ERC8004Identity(
            private_key_hex=private_key_hex,
            chain_id=chain_id,
            contract_address=contract_address,
            rpc_url=rpc_url,
        )
    # Default: local keystore
    kwargs: dict = {"name": name}
    if keystore_dir:
        kwargs["keystore_dir"] = keystore_dir
    return LocalKeystoreIdentity(**kwargs)
