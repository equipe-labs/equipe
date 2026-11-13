//! The handshake and the client's messages (RFC 6143, plus the community's
//! extensions the panel uses: continuous updates and fences).

use std::fmt;
use std::io::{self, Read, Write};

/// Security types (RFC 6143 7.1.2).
pub mod security {
    pub const NONE: u8 = 1;
    pub const VNC: u8 = 2;
}

/// Encodings and pseudo-encodings the panel asks for.
pub mod encoding {
    pub const RAW: i32 = 0;
    pub const COPY_RECT: i32 = 1;
    pub const TIGHT: i32 = 7;
    pub const ZRLE: i32 = 16;
    pub const DESKTOP_SIZE: i32 = -223;
    pub const LAST_RECT: i32 = -224;
    pub const EXTENDED_DESKTOP_SIZE: i32 = -308;
    pub const FENCE: i32 = -312;
    pub const CONTINUOUS_UPDATES: i32 = -313;

    /// Tight's JPEG quality, 0 (smallest) to 9 (best).
    pub fn quality(level: u8) -> i32 {
        -32 + i32::from(level.min(9))
    }

    /// Tight's and ZRLE's zlib effort, 0 (fastest) to 9 (smallest).
    pub fn compress(level: u8) -> i32 {
        -256 + i32::from(level.min(9))
    }
}

/// Client message types.
pub mod client {
    pub const SET_PIXEL_FORMAT: u8 = 0;
    pub const SET_ENCODINGS: u8 = 2;
    pub const UPDATE_REQUEST: u8 = 3;
    pub const KEY: u8 = 4;
    pub const POINTER: u8 = 5;
    pub const CUT_TEXT: u8 = 6;
    pub const ENABLE_CONTINUOUS_UPDATES: u8 = 150;
    pub const FENCE: u8 = 248;
}

/// Fence flags.
pub mod fence {
    pub const BLOCK_BEFORE: u32 = 1;
    pub const BLOCK_AFTER: u32 = 2;
    pub const SYNC_NEXT: u32 = 4;
    pub const REQUEST: u32 = 1 << 31;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelFormat {
    pub bits_per_pixel: u8,
    pub depth: u8,
    pub big_endian: bool,
    pub true_colour: bool,
    pub red_max: u16,
    pub green_max: u16,
    pub blue_max: u16,
    pub red_shift: u8,
    pub green_shift: u8,
    pub blue_shift: u8,
}

impl PixelFormat {
    /// What the panel always asks for: 32 bits, little-endian, so a pixel's
    /// bytes are blue, green, red, unused — the window's own BGRA order.
    pub const BGRX: PixelFormat = PixelFormat {
        bits_per_pixel: 32,
        depth: 24,
        big_endian: false,
        true_colour: true,
        red_max: 255,
        green_max: 255,
        blue_max: 255,
        red_shift: 16,
        green_shift: 8,
        blue_shift: 0,
    };

    pub fn from_bytes(b: &[u8; 16]) -> Self {
        PixelFormat {
            bits_per_pixel: b[0],
            depth: b[1],
            big_endian: b[2] != 0,
            true_colour: b[3] != 0,
            red_max: u16::from_be_bytes([b[4], b[5]]),
            green_max: u16::from_be_bytes([b[6], b[7]]),
            blue_max: u16::from_be_bytes([b[8], b[9]]),
            red_shift: b[10],
            green_shift: b[11],
            blue_shift: b[12],
        }
    }

    pub fn to_bytes(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0] = self.bits_per_pixel;
        b[1] = self.depth;
        b[2] = u8::from(self.big_endian);
        b[3] = u8::from(self.true_colour);
        b[4..6].copy_from_slice(&self.red_max.to_be_bytes());
        b[6..8].copy_from_slice(&self.green_max.to_be_bytes());
        b[8..10].copy_from_slice(&self.blue_max.to_be_bytes());
        b[10] = self.red_shift;
        b[11] = self.green_shift;
        b[12] = self.blue_shift;
        b
    }
}

/// What the server says once the handshake is done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInit {
    pub width: u16,
    pub height: u16,
    pub format: PixelFormat,
    pub name: String,
}

