//! A stand-in daemon for looking at the screens without spending a real
//! engine's turns: it answers `hello` with a squad (a PM, frontend, backend and
//! designer, three projects — real git repositories it makes under
//! `$TMPDIR/equipe-fake`, with a branch for each piece of work handed in) and a
//! board with work in every state, and answers every message with a streamed
//! reply — Markdown with a list, a table, a code block and `T-12` —
//! between tool steps. `interrupt` ends the turn at once; `focus` moves the PM's
//! thread; accept, merge, reopen, resume and cancel move tickets as the daemon
//! would — and so do the person's own create, edit, move and comment (by
//! `equipe_core::board`'s rules), each kept in the ticket's record.
//! `FAKE_EMPTY=1` starts with nothing waiting for the person;
//! `FAKE_PHONE_QA=<file>` ends the conversation on a phone test's report kept in that file;
//! `FAKE_CROWD=1` has nearly every project wait on the person (`fake_daemon/crowd.rs`);
//! `FAKE_BUSY=1` adds four members at work in web and more finished today;
//! `FAKE_SUBAGENTS=1` has frontend and backend hand work to subagents that come and go;
//! `FAKE_HELPERS=1` puts backend on Nemotron with equipe's helpers at work, one looking
//! stuck (`fake_daemon/helpers.rs`; `codex`: a quiet subagent of Codex's own instead);
//! `FAKE_WORKING=<n>` has only the first n of frontend and backend at work (`0`: nobody).
//! `FAKE_COMMAND=working|busy|nothing|asks|stuck|stuck_nothing`: a turn sits on a
//! command, the daemon's look at it shown (`fake_daemon/command.rs`).
//!
//! ```text
//! export EQUIPE_HOME=$(mktemp -d) EQUIPE_SQUAD=demo
//! cargo run -p equipe-desktop --example fake_daemon &
//! cargo run -p equipe-desktop
//! ```
//! `FAKE_COMPACT=auto|manual|long|queued|done|fail` has the PM's (and frontend's)
//! conversation being summarized when a screen attaches — `long` for 75 seconds
//! already, `queued` with two messages waiting; `done` and `fail` end it after
//! `FAKE_COMPACT_AFTER` ms (default 4000). "Try again" (`/compact`) starts it again.
//! `FAKE_SECONDS` (default 4) is how long a reply streams; `FAKE_SAID` adds a
//! last message of the person's to the PM's history (attachment tags and all).
//! `FAKE_CARDS=1` ends it on a reply naming T-42 (a review), T-117 and T-999
//! (no such ticket), to rest the pointer on; `FAKE_CARDS=long` gives T-42 a
//! hand-in long enough to scroll on its card, and with `FAKE_REPLY_IMAGES=1`
//! the reply names T-4 too — its pictures and all, a card taller than a small window.
//! `FAKE_REVIEW=1` gives T-42 and T-71 a made-up Nemotron review (canned, not a model's).
//! `FAKE_ASKLIST=1` ends the PM's history on a reply asking three things in a
//! numbered list of its own words, as a member does that skips `ask_decision`.
//!
//! `FAKE_TURN=1` puts T-117 down for the person — a question (the App Review
//! contact) and a thing to do (the EU DSA status in App Store Connect), as the
//! person once saw it — with its news last in the PM's conversation;
//! `FAKE_TURN=files` asks the older three, one naming a folder with spaces and
//! a file it made under the app's repository; `FAKE_TURN=elsewhere` also
//! answers it from "another screen" after `FAKE_TURN_AFTER` ms (default 5000).
//!
//! `FAKE_DECIDE=1` has the PM put a decision to the person — two questions,
//! the second several-pick — and backend one more about T-70;
//! `FAKE_DECIDE=elsewhere` answers the PM's from "another screen" after
//! `FAKE_DECIDE_AFTER` ms (default 5000). Answered here, the answers go to the
//! member as the person's message and it replies. `FAKE_DECIDE=logo` puts
//! the designer's logo question first, each option its picture, and has the
//! designer's brand guide (T-144) handed in with its pictures
//! (`fake_daemon/pictures.rs`); `FAKE_DECIDE=brand` puts the designer's three
//! brand directions first instead, each on the product's home screen, with
//! the aspects the person can mix.
//!
//! `FAKE_DECIDE=scope` puts the PM's scope card first: one question, the
//! plan step's four modes, its recommendation first. `FAKE_PLAN=1` has T-42
//! cut from a plan, its body ending in its slice of the test plan.
//!
//! `FAKE_REPLY_IMAGES=1`: the PM's last reply shows the designer's drafts
//! for T-2 — three pictures it embeds and the folder they are in — from "the
//! daemon's machine", through `read_image` and `list_images` only; and T-4's
//! hand-in has its renders and textures, T-6's pictures are gone
//! (`fake_daemon/replies.rs`; `broken`: the floor plan is missing).
//!
//! `FAKE_HISTORY=<n>`: the PM's history is a long day's, `n` lines of it
//! (`fake_daemon/history.rs`); `FAKE_LONG_REPLY=1`: its answers are long too.
//! `FAKE_HISTORY_PICTURES=<n>`: its screenshots are n different ones, and every
//! other reply embeds one too — for measuring what pictures hold (`EQUIPE_SNAPSHOT_HIDDEN`, `snapshot.rs`).
//! `FAKE_READING=1`: it ends on a long reply with every kind of text
//! (`fake_daemon/reading.rs`), to compare how the conversation reads.
//!
//! `FAKE_TEAM=1`: the PM proposes one more frontend, for the person to
//! approve on a card (`fake_daemon/team.rs`; `remove`, `elsewhere`).
//!
//! `FAKE_ROUTINE=1`: the PM proposes a weekly routine, for the person to turn
//! on from a card (`fake_daemon/routine.rs`; `daily`: a second one).
//!
//! `FAKE_DESIGN_REVIEW=1`: the designer's weekly design review has come — its
//! routine's message and the review in the designer's conversation, its
//! proposal waiting in the inbox as a visual question (`fake_daemon/review.rs`).
//!
//! `FAKE_CHECKS=1`: the designer's conversation checks a screen in code and has a
//! second look at it (`fake_daemon/checks.rs`); `FAKE_CHECKS=fail` stops at a failing check.
//! `FAKE_NOISE=1`: the designer's conversation holds the engine's own turns —
//! background tasks ending, a reminder, a `sleep` refused (`fake_daemon/noise.rs`).
//!
//! `FAKE_ACCEPT=1`: the PM puts T-42 and T-71 to the person on a card to
//! confirm accepting them (`fake_daemon/accept.rs`; `merge`, `words`).
//!
//! `FAKE_USAGE=healthy|claude86|claude_out|both_low`: both engines' plans,
//! and the card for moving members between them (`fake_daemon/engines.rs`;
//! `FAKE_SWITCHING=auto|off`, `FAKE_SETUP=1`); `FAKE_ENGINES_MIXED=1` puts
//! backend on Codex.
//!
//! `FAKE_BROWSER=1`: the shared Chrome's tabs and a picture stream for the
//! browser panel (`fake_daemon/browser.rs`); `FAKE_BROWSER=empty`, no tabs;
//! `FAKE_BROWSER=handover`, the PM's browser handed to the person to sign in
//! (its news line, as an English daemon wrote it, last in the PM's history).
//!
//! `FAKE_DISK=low|critical|full`: the disk runs short, with a realistic look
//! at what takes it and a clean that gives it back (`fake_daemon/disk.rs`).
//! `FAKE_ENGINES`: settings' AI platforms in each state (`fake_daemon/engines.rs`).
//! `FAKE_SIGNIN=outside|outside_stuck|locked|expired|failed|missing|unknown|back|checking|codex`:
//! the PM's last turn stopped by a lost sign-in, its card in each state
//! (`fake_daemon/signin.rs`).
//! `FAKE_DESIGN=1`: the squad's design system and the web project's own,
//! modelled on T-144's brand guideline; `draft`: T-144 in review with its
//! draft; `working`: T-378 still making the squad's UI map, the designer at
//! it; `empty`: none yet (`fake_daemon/design.rs`). Pictures come through
//! `read_image`, as from the daemon. `FAKE_NO_DESIGNER=1`: a team with no designer.
//!
//! Pairing a phone: a code at once, and two phones paired already.
//! `FAKE_PAIR=ask` has a phone ask 1.5 s after the code shows (`allow` pairs
//! it); `relay` fails as an unreachable relay would; `silent` never answers,
//! as a daemon older than the app.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use equipe_core::gateway;
use equipe_core::protocol::decide::{Answer, Choice, Question, QuestionSet};
use equipe_core::protocol::rpc::{self, Hello, Message};
use equipe_core::protocol::{Access, Approval, Command, CompactTrigger, CompactingInfo, Engine, Entry, Estimate, Event, Member, Priority, SessionId, SessionInfo, SessionState, Snapshot, SubagentInfo, Ticket, TicketEntry, TicketStatus, TurnOutcome, Usage, WorkInfo};
use equipe_providers::credentials::equipe_home;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, broadcast};

/// The language the daemon's own words are in: `EQUIPE_LANG`, as the screen
/// is run with, else English.
fn lang() -> equipe_core::i18n::Lang {
    std::env::var("EQUIPE_LANG").ok().and_then(|tag| equipe_core::i18n::Lang::parse(&tag)).unwrap_or(equipe_core::i18n::Lang::En)
}

const REPLY: &str = "Sure, got it. I'll look over the board now and take **T-12** first.\n\nHere's the order:\n\n- Tidy the header component\n- Check IME composition in the composer\n- Check the streaming display (`T-13`)\n\n| Item | Status | Owner |\n|---|---|---|\n| Header | In progress | frontend |\n| Composer | Waiting | frontend |\n| Daemon link | Done | backend |\n\n```rust\nfn main() {\n    println!(\"hello, equipe\");\n}\n```\n\nI'll let you know when it's done.";

/// `FAKE_ASKLIST`'s reply: what the PM needs to know, as a numbered list in prose.
const ASKLIST: &str = "Under the EU Digital Services Act (DSA), an app in the EU stores has to show its trader's contact details. They're still empty in App Store Connect, so the **T-117** submission is stuck here.\n\n## Please tell me\n\n1. The phone number to show in the EU stores, with the country code (e.g. +1 415 555 0134)\n2. The email to show: it appears on the store page as it is\n3. The T-117 review contact: whether the same number and email will do\n\nI'll enter them as soon as I have them.";

fn info(session: SessionId, role: &str, recent: Vec<Entry>) -> SessionInfo {
    SessionInfo {
        session,
        project: "/tmp/fake/site".into(),
        member: Some(Member { role: role.into(), name: role.into() }),
        engine: Engine::Claude,
        model: Some("claude-opus-5-5".into()),
        asked_model: None,
        access: Access::EditFiles,
        workspace: None,
        state: SessionState::Ready,
        title: None,
        tokens: 0,
        context: Some((310, 1000)),
        queued_project: None,
        queued_in: vec![],
        steering: vec![],
        steering_in: vec![],
        lane: None,
        estimate: None,
        recent,
        recent_at: Vec::new(),
        queued: vec![],
        work: None,
        mode: None,
        effort: None,
        subagents: Vec::new(),
        compacting: None,
        visuals: Vec::new(),
    }
}

/// Requests for permission still unanswered; a `decide` removes one. `FAKE_ASKS=0` starts with none.
/// The squad's "always allow" rules, as settings lists them.
/// The looks screens told (`rpc::SCREEN_LOOK`).
fn looks() -> &'static std::sync::Mutex<equipe_core::looks::Looks> {
    static LOOKS: std::sync::OnceLock<std::sync::Mutex<equipe_core::looks::Looks>> = std::sync::OnceLock::new();
    LOOKS.get_or_init(Default::default)
}

