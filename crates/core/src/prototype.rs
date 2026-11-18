//! Live prototypes of a decision's options (`try`): what the person tries
//! — hovers, presses, tabs through, types in — rather than only looks at,
//! when a choice is about how a screen behaves (docs/VISUALS.md).
//!
//! A member gives each option one of two: a loopback page it serves on the
//! daemon's machine (`http://localhost:5173/states`), or the path of an HTML
//! page it saved in its worktree or the squad's folder (a file, or a folder
//! with `index.html`). Both are checked when the decision is asked
//! ([`check`]): a page whose port answers, a file that is there and inside
//! where the member works — else the member hears what to fix.
//!
//! A saved page is served here ([`Server`]): `127.0.0.1` only, under a
//! token no other page knows, its folder's files — the page's own CSS, JS,
//! pictures and fonts, never a hidden file nor one of another kind — and
//! nothing fetched from elsewhere (its CSP). A screen asks where
//! (`prototype_url`) and shows it in its live preview, as it shows a
//! member's dev server, forwarded over its SSH link from another Mac.

use std::collections::HashMap;
use std::net::{SocketAddr, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// How long a loopback page's port is given to answer when the decision is asked.
const ANSWER_WAIT: Duration = Duration::from_millis(600);
/// The largest file of a prototype served.
const FILE_MAX: u64 = 30 << 20;
/// The kinds of file a prototype's folder serves, and the type each goes as.
const KINDS: [(&str, &str); 16] = [
    ("html", "text/html; charset=utf-8"),
    ("htm", "text/html; charset=utf-8"),
    ("css", "text/css; charset=utf-8"),
    ("js", "text/javascript; charset=utf-8"),
    ("mjs", "text/javascript; charset=utf-8"),
    ("json", "application/json"),
    ("svg", "image/svg+xml"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("webp", "image/webp"),
    ("gif", "image/gif"),
    ("woff2", "font/woff2"),
    ("woff", "font/woff"),
    ("ttf", "font/ttf"),
    ("ico", "image/x-icon"),
];
/// What a prototype may do: run its own scripts and styles, load its own
/// files — nothing from the network, no forms sent anywhere, no framing.
const CSP: &str = "default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob:; connect-src 'self'; form-action 'none'; frame-ancestors 'none'; base-uri 'none'";

/// The type a prototype's file goes as, by its name; `None`: not served.
fn kind_of(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    KINDS.iter().find(|(e, _)| *e == ext).map(|(_, k)| *k)
}

/// Whether `given` names a page served on a port rather than a saved file.
pub fn is_address(given: &str) -> bool {
    given.contains("://")
}

/// An option's `try` checked as the member gave it: a loopback page whose
/// port answers now, as given; or an HTML page saved under one of `roots`
/// (relative to `cwd`, where the member works; a folder: its `index.html`),
/// as its full path. What is wrong is said so the member can fix it.
pub fn check(given: &str, cwd: &Path, roots: &[PathBuf]) -> Result<String, String> {
    let given = given.trim();
    if is_address(given) {
        let target = crate::live_preview::loopback(given).map_err(|_| format!("{given} isn't a page served on this machine (http://localhost:<port>/…)"))?;
        if !answers(target.port) {
            return Err(format!("nothing answers at {given} — start its server, keep it running while the person decides, and ask again"));
        }
        return Ok(given.to_owned());
    }
    page_file(given, cwd, roots).map(|p| p.to_string_lossy().into_owned())
}

/// Whether something listens at `port` on this machine's loopback.
fn answers(port: u16) -> bool {
    ["127.0.0.1", "::1"].iter().filter_map(|ip| format!("{ip}:{port}").parse::<SocketAddr>().ok().or_else(|| format!("[{ip}]:{port}").parse().ok())).any(|at| TcpStream::connect_timeout(&at, ANSWER_WAIT).is_ok())
}

/// The HTML page `given` names, as it really is: under one of `roots`, no
/// `..`, a file named `.html` — a folder's `index.html`.
fn page_file(given: &str, cwd: &Path, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let given = given.strip_prefix("file://").unwrap_or(given);
    let path = Path::new(given);
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("{given}: give the page's path without `..`"));
    }
    let path = if path.is_absolute() { path.to_owned() } else { cwd.join(path) };
    let path = if path.is_dir() { path.join("index.html") } else { path };
    let real = std::fs::canonicalize(&path).map_err(|_| format!("{}: no such page — save the prototype's HTML there first", path.display()))?;
    if !real.is_file() || !matches!(kind_of(&real), Some(k) if k.starts_with("text/html")) {
        return Err(format!("{}: not an HTML page (a .html file, or a folder with index.html)", path.display()));
    }
    let inside = roots.iter().filter_map(|r| std::fs::canonicalize(r).ok()).any(|root| real.starts_with(root));
    if !inside {
        return Err(format!("{}: outside your worktree and the squad's folder — save it there", path.display()));
    }
    Ok(real)
}