impl ServerInit {
    pub fn read(r: &mut impl Read) -> io::Result<Self> {
        let mut head = [0u8; 24];
        r.read_exact(&mut head)?;
        let format = PixelFormat::from_bytes(head[4..20].try_into().expect("16 bytes"));
        let len = u32::from_be_bytes(head[20..24].try_into().expect("4 bytes")) as usize;
        if len > 1 << 16 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "a desktop name that long is no RFB server"));
        }
        let mut name = vec![0u8; len];
        r.read_exact(&mut name)?;
        Ok(ServerInit {
            width: u16::from_be_bytes([head[0], head[1]]),
            height: u16::from_be_bytes([head[2], head[3]]),
            format,
            name: String::from_utf8_lossy(&name).into_owned(),
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24 + self.name.len());
        out.extend_from_slice(&self.width.to_be_bytes());
        out.extend_from_slice(&self.height.to_be_bytes());
        out.extend_from_slice(&self.format.to_bytes());
        out.extend_from_slice(&(self.name.len() as u32).to_be_bytes());
        out.extend_from_slice(self.name.as_bytes());
        out
    }
}

/// Why a handshake ended.
#[derive(Debug)]
pub enum HandshakeError {
    Io(io::Error),
    /// The other end isn't an RFB server (or client).
    NotRfb,
    /// The server refused the connection, saying why.
    Refused(String),
    /// None of the security types offered is one this client speaks (Apple's
    /// own, VeNCrypt, …): their numbers.
    Unsupported(Vec<u8>),
    /// The server wants VNC authentication and no password was given.
    NeedsPassword,
    /// The password was wrong, or the server said no for another reason.
    AuthFailed(String),
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandshakeError::Io(e) => write!(f, "{e}"),
            HandshakeError::NotRfb => f.write_str("not a VNC server"),
            HandshakeError::Refused(why) if why.is_empty() => f.write_str("the VNC server refused the connection"),
            HandshakeError::Refused(why) => write!(f, "the VNC server refused the connection: {why}"),
            HandshakeError::Unsupported(types) => write!(f, "the VNC server offers no security type equipe speaks (it offers {types:?}; equipe speaks None and VNC password)"),
            HandshakeError::NeedsPassword => f.write_str("the VNC server wants a password and none is kept for it"),
            HandshakeError::AuthFailed(why) if why.is_empty() => f.write_str("the VNC server refused the password"),
            HandshakeError::AuthFailed(why) => write!(f, "the VNC server refused the password: {why}"),
        }
    }
}

impl std::error::Error for HandshakeError {}

impl From<io::Error> for HandshakeError {
    fn from(e: io::Error) -> Self {
        HandshakeError::Io(e)
    }
}

fn read_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}

/// A reason string: a length, then that many bytes (capped: it is only shown).
fn read_reason(r: &mut impl Read) -> io::Result<String> {
    let len = read_u32(r)? as usize;
    let mut text = vec![0u8; len.min(4096)];
    r.read_exact(&mut text)?;
    // Anything past the cap is read and dropped.
    io::copy(&mut r.take((len.saturating_sub(4096)) as u64), &mut io::sink())?;
    Ok(String::from_utf8_lossy(&text).trim().to_owned())
}

/// `RFB 003.008\n` → `(3, 8)`.
fn parse_version(v: &[u8; 12]) -> Option<(u32, u32)> {
    if &v[..4] != b"RFB " || v[7] != b'.' || v[11] != b'\n' {
        return None;
    }
    let num = |s: &[u8]| std::str::from_utf8(s).ok()?.parse::<u32>().ok();
    Some((num(&v[4..7])?, num(&v[8..11])?))
}

/// The protocol minor version to speak with a server that says `(major, minor)`:
/// 3.3, 3.7 or 3.8 (Apple's 3.889 and anything newer speak 3.8).
fn speak(major: u32, minor: u32) -> u32 {
    match (major, minor) {
        (3, 0..=6) => 3,
        (3, 7) => 7,
        _ => 8,
    }
}

