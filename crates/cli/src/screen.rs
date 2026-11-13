//! `equipe screen --stdio`: this machine's VNC server on standard input and
//! output, so the desktop app on another machine shows this screen through
//! `ssh <host> equipe screen --stdio` — the same SSH host and sign-in as
//! `equipe attach --stdio`, and no port of its own.
//!
//! The first line out is JSON: `{"screen":1,"ok":true,…}` with the server's
//! size and name, or `{"screen":1,"error":"<kind>","detail":"…"}` when there is
//! no screen to show (then the process exits [`NO_SCREEN`]). After an `ok`
//! line the bridge plays an RFB 3.8 server with no security of its own — SSH
//! did the signing in — in front of the real server, which it has already
//! joined (shared: other viewers stay) and signed in to. A VNC password stays
//! on this machine, in `<home>/screen/passwd` (0600), and is never sent on.
//!
//! Only a server on this machine is reached: a Unix socket, or a loopback
//! address. See docs/DESKTOP.md, "The Screen panel".

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use equipe_rfb::wire::{self, HandshakeError, ServerInit};
use serde_json::json;

/// The exit status when there is no screen to show, as `equipe attach`'s
/// "no daemon" (`EX_UNAVAILABLE`).
pub const NO_SCREEN: u8 = 69;

/// Where the VNC server is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Server {
    Unix(PathBuf),
    /// A loopback address and port.
    Tcp(String, u16),
}

impl std::fmt::Display for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Server::Unix(path) => write!(f, "unix:{}", path.display()),
            Server::Tcp(host, port) if host.contains(':') => write!(f, "[{host}]:{port}"),
            Server::Tcp(host, port) => write!(f, "{host}:{port}"),
        }
    }
}

/// `unix:/path`, `/path`, `127.0.0.1:5901`, `[::1]:5900`, `localhost:5900`,
/// or `:1` (display 1, port 5901). Anything not on this machine is refused:
/// the screen is reached through SSH, never across the network.
pub fn parse(text: &str) -> Result<Server, String> {
    let text = text.trim();
    if let Some(path) = text.strip_prefix("unix:").or(text.starts_with('/').then_some(text)) {
        return Ok(Server::Unix(PathBuf::from(path)));
    }
    if let Some(display) = text.strip_prefix(':') {
        let n: u16 = display.parse().map_err(|_| format!("`{text}`: a display is `:<number>`"))?;
        return Ok(Server::Tcp("127.0.0.1".into(), 5900u16.saturating_add(n)));
    }
    let (host, port) = match text.strip_prefix('[') {
        Some(rest) => {
            let (host, port) = rest.split_once("]:").ok_or_else(|| format!("`{text}`: write an IPv6 address as `[::1]:5900`"))?;
            (host, port)
        }
        None => text.rsplit_once(':').ok_or_else(|| format!("`{text}`: give the port, as `127.0.0.1:5900`"))?,
    };
    let port: u16 = port.parse().map_err(|_| format!("`{text}`: `{port}` is not a port"))?;
    let loopback = host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    if !loopback {
        return Err(format!("`{text}` is not on this machine: equipe only reaches a VNC server on a Unix socket or a loopback address (127.0.0.1, ::1), and the app reaches it through SSH"));
    }
    Ok(Server::Tcp(host.to_owned(), port))
}

/// equipe's own folder for the screen: `<home>/screen`.
pub fn dir(home: &Path) -> PathBuf {
    home.join("screen")
}

/// Where the VNC password is kept (TigerVNC's `vncpasswd` format).
pub fn passwd_path(home: &Path) -> PathBuf {
    dir(home).join("passwd")
}

