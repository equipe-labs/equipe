//! Keys members obtain, kept and used (`equipe_core::squad_secrets`), driven
//! as a member and the person drive them: a member saves a key and never gets
//! it back as text, lists them, writes one into a git-ignored file (and is
//! refused for a tracked one), uses one as an environment variable of its next
//! turn — on a project the person hasn't allowed it asks once — and the value
//! is cut out of tickets, events and hand-ins. The secret store is a fake in
//! memory (`EQUIPE_VAULT_IN_MEMORY`): nothing here touches the Keychain.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::rpc::ToolCall;
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A value no other test in this binary (or any other) has.
const VALUE: &str = "phc_Zq7vK2mX9wLpTr4NsB8dY3hJ6cF1aGe5";

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-squad-secrets-it-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A `claude` that says it is signed in and, for a turn, writes its environment where the test reads it.
fn recording_claude(env_file: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = scratch("bin").join("claude");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\nenv > '{}'\nexec sleep 30\n",
        env_file.display()
    );
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn config(claude: &Path, home: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        codex_binary: "/nonexistent/codex".into(),
        persist: true,
        retention_days: 90,
        equipe_binary: Some("/opt/equipe".into()),
        npx: Some("/nonexistent/npx".into()),
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

async fn until(rx: &mut UnboundedReceiver<Event>, done: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    let wait = async {
        while let Some(event) = rx.recv().await {
            let stop = done(&event);
            seen.push(event);
            if stop {
                return;
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(15), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).env_remove("GIT_DIR").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A git repository with `.env.local` ignored and `tracked.env` tracked.
fn repo(name: &str) -> PathBuf {
    let dir = scratch(name).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    std::fs::write(dir.join(".gitignore"), ".env.local\n").unwrap();
    std::fs::write(dir.join("tracked.env"), "A=1\n").unwrap();
    git(&dir, &["add", "tracked.env", ".gitignore"]);
    dir.canonicalize().unwrap()
}

struct World {
    handle: Handle,
    events: UnboundedReceiver<Event>,
    home: PathBuf,
    /// Where members work at their desk: the squad's home, a git repository.
    desk: PathBuf,
    env_file: PathBuf,
}

async fn world() -> World {
    // SAFETY: set once, before the runtime starts, and only by this test binary.
    unsafe { std::env::set_var("EQUIPE_VAULT_IN_MEMORY", "1") };
    let project = repo("shop");
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let env_file = scratch("env").join("turn.env");
    let (handle, mut events) = runtime::start(config(&recording_claude(&env_file), &home));
    for (id, role) in [(1, "pm"), (2, "backend")] {
        let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some(role.into()), squad: Some("acme".into()) };
        handle.send(Command::Start { session: id, spec });
        until(&mut events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
    }
    // A member's desk is the squad's home: a git repository, here with one ignored and one tracked file.
    let desk = equipe_core::squad_home::path(&home, "acme");
    if !desk.join(".git").exists() {
        git(&desk, &["init", "-q"]);
    }
    let ignore = std::fs::read_to_string(desk.join(".gitignore")).unwrap_or_default();
    std::fs::write(desk.join(".gitignore"), format!("{ignore}\n.env.local\n")).unwrap();
    std::fs::write(desk.join("tracked.env"), "A=1\n").unwrap();
    git(&desk, &["add", "-f", "tracked.env"]);
    World { handle, events, home, desk: desk.canonicalize().unwrap(), env_file }
}

impl World {
    async fn call(&self, member: &str, name: &str, arguments: serde_json::Value) -> equipe_core::protocol::rpc::ToolResult {
        self.handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
    }

    async fn save(&self, name: &str, projects: serde_json::Value) -> equipe_core::protocol::rpc::ToolResult {
        self.call("backend", "secret_save", json!({ "name": name, "service": "posthog", "kind": "api_key", "value": VALUE, "projects": projects, "note": "project key" })).await
    }
}

#[tokio::test]
async fn a_saved_key_is_listed_by_facts_and_never_echoed_or_shown_to_a_screen() {
    let mut w = world().await;
    let saved = w.save("posthog-project-key", json!([])).await;
    assert!(!saved.is_error && saved.text.contains("Saved posthog-project-key"), "{saved:?}");
    assert!(!saved.text.contains(VALUE), "the tool never echoes the value: {saved:?}");

    // Facts for every screen; the value in none of them.
    let seen = until(&mut w.events, |e| matches!(e, Event::SquadSecrets { secrets } if !secrets.is_empty())).await;
    let Some(Event::SquadSecrets { secrets }) = seen.last() else { panic!() };
    assert_eq!((secrets[0].name.as_str(), secrets[0].created_by.as_str(), secrets[0].service.as_str()), ("posthog-project-key", "backend", "posthog"));
    assert!(secrets[0].last_used.is_none() && !secrets[0].any_project);
    for e in &seen {
        assert!(!format!("{e:?}").contains(VALUE), "{e:?}");
        let wire = serde_json::to_string(e).unwrap();
        assert!(!wire.contains(VALUE), "no event carries the value on the wire: {wire}");
    }

    // Listing: names and facts only.
    let listed = w.call("backend", "secret_list", json!({})).await;
    assert!(!listed.is_error && listed.text.contains("posthog-project-key") && listed.text.contains("posthog") && !listed.text.contains(VALUE), "{listed:?}");

    // The journal holds facts, not the value.
    let db = std::fs::read(equipe_core::journal::squad_dir(&w.home, "acme").join("squad.db")).unwrap();
    assert!(!String::from_utf8_lossy(&db).contains(VALUE), "the squad's journal keeps no value");

    // A bad call is refused with the way out.
    let bad = w.call("backend", "secret_save", json!({ "name": "x y", "service": "s", "kind": "token", "value": "v" })).await;
    assert!(bad.is_error && bad.text.contains("a name is letters"), "{bad:?}");
    let nope = w.call("backend", "secret_use", json!({ "name": "nope", "as_env": "X_KEY" })).await;
    assert!(nope.is_error && nope.text.contains("secret_list"), "{nope:?}");
    w.handle.shutdown().await;
}

#[tokio::test]
async fn the_value_is_cut_out_of_tickets_events_and_what_a_member_says() {
    let mut w = world().await;
    w.save("posthog-project-key", json!([])).await;

    // A ticket written with the value in it: the board has it masked.
    let made = w.call("backend", "create_ticket", json!({ "title": format!("Use {VALUE}"), "body": format!("key is {VALUE}, thanks") })).await;
    assert!(!made.is_error, "{made:?}");
    let tickets = w.handle.snapshot("acme").await.tickets;
    assert!(tickets.iter().all(|t| !t.title.contains(VALUE) && !t.body.contains(VALUE)), "{tickets:?}");
    assert!(tickets.iter().any(|t| t.body.contains("[secret:posthog-project-key]")), "{tickets:?}");

    // A comment and a hand-in go the same way through the member's tool arguments: the record has it masked.
    let id = tickets.iter().map(|t| t.id).max().unwrap();
    let noted = w.call("backend", "comment", json!({ "id": id, "text": format!("used {VALUE} on the dashboard") })).await;
    assert!(!noted.is_error, "{noted:?}");
    let history = equipe_core::journal::Journal::for_squad(&w.home, "acme").unwrap().ticket_history(id).unwrap();
    assert!(history.iter().all(|(_, _, text)| !text.contains(VALUE)), "{history:?}");
    assert!(history.iter().any(|(_, _, text)| text.contains("used [secret:posthog-project-key] on the dashboard")), "{history:?}");

    // What the person or a screen hears of a message with it in: masked.
    w.handle.send(Command::Submit { session: 2, text: format!("remember {VALUE} for later"), about: None });
    let seen = until(&mut w.events, |e| matches!(e, Event::UserMessage { .. })).await;
    let Some(Event::UserMessage { text, .. }) = seen.last() else { panic!() };
    assert_eq!(text, "remember [secret:posthog-project-key] for later");

    // A key nobody kept, in what a member writes: cut out, and the member is told to keep it.
    let unsaved = "phc_Hh3kL9pQ2wXz7vB5nM1cT8yR4jD6sFg";
    let told = w.call("backend", "create_ticket", json!({ "title": "x", "body": format!("the new key is {unsaved}") })).await;
    assert!(!told.is_error && told.text.contains("secret_save"), "{told:?}");
    let tickets = w.handle.snapshot("acme").await.tickets;
    assert!(tickets.iter().all(|t| !t.body.contains(unsaved)), "{tickets:?}");
    assert!(tickets.iter().any(|t| t.body.contains("[key redacted]")));
    w.handle.shutdown().await;
}

#[tokio::test]
async fn a_key_is_written_only_into_a_git_ignored_untracked_file_for_the_person_alone() {
    use std::os::unix::fs::PermissionsExt as _;
    let mut w = world().await;
    // The first use on this project: the person is asked once, and says yes for it.
    w.save("posthog-project-key", json!([])).await;
    let asked = w.call("backend", "secret_write", json!({ "name": "posthog-project-key", "path": ".env.local", "key": "POSTHOG_KEY" })).await;
    assert!(!asked.is_error && asked.text.contains("asked on a card"), "{asked:?}");
    assert!(!w.desk.join(".env.local").exists(), "nothing is written before the person says yes");
    let seen = until(&mut w.events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
    let Some(Event::QuestionsAsked { questions, .. }) = seen.last().cloned() else { panic!() };
    assert_eq!(questions.questions[0].options.len(), 3);
    assert!(questions.questions[0].question.contains("posthog-project-key"));
    // Asking again before they answer doesn't put a second card.
    let again = w.call("backend", "secret_write", json!({ "name": "posthog-project-key", "path": ".env.local" })).await;
    assert!(again.text.contains("already asked"), "{again:?}");
    // Yes, on this project.
    w.handle.send(Command::AnswerQuestions { squad: "acme".into(), id: questions.id, answers: vec![equipe_core::protocol::decide::Answer { picked: vec![0], ..Default::default() }], about: None });
    let seen = until(&mut w.events, |e| matches!(e, Event::SquadSecrets { secrets } if secrets.iter().any(|s| s.projects == [String::new()]))).await;
    assert!(!seen.is_empty());
    until(&mut w.events, |e| matches!(e, Event::QuestionsClosed { answers: Some(_), .. })).await;

    let wrote = w.call("backend", "secret_write", json!({ "name": "posthog-project-key", "path": ".env.local", "key": "POSTHOG_KEY" })).await;
    assert!(!wrote.is_error && wrote.text.contains("Wrote posthog-project-key") && !wrote.text.contains(VALUE), "{wrote:?}");
    let file = w.desk.join(".env.local");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("POSTHOG_KEY=\"{VALUE}\"\n"));
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);

    // A tracked file, a file git doesn't ignore, a way out of the worktree: refused.
    for (path, why) in [("tracked.env", "git tracks"), ("notes.txt", "isn't git-ignored"), ("../escape.env", "without `..`"), ("/tmp/abs.env", "relative")] {
        let refused = w.call("backend", "secret_write", json!({ "name": "posthog-project-key", "path": path })).await;
        assert!(refused.is_error && refused.text.contains(why), "{path}: {refused:?}");
    }
    assert_eq!(std::fs::read_to_string(w.desk.join("tracked.env")).unwrap(), "A=1\n", "the tracked file is untouched");

    // Every use is recorded: who, which project, when.
    let seen = until(&mut w.events, |e| matches!(e, Event::SquadSecrets { secrets } if secrets.iter().any(|s| s.last_used.is_some()))).await;
    let Some(Event::SquadSecrets { secrets }) = seen.last() else { panic!() };
    let used = secrets[0].last_used.clone().unwrap();
    assert_eq!((used.member.as_str(), used.how.as_str()), ("backend", "file"));
    w.handle.shutdown().await;
}

#[tokio::test]
async fn any_project_never_asks_and_another_squads_member_cannot_use_a_key_at_all() {
    let mut w = world().await;
    w.save("posthog-project-key", json!([])).await;
    // The person marks it for any project: no card.
    w.handle.send(Command::SetSquadSecretProjects { squad: "acme".into(), name: "posthog-project-key".into(), projects: Vec::new(), any_project: true });
    until(&mut w.events, |e| matches!(e, Event::SquadSecrets { secrets } if secrets.iter().any(|s| s.any_project))).await;
    let wrote = w.call("backend", "secret_write", json!({ "name": "posthog-project-key", "path": ".env.local" })).await;
    assert!(!wrote.is_error && wrote.text.contains("Wrote"), "{wrote:?}");
    assert_eq!(std::fs::read_to_string(w.desk.join(".env.local")).unwrap(), format!("POSTHOG_PROJECT_KEY=\"{VALUE}\"\n"), "the secret's name in capitals by default");

    // Another squad: it keeps nothing of acme's.
    let other_project = repo("other");
    squads::create(&w.home, "beta", &other_project).unwrap();
    let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: other_project.clone(), role: Some("backend".into()), squad: Some("beta".into()) };
    w.handle.send(Command::Start { session: 9, spec });
    until(&mut w.events, |e| matches!(e, Event::SessionStarted { session: 9, .. })).await;
    let theirs = w.handle.tool(ToolCall { run: None, squad: "beta".into(), member: "backend".into(), name: "secret_use".into(), arguments: json!({ "name": "posthog-project-key", "as_env": "POSTHOG_KEY" }) }).await;
    assert!(theirs.is_error && theirs.text.contains("No key called"), "{theirs:?}");
    let list = w.handle.tool(ToolCall { run: None, squad: "beta".into(), member: "backend".into(), name: "secret_list".into(), arguments: json!({}) }).await;
    assert!(list.text.contains("No keys are kept"), "{list:?}");

    // Deleting one (a key of its own: the masking list is shared by the tests of this binary): gone from the list and from the secret store.
    w.save("temp-key", json!([])).await;
    w.handle.send(Command::DeleteSquadSecret { squad: "acme".into(), name: "temp-key".into() });
    until(&mut w.events, |e| matches!(e, Event::SquadSecrets { secrets } if secrets.iter().all(|s| s.name != "temp-key"))).await;
    let gone = w.call("backend", "secret_write", json!({ "name": "temp-key", "path": ".env.local" })).await;
    assert!(gone.is_error && gone.text.contains("No key called"), "{gone:?}");
    w.handle.shutdown().await;
}

#[tokio::test]
async fn a_key_used_as_an_environment_variable_reaches_the_next_turn_and_only_there() {
    let mut w = world().await;
    w.save("posthog-project-key", json!([])).await;
    w.handle.send(Command::SetSquadSecretProjects { squad: "acme".into(), name: "posthog-project-key".into(), projects: Vec::new(), any_project: true });
    until(&mut w.events, |e| matches!(e, Event::SquadSecrets { secrets } if secrets.iter().any(|s| s.any_project))).await;

    let bad = w.call("backend", "secret_use", json!({ "name": "posthog-project-key", "as_env": "lower" })).await;
    assert!(bad.is_error, "{bad:?}");
    let used = w.call("backend", "secret_use", json!({ "name": "posthog-project-key", "as_env": "POSTHOG_KEY" })).await;
    assert!(!used.is_error && used.text.contains("$POSTHOG_KEY") && !used.text.contains(VALUE), "{used:?}");

    // The member's next turn: its commands' environment has it.
    w.handle.send(Command::Submit { session: 2, text: "go".into(), about: None });
    let waited = async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&w.env_file)
                && text.contains("POSTHOG_KEY=")
            {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let env = tokio::time::timeout(Duration::from_secs(15), waited).await.expect("the turn started");
    assert!(env.contains(&format!("POSTHOG_KEY={VALUE}")), "the turn's environment has the key");

    // …and none of what the screens saw of it has the value.
    while let Ok(event) = w.events.try_recv() {
        assert!(!format!("{event:?}").contains(VALUE), "{event:?}");
    }
    w.handle.shutdown().await;
}

#[tokio::test]
async fn the_person_sees_a_value_once_and_a_phone_never_does() {
    let mut w = world().await;
    w.save("posthog-project-key", json!([])).await;
    w.handle.send(Command::RevealSquadSecret { squad: "acme".into(), name: "posthog-project-key".into(), ask: 7 });
    let seen = until(&mut w.events, |e| matches!(e, Event::SquadSecretRevealed { .. })).await;
    let Some(Event::SquadSecretRevealed { name, ask, value, .. }) = seen.last() else { panic!() };
    assert_eq!((name.as_str(), *ask, value.0.as_str()), ("posthog-project-key", 7, VALUE));
    // From a phone: nothing comes back.
    w.handle.send(Command::OnPhone { command: Box::new(Command::RevealSquadSecret { squad: "acme".into(), name: "posthog-project-key".into(), ask: 8 }) });
    tokio::time::sleep(Duration::from_millis(300)).await;
    while let Ok(event) = w.events.try_recv() {
        assert!(!matches!(event, Event::SquadSecretRevealed { ask: 8, .. }), "{event:?}");
    }
    w.handle.shutdown().await;
}