/// Connect as a client: the version, the security (None when offered, else
/// VNC authentication with `password`), then `ClientInit` (`shared`: the
/// other viewers stay connected) and the server's `ServerInit`.
pub fn client_handshake<S: Read + Write>(s: &mut S, password: Option<&[u8]>, shared: bool) -> Result<ServerInit, HandshakeError> {
    let mut v = [0u8; 12];
    s.read_exact(&mut v)?;
    let (major, minor) = parse_version(&v).ok_or(HandshakeError::NotRfb)?;
    let minor = speak(major, minor);
    s.write_all(format!("RFB 003.{minor:03}\n").as_bytes())?;
    s.flush()?;
    let chosen = if minor == 3 {
        // 3.3: the server picks.
        match read_u32(s)? {
            0 => return Err(HandshakeError::Refused(read_reason(s)?)),
            t => u8::try_from(t).unwrap_or(0),
        }
    } else {
        let n = read_u8(s)?;
        if n == 0 {
            return Err(HandshakeError::Refused(read_reason(s)?));
        }
        let mut types = vec![0u8; usize::from(n)];
        s.read_exact(&mut types)?;
        let pick = if types.contains(&security::NONE) {
            security::NONE
        } else if types.contains(&security::VNC) {
            if password.is_none() {
                return Err(HandshakeError::NeedsPassword);
            }
            security::VNC
        } else {
            return Err(HandshakeError::Unsupported(types));
        };
        s.write_all(&[pick])?;
        s.flush()?;
        pick
    };
    match chosen {
        security::NONE => {
            // Only 3.8 says how None went.
            if minor == 8 {
                security_result(s, minor)?;
            }
        }
        security::VNC => {
            let password = password.ok_or(HandshakeError::NeedsPassword)?;
            let mut challenge = [0u8; 16];
            s.read_exact(&mut challenge)?;
            s.write_all(&crate::auth::response(password, &challenge))?;
            s.flush()?;
            security_result(s, minor)?;
        }
        other => return Err(HandshakeError::Unsupported(vec![other])),
    }
    s.write_all(&[u8::from(shared)])?;
    s.flush()?;
    Ok(ServerInit::read(s)?)
}

fn security_result<S: Read>(s: &mut S, minor: u32) -> Result<(), HandshakeError> {
    match read_u32(s)? {
        0 => Ok(()),
        _ if minor == 8 => Err(HandshakeError::AuthFailed(read_reason(s).unwrap_or_default())),
        _ => Err(HandshakeError::AuthFailed(String::new())),
    }
}

/// The server's side, as `equipe screen --stdio` plays it to the app after
/// it has signed in to the real server itself: version 3.8, security None
/// (SSH already did the signing in; the VNC password never leaves the
/// remote machine), then `init` — the real server's.
pub fn serve_handshake<S: Read + Write>(s: &mut S, init: &ServerInit) -> Result<(), HandshakeError> {
    s.write_all(b"RFB 003.008\n")?;
    s.flush()?;
    let mut v = [0u8; 12];
    s.read_exact(&mut v)?;
    let (major, minor) = parse_version(&v).ok_or(HandshakeError::NotRfb)?;
    let minor = speak(major, minor);
    if minor == 3 {
        s.write_all(&u32::from(security::NONE).to_be_bytes())?;
    } else {
        s.write_all(&[1, security::NONE])?;
        s.flush()?;
        if read_u8(s)? != security::NONE {
            return Err(HandshakeError::Unsupported(vec![]));
        }
        if minor == 8 {
            s.write_all(&0u32.to_be_bytes())?;
        }
    }
    s.flush()?;
    // ClientInit's shared flag: the bridge always joined the real server shared.
    read_u8(s)?;
    s.write_all(&init.to_bytes())?;
    s.flush()?;
    Ok(())
}

// ── client messages ──

pub fn set_pixel_format(format: &PixelFormat) -> Vec<u8> {
    let mut out = vec![client::SET_PIXEL_FORMAT, 0, 0, 0];
    out.extend_from_slice(&format.to_bytes());
    out
}

pub fn set_encodings(encodings: &[i32]) -> Vec<u8> {
    let mut out = vec![client::SET_ENCODINGS, 0];
    out.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
    for e in encodings {
        out.extend_from_slice(&e.to_be_bytes());
    }
    out
}

