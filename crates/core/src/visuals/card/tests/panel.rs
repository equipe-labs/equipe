//! A card's kept page opened as the browser panel opens it — a real Chrome,
//! the page as a `data:` URL (and as its file) — then every element in it
//! clicked, with and without a modifier, and every focusable one reached by
//! Tab and pressed with Enter and Space: the page never goes anywhere else
//! (T-357). Chrome is driven over `--remote-debugging-pipe`, DevTools'
//! protocol on two pipes, so no port is opened for it.

use super::*;

use std::io::{Read as _, Write as _};
use std::os::fd::{FromRawFd as _, OwnedFd};
use std::sync::mpsc;

use base64::Engine as _;
use serde_json::{Value, json};

/// A headless Chrome of the test's own, spoken to over DevTools' pipe.
struct Devtools {
    child: std::process::Child,
    to: std::fs::File,
    from: mpsc::Receiver<Value>,
    next: u64,
    /// Events read while waiting for an answer, in order.
    events: Vec<Value>,
    _scratch: Scratch,
}

impl Devtools {
    fn start(chrome: &Path, proxy: &str) -> Self {
        use std::os::unix::process::CommandExt as _;
        let scratch = Scratch::new().unwrap();
        let pipe = || {
            let mut fds = [0; 2];
            // SAFETY: two fds written into an array of two.
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            // Chrome gets them as 3 and 4 only: not inherited past exec.
            for fd in fds {
                // SAFETY: an fd of ours.
                unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
            }
            // SAFETY: both are fresh fds of ours.
            unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
        };
        let (their_in, our_out) = pipe();
        let (our_in, their_out) = pipe();
        let (r, w) = (std::os::fd::AsRawFd::as_raw_fd(&their_in), std::os::fd::AsRawFd::as_raw_fd(&their_out));
        let mut command = Command::new(chrome);
        command
            // A transparent ground is the picture's, not the panel's — and
            // with DevTools attached it takes the page's renderer down.
            .args(CHROME_ARGS.iter().filter(|a| !a.starts_with("--default-background-color")))
            .arg("--remote-debugging-pipe")
            .arg(format!("--user-data-dir={}", scratch.0.join("profile").display()))
            .arg(format!("--proxy-server=http://{proxy}"))
            .arg(format!("--window-size={WIDTH},{HEIGHT}"))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::env::var_os("EQUIPE_CARD_CHROME_LOG").map_or(Stdio::null(), |p| std::fs::File::create(p).unwrap().into()))
            .process_group(0);
        // SAFETY: only dup2 between fork and exec. Chrome reads fd 3, writes fd 4.
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(r, 3) < 0 || libc::dup2(w, 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().expect("Chrome started");
        drop((their_in, their_out));
        let (tx, from) = mpsc::channel();
        let mut reader = std::fs::File::from(our_in);
        std::thread::spawn(move || {
            let mut pending = Vec::new();
            let mut buf = [0u8; 65536];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                pending.extend_from_slice(&buf[..n]);
                while let Some(end) = pending.iter().position(|b| *b == 0) {
                    let message: Vec<u8> = pending.drain(..=end).collect();
                    if let Ok(value) = serde_json::from_slice(&message[..end])
                        && tx.send(value).is_err()
                    {
                        return;
                    }
                }
            }
        });
        Self { child, to: std::fs::File::from(our_out), from, next: 0, events: Vec::new(), _scratch: scratch }
    }

    /// `method` sent (to `session` when given), its result waited for.
    fn call(&mut self, session: Option<&str>, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        let mut bytes = serde_json::to_vec(&message).unwrap();
        bytes.push(0);
        self.to.write_all(&bytes).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let Ok(value) = self.from.recv_timeout(left) else {
                panic!("no answer to {method}\n{}", self.log());
            };
            if value["id"] == id {
                assert!(value.get("error").is_none(), "{method}: {value}\n{}", self.log());
                return value["result"].clone();
            }
            self.events.push(value);
        }
    }

    /// Events that come within `wait`, kept with the others.
    fn settle(&mut self, wait: Duration) {
        let until = Instant::now() + wait;
        while let Ok(value) = self.from.recv_timeout(until.saturating_duration_since(Instant::now())) {
            self.events.push(value);
        }
    }

    /// The events so far, short, for a failure's message.
    fn log(&self) -> String {
        self.events.iter().map(|e| format!("{} {:.160}\n", e["method"], e["params"].to_string())).collect()
    }

    fn named(&self, method: &str) -> Vec<&Value> {
        self.events.iter().filter(|e| e["method"] == method).collect()
    }
}