fn allowed() -> &'static std::sync::Mutex<Vec<String>> {
    static ALLOWED: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    ALLOWED.get_or_init(|| std::sync::Mutex::new(vec!["Bash(npm install:*)".into(), "Bash(gh:*)".into(), "WebFetch(domain:vercel.com)".into()]))
}

/// The phones paired, as `devices` lists them.
fn phones() -> &'static std::sync::Mutex<Vec<equipe_core::protocol::PairedDevice>> {
    static PHONES: std::sync::OnceLock<std::sync::Mutex<Vec<equipe_core::protocol::PairedDevice>>> = std::sync::OnceLock::new();
    PHONES.get_or_init(|| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let phone = |id: &str, name: &str, platform: &str, online: bool, seen: Option<u64>| equipe_core::protocol::PairedDevice { id: id.into(), name: name.into(), platform: platform.into(), added: now - 9 * 86_400, online, seen };
        std::sync::Mutex::new(vec![phone("a1", "Abel's iPhone", "ios", true, Some(now - 60)), phone("b2", "iPad Pro", "ipados", false, Some(now - 3 * 3600))])
    })
}

/// What the daemon would say to a pairing command.
async fn pairing(command: &Command, events: &broadcast::Sender<Event>) {
    use equipe_core::protocol::PairEnd;
    let mode = std::env::var("FAKE_PAIR").unwrap_or_default();
    if mode == "silent" {
        return;
    }
    let devices = || Event::Devices { devices: phones().lock().unwrap().clone(), relay: "https://relay.equipe.im".into(), connected: true };
    match command {
        Command::Devices => drop(events.send(devices())),
        Command::PairStart if mode == "relay" => {
            tokio::time::sleep(Duration::from_millis(600)).await;
            let _ = events.send(Event::PairEnded { why: PairEnd::Relay, message: "couldn't reach the relay: connection refused".into() });
        }
        Command::PairStart => {
            tokio::time::sleep(Duration::from_millis(400)).await;
            // Shaped as the daemon makes them; nothing in it opens anything.
            let url = "equipe://pair?r=https%3A%2F%2Frelay.equipe.im&d=5f0c2a9e41d7b3c86e1f2a4b9d0c7e35&k=Qm9vdHN0cmFwLWZha2UtZGFlbW9uLXB1YmxpYy1rZXk&s=ZmFrZS1zZWNyZXQtZm9yLXNjcmVlbnNob3RzLW9ubHkh&n=studio";
            let _ = events.send(Event::PairOffer { url: url.into(), seconds: 300 });
            if mode == "ask" {
                let events = events.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    let _ = events.send(Event::PairAsked { name: "Abel's iPhone 16".into(), platform: "ios".into() });
                });
            }
        }
        Command::PairAnswer { approve: true } => {
            tokio::time::sleep(Duration::from_millis(700)).await;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            phones().lock().unwrap().push(equipe_core::protocol::PairedDevice { id: "c3".into(), name: "Abel's iPhone 16".into(), platform: "ios".into(), added: now, online: true, seen: Some(now) });
            let _ = events.send(Event::Paired { name: "Abel's iPhone 16".into() });
        }
        Command::PairAnswer { approve: false } => drop(events.send(Event::PairEnded { why: PairEnd::Refused, message: String::new() })),
        Command::Unpair { device } => {
            phones().lock().unwrap().retain(|d| d.id != *device);
            let _ = events.send(devices());
        }
        _ => {}
    }
}

fn asks() -> &'static std::sync::Mutex<Vec<Approval>> {
    static ASKS: std::sync::OnceLock<std::sync::Mutex<Vec<Approval>>> = std::sync::OnceLock::new();
    ASKS.get_or_init(|| {
        let ask = |id, session, member: &str, tool: &str, action: &str, reason: Option<&str>| Approval {
            ticket: Some(if session == 3 { 70 } else { 47 }),
            id,
            session,
            member: member.into(),
            tool: tool.into(),
            action: action.into(),
            reason: reason.map(Into::into),
            preview: None,
        };
        let on = std::env::var("FAKE_ASKS").map_or(true, |v| v != "0") && !crowd::on();
        std::sync::Mutex::new(if on {
            vec![
                ask(7, 3, "backend", "Bash", "cargo test -p equipe-core -- --nocapture\ncat > /tmp/out.txt <<EOF\nx\nEOF", Some("Permission needed before running the tests")),
                ask(8, 2, "frontend", "Send", "press return in Slack · #general\nMoving tomorrow's meeting to 3pm", None),
            ]
        } else {
            vec![]
        })
    })
}

/// Where the fake repositories live.
fn root() -> std::path::PathBuf {
    std::env::temp_dir().join("equipe-fake")
}

fn repo(project: &str) -> std::path::PathBuf {
    root().join(project)
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let _ = std::process::Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=equipe", "-c", "user.email=fake@equipe.im", "-c", "commit.gpgsign=false"]).args(args).output();
}

type Branch = (&'static str, Vec<(&'static str, usize)>);

/// Conversations with the PM put away, the latest first: taken up again
/// from their project's page.
fn put_away() -> &'static std::sync::Mutex<Vec<equipe_core::protocol::PutAway>> {
    static AWAY: std::sync::OnceLock<std::sync::Mutex<Vec<equipe_core::protocol::PutAway>>> = std::sync::OnceLock::new();
    AWAY.get_or_init(|| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or_default();
        let away = |session, title: &str, ago: i64| equipe_core::protocol::PutAway { session, project: Some("web".into()), title: Some(title.into()), at: now - ago };
        std::sync::Mutex::new(vec![away(90, "Polish the landing copy", 3 * 3600), away(91, "Find the cause of the payment error", 2 * 86400)])
    })
}

/// `FAKE_REVIEW=1`: made-up Nemotron reviews of T-42 and T-71, to look at
/// how the peek shows one — canned here, never a real model's answer.
fn reviews() -> Vec<equipe_core::protocol::review::CodeReview> {
    use equipe_core::protocol::review::{CodeReview, ReviewPoint, ReviewVerdict};
    if std::env::var_os("FAKE_REVIEW").is_none() {
        return Vec::new();
    }
    vec![
        CodeReview {
            ticket: 42,
            model: "nvidia/fake-nemotron (FAKE_REVIEW)".into(),
            verdict: ReviewVerdict::NeedsAttention,
            summary: "Choosing a payment method works, but the code doesn't meet the criterion that the choice survives a refresh.".into(),
            risks: vec![
                ReviewPoint { file: "src/checkout.tsx".into(), line: Some(31), why: "The chosen method is only kept in state, so a refresh loses it.".into() },
                ReviewPoint { file: "src/api/pay.ts".into(), line: Some(12), why: "It sends the amount to the server straight from the client.".into() },
            ],
            missing_tests: vec!["The choice kept after a refresh".into(), "The layout at mobile width".into()],
            truncated: false,
            at: unix(),
        },
        CodeReview {
            ticket: 71,
            model: "nvidia/fake-nemotron (FAKE_REVIEW)".into(),
            verdict: ReviewVerdict::LooksGood,
            summary: "The token-bucket limit and the remaining-requests header are in, as the criteria ask.".into(),
            risks: Vec::new(),
            missing_tests: Vec::new(),
            truncated: false,
            at: unix(),
        },
    ]
}

/// T-42's hand-in: a few lines, or (`FAKE_CARDS=long`) one long enough to scroll on its card.
fn checkout_note() -> String {
    let short = "Payment method choice is now a **radio group**.\n\n- Card / bank transfer / wallet\n- The choice is kept in `localStorage`\n- Payment API calls are gathered in `pay.ts`";
    if std::env::var("FAKE_CARDS").is_ok_and(|v| v == "long") {
        let more: Vec<String> = (1..=14).map(|n| format!("{n}. Reworked screen {n} of the checkout flow — button spacing, error copy and keyboard order.")).collect();
        return format!("{short}\n\n## What changed\n\n{}\n\nThis is the last line.", more.join("\n"));
    }
    short.into()
}