/// Whether a waiting decision of the squad whose journal is `db` names
/// `path` as an option's `try`.
pub fn named(db: &Path, path: &str) -> bool {
    crate::journal::waiting_questions_at(db).unwrap_or_default().iter().flat_map(|set| &set.questions).flat_map(|q| &q.options).any(|o| o.try_it == path)
}

/// Where a saved page `page` (a full path a waiting decision names) is served:
/// the server started on first use.
pub async fn url_of(page: &Path) -> Result<String, String> {
    let server = Server::get().await.ok_or("the prototype server couldn't start")?;
    server.put(page)
}

/// The daemon's prototype server: `127.0.0.1` only, each page's folder at a
/// path with a token no other page knows.
#[derive(Clone)]
pub struct Server {
    port: u16,
    token: String,
    /// Each folder served, by its key.
    folders: Arc<Mutex<HashMap<String, PathBuf>>>,
}

static SERVER: tokio::sync::OnceCell<Option<Server>> = tokio::sync::OnceCell::const_new();

/// A random token, from the system's randomness.
fn token() -> String {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0u8; 16];
    let _ = ring::rand::SystemRandom::new().fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Server {
    /// The server, started on first use; `None` when it couldn't listen.
    pub async fn get() -> Option<Server> {
        SERVER.get_or_init(|| async { Server::start().await.ok() }).await.clone()
    }

    async fn start() -> std::io::Result<Server> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let server = Server { port: listener.local_addr()?.port(), token: token(), folders: Arc::default() };
        let serving = server.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(serving.clone().answer(socket));
            }
        });
        Ok(server)
    }

    /// Serve page `page`'s folder; the page's address. The same folder keeps its address.
    pub fn put(&self, page: &Path) -> Result<String, String> {
        let real = std::fs::canonicalize(page).map_err(|_| format!("{}: no such page", page.display()))?;
        let folder = real.parent().ok_or("a page with no folder")?.to_path_buf();
        let name = real.file_name().and_then(|n| n.to_str()).ok_or("a page with no name")?.to_owned();
        let mut folders = self.folders.lock().map_err(|_| "the prototype server is unwell")?;
        let key = match folders.iter().find(|(_, f)| **f == folder) {
            Some((key, _)) => key.clone(),
            None => {
                let key = token();
                folders.insert(key.clone(), folder);
                key
            }
        };
        Ok(format!("http://127.0.0.1:{}/{}/{key}/{}", self.port, self.token, encode(&name)))
    }

    /// The file asked for by its path, `/<token>/<key>/<rel>`: inside its
    /// folder (links resolved), never hidden, of a kind served.
    fn file_at(&self, path: &str) -> Option<(PathBuf, &'static str)> {
        let path = path.split(['?', '#']).next()?;
        let rest = path.strip_prefix('/')?.strip_prefix(self.token.as_str())?.strip_prefix('/')?;
        let (key, rel) = rest.split_once('/')?;
        let folder = self.folders.lock().ok()?.get(key)?.clone();
        let rel = decode(rel)?;
        let rel = if rel.is_empty() || rel.ends_with('/') { format!("{rel}index.html") } else { rel };
        let rel = Path::new(&rel);
        if !rel.components().all(|c| matches!(c, Component::Normal(n) if !n.to_string_lossy().starts_with('.'))) {
            return None;
        }
        let real = std::fs::canonicalize(folder.join(rel)).ok()?;
        let kind = kind_of(&real)?;
        (real.starts_with(&folder) && std::fs::metadata(&real).is_ok_and(|m| m.is_file() && m.len() <= FILE_MAX)).then_some((real, kind))
    }

    async fn answer(self, mut socket: tokio::net::TcpStream) {
        // The whole head, not just the first read's worth: a request can come
        // in pieces, and closing with some of it unread resets the connection
        // before the reply is read.
        let mut buf = vec![0u8; 8192];
        let mut n = 0;
        let read_head = async {
            while n < buf.len() && !buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                match socket.read(&mut buf[n..]).await {
                    Ok(0) | Err(_) => break,
                    Ok(got) => n += got,
                }
            }
        };
        if tokio::time::timeout(Duration::from_secs(10), read_head).await.is_err() || n == 0 {
            return;
        }
        let head = String::from_utf8_lossy(&buf[..n]);
        let mut words = head.lines().next().unwrap_or_default().split(' ');
        let found = match (words.next(), words.next()) {
            (Some("GET" | "HEAD"), Some(path)) => self.file_at(path),
            _ => None,
        };
        let (status, kind, body) = match found.and_then(|(path, kind)| std::fs::read(path).ok().map(|b| (kind, b))) {
            Some((kind, body)) => ("200 OK", kind, body),
            None => ("404 Not Found", "text/plain; charset=utf-8", b"Not here.".to_vec()),
        };
        let reply = format!(
            "HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-security-policy: {CSP}\r\ncache-control: no-store\r\nx-content-type-options: nosniff\r\nreferrer-policy: no-referrer\r\nconnection: close\r\ncontent-length: {}\r\n\r\n",
            body.len()
        );
        let _ = socket.write_all(reply.as_bytes()).await;
        let _ = socket.write_all(&body).await;
        let _ = socket.shutdown().await;
    }
}

