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
//!
//! A design session's page (`equipe design session show page.html`) is
//! served the same way ([`Server::put_session`]), so the person's screen
//! opens it wherever it is — never as a `file://` address only the
//! daemon's machine can open: its design folder (the page may reach
//! `../tokens.css`), and the web's fonts and pictures it uses, in a frame of
//! the variants side by side (docs/REMOTE.md).

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
/// What a design session's page may do: as a prototype, and load the web's
/// fonts, pictures, styles and scripts it names (as the `file://` page did),
/// and be framed — the variants side by side are frames of one page.
const SESSION_CSP: &str = "default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob: https:; connect-src 'self' https:; form-action 'none'; base-uri 'none'";

/// How a folder is served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// A decision's prototype: its own files only, never framed.
    Prototype,
    /// A design session's page: the web's resources too, framed side by side.
    Session,
}

impl Policy {
    /// A session's page names its own server's address as it asks for a
    /// file, so one named from the root is found ([`Server::root_address`]);
    /// nothing goes anywhere else.
    fn referrer(self) -> &'static str {
        match self {
            Policy::Prototype => "no-referrer",
            Policy::Session => "same-origin",
        }
    }

    fn csp(self) -> &'static str {
        match self {
            Policy::Prototype => CSP,
            Policy::Session => SESSION_CSP,
        }
    }
}

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
    /// Each folder served, by its key, and how.
    folders: Arc<Mutex<HashMap<String, (PathBuf, Policy)>>>,
}

