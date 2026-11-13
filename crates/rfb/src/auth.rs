//! VNC authentication (RFB security type 2) and the `vncpasswd` file.
//!
//! The server sends 16 random bytes; the client encrypts them with DES, the
//! key being the password's first 8 bytes. VNC's DES takes each key byte with
//! its bits in the opposite order from standard DES, so the key bytes are
//! reversed before a standard DES is used.
//!
//! TigerVNC's `vncpasswd` file holds the password (8 bytes, zero-padded) DES-
//! encrypted with a fixed key: obfuscated, not protected — the file is kept
//! 0600 for that reason.

use des::Des;
use des::cipher::generic_array::GenericArray;
use des::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};

/// The key every VNC implementation obfuscates the password file with.
const FILE_KEY: [u8; 8] = [23, 82, 107, 6, 35, 78, 88, 7];

fn cipher(key: [u8; 8]) -> Des {
    let reversed = key.map(u8::reverse_bits);
    Des::new_from_slice(&reversed).expect("DES takes an 8-byte key")
}

/// The password as VNC uses it: its first 8 bytes, zero-padded.
fn padded(password: &[u8]) -> [u8; 8] {
    let mut key = [0u8; 8];
    for (k, b) in key.iter_mut().zip(password) {
        *k = *b;
    }
    key
}

/// The answer to the server's challenge.
pub fn response(password: &[u8], challenge: &[u8; 16]) -> [u8; 16] {
    let des = cipher(padded(password));
    let mut out = *challenge;
    for half in out.as_chunks_mut::<8>().0 {
        des.encrypt_block(GenericArray::from_mut_slice(half));
    }
    out
}

/// The password a `vncpasswd` file holds (its first 8 bytes; a longer file's
/// rest is a view-only password, not used here). Trailing zeros dropped.
pub fn read_file(bytes: &[u8]) -> Option<Vec<u8>> {
    let block: [u8; 8] = bytes.get(..8)?.try_into().ok()?;
    let mut plain = block;
    cipher(FILE_KEY).decrypt_block(GenericArray::from_mut_slice(&mut plain));
    let end = plain.iter().position(|&b| b == 0).unwrap_or(8);
    Some(plain[..end].to_vec())
}

/// A `vncpasswd` file's bytes for `password` (only its first 8 bytes count,
/// as with every VNC server).
pub fn file(password: &[u8]) -> [u8; 8] {
    let mut block = padded(password);
    cipher(FILE_KEY).encrypt_block(GenericArray::from_mut_slice(&mut block));
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_password_file_reads_as_vncpasswd_writes_it() {
        // `printf password | vncpasswd -f | xxd -p` (TigerVNC 1.13).
        let made = [0xdb, 0xd8, 0x3c, 0xfd, 0x72, 0x7a, 0x14, 0x58];
        assert_eq!(file(b"password"), made);
        assert_eq!(read_file(&made).unwrap(), b"password");
        assert_eq!(read_file(&file(b"abc")).unwrap(), b"abc");
        assert_eq!(read_file(&file(b"longer-than-eight")).unwrap(), b"longer-t", "VNC keeps 8 bytes");
        assert_eq!(read_file(b"short"), None);
    }

    #[test]
    fn the_challenge_is_answered_with_the_bit_reversed_key() {
        // (Checked against TigerVNC 1.13's x0vncserver too: `equipe screen` signs in with it.)
        let challenge: [u8; 16] = core::array::from_fn(|i| i as u8);
        let answer = response(b"password", &challenge);
        // Each half is the same DES over its own 8 bytes.
        let first: [u8; 16] = { let mut c = [0u8; 16]; c[..8].copy_from_slice(&challenge[..8]); c[8..].copy_from_slice(&challenge[..8]); c };
        let doubled = response(b"password", &first);
        assert_eq!(answer[..8], doubled[..8]);
        assert_eq!(doubled[..8], doubled[8..]);
        assert_ne!(answer[..8], answer[8..]);
        // Only 8 bytes of the password count.
        assert_eq!(response(b"password", &challenge), response(b"password-and-more", &challenge));
    }
}
