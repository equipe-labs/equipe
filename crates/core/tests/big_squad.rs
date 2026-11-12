//! A big squad, measured: how long the daemon's loop is held by what screens
//! and members ask of a squad that has worked for weeks — two thousand
//! tickets with long records, hundreds of conversations, a dozen members —
//! and how long a decision card's pictures take to come.
//!
//! Not run by default (it seeds a large journal and takes a minute):
//!
//! ```sh
//! cargo test -p equipe-core --release --test big_squad -- --ignored --nocapture
//! ```
//!
//! Each line it prints is one measurement: what was asked, how long it took
//! (the median of a few, and the slowest), and for the loop's own steps the
//! parts that took longest. `EQUIPE_BIG_TICKETS` sets how many tickets.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::{Duration, Instant};

use equipe_core::journal::{Journal, SessionRecord};
use equipe_core::protocol::decide::{Choice, Question, QuestionSet};
use equipe_core::protocol::rpc::{ReadImage, ToolCall};
use equipe_core::protocol::{Access, Command, Engine, Entry, Event, Member, SessionSpec, Ticket};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use tokio::sync::mpsc::UnboundedReceiver;

const SQUAD: &str = "big";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("equipe-big-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A repository with one commit, as a squad's project.
fn repo(root: &Path) -> PathBuf {
    let dir = root.join("site");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

/// A stand-in `claude` that answers `auth status` and ends each turn at once.
fn fake_claude(root: &Path) -> PathBuf {
    let bin = root.join("claude");
    let script = r#"#!/bin/sh
if [ "$1" = "auth" ]; then echo '{"loggedIn":true,"authMethod":"claude.ai"}'; exit 0; fi
cat <<EOF
{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}
{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{"input_tokens":3,"output_tokens":1}}
EOF
"#;
    std::fs::write(&bin, script).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn config(claude: &Path, home: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "bench".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        codex_binary: "/nonexistent/codex".into(),
        persist: false,
        retention_days: 90,
        equipe_binary: None,
        npx: None,
        design_path: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::En,
        nemotron: None,
        watch_disk: false,
    }
}

/// Text of about `n` bytes.
fn words(n: usize, seed: usize) -> String {
    let base = "The checkout page keeps the cart when the session expires; the tests cover the empty cart and a lost connection. ";
    let mut out = String::with_capacity(n + base.len());
    while out.len() < n {
        out.push_str(base);
        out.push_str(&seed.to_string());
    }
    out.truncate(n);
    out
}

/// A picture like a member's render: `w`×`h`, noisy enough that it
/// compresses as a real one does.
fn render(path: &Path, w: u32, h: u32, seed: u32) {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    let img = image::RgbaImage::from_fn(w, h, |i, j| {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let n = (x & 0x1f) as u8;
        image::Rgba([(i * 255 / w) as u8 ^ n, (j * 255 / h) as u8, (seed as u8).wrapping_mul(40) ^ n, 255])
    });
    img.save(path).unwrap();
}

/// Seed squad `SQUAD`'s journal: `tickets` tickets with long records, past
/// conversations, and a decision waiting with 13 pictures (returned).
fn seed(home: &Path, project: &Path, tickets: usize) -> Vec<String> {
    let started = Instant::now();
    squads::create(home, SQUAD, project).unwrap();
    let journal = Journal::for_squad(home, SQUAD).unwrap();
    let worktrees = home.join("worktrees").join("site");
    for n in 1..=tickets {
        let status = match n % 50 {
            0 => "in_progress",
            1 => "todo",
            2 => "waiting",
            3 => "review",
            4..=8 => "cancelled",
            _ => "done",
        };
        // Most hand-ins showed pictures, in worktrees long gone.
        let images: Vec<String> = if n % 5 < 3 { (0..7).map(|k| worktrees.join(format!("t{n}-ab{n:x}/design/shot-{k}.png")).to_string_lossy().into_owned()).collect() } else { Vec::new() };
        let note = if n % 400 == 7 { words(2 << 20, n) } else { words(5_000, n) };
        let ticket: Ticket = serde_json::from_value(serde_json::json!({
            "id": 0, "title": format!("T{n}: make the checkout keep the cart"), "body": words(500, n), "status": status, "kind": "code",
            "depends_on": [], "owner": "frontend", "assignee": "frontend", "created_by": "pm", "note": note,
            "project": project, "branch": format!("equipe/t{n}"), "images": images,
        }))
        .unwrap();
        let id = journal.create_ticket(&ticket).unwrap();
        journal.save_ticket(&Ticket { id, ..ticket }).unwrap();
        for k in 0..8 {
            journal.ticket_event(id, "frontend", if k == 0 { "assigned" } else { "comment" }, &words(800, k)).unwrap();
        }
    }
    // Past conversations: members' lanes that ended.
    for s in 0..200u64 {
        let record = SessionRecord {
            id: 10_000 + s,
            member: Some(Member { role: "frontend".into(), name: format!("frontend-{}", s % 12) }),
            mode: None,
            engine: Engine::Claude,
            model: None,
            access: Access::EditFiles,
            cwd: project.to_owned(),
            repo: None,
            branch: None,
            base: None,
            engine_session: None,
            title: None,
            tokens: 0,
            open: false,
        };
        journal.record_session(&record).unwrap();
        for e in 0..300 {
            let entry = if e % 3 == 0 { Entry::User { text: words(200, e), kept: Vec::new() } } else { Entry::Reply { text: words(400, e) } };
            journal.append(record.id, &entry).unwrap();
        }
    }
    // A decision with a picture on top and four on each of three options: renders a designer made.
    let folder = equipe_core::squad_home::path(home, SQUAD).join("drafts");
    std::fs::create_dir_all(&folder).unwrap();
    let mut pictures = Vec::new();
    for k in 0..13u32 {
        let path = folder.join(format!("option-{k}.png"));
        render(&path, 2400, 1600, k + 1);
        pictures.push(path.canonicalize().unwrap().to_string_lossy().into_owned());
    }
    let choice = |label: &str, images: &[String]| Choice { label: label.into(), images: images.to_vec(), ..Choice::default() };
    let question = Question {
        question: "Which landing page?".into(),
        images: vec![pictures[0].clone()],
        options: vec![choice("A", &pictures[1..5]), choice("B", &pictures[5..9]), choice("C", &pictures[9..13])],
        ..Question::default()
    };
    let set = QuestionSet { id: 1, session: 10_000, member: "designer".into(), ticket: None, questions: vec![question], asked_at: 1, review: false, retro: None };
    journal.set_questions(&[set]).unwrap();
    println!("seeded {tickets} tickets, 200 conversations of 300 lines, 13 pictures in {:.1}s", started.elapsed().as_secs_f64());
    pictures
}

/// How long each of `n` runs of `f` took: the median and the slowest, in ms.
async fn timed<F, Fut>(n: usize, mut f: F) -> (f64, f64)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut took = Vec::with_capacity(n);
    for _ in 0..n {
        let at = Instant::now();
        f().await;
        took.push(at.elapsed().as_secs_f64() * 1000.);
    }
    took.sort_by(f64::total_cmp);
    (took[took.len() / 2], took[took.len() - 1])
}

fn say(what: &str, (median, worst): (f64, f64)) {
    println!("{what:<44} median {median:>8.1} ms   slowest {worst:>8.1} ms");
}

/// The loop's slow steps since last asked, each with its slowest parts.
fn steps(handle: &Handle) {
    for step in handle.slow_steps() {
        println!("    loop: {:<38} {:>8.1} ms   {}", step.doing, step.took.as_secs_f64() * 1000., step.slowest_parts(3));
    }
}

async fn until(rx: &mut UnboundedReceiver<Event>, done: impl Fn(&Event) -> bool) {
    let wait = async {
        while let Some(event) = rx.recv().await {
            if done(&event) {
                return;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), wait).await.expect("in time");
}

/// All 13 pictures at `side`, at once, as a card asks: how long until the last came.
fn card(home: &Path, pictures: &[String], side: u32) -> f64 {
    let at = Instant::now();
    std::thread::scope(|scope| {
        for path in pictures {
            scope.spawn(move || {
                let ask = ReadImage { squad: SQUAD.into(), path: path.clone(), max: Some(side) };
                equipe_core::visuals::serve(home, &ask, None).unwrap_or_else(|e| panic!("{e}"));
            });
        }
    });
    at.elapsed().as_secs_f64() * 1000.
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a measurement: seeds a big journal; run it with --ignored --nocapture"]
async fn a_big_squad_measured() {
    let root = scratch("root");
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let project = repo(&root);
    let tickets = std::env::var("EQUIPE_BIG_TICKETS").ok().and_then(|n| n.parse().ok()).unwrap_or(2000);
    let pictures = seed(&home, &project, tickets);
    let journal = Journal::for_squad(&home, SQUAD).unwrap();

    say("journal: every ticket", timed(5, || async { assert!(journal.tickets().unwrap().len() >= tickets) }).await);
    say("pictures: what the squad shows", timed(3, || async { std::hint::black_box(equipe_core::visuals::shown(&home, SQUAD)); }).await);
    println!("{:<44} {:>8.1} ms", "pictures: a card's 13 tiny previews, cold", card(&home, &pictures, 48));
    println!("{:<44} {:>8.1} ms", "pictures: a card's 13 thumbnails", card(&home, &pictures, 640));
    println!("{:<44} {:>8.1} ms", "pictures: a card's 13 thumbnails, again", card(&home, &pictures, 640));
    std::thread::sleep(Duration::from_secs(6));
    println!("{:<44} {:>8.1} ms", "pictures: a card's 13 thumbnails, 6s on", card(&home, &pictures, 640));

    let claude = fake_claude(&root);
    let (handle, mut events) = runtime::start(config(&claude, &home));
    handle.keep_steps_from(Duration::from_millis(100));
    let roles = ["pm", "designer", "frontend", "frontend", "frontend", "backend", "backend", "qa", "frontend", "backend", "designer", "qa"];
    for (n, role) in roles.iter().enumerate() {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some((*role).into()), squad: Some(SQUAD.into()) };
        let session = 100 + n as u64;
        handle.send(Command::Start { session, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session: s, .. } if *s == session)).await;
    }
    let snapshot = handle.snapshot(SQUAD).await;
    let names: Vec<String> = snapshot.sessions.iter().filter_map(|s| s.member.as_ref().map(|m| m.name.clone())).collect();
    println!("{} members: {}", names.len(), names.join(", "));
    steps(&handle);

    say("loop: a screen's snapshot (hello)", timed(5, || async { std::hint::black_box(handle.snapshot(SQUAD).await); }).await);
    steps(&handle);
    say("loop: minute tick", timed(3, || async { handle.minute_now().await }).await);
    steps(&handle);
    let call = |member: &str, name: &str, arguments: serde_json::Value| ToolCall { run: None, squad: SQUAD.into(), member: member.into(), name: name.into(), arguments };
    say("loop: tool board", timed(5, || async { std::hint::black_box(handle.tool(call("frontend", "board", serde_json::json!({}))).await); }).await);
    steps(&handle);
    let mut n = 0;
    say(
        "loop: tool create_ticket",
        timed(5, || {
            n += 1;
            let args = serde_json::json!({ "title": format!("Bench ticket {n}"), "body": "Measure it.", "owner": "backend" });
            let handle = &handle;
            let call = call("pm", "create_ticket", args);
            async move {
                let result = handle.tool(call).await;
                assert!(!result.is_error, "{result:?}");
            }
        })
        .await,
    );
    steps(&handle);
    say("loop: tool comment", timed(5, || async { std::hint::black_box(handle.tool(call("pm", "comment", serde_json::json!({ "id": 50, "text": "Looks right." }))).await); }).await);
    steps(&handle);
    // A member's turn, start to end: its end is one step of the loop.
    let mut took = Vec::new();
    for _ in 0..3 {
        let at = Instant::now();
        handle.send(Command::Submit { session: 102, text: "carry on".into(), about: None });
        until(&mut events, |e| matches!(e, Event::TurnEnded { session: 102, .. })).await;
        took.push(at.elapsed().as_secs_f64() * 1000.);
    }
    took.sort_by(f64::total_cmp);
    say("loop: a member's turn (submit to end)", (took[1], took[2]));
    steps(&handle);
    handle.shutdown().await;
    let _ = std::fs::remove_dir_all(&root);
}
