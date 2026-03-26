/*!
ANR — Agent Network Record
──────────────────────────────────────────────────────────────────────────────
Rust reference implementation.

Wire format  : RLP( [sig, seq, k₁, v₁, k₂, v₂, …] )
Signed over  : keccak256( RLP( [b"anr-v1", seq_bytes, k₁, v₁, …] ) )
Text form    : "anr:" + base64url(wire, no padding)
Max size     : 512 bytes

Key ordering : lexicographic, unique
*/

pub mod rlp;

use crate::anr::rlp::{rlp_encode, rlp_decode, RlpItem};

use std::collections::BTreeMap;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use k256::ecdsa::{SigningKey, VerifyingKey, Signature, signature::Signer, signature::Verifier};
use sha3::{Digest, Keccak256};
use thiserror::Error;

// ── constants ─────────────────────────────────────────────────────────────────

pub const ANR_PREFIX:    &str  = "anr:";
pub const ANR_ID_SCHEME: &str  = "amp-v1";
pub const ANR_MAX_BYTES: usize = 512;
const     SIGN_DOMAIN:   &[u8] = b"anr-v1";

// ── error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum AnrError {
    #[error("ANR too large: {0} bytes (max {ANR_MAX_BYTES})")]
    TooLarge(usize),
    #[error("Invalid ANR structure: {0}")]
    InvalidStructure(String),
    #[error("Missing prefix 'anr:'")]
    MissingPrefix,
    #[error("Base64 decode failed: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("Crypto error: {0}")]
    Crypto(String),
}

// ── ANR record ────────────────────────────────────────────────────────────────

/// A decoded ANR record. Keys are stored in a BTreeMap so they remain
/// lexicographically ordered for deterministic encoding.
#[derive(Debug, Clone)]
pub struct AnrRecord {
    /// Sequence number — increment on every update
    pub seq: u64,
    /// Key-value pairs (BTreeMap keeps them sorted)
    pub kv: BTreeMap<String, Vec<u8>>,
    /// 64-byte compact secp256k1 signature (r‖s)
    pub signature: [u8; 64],
}

impl AnrRecord {
    // ── encoding ──────────────────────────────────────────────────────────────

    /// Encode to binary RLP wire format.
    pub fn encode(&self) -> Result<Vec<u8>, AnrError> {
        let mut items = vec![
            RlpItem::Bytes(self.signature.to_vec()),
            RlpItem::Bytes(self.seq.to_be_bytes().to_vec()),
        ];
        for (k, v) in &self.kv {
            items.push(RlpItem::Bytes(k.as_bytes().to_vec()));
            items.push(RlpItem::Bytes(v.clone()));
        }
        let wire = rlp_encode(&RlpItem::List(items));
        if wire.len() > ANR_MAX_BYTES {
            return Err(AnrError::TooLarge(wire.len()));
        }
        Ok(wire)
    }

    /// Encode to canonical "anr:<base64url>" text form.
    pub fn encode_text(&self) -> Result<String, AnrError> {
        let wire = self.encode()?;
        Ok(format!("{}{}", ANR_PREFIX, URL_SAFE_NO_PAD.encode(&wire)))
    }

    // ── decoding ──────────────────────────────────────────────────────────────

    /// Decode from raw RLP bytes.
    pub fn decode(wire: &[u8]) -> Result<Self, AnrError> {
        if wire.len() > ANR_MAX_BYTES {
            return Err(AnrError::TooLarge(wire.len()));
        }
        let items = match rlp_decode(wire)
            .map_err(|e| AnrError::InvalidStructure(e.to_string()))?
        {
            RlpItem::List(l) => l,
            _ => return Err(AnrError::InvalidStructure("expected list".into())),
        };

        if items.len() < 2 {
            return Err(AnrError::InvalidStructure("too few items".into()));
        }

        let sig_bytes = match &items[0] {
            RlpItem::Bytes(b) if b.len() == 64 => {
                let mut arr = [0u8; 64];
                arr.copy_from_slice(b);
                arr
            }
            _ => return Err(AnrError::InvalidStructure("bad signature".into())),
        };

        let seq = match &items[1] {
            RlpItem::Bytes(b) => u64::from_be_bytes(b.as_slice().try_into()
                .map_err(|_| AnrError::InvalidStructure("bad seq".into()))?),
            _ => return Err(AnrError::InvalidStructure("bad seq type".into())),
        };

        let rest = &items[2..];
        if rest.len() % 2 != 0 {
            return Err(AnrError::InvalidStructure("odd kv count".into()));
        }

        let mut kv = BTreeMap::new();
        for chunk in rest.chunks_exact(2) {
            let k = match &chunk[0] {
                RlpItem::Bytes(b) => String::from_utf8(b.clone())
                    .map_err(|e| AnrError::InvalidStructure(e.to_string()))?,
                _ => return Err(AnrError::InvalidStructure("key not bytes".into())),
            };
            let v = match &chunk[1] {
                RlpItem::Bytes(b) => b.clone(),
                _ => return Err(AnrError::InvalidStructure("value not bytes".into())),
            };
            kv.insert(k, v);
        }

        Ok(AnrRecord { seq, kv, signature: sig_bytes })
    }

