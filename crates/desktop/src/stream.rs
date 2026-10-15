//! The browser panel's own line to the daemon: a second connection beside the
//! one that carries commands and events, which after a `browser_stream` request
//! carries pictures of the page the panel watches and the person's input to it
//! (`equipe_protocol::stream`). On this Mac it is the socket again; to another
//! machine, a second `ssh … equipe attach --stdio` over the same shared SSH
//! connection.
//!
//! Frames are decoded here, off the window's thread, into the BGRA pictures the
//! window draws; the window gives credit back once it has drawn one, so a slow
//! line gets fewer, newer frames rather than a queue of old ones. When the line
//! drops it is tried again, and what was watched is asked for again.

use std::sync::Arc;
use std::time::Duration;

use equipe_core::protocol::rpc::{self, Message};
use equipe_core::protocol::stream::{self, FrameHeader, StreamHello, StreamIn};
use gpui_kit::RenderImage;
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::ssh::Launcher;
use crate::target::Endpoint;

/// A picture of the watched page, ready to draw.
pub struct Picture {
    pub header: FrameHeader,
    pub image: Arc<RenderImage>,
}

/// What the stream tells the panel.
pub enum Update {
    Picture(Picture),
    /// Connected (`true`), or lost and trying again.
    Line(bool),
}

/// The panel's end. Dropping it closes the line.
pub struct Link {
    to: UnboundedSender<StreamIn>,
}

impl Link {
    pub fn send(&self, message: StreamIn) {
        let _ = self.to.send(message);
    }

    /// A way to send from anywhere: credit, after a frame is drawn.
    pub fn sender(&self) -> UnboundedSender<StreamIn> {
        self.to.clone()
    }
}

/// Start the line to `endpoint`'s daemon on a thread of its own.
pub fn spawn(endpoint: Endpoint) -> (Link, UnboundedReceiver<Update>) {
    let (to_tx, to_rx) = mpsc::unbounded_channel();
    let (up_tx, up_rx) = mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("equipe-browser-stream".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
            runtime.block_on(run(endpoint, to_rx, up_tx));
        })
        .expect("spawn the browser stream thread");
    (Link { to: to_tx }, up_rx)
}

/// JPEG quality to ask for: lower over a network.
fn quality(endpoint: &Endpoint) -> u8 {
    if endpoint.is_remote() { 50 } else { 70 }
}

async fn run(endpoint: Endpoint, mut from_panel: UnboundedReceiver<StreamIn>, updates: UnboundedSender<Update>) {
    let mut wait = Duration::from_millis(300);
    // What the panel watches, asked for again on every new line.
    let mut watching: Option<StreamIn> = None;
    loop {
        match connect(&endpoint).await {
            Ok((reader, writer, _process)) => {
                wait = Duration::from_millis(300);
                if updates.send(Update::Line(true)).is_err() {
                    return;
                }
                let ended = follow(reader, writer, &mut from_panel, &updates, &mut watching).await;
                if ended || updates.send(Update::Line(false)).is_err() {
                    return;
                }
            }
            Err(_) => {
                if updates.is_closed() {
                    return;
                }
            }
        }
        // Wait to try again; what the panel says meanwhile is kept if it is what to watch.
        let nap = tokio::time::sleep(wait);
        tokio::pin!(nap);
        loop {
            tokio::select! {
                () = &mut nap => break,
                message = from_panel.recv() => match message {
                    None => return,
                    Some(m @ (StreamIn::Watch { .. } | StreamIn::Unwatch)) => watching = Some(m),
                    Some(_) => {}
                },
            }
        }
        wait = (wait * 2).min(Duration::from_secs(5));
    }
}

type Reader = Box<dyn AsyncRead + Unpin + Send>;
type Writer = Box<dyn AsyncWrite + Unpin + Send>;

