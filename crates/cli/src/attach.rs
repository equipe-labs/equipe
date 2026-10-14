//! `equipe attach --stdio`: the daemon's socket on standard input and output,
//! so a screen on another machine reaches it through `ssh <host> equipe attach
//! --stdio` — SSH does the sign-in, equipe adds none of its own.
//!
//! The first line out is an `attached` notification ([`rpc::Attached`]): which
//! equipe answered and which daemon it reached. After it, the bytes go both
//! ways untouched — the same JSON-RPC lines a screen on the daemon's own
//! machine reads from `~/.equipe/equipe.sock`. See docs/DESKTOP.md, "Remote".

use std::io;
use std::process::ExitCode;

use equipe_core::gateway;
use equipe_core::protocol::rpc::{self, Attached, Message};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;

use crate::daemon;

/// The exit status when no daemon could be reached (`EX_UNAVAILABLE`), so the
/// screen can tell it from SSH's own 255 and a missing `equipe`'s 127.
pub const UNAVAILABLE: u8 = 69;

/// Bridge until either side leaves. Starts the daemon when none answers,
/// unless `no_start`. Never returns: the process exits, because a read
/// pending on standard input would otherwise hold it open.
pub async fn stdio(no_start: bool) -> ! {
    let code = match bridge(no_start).await {
        Ok(()) => 0,
        Err(why) => {
            eprintln!("equipe attach: {why}");
            UNAVAILABLE
        }
    };
    std::process::exit(code.into())
}

async fn bridge(no_start: bool) -> io::Result<()> {
    let socket = daemon::socket();
    let started = if no_start {
        if !gateway::is_running(&socket).await {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "the daemon isn't running — start it with `equipe daemon start`"));
        }
        false
    } else {
        daemon::ensure_running().await?
    };
    let version = gateway::version(&socket).await?;
    let attached = Attached {
        bridge: daemon::build_id(),
        protocol: version.as_ref().map_or(rpc::PROTOCOL, |v| v.protocol),
        daemon: version.map(|v| v.version),
        started,
    };
    let stream = UnixStream::connect(&socket).await?;
    let (mut from_daemon, mut to_daemon) = stream.into_split();

    let mut stdout = tokio::io::stdout();
    stdout.write_all(Message::notification(rpc::ATTACHED, &attached).to_line().as_bytes()).await?;
    stdout.flush().await?;

    // The screen's lines go to the daemon; when it hangs up, the daemon sees
    // the end of the stream and lets the connection go, which ends the loop below.
    tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let _ = tokio::io::copy(&mut stdin, &mut to_daemon).await;
        let _ = to_daemon.shutdown().await;
    });
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = from_daemon.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        stdout.write_all(&buf[..n]).await?;
        stdout.flush().await?;
    }
}

/// What `equipe attach` without `--stdio` says.
pub fn usage() -> ExitCode {
    eprintln!("equipe attach --stdio connects standard input and output to the daemon, for a screen on another machine:\n  ssh <host> equipe attach --stdio\nTo open the console here, run equipe.");
    ExitCode::from(2)
}
