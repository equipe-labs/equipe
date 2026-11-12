//! Authenticator codes (TOTP, RFC 6238 over HOTP, RFC 4226) from a seed the
//! person kept for a login: given as an `otpauth://totp/…` address (what a
//! site's QR code holds) or as the bare base32 secret a site shows under
//! "can't scan it?". The seed is kept in the secret store as a canonical
//! `otpauth://` address ([`Totp::to_uri`]); a code is made when a code page
//! of that login's exact site waits, typed in by equipe, and never shown.

use ring::hmac;

/// The hash a seed's codes are made with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    fn name(self) -> &'static str {
        match self {
            Algorithm::Sha1 => "SHA1",
            Algorithm::Sha256 => "SHA256",
            Algorithm::Sha512 => "SHA512",
        }
    }

    fn ring(self) -> hmac::Algorithm {
        match self {
            Algorithm::Sha1 => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
            Algorithm::Sha256 => hmac::HMAC_SHA256,
            Algorithm::Sha512 => hmac::HMAC_SHA512,
        }
    }
}

/// A seed and how its codes are made.
#[derive(Clone, PartialEq, Eq)]
pub struct Totp {
    secret: Vec<u8>,
    pub algorithm: Algorithm,
    pub digits: u32,
    pub period: u64,
}

/// Never the seed, whoever prints it.
impl std::fmt::Debug for Totp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Totp").field("secret", &"•••").field("algorithm", &self.algorithm).field("digits", &self.digits).field("period", &self.period).finish()
    }
}