impl Drop for Devtools {
    fn drop(&mut self) {
        // SAFETY: a signal to the process group our Chrome leads.
        unsafe {
            libc::killpg(self.child.id() as libc::pid_t, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

/// What opening `url` and pressing on everything in it did: the URLs its
/// top frame was asked to go to and went to after it loaded (a place in the
/// page among them), the tabs and windows that opened (the browser's own
/// `chrome://` ones too), and where it is at the end.
#[derive(Debug, Default)]
struct Went {
    navigated: Vec<String>,
    opened: Vec<String>,
    href: String,
}

fn press_everything(chrome: &Path, proxy: &str, url: &str) -> Went {
    let mut tools = Devtools::start(chrome, proxy);
    tools.call(None, "Target.setDiscoverTargets", json!({ "discover": true }));
    let target = tools.call(None, "Target.createTarget", json!({ "url": "about:blank" }))["targetId"].as_str().unwrap().to_owned();
    let session = tools.call(None, "Target.attachToTarget", json!({ "targetId": target, "flatten": true }))["sessionId"].as_str().unwrap().to_owned();
    let s = Some(session.as_str());
    tools.call(s, "Page.enable", json!({}));
    tools.call(s, "Page.navigate", json!({ "url": url }));
    let until = Instant::now() + Duration::from_secs(10);
    while tools.named("Page.loadEventFired").is_empty() {
        assert!(Instant::now() < until, "the page never loaded");
        tools.settle(Duration::from_millis(50));
    }
    let loaded = tools.events.len();
    // DevTools' own script runs whatever the page's CSP says: it only finds
    // where things are, it presses nothing.
    let found = tools.call(
        s,
        "Runtime.evaluate",
        json!({ "returnByValue": true, "expression": "JSON.stringify([...document.querySelectorAll('article *')].map(e => e.getBoundingClientRect()).filter(r => r.width && r.height).map(r => [r.x + r.width / 2, r.y + r.height / 2]))" }),
    );
    let points: Vec<(f64, f64)> = serde_json::from_str(found["result"]["value"].as_str().unwrap()).unwrap();
    assert!(!points.is_empty(), "nothing in the card to press");
    // Plain, ⌘ (a new tab), ⇧ (a new window), and the middle button.
    for (x, y) in points {
        for (button, modifiers) in [("left", 0), ("left", 4), ("left", 8), ("middle", 0)] {
            for kind in ["mousePressed", "mouseReleased"] {
                tools.call(s, "Input.dispatchMouseEvent", json!({ "type": kind, "x": x, "y": y, "button": button, "buttons": 1, "clickCount": 1, "modifiers": modifiers }));
            }
        }
    }
    let key = |tools: &mut Devtools, key: &str, code: &str, vk: u32, text: &str| {
        tools.call(s, "Input.dispatchKeyEvent", json!({ "type": "keyDown", "key": key, "code": code, "windowsVirtualKeyCode": vk, "text": text }));
        tools.call(s, "Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk }));
    };
    for _ in 0..40 {
        key(&mut tools, "Tab", "Tab", 9, "");
        key(&mut tools, "Enter", "Enter", 13, "\r");
        key(&mut tools, " ", "Space", 32, " ");
    }
    tools.settle(Duration::from_millis(800));
    let href = tools.call(s, "Runtime.evaluate", json!({ "returnByValue": true, "expression": "location.href" }))["result"]["value"].as_str().unwrap_or_default().to_owned();
    let after = &tools.events[loaded..];
    let navigated = after
        .iter()
        .filter_map(|e| match e["method"].as_str() {
            // Asked for, gone through or not (a blocked one never arrives).
            Some("Page.frameRequestedNavigation") => e["params"]["url"].as_str(),
            Some("Page.frameNavigated") if e["params"]["frame"].get("parentId").is_none() => e["params"]["frame"]["url"].as_str(),
            _ => None,
        })
        .map(str::to_owned)
        .collect();
    let opened = after.iter().filter(|e| e["method"] == "Target.targetCreated").map(|e| e["params"]["targetInfo"]["url"].as_str().unwrap_or_default().to_owned()).collect();
    Went { navigated, opened, href }
}

/// A card that does all a card may with links and things to press: places
/// in the page, written every way that passes, and links without an href.
const PRESSABLE: &str = "<p id=\"top\"><a href=\"#end\" style=\"display:block;padding:20px;background:var(--accent)\">to the end</a></p>\
<p><a href=\" #top \">back up</a> · <a href=#top>again</a> · <a>no href</a> · <a href=\"#\">top</a></p>\
<details><summary>more</summary><p>inside</p></details>\
<svg width=\"200\" height=\"40\"><a href=\"#top\"><rect width=\"200\" height=\"40\" fill=\"currentColor\"/></a></svg>\
<p contenteditable>https://equipe.im/ file:///etc/passwd</p><div tabindex=\"0\">focus</div><p id=\"end\">end</p>";

fn data_url(html: &str) -> String {
    format!("data:text/html;base64,{}", base64::engine::general_purpose::STANDARD.encode(html))
}

#[test]
fn a_kept_page_in_the_panel_goes_nowhere_by_click_or_key() {
    let Some(chrome) = chrome_here() else { return };
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_at = proxy.local_addr().unwrap().to_string();
    let port = server.local_addr().unwrap().port();
    assert_eq!(check(PRESSABLE), Ok(()));

    // The harness sees a link out when there is one: the same page with one,
    // past the check, goes.
    let out = format!("<a href=\"http://127.0.0.1:{port}/out\" style=\"display:block;padding:40px\">out</a>{PRESSABLE}");
    assert!(check(&out).is_err());
    let went = press_everything(&chrome, &proxy_at, &data_url(&page(&out, None)));
    assert!(went.navigated.iter().any(|u| u.contains("/out")), "the harness saw nothing leave: {went:?}");
    knocks(&proxy);

    // The page as the desktop opens it (a `data:` URL), and as its file.
    let home = temp_home("panel");
    let page_path = page_file(&home, "acme", &diagram::key(PRESSABLE));
    keep(&page_path, page(PRESSABLE, None).as_bytes()).unwrap();
    let kept_html = std::fs::read_to_string(&page_path).unwrap();
    kept_fragment(&kept_html).expect("the kept page keeps the contract");
    let file_url = format!("file://{}", page_path.display().to_string().replace('%', "%25").replace(' ', "%20"));
    for url in [data_url(&kept_html), file_url] {
        let went = press_everything(&chrome, &proxy_at, &url);
        let here = url.split('#').next().unwrap();
        let away: Vec<&String> = went.navigated.iter().filter(|u| u.split('#').next() != Some(here)).collect();
        assert!(away.is_empty(), "{url:.40}: went to {away:.80?}");
        // ⌘-click on a `#` link of the file opens the same page in a tab:
        // the same page still. As the panel opens it (`data:`), not even that.
        let elsewhere: Vec<&String> = went.opened.iter().filter(|u| !u.starts_with("chrome://") && (url.starts_with("data:") || u.split('#').next() != Some(here))).collect();
        assert!(elsewhere.is_empty(), "{url:.40}: opened {elsewhere:.80?}");
        assert_eq!(went.href.split('#').next().unwrap(), here, "left the page");
    }
    assert_eq!(knocks(&server), 0, "the card's server was reached");
    assert_eq!(knocks(&proxy), 0, "something went out through Chrome's proxy");
    let _ = std::fs::remove_dir_all(&home);
}
