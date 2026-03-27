//! DIDComm v2 encrypted messaging for Borgkit agents.
//!
//! Implements authenticated encryption (authcrypt) and anonymous encryption
//! (anoncrypt) between DID-identified agents using X25519 ECDH +
//! ChaCha20-Poly1305 AEAD.
//!
//! # Crypto stack
//! - Key agreement : X25519 ([`x25519_dalek`])
//! - AEAD          : ChaCha20-Poly1305 ([`chacha20poly1305`])
//! - Encoding      : base64url (no-pad), base58btc (custom, Bitcoin alphabet)
//! - DID method    : `did:key` (multicodec x25519-pub prefix `0xec 0x01`)
//!
//! # Cargo.toml dependencies
//! ```toml
//! x25519-dalek      = { version = "2", features = ["static_secrets"] }
//! chacha20poly1305  = "0.10"
//! rand              = "0.8"
//! base64            = { version = "0.22", features = ["alloc"] }
//! uuid              = { version = "1", features = ["v4"] }
//! serde             = { version = "1", features = ["derive"] }
//! serde_json        = "1"
//! bs58              = "0.5"
//! ```
//!
//! # Usage example
//! ```rust
//! use crate::didcomm::{DidcommClient, MSG_INVOKE};
//! use serde_json::json;
//!
//! let alice = DidcommClient::generate().unwrap();
//! let bob   = DidcommClient::generate().unwrap();
//!
//! // Alice encrypts an invoke message for Bob (authcrypt)
//! let encrypted = alice.invoke(&bob.did, "translate", json!({"text": "hello"}), false).unwrap();
//!
//! // Bob decrypts
//! let (msg, sender_did) = bob.unpack(&encrypted).unwrap();
//! println!("{}", msg.body);       // {"capability":"translate","input":{"text":"hello"}}
//! println!("{:?}", sender_did);   // Some("did:key:z6Mk...")
//! ```

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng as AeadOsRng},
    ChaCha20Poly1305, Key, Nonce,
};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use x25519_dalek::{PublicKey, StaticSecret};

// ── Message type URIs ─────────────────────────────────────────────────────────

pub const MSG_INVOKE:   &str = "https://borgkit.dev/didcomm/1.0/invoke";
pub const MSG_RESPONSE: &str = "https://borgkit.dev/didcomm/1.0/response";
pub const MSG_FORWARD:  &str = "https://borgkit.dev/didcomm/1.0/forward";
pub const MSG_PING:     &str = "https://borgkit.dev/didcomm/1.0/ping";
pub const MSG_PONG:     &str = "https://borgkit.dev/didcomm/1.0/pong";

// ── Types ─────────────────────────────────────────────────────────────────────

/// A plaintext DIDComm v2 message (JWM body).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DidcommMessage {
    /// UUID v4 message identifier.
    pub id: String,
    /// Message type URI, e.g. [`MSG_INVOKE`].
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Sender DID — absent for anoncrypt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Recipient DIDs.
    pub to: Vec<String>,
    /// Unix timestamp (seconds) when the message was created.
    pub created_time: u64,
    /// Optional expiry timestamp (Unix seconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_time: Option<u64>,
    /// Application-level body payload.
    pub body: serde_json::Value,
    /// Optional attachments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Attachment>>,
}

/// A DIDComm v2 attachment descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub data: AttachmentData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

/// Data for a DIDComm attachment — either base64-encoded bytes or inline JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<serde_json::Value>,
}

/// JWE JSON-serialization envelope for an encrypted DIDComm message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedMessage {
    /// Base64url-encoded ChaCha20-Poly1305 ciphertext.
    pub ciphertext: String,
    /// Base64url-encoded protected header JSON.
    pub protected: String,
    /// Per-recipient encrypted content keys.
    pub recipients: Vec<RecipientHeader>,
    /// Base64url-encoded 12-byte nonce.
    pub iv: String,
    /// Base64url-encoded 16-byte Poly1305 authentication tag.
    pub tag: String,
}

/// Per-recipient entry in the JWE envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipientHeader {
    /// `{"kid": "did:key:...#key-1"}`
    pub header: HashMap<String, String>,
    /// Base64url-encoded wrapped content key (nonce || ciphertext || tag).
    pub encrypted_key: String,
}

/// Protected header stored inside the JWE envelope.
#[derive(Debug, Serialize, Deserialize)]
struct ProtectedHeader {
    alg: String,
    enc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    skid: Option<String>, // sender DID for authcrypt
    #[serde(skip_serializing_if = "Option::is_none")]
    epk: Option<String>, // base64url(ephemeral_pub) for anoncrypt
}