/// A repository per project, each with a branch per piece of work handed in.
fn seed_repos() {
    let _ = std::fs::remove_dir_all(root());
    let projects: [(&str, Vec<Branch>); 3] = [
        ("web", vec![("equipe/t-42-checkout", vec![("src/checkout.tsx", 48), ("src/cart.ts", 12), ("src/api/pay.ts", 21)]), ("equipe/t-51-footer", vec![("src/footer.tsx", 9)]), ("equipe/t-49-banner", vec![("src/banner.tsx", 14), ("README.md", 2)])]),
        ("app", vec![]),
        ("api", vec![("equipe/t-71-rate-limit", vec![("src/limit.rs", 64), ("src/main.rs", 6), ("tests/limit.rs", 30)])]),
    ];
    for (project, branches) in projects {
        let dir = repo(project);
        let _ = std::fs::create_dir_all(dir.join("src"));
        git(&dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("README.md"), format!("# {project}\n")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "start"]);
        for (branch, files) in branches {
            git(&dir, &["checkout", "-q", "-b", branch]);
            for (file, lines) in files {
                let path = dir.join(file);
                let _ = std::fs::create_dir_all(path.parent().unwrap());
                let old = std::fs::read_to_string(&path).unwrap_or_default();
                let mut text: String = old.lines().skip(1).map(|l| format!("{l}\n")).collect();
                for n in 0..lines {
                    text.push_str(&format!("// {branch} line {n}: tidies the checkout flow\n"));
                }
                std::fs::write(path, text).unwrap();
            }
            git(&dir, &["add", "-A"]);
            git(&dir, &["commit", "-q", "-m", branch]);
            git(&dir, &["checkout", "-q", "main"]);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn ticket(id: u32, status: &str, project: &str, who: &str, title: &str, body: &str, deps: &[u32], branch: Option<&str>, note: Option<&str>) -> Ticket {
    serde_json::from_value(serde_json::json!({"id": id, "title": title, "body": body, "status": status, "owner": who, "assignee": who, "project": repo(project), "depends_on": deps, "branch": branch, "created_by": "pm", "note": note})).unwrap()
}

/// The board: work in every state, across three projects.
fn board() -> Vec<Ticket> {
    if crowd::on() {
        return crowd::tickets();
    }
    let empty = std::env::var("FAKE_EMPTY").is_ok_and(|v| v == "1");
    let mut out = vec![
        ticket(47, "in_progress", "web", "frontend", "Product detail page image gallery", "A gallery with swipe and zoom.", &[], Some("equipe/t-47-gallery"), None),
        ticket(45, "todo", "web", "frontend", "Go to order history after paying", "- A link to order history on the payment-done screen\n- Check the receipt email is sent", &[42], None, None),
        ticket(38, "done", "web", "frontend", "Tidy the header navigation", "", &[], None, Some("Tidied.")),
        ticket(36, "done", "web", "designer", "Sign-in screen draft", "", &[], None, None),
        ticket(60, "in_progress", "app", "designer", "Three onboarding screen drafts", "Three onboarding screens for the app's first run.", &[], None, None),
        ticket(61, "todo", "app", "frontend", "Rethink when to ask for push permission", "", &[], None, None),
        ticket(62, "todo", "app", "frontend", "Dark mode for the settings screen", "", &[60], None, None),
        ticket(58, "done", "app", "designer", "Three app icon candidates", "", &[], None, None),
        ticket(66, "cancelled", "app", "frontend", "Tablet layout", "", &[], None, None),
        ticket(70, "in_progress", "api", "backend", "Orders API pagination", "Cursor-based pagination.", &[], Some("equipe/t-70-cursor"), None),
        ticket(72, "todo", "api", "backend", "Order search index", "", &[70], None, None),
        ticket(68, "done", "api", "backend", "Health check endpoint", "", &[], None, None),
    ];
    if !empty {
        out.extend([
            ticket(42, "review", "web", "frontend", "Checkout page payment method choice", "- [ ] Both card and bank transfer can be chosen\n- [ ] The choice survives a refresh\n- [ ] Nothing breaks at mobile width", &[], Some("equipe/t-42-checkout"), Some(&checkout_note())),
            ticket(71, "review", "api", "backend", "Rate-limit middleware", "- [ ] 429 past 60 a minute\n- [ ] The remaining count in a header", &[], Some("equipe/t-71-rate-limit"), Some("Added a 60-a-minute limit with a token bucket. 6 tests added.")),
            ticket(55, "waiting", "web", "frontend", "Connect the payment test key", "Run a payment all the way through in Stripe's test mode.", &[], None, Some("Waiting on you:\n- [ ] Make a test key in the Stripe dashboard\n- [ ] Put STRIPE_KEY in `.env.local`\n- [ ] Add the same key to the Vercel project")),
            ticket(73, "waiting", "api", "backend", "Read-only production DB account", "", &[], None, Some("Waiting on you:\n- [ ] Share the production DB's read-only password")),
            ticket(52, "proposed", "web", "frontend", "Dark mode toggle", "Dark mode in the user settings would likely lengthen night-time use.\n\n- [ ] Follows the system setting\n- [ ] Can be picked by hand", &[], None, None),
            ticket(64, "proposed", "app", "designer", "Empty-state illustration set", "", &[], None, None),
            // The person's ideas, kept for later — and one the PM brought back.
            ticket(80, "idea", "app", "pm", "Run equipe on a cloud server", "Usable without a Mac mini: rent a server and hand it a whole computer.", &[], None, None),
            ticket(81, "idea", "web", "marketing", "Onboarding video guide", "A 3-minute video a first-time user can follow along.", &[], None, None),
            ticket(82, "proposed", "web", "frontend", "Weekly report email", "Last week's metrics by email, every Monday.", &[], None, Some("Why now: the weekly metrics dashboard just shipped, so this can be built from the same data right away. Some members are free now, too.")),
        ]);
        let mut task = ticket(63, "review", "app", "designer", "Competitor onboarding research", "", &[], None, Some("Summed up five apps' onboarding. `research/onboarding.md`"));
        task.kind = equipe_core::protocol::TicketKind::Task;
        out.push(task);
        out.extend(design::ticket());
        let mut done = ticket(51, "done", "web", "frontend", "Tidy the footer links", "", &[], Some("equipe/t-51-footer"), Some("Tidied the links."));
        done.merged = Some(false);
        out.push(done);
        let mut clash = ticket(49, "done", "web", "frontend", "Announcement banner", "", &[], Some("equipe/t-49-banner"), Some("Added the announcement banner at the top."));
        clash.merged = Some(false);
        clash.off_main = Some(equipe_core::protocol::OffMain::Conflict { detail: "equipe/t-49-banner conflicts with main in README.md — nothing was merged".into() });
        out.push(clash);
    }
    if std::env::var("FAKE_PLAN").is_ok_and(|v| v == "1")
        && let Some(t) = out.iter_mut().find(|t| t.id == 42)
    {
        let plan = briefs::folder().join("docs/web/plans/2026-11-04-checkout-payment.md").to_string_lossy().into_owned();
        t.plan = Some(plan.clone());
        t.body.push_str(&format!("\n\nPlan: `{plan}`\n\nThis ticket's test plan:\n\n- TP-1: choosing a card and paying creates an order\n- TP-3: the chosen payment method survives a refresh"));
    }
    if your_turn().is_some() {
        out.push(turn_ticket());
    }
    if replies::on() {
        out.extend(replies::tickets(|id, title, note| ticket(id, "review", "web", "frontend", title, "", &[], Some("equipe/frontend-t4-6ad33ac5"), Some(note))));
    }
    // `FAKE_DECIDE=logo`: the designer's brand guide, handed in with its pictures.
    if pictures::on() {
        let note = "I put the brand guide in order — the name and logo stay as they are, and how they're used is now consistent.\n\n- Logo sheet: on white and on dark\n- App icon at 1024px\n- Five colours (`brand/palette.md`)";
        out.push(pictures::hand_in(ticket(144, "review", "app", "designer", "equipe brand clean-up and system (name and logo kept)", "- [ ] The logo reads at small sizes\n- [ ] Colour contrast is enough", &[], None, Some(note))));
    }
    if busy() {
        out.extend(BUSY.iter().map(|(id, _, who, title)| ticket(*id, "in_progress", "web", who, title, "", &[], None, None)));
        out.extend([
            ticket(84, "waiting", "web", "frontend", "Check the domain's DNS records", "", &[], None, Some("Waiting on you:\n- [ ] Add a CNAME to the DNS")),
            ticket(85, "todo", "web", "frontend", "Cart quantity button accessibility", "", &[], None, None),
            ticket(86, "todo", "web", "qa", "Checkout regression test list", "", &[], None, None),
            ticket(87, "done", "web", "qa", "Sign-in error message copy", "", &[], None, None),
            ticket(88, "done", "app", "designer", "Splash screen colour tweak", "", &[], None, None),
            ticket(89, "done", "api", "backend", "Order status webhook", "", &[], None, None),
        ]);
        // Three finished within the hour or two, the rest days ago.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        for t in out.iter_mut() {
            t.updated_at = Some(match t.id {
                87 => now - 600,
                88 => now - 3_600,
                89 => now - 2 * 3_600,
                _ if t.status.is_closed() => now - 2 * 86_400,
                _ => now - 25 * 60,
            });
        }
    }
    out
}

/// `FAKE_DECIDE`: the PM's two questions and backend's one wait for the person.
fn deciding() -> Option<String> {
    std::env::var("FAKE_DECIDE").ok().filter(|v| !v.is_empty() && v != "0")
}

/// The question sets still waiting.
fn questions() -> &'static std::sync::Mutex<Vec<QuestionSet>> {
    static QUESTIONS: std::sync::OnceLock<std::sync::Mutex<Vec<QuestionSet>>> = std::sync::OnceLock::new();
    QUESTIONS.get_or_init(|| {
        let choice = |label: &str, description: &str| Choice { label: label.into(), description: description.into(), ..Choice::default() };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let sets = vec![
            QuestionSet {
                id: 9001,
                session: 1,
                member: "pm".into(),
                ticket: None,
                questions: vec![
                    Question {
                        question: "How should sign-in work?".into(),
                        header: "Sign-in".into(),
                        options: vec![
                            choice("Email magic link (Recommended)", "Sign in from a link in an email, no password — the quickest to build"),
                            choice("Sign in with Google", "Sign-up in one step, but it needs a Google account"),
                            choice("Email + password", "Familiar, but the reset flow has to be built too"),
                        ],
                        multi_select: false,
                        ..Question::default()
                    },
                    Question {
                        question: "Which screens go in the first release?".into(),
                        header: "Scope".into(),
                        options: vec![
                            choice("Onboarding", "Three screens — the drafts are on T-60"),
                            choice("Order history", "After api's pagination (T-70)"),
                            choice("Settings", "Notifications and account"),
                        ],
                        multi_select: true,
                        ..Question::default()
                    },
                ],
                asked_at: now.saturating_sub(90),
                review: false,
                retro: None,
            },
            QuestionSet {
                id: 9002,
                session: 3,
                member: "backend".into(),
                ticket: Some(70),
                questions: vec![Question {
                    question: "What page size should the order list use?".into(),
                    header: "Page".into(),
                    options: vec![choice("20 (Recommended)", "Fits the app's list screen in one go"), choice("50", "Fewer requests, but a slower first screen")],
                    multi_select: false,
                    ..Question::default()
                }],
                asked_at: now.saturating_sub(30),
                review: false,
                retro: None,
            },
        ];
        let sets = if pictures::on() { std::iter::once(pictures::question_set(now)).chain(sets).collect() } else { sets };
        let sets = if deciding().as_deref() == Some("scope") { std::iter::once(scope_set(now)).chain(sets).collect() } else { sets };
        let mut waiting = if briefs::office() { vec![briefs::office_hours_set(now)] } else if deciding().is_some() { sets } else { Vec::new() };
        if review::on() {
            waiting.insert(0, review::set(now));
        }
        if retro::on() {
            waiting.insert(0, retro::set(now));
        }
        std::sync::Mutex::new(waiting)
    })
}

/// `FAKE_DECIDE=scope`: the plan step's scope card, as the PM puts it.
fn scope_set(now: u64) -> QuestionSet {
    let choice = |label: &str, description: &str| Choice { label: label.into(), description: description.into(), ..Choice::default() };
    QuestionSet {
        id: 9000,
        session: 1,
        member: "pm".into(),
        ticket: None,
        questions: vec![Question {
            question: "The checkout payment methods plan: how wide should the scope be?".into(),
            header: "Scope".into(),
            options: vec![
                choice("Widen selectively (Recommended)", "One wallet on top of the card and bank transfer asked for — the most requested thing in reviews"),
                choice("Widen", "Three wallets and saved payment methods too — 7 tickets, about two weeks"),
                choice("Keep the scope", "Card and bank transfer only, as asked — 4 tickets"),
                choice("Narrow", "Ship card payments first, bank transfer later — 2 tickets"),
            ],
            multi_select: false,
            ..Question::default()
        }],
        asked_at: now.saturating_sub(20),
        review: false,
        retro: None,
    }
}

/// A set answered or closed: the screens are told, and the member gets the
/// answers as the person's message and replies.
fn close_questions(id: u64, answers: Option<Vec<Answer>>, events: &broadcast::Sender<Event>) -> Option<(SessionId, String)> {
    let set = {
        let mut all = questions().lock().unwrap();
        let at = all.iter().position(|s| s.id == id)?;
        all.remove(at)
    };
    let text = match &answers {
        Some(answers) => equipe_core::protocol::decide::answer_message(&set, answers),
        None => equipe_core::protocol::decide::dismissed_message(&set),
    };
    let _ = events.send(Event::QuestionsClosed { squad: "demo".into(), id, answers });
    let _ = events.send(Event::UserMessage { session: set.session, text: text.clone() });
    Some((set.session, text))
}

/// `FAKE_BUSY=1`: four more members, each at work on a web ticket, and more
/// waiting and finished there — a project with more live work than the
/// sidebar shows, a team taller than its block.
fn busy() -> bool {
    std::env::var("FAKE_BUSY").is_ok_and(|v| v == "1")
}

/// `FAKE_WORKING=<n>`: how many of frontend and backend are at work when a
/// screen attaches (default both; `0`: nothing turns anywhere).
fn working() -> u32 {
    std::env::var("FAKE_WORKING").ok().and_then(|v| v.parse().ok()).unwrap_or(2)
}

/// The busy squad's extra members: (ticket, session, name, title).
const BUSY: [(u32, SessionId, &str, &str); 4] = [
    (80, 5, "frontend-2", "Infinite scroll for search results"),
    (81, 6, "frontend-3", "Product card skeleton loading, lazy image loading and accessibility labels, all in one go"),
    (82, 7, "qa", "Checkout end-to-end tests"),
    (83, 8, "data", "Purchase conversion dashboard events"),
];

/// `FAKE_SUBAGENTS=1`: frontend and backend hand side work to subagents, which
/// start, take steps and hand back over time, round and round — backend's
/// first already at it when a screen attaches (in the snapshot, as a daemon
/// lists them). `FAKE_SUBAGENTS_MS` (default 1500) is the beat.
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn subagents_on() -> bool {
    std::env::var("FAKE_SUBAGENTS").is_ok_and(|v| v == "1")
}

/// The subagents at work now, by session: what a snapshot lists.
fn live_subagents() -> &'static std::sync::Mutex<Vec<(SessionId, SubagentInfo, std::time::Instant)>> {
    static LIVE: std::sync::OnceLock<std::sync::Mutex<Vec<(SessionId, SubagentInfo, std::time::Instant)>>> = std::sync::OnceLock::new();
    LIVE.get_or_init(|| {
        // Backend's, at it since before anyone looked.
        let since = std::time::Instant::now() - Duration::from_secs(48);
        std::sync::Mutex::new(vec![(3, SubagentInfo { id: "sub-b0".into(), what: "Find the cause of timeouts in the API error logs".into(), step: Some("Grep TimeoutError logs/".into()), secs: 0, ..Default::default() }, since)])
    })
}

