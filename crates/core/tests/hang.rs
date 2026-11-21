//! The daemon keeps answering, whatever its work waits on: a git that takes
//! its time, a sign-in check that never ends. Each wait here once held up
//! the one loop every screen talks to (docs/INCIDENT-2026-10-16-daemon-hang.md).
//!
//! This file is a process of its own: its tests run git through a stand-in
//! ([`worktree::use_git`]) that writes down each call, and is slow for a
//! repository that asks it to be.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::{Duration, Instant};

use equipe_core::gateway::{self, Client};
use equipe_core::journal::Journal;
use equipe_core::protocol::{Access, Command, Event, SessionSpec, Ticket, TicketKind, TicketStatus};
use equipe_core::runtime::Config;
use equipe_core::{squads, worktree};
use equipe_providers::CredentialStore;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A fresh directory under /tmp: Unix socket paths are limited to ~100 bytes.
fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = PathBuf::from("/tmp").join(format!("eqh-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn write_script(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The stand-in git, for every test of this process: each call goes down in
/// `<repo>.log`, and `merge-tree` takes six seconds where `<repo>.slow` is.
fn fake_git() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let bin = scratch("git").join("git");
        // `git -C <dir> …`: the directory is the second argument.
        write_script(
            &bin,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$2.log\" 2>/dev/null\ncase \"$*\" in *merge-tree*) [ -f \"$2.slow\" ] && sleep 6;; esac\nexec git \"$@\"\n",
        );
        worktree::use_git(bin);
    });
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn repo() -> PathBuf {
    let dir = scratch("repo");
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

/// `equipe/dev-t<id>` off main — with a file of its own when `work`.
fn branch(repo: &Path, id: u32, work: bool) -> String {
    let name = format!("equipe/dev-t{id}");
    git(repo, &["branch", &name, "main"]);
    if work {
        let wt = scratch("wt").join("w");
        git(repo, &["worktree", "add", "-q", &wt.to_string_lossy(), &name]);
        std::fs::write(wt.join(format!("T{id}.md")), "work\n").unwrap();
        git(&wt, &["add", "-A"]);
        git(&wt, &["commit", "-qm", "work"]);
        git(repo, &["worktree", "remove", "--force", &wt.to_string_lossy()]);
    }
    name
}

/// Ticket `T-<n>` of `acme`, in `status`, on `branch`.
fn ticket(home: &Path, repo: &Path, status: TicketStatus, branch: &str) {
    let journal = Journal::for_squad(home, "acme").unwrap();
    let made = journal
        .create_ticket(&Ticket {
            id: 0,
            title: format!("On {branch}"),
            body: "-".into(),
            status,
            owner: Some("frontend".into()),
            assignee: None,
            project: Some(repo.to_owned()),
            depends_on: Vec::new(),
            branch: Some(branch.to_owned()),
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            design_qa: None, landing: false, plan: None, model: None,
            flow: None,
            kind: TicketKind::Code,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            label: None, remaining: Vec::new(), images: Vec::new(),
            updated_at: None,
            priority: Default::default(),
            rank: 0,
        })
        .unwrap();
    let mut t = journal.ticket(made).unwrap().unwrap();
    t.merged = (status == TicketStatus::Done).then_some(false);
    journal.save_ticket(&t).unwrap();
}

fn fake_claude(auth: &str) -> PathBuf {
    let bin = scratch("bin").join("claude");
    write_script(&bin, &format!("#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then {auth}; echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\nexit 1\n"));
    bin
}

fn config(home: &Path, claude: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        // No Codex CLI either: the person's own must never run in a test.
        codex_binary: "/nonexistent/codex".into(),
        persist: true,
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

async fn daemon(home: &Path, claude: &Path) -> PathBuf {
    let socket = gateway::socket_path(home);
    tokio::spawn({
        let (home, claude, socket) = (home.to_owned(), claude.to_owned(), socket.clone());
        async move { gateway::serve(config(&home, &claude), &socket, "test").await }
    });
    for _ in 0..200 {
        if gateway::is_running(&socket).await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    socket
}

/// The daemon answers `busy` within `within`, and what it said.
async fn busy_within(socket: &Path, within: Duration) -> equipe_core::protocol::rpc::Busy {
    let started = Instant::now();
    let busy = tokio::time::timeout(within, gateway::busy(socket)).await.expect("busy answers in time").unwrap().unwrap();
    assert!(started.elapsed() <= within);
    busy
}

async fn until(client: &mut Client, within: Duration, done: impl Fn(&Event) -> bool) {
    let wait = async {
        while let Some(event) = client.next_event().await {
            if done(&event) {
                return;
            }
        }
        panic!("the daemon went away");
    };
    tokio::time::timeout(within, wait).await.expect("the event came in time");
}

fn log_lines(repo: &Path, with: &str) -> usize {
    let log = std::fs::read_to_string(repo.with_extension("log")).unwrap_or_default();
    log.lines().filter(|l| l.contains(with)).count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_merge_whose_git_takes_its_time_holds_no_screen_up() {
    fake_git();
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let work = branch(&repo, 1, true);
    ticket(&home, &repo, TicketStatus::Done, &work);
    std::fs::write(repo.with_extension("slow"), "").unwrap();
    let socket = daemon(&home, &fake_claude("true")).await;
    let (mut screen, _) = tokio::time::timeout(Duration::from_secs(5), Client::connect(&socket, "test", "acme")).await.unwrap().unwrap();

    // The person merges; git takes six seconds to work it out.
    screen.send(Command::Merge { squad: "acme".into(), ticket: 1, push: false });
    let deadline = Instant::now() + Duration::from_secs(20);
    while log_lines(&repo, "merge-tree") == 0 {
        assert!(Instant::now() < deadline, "the merge reached git");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Meanwhile: `equipe daemon status`, and a screen saying hello, answered at once.
    let busy = busy_within(&socket, Duration::from_secs(2)).await;
    assert_eq!(busy.stalled, None, "{busy:?}");
    let hello = tokio::time::timeout(Duration::from_secs(2), Client::connect(&socket, "test", "acme")).await;
    assert!(hello.is_ok_and(|h| h.is_ok()), "a hello waits for no merge");
    // And the merge lands when git is done.
    until(&mut screen, Duration::from_secs(30), |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.merged == Some(true))).await;
    assert!(repo.join("T1.md").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sweep_works_a_merge_out_again_only_for_a_branch_or_main_that_moved() {
    fake_git();
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let work = branch(&repo, 1, true);
    ticket(&home, &repo, TicketStatus::Done, &work);
    // T-2: in review, its branch nothing main lacks.
    let empty = branch(&repo, 2, false);
    ticket(&home, &repo, TicketStatus::Review, &empty);
    let socket = daemon(&home, &fake_claude("true")).await;
    let (mut screen, _) = Client::connect(&socket, "test", "acme").await.unwrap();

    // The daemon's first sweep works T-1's merge out, off its loop.
    let deadline = Instant::now() + Duration::from_secs(20);
    while log_lines(&repo, &format!("merge-tree --write-tree --no-messages refs/heads/main {work}")) == 0 {
        assert!(Instant::now() < deadline, "the first sweep looked at T-1");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Accepting T-2 sweeps again at once: T-2 is settled as merged…
    screen.send(Command::Accept { squad: "acme".into(), ticket: 2 });
    until(&mut screen, Duration::from_secs(20), |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 2 && ticket.merged == Some(true))).await;
    // …and T-1, whose branch and main stayed where they were, cost no merge again.
    assert_eq!(log_lines(&repo, &format!("refs/heads/main {work}")), 1, "{}", std::fs::read_to_string(repo.with_extension("log")).unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_step_of_the_core_that_never_ends_is_what_status_says() {
    fake_git();
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    // `claude auth status` that answers after eight seconds.
    let socket = daemon(&home, &fake_claude("sleep 8")).await;
    let (screen, _) = Client::connect(&socket, "test", "acme").await.unwrap();
    let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: repo.clone(), role: Some("pm".into()), squad: Some("acme".into()) };
    screen.send(Command::Start { session: 1, spec });
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Status answers within its wait, and says what the core is on, since when.
    let busy = busy_within(&socket, equipe_core::runtime::ANSWER_WAIT + Duration::from_secs(2)).await;
    let stall = busy.stalled.expect("the core is stuck");
    assert_eq!(stall.doing, "command start");
    assert!(stall.since > 0 && stall.secs < 10, "{stall:?}");
}

/// Messages' `chat.db` is a file macOS guards: opening it makes macOS ask the
/// person ("equipe wants to access data from other apps") and waits for the
/// answer. Here it is a named pipe, whose `open()` waits until someone writes.
#[cfg(target_os = "macos")]
fn messages_pipe(home: &Path) -> PathBuf {
    let messages = home.join("person/Library/Messages");
    std::fs::create_dir_all(&messages).unwrap();
    let chat_db = messages.join("chat.db");
    assert!(Process::new("mkfifo").arg(&chat_db).status().unwrap().success());
    chat_db
}

/// Whether anyone has the pipe open for reading: a write that doesn't wait fails with ENXIO when nobody has.
#[cfg(target_os = "macos")]
fn pipe_is_read(chat_db: &Path) -> bool {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new().write(true).custom_flags(0x0004).open(chat_db).is_ok()
}

/// The next `Event::Autopilot` the screen hears, within `within`.
#[cfg(target_os = "macos")]
async fn next_card(screen: &mut Client, within: Duration) -> equipe_core::protocol::vault::AutopilotStatus {
    let wait = async {
        while let Some(event) = screen.next_event().await {
            if let Event::Autopilot { status } = event {
                return status;
            }
        }
        panic!("the daemon went away");
    };
    tokio::time::timeout(within, wait).await.expect("a card came in time")
}

/// Nobody who left Messages off is ever shown macOS's question: the card
/// comes, and `chat.db` isn't touched.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_is_left_alone_until_the_person_turns_it_on() {
    fake_git();
    let home = scratch("home");
    let chat_db = messages_pipe(&home);
    let socket = daemon(&home, &fake_claude("true")).await;
    let (mut screen, _) = tokio::time::timeout(Duration::from_secs(5), Client::connect(&socket, "test", "acme")).await.unwrap().unwrap();

    for _ in 0..2 {
        screen.send(Command::Autopilot);
        let card = next_card(&mut screen, Duration::from_secs(3)).await;
        assert!(!card.sms && !card.sms_checking && card.sms_access == Some(true), "{card:?}");
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!pipe_is_read(&chat_db), "chat.db was opened for a person who never turned Messages on");
}

/// Turning Messages on is what asks macOS: off the loop, the card first saying
/// it is waiting, and the core keeps answering until the person has allowed it.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_of_messages_that_waits_for_consent_holds_no_screen_up() {
    fake_git();
    let home = scratch("home");
    let chat_db = messages_pipe(&home);
    let socket = daemon(&home, &fake_claude("true")).await;
    let (mut screen, _) = tokio::time::timeout(Duration::from_secs(5), Client::connect(&socket, "test", "acme")).await.unwrap().unwrap();

    screen.send(Command::SetSmsCodes { on: true });
    // The card says it is waiting for macOS…
    loop {
        let card = next_card(&mut screen, Duration::from_secs(5)).await;
        if card.sms && card.sms_checking {
            break;
        }
    }
    // …and the core keeps answering while the open still waits.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let busy = busy_within(&socket, Duration::from_secs(2)).await;
    assert_eq!(busy.stalled, None, "{busy:?}");
    let hello = tokio::time::timeout(Duration::from_secs(2), Client::connect(&socket, "test", "acme")).await;
    assert!(hello.is_ok_and(|h| h.is_ok()), "a hello waits for no consent prompt");

    // The person says Allow (a writer opens the pipe): the card says it's fine.
    let _writer = tokio::task::spawn_blocking(move || std::fs::OpenOptions::new().write(true).open(&chat_db)).await.unwrap().unwrap();
    until(&mut screen, Duration::from_secs(10), |e| matches!(e, Event::Autopilot { status } if status.sms && !status.sms_checking && status.sms_access == Some(true))).await;
}

/// A "no" from macOS is said, once, and not asked again on its own.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_messages_is_said_and_not_asked_again() {
    use std::os::unix::fs::PermissionsExt as _;
    fake_git();
    let home = scratch("home");
    let messages = home.join("person/Library/Messages");
    std::fs::create_dir_all(&messages).unwrap();
    let chat_db = messages.join("chat.db");
    std::fs::write(&chat_db, "").unwrap();
    std::fs::set_permissions(&chat_db, std::fs::Permissions::from_mode(0o000)).unwrap();
    let socket = daemon(&home, &fake_claude("true")).await;
    let (mut screen, _) = tokio::time::timeout(Duration::from_secs(5), Client::connect(&socket, "test", "acme")).await.unwrap().unwrap();

    screen.send(Command::SetSmsCodes { on: true });
    until(&mut screen, Duration::from_secs(10), |e| matches!(e, Event::Autopilot { status } if status.sms && !status.sms_checking && status.sms_access == Some(false))).await;
    // Allowed behind its back, the card still says no until the person turns it off and on.
    std::fs::set_permissions(&chat_db, std::fs::Permissions::from_mode(0o644)).unwrap();
    screen.send(Command::Autopilot);
    let card = next_card(&mut screen, Duration::from_secs(3)).await;
    assert!(card.sms && card.sms_access == Some(false) && !card.sms_checking, "{card:?}");
    screen.send(Command::SetSmsCodes { on: false });
    screen.send(Command::SetSmsCodes { on: true });
    until(&mut screen, Duration::from_secs(10), |e| matches!(e, Event::Autopilot { status } if status.sms && !status.sms_checking && status.sms_access == Some(true))).await;
}