static SERVER: std::sync::OnceLock<Option<Server>> = std::sync::OnceLock::new();

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
        Server::now()
    }

    /// [`Server::get`], from code that can't wait (the runtime's loop): it
    /// starts at once, on the Tokio runtime this is called on. `None` when
    /// it couldn't listen, or there is no runtime to serve on.
    pub fn now() -> Option<Server> {
        SERVER.get_or_init(|| Server::start().ok()).clone()
    }

    fn start() -> std::io::Result<Server> {
        let tokio = tokio::runtime::Handle::try_current().map_err(std::io::Error::other)?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let server = Server { port, token: token(), folders: Arc::default() };
        let serving = server.clone();
        tokio.spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return };
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(serving.clone().answer(socket));
            }
        });
        Ok(server)
    }

    /// Serve page `page`'s folder as a decision's prototype; the page's
    /// address. The same folder keeps its address.
    pub fn put(&self, page: &Path) -> Result<String, String> {
        let real = std::fs::canonicalize(page).map_err(|_| format!("{}: no such page", page.display()))?;
        let folder = real.parent().ok_or("a page with no folder")?.to_path_buf();
        self.serve(&real, &folder, Policy::Prototype)
    }

    /// Serve a design session's page `page` (already checked to be where its
    /// member works): `scope`, the folder whose files it may load — its
    /// design folder, else its own; the page's address.
    pub fn put_session(&self, page: &Path, scope: &Path) -> Result<String, String> {
        let real = std::fs::canonicalize(page).map_err(|_| format!("{}: no such page", page.display()))?;
        let scope = std::fs::canonicalize(scope).ok().filter(|s| real.starts_with(s)).or_else(|| real.parent().map(Path::to_path_buf)).ok_or("a page with no folder")?;
        self.serve(&real, &scope, Policy::Session)
    }

    /// Where `page` is served already, if it is: by the folder it went in.
    pub fn address_of(&self, page: &Path) -> Option<String> {
        let real = std::fs::canonicalize(page).ok()?;
        let folders = self.folders.lock().ok()?;
        let (key, (folder, _)) = folders.iter().filter(|(_, (f, _))| real.starts_with(f)).max_by_key(|(_, (f, _))| f.components().count())?;
        Some(self.address(key, &real.strip_prefix(folder).ok()?.to_string_lossy()))
    }

    /// The file an address of this server's names, when it names one.
    pub fn file_of(&self, url: &str) -> Option<PathBuf> {
        let path = url.strip_prefix(&format!("http://127.0.0.1:{}", self.port))?;
        self.file_at(path).map(|(file, _, _)| file)
    }

    fn serve(&self, real: &Path, folder: &Path, policy: Policy) -> Result<String, String> {
        let rel = real.strip_prefix(folder).map_err(|_| format!("{}: outside the folder it is served from", real.display()))?.to_string_lossy().into_owned();
        if rel.is_empty() {
            return Err(format!("{}: a page, not a folder", real.display()));
        }
        let mut folders = self.folders.lock().map_err(|_| "the prototype server is unwell")?;
        let key = match folders.iter().find(|(_, (f, p))| f == folder && *p == policy) {
            Some((key, _)) => key.clone(),
            None => {
                let key = token();
                folders.insert(key.clone(), (folder.to_path_buf(), policy));
                key
            }
        };
        Ok(self.address(&key, &rel))
    }

    /// The address of `rel` (`/`-separated) in the folder served at `key`.
    fn address(&self, key: &str, rel: &str) -> String {
        let rel: Vec<String> = Path::new(rel).components().map(|c| encode(&c.as_os_str().to_string_lossy())).collect();
        format!("http://127.0.0.1:{}/{}/{key}/{}", self.port, self.token, rel.join("/"))
    }

    /// The file asked for by its path, `/<token>/<key>/<rel>`: inside its
    /// folder (links resolved), never hidden, of a kind served.
    fn file_at(&self, path: &str) -> Option<(PathBuf, &'static str, Policy)> {
        let path = path.split(['?', '#']).next()?;
        let rest = path.strip_prefix('/')?.strip_prefix(self.token.as_str())?.strip_prefix('/')?;
        let (key, rel) = rest.split_once('/')?;
        let (folder, policy) = self.folders.lock().ok()?.get(key)?.clone();
        let rel = decode(rel)?;
        let rel = if rel.is_empty() || rel.ends_with('/') { format!("{rel}index.html") } else { rel };
        let rel = Path::new(&rel);
        if !rel.components().all(|c| matches!(c, Component::Normal(n) if !n.to_string_lossy().starts_with('.'))) {
            return None;
        }
        let real = std::fs::canonicalize(folder.join(rel)).ok()?;
        let kind = kind_of(&real)?;
        (real.starts_with(&folder) && std::fs::metadata(&real).is_ok_and(|m| m.is_file() && m.len() <= FILE_MAX)).then_some((real, kind, policy))
    }

    /// Where a design session's page's address from the root (`/tokens.css`,
    /// `/assets/logo.svg`) is served: found from the page that asked
    /// (`referer`, one of this server's session pages), in its design folder
    /// and then the project around it (`design::page_check::root_absolute`).
    /// The address under the token to send the browser on to, else `None`.
    fn root_address(&self, path: &str, referer: Option<&str>) -> Option<String> {
        let path = path.split(['?', '#']).next()?;
        if path.starts_with(&format!("/{}/", self.token)) {
            return None;
        }
        let asking = referer?.split_once("://")?.1;
        let asking = &asking[asking.find('/')?..];
        let key = asking.strip_prefix('/')?.strip_prefix(self.token.as_str())?.strip_prefix('/')?.split('/').next()?;
        let (folder, policy) = self.folders.lock().ok()?.get(key)?.clone();
        if policy != Policy::Session {
            return None;
        }
        let scope = crate::design::page_check::Scope::of_served(&folder);
        let found = std::fs::canonicalize(crate::design::page_check::root_absolute(&decode(path)?, &scope)?).ok()?;
        if let Ok(rel) = found.strip_prefix(&folder) {
            return Some(self.address(key, &rel.to_string_lossy()));
        }
        // In the project around the design folder: served as a session's folder of its own.
        let project = scope.project.and_then(|p| std::fs::canonicalize(p).ok()).filter(|p| found.starts_with(p))?;
        self.serve(&found, &project, Policy::Session).ok()
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
        let referer = head.lines().skip(1).find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("referer")).map(|(_, v)| v.trim().to_owned()));
        let (method, path) = (words.next(), words.next());
        let found = match (method, path) {
            (Some("GET" | "HEAD"), Some(path)) => self.file_at(path),
            _ => None,
        };
        // An address from the root, asked by a session's page: on to where it is.
        if found.is_none()
            && let (Some("GET" | "HEAD"), Some(path)) = (method, path)
            && let Some(to) = self.root_address(path, referer.as_deref())
        {
            let at = to.strip_prefix(&format!("http://127.0.0.1:{}", self.port)).unwrap_or(&to).to_owned();
            let reply = format!("HTTP/1.1 307 Temporary Redirect\r\nlocation: {at}\r\ncache-control: no-store\r\nconnection: close\r\ncontent-length: 0\r\n\r\n");
            let _ = socket.write_all(reply.as_bytes()).await;
            let _ = socket.shutdown().await;
            return;
        }
        let (status, kind, body, policy) = match found.and_then(|(path, kind, policy)| std::fs::read(path).ok().map(|b| (kind, b, policy))) {
            Some((kind, body, policy)) => ("200 OK", kind, body, policy),
            None => ("404 Not Found", "text/plain; charset=utf-8", b"Not here.".to_vec(), Policy::Prototype),
        };
        let reply = format!(
            "HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-security-policy: {}\r\ncache-control: no-store\r\nx-content-type-options: nosniff\r\nreferrer-policy: {}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n",
            policy.csp(),
            policy.referrer(),
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
        let server = Server::start().unwrap();
        let url = server.put(&page.join("index.html")).unwrap();
        assert!(url.starts_with(&format!("http://127.0.0.1:{}/{}/", server.port, server.token)) && url.ends_with("/index.html"), "{url}");
        assert_eq!(server.put(&page.join("index.html")).unwrap(), url, "the same folder, the same address");
        let base = url.trim_start_matches(&format!("http://127.0.0.1:{}", server.port)).trim_end_matches("index.html").to_owned();
        let get = |rel: &str| server.file_at(&format!("{base}{rel}")).map(|(p, k, _)| (p.file_name().unwrap().to_string_lossy().into_owned(), k));
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

    #[tokio::test]
    async fn a_session_pages_address_from_the_root_is_found_in_its_design_folder_then_its_project() {
        let project = dir("from-root");
        let design = project.join("design");
        std::fs::create_dir_all(design.join("sessions/pricing")).unwrap();
        std::fs::create_dir_all(design.join("assets")).unwrap();
        std::fs::create_dir_all(project.join("public")).unwrap();
        std::fs::write(design.join("tokens.css"), ":root{}").unwrap();
        std::fs::write(design.join("assets/logo.svg"), "<svg/>").unwrap();
        std::fs::write(project.join("public/og.png"), "png").unwrap();
        std::fs::write(design.join("sessions/pricing/a.html"), "<link rel=stylesheet href=/tokens.css>").unwrap();
        let server = Server::start().unwrap();
        let page = server.put_session(&design.join("sessions/pricing/a.html"), &design).unwrap();
        let base = page.trim_end_matches("sessions/pricing/a.html").to_owned();
        let from = Some(page.as_str());
        assert_eq!(server.root_address("/tokens.css", from), Some(format!("{base}tokens.css")), "the design folder first");
        assert_eq!(server.root_address("/design/assets/logo.svg?v=1", from), Some(format!("{base}assets/logo.svg")), "named with its design folder");
        let og = server.root_address("/public/og.png", from).expect("then the project");
        assert!(og.ends_with("/public/og.png") && !og.starts_with(&base), "{og}");
        assert_eq!(server.file_of(&og), Some(project.join("public/og.png")));
        assert_eq!(server.root_address("/nowhere.css", from), None);
        assert_eq!(server.root_address("/tokens.css", None), None, "only for a page that asked");
        assert_eq!(server.root_address("/tokens.css", Some("http://127.0.0.1:1/wrong/x/a.html")), None, "only this server's pages");
        // A decision's prototype gets none of it.
        std::fs::write(project.join("proto.html"), "x").unwrap();
        let proto = server.put(&project.join("proto.html")).unwrap();
        assert_eq!(server.root_address("/public/og.png", Some(&proto)), None);
        // Over the wire: sent on, and the session's page says where it is from.
        let port = server.port;
        let asked = page.clone();
        let reply = tokio::task::spawn_blocking(move || {
            use std::io::{Read as _, Write as _};
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(format!("GET /tokens.css HTTP/1.1\r\nHost: x\r\nReferer: {asked}\r\n\r\n").as_bytes()).unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        })
        .await
        .unwrap();
        assert!(reply.starts_with("HTTP/1.1 307") && reply.contains(&format!("location: {}tokens.css", base.trim_start_matches(&format!("http://127.0.0.1:{port}")))), "{reply}");
        assert_eq!(Policy::Session.referrer(), "same-origin");
        assert_eq!(Policy::Prototype.referrer(), "no-referrer");
        std::fs::remove_dir_all(project).unwrap();
    }

    #[tokio::test]
    async fn a_design_sessions_page_is_served_from_its_design_folder_and_may_be_framed() {
        let root = dir("session");
        let design = root.join("design");
        std::fs::create_dir_all(design.join("sessions/pricing")).unwrap();
        std::fs::write(design.join("tokens.css"), ":root{--ink:#111}").unwrap();
        std::fs::write(design.join("sessions/pricing/a.html"), "<link rel=stylesheet href=../../tokens.css>A").unwrap();
        std::fs::write(design.join("sessions/pricing/b b.html"), "B").unwrap();
        let server = Server::start().unwrap();
        let a = server.put_session(&design.join("sessions/pricing/a.html"), &design).unwrap();
        let b = server.put_session(&design.join("sessions/pricing/b b.html"), &design).unwrap();
        assert!(a.ends_with("/sessions/pricing/a.html") && b.ends_with("/sessions/pricing/b%20b.html"), "{a} {b}");
        assert_eq!(a.rsplit_once("/sessions/").unwrap().0, b.rsplit_once("/sessions/").unwrap().0, "one folder, one address: side by side on one port");
        assert_eq!(server.address_of(&design.join("sessions/pricing/a.html")).as_deref(), Some(a.as_str()), "found again by its file");
        assert_eq!(server.file_of(&b), Some(design.join("sessions/pricing/b b.html")), "and its file by its address");
        assert!(server.file_of("http://127.0.0.1:1/x/y/z.html").is_none());
        assert!(server.address_of(&root.join("elsewhere.html")).is_none());
        let base = a.trim_start_matches(&format!("http://127.0.0.1:{}", server.port)).trim_end_matches("sessions/pricing/a.html").to_owned();
        let (css, _, policy) = server.file_at(&format!("{base}tokens.css")).unwrap();
        assert_eq!((css.file_name().unwrap().to_str().unwrap(), policy), ("tokens.css", Policy::Session), "the page's ../ reaches its design folder");
        assert!(!SESSION_CSP.contains("frame-ancestors") && SESSION_CSP.contains("https:"), "framed side by side, the web's fonts load");
        // A scope that isn't the page's: its own folder.
        let own = server.put_session(&design.join("sessions/pricing/a.html"), &root.join("nowhere")).unwrap();
        assert!(own.ends_with("/a.html") && !own.contains("/sessions/"), "{own}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_name_goes_in_an_address_and_comes_back() {
        assert_eq!(encode("상태 a.html"), "%EC%83%81%ED%83%9C%20a.html");
        assert_eq!(decode(&encode("상태 a.html")).unwrap(), "상태 a.html");
        assert_eq!(decode("a%2"), Some("a%2".to_owned()));
    }
}
