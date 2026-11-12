//! equipe signing a member's browser back in by itself, on the real browser
//! host and a real (headless) Chrome with a profile of its own — never the
//! person's — against a sign-in page served here: the host's `fill` types
//! the kept login into the member's tab while its tools are kept off that
//! tab, answers only what the script did, and types nothing on another
//! host. Needs Node, npx and Google Chrome (or set `EQUIPE_BROWSER_CHANNEL`),
//! so it runs only when asked:
//!
//! ```sh
//! cargo test -p equipe-core --test relogin_live -- --ignored --nocapture
//! ```

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use equipe_core::browser::{self, HostEvent};
use equipe_core::protocol::vault::Login;
use equipe_core::vault::{self, FillGuard};
use serde_json::json;

/// A site with a sign-in form: `/login` shows it, `/session` takes it (the
/// body kept in `posted`) and goes on to `/home`, which has no form.
fn site(posted: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let posted = posted.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    return;
                }
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length];
                let _ = reader.read_exact(&mut body);
                let mut out = stream;
                let page = |html: &str| format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{html}", html.len());
                let reply = if first.starts_with("POST /session") {
                    posted.lock().unwrap().push(String::from_utf8_lossy(&body).into_owned());
                    "HTTP/1.1 303 See Other\r\nlocation: /home\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
                } else if first.starts_with("GET /home") {
                    page("<title>Home</title><h1>Welcome back</h1>")
                } else {
                    page("<title>Sign in</title><form method=post action=/session><input name=username autocomplete=username><input name=password type=password><button type=submit>Sign in</button></form>")
                };
                let _ = out.write_all(reply.as_bytes());
            });
        }
    });
    port
}

fn npx() -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join("npx")).find(|p| p.is_file()))
}

#[tokio::test]
#[ignore = "starts the browser host with a real Chrome"]
async fn the_host_types_a_kept_login_into_a_members_tab_and_nowhere_else() {
    let Some(npx) = npx() else {
        eprintln!("no npx here: nothing to test");
        return;
    };
    // SAFETY: set before the host starts, only by this test binary.
    unsafe { std::env::set_var("EQUIPE_BROWSER_HEADLESS", "1") };
    let home = std::env::temp_dir().join(format!("equipe-relogin-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let posted = Arc::new(Mutex::new(Vec::new()));
    let port = site(posted.clone());
    let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let hub = browser::Hub::new();
    let host = browser::Host::start(&npx, &home, &hub, tx).expect("the host starts");
    let output = home.join("out");
    let lane = browser::lane(&host, &home, "acme", "qa", &output, false).expect("a lane");
    let client = reqwest::Client::new();
    assert!(browser::hold(&client, lane.port).await, "the lane's tools answer");
    let login_url = format!("http://127.0.0.1:{port}/login");
    browser::navigate(&client, lane.port, &login_url).await;

    // The host reads the page as a sign-in waiting on a person.
    let page = tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(event) = events.recv().await {
            if let HostEvent::Tabs { lane, pages, .. } = event
                && lane == "acme/qa"
                && let Some(p) = pages.iter().find(|p| p.url.ends_with("/login") && p.signals.as_ref().is_some_and(|s| s.password))
            {
                assert_eq!(browser::human_check(&p.url, &p.title, p.signals.as_ref()), Some(equipe_core::protocol::HumanCheck::SignIn));
                return p.id;
            }
        }
        panic!("the host went");
    })
    .await
    .expect("the sign-in page is seen");

    let login = Login { id: 1, site: "127.0.0.1".into(), url: None, username: "abel".into(), note: None, allowed: Vec::new(), auto: true, stopped: None, created_at: 0, password_at: None, uses: Vec::new() };
    assert!(vault::auto_login_for(std::slice::from_ref(&login), &login_url).is_some());
    let fill = |token: u64, page: u64| json!({ "op": "fill", "lane": "acme/qa", "page": page, "script": vault::fill_script("abel", "pw-live-1", &FillGuard::exact(&login)), "token": token });
    host.lock().unwrap().op(&fill(1, page));
    // Meanwhile the member's own tools keep off that tab.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let refused = browser::call(&client, lane.port, "browser_snapshot", json!({})).await.unwrap_or_default();
    let refused: String = refused.iter().filter_map(|i| i["text"].as_str()).collect();
    assert!(refused.contains("signing this tab back in"), "{refused}");
    let answer = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(event) = events.recv().await {
            if let HostEvent::Filled { token: 1, answer, .. } = event {
                return answer;
            }
        }
        None
    })
    .await
    .expect("the host answers");
    let filled = answer.as_deref().and_then(vault::fill_answer).expect("the script's answer");
    assert!(filled.submitted && filled.refused.is_none(), "{filled:?}");
    assert!(!answer.unwrap_or_default().contains("pw-live-1"), "the answer never carries the password");
    assert_eq!(posted.lock().unwrap().as_slice(), ["username=abel&password=pw-live-1"]);

    // On another host (the same machine by another name): nothing typed.
    browser::navigate(&client, lane.port, &format!("http://localhost:{port}/login")).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let page = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(event) = events.recv().await {
            if let HostEvent::Tabs { lane, pages, .. } = event
                && lane == "acme/qa"
                && let Some(p) = pages.iter().find(|p| p.url.starts_with("http://localhost:"))
            {
                return p.id;
            }
        }
        panic!("the host went");
    })
    .await
    .unwrap_or(page);
    host.lock().unwrap().op(&fill(2, page));
    let answer = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(event) = events.recv().await {
            if let HostEvent::Filled { token: 2, answer, .. } = event {
                return answer;
            }
        }
        None
    })
    .await
    .expect("the host answers");
    let filled = answer.as_deref().and_then(vault::fill_answer).expect("the script's answer");
    assert_eq!(filled.refused.as_deref(), Some("host"), "{filled:?}");
    assert_eq!(posted.lock().unwrap().len(), 1, "nothing more was sent");

    // A member's own `sign_in` fills through its browser tools (`browser_evaluate`):
    // the answer is read back out of the tool's text, which quotes the script too.
    browser::navigate(&client, lane.port, &login_url).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let said = browser::call(&client, lane.port, "browser_evaluate", json!({ "function": vault::fill_script("abel", "pw-live-1", &FillGuard::serving(&login)) })).await.unwrap_or_default();
    let text: String = said.iter().filter_map(|i| i["text"].as_str()).collect::<Vec<_>>().join("\n");
    let filled = vault::fill_answer(&text).unwrap_or_else(|| panic!("the answer in: {}", text.replace("pw-live-1", "•••")));
    assert_eq!((filled.submitted, filled.step.as_deref()), (true, Some("password")), "{filled:?}");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(posted.lock().unwrap().len(), 2);
    drop(lane);
    drop(host);
    let _ = std::fs::remove_dir_all(&home);
}