/// The server to reach, and what said so: `--vnc`, `$EQUIPE_SCREEN_VNC`,
/// `<home>/screen/vnc` (its first line), else `<home>/screen/vnc.sock` when it
/// exists, else `127.0.0.1:5900` (macOS Screen Sharing, x11vnc, GNOME's).
pub fn locate(home: &Path, flag: Option<&str>) -> Result<(Server, &'static str), String> {
    if let Some(text) = flag {
        return parse(text).map(|s| (s, "--vnc"));
    }
    if let Ok(text) = std::env::var("EQUIPE_SCREEN_VNC")
        && !text.trim().is_empty()
    {
        return parse(&text).map(|s| (s, "EQUIPE_SCREEN_VNC"));
    }
    let file = dir(home).join("vnc");
    if let Ok(text) = std::fs::read_to_string(&file)
        && let Some(line) = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'))
    {
        return parse(line).map(|s| (s, "screen/vnc"));
    }
    let socket = dir(home).join("vnc.sock");
    if socket.exists() {
        return Ok((Server::Unix(socket), "screen/vnc.sock"));
    }
    Ok((Server::Tcp("127.0.0.1".into(), 5900), "default"))
}

/// The kept VNC password, if there is one. A file others can read is refused.
fn password(home: &Path) -> Result<Option<Vec<u8>>, String> {
    let path = passwd_path(home);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).map(|m| m.permissions().mode()).unwrap_or(0);
        if mode & 0o077 != 0 {
            return Err(format!("{} can be read by others: `chmod 600` it (or run `equipe screen password` again)", path.display()));
        }
    }
    equipe_rfb::auth::read_file(&bytes).map(Some).ok_or_else(|| format!("{} is not a vncpasswd file", path.display()))
}

/// Keep `password` for the VNC server, 0600 in a 0700 folder.
pub fn save_password(home: &Path, password: &[u8]) -> io::Result<PathBuf> {
    let dir = dir(home);
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        let path = passwd_path(home);
        let _ = std::fs::remove_file(&path);
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
        file.write_all(&equipe_rfb::auth::file(password))?;
        Ok(path)
    }
    #[cfg(not(unix))]
    {
        let path = passwd_path(home);
        std::fs::write(&path, equipe_rfb::auth::file(password))?;
        Ok(path)
    }
}

/// A connection to the server, either kind.
#[derive(Debug)]
enum Conn {
    Tcp(std::net::TcpStream),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
}

