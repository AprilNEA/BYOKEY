//! PKCE (Proof Key for Code Exchange) and random state generation utilities.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore as _;
use sha2::{Digest, Sha256};

/// A PKCE pair: the secret `code_verifier` the client keeps, and its S256
/// `code_challenge` sent with the authorization request.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    /// A fresh random verifier and its SHA-256 challenge.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier,
            challenge,
        }
    }
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
        let Pkce { verifier, .. } = Pkce::generate();
        // base64url-no-pad: only A-Z a-z 0-9 - _
        assert!(
            verifier
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        );
        assert!(!verifier.contains('='));
    }

    #[test]
    fn test_challenge_is_the_s256_of_the_verifier() {
        let pkce = Pkce::generate();
        assert_eq!(
            pkce.challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.verifier.as_bytes()))
        );
        assert_ne!(pkce.verifier, pkce.challenge);
    }

    #[test]
    fn test_two_calls_produce_different_values() {
        let (a, b) = (Pkce::generate(), Pkce::generate());
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(a.challenge, b.challenge);
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