/// A session's subagents as a snapshot carries them, and the delegations
/// that started them, for its conversation.
fn subagents_of(session: SessionId) -> (Vec<SubagentInfo>, Vec<Entry>) {
    let live = live_subagents().lock().unwrap();
    let subs: Vec<SubagentInfo> = live.iter().filter(|(s, ..)| *s == session).map(|(_, info, since)| SubagentInfo { secs: since.elapsed().as_secs(), ..info.clone() }).collect();
    let calls = subs.iter().map(|s| Entry::Tool { tool: "Agent".into(), target: Some(s.what.clone()), outcome: None, label: None, background: false }).collect();
    (subs, calls)
}

/// Starts, steps and ends, `beat` apart, round and round: (session, id, what), a step, or an end.
async fn subagents_play(events: broadcast::Sender<Event>) {
    let beat = Duration::from_millis(std::env::var("FAKE_SUBAGENTS_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500));
    let tasks: [(SessionId, &str, &[&str]); 6] = [
        (2, "Look into the gallery thumbnails' size and ratio", &["Read src/gallery.tsx", "Grep aspect-ratio", "Read src/thumb.tsx"]),
        (2, "Check the mobile swipe gesture", &["Read src/swipe.ts", "Bash npm test swipe"]),
        (3, "Compare pagination cursor encodings", &["Read api/cursor.rs", "WebFetch RFC 4648"]),
        (2, "Write the lazy image loading tests", &["Write src/gallery.test.tsx", "Bash npm test gallery"]),
        (2, "Check accessibility labels and keyboard navigation", &["Read src/gallery.tsx", "Grep aria-label"]),
        (2, "Measure the change in bundle size", &["Bash npm run build", "Read dist/stats.json"]),
    ];
    let mut round = 0u32;
    loop {
        let mut started: Vec<(SessionId, String, usize)> = Vec::new();
        for (n, (session, what, _)) in tasks.iter().enumerate() {
            let id = format!("sub-{round}-{n}");
            let _ = events.send(Event::ToolUse { session: *session, tool: "Agent".into(), target: Some((*what).into()), label: None, background: false });
            let brief = format!("{what}.\n\nRead the code and hand back what you find, with file paths and line numbers. Don't fix anything — just gather the evidence.");
            live_subagents().lock().unwrap().push((*session, SubagentInfo { id: id.clone(), what: (*what).into(), step: None, secs: 0, brief: brief.clone(), ..Default::default() }, std::time::Instant::now()));
            let _ = events.send(Event::SubagentStarted { session: *session, id: id.clone(), what: (*what).into(), brief });
            started.push((*session, id, n));
            tokio::time::sleep(beat).await;
            // Everyone at work takes a step.
            for (s, id, task) in &started {
                let steps = tasks[*task].2;
                let step = steps[(round as usize + n + task) % steps.len()];
                // Its last step done (with what it printed), now and then a word, then the next step.
                let (tool, target) = step.split_once(' ').unwrap_or((step, ""));
                let done = Entry::Tool {
                    tool: tool.into(),
                    target: Some(target.into()),
                    outcome: Some(Box::new(equipe_core::protocol::ToolOutcome { ok: true, exit: None, ms: Some(400 + 137 * (*task as u64 + 1)), output: format!("{target}\n  12 | const size = 240;\n  13 | const ratio = 4 / 3;"), skipped: 0, diff: None })),
                    label: None,
                    background: false,
                };
                let said = (n % 2 == 0).then(|| Entry::Reply { text: format!("Looked at `{target}`. Next, the tests around it.") });
                if let Some(entry) = live_subagents().lock().unwrap().iter_mut().find(|(_, info, _)| info.id == *id) {
                    entry.1.step = Some(step.into());
                    entry.1.entries.push(done.clone());
                    entry.1.entries.extend(said.clone());
                }
                let _ = events.send(Event::SubagentEntry { session: *s, id: id.clone(), entry: done });
                if let Some(said) = said {
                    let _ = events.send(Event::SubagentEntry { session: *s, id: id.clone(), entry: said });
                }
                let _ = events.send(Event::SubagentStep { session: *s, id: id.clone(), step: step.into() });
            }
            // From the sixth on, the oldest hands back as the next starts.
            if n >= 5 {
                let (s, id, _) = started.remove(0);
                end_subagent(&events, s, &id);
            }
        }
        tokio::time::sleep(beat * 2).await;
        for (s, id, _) in started.drain(..) {
            end_subagent(&events, s, &id);
            tokio::time::sleep(beat / 2).await;
        }
        // Backend's first, from before, hands back once.
        if round == 0 {
            end_subagent(&events, 3, "sub-b0");
        }
        tokio::time::sleep(beat * 2).await;
        round += 1;
    }
}

fn end_subagent(events: &broadcast::Sender<Event>, session: SessionId, id: &str) {
    let what = {
        let mut live = live_subagents().lock().unwrap();
        let what = live.iter().find(|(_, info, _)| info.id == id).map(|(_, info, _)| info.what.clone());
        live.retain(|(_, info, _)| info.id != id);
        what
    };
    let outcome = equipe_core::protocol::ToolOutcome { ok: true, exit: None, ms: Some(9_000), output: String::new(), skipped: 0, diff: None };
    let _ = events.send(Event::ToolResult { session, tool: "Agent".into(), target: what.clone(), outcome });
    let result = what.as_ref().map(|w| format!("**{w}** — done.\n\n- `src/gallery.tsx:12` thumbnails fixed at 240px\n- `src/thumb.tsx:13` ratio 4:3"));
    let _ = events.send(Event::SubagentEnded { session, id: id.into(), result, failed: false });
}

/// `FAKE_TURN`: T-117 waits for the person (`1`), and is answered elsewhere (`elsewhere`).
fn your_turn() -> Option<String> {
    std::env::var("FAKE_TURN").ok().filter(|v| !v.is_empty() && v != "0")
}

/// What T-117 asks: as the person saw it — one question, one thing to do.
const TURN_ASKS: [&str; 2] = [
    "Tell me the App Review contact: a phone number (with the country code) and an email. Only the review team sees them; they aren't public.",
    "In App Store Connect, set the EU DSA status to trader — App Information › Digital Services Act.",
];

/// What T-117 asked before (`FAKE_TURN=files`): as the person once saw it joined into one line.
const TURN_ASKS_FILES: [&str; 3] = [
    "Tell me the review contact (App Review Information): a name, a phone number (with the country code) and an email. Only the review team sees them; they aren't public.",
    "Decide when, and by whom, the company's iOS engineer review happens. equipe-app's rules require it before a store release.",
    "Check the screenshots and copy: outputs/T-117 App Store submission/screenshots-6.9/ (5 each in en and fr, real app screens) and listing.json. The ticket titles in the fr screenshots are demo data, so they're in English. If they can stay as they are, just answer 'fine'.",
];

/// T-117, waiting, with the folder and file its third ask names on disk.
fn turn_ticket() -> Ticket {
    let shots = repo("app").join("outputs/T-117 App Store submission/screenshots-6.9");
    let _ = std::fs::create_dir_all(&shots);
    let _ = std::fs::write(repo("app").join("listing.json"), "{ \"name\": \"equipe\" }\n");
    let files = your_turn().as_deref() == Some("files");
    let asks: Vec<String> = if files { TURN_ASKS_FILES.iter().map(|a| (*a).to_owned()).collect() } else { TURN_ASKS.iter().map(|a| (*a).to_owned()).collect() };
    let note = equipe_core::protocol::asks::waiting_note(&asks, Some("Needed for the App Store review submission"));
    let mut t = ticket(117, "waiting", "app", "frontend", "App Store submission", "Submit 1.0 in App Store Connect.", &[], None, Some(&note));
    t.kind = equipe_core::protocol::TicketKind::Task;
    t.asks = asks;
    t
}

fn tickets() -> &'static std::sync::Mutex<Vec<Ticket>> {
    static TICKETS: std::sync::OnceLock<std::sync::Mutex<Vec<Ticket>>> = std::sync::OnceLock::new();
    TICKETS.get_or_init(|| {
        let now = unix();
        let mut all = board();
        // How pressing, and when each last moved: a spread, as a real board has.
        for t in &mut all {
            t.priority = match t.id {
                42 | 55 | 72 => Priority::Urgent,
                47 | 70 | 61 | 52 => Priority::High,
                62 | 64 | 66 => Priority::Low,
                _ => Priority::Normal,
            };
            t.updated_at = Some(now - u64::from(t.id % 9) * 3_400 - if t.status.is_closed() { u64::from(t.id % 5) * 86_400 } else { 0 });
        }
        std::sync::Mutex::new(all)
    })
}

fn unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// When each line was said, as a conversation spreads out over days: up to
/// `latest`, the last exchange minutes before it, the ones before hours
/// before, the day before, days back, then last year; in an exchange, the
/// message, then its steps and reply a little after.
fn said_at(lines: &[Entry], latest: u64) -> Vec<Option<u64>> {
    const AGO: [u64; 8] = [4 * 60, 47 * 60, 5 * 3600 + 20 * 60, 26 * 3600, 3 * 86_400 + 2 * 3600, 6 * 86_400, 12 * 86_400, 400 * 86_400];
    let mut out = vec![None; lines.len()];
    let mut exchange = 0;
    let mut after = 0;
    for (ix, line) in lines.iter().enumerate().rev() {
        let base = latest.saturating_sub(AGO[exchange.min(AGO.len() - 1)]);
        out[ix] = Some(match line {
            Entry::User { .. } => base.saturating_sub(after + 30),
            _ => base.saturating_sub(after),
        });
        after += 15;
        if matches!(line, Entry::User { .. }) {
            exchange += 1;
            after = 0;
        }
    }
    out
}

/// Each ticket's record, as `ticket_history` answers it.
fn records() -> &'static std::sync::Mutex<HashMap<u32, Vec<TicketEntry>>> {
    static RECORDS: std::sync::OnceLock<std::sync::Mutex<HashMap<u32, Vec<TicketEntry>>>> = std::sync::OnceLock::new();
    RECORDS.get_or_init(|| {
        let now = unix();
        let e = |ago: u64, by: &str, kind: &str, text: &str| TicketEntry { at: now - ago, by: by.into(), kind: kind.into(), text: text.into() };
        let mut out = HashMap::new();
        out.insert(
            42,
            vec![
                e(3 * 86_400, "pm", "created", "Checkout page payment method choice"),
                e(3 * 86_400 - 60, "equipe", "assigned", "frontend"),
                e(2 * 86_400, "you", "edited", "Priority normal → urgent"),
                e(86_400, "you", "comment", "For wallets, add only Apple Pay first, please."),
                e(5_400, "frontend", "handed in", "Payment method choice is now a radio group."),
            ],
        );
        out.insert(47, vec![e(2 * 86_400, "pm", "created", "Product detail page image gallery"), e(86_400, "equipe", "assigned", "frontend")]);
        std::sync::Mutex::new(out)
    })
}

fn record(id: u32, kind: &str, text: &str) {
    records().lock().unwrap().entry(id).or_default().push(TicketEntry { at: unix(), by: "you".into(), kind: kind.into(), text: text.into() });
}