/// A connection that has become a stream: its two halves, and the `ssh`
/// behind it (kept alive with them).
async fn connect(endpoint: &Endpoint) -> std::io::Result<(Reader, Writer, Option<crate::ssh::Process>)> {
    let (reader, mut writer, process): (Reader, Writer, _) = match endpoint {
        Endpoint::Local { socket } => {
            let (r, w) = tokio::net::UnixStream::connect(socket).await?.into_split();
            (Box::new(r), Box::new(w), None)
        }
        Endpoint::Ssh(host) => {
            // The shared connection the window's own line opened: no new sign-in.
            let (process, stdout, stdin) = Launcher::system(None).attach(host)?;
            (Box::new(stdout), Box::new(stdin), Some(process))
        }
    };
    let mut reader = BufReader::new(reader);
    let request = Message::request(1, rpc::BROWSER_STREAM, StreamHello { quality: Some(quality(endpoint)) });
    writer.write_all(request.to_line().as_bytes()).await?;
    writer.flush().await?;
    // The bridge's `attached` line comes first over SSH; then the answer.
    let answered = tokio::time::timeout(Duration::from_secs(20), async {
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the daemon closed the stream"));
            }
            if let Ok(message) = Message::from_line(&line)
                && message.id == Some(1)
                && message.method.is_none()
            {
                return if message.result_as::<bool>() == Some(true) { Ok(()) } else { Err(std::io::Error::other("the daemon has no browser stream")) };
            }
        }
    })
    .await;
    match answered {
        Ok(Ok(())) => Ok((Box::new(reader), writer, process)),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "no answer")),
    }
}

/// Carry the panel's messages out and pictures in until the line ends
/// (`false`) or the panel goes away (`true`).
async fn follow(
    mut reader: Reader,
    mut writer: Writer,
    from_panel: &mut UnboundedReceiver<StreamIn>,
    updates: &UnboundedSender<Update>,
    watching: &mut Option<StreamIn>,
) -> bool {
    if let Some(watch) = watching.clone()
        && writer.write_all(&stream::encode_json(&watch)).await.is_err()
    {
        return false;
    }
    let mut decoder = stream::Decoder::default();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        tokio::select! {
            message = from_panel.recv() => {
                let Some(message) = message else { return true };
                if matches!(message, StreamIn::Watch { .. } | StreamIn::Unwatch) {
                    *watching = Some(message.clone());
                }
                if writer.write_all(&stream::encode_json(&message)).await.is_err() {
                    return false;
                }
            }
            read = reader.read(&mut buf) => {
                let n = match read {
                    Ok(0) | Err(_) => return false,
                    Ok(n) => n,
                };
                decoder.push(&buf[..n]);
                loop {
                    match decoder.message() {
                        Ok(Some((stream::FRAME, payload))) => {
                            let picture = tokio::task::spawn_blocking(move || decode(&payload)).await.ok().flatten();
                            match picture {
                                Some(picture) => {
                                    if updates.send(Update::Picture(picture)).is_err() {
                                        return true;
                                    }
                                }
                                // Unreadable: its credit comes back, or the stream would stall.
                                None => {
                                    if writer.write_all(&stream::encode_json(&StreamIn::Credit { n: 1 })).await.is_err() {
                                        return false;
                                    }
                                }
                            }
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => break,
                        Err(_) => return false,
                    }
                }
            }
        }
    }
}

/// A frame message, as the window draws it: BGRA.
fn decode(payload: &[u8]) -> Option<Picture> {
    let (header, jpeg) = stream::parse_frame(payload)?;
    let mut rgba = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).ok()?.into_rgba8();
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let image = Arc::new(RenderImage::new(vec![image::Frame::new(rgba)]));
    Some(Picture { header, image })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_decodes_to_the_windows_byte_order() {
        // A 2×1 JPEG of pure red.
        let mut jpeg = Vec::new();
        let red = image::RgbImage::from_pixel(2, 1, image::Rgb([255, 0, 0]));
        image::DynamicImage::ImageRgb8(red).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        let header = FrameHeader { page: 1, seq: 1, w: 2, h: 1, css_w: 2., css_h: 1., sx: 0., sy: 0., top: 0. };
        let picture = decode(&stream::frame_payload(&header, &jpeg)).unwrap();
        let bytes = picture.image.as_bytes(0).unwrap();
        // Blue first, red third.
        assert!(bytes[0] < 40 && bytes[2] > 200, "{:?}", &bytes[..4]);
        assert_eq!(picture.header, header);
    }

    #[test]
    fn a_far_machine_gets_lighter_pictures() {
        assert_eq!(quality(&Endpoint::Local { socket: "/x".into() }), 70);
        assert_eq!(quality(&Endpoint::Ssh(crate::target::SshHost::parse("me@far").unwrap())), 50);
    }
}