// ── Key material ──────────────────────────────────────────────────────────────

/// An X25519 keypair with its derived did:key DID.
pub struct DidKeyPair {
    /// Full DID string: `"did:key:z6Mk..."`.
    pub did: String,
    pub public_key: PublicKey,
    pub private_key: StaticSecret,
}

// ── DidcommClient ─────────────────────────────────────────────────────────────

/// DIDComm v2 client bound to a single keypair / DID identity.
pub struct DidcommClient {
    pub did: String,
    key_pair: DidKeyPair,
}

impl DidcommClient {
    // ── Constructors ─────────────────────────────────────────────────────────

    /// Generate a fresh X25519 keypair and derive the corresponding `did:key` DID.
    pub fn generate() -> Result<Self, Box<dyn std::error::Error>> {
        let private_key = StaticSecret::random_from_rng(OsRng);
        let public_key  = PublicKey::from(&private_key);
        let did         = encode_did_key(public_key.as_bytes());
        Ok(Self {
            did: did.clone(),
            key_pair: DidKeyPair { did, public_key, private_key },
        })
    }

    // ── Key resolution ───────────────────────────────────────────────────────

    /// Decode the raw 32-byte X25519 public key from a `did:key` DID string.
    ///
    /// Strips the 2-byte multicodec prefix (`0xec 0x01`).
    pub fn resolve_public_key(did: &str) -> Result<[u8; 32], Box<dyn std::error::Error>> {
        decode_did_key(did)
    }

    // ── Encryption ───────────────────────────────────────────────────────────

    /// Authcrypt: encrypt `message` for `recipient_dids` while authenticating
    /// the sender.
    ///
    /// Each recipient receives a separately wrapped copy of the random content
    /// key, encrypted via X25519 ECDH between the sender's static keypair and
    /// the recipient's public key. The protected header records the sender's DID.
    pub fn pack_authcrypt(
        &self,
        message: &DidcommMessage,
        recipient_dids: &[&str],
    ) -> Result<EncryptedMessage, Box<dyn std::error::Error>> {
        self.pack(message, recipient_dids, false)
    }

    /// Anoncrypt: encrypt `message` for `recipient_dids` without revealing the
    /// sender.
    ///
    /// A fresh ephemeral X25519 keypair is used as the sender for each call.
    /// The ephemeral public key is embedded in the protected header (`epk`).
    pub fn pack_anoncrypt(
        &self,
        message: &DidcommMessage,
        recipient_dids: &[&str],
    ) -> Result<EncryptedMessage, Box<dyn std::error::Error>> {
        self.pack(message, recipient_dids, true)
    }