/// A file name as it goes in an address: letters, digits and `-._~` as they are.
fn encode(name: &str) -> String {
    name.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// An address's path as the file's: `%XX` read back; `None` when it isn't text.
fn decode(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-prototype-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn a_saved_page_is_checked_where_the_member_works_and_a_served_one_must_answer() {
        let root = dir("check");
        let work = root.join("work");
        std::fs::create_dir_all(work.join("design/proto/a")).unwrap();
        std::fs::write(work.join("design/proto/a/index.html"), "<button>Save</button>").unwrap();
        std::fs::write(work.join("design/proto/b.html"), "<button>Save</button>").unwrap();
        std::fs::write(work.join("design/proto/notes.txt"), "no").unwrap();
        let roots = [work.clone()];
        assert_eq!(check("design/proto/a", &work, &roots).unwrap(), work.join("design/proto/a/index.html").to_string_lossy());
        assert_eq!(check(&work.join("design/proto/b.html").to_string_lossy(), &root, &roots).unwrap(), work.join("design/proto/b.html").to_string_lossy());
        assert!(check("design/proto/missing.html", &work, &roots).unwrap_err().contains("no such page"));
        assert!(check("design/proto/notes.txt", &work, &roots).unwrap_err().contains("not an HTML page"));
        assert!(check("../work/design/proto/b.html", &work.join("design"), &roots).unwrap_err().contains("`..`"));
        // Saved outside where the member works.
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("x.html"), "<p>").unwrap();
        assert!(check(&outside.join("x.html").to_string_lossy(), &work, &roots).unwrap_err().contains("outside"));
        // A page served here: its port must answer; another host never.
        let listening = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listening.local_addr().unwrap().port();
        let page = format!("http://localhost:{port}/states");
        assert_eq!(check(&page, &work, &roots).unwrap(), page);
        drop(listening);
        assert!(check(&page, &work, &roots).unwrap_err().contains("nothing answers"));
        assert!(check("https://acme.vercel.app/", &work, &roots).unwrap_err().contains("isn't a page served on this machine"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_saved_page_is_served_with_its_own_files_and_nothing_else() {
        let root = dir("serve");
        let page = root.join("proto");
        std::fs::create_dir_all(page.join("assets")).unwrap();
        std::fs::write(page.join("index.html"), "<link rel=stylesheet href=assets/app.css><button>Save</button>").unwrap();
        std::fs::write(page.join("assets/app.css"), "button:hover{background:#000}").unwrap();
        std::fs::write(page.join(".env"), "SECRET=1").unwrap();
        std::fs::write(page.join("notes.md"), "private").unwrap();
        std::fs::write(root.join("outside.html"), "<p>outside").unwrap();
        let server = Server::start().await.unwrap();
        let url = server.put(&page.join("index.html")).unwrap();
        assert!(url.starts_with(&format!("http://127.0.0.1:{}/{}/", server.port, server.token)) && url.ends_with("/index.html"), "{url}");
        assert_eq!(server.put(&page.join("index.html")).unwrap(), url, "the same folder, the same address");
        let base = url.trim_start_matches(&format!("http://127.0.0.1:{}", server.port)).trim_end_matches("index.html").to_owned();
        let get = |rel: &str| server.file_at(&format!("{base}{rel}")).map(|(p, k)| (p.file_name().unwrap().to_string_lossy().into_owned(), k));
        assert_eq!(get("index.html").unwrap().0, "index.html");
        assert_eq!(get("").unwrap().0, "index.html", "the folder: its index");
        assert_eq!(get("assets/app.css?v=2").unwrap(), ("app.css".to_owned(), "text/css; charset=utf-8"));
        assert!(get(".env").is_none(), "never a hidden file");
        assert!(get("notes.md").is_none(), "never a file of another kind");
        assert!(get("../outside.html").is_none() && get("%2E%2E/outside.html").is_none(), "never outside its folder");
        assert!(server.file_at("/wrong-token/x/index.html").is_none());
        // Over the wire: the page, its CSP.
        let port = server.port;
        let path = base.clone() + "index.html";
        let reply = tokio::task::spawn_blocking(move || {
            use std::io::{Read as _, Write as _};
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        })
        .await
        .unwrap();
        assert!(reply.starts_with("HTTP/1.1 200") && reply.contains("connect-src 'self'") && reply.ends_with("<button>Save</button>"), "{reply}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_name_goes_in_an_address_and_comes_back() {
        assert_eq!(encode("상태 a.html"), "%EC%83%81%ED%83%9C%20a.html");
        assert_eq!(decode(&encode("상태 a.html")).unwrap(), "상태 a.html");
        assert_eq!(decode("a%2"), Some("a%2".to_owned()));
    }
}