impl Conn {
    fn open(server: &Server) -> io::Result<Conn> {
        match server {
            Server::Tcp(host, port) => {
                let mut last = io::Error::new(io::ErrorKind::NotFound, format!("{host} has no address"));
                for addr in std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), *port))? {
                    // Resolved names are checked again: only this machine.
                    if !addr.ip().is_loopback() {
                        continue;
                    }
                    match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
                        Ok(s) => {
                            let _ = s.set_nodelay(true);
                            return Ok(Conn::Tcp(s));
                        }
                        Err(e) => last = e,
                    }
                }
                Err(last)
            }
            #[cfg(unix)]
            Server::Unix(path) => Ok(Conn::Unix(std::os::unix::net::UnixStream::connect(path)?)),
            #[cfg(not(unix))]
            Server::Unix(_) => Err(io::Error::new(io::ErrorKind::Unsupported, "no Unix sockets here")),
        }
    }

    fn try_clone(&self) -> io::Result<Conn> {
        Ok(match self {
            Conn::Tcp(s) => Conn::Tcp(s.try_clone()?),
            #[cfg(unix)]
            Conn::Unix(s) => Conn::Unix(s.try_clone()?),
        })
    }

    fn set_timeout(&self, wait: Option<Duration>) {
        let _ = match self {
            Conn::Tcp(s) => s.set_read_timeout(wait),
            #[cfg(unix)]
            Conn::Unix(s) => s.set_read_timeout(wait),
        };
    }

    fn shutdown_write(&self) {
        let _ = match self {
            Conn::Tcp(s) => s.shutdown(std::net::Shutdown::Write),
            #[cfg(unix)]
            Conn::Unix(s) => s.shutdown(std::net::Shutdown::Write),
        };
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Conn::Tcp(s) => s.read(buf),
            #[cfg(unix)]
            Conn::Unix(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Conn::Tcp(s) => s.write(buf),
            #[cfg(unix)]
            Conn::Unix(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Why there is no screen to show: a kind the app keys its words on, and the detail.
#[derive(Debug)]
pub struct NoScreen {
    pub kind: &'static str,
    pub detail: String,
}

/// Reach the server, sign in, join it shared.
fn join(home: &Path, server: &Server) -> Result<(Conn, ServerInit), NoScreen> {
    let mut conn = Conn::open(server).map_err(|e| NoScreen {
        kind: "no_server",
        detail: format!("no VNC server answers at {server} ({e})"),
    })?;
    // A server that takes the connection and says nothing is no server.
    conn.set_timeout(Some(Duration::from_secs(10)));
    let password = password(home).map_err(|detail| NoScreen { kind: "password_file", detail })?;
    let init = wire::client_handshake(&mut conn, password.as_deref(), true).map_err(|e| NoScreen {
        kind: match &e {
            HandshakeError::Io(_) | HandshakeError::NotRfb => "no_server",
            HandshakeError::Refused(_) => "refused",
            HandshakeError::Unsupported(_) => "unsupported",
            HandshakeError::NeedsPassword => "needs_password",
            HandshakeError::AuthFailed(_) => "auth_failed",
        },
        detail: format!("{server}: {e}"),
    })?;
    conn.set_timeout(None);
    Ok((conn, init))
}

/// Standard input and output as one stream, for the handshake.
struct Stdio {
    input: io::Stdin,
    output: std::fs::File,
}

impl Read for Stdio {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.input.read(buf)
    }
}

impl Write for Stdio {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.output.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

/// Standard output, unbuffered: `io::stdout` holds bytes back until a newline.
fn raw_stdout() -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsFd as _, OwnedFd};
        let fd: OwnedFd = io::stdout().as_fd().try_clone_to_owned()?;
        Ok(std::fs::File::from(fd))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsHandle as _, OwnedHandle};
        let handle: OwnedHandle = io::stdout().as_handle().try_clone_to_owned()?;
        Ok(std::fs::File::from(handle))
    }
}

/// The first line: what the app is told before any RFB.
pub fn ok_line(server: &Server, init: &ServerInit) -> String {
    json!({
        "screen": 1,
        "ok": true,
        "bridge": crate::daemon::build_id(),
        "os": std::env::consts::OS,
        "server": server.to_string(),
        "width": init.width,
        "height": init.height,
        "name": init.name,
    })
    .to_string()
}

pub fn error_line(why: &NoScreen) -> String {
    json!({ "screen": 1, "error": why.kind, "detail": why.detail, "os": std::env::consts::OS }).to_string()
}

/// Bridge until either side leaves. Never returns: a read pending on
/// standard input would otherwise hold the process open.
pub fn stdio(vnc: Option<&str>) -> ! {
    let home = equipe_providers::credentials::equipe_home();
    let code = match bridge(&home, vnc) {
        Ok(()) => 0,
        Err(why) => {
            eprintln!("equipe screen: {}", why.detail);
            NO_SCREEN
        }
    };
    std::process::exit(code.into())
}

fn bridge(home: &Path, vnc: Option<&str>) -> Result<(), NoScreen> {
    let mut out = raw_stdout().map_err(|e| NoScreen { kind: "io", detail: e.to_string() })?;
    let say = |out: &mut std::fs::File, line: String| {
        let _ = out.write_all(format!("{line}\n").as_bytes());
        let _ = out.flush();
    };
    let joined = locate(home, vnc).map_err(|detail| NoScreen { kind: "config", detail }).and_then(|(server, _)| join(home, &server).map(|(c, i)| (server, c, i)));
    let (server, conn, init) = match joined {
        Ok(joined) => joined,
        Err(why) => {
            say(&mut out, error_line(&why));
            return Err(why);
        }
    };
    say(&mut out, ok_line(&server, &init));
    let mut app = Stdio { input: io::stdin(), output: out };
    wire::serve_handshake(&mut app, &init).map_err(|e| NoScreen { kind: "io", detail: format!("the app's side: {e}") })?;
    let Stdio { input, output: mut out } = app;

    // The app's messages to the server; when it hangs up, so does the bridge.
    let mut to_server = conn.try_clone().map_err(|e| NoScreen { kind: "io", detail: e.to_string() })?;
    std::thread::spawn(move || {
        let mut input = input.lock();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match input.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if to_server.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
        to_server.shutdown_write();
        std::process::exit(0);
    });
    let mut from_server = conn;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        match from_server.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                if out.write_all(&buf[..n]).and_then(|()| out.flush()).is_err() {
                    return Ok(());
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(NoScreen { kind: "io", detail: format!("the VNC server: {e}") }),
        }
    }
}