pub fn update_request(incremental: bool, x: u16, y: u16, w: u16, h: u16) -> [u8; 10] {
    let mut out = [client::UPDATE_REQUEST, u8::from(incremental), 0, 0, 0, 0, 0, 0, 0, 0];
    out[2..4].copy_from_slice(&x.to_be_bytes());
    out[4..6].copy_from_slice(&y.to_be_bytes());
    out[6..8].copy_from_slice(&w.to_be_bytes());
    out[8..10].copy_from_slice(&h.to_be_bytes());
    out
}

pub fn key_event(down: bool, keysym: u32) -> [u8; 8] {
    let mut out = [client::KEY, u8::from(down), 0, 0, 0, 0, 0, 0];
    out[4..8].copy_from_slice(&keysym.to_be_bytes());
    out
}

/// Pointer buttons, as the button mask's bits.
pub mod button {
    pub const LEFT: u8 = 1;
    pub const MIDDLE: u8 = 2;
    pub const RIGHT: u8 = 4;
    pub const WHEEL_UP: u8 = 8;
    pub const WHEEL_DOWN: u8 = 16;
    pub const WHEEL_LEFT: u8 = 32;
    pub const WHEEL_RIGHT: u8 = 64;
}

pub fn pointer_event(buttons: u8, x: u16, y: u16) -> [u8; 6] {
    let mut out = [client::POINTER, buttons, 0, 0, 0, 0];
    out[2..4].copy_from_slice(&x.to_be_bytes());
    out[4..6].copy_from_slice(&y.to_be_bytes());
    out
}

pub fn enable_continuous_updates(on: bool, x: u16, y: u16, w: u16, h: u16) -> [u8; 10] {
    let mut out = update_request(on, x, y, w, h);
    out[0] = client::ENABLE_CONTINUOUS_UPDATES;
    out
}

pub fn fence(flags: u32, payload: &[u8]) -> Vec<u8> {
    let payload = &payload[..payload.len().min(64)];
    let mut out = vec![client::FENCE, 0, 0, 0];
    out.extend_from_slice(&flags.to_be_bytes());
    out.push(payload.len() as u8);
    out.extend_from_slice(payload);
    out
}

