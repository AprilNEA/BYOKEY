//! PKCE (Proof Key for Code Exchange) and random state generation utilities.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore as _;
use sha2::{Digest, Sha256};

/// Generate a PKCE `(code_verifier, code_challenge_s256)` pair using SHA-256.
#[must_use]
pub fn generate_pkce() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(digest.as_slice());
    (verifier, challenge)
}

/// Generate a random `state` parameter (hex-encoded, 32 lowercase hex chars, matching the vibeproxy Go implementation).
#[must_use]
pub fn random_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().fold(String::with_capacity(32), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verifier_is_base64url() {
        let (verifier, _) = generate_pkce();
        // base64url-no-pad: only A-Z a-z 0-9 - _
        assert!(
            verifier
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        );
        assert!(!verifier.contains('='));
    }

    #[test]
    fn test_challenge_differs_from_verifier() {
        let (verifier, challenge) = generate_pkce();
        assert_ne!(verifier, challenge);
    }

    #[test]
    fn test_two_calls_produce_different_values() {
        let (v1, c1) = generate_pkce();
        let (v2, c2) = generate_pkce();
        assert_ne!(v1, v2);
        assert_ne!(c1, c2);
    }

    #[test]
    fn test_random_state_is_hex() {
        let s = random_state();
        // hex: 32 lowercase chars in 0-9a-f
        assert_eq!(s.len(), 32, "state should be 32 hex chars");
        assert!(
            s.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        );
    }

    #[test]
    fn test_random_state_different_each_call() {
        let s1 = random_state();
        let s2 = random_state();
        assert_ne!(s1, s2);
    }
}