/// The person's own hand on the board, as the daemon does it: what changed,
/// or why not (an error, as the daemon says it).
fn on_board_command(command: &Command) -> Option<Vec<Event>> {
    let squad = || "demo".to_owned();
    let changed = |t: &Ticket| Event::TicketChanged { squad: squad(), ticket: Ticket { updated_at: Some(unix()), ..t.clone() } };
    let refuse = |message: String| vec![Event::Error { session: None, message, next: None }];
    // Several at once: each as one, then a single word.
    if let Command::EditTickets { squad, tickets, changes } = command {
        let each = tickets.iter().flat_map(|id| on_board_command(&Command::EditTicket { squad: squad.clone(), ticket: *id, changes: changes.clone() }).unwrap_or_default());
        return Some(each.collect());
    }
    let mut all = tickets().lock().unwrap();
    match command {
        Command::CreateTicket { draft, .. } => {
            if draft.title.trim().is_empty() {
                return Some(refuse("A ticket needs a title.".into()));
            }
            let id = all.iter().map(|t| t.id).max().unwrap_or(0) + 1;
            let mut t = ticket(id, "todo", "web", "", draft.title.trim(), &draft.body, &draft.depends_on, None, None);
            t.owner = draft.assignee.clone();
            t.assignee = None;
            t.project = draft.project.clone().or_else(|| Some(repo("web")));
            t.created_by = "you".into();
            t.priority = draft.priority;
            if let Some(kind) = draft.kind {
                t.kind = kind;
            }
            all.push(t.clone());
            record(id, "created", &t.title);
            Some(vec![changed(&t)])
        }
        Command::EditTicket { ticket, changes, .. } => {
            if let Some(deps) = &changes.depends_on
                && let Some(path) = equipe_core::board::dependency_loop(&all, *ticket, deps)
            {
                let path: Vec<String> = path.iter().map(|id| format!("T-{id}")).collect();
                return Some(refuse(format!("That makes a loop of tickets waiting on each other: {}.", path.join(" → "))));
            }
            let t = all.iter_mut().find(|t| t.id == *ticket)?;
            if let Some(title) = &changes.title {
                t.title = title.trim().to_owned();
            }
            if let Some(body) = &changes.body {
                t.body = body.trim().to_owned();
            }
            if let Some(p) = changes.priority {
                t.priority = p;
            }
            if let Some(owner) = &changes.assignee {
                t.owner = owner.clone();
                t.assignee = owner.clone().filter(|_| t.status == TicketStatus::InProgress);
            }
            if let Some(project) = &changes.project {
                t.project = project.clone();
            }
            if let Some(kind) = changes.kind {
                t.kind = kind;
            }
            if let Some(deps) = &changes.depends_on {
                t.depends_on = deps.clone();
            }
            let t = t.clone();
            record(t.id, "edited", "");
            Some(vec![changed(&t)])
        }
        Command::MoveTicket { ticket, status, before, .. } => {
            let found = all.iter().find(|t| t.id == *ticket)?.clone();
            let to = status.unwrap_or(found.status);
            let rule = match equipe_core::board::move_rule(found.status, to) {
                Ok(rule) => rule,
                Err(why) => return Some(refuse(format!("T-{ticket}: {}", why.code()))),
            };
            let mut out = Vec::new();
            let placed = {
                let mut column: Vec<&Ticket> = all.iter().filter(|t| t.status == to && t.id != *ticket).collect();
                equipe_core::board::board_order(&mut column);
                equipe_core::board::rank_before(&column, *before).ok_or_else(|| equipe_core::board::spread(&column))
            };
            let rank = match placed {
                Ok(rank) => rank,
                Err(spread) => {
                    for (id, r) in &spread {
                        if let Some(t) = all.iter_mut().find(|t| t.id == *id) {
                            t.rank = *r;
                            out.push(changed(t));
                        }
                    }
                    let mut column: Vec<&Ticket> = all.iter().filter(|t| t.status == to && t.id != *ticket).collect();
                    equipe_core::board::board_order(&mut column);
                    equipe_core::board::rank_before(&column, *before).unwrap_or(0)
                }
            };
            let t = all.iter_mut().find(|t| t.id == *ticket)?;
            t.rank = rank;
            use equipe_core::board::Move;
            t.status = match rule {
                Move::Reorder => t.status,
                Move::Accept => TicketStatus::Done,
                Move::Cancel => TicketStatus::Cancelled,
                Move::Resume | Move::SendBack | Move::TakeOn => TicketStatus::Todo,
                Move::BackToProposal => TicketStatus::Proposed,
                Move::Shelve => TicketStatus::Idea,
                Move::Restore(s) => s,
            };
            if t.status != found.status {
                record(t.id, "moved", t.status.label());
            }
            out.push(changed(t));
            Some(out)
        }
        Command::CommentTicket { ticket, text, .. } => {
            record(*ticket, "comment", text);
            let entries = records().lock().unwrap().get(ticket).cloned().unwrap_or_default();
            Some(vec![Event::TicketHistory { squad: squad(), ticket: *ticket, entries }])
        }
        Command::TicketHistory { ticket, .. } => {
            let entries = records().lock().unwrap().get(ticket).cloned().unwrap_or_default();
            Some(vec![Event::TicketHistory { squad: squad(), ticket: *ticket, entries }])
        }
        _ => None,
    }
}


/// A command about a ticket, done as the daemon would: the ticket as it is now.
/// Commands a decision sends — a card confirmed, words decided — run as the
/// board's buttons run on the fake board.
fn run_commands(commands: Vec<Command>, events: &broadcast::Sender<Event>) {
    for command in commands {
        if let Some(more) = accept::on_command(&command, events) {
            run_commands(more, events);
            continue;
        }
        if let Some(said) = on_board_command(&command) {
            for event in said {
                let _ = events.send(event);
            }
        } else if let Some(ticket) = on_ticket_command(&command) {
            let _ = events.send(Event::TicketChanged { squad: "demo".into(), ticket });
        }
    }
}

fn on_ticket_command(command: &Command) -> Option<Ticket> {
    let mut all = tickets().lock().unwrap();
    let id = match command {
        Command::Accept { ticket, .. } | Command::Merge { ticket, .. } | Command::KeepOnBranch { ticket, .. } | Command::Reopen { ticket, .. } | Command::Resume { ticket, .. } | Command::Cancel { ticket, .. } => *ticket,
        _ => return None,
    };
    let t = all.iter_mut().find(|t| t.id == id)?;
    match command {
        Command::Accept { .. } if t.status == TicketStatus::Proposed => t.status = TicketStatus::Todo,
        Command::Accept { .. } => {
            t.status = TicketStatus::Done;
            t.merged = t.branch.as_ref().map(|_| false);
        }
        Command::Merge { .. } => {
            t.merged = Some(true);
            t.off_main = None;
        }
        Command::KeepOnBranch { .. } => t.off_main = Some(equipe_core::protocol::OffMain::Kept),
        Command::Reopen { note, .. } => {
            t.status = TicketStatus::InProgress;
            t.note = Some(note.clone());
        }
        Command::Resume { note, .. } => {
            // What the answers leave open stays on the ticket, as the daemon keeps it.
            t.remaining = equipe_core::protocol::asks::still_open(&t.asks(), note);
            t.status = TicketStatus::InProgress;
            t.note = Some(if note.trim().is_empty() { "Resumed: the person did what you were waiting for.".into() } else { format!("Resumed: {note}") });
            t.asks.clear();
        }
        _ => t.status = TicketStatus::Cancelled,
    }
    Some(t.clone())
}

/// A member at work on a ticket: its conversation starts with the work's header.
fn on(mut info: SessionInfo, ticket: u32, project: &str, title: &str, working: bool, tool: (&str, &str), estimate: Option<Estimate>) -> SessionInfo {
    info.work = Some(WorkInfo { ticket, project: repo(project), workspace: None });
    info.state = if working { SessionState::Working } else { SessionState::Ready };
    info.recent = vec![
        Entry::Note { text: format!("── T-{ticket} · {title} — on branch equipe/t-{ticket}") },
        Entry::User { text: format!("Please start T-{ticket}.") },
        Entry::Reply { text: "Sure, I'll look at the structure first. I'll reuse the existing components and write the tests first.".into() },
        Entry::Tool { tool: tool.0.into(), target: Some(if tool.0 == "Bash" { tool.1.to_owned() } else { format!("{}/{}", repo(project).display(), tool.1) }), outcome: None, label: None, background: false },
    ];
    info.estimate = estimate;
    info
}

#[path = "fake_daemon/browser.rs"]
mod browser;

#[path = "fake_daemon/torture.rs"]
mod torture;
#[path = "fake_daemon/history.rs"]
mod history;
#[path = "fake_daemon/reading.rs"]
mod reading;

#[path = "fake_daemon/pictures.rs"]
mod pictures;
#[path = "fake_daemon/team.rs"]
mod team;
#[path = "fake_daemon/routine.rs"]
mod routine;
#[path = "fake_daemon/signals.rs"]
mod signals;

#[path = "fake_daemon/accept.rs"]
mod accept;

#[path = "fake_daemon/replies.rs"]
mod replies;
#[path = "fake_daemon/engines.rs"]
mod engines;

#[path = "fake_daemon/disk.rs"]
mod disk;
#[path = "fake_daemon/command.rs"]
mod command;
#[path = "fake_daemon/helpers.rs"]
mod helpers;

#[path = "fake_daemon/switching.rs"]
mod switching;

#[path = "fake_daemon/design.rs"]
mod design;

#[path = "fake_daemon/briefs.rs"]
mod briefs;

#[path = "fake_daemon/flows.rs"]
mod flows;

#[path = "fake_daemon/review.rs"]
mod review;

#[path = "fake_daemon/retro.rs"]
mod retro;

#[path = "fake_daemon/noise.rs"]
mod noise;
#[path = "fake_daemon/checks.rs"]
mod checks;

#[path = "fake_daemon/signin.rs"]
mod signin;

#[path = "fake_daemon/crowd.rs"]
mod crowd;

// Deciding in the person's own words, as the daemon reads them: `FAKE_WORDS`, `FAKE_WORDS_LANG`.
#[path = "fake_daemon/words.rs"]
mod words;

// A phone test's report, at the conversation's end: `FAKE_PHONE_QA=<file with its note>`.
#[path = "fake_daemon/phone_qa.rs"]
mod phone_qa;

type Turns = Arc<Mutex<HashMap<SessionId, tokio::task::JoinHandle<()>>>>;

/// What waits for each member at work: its words and project, in order.
type Waits = Mutex<HashMap<SessionId, Vec<(String, Option<String>)>>>;

fn waiting() -> &'static Waits {
    static WAITING: std::sync::OnceLock<Waits> = std::sync::OnceLock::new();
    WAITING.get_or_init(Mutex::default)
}

/// Where a message goes, as the daemon decides: the project in focus, each
/// member's current conversation, and the lanes opened beside them.
#[derive(Default)]
struct Routes {
    focus: Option<String>,
    thread: HashMap<SessionId, Option<String>>,
    /// `(lane session, member session, project)`.
    lanes: Vec<(SessionId, SessionId, Option<String>)>,
    /// The conversations the person opened with the PM: said to as they are.
    kept: std::collections::HashSet<SessionId>,
}

type Shared = Arc<Mutex<Routes>>;