    /// Decrypt an incoming `EncryptedMessage` addressed to this client's keypair.
    ///
    /// Returns the plaintext [`DidcommMessage`] and the sender's DID (or `None`
    /// for anoncrypt messages).
    ///
    /// # Errors
    /// Returns an error if no recipient entry matches this client's key, or if
    /// decryption fails (wrong key, tampered data, etc.).
    pub fn unpack(
        &self,
        encrypted: &EncryptedMessage,
    ) -> Result<(DidcommMessage, Option<String>), Box<dyn std::error::Error>> {
        // 1. Decode and parse the protected header
        let header_bytes = URL_SAFE_NO_PAD.decode(&encrypted.protected)?;
        let header: ProtectedHeader = serde_json::from_slice(&header_bytes)?;
        let is_anon = header.alg == "ECDH+ChaCha20Poly1305";

        // 2. Recover the sender / ephemeral public key
        let sender_pub_bytes: [u8; 32] = if is_anon {
            let epk_str = header.epk.as_deref()
                .ok_or("Anoncrypt envelope missing 'epk' in protected header")?;
            let epk_bytes = URL_SAFE_NO_PAD.decode(epk_str)?;
            epk_bytes.try_into().map_err(|_| "epk must be 32 bytes")?
        } else {
            let skid = header.skid.as_deref()
                .ok_or("Authcrypt envelope missing 'skid' in protected header")?;
            decode_did_key(skid)?
        };
        let sender_pub = PublicKey::from(sender_pub_bytes);

        // 3. Find the recipient entry matching this client's key
        let my_kid = format!("{}#key-1", self.did);
        let entry = encrypted.recipients.iter().find(|r| {
            r.header.get("kid").map(|k| k == &my_kid).unwrap_or(false)
        }).ok_or_else(|| format!("No recipient entry for key {my_kid}"))?;

        // 4. ECDH → unwrap content key
        //    Stored as base64url(nonce || ciphertext || tag) (ChaCha20-Poly1305 output)
        let ek_bytes = URL_SAFE_NO_PAD.decode(&entry.encrypted_key)?;
        if ek_bytes.len() < 12 {
            return Err("encrypted_key too short".into());
        }
        let (ek_nonce_bytes, ek_ct) = ek_bytes.split_at(12);
        let ek_nonce = Nonce::from_slice(ek_nonce_bytes);

        let shared_secret = self.key_pair.private_key.diffie_hellman(&sender_pub);
        let ek_cipher = ChaCha20Poly1305::new(Key::from_slice(shared_secret.as_bytes()));
        let content_key_bytes = ek_cipher
            .decrypt(ek_nonce, ek_ct)
            .map_err(|e| format!("Content key decryption failed: {e}"))?;
        if content_key_bytes.len() != 32 {
            return Err("Decrypted content key has unexpected length".into());
        }
        let content_key = Key::from_slice(&content_key_bytes);

        // 5. Decrypt body
        let iv_bytes = URL_SAFE_NO_PAD.decode(&encrypted.iv)?;
        let body_nonce = Nonce::from_slice(&iv_bytes);
        let mut body_ct = URL_SAFE_NO_PAD.decode(&encrypted.ciphertext)?;
        let tag_bytes   = URL_SAFE_NO_PAD.decode(&encrypted.tag)?;
        // ChaCha20-Poly1305 expects ciphertext || tag in a single buffer
        body_ct.extend_from_slice(&tag_bytes);

        let body_cipher = ChaCha20Poly1305::new(content_key);
        let plaintext   = body_cipher
            .decrypt(body_nonce, body_ct.as_slice())
            .map_err(|e| format!("Body decryption failed: {e}"))?;

        let message: DidcommMessage = serde_json::from_slice(&plaintext)?;
        let sender_did = if is_anon { None } else { header.skid };

        Ok((message, sender_did))
    }

    // ── Convenience builders ─────────────────────────────────────────────────

    /// Build and encrypt an INVOKE message to `recipient_did`.
    ///
    /// # Arguments
    /// - `recipient_did` — Target agent's DID.
    /// - `capability`    — Capability name to invoke.
    /// - `input`         — Input payload (JSON value).
    /// - `anon`          — If `true`, use anoncrypt; otherwise authcrypt.
    pub fn invoke(
        &self,
        recipient_did: &str,
        capability: &str,
        input: serde_json::Value,
        anon: bool,
    ) -> Result<EncryptedMessage, Box<dyn std::error::Error>> {
        let msg = DidcommMessage {
            id:           uuid::Uuid::new_v4().to_string(),
            msg_type:     MSG_INVOKE.to_string(),
            from:         if anon { None } else { Some(self.did.clone()) },
            to:           vec![recipient_did.to_string()],
            created_time: now_secs(),
            expires_time: None,
            body:         serde_json::json!({ "capability": capability, "input": input }),
            attachments:  None,
        };
        if anon {
            self.pack_anoncrypt(&msg, &[recipient_did])
        } else {
            self.pack_authcrypt(&msg, &[recipient_did])
        }
    }