/// The answer to a fence the server asked for: the same payload, the flags
/// this client honours (it handles messages in order, so blocking before and
/// after is what it does anyway), without the request bit.
pub fn fence_reply(flags: u32, payload: &[u8]) -> Option<Vec<u8>> {
    (flags & fence::REQUEST != 0).then(|| fence(flags & (fence::BLOCK_BEFORE | fence::BLOCK_AFTER | fence::SYNC_NEXT), payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Both directions of a connection, scripted: what the far end says, and
    /// what was written to it.
    struct Script {
        input: VecDeque<u8>,
        output: Vec<u8>,
    }

    impl Script {
        fn new(input: Vec<u8>) -> Self {
            Script { input: input.into(), output: Vec::new() }
        }
    }

    impl Read for Script {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Script {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn init() -> ServerInit {
        ServerInit { width: 1440, height: 900, format: PixelFormat::BGRX, name: "tenki:1".into() }
    }

    #[test]
    fn a_38_server_with_none_is_joined_shared() {
        let mut said = b"RFB 003.008\n".to_vec();
        said.extend([2, 30, 1]); // Apple's, then None
        said.extend(0u32.to_be_bytes());
        said.extend(init().to_bytes());
        let mut s = Script::new(said);
        assert_eq!(client_handshake(&mut s, None, true).unwrap(), init());
        let mut want = b"RFB 003.008\n".to_vec();
        want.extend([1, 1]);
        assert_eq!(s.output, want, "version, None, shared");
    }

    #[test]
    fn a_password_server_gets_the_des_answer() {
        let challenge = [7u8; 16];
        let mut said = b"RFB 003.008\n".to_vec();
        said.extend([1, 2]);
        said.extend(challenge);
        said.extend(0u32.to_be_bytes());
        said.extend(init().to_bytes());
        let mut s = Script::new(said);
        client_handshake(&mut s, Some(b"secret"), true).unwrap();
        let mut want = b"RFB 003.008\n".to_vec();
        want.push(2);
        want.extend(crate::auth::response(b"secret", &challenge));
        want.push(1);
        assert_eq!(s.output, want);
    }

    #[test]
    fn a_wrong_password_and_a_refusal_say_why() {
        let mut said = b"RFB 003.008\n".to_vec();
        said.extend([1, 2]);
        said.extend([0u8; 16]);
        said.extend(1u32.to_be_bytes());
        said.extend(14u32.to_be_bytes());
        said.extend(b"Authentication");
        match client_handshake(&mut Script::new(said), Some(b"x"), true) {
            Err(HandshakeError::AuthFailed(why)) => assert_eq!(why, "Authentication"),
            other => panic!("{other:?}"),
        }
        let mut said = b"RFB 003.008\n".to_vec();
        said.push(0);
        said.extend(9u32.to_be_bytes());
        said.extend(b"Too many!");
        assert!(matches!(client_handshake(&mut Script::new(said), None, true), Err(HandshakeError::Refused(w)) if w == "Too many!"));
        let mut said = b"RFB 003.008\n".to_vec();
        said.extend([1, 2]);
        assert!(matches!(client_handshake(&mut Script::new(said), None, true), Err(HandshakeError::NeedsPassword)));
        let mut said = b"RFB 003.008\n".to_vec();
        said.extend([2, 30, 19]);
        assert!(matches!(client_handshake(&mut Script::new(said), None, true), Err(HandshakeError::Unsupported(t)) if t == vec![30, 19]));
        assert!(matches!(client_handshake(&mut Script::new(b"SSH-2.0-Open".to_vec()), None, true), Err(HandshakeError::NotRfb)));
    }

    #[test]
    fn older_and_apples_versions_are_spoken_to_in_theirs() {
        // 3.3: the server picks None, and says nothing more about it.
        let mut said = b"RFB 003.003\n".to_vec();
        said.extend(1u32.to_be_bytes());
        said.extend(init().to_bytes());
        let mut s = Script::new(said);
        client_handshake(&mut s, None, true).unwrap();
        assert_eq!(&s.output[..12], b"RFB 003.003\n");
        // Apple's 3.889 is answered with 3.8.
        let mut said = b"RFB 003.889\n".to_vec();
        said.extend([1, 1]);
        said.extend(0u32.to_be_bytes());
        said.extend(init().to_bytes());
        let mut s = Script::new(said);
        client_handshake(&mut s, None, true).unwrap();
        assert_eq!(&s.output[..12], b"RFB 003.008\n");
    }

    #[test]
    fn the_bridge_plays_a_none_server_to_the_app() {
        // The app's side of it, through the client handshake itself.
        let mut app_said = b"RFB 003.008\n".to_vec();
        app_said.extend([security::NONE, 1]);
        let mut bridge = Script::new(app_said);
        serve_handshake(&mut bridge, &init()).unwrap();
        let mut app = Script::new(bridge.output.clone());
        assert_eq!(client_handshake(&mut app, None, true).unwrap(), init());
    }

    #[test]
    fn messages_are_laid_out_as_the_rfc_says() {
        assert_eq!(key_event(true, 0xff0d), [4, 1, 0, 0, 0, 0, 0xff, 0x0d]);
        assert_eq!(pointer_event(button::LEFT, 300, 2), [5, 1, 1, 44, 0, 2]);
        assert_eq!(update_request(true, 0, 0, 1440, 900), [3, 1, 0, 0, 0, 0, 5, 160, 3, 132]);
        assert_eq!(set_encodings(&[encoding::TIGHT, encoding::LAST_RECT]), vec![2, 0, 0, 2, 0, 0, 0, 7, 0xff, 0xff, 0xff, 0x20]);
        assert_eq!(enable_continuous_updates(true, 0, 0, 1, 1)[..2], [150, 1]);
        assert_eq!(encoding::quality(9), -23);
        assert_eq!(encoding::compress(0), -256);
        let reply = fence_reply(fence::REQUEST | fence::BLOCK_BEFORE | 0x40, b"abc").unwrap();
        assert_eq!(reply, vec![248, 0, 0, 0, 0, 0, 0, 1, 3, b'a', b'b', b'c']);
        assert_eq!(fence_reply(fence::BLOCK_BEFORE, b""), None, "a reply to ours is not answered");
        assert_eq!(set_pixel_format(&PixelFormat::BGRX), vec![0, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]);
    }
}
