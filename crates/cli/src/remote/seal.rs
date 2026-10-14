//! Sealing frames between the daemon and a device: HPKE (RFC 9180) with
//! X25519, HKDF-SHA256 and ChaCha20-Poly1305, one setup per frame — what
//! CryptoKit, BouncyCastle and this crate all speak. See
//! `docs/remote-protocol.md` § Sealing.

use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable as _, Kem as _, OpModeR, OpModeS, PskBundle, Serializable as _};

type Kem = X25519HkdfSha256;
type Aead = hpke::aead::ChaCha20Poly1305;
type Kdf = hpke::kdf::HkdfSha256;
type PublicKey = <Kem as hpke::Kem>::PublicKey;
type PrivateKey = <Kem as hpke::Kem>::PrivateKey;

const PSK_ID: &[u8] = b"equipe-pair";
/// The encapsulated key at the front of every sealed frame.
const ENC: usize = 32;

/// Which of the four kinds of frame: each has its own `info`, so one can't
/// pass for another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    PairRequest,
    PairAnswer,
    ToDaemon,
    ToDevice,
}

impl Kind {
    fn info(self) -> &'static [u8] {
        match self {
            Kind::PairRequest => b"equipe/1 pair-request",
            Kind::PairAnswer => b"equipe/1 pair-answer",
            Kind::ToDaemon => b"equipe/1 device-to-daemon",
            Kind::ToDevice => b"equipe/1 daemon-to-device",
        }
    }
}

/// A key pair, as kept: base64url of the private and the public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keys {
    pub private: String,
    pub public: String,
}

pub fn generate() -> Keys {
    let (private, public) = Kem::gen_keypair();
    Keys { private: super::b64(&private.to_bytes()), public: super::b64(&public.to_bytes()) }
}

fn public(key: &str) -> Result<PublicKey, String> {
    PublicKey::from_bytes(&super::unb64(key)?).map_err(|_| "not a public key".to_owned())
}

fn private(key: &str) -> Result<PrivateKey, String> {
    PrivateKey::from_bytes(&super::unb64(key)?).map_err(|_| "not a private key".to_owned())
}

/// Seal `plaintext` for `to`. `from` (the sender's keys) authenticates it;
/// `psk` (the pairing secret) proves the QR code was seen.
pub fn seal(kind: Kind, to: &str, from: Option<&Keys>, psk: Option<&[u8]>, aad: &str, plaintext: &[u8]) -> Result<String, String> {
    let to = public(to)?;
    let sender = from.map(|k| Ok::<_, String>((private(&k.private)?, public(&k.public)?))).transpose()?;
    let mode = sender_mode(sender, psk)?;
    let (enc, ciphertext) = hpke::single_shot_seal::<Aead, Kdf, Kem>(&mode, &to, kind.info(), plaintext, aad.as_bytes()).map_err(|err| err.to_string())?;
    let mut frame = enc.to_bytes().to_vec();
    frame.extend_from_slice(&ciphertext);
    Ok(super::b64(&frame))
}

/// `seal` with the ephemeral key made from `ikm_e` instead of at random —
/// for the conformance vectors, whose frames must come out the same each time.
#[cfg(test)]
pub(crate) fn seal_with_ephemeral(kind: Kind, to: &str, from: Option<&Keys>, psk: Option<&[u8]>, aad: &str, plaintext: &[u8], ikm_e: &[u8; 32]) -> Result<String, String> {
    /// Hands out `ikm_e` once: the bytes `gen_keypair_with_rng` derives the ephemeral key pair from.
    struct Fixed([u8; 32], bool);
    impl hpke::rand_core::TryRng for Fixed {
        type Error = std::convert::Infallible;
        fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
            unreachable!("only whole keys are drawn")
        }
        fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
            unreachable!("only whole keys are drawn")
        }
        fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
            assert!(!self.1 && dst.len() == 32, "one ephemeral key per frame");
            dst.copy_from_slice(&self.0);
            self.1 = true;
            Ok(())
        }
    }
    impl hpke::rand_core::TryCryptoRng for Fixed {}
    let to = public(to)?;
    let sender = from.map(|k| Ok::<_, String>((private(&k.private)?, public(&k.public)?))).transpose()?;
    let mode = sender_mode(sender, psk)?;
    let (enc, ciphertext) = hpke::single_shot_seal_with_rng::<Aead, Kdf, Kem>(&mode, &to, kind.info(), plaintext, aad.as_bytes(), &mut Fixed(*ikm_e, false)).map_err(|err| err.to_string())?;
    let mut frame = enc.to_bytes().to_vec();
    frame.extend_from_slice(&ciphertext);
    Ok(super::b64(&frame))
}