    /// Build and encrypt a RESPONSE message to `recipient_did`.
    ///
    /// # Arguments
    /// - `recipient_did` — Recipient's DID.
    /// - `reply_to_id`   — The `id` of the INVOKE this responds to.
    /// - `output`        — Result payload (JSON value).
    pub fn respond(
        &self,
        recipient_did: &str,
        reply_to_id: &str,
        output: serde_json::Value,
    ) -> Result<EncryptedMessage, Box<dyn std::error::Error>> {
        let msg = DidcommMessage {
            id:           uuid::Uuid::new_v4().to_string(),
            msg_type:     MSG_RESPONSE.to_string(),
            from:         Some(self.did.clone()),
            to:           vec![recipient_did.to_string()],
            created_time: now_secs(),
            expires_time: None,
            body:         serde_json::json!({ "reply_to": reply_to_id, "output": output }),
            attachments:  None,
        };
        self.pack_authcrypt(&msg, &[recipient_did])
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    /// Shared implementation for authcrypt and anoncrypt.
    ///
    /// Protocol:
    /// 1. Generate a random 32-byte content key and 12-byte body nonce.
    /// 2. Encrypt plaintext message with ChaCha20-Poly1305 (content key).
    /// 3. For each recipient: X25519 ECDH(sender_priv, recipient_pub) →
    ///    shared secret, then ChaCha20-Poly1305 wrap the content key.
    /// 4. Assemble the JWE envelope.
    fn pack(
        &self,
        message: &DidcommMessage,
        recipient_dids: &[&str],
        anon: bool,
    ) -> Result<EncryptedMessage, Box<dyn std::error::Error>> {
        // Step 1 — content key + body nonce
        let content_key_bytes: [u8; 32] = rand::random();
        let content_key = Key::from_slice(&content_key_bytes);
        let body_nonce  = ChaCha20Poly1305::generate_nonce(&mut AeadOsRng);

        // Step 2 — encrypt body
        let plaintext  = serde_json::to_vec(message)?;
        let body_cipher = ChaCha20Poly1305::new(content_key);
        let body_ct_full = body_cipher
            .encrypt(&body_nonce, plaintext.as_slice())
            .map_err(|e| format!("Body encryption failed: {e}"))?;
        // chacha20poly1305 appends the 16-byte tag after the ciphertext
        let (body_ct, tag) = body_ct_full.split_at(body_ct_full.len() - 16);

        // Step 3 — sender keypair: ephemeral for anoncrypt, static for authcrypt.
        //
        // For anoncrypt we generate a fresh ephemeral keypair and embed the
        // ephemeral public key in the protected header (`epk`).
        // For authcrypt we use our own static private key; the sender's DID
        // is recorded in the protected header (`skid`).
        //
        // `eph_priv` holds an ephemeral secret only used in the anoncrypt path.
        // In the authcrypt path we call `self.key_pair.private_key.diffie_hellman`
        // directly, which avoids needing to clone or move the StaticSecret.
        let (sender_pub, eph_priv): (PublicKey, Option<StaticSecret>) = if anon {
            let eph_priv = StaticSecret::random_from_rng(OsRng);
            let eph_pub  = PublicKey::from(&eph_priv);
            (eph_pub, Some(eph_priv))
        } else {
            (PublicKey::from(&self.key_pair.private_key), None)
        };

        let recipients: Result<Vec<RecipientHeader>, Box<dyn std::error::Error>> =
            recipient_dids.iter().map(|did| {
                let recipient_key_bytes = decode_did_key(did)?;
                let recipient_pub       = PublicKey::from(recipient_key_bytes);

                let shared: [u8; 32] = match eph_priv.as_ref() {
                    Some(ep) => ep.diffie_hellman(&recipient_pub).to_bytes(),
                    None     => self.key_pair.private_key.diffie_hellman(&recipient_pub).to_bytes(),
                };

                let ek_nonce  = ChaCha20Poly1305::generate_nonce(&mut AeadOsRng);
                let ek_cipher = ChaCha20Poly1305::new(Key::from_slice(&shared));
                let ek_ct     = ek_cipher
                    .encrypt(&ek_nonce, content_key_bytes.as_ref())
                    .map_err(|e| format!("Key wrapping failed: {e}"))?;

                // Pack as nonce || ciphertext (tag is appended by aead crate)
                let mut ek_bytes = ek_nonce.to_vec();
                ek_bytes.extend_from_slice(&ek_ct);

                let mut header_map = HashMap::new();
                header_map.insert("kid".to_string(), format!("{did}#key-1"));

                Ok(RecipientHeader {
                    header:        header_map,
                    encrypted_key: URL_SAFE_NO_PAD.encode(&ek_bytes),
                })
            }).collect();

        let recipients = recipients?;

        // Step 4 — protected header
        let protected_header = ProtectedHeader {
            alg:  if anon { "ECDH+ChaCha20Poly1305".into() }
                  else    { "ECDH-1PU+ChaCha20Poly1305".into() },
            enc:  "ChaCha20Poly1305".into(),
            skid: if anon { None } else { Some(self.did.clone()) },
            epk:  if anon {
                Some(URL_SAFE_NO_PAD.encode(sender_pub.as_bytes()))
            } else {
                None
            },
        };
        let protected_json  = serde_json::to_vec(&protected_header)?;
        let protected_b64   = URL_SAFE_NO_PAD.encode(&protected_json);

        Ok(EncryptedMessage {
            ciphertext: URL_SAFE_NO_PAD.encode(body_ct),
            protected:  protected_b64,
            recipients,
            iv:         URL_SAFE_NO_PAD.encode(body_nonce.as_slice()),
            tag:        URL_SAFE_NO_PAD.encode(tag),
        })
    }
}

// ── did:key helpers ───────────────────────────────────────────────────────────

/// X25519 multicodec prefix: `0xec 0x01` (varint-encoded).
const MULTICODEC_X25519_PUB: [u8; 2] = [0xec, 0x01];

/// Encode 32-byte X25519 public key as a `did:key` DID.
///
/// Format: `"did:key:z"` + base58btc(`0xec 0x01` || pubkey)
fn encode_did_key(public_key_bytes: &[u8; 32]) -> String {
    let mut prefixed = Vec::with_capacity(2 + 32);
    prefixed.extend_from_slice(&MULTICODEC_X25519_PUB);
    prefixed.extend_from_slice(public_key_bytes);
    format!("did:key:z{}", bs58::encode(&prefixed).into_string())
}

/// Decode a `did:key` DID and return the raw 32-byte X25519 public key.
///
/// Strips the 2-byte multicodec prefix.
fn decode_did_key(did: &str) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    if !did.starts_with("did:key:z") {
        return Err(format!("Not a did:key DID with multibase 'z': {did}").into());
    }
    let encoded = &did["did:key:z".len()..];
    let decoded = bs58::decode(encoded).into_vec()?;
    if decoded.len() < 34 {
        return Err(format!("did:key payload too short: {} bytes", decoded.len()).into());
    }
    // Skip the 2-byte multicodec prefix
    let key_bytes: [u8; 32] = decoded[2..34]
        .try_into()
        .map_err(|_| "Key slice has unexpected length")?;
    Ok(key_bytes)
}