#[tokio::main]
async fn main() {
    let home = equipe_home();
    let socket = gateway::socket_path(&home);
    let _ = std::fs::create_dir_all(&home);
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind the fake daemon's socket");
    seed_repos();
    design::seed();
    briefs::seed();
    if pictures::on() || review::on() {
        pictures::make();
    }
    replies::make();
    eprintln!("fake daemon on {}", socket.display());
    let seconds: f32 = std::env::var("FAKE_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(4.);
    let (events, _) = broadcast::channel::<Event>(1024);
    let turns: Turns = Arc::default();
    let routes: Shared = Arc::default();
    loop {
        let (stream, _) = listener.accept().await.unwrap();
        tokio::spawn(serve(stream, events.clone(), turns.clone(), routes.clone(), seconds));
    }
}

async fn serve(stream: UnixStream, events: broadcast::Sender<Event>, turns: Turns, routes: Shared, seconds: f32) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let mut feed = events.subscribe();
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { return };
                let Ok(message) = Message::from_line(&line) else { continue };
                match (message.method.as_deref(), message.id) {
                    (Some(rpc::SQUADS), Some(id)) => {
                        let squads = rpc::Squads {
                            // `FAKE_NO_SQUADS=1`: a first run, nothing made yet.
                            squads: if std::env::var("FAKE_NO_SQUADS").is_ok_and(|v| v == "1") {
                                Vec::new()
                            } else {
                                ["demo", "side"].map(|name| rpc::SquadEntry { name: name.into(), projects: vec!["/tmp/fake/site".into()], home: format!("/tmp/fake/{name}-home").into() }).to_vec()
                            },
                            last: Some("demo".into()),
                        };
                        if writer.write_all(Message::response(id, squads).to_line().as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    (Some(rpc::HELLO), Some(id)) => {
                        let params: rpc::HelloParams = message.params_as().unwrap();
                        let history = vec![
                            Entry::User { text: "Carry on with what you were doing yesterday".into() },
                            Entry::Reply { text: "Sure, I'll carry on from **T-9**.".into() },
                            Entry::Tool { tool: "Read".into(), target: Some("/tmp/fake/site/a.rs".into()), outcome: None, label: None, background: false },
                            Entry::Tool { tool: "Edit".into(), target: Some("/tmp/fake/site/a.rs".into()), outcome: None, label: None, background: false },
                            Entry::Reply { text: "Fixed.".into() },
                        ];
                        // `FAKE_SAID`: one more message of the person's, last — as one with
                        // attachment tags comes back after a restart.
                        let mut history = history;
                        // `FAKE_SCOPED=1`: a history over two projects and the whole
                        // squad, each turn marked with what it was about.
                        if std::env::var_os("FAKE_SCOPED").is_some() {
                            history = scoped_history();
                            routes.lock().await.thread.insert(1, Some("app".into()));
                        }
                        // `FAKE_HISTORY=<n>`: a long day's history, `n` lines (`fake_daemon/history.rs`).
                        if let Some(n) = std::env::var("FAKE_HISTORY").ok().and_then(|n| n.parse().ok()) {
                            history = history::history(n);
                        }
                        history.extend(std::env::var("FAKE_SAID").ok().filter(|s| !s.is_empty()).map(|text| Entry::User { text }));
                        // `FAKE_RETRO=1`: last week's retro and this week's, in the PM's conversation.
                        if retro::on() {
                            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                            history.extend(retro::history(now));
                        }
                        // `FAKE_ASKLIST=1`: the PM's last reply asks three things in a list of its own words.
                        if std::env::var("FAKE_ASKLIST").is_ok_and(|v| v == "1") {
                            history.push(Entry::User { text: "Can we ship to the EU stores right away too?".into() });
                            history.push(Entry::Tool { tool: "WebFetch".into(), target: Some("https://developer.apple.com/help/app-store-connect/manage-compliance-information/manage-european-union-digital-services-act-trader-requirements".into()), outcome: None, label: None, background: false });
                            history.push(Entry::Reply { text: ASKLIST.into() });
                        }
                        if reading::on() {
                            history.push(Entry::User { text: reading::ASKED.into() });
                            history.push(Entry::Reply { text: reading::REPLY.into() });
                        }
                        if pictures::on() {
                            history.push(Entry::Note { text: pictures::NEWS.into() });
                        }
                        if let Some(text) = phone_qa::note() {
                            history.push(Entry::Note { text });
                        }
                        if replies::on() {
                            history.extend(replies::history());
                        }
                        // `FAKE_SIGNIN`: the PM's last turn stopped by a lost sign-in, and its card.
                        if signin::on() {
                            history.extend(signin::history());
                            signin::attached(events.clone());
                        }
                        if browser::handover() {
                            history.push(Entry::User { text: "Change the EU DSA trader status in App Store Connect".into() });
                            history.push(Entry::Reply { text: "My browser isn't signed in to App Store Connect, so you'll need to sign in first.".into() });
                            history.push(Entry::Note { text: format!("🖥 pm's browser comes up on this Mac's screen: {}", browser::PM_WHY) });
                        }
                        // `FAKE_CARDS=1`: a last reply naming a review, a wait and a number
                        // long gone, for the tickets' cards on the pointer.
                        if std::env::var_os("FAKE_CARDS").is_some() {
                            history.push(Entry::User { text: "Anything for me to look at?".into() });
                            let pictures = if replies::on() { " Have a look at T-4's pictures too." } else { "" };
                            history.push(Entry::Reply { text: format!("T-42 is waiting for review. T-117 is your turn, and the old T-999 is gone.{pictures}") });
                        }
                        if let Some(how) = your_turn() {
                            history.push(Entry::Note { text: "frontend handed in T-63 for your review · Competitor onboarding research.".into() });
                            history.push(Entry::Note { text: "T-117 waits for you — frontend needs something only you can do · App Store submission.".into() });
                            if how == "elsewhere" {
                                let events = events.clone();
                                tokio::spawn(async move {
                                    let after = std::env::var("FAKE_TURN_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(5000);
                                    tokio::time::sleep(Duration::from_millis(after)).await;
                                    let changed = {
                                        let mut all = tickets().lock().unwrap();
                                        all.iter_mut().find(|t| t.id == 117 && t.status == TicketStatus::Waiting).map(|t| {
                                            t.status = TicketStatus::InProgress;
                                            use equipe_core::protocol::asks::{self, Answers, Reply};
                                            let answers = Answers { replies: vec![Reply::Text("+1 415 555 0134, review@equipe.im".into()), Reply::Later], ..Answers::default() };
                                            let message = asks::answer_message(&t.asks(), &answers);
                                            t.remaining = asks::still_open(&t.asks(), &message);
                                            t.note = Some(format!("Resumed: {message}"));
                                            t.asks.clear();
                                            t.clone()
                                        })
                                    };
                                    if let Some(ticket) = changed {
                                        let _ = events.send(Event::TicketChanged { squad: "demo".into(), ticket });
                                    }
                                });
                            }
                        }
                        if deciding().as_deref() == Some("elsewhere") {
                            let events = events.clone();
                            tokio::spawn(async move {
                                let after = std::env::var("FAKE_DECIDE_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(5000);
                                tokio::time::sleep(Duration::from_millis(after)).await;
                                close_questions(9001, Some(vec![Answer { picked: vec![1], ..Default::default() }, Answer { picked: vec![0, 2], other: Some("Settings: notifications only".into()), ..Default::default() }]), &events);
                            });
                        }
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                        let frontend = on(info(2, "frontend", vec![]), 47, "web", "Product detail page image gallery", working() >= 1, ("Edit", "src/gallery.tsx"), Some(Estimate { done_at: Some(now + 25 * 60), ..Default::default() }));
                        let backend = on(info(3, "backend", vec![]), 70, "api", "Orders API pagination", working() >= 2, ("Bash", "cargo test -p api"), Some(Estimate { done_at: Some(now + 50 * 60), guessed: true, ..Default::default() }));
                        let designer = on(info(4, "designer", vec![]), 60, "app", "Three onboarding screen drafts", false, ("Read", "brief.md"), Some(Estimate { done_at: Some(now.saturating_sub(12 * 60)), ..Default::default() }));
                        // `FAKE_DESIGN=working`: the designer mid-turn on T-378, shooting the map.
                        let designer = if design::mode().as_deref() == Some("working") {
                            let what = Estimate { done_at: Some(now + 18 * 60), what: Some("Shooting the checkout flow's states".into()), ..Default::default() };
                            on(info(4, "designer", vec![]), design::WORKING_TICKET, "app", "Make the UI map", true, ("Write", "design/flows/checkout/pay/default.light.390.png"), Some(what))
                        } else {
                            designer
                        };
                        // Settings have something to show: a member on its own mode and effort, rules kept.
                        let mut designer = designer;
                        designer.mode = Some(equipe_core::protocol::Mode::Ask);
                        if review::on() {
                            designer.work = None;
                            designer.recent = review::history();
                        }
                        // `FAKE_NOISE=1`: the engine's own turns in the designer's conversation.
                        if noise::on() {
                            designer.work = None;
                            designer.recent = noise::history();
                        }
                        // `FAKE_CHECKS=1`: the designer checks a screen in code and has fresh eyes look at it.
                        if checks::on() {
                            designer.work = None;
                            designer.state = SessionState::Ready;
                            designer.recent = checks::history();
                        }
                        let mut backend = backend;
                        backend.effort = Some("high".into());
                        if let Some(mode) = helpers::mode() {
                            helpers::dress(&mut backend, mode == "codex");
                            static HELPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                            if mode != "codex" && !HELPING.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                tokio::spawn(helpers::play(events.clone()));
                            }
                        }
                        let mut frontend = frontend;
                        frontend.recent.extend(disk::failed_build());
                        if subagents_on() {
                            for member in [&mut frontend, &mut backend] {
                                let (subs, calls) = subagents_of(member.session);
                                member.subagents = subs;
                                member.recent.extend(calls);
                            }
                            static PLAYING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                            if !PLAYING.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                tokio::spawn(subagents_play(events.clone()));
                            }
                        }
                        // `FAKE_BACKGROUND=1`: the PM at work, waiting on a command it left
                        // running in the background (described, as Claude Code's are).
                        let pm_waits = std::env::var("FAKE_BACKGROUND").is_ok_and(|v| v == "1");
                        if pm_waits {
                            history.push(Entry::User { text: "Check it as soon as the new deploy is up".into() });
                            history.push(Entry::Tool { tool: "Bash".into(), target: Some("git -C ~/site log -1 --format=%h".into()), outcome: Some(Box::new(equipe_core::protocol::ToolOutcome { ok: true, exit: None, ms: Some(100), output: "6ad5f11".into(), skipped: 0, diff: None })), label: Some("Check the current commit".into()), background: false });
                            history.push(Entry::Tool { tool: "Bash".into(), target: Some("until [ \"$(git -C ~/site log -1 --format=%h)\" != 6ad5f11 ]; do sleep 30; done".into()), outcome: None, label: Some("Watch for a new commit and deploy".into()), background: true });
                            history.push(Entry::Reply { text: "Sure, I'm watching for a new commit and deploy. When it's up I'll check the screen and show you.".into() });
                        }
                        let mut pm_info = info(1, "pm", history);
                        if let Some(mode) = std::env::var("FAKE_COMPACT").ok().filter(|m| !m.is_empty()) {
                            let ago = |secs: u64| now_ms().saturating_sub(secs * 1000);
                            let (trigger, secs) = match mode.as_str() {
                                "manual" => (CompactTrigger::Manual, 12),
                                "long" => (CompactTrigger::Auto, 75),
                                "done" | "fail" => (CompactTrigger::Auto, 5),
                                _ => (CompactTrigger::Auto, 12),
                            };
                            pm_info.compacting = Some(CompactingInfo { trigger, started_at: ago(secs), estimate_ms: 120_000 });
                            frontend.compacting = Some(CompactingInfo { trigger: CompactTrigger::Auto, started_at: ago(41), estimate_ms: 120_000 });
                            if mode == "queued" {
                                pm_info.steering = vec!["And check the web too".into(), "Tell me when it's done".into()];
                                pm_info.steering_in = vec![None, None];
                            }
                            if mode == "done" || mode == "fail" {
                                let events = events.clone();
                                let after = std::env::var("FAKE_COMPACT_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(4000);
                                let fail = mode == "fail";
                                tokio::spawn(async move {
                                    tokio::time::sleep(Duration::from_millis(after)).await;
                                    let _ = events.send(if fail {
                                        Event::CompactFailed { session: 1, member: Some("pm".into()), project: None, trigger: CompactTrigger::Auto, message: "timeout".into() }
                                    } else {
                                        Event::Compacted { session: 1, member: Some("pm".into()), project: None, trigger: CompactTrigger::Auto, took_ms: after }
                                    });
                                });
                            }
                        }
                        if pm_waits {
                            pm_info.state = SessionState::Working;
                        }
                        let mut sessions = vec![pm_info, frontend, backend, designer];
                        if std::env::var_os("FAKE_NO_DESIGNER").is_some() {
                            sessions.pop();
                        }
                        if busy() {
                            for (ticket, session, name, title) in BUSY {
                                let role = name.split('-').next().unwrap_or(name);
                                let mut member = on(info(session, role, vec![]), ticket, "web", title, true, ("Edit", "src/search.tsx"), None);
                                member.member = Some(Member { role: role.into(), name: name.into() });
                                sessions.push(member);
                            }
                        }
                        switching::mix(&mut sessions);
                        // Read apart: the board's lock is held while the snapshot is put together.
                        let waiting_words = {
                            let board = tickets().lock().unwrap().clone();
                            words::on_hello(&board)
                        };
                        for s in &mut sessions {
                            s.recent_at = said_at(&s.recent, unix());
                        }
                        let snapshot = Snapshot {
                            plans: switching::plans(),
                            plan: switching::claude().or_else(engines::plan),
                            engine_switching: switching::switching(),
                            pm_accepts: true,
                            role_models: switching::role_models(),
                            // `FAKE_SECRET=1`: backend waits for the NAS login, typed into a card.
                            secret_asks: if std::env::var_os("FAKE_SECRET").is_some() {
                                vec![equipe_core::protocol::secrets::SecretAsk {
                                    id: 41,
                                    session: 3,
                                    member: "backend".into(),
                                    ticket: Some(117),
                                    why: "The NAS sign-in for the daily automatic collection".into(),
                                    fields: vec![
                                        equipe_core::protocol::secrets::SecretField { key: "NAS_USER".into(), label: "NAS user name".into(), secret: false },
                                        equipe_core::protocol::secrets::SecretField { key: "NAS_PASSWORD".into(), label: "NAS password".into(), secret: true },
                                    ],
                                    save_to: "~/.config/atinum/nas.env".into(),
                                    at: 0,
                                }]
                            } else {
                                Vec::new()
                            },
                            engine_proposals: switching::proposals(),
                            squad: params.squad,
                            projects: if crowd::on() { crowd::projects() } else { vec![repo("web"), repo("app"), repo("api")] },
                            sessions,
                            approvals: asks().lock().unwrap().clone(),
                            questions: questions().lock().unwrap().clone(),
                            team_changes: team::changes(),
                            routine_proposals: routine::proposals(),
                            routines: if retro::on() { retro::routines() } else { routine::routines() },
                            idle_wake_hours: Some(routine::idle_wake_hours()),
                            signals: signals::signals(),
                            accepts: accept::cards(),
                            tickets: tickets().lock().unwrap().clone(),
                            allowed: allowed().lock().unwrap().clone(),
                            browsers_shown: true,
                            disk: disk::status(),
                            reviews: reviews(),
                            put_away: put_away().lock().unwrap().clone(),
                            words: Some(waiting_words),
                            words_all: words::all(),
                            ..Default::default()
                        };
                        team::on_hello(&events);
                        // `FAKE_VERSION=0.1.0`: an older daemon, for the update banner.
                        let hello = Hello { version: std::env::var("FAKE_VERSION").unwrap_or_else(|_| "fake".into()), snapshot };
                        writer.write_all(Message::response(id, hello).to_line().as_bytes()).await.unwrap();
                        // `FAKE_BROWSER`: the shared Chrome's tabs as they stand, as the daemon tells them.
                        for event in browser::replay() {
                            if writer.write_all(Message::event(&event).to_line().as_bytes()).await.is_err() {
                                return;
                            }
                        }
                    }
                    // Kept as the daemon keeps them: under `uploads`, named by equipe.
                    // The browser panel's own line: pictures of the tab it watches.
                    (Some(rpc::BROWSER_STREAM), Some(id)) if browser::enabled() => {
                        if writer.write_all(Message::response(id, true).to_line().as_bytes()).await.is_err() {
                            return;
                        }
                        return browser::serve_stream(lines.into_inner(), writer, events.clone()).await;
                    }
                    // The real consent file, in the fake home: `unset` until a screen answers.
                    (Some(rpc::TELEMETRY), Some(id)) => {
                        if writer.write_all(Message::response(id, equipe_core::telemetry::status(&equipe_home())).to_line().as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    (Some(rpc::TELEMETRY_SET), Some(id)) => {
                        let ask: equipe_core::protocol::telemetry::SetTelemetry = message.params_as().unwrap();
                        let reply = match equipe_core::telemetry::set(&equipe_home(), ask.on) {
                            Ok(status) => Message::response(id, status),
                            Err(why) => Message::failure(id, -32000, why),
                        };
                        if writer.write_all(reply.to_line().as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    (Some(rpc::UPLOAD), Some(id)) => {
                        use base64::Engine as _;
                        use std::io::Write as _;
                        let upload: rpc::Upload = message.params_as().unwrap();
                        let kept = base64::engine::general_purpose::STANDARD
                            .decode(upload.data.trim())
                            .map_err(|e| e.to_string())
                            .and_then(|bytes| {
                                let (mut file, path) = gateway::create_upload(&equipe_home(), &upload.name, upload.kind)?;
                                file.write_all(&bytes).map_err(|e| e.to_string())?;
                                Ok(path)
                            });
                        let reply = match kept {
                            Ok(path) => Message::response(id, rpc::Uploaded { path }),
                            Err(why) => Message::failure(id, -32000, why),
                        };
                        if writer.write_all(reply.to_line().as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    // Read back as the daemon does: from its uploads only.
                    (Some(rpc::READ_UPLOAD), Some(id)) => {
                        let ask: rpc::ReadUpload = message.params_as().unwrap();
                        let reply = match gateway::read_upload(&equipe_home(), &ask) {
                            Ok(read) => Message::response(id, read),
                            Err(why) => Message::failure(id, -32000, why),
                        };
                        if writer.write_all(reply.to_line().as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    // A member's picture: from the pictures' folder only — or a
                    // design system's, checked as the daemon checks it.
                    (Some(rpc::READ_IMAGE), Some(id)) => {
                        let ask: rpc::ReadImage = message.params_as().unwrap();
                        let reply = match phone_qa::read(&ask).or_else(|why| pictures::read(&ask).map_err(|_| why)).or_else(|why| if replies::on() { replies::read(&ask) } else { Err(why) }).or_else(|why| if design::mode().is_some() { design::read_image(&ask) } else { Err(why) }).or_else(|why| history::read(&ask).map_err(|_| why)) {
                            Ok(read) => Message::response(id, read),
                            Err(why) => Message::failure(id, -32000, why),
                        };
                        if writer.write_all(reply.to_line().as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    // A folder a reply names: the drafts' only.
                    (Some(rpc::LIST_IMAGES), Some(id)) => {
                        let ask: rpc::ListImages = message.params_as().unwrap();
                        let reply = match replies::list(&ask) {
                            Ok(list) => Message::response(id, list),
                            Err(why) => Message::failure(id, -32000, why),
                        };
                        if writer.write_all(reply.to_line().as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    // The look a screen shows, kept as the daemon keeps it (`equipe_core::looks`), in memory.
                    (Some(rpc::SCREEN_LOOK), None) => {
                        if let Some(look) = message.params_as::<equipe_core::protocol::look::ScreenLook>() {
                            eprintln!("screen look from {}: {} {:?} {}x{} sidebar {:?} zoom {} idle {:?}", look.machine, look.lang, look.theme, look.width, look.height, look.sidebar_width, look.zoom, look.idle);
                            looks().lock().unwrap().report(look, now_ms() / 1000);
                        }
                    }
                    // The look of the screen used last: what the real view is shot in.
                    (Some(rpc::PERSON_LOOK), Some(id)) => {
                        let look = looks().lock().unwrap().person().cloned();
                        eprintln!("person look asked: {:?}", look.as_ref().map(|s| &s.look.machine));
                        if writer.write_all(Message::response(id, equipe_core::protocol::look::PersonLook { look }).to_line().as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    (Some(rpc::COMMAND), None) => {
                        let command: Command = message.params_as().unwrap();
                        eprintln!("command: {command:?}");
                        // The person's own message: read for a decision in words first, as the daemon does.
                        let command = match command {
                            Command::Say { text, later, pasted, session, about, .. } => {
                                if !words::on_say(&text, pasted, &events) {
                                    continue;
                                }
                                if later { Command::SubmitLater { session, text, about } } else { Command::Submit { session, text, about } }
                            }
                            Command::UndoWords { id, .. } => {
                                words::on_undo(id, &events);
                                continue;
                            }
                            Command::WordsLog { squad } => {
                                let _ = events.send(Event::WordsLog { squad, entries: words::log() });
                                continue;
                            }
                            Command::ClearWordsLog { squad } => {
                                let _ = events.send(Event::WordsLog { squad, entries: Vec::new() });
                                continue;
                            }
                            Command::SetWordsAll { on, .. } => {
                                words::set_all(on, &events);
                                continue;
                            }
                            other => other,
                        };
                        match command {
                            Command::Submit { session, text, about } => {
                                let working = |id: SessionId| {
                                    let turns = turns.clone();
                                    async move { turns.lock().await.get(&id).is_some_and(|t| !t.is_finished()) }
                                };
                                // Route like the daemon: the conversation about the project in focus,
                                // beside the member's other work if it is at work in another.
                                let mut r = routes.lock().await;
                                // What the screen that sent it shows, else the focus.
                                let wanted = about.clone().unwrap_or_else(|| r.focus.clone());
                                let lane = if r.kept.contains(&session) { Some(session) } else { r.lanes.iter().find(|(_, of, project)| *of == session && *project == wanted).map(|l| l.0) };
                                let session = match lane {
                                    Some(lane) => lane,
                                    None => {
                                        let here = r.thread.get(&session).cloned().unwrap_or(None);
                                        if here != wanted && working(session).await {
                                            let id = 100 + r.lanes.len() as SessionId;
                                            r.lanes.push((id, session, wanted.clone()));
                                            let role = ["pm", "frontend", "backend", "designer"][(session as usize - 1).min(3)];
                                            let _ = events.send(Event::SessionStarted { session: id, project: "/tmp/fake/site".into(), engine: Engine::Claude, model: None, workspace: None, member: Some(Member { role: role.into(), name: role.into() }), squad: Some("demo".into()) });
                                            let _ = events.send(Event::LaneOpened { session: id, of: session, project: wanted.clone(), kept: false });
                                            id
                                        } else {
                                            if here != wanted {
                                                r.thread.insert(session, wanted.clone());
                                                let _ = events.send(Event::Thread { session, project: wanted.clone() });
                                            }
                                            session
                                        }
                                    }
                                };
                                drop(r);
                                let busy = working(session).await;
                                if busy {
                                    // Like the daemon: said to a member at work, it steers.
                                    waiting().lock().await.entry(session).or_default().push((text.clone(), wanted.clone()));
                                    let _ = events.send(Event::SteerQueued { session, text, project: wanted.clone() });
                                } else {
                                    // `torture <n>`: a part of the Markdown torture test (see `torture.rs`).
                                    let part = text.strip_prefix("torture").and_then(|n| n.trim().parse::<usize>().ok());
                                    let _ = events.send(Event::UserMessage { session, text });
                                    let task = match part {
                                        Some(part) => tokio::spawn(torture::turn(session, part, events.clone(), seconds)),
                                        None => tokio::spawn(turn(session, events.clone(), seconds)),
                                    };
                                    turns.lock().await.insert(session, task);
                                }
                            }
                            // Like the daemon: a conversation of the PM's of its own, kept.
                            Command::NewConversation { session, project, .. } => {
                                routes.lock().await.kept.insert(session);
                                let _ = events.send(Event::SessionStarted { session, project: "/tmp/fake/site".into(), engine: Engine::Claude, model: None, workspace: None, member: Some(Member { role: "pm".into(), name: "pm".into() }), squad: Some("demo".into()) });
                                let _ = events.send(Event::LaneOpened { session, of: 1, project, kept: true });
                            }
                            Command::MoveConversation { session, project } => {
                                let _ = events.send(Event::LaneMoved { session, project });
                            }
                            Command::CloseConversation { session } => {
                                routes.lock().await.kept.remove(&session);
                                let _ = events.send(Event::SessionClosed { session });
                            }
                            Command::ReopenConversation { squad, session } => {
                                let back = {
                                    let mut away = put_away().lock().unwrap();
                                    away.iter().position(|c| c.session == session).map(|at| (away.remove(at), away.clone()))
                                };
                                if let Some((c, left)) = back {
                                    routes.lock().await.kept.insert(session);
                                    let _ = events.send(Event::SessionStarted { session, project: "/tmp/fake/site".into(), engine: Engine::Claude, model: None, workspace: None, member: Some(Member { role: "pm".into(), name: "pm".into() }), squad: Some(squad.clone()) });
                                    let _ = events.send(Event::LaneOpened { session, of: 1, project: c.project.clone(), kept: true });
                                    let title = c.title.clone().unwrap_or_default();
                                    let lines = vec![Entry::User { text: title }, Entry::Reply { text: "This is where we left off last time. Shall I carry on?".into() }];
                                    let lines_at = said_at(&lines, unix() - 2 * 86_400);
                                    let _ = events.send(Event::ConversationBack { session, title: c.title, lines, lines_at });
                                    let _ = events.send(Event::PutAwayChanged { squad, list: left });
                                }
                            }
                            Command::SubmitLater { session, text, about } => {
                                let project = match about { Some(about) => about, None => routes.lock().await.focus.clone() };
                                waiting().lock().await.entry(session).or_default().push((text.clone(), project.clone()));
                                let _ = events.send(Event::MessageQueued { session, text, project });
                            }
                            Command::Unqueue { session, text } => {
                                if let Some(w) = waiting().lock().await.get_mut(&session)
                                    && let Some(at) = w.iter().position(|(t, _)| *t == text)
                                {
                                    w.remove(at);
                                }
                                let _ = events.send(Event::MemoWithdrawn { session, text });
                            }
                            // Like the daemon: the turn stops and what waits goes as one message.
                            Command::SendNow { session, texts } => {
                                let mut all = waiting().lock().await.remove(&session).unwrap_or_default();
                                let mut order: Vec<String> = Vec::new();
                                for text in texts {
                                    if let Some(at) = all.iter().position(|(t, _)| *t == text) {
                                        order.push(all.remove(at).0);
                                    }
                                }
                                order.extend(all.into_iter().map(|(t, _)| t));
                                if order.is_empty() {
                                    continue;
                                }
                                // Like the daemon: never the end of the work. At work, read at
                                // its next step — at once when a command it sat on is stopped.
                                if turns.lock().await.get(&session).is_some_and(|t| !t.is_finished()) {
                                    if command::mode().is_some()
                                        && let Some(task) = turns.lock().await.remove(&session)
                                    {
                                        task.abort();
                                        tokio::spawn(command::stopped_and_read(session, events.clone(), order));
                                    } else {
                                        tokio::spawn(command::read_at_next_step(session, events.clone(), order));
                                    }
                                    continue;
                                }
                                let _ = events.send(Event::QueueSent { session, texts: order.clone() });
                                let _ = events.send(Event::UserMessage { session, text: order.join("\n\n") });
                                turns.lock().await.insert(session, tokio::spawn(turn(session, events.clone(), seconds)));
                            }
                            Command::Refresh { session, summary: true } => {
                                let _ = events.send(Event::Compacting { session, member: None, project: None, trigger: CompactTrigger::Manual, started_at: now_ms(), estimate_ms: 120_000 });
                            }
                            Command::Interrupt { session } => {
                                if let Some(task) = turns.lock().await.remove(&session) {
                                    task.abort();
                                }
                                let _ = events.send(Event::TurnEnded { session, outcome: TurnOutcome::Interrupted, usage: None });
                            }
                            // Only that subagent: a helper hands back what it had.
                            Command::StopSubagent { session, id } => {
                                tokio::spawn(helpers::stopped(session, id, events.clone()));
                            }
                            // Only the command: its step fails, and the turn goes on to its end.
                            Command::StopCommand { session } => {
                                if let Some(task) = turns.lock().await.remove(&session) {
                                    task.abort();
                                    tokio::spawn(command::stopped(session, events.clone()));
                                }
                            }
                            Command::Decide { squad, approval, decision, words, .. } => {
                                eprintln!("decide {approval} {decision:?} {words:?}");
                                asks().lock().unwrap().retain(|a| a.id != approval);
                                let _ = events.send(Event::ApprovalResolved { squad, approval, decision });
                            }
                            Command::Focus { project, .. } => {
                                routes.lock().await.focus = project;
                            }
                            // Off the squad: the list without it, as the daemon sends it.
                            Command::RemoveProject { squad, path } => {
                                eprintln!("remove project {}", path.display());
                                let projects = [repo("web"), repo("app"), repo("api")].into_iter().filter(|p| *p != path).collect();
                                let _ = events.send(Event::ProjectsChanged { squad, projects, hq: None });
                            }
                            Command::AddProject { squad, path } => {
                                eprintln!("add project {path:?}");
                                let _ = events.send(Event::ProjectsChanged { squad, projects: vec![repo("web"), repo("app"), repo("api")], hq: None });
                            }
                            Command::SaveSecret { squad, id, values } => {
                                eprintln!("secret {id}: {} values {values:?}", values.len());
                                let _ = events.send(Event::SecretClosed { squad, id, saved: true });
                            }
                            // One other person's accounts, as a squad testing two of them keeps.
                            Command::BrowserProfiles => {
                                let profiles = vec![equipe_core::protocol::BrowserProfile { name: "jhjung".into(), label: "jhjung@atinuminvest.co.kr".into() }];
                                let _ = events.send(Event::BrowserProfiles { profiles });
                            }
                            Command::AnswerQuestions { id, answers, .. } => {
                                if let Some((session, _)) = close_questions(id, Some(answers), &events) {
                                    turns.lock().await.insert(session, tokio::spawn(turn(session, events.clone(), seconds)));
                                }
                            }
                            Command::DismissQuestions { id, .. } => {
                                if let Some((session, _)) = close_questions(id, None, &events) {
                                    turns.lock().await.insert(session, tokio::spawn(turn(session, events.clone(), seconds)));
                                }
                            }
                            Command::Disallow { squad, rule } => {
                                let rules = {
                                    let mut kept = allowed().lock().unwrap();
                                    kept.retain(|r| *r != rule);
                                    kept.clone()
                                };
                                let _ = events.send(Event::Allowed { squad, rules });
                            }
                            Command::SetMode { squad, member, mode } => {
                                let _ = events.send(Event::ModeChanged { squad, member, mode });
                            }
                            pair @ (Command::PairStart | Command::PairAnswer { .. } | Command::PairCancel | Command::Devices | Command::Unpair { .. }) => {
                                let events = events.clone();
                                tokio::spawn(async move { pairing(&pair, &events).await });
                            }
                            other => {
                                // The PM's browser given back: it carries on, told so, as the daemon does.
                                if let Command::BrowserControl { lane, take: false } = &other
                                    && browser::handover()
                                    && lane == browser::PM
                                {
                                    let _ = events.send(Event::Notice { session: 1, text: "🖐 Handed the browser back.".into() });
                                    turns.lock().await.insert(1, tokio::spawn(turn(1, events.clone(), seconds)));
                                }
                                // A card confirmed: what the board's buttons send, run as the board runs them.
                                if let Some(commands) = accept::on_command(&other, &events) {
                                    for command in commands {
                                        if let Some(said) = on_board_command(&command) {
                                            for event in said {
                                                let _ = events.send(event);
                                            }
                                        } else if let Some(ticket) = on_ticket_command(&command) {
                                            let _ = events.send(Event::TicketChanged { squad: "demo".into(), ticket });
                                        }
                                    }
                                    continue;
                                }
                                if signin::on_command(&other, &events) || browser::on_command(&other, &events) || team::on_command(&other, &events) || routine::on_command(&other, &events) || signals::on_command(&other, &events) || disk::on_command(&other, &events) || engines::on_command(&other, &events) || switching::on_command(&other, &events) || design::on_command(&other, &events) || briefs::on_command(&other, &events) {
                                    continue;
                                }
                                if let Some(said) = on_board_command(&other) {
                                    for event in said {
                                        let _ = events.send(event);
                                    }
                                } else if let Some(ticket) = on_ticket_command(&other) {
                                    let _ = events.send(Event::TicketChanged { squad: "demo".into(), ticket });
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            event = feed.recv() => {
                let Ok(event) = event else { continue };
                if writer.write_all(Message::event(&event).to_line().as_bytes()).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// The PM's history with `FAKE_SCOPED`: the app, news of the web, the web,
/// the whole squad, the app again.
fn scoped_history() -> Vec<Entry> {
    let about = |p: Option<&str>| Entry::Thread { project: p.map(str::to_owned) };
    let user = |t: &str| Entry::User { text: t.into() };
    let reply = |t: &str| Entry::Reply { text: t.into() };
    let read = |p: &str, f: &str| Entry::Tool { tool: "Read".into(), target: Some(format!("/tmp/fake/site/{p}/{f}")), outcome: None, label: None, background: false };
    vec![
        about(Some("app")),
        user("Polish the copy on the first onboarding screen"),
        read("app", "src/onboarding.tsx"),
        reply("Sure — to match the **T-60** draft, I'll change the first screen's title to \"Where your team works\" and cut the buttons down to one, \"Get started\"."),
        about(Some("web")),
        Entry::Note { text: "frontend handed in T-47 for your review · Product detail page image gallery.".into() },
        user("How far along is the product detail gallery?"),
        read("web", "src/gallery.tsx"),
        read("web", "src/gallery.test.tsx"),
        reply("**T-47** is waiting for review. Swipe and zoom are done; the thumbnail strip is next."),
        about(None),
        user("Sort out this week's priorities"),
        reply("Three things this week.\n\n1. web — the gallery review (**T-47**)\n2. api — orders pagination (**T-70**)\n3. app — the onboarding drafts (**T-60**)"),
        about(Some("app")),
        user("The second onboarding screen in the same tone too"),
        read("app", "src/onboarding.tsx"),
        reply("The second screen now has the same tone. I cut its description down to two lines."),
    ]
}

/// One turn: a couple of steps, then the reply a few characters at a time.
async fn turn(session: SessionId, events: broadcast::Sender<Event>, seconds: f32) {
    let tool = |tool: &str, target: &str| Event::ToolUse { session, tool: tool.into(), target: Some(target.into()), label: None, background: false };
    tokio::time::sleep(Duration::from_millis(200)).await;
    for step in [tool("Read", "/tmp/fake/site/a.rs"), tool("Read", "/tmp/fake/site/b.rs"), tool("Edit", "/tmp/fake/site/a.rs")] {
        let _ = events.send(step);
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    if let Some(mode) = command::mode() {
        return command::run(session, &events, &mode).await;
    }
    let long = std::env::var("FAKE_LONG_REPLY").is_ok_and(|v| v == "1").then(history::long_reply);
    let chars: Vec<char> = long.as_deref().unwrap_or(REPLY).chars().collect();
    let chunks = chars.chunks(4).count().max(1);
    let gap = Duration::from_secs_f32(seconds / chunks as f32);
    for chunk in chars.chunks(4) {
        let _ = events.send(Event::TextDelta { session, text: chunk.iter().collect() });
        tokio::time::sleep(gap).await;
    }
    let usage = Usage { input_tokens: Some(1200), output_tokens: Some(340), context_tokens: Some(420), context_window: Some(1000), ..Default::default() };
    let _ = events.send(Event::TurnEnded { session, outcome: TurnOutcome::Completed, usage: Some(usage) });
    let _ = events.send(Event::Suggestion { session, text: "Tidy up the other pages too".into() });
}