/// RFC 4648 base32, as authenticator seeds are written: any case, spaces,
/// hyphens and `=` padding ignored.
pub fn base32_decode(text: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u64, 0u32);
    let mut any = false;
    for c in text.chars().filter(|c| !matches!(c, ' ' | '-' | '=' | '\t')) {
        let v = ALPHABET.iter().position(|&a| a == c.to_ascii_uppercase() as u8)? as u64;
        any = true;
        buffer = (buffer << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    any.then_some(out)
}

fn base32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u64, 0u32);
    for &b in bytes {
        buffer = (buffer << 8) | b as u64;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// `%XX` in an address's query, read back.
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Totp {
    /// What the person gave: an `otpauth://totp/…` address, or a bare base32
    /// secret (SHA-1, six digits, thirty seconds: what every site means by one).
    pub fn parse(given: &str) -> Result<Totp, String> {
        let given = given.trim();
        if given.is_empty() {
            return Err("give the authenticator's secret or its otpauth:// address".into());
        }
        let lower = given.to_ascii_lowercase();
        if lower.starts_with("otpauth://hotp") {
            return Err("that is a counter-based (HOTP) seed; equipe makes time-based (TOTP) codes only".into());
        }
        if !lower.starts_with("otpauth://") {
            let secret = base32_decode(given).ok_or("that isn't an authenticator secret (letters A–Z and digits 2–7)")?;
            return Totp::new(secret, Algorithm::Sha1, 6, 30);
        }
        if !lower.starts_with("otpauth://totp/") && !lower.starts_with("otpauth://totp?") {
            return Err("that isn't an otpauth://totp/ address".into());
        }
        let query = given.split_once('?').map_or("", |(_, q)| q);
        let (mut secret, mut algorithm, mut digits, mut period) = (None, Algorithm::Sha1, 6, 30);
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = unescape(value);
            match key.to_ascii_lowercase().as_str() {
                "secret" => secret = base32_decode(&value),
                "algorithm" => {
                    algorithm = match value.to_ascii_uppercase().as_str() {
                        "SHA1" => Algorithm::Sha1,
                        "SHA256" => Algorithm::Sha256,
                        "SHA512" => Algorithm::Sha512,
                        other => return Err(format!("the {other} algorithm isn't one authenticators use")),
                    }
                }
                "digits" => digits = value.parse().map_err(|_| "its digits aren't a number")?,
                "period" => period = value.parse().map_err(|_| "its period isn't a number")?,
                _ => {}
            }
        }
        Totp::new(secret.ok_or("the address has no secret in it")?, algorithm, digits, period)
    }

    fn new(secret: Vec<u8>, algorithm: Algorithm, digits: u32, period: u64) -> Result<Totp, String> {
        if secret.len() < 10 {
            return Err("that secret is too short to be an authenticator's".into());
        }
        if !(6..=8).contains(&digits) {
            return Err("codes are 6 to 8 digits".into());
        }
        if !(15..=120).contains(&period) {
            return Err("a code's period is 15 to 120 seconds".into());
        }
        Ok(Totp { secret, algorithm, digits, period })
    }

    /// The canonical address it is kept as.
    pub fn to_uri(&self) -> String {
        format!("otpauth://totp/equipe?secret={}&algorithm={}&digits={}&period={}", base32_encode(&self.secret), self.algorithm.name(), self.digits, self.period)
    }

    /// HOTP of `counter`.
    fn hotp(&self, counter: u64) -> String {
        let key = hmac::Key::new(self.algorithm.ring(), &self.secret);
        let tag = hmac::sign(&key, &counter.to_be_bytes());
        let h = tag.as_ref();
        let offset = (h[h.len() - 1] & 0x0f) as usize;
        let bin = ((h[offset] as u32 & 0x7f) << 24) | ((h[offset + 1] as u32) << 16) | ((h[offset + 2] as u32) << 8) | h[offset + 3] as u32;
        let code = bin % 10u32.pow(self.digits);
        format!("{code:0width$}", width = self.digits as usize)
    }

    /// The code at unix time `at`.
    pub fn code_at(&self, at: u64) -> String {
        self.hotp(at / self.period)
    }

    /// The code to type now: the next one when this one has under three
    /// seconds left — it would run out between typing and the site's check.
    pub fn code_to_type(&self, now: u64) -> String {
        let left = self.period - now % self.period;
        if left < 3 { self.code_at(now + left) } else { self.code_at(now) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rfc(secret: &[u8], algorithm: Algorithm) -> Totp {
        Totp { secret: secret.to_vec(), algorithm, digits: 8, period: 30 }
    }

    #[test]
    fn codes_match_rfc_6238s_test_vectors() {
        let sha1 = rfc(b"12345678901234567890", Algorithm::Sha1);
        let sha256 = rfc(b"12345678901234567890123456789012", Algorithm::Sha256);
        let sha512 = rfc(b"1234567890123456789012345678901234567890123456789012345678901234", Algorithm::Sha512);
        for (at, a, b, c) in [
            (59, "94287082", "46119246", "90693936"),
            (1_111_111_109, "07081804", "68084774", "25091201"),
            (1_234_567_890, "89005924", "91819424", "93441116"),
            (20_000_000_000, "65353130", "77737706", "47863826"),
        ] {
            assert_eq!((sha1.code_at(at).as_str(), sha256.code_at(at).as_str(), sha512.code_at(at).as_str()), (a, b, c), "at {at}");
        }
    }

    #[test]
    fn a_seed_is_read_from_an_otpauth_address_or_a_bare_secret_and_kept_canonical() {
        let uri = "otpauth://totp/GitHub:abel?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=GitHub";
        let t = Totp::parse(uri).unwrap();
        assert_eq!((t.digits, t.period, t.algorithm), (6, 30, Algorithm::Sha1));
        assert_eq!(t.code_at(59), "287082", "RFC 6238's first vector, six digits");
        let bare = Totp::parse("gezd gnbv gy3t qojq gezd gnbv gy3t qojq").unwrap();
        assert_eq!(bare, t);
        let again = Totp::parse(&t.to_uri()).unwrap();
        assert_eq!(again, t, "kept as an address it reads back from");
        let eight = Totp::parse("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=8&algorithm=SHA1&period=30").unwrap();
        assert_eq!(eight.code_at(59), "94287082");
        for bad in ["", "otpauth://hotp/x?secret=GEZDGNBVGY3TQOJQ&counter=1", "not a secret!", "otpauth://totp/x?issuer=a", "GEZDG", "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQGEZD&digits=12"] {
            assert!(Totp::parse(bad).is_err(), "{bad}");
        }
        assert!(!format!("{t:?}").contains("GEZD"), "never printed");
    }

    #[test]
    fn a_code_about_to_run_out_is_the_next_one() {
        let t = Totp::parse("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").unwrap();
        assert_eq!(t.code_to_type(30), t.code_at(30));
        assert_eq!(t.code_to_type(58), t.code_at(60), "two seconds left");
    }
}