    /// Decode from "anr:<base64url>" text form.
    pub fn decode_text(text: &str) -> Result<Self, AnrError> {
        let b64 = text.strip_prefix(ANR_PREFIX).ok_or(AnrError::MissingPrefix)?;
        let wire = URL_SAFE_NO_PAD.decode(b64)?;
        Self::decode(&wire)
    }

    // ── verify ────────────────────────────────────────────────────────────────

    /// Verify the record's signature against its stored public key.
    pub fn verify(&self) -> bool {
        let Some(pubkey_bytes) = self.kv.get("secp256k1") else { return false; };
        let Ok(vk) = VerifyingKey::from_sec1_bytes(pubkey_bytes) else { return false; };
        let Ok(sig) = Signature::from_slice(&self.signature) else { return false; };
        let content = self.content_rlp();
        let hash = Keccak256::digest(&content);
        vk.verify(&hash, &sig).is_ok()
    }

    fn content_rlp(&self) -> Vec<u8> {
        let mut items = vec![
            RlpItem::Bytes(SIGN_DOMAIN.to_vec()),
            RlpItem::Bytes(self.seq.to_be_bytes().to_vec()),
        ];
        for (k, v) in &self.kv {
            items.push(RlpItem::Bytes(k.as_bytes().to_vec()));
            items.push(RlpItem::Bytes(v.clone()));
        }
        rlp_encode(&RlpItem::List(items))
    }

    /// Parse well-known fields into a typed struct.
    pub fn parsed(&self) -> ParsedAnr {
        ParsedAnr::from_record(self)
    }
}

// ── sign ──────────────────────────────────────────────────────────────────────

/// Create a signed ANR record from a secp256k1 signing key.
pub fn sign_anr(key: &SigningKey, seq: u64, kv: BTreeMap<String, Vec<u8>>) -> Result<AnrRecord, AnrError> {
    let mut kv = kv;
    let pubkey = VerifyingKey::from(key).to_sec1_bytes().to_vec(); // 33-byte compressed

    kv.insert("id".into(),        ANR_ID_SCHEME.as_bytes().to_vec());
    kv.insert("secp256k1".into(), pubkey);

    let mut record = AnrRecord { seq, kv, signature: [0u8; 64] };

    let content  = record.content_rlp();
    let hash     = Keccak256::digest(&content);
    let sig: Signature = key.sign(&hash);
    record.signature.copy_from_slice(&sig.to_bytes());

    Ok(record)
}

// ── builder ───────────────────────────────────────────────────────────────────

/// Fluent builder for ANR records.
#[derive(Default)]
pub struct AnrBuilder {
    seq: u64,
    kv:  BTreeMap<String, Vec<u8>>,
}

impl AnrBuilder {
    pub fn new() -> Self { Self::default() }

    pub fn seq(mut self, n: u64)             -> Self { self.seq = n; self }

    // Agent fields
    pub fn agent_id(mut self, v: &str)       -> Self { self.kv.insert("a.id".into(),    v.as_bytes().to_vec()); self }
    pub fn name(mut self, v: &str)           -> Self { self.kv.insert("a.name".into(),  v.as_bytes().to_vec()); self }
    pub fn version(mut self, v: &str)        -> Self { self.kv.insert("a.ver".into(),   v.as_bytes().to_vec()); self }
    pub fn capabilities(mut self, caps: &[&str]) -> Self {
        let items: Vec<RlpItem> = caps.iter().map(|c| RlpItem::Bytes(c.as_bytes().to_vec())).collect();
        self.kv.insert("a.caps".into(), rlp_encode(&RlpItem::List(items)));
        self
    }
    pub fn tags(mut self, tags: &[&str])     -> Self {
        let items: Vec<RlpItem> = tags.iter().map(|t| RlpItem::Bytes(t.as_bytes().to_vec())).collect();
        self.kv.insert("a.tags".into(), rlp_encode(&RlpItem::List(items)));
        self
    }
    pub fn proto(mut self, v: &str)          -> Self { self.kv.insert("a.proto".into(), v.as_bytes().to_vec()); self }
    pub fn agent_port(mut self, port: u16)   -> Self { self.kv.insert("a.port".into(),  port.to_be_bytes().to_vec()); self }
    pub fn tls(mut self, on: bool)           -> Self { self.kv.insert("a.tls".into(),   vec![on as u8]); self }
    pub fn meta_uri(mut self, uri: &str)     -> Self { self.kv.insert("a.meta".into(),  uri.as_bytes().to_vec()); self }
    pub fn owner(mut self, addr: &[u8])      -> Self { self.kv.insert("a.owner".into(), addr.to_vec()); self }
    pub fn chain_id(mut self, id: u64)       -> Self { self.kv.insert("a.chain".into(), id.to_be_bytes().to_vec()); self }

