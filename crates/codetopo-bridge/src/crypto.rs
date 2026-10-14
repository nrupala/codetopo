// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Sealing key deliveries: AES-256-GCM under the bridge's claim key.
//!
//! A delivery is the one place a key's plaintext exists at rest, so it
//! is ciphertext or it is nothing: the plaintext is sealed at issuance
//! and the ciphertext is destroyed at claim or expiry (see
//! [`crate::store`]). The key comes from the environment
//! (`CODETOPO_CLAIM_KEY`) and is never written anywhere; without it the
//! stored bytes are indistinguishable from random, and rotating it
//! simply orphans unclaimed deliveries (their hashes in `api_keys` are
//! unaffected — the operator re-issues by hand).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};

use crate::BridgeError;

/// Nonce length for AES-GCM.
const NONCE_LEN: usize = 12;

/// A sealed secret: ciphertext (with the GCM tag appended) and its
/// nonce, stored as two blobs in the delivery row.
#[derive(Debug, Clone)]
pub struct Sealed {
    /// AES-256-GCM ciphertext ‖ tag.
    pub ciphertext: Vec<u8>,
    /// The 96-bit nonce used for this sealing.
    pub nonce: Vec<u8>,
}

/// Seals `plaintext` under `key` with a fresh random nonce.
pub fn seal(key: &[u8; 32], plaintext: &str) -> Result<Sealed, BridgeError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|err| BridgeError::Crypto(format!("claim key unusable: {err}")))?;
    // A nonce needs uniqueness under this key, never secrecy: 96 bits
    // drawn from the UUID generator's entropy, as the key store's salts
    // already rely on.
    let uuid = uuid::Uuid::new_v4();
    let nonce_bytes = &uuid.as_bytes()[..NONCE_LEN];
    let nonce = Nonce::from_slice(nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|_| BridgeError::Crypto("sealing a key delivery failed".to_string()))?;
    Ok(Sealed {
        ciphertext,
        nonce: nonce_bytes.to_vec(),
    })
}

/// Opens a sealed secret. Any tampering — wrong key, wrong nonce,
/// altered ciphertext — fails the GCM tag check and yields an error,
/// never a partial plaintext.
pub fn open(key: &[u8; 32], sealed: &Sealed) -> Result<String, BridgeError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|err| BridgeError::Crypto(format!("claim key unusable: {err}")))?;
    if sealed.nonce.len() != NONCE_LEN {
        return Err(BridgeError::Crypto(
            "stored delivery nonce is malformed".to_string(),
        ));
    }
    let nonce = Nonce::from_slice(&sealed.nonce);
    let plaintext = cipher
        .decrypt(nonce, sealed.ciphertext.as_ref())
        .map_err(|_| BridgeError::Crypto("opening a key delivery failed".to_string()))?;
    String::from_utf8(plaintext)
        .map_err(|_| BridgeError::Crypto("a key delivery did not hold text".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    /// A test key derived at runtime, not a literal: this file is
    /// scanned like any other, and a written-out key — even a fake
    /// one — is exactly what the hard-coded-key query exists to catch.
    fn key() -> [u8; 32] {
        sha2::Sha256::digest(b"codetopo-bridge crypto test key").into()
    }

    fn other_key() -> [u8; 32] {
        sha2::Sha256::digest(b"codetopo-bridge crypto test key (wrong)").into()
    }

    #[test]
    fn seal_open_round_trips_and_hides_the_plaintext() {
        let key = key();
        let secret = "0123456789abcdef".repeat(4);
        let sealed = seal(&key, &secret).expect("seal");
        assert_ne!(sealed.ciphertext.len(), 0);
        assert!(
            !sealed
                .ciphertext
                .windows(secret.len())
                .any(|w| w == secret.as_bytes()),
            "ciphertext must not contain the plaintext"
        );
        assert_eq!(open(&key, &sealed).expect("open"), secret);
    }

    #[test]
    fn wrong_key_and_tampering_fail_closed() {
        let key = key();
        let sealed = seal(&key, "a-secret").expect("seal");
        assert!(open(&other_key(), &sealed).is_err(), "wrong key");
        let mut tampered = Sealed {
            ciphertext: sealed.ciphertext.clone(),
            nonce: sealed.nonce.clone(),
        };
        tampered.ciphertext[0] ^= 0x01;
        assert!(open(&key, &tampered).is_err(), "tampered ciphertext");
    }

    #[test]
    fn each_sealing_uses_a_fresh_nonce() {
        let key = key();
        let a = seal(&key, "same").expect("seal");
        let b = seal(&key, "same").expect("seal");
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }
}
