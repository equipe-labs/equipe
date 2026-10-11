//! PKCE (RFC 7636, S256) and the OAuth `state` value.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

pub struct Pkce {
    /// Stays in this process. Only its hash goes on the wire.
    pub verifier: String,
    pub challenge: String,
}

pub fn generate() -> Pkce {
    let mut bytes = [0u8; 64];
    rand::fill(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = challenge_for(&verifier);
    Pkce {
        verifier,
        challenge,
    }
}

/// `BASE64URL(SHA256(verifier))`, unpadded.
pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A random, URL-safe value that binds the callback to this login (CSRF).
pub fn random_state() -> String {
    let mut bytes = [0u8; 16];
    rand::fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn is_base64url(s: &str) -> bool {
        !s.is_empty()
            && s
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }

    #[test]
    fn challenge_matches_the_rfc_7636_example() {
        // RFC 7636, Appendix B.
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generated_pair_is_consistent() {
        let pkce = generate();
        assert_eq!(pkce.challenge, challenge_for(&pkce.verifier));
    }

    #[test]
    fn values_are_url_safe_and_within_spec_length() {
        let pkce = generate();
        assert!(is_base64url(&pkce.verifier));
        assert!(is_base64url(&pkce.challenge));
        // RFC 7636 §4.1 constrains the verifier to 43–128 characters.
        assert!((43..=128).contains(&pkce.verifier.len()), "{}", pkce.verifier.len());
    }

    #[test]
    fn verifiers_never_repeat() {
        let seen: HashSet<_> = (0..50).map(|_| generate().verifier).collect();
        assert_eq!(seen.len(), 50);
    }

    #[test]
    fn state_is_unique_and_url_safe() {
        let seen: HashSet<_> = (0..50).map(|_| random_state()).collect();
        assert_eq!(seen.len(), 50);
        assert!(is_base64url(&random_state()));
    }
}