    // Network fields
    pub fn ipv4(mut self, b: [u8; 4])        -> Self { self.kv.insert("ip".into(),  b.to_vec()); self }
    pub fn ipv6(mut self, b: [u8; 16])       -> Self { self.kv.insert("ip6".into(), b.to_vec()); self }
    pub fn tcp_port(mut self, port: u16)     -> Self { self.kv.insert("tcp".into(), port.to_be_bytes().to_vec()); self }
    pub fn udp_port(mut self, port: u16)     -> Self { self.kv.insert("udp".into(), port.to_be_bytes().to_vec()); self }

    /// Sign and produce the final ANR record.
    pub fn sign(self, key: &SigningKey) -> Result<AnrRecord, AnrError> {
        sign_anr(key, self.seq, self.kv)
    }
}

// ── parsed view ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ParsedAnr {
    pub seq:          u64,
    pub id_scheme:    Option<String>,
    pub pubkey:       Option<Vec<u8>>,
    pub agent_id:     Option<String>,
    pub name:         Option<String>,
    pub version:      Option<String>,
    pub capabilities: Vec<String>,
    pub tags:         Vec<String>,
    pub proto:        Option<String>,
    pub agent_port:   Option<u16>,
    pub tls:          bool,
    pub meta_uri:     Option<String>,
    pub owner:        Option<Vec<u8>>,
    pub chain_id:     Option<u64>,
    pub ip:           Option<[u8; 4]>,
    pub ip6:          Option<[u8; 16]>,
    pub tcp_port:     Option<u16>,
    pub udp_port:     Option<u16>,
}

impl ParsedAnr {
    fn from_record(r: &AnrRecord) -> Self {
        let str_val = |k: &str| -> Option<String> {
            r.kv.get(k).and_then(|b| String::from_utf8(b.clone()).ok())
        };
        let u16_val = |k: &str| -> Option<u16> {
            r.kv.get(k).and_then(|b| b.as_slice().try_into().ok().map(u16::from_be_bytes))
        };
        let decode_list = |k: &str| -> Vec<String> {
            r.kv.get(k).and_then(|raw| {
                if let Ok(RlpItem::List(items)) = rlp_decode(raw) {
                    Some(items.iter().filter_map(|i| match i {
                        RlpItem::Bytes(b) => String::from_utf8(b.clone()).ok(),
                        _ => None,
                    }).collect())
                } else { None }
            }).unwrap_or_default()
        };

        ParsedAnr {
            seq:          r.seq,
            id_scheme:    str_val("id"),
            pubkey:       r.kv.get("secp256k1").cloned(),
            agent_id:     str_val("a.id"),
            name:         str_val("a.name"),
            version:      str_val("a.ver"),
            capabilities: decode_list("a.caps"),
            tags:         decode_list("a.tags"),
            proto:        str_val("a.proto"),
            agent_port:   u16_val("a.port"),
            tls:          r.kv.get("a.tls").map_or(false, |b| b.first() == Some(&1)),
            meta_uri:     str_val("a.meta"),
            owner:        r.kv.get("a.owner").cloned(),
            chain_id:     r.kv.get("a.chain").and_then(|b| b.as_slice().try_into().ok().map(u64::from_be_bytes)),
            ip:           r.kv.get("ip").and_then(|b| b.as_slice().try_into().ok()),
            ip6:          r.kv.get("ip6").and_then(|b| b.as_slice().try_into().ok()),
            tcp_port:     u16_val("tcp"),
            udp_port:     u16_val("udp"),
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;
    use rand_core::OsRng;

    #[test]
    fn round_trip_and_verify() {
        let key = SigningKey::random(&mut OsRng);

        let record = AnrBuilder::new()
            .seq(1)
            .agent_id("sentrix://agent/0xABC123")
            .name("WeatherAgent")
            .version("1.0.0")
            .capabilities(&["getWeather", "getForecast"])
            .tags(&["weather", "data"])
            .proto("http")
            .agent_port(6174)
            .tls(false)
            .meta_uri("ipfs://QmWeatherMeta")
            .ipv4([127, 0, 0, 1])
            .tcp_port(9000)
            .sign(&key)
            .expect("signing failed");

        let text    = record.encode_text().expect("encode_text failed");
        assert!(text.starts_with("anr:"), "should start with anr:");

        let decoded = AnrRecord::decode_text(&text).expect("decode failed");
        assert!(decoded.verify(), "signature should be valid");

        let parsed = decoded.parsed();
        assert_eq!(parsed.name.as_deref(), Some("WeatherAgent"));
        assert!(parsed.capabilities.contains(&"getWeather".to_string()));
        assert_eq!(parsed.agent_port, Some(8080));
    }

    #[test]
    fn tamper_detection() {
        let key = SigningKey::random(&mut OsRng);
        let mut record = AnrBuilder::new()
            .seq(1).name("Test").sign(&key).unwrap();
        assert!(record.verify());
        record.kv.insert("a.name".into(), b"TAMPERED".to_vec());
        assert!(!record.verify(), "tampered record should fail verification");
    }
}