/// `equipe screen` (status): where the bridge would go, and whether it gets in.
pub fn status(vnc: Option<&str>) -> ExitCode {
    let home = equipe_providers::credentials::equipe_home();
    match locate(&home, vnc) {
        Err(why) => {
            eprintln!("{why}");
            ExitCode::from(2)
        }
        Ok((server, from)) => {
            println!("VNC server: {server} (from {from})");
            let kept = passwd_path(&home);
            println!("Password:   {}", if kept.exists() { format!("kept in {}", kept.display()) } else { "none kept".into() });
            match join(&home, &server) {
                Ok((_, init)) => {
                    println!("Screen:     {} × {} — {:?}", init.width, init.height, init.name);
                    println!("The desktop app attached to this machine over SSH can show it (Screen panel).");
                    ExitCode::SUCCESS
                }
                Err(why) => {
                    println!("No screen:  {}", why.detail);
                    println!("See docs/DESKTOP.md, \"The Screen panel\", for setting up a VNC server on this machine.");
                    ExitCode::from(NO_SCREEN)
                }
            }
        }
    }
}

/// `equipe screen password`: keep the VNC server's password (read from
/// standard input, one line), for servers that ask for one.
pub fn set_password() -> ExitCode {
    let home = equipe_providers::credentials::equipe_home();
    let mut line = String::new();
    if std::io::IsTerminal::is_terminal(&io::stdin()) {
        eprintln!("The VNC server's password (only its first 8 characters count; it isn't shown):");
        match dialoguer::Password::new().interact() {
            Ok(p) => line = p,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    } else if io::stdin().read_line(&mut line).is_err() {
        return ExitCode::FAILURE;
    }
    let password = line.trim_end_matches(['\r', '\n']);
    if password.is_empty() {
        eprintln!("No password given; nothing kept.");
        return ExitCode::from(2);
    }
    match save_password(&home, password.as_bytes()) {
        Ok(path) => {
            println!("Kept in {} (0600). It stays on this machine.", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_machines_servers_are_reached() {
        assert_eq!(parse("unix:/run/vnc.sock").unwrap(), Server::Unix("/run/vnc.sock".into()));
        assert_eq!(parse("/home/me/.equipe/screen/vnc.sock").unwrap(), Server::Unix("/home/me/.equipe/screen/vnc.sock".into()));
        assert_eq!(parse("127.0.0.1:5902").unwrap(), Server::Tcp("127.0.0.1".into(), 5902));
        assert_eq!(parse("localhost:5900").unwrap(), Server::Tcp("localhost".into(), 5900));
        assert_eq!(parse("[::1]:5901").unwrap(), Server::Tcp("::1".into(), 5901));
        assert_eq!(parse(":1").unwrap(), Server::Tcp("127.0.0.1".into(), 5901));
        assert_eq!(parse("[::1]:5901").unwrap().to_string(), "[::1]:5901");
        for far in ["10.0.0.4:5900", "example.com:5900", "0.0.0.0:5900", "[2001:db8::1]:5900"] {
            assert!(parse(far).unwrap_err().contains("not on this machine"), "{far}");
        }
        assert!(parse("127.0.0.1").is_err());
        assert!(parse("127.0.0.1:vnc").is_err());
    }

    #[test]
    fn the_server_is_found_from_the_flag_the_file_or_the_socket() {
        let home = std::env::temp_dir().join(format!("eq-screen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(dir(&home)).unwrap();
        // SAFETY-free: the variable is only read here, and these tests don't set it.
        if std::env::var("EQUIPE_SCREEN_VNC").is_ok() {
            return;
        }
        assert_eq!(locate(&home, None).unwrap(), (Server::Tcp("127.0.0.1".into(), 5900), "default"));
        std::fs::write(dir(&home).join("vnc.sock"), "").unwrap();
        assert_eq!(locate(&home, None).unwrap().1, "screen/vnc.sock");
        std::fs::write(dir(&home).join("vnc"), "# x0vncserver on :1\n127.0.0.1:5902\n").unwrap();
        assert_eq!(locate(&home, None).unwrap(), (Server::Tcp("127.0.0.1".into(), 5902), "screen/vnc"));
        assert_eq!(locate(&home, Some(":3")).unwrap(), (Server::Tcp("127.0.0.1".into(), 5903), "--vnc"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn the_password_is_kept_private_and_refused_when_it_is_not() {
        use std::os::unix::fs::PermissionsExt as _;
        let home = std::env::temp_dir().join(format!("eq-screen-pw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(password(&home).unwrap(), None);
        let path = save_password(&home, b"hunter22").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(dir(&home)).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(password(&home).unwrap().unwrap(), b"hunter22");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(password(&home).unwrap_err().contains("chmod 600"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_first_line_says_what_the_app_needs_and_nothing_secret() {
        let init = ServerInit { width: 1440, height: 900, format: equipe_rfb::PixelFormat::BGRX, name: "tenki:1".into() };
        let line: serde_json::Value = serde_json::from_str(&ok_line(&Server::Unix("/h/.equipe/screen/vnc.sock".into()), &init)).unwrap();
        assert_eq!(line["ok"], true);
        assert_eq!(line["width"], 1440);
        assert_eq!(line["server"], "unix:/h/.equipe/screen/vnc.sock");
        let line: serde_json::Value = serde_json::from_str(&error_line(&NoScreen { kind: "no_server", detail: "nothing at 127.0.0.1:5900".into() })).unwrap();
        assert_eq!(line["error"], "no_server");
        assert!(line.get("ok").is_none());
    }

    /// The whole bridge, through a stand-in VNC server that wants a password.
    #[cfg(unix)]
    #[test]
    fn a_password_server_is_joined_with_the_kept_password() {
        let home = std::env::temp_dir().join(format!("eq-screen-join-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        save_password(&home, b"s3cret").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(b"RFB 003.008\n").unwrap();
            let mut v = [0u8; 12];
            s.read_exact(&mut v).unwrap();
            s.write_all(&[1, 2]).unwrap();
            let mut pick = [0u8; 1];
            s.read_exact(&mut pick).unwrap();
            let challenge = [3u8; 16];
            s.write_all(&challenge).unwrap();
            let mut answer = [0u8; 16];
            s.read_exact(&mut answer).unwrap();
            let ok = answer == equipe_rfb::auth::response(b"s3cret", &challenge);
            s.write_all(&u32::from(!ok).to_be_bytes()).unwrap();
            let mut shared = [0u8; 1];
            s.read_exact(&mut shared).unwrap();
            s.write_all(&ServerInit { width: 8, height: 4, format: equipe_rfb::PixelFormat::BGRX, name: "x".into() }.to_bytes()).unwrap();
            (ok, shared[0])
        });
        let (_, init) = join(&home, &Server::Tcp("127.0.0.1".into(), port)).unwrap();
        assert_eq!((init.width, init.height), (8, 4));
        assert_eq!(server.join().unwrap(), (true, 1), "the right answer, and joined shared");
        // Nobody there: said plainly.
        let gone = join(&home, &Server::Unix(home.join("nothing.sock"))).unwrap_err();
        assert_eq!(gone.kind, "no_server");
        let _ = std::fs::remove_dir_all(&home);
    }
}