// ── Misc helpers ──────────────────────────────────────────────────────────────

/// Return the current Unix time in whole seconds.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_did_key_roundtrip() {
        let client = DidcommClient::generate().unwrap();
        assert!(client.did.starts_with("did:key:z"));
        let decoded = decode_did_key(&client.did).unwrap();
        let re_encoded = encode_did_key(&decoded);
        assert_eq!(client.did, re_encoded);
    }

    #[test]
    fn test_authcrypt_roundtrip() {
        let alice = DidcommClient::generate().unwrap();
        let bob   = DidcommClient::generate().unwrap();

        let encrypted = alice
            .invoke(&bob.did, "echo", json!({"msg": "hello"}), false)
            .unwrap();

        let (msg, sender) = bob.unpack(&encrypted).unwrap();
        assert_eq!(msg.msg_type, MSG_INVOKE);
        assert_eq!(sender.as_deref(), Some(alice.did.as_str()));
        assert_eq!(msg.body["capability"], "echo");
        assert_eq!(msg.body["input"]["msg"], "hello");
    }

    #[test]
    fn test_anoncrypt_roundtrip() {
        let alice = DidcommClient::generate().unwrap();
        let bob   = DidcommClient::generate().unwrap();

        let encrypted = alice
            .invoke(&bob.did, "ping", json!({}), true)
            .unwrap();

        let (msg, sender) = bob.unpack(&encrypted).unwrap();
        assert_eq!(msg.msg_type, MSG_INVOKE);
        assert!(sender.is_none(), "anoncrypt should not reveal sender");
        assert_eq!(msg.body["capability"], "ping");
    }

    #[test]
    fn test_respond_roundtrip() {
        let alice = DidcommClient::generate().unwrap();
        let bob   = DidcommClient::generate().unwrap();

        let req = alice.invoke(&bob.did, "greet", json!({"name": "Alice"}), false).unwrap();
        let (req_msg, _) = bob.unpack(&req).unwrap();

        let resp = bob.respond(&alice.did, &req_msg.id, json!({"greeting": "Hello, Alice!"})).unwrap();
        let (resp_msg, sender) = alice.unpack(&resp).unwrap();
        assert_eq!(resp_msg.msg_type, MSG_RESPONSE);
        assert_eq!(sender.as_deref(), Some(bob.did.as_str()));
        assert_eq!(resp_msg.body["output"]["greeting"], "Hello, Alice!");
    }

    #[test]
    fn test_wrong_key_fails() {
        let alice = DidcommClient::generate().unwrap();
        let bob   = DidcommClient::generate().unwrap();
        let carol = DidcommClient::generate().unwrap();

        // Alice encrypts for Bob; Carol tries to decrypt — should fail
        let encrypted = alice.invoke(&bob.did, "secret", json!({}), false).unwrap();
        let result = carol.unpack(&encrypted);
        assert!(result.is_err(), "Carol should not be able to decrypt Bob's message");
    }
}
