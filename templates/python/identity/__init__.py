"""
Borgkit Identity Providers
──────────────────────────
Flexible identity options — ERC-8004 on-chain registration is optional.

Quick start (no wallet required):
    from identity.provider import LocalKeystoreIdentity
    identity = LocalKeystoreIdentity(name="my-agent")   # auto-creates key in ~/.borgkit/keystore/

With environment variable:
    from identity.provider import EnvKeyIdentity
    identity = EnvKeyIdentity()   # reads BORGKIT_AGENT_KEY

On-chain (optional, requires wallet + gas):
    from identity.provider import ERC8004Identity
    identity = ERC8004Identity(private_key_hex="0x...")
"""

from .provider import (
    IdentityProvider,
    AnonymousIdentity,
    LocalKeystoreIdentity,
    EnvKeyIdentity,
    RawKeyIdentity,
    ERC8004Identity,
    identity_from_config,
)

__all__ = [
    "IdentityProvider",
    "AnonymousIdentity",
    "LocalKeystoreIdentity",
    "EnvKeyIdentity",
    "RawKeyIdentity",
    "ERC8004Identity",
    "identity_from_config",
]
