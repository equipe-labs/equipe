//! A small RFB (VNC) client: what the desktop app's Screen panel speaks to see
//! another machine's screen and, when the person takes control, to drive it.
//!
//! - [`wire`]: the handshake (RFB 3.3, 3.7 and 3.8; security None and VNC
//!   authentication), the server's side of it for `equipe screen --stdio`, and
//!   the client's messages.
//! - [`decode`]: the server's messages, and a framebuffer the rectangles are
//!   drawn into — Raw, CopyRect, Tight (zlib and JPEG) and ZRLE, with desktop
//!   resizes, fences and continuous updates.
//! - [`keysym`]: X keysyms for the person's keys and for typed text (any
//!   Unicode character, so Korean arrives as the syllables composed on the Mac).
//! - [`auth`]: VNC authentication's DES and the `vncpasswd` file.
//!
//! Blocking `std::io` throughout: a session reads on a thread of its own.
//! See docs/DESKTOP.md, "The Screen panel".

pub mod auth;
pub mod decode;
pub mod keysym;
pub mod wire;

pub use decode::{Decoder, Event, Framebuffer, Rect};
pub use wire::{HandshakeError, PixelFormat, ServerInit};