/// The HPKE mode a frame is sealed in: who sent it, and whether the pairing secret is in it.
fn sender_mode(sender: Option<(PrivateKey, PublicKey)>, psk: Option<&[u8]>) -> Result<OpModeS<'_, Kem>, String> {
    let bundle = psk.map(|psk| PskBundle::new(psk, PSK_ID)).transpose().map_err(|_| "bad pairing secret")?;
    Ok(match (sender, bundle) {
        (None, None) => OpModeS::Base,
        (None, Some(b)) => OpModeS::Psk(b),
        (Some(pair), None) => OpModeS::Auth(pair),
        (Some(pair), Some(b)) => OpModeS::AuthPsk(pair, b),
    })
}

/// The key pair a 32-byte seed derives (RFC 9180 `DeriveKeyPair`), as kept.
#[cfg(test)]
pub(crate) fn derive(ikm: &[u8]) -> Keys {
    let (private, public) = Kem::derive_keypair(ikm);
    Keys { private: super::b64(&private.to_bytes()), public: super::b64(&public.to_bytes()) }
}

/// Open a frame sealed for `me`; `from` must be who sealed it, when it says who.
pub fn open(kind: Kind, me: &Keys, from: Option<&str>, psk: Option<&[u8]>, aad: &str, sealed: &str) -> Result<Vec<u8>, String> {
    let frame = super::unb64(sealed)?;
    if frame.len() < ENC {
        return Err("frame too short".into());
    }
    let enc = <Kem as hpke::Kem>::EncappedKey::from_bytes(&frame[..ENC]).map_err(|_| "bad frame")?;
    let me = private(&me.private)?;
    let sender = from.map(public).transpose()?;
    let bundle = psk.map(|psk| PskBundle::new(psk, PSK_ID)).transpose().map_err(|_| "bad pairing secret")?;
    let mode = match (sender, bundle) {
        (None, None) => OpModeR::Base,
        (None, Some(b)) => OpModeR::Psk(b),
        (Some(pk), None) => OpModeR::Auth(pk),
        (Some(pk), Some(b)) => OpModeR::AuthPsk(pk, b),
    };
    hpke::single_shot_open::<Aead, Kdf, Kem>(&mode, &me, &enc, kind.info(), &frame[ENC..], aad.as_bytes()).map_err(|_| "the frame didn't open".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_open_only_for_who_they_were_sealed_for_and_from_who_sealed_them() {
        let daemon = generate();
        let device = generate();
        let stranger = generate();
        let sealed = seal(Kind::ToDaemon, &daemon.public, Some(&device), None, "d:x", b"hello").unwrap();
        assert_eq!(open(Kind::ToDaemon, &daemon, Some(&device.public), None, "d:x", &sealed).unwrap(), b"hello");
        // Not from someone else, not for someone else, not as another kind, not with other aad.
        assert!(open(Kind::ToDaemon, &daemon, Some(&stranger.public), None, "d:x", &sealed).is_err());
        assert!(open(Kind::ToDaemon, &stranger, Some(&device.public), None, "d:x", &sealed).is_err());
        assert!(open(Kind::ToDevice, &daemon, Some(&device.public), None, "d:x", &sealed).is_err());
        assert!(open(Kind::ToDaemon, &daemon, Some(&device.public), None, "d:y", &sealed).is_err());
        // A stranger can't pass for the device.
        let forged = seal(Kind::ToDaemon, &daemon.public, Some(&stranger), None, "d:x", b"hello").unwrap();
        assert!(open(Kind::ToDaemon, &daemon, Some(&device.public), None, "d:x", &forged).is_err());
    }

    #[test]
    fn pairing_needs_the_secret_from_the_qr_code() {
        let daemon = generate();
        let device = generate();
        let secret = [7u8; 32];
        let request = seal(Kind::PairRequest, &daemon.public, None, Some(&secret), "d", b"{}").unwrap();
        assert_eq!(open(Kind::PairRequest, &daemon, None, Some(&secret), "d", &request).unwrap(), b"{}");
        assert!(open(Kind::PairRequest, &daemon, None, Some(&[8u8; 32]), "d", &request).is_err());
        let answer = seal(Kind::PairAnswer, &device.public, Some(&daemon), Some(&secret), "d", b"ok").unwrap();
        assert_eq!(open(Kind::PairAnswer, &device, Some(&daemon.public), Some(&secret), "d", &answer).unwrap(), b"ok");
    }
}
