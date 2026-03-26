"""
ANR — Agent Network Record
──────────────────────────────────────────────────────────────────────────────
Python reference implementation.

Wire format  : RLP( [sig, seq, k₁, v₁, k₂, v₂, …] )
Signed over  : keccak256( RLP( [b"anr-v1", seq_bytes, k₁, v₁, …] ) )
Text form    : "anr:" + base64url(wire, no padding)
Max size     : 512 bytes

Standard ANR keys
─────────────────
  Network  : id  secp256k1  ip  ip6  tcp  udp
  Agent    : a.id  a.name  a.ver  a.caps  a.tags
             a.proto  a.port  a.tls  a.meta  a.owner  a.chain

Install deps
────────────
  pip install eth-hash[pycryptodome] eth-rlp coincurve
"""

from __future__ import annotations

import struct
import base64
from dataclasses import dataclass, field
from typing import Optional

# ── optional crypto deps (install separately) ─────────────────────────────────
try:
    from eth_hash.auto import keccak
    import coincurve
    _CRYPTO_OK = True
except ImportError:
    _CRYPTO_OK = False
    import warnings
    warnings.warn("coincurve / eth-hash not installed — sign/verify disabled")

# ── RLP (minimal self-contained implementation) ───────────────────────────────

def _rlp_length_prefix(length: int, offset: int) -> bytes:
    if length < 56:
        return bytes([offset + length])
    enc = length.to_bytes((length.bit_length() + 7) // 8, 'big')
    return bytes([offset + 55 + len(enc)]) + enc

def rlp_encode(item) -> bytes:
    if isinstance(item, (bytes, bytearray)):
        b = bytes(item)
        if len(b) == 1 and b[0] < 0x80:
            return b
        return _rlp_length_prefix(len(b), 0x80) + b
    if isinstance(item, list):
        payload = b''.join(rlp_encode(i) for i in item)
        return _rlp_length_prefix(len(payload), 0xC0) + payload
    raise TypeError(f"RLP cannot encode {type(item)}")

def _decode_length(data: bytes, offset: int):
    prefix = data[offset]
    if prefix < 0x80:
        return offset + 1, 1, 'str'
    if prefix <= 0xB7:
        length = prefix - 0x80
        return offset + 1, length, 'str'
    if prefix <= 0xBF:
        ll = prefix - 0xB7
        length = int.from_bytes(data[offset + 1 : offset + 1 + ll], 'big')
        return offset + 1 + ll, length, 'str'
    if prefix <= 0xF7:
        length = prefix - 0xC0
        return offset + 1, length, 'list'
    ll = prefix - 0xF7
    length = int.from_bytes(data[offset + 1 : offset + 1 + ll], 'big')
    return offset + 1 + ll, length, 'list'

def rlp_decode(data: bytes):
    def _decode(data, offset):
        start, length, kind = _decode_length(data, offset)
        if kind == 'str':
            return data[start : start + length], start + length
        items, pos = [], start
        while pos < start + length:
            item, pos = _decode(data, pos)
            items.append(item)
        return items, start + length
    result, _ = _decode(data, 0)
    return result

# ── encoding helpers ──────────────────────────────────────────────────────────

ANR_PREFIX    = "anr:"
ANR_ID_SCHEME = "amp-v1"
ANR_MAX_BYTES = 512
SIGN_DOMAIN   = b"anr-v1"


def _enc_uint16(n: int) -> bytes:
    return struct.pack(">H", n)

def _dec_uint16(b: bytes) -> int:
    return struct.unpack(">H", b)[0]

def _enc_uint64(n: int) -> bytes:
    return struct.pack(">Q", n)

def _dec_uint64(b: bytes) -> int:
    return struct.unpack(">Q", b)[0]

def _sorted_kv(kv: dict[str, bytes]) -> list[tuple[str, bytes]]:
    return sorted(kv.items(), key=lambda x: x[0])

# ── ANR dataclass ─────────────────────────────────────────────────────────────

@dataclass
class ANR:
    seq:       int
    kv:        dict[str, bytes] = field(default_factory=dict)
    signature: bytes = b'\x00' * 64   # 64-byte r‖s

    def encode(self) -> bytes:
        """Encode to binary RLP wire format."""
        pairs = []
        for k, v in _sorted_kv(self.kv):
            pairs.append(k.encode())
            pairs.append(v)
        wire = rlp_encode([self.signature, _enc_uint64(self.seq)] + pairs)
        if len(wire) > ANR_MAX_BYTES:
            raise ValueError(f"ANR exceeds max size: {len(wire)} > {ANR_MAX_BYTES}")
        return wire

    def encode_text(self) -> str:
        """Encode to canonical 'anr:<base64url>' text form."""
        wire = self.encode()
        b64  = base64.urlsafe_b64encode(wire).rstrip(b'=').decode()
        return ANR_PREFIX + b64

    @staticmethod
    def decode(wire: bytes) -> "ANR":
        """Decode from RLP bytes."""
        if len(wire) > ANR_MAX_BYTES:
            raise ValueError(f"ANR too large: {len(wire)}")
        lst = rlp_decode(wire)
        if not isinstance(lst, list) or len(lst) < 2:
            raise ValueError("Invalid ANR structure")
        sig_bytes, seq_bytes, *rest = lst
        kv: dict[str, bytes] = {}
        for i in range(0, len(rest), 2):
            kv[rest[i].decode()] = rest[i + 1]
        return ANR(seq=_dec_uint64(seq_bytes), kv=kv, signature=sig_bytes)

    @staticmethod
    def decode_text(text: str) -> "ANR":
        """Decode from 'anr:<base64url>' text form."""
        if not text.startswith(ANR_PREFIX):
            raise ValueError(f"ANR text must start with '{ANR_PREFIX}'")
        # Re-add padding
        b64_part = text[len(ANR_PREFIX):]
        pad      = 4 - len(b64_part) % 4
        if pad != 4:
            b64_part += '=' * pad
        wire = base64.urlsafe_b64decode(b64_part)
        return ANR.decode(wire)

    def verify(self) -> bool:
        """Verify the record's signature against its stored public key."""
        if not _CRYPTO_OK:
            raise RuntimeError("coincurve / eth-hash not installed")
        pubkey_bytes = self.kv.get('secp256k1')
        if not pubkey_bytes:
            return False
        try:
            content = _content_rlp(self.seq, self.kv)
            digest  = keccak(content)
            pub     = coincurve.PublicKey(pubkey_bytes)
            return pub.verify(self.signature, digest, hasher=None)
        except Exception:
            return False

    def parsed(self) -> "ParsedANR":
        """Decode well-known fields into a typed object."""
        return ParsedANR.from_anr(self)


def _content_rlp(seq: int, kv: dict[str, bytes]) -> bytes:
    pairs = []
    for k, v in _sorted_kv(kv):
        pairs.append(k.encode())
        pairs.append(v)
    return rlp_encode([SIGN_DOMAIN, _enc_uint64(seq)] + pairs)


# ── signing ───────────────────────────────────────────────────────────────────

def sign_anr(private_key_bytes: bytes, seq: int, kv: dict[str, bytes]) -> ANR:
    """Create a signed ANR from a raw secp256k1 private key (32 bytes)."""
    if not _CRYPTO_OK:
        raise RuntimeError("coincurve not installed — pip install coincurve")
    priv    = coincurve.PrivateKey(private_key_bytes)
    pub     = priv.public_key.format(compressed=True)          # 33 bytes
    kv['id']        = ANR_ID_SCHEME.encode()
    kv['secp256k1'] = pub
    content  = _content_rlp(seq, kv)
    digest   = keccak(content)
    sig      = priv.sign(digest, hasher=None)[:64]             # drop recovery byte
    return ANR(seq=seq, kv=kv, signature=sig)


# ── builder ───────────────────────────────────────────────────────────────────

class AnrBuilder:
    """Fluent builder for constructing ANRs without raw byte manipulation."""

    def __init__(self):
        self._seq: int = 0
        self._kv: dict[str, bytes] = {}

    def seq(self, n: int)                -> "AnrBuilder": self._seq = n; return self
    def agent_id(self, v: str)           -> "AnrBuilder": self._kv['a.id']    = v.encode(); return self
    def name(self, v: str)               -> "AnrBuilder": self._kv['a.name']  = v.encode(); return self
    def version(self, v: str)            -> "AnrBuilder": self._kv['a.ver']   = v.encode(); return self
    def capabilities(self, caps: list[str]) -> "AnrBuilder":
        self._kv['a.caps'] = rlp_encode([c.encode() for c in caps]); return self
    def tags(self, tags: list[str])      -> "AnrBuilder":
        self._kv['a.tags'] = rlp_encode([t.encode() for t in tags]); return self
    def proto(self, v: str)              -> "AnrBuilder": self._kv['a.proto'] = v.encode(); return self
    def agent_port(self, port: int)      -> "AnrBuilder": self._kv['a.port']  = _enc_uint16(port); return self
    def tls(self, enabled: bool)         -> "AnrBuilder": self._kv['a.tls']   = bytes([1 if enabled else 0]); return self
    def meta_uri(self, uri: str)         -> "AnrBuilder": self._kv['a.meta']  = uri.encode(); return self
    def owner(self, addr_bytes: bytes)   -> "AnrBuilder": self._kv['a.owner'] = addr_bytes; return self
    def chain_id(self, cid: int)         -> "AnrBuilder": self._kv['a.chain'] = _enc_uint64(cid); return self
    def ipv4(self, b: bytes)             -> "AnrBuilder": self._kv['ip']  = b; return self
    def ipv6(self, b: bytes)             -> "AnrBuilder": self._kv['ip6'] = b; return self
    def tcp_port(self, port: int)        -> "AnrBuilder": self._kv['tcp'] = _enc_uint16(port); return self
    def udp_port(self, port: int)        -> "AnrBuilder": self._kv['udp'] = _enc_uint16(port); return self

    def sign(self, private_key: bytes) -> ANR:
        return sign_anr(private_key, self._seq, dict(self._kv))


# ── parsed view ───────────────────────────────────────────────────────────────

@dataclass
class ParsedANR:
    seq:          int
    id_scheme:    Optional[str]
    pubkey:       Optional[bytes]
    agent_id:     Optional[str]
    name:         Optional[str]
    version:      Optional[str]
    capabilities: list[str]
    tags:         list[str]
    proto:        Optional[str]
    agent_port:   Optional[int]
    tls:          bool
    meta_uri:     Optional[str]
    owner:        Optional[bytes]
    chain_id:     Optional[int]
    ip:           Optional[bytes]
    ip6:          Optional[bytes]
    tcp_port:     Optional[int]
    udp_port:     Optional[int]

    @staticmethod
    def from_anr(r: ANR) -> "ParsedANR":
        def s(k):
            v = r.kv.get(k)
            return v.decode() if v else None

        def decode_list(k):
            raw = r.kv.get(k)
            if not raw:
                return []
            try:
                items = rlp_decode(raw)
                return [i.decode() for i in items]
            except Exception:
                return []

        return ParsedANR(
            seq          = r.seq,
            id_scheme    = s('id'),
            pubkey       = r.kv.get('secp256k1'),
            agent_id     = s('a.id'),
            name         = s('a.name'),
            version      = s('a.ver'),
            capabilities = decode_list('a.caps'),
            tags         = decode_list('a.tags'),
            proto        = s('a.proto'),
            agent_port   = _dec_uint16(r.kv['a.port']) if 'a.port' in r.kv else None,
            tls          = r.kv.get('a.tls', b'\x00')[0] == 1,
            meta_uri     = s('a.meta'),
            owner        = r.kv.get('a.owner'),
            chain_id     = _dec_uint64(r.kv['a.chain']) if 'a.chain' in r.kv else None,
            ip           = r.kv.get('ip'),
            ip6          = r.kv.get('ip6'),
            tcp_port     = _dec_uint16(r.kv['tcp']) if 'tcp' in r.kv else None,
            udp_port     = _dec_uint16(r.kv['udp']) if 'udp' in r.kv else None,
        )


# ── smoke test ────────────────────────────────────────────────────────────────

if __name__ == '__main__':
    import os, json

    priv = os.urandom(32)   # random key — use a fixed key in production

    record = (
        AnrBuilder()
        .seq(1)
        .agent_id('sentrix://agent/0xABC123')
        .name('WeatherAgent')
        .version('1.0.0')
        .capabilities(['getWeather', 'getForecast'])
        .tags(['weather', 'data'])
        .proto('http')
        .agent_port(6174)
        .tls(False)
        .meta_uri('ipfs://QmWeatherMeta')
        .ipv4(bytes([127, 0, 0, 1]))
        .tcp_port(9000)
        .sign(priv)
    )

    text    = record.encode_text()
    decoded = ANR.decode_text(text)
    parsed  = decoded.parsed()

    print("=== ANR text ===")
    print(text)
    print("\n=== Parsed ===")
    print(json.dumps({
        'seq':          parsed.seq,
        'agent_id':     parsed.agent_id,
        'name':         parsed.name,
        'version':      parsed.version,
        'capabilities': parsed.capabilities,
        'tags':         parsed.tags,
        'proto':        parsed.proto,
        'agent_port':   parsed.agent_port,
        'tls':          parsed.tls,
    }, indent=2))

    if _CRYPTO_OK:
        valid = decoded.verify()
        print(f"\n✔ Signature valid: {valid}")
        assert valid

    print("\n✔ Round-trip OK")
    assert parsed.name == 'WeatherAgent'
    assert 'getWeather' in parsed.capabilities
