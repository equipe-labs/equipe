//! A stand-in daemon for looking at the screens without spending a real
//! engine's turns: it answers `hello` with a squad (a PM, frontend, backend and
//! designer, three projects — real git repositories it makes under
//! `$TMPDIR/equipe-fake`, with a branch for each piece of work handed in) and a
//! board with work in every state, and answers every message with a streamed
//! Korean reply — Markdown with a list, a table, a code block and `T-12` —
//! between tool steps. `interrupt` ends the turn at once; `focus` moves the PM's
//! thread; accept, merge, reopen, resume and cancel move tickets as the daemon
//! would. `FAKE_EMPTY=1` starts with nothing waiting for the person.
//!
//! ```text
//! export EQUIPE_HOME=$(mktemp -d) EQUIPE_SQUAD=demo
//! cargo run -p equipe-desktop --example fake_daemon &
//! cargo run -p equipe-desktop
//! ```
//! `FAKE_SECONDS` (default 4) is how long a reply streams; `FAKE_SAID` adds a
//! last message of the person's to the PM's history (attachment tags and all).
//!
//! `FAKE_TURN=1` puts T-117 down for the person — three asks, one naming a
//! folder with spaces and a file it made under the app's repository — with
//! its news last in the PM's conversation; `FAKE_TURN=elsewhere` also answers
//! it from "another screen" after `FAKE_TURN_AFTER` ms (default 5000).
//!
//! Pairing a phone: a code at once, and two phones paired already.
//! `FAKE_PAIR=ask` has a phone ask 1.5 s after the code shows (`allow` pairs
//! it); `relay` fails as an unreachable relay would; `silent` never answers,
//! as a daemon older than the app.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use equipe_core::gateway;
use equipe_core::protocol::rpc::{self, Hello, Message};
use equipe_core::protocol::{Access, Approval, Command, Engine, Entry, Estimate, Event, Member, SessionId, SessionInfo, SessionState, Snapshot, Ticket, TicketStatus, TurnOutcome, Usage, WorkInfo};
use equipe_providers::credentials::equipe_home;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, broadcast};

const REPLY: &str = "네, 확인했습니다. 지금 보드를 살펴보고 **T-12**를 먼저 처리하겠습니다.\n\n진행 순서는 다음과 같습니다.\n\n- 헤더 컴포넌트 정리\n- 입력창 한글 조합 확인\n- 스트리밍 표시 점검 (`T-13`)\n\n| 항목 | 상태 | 담당 |\n|---|---|---|\n| 헤더 | 진행 중 | frontend |\n| 입력창 | 대기 | frontend |\n| 데몬 연결 | 완료 | backend |\n\n```rust\nfn main() {\n    println!(\"안녕, equipe\");\n}\n```\n\n끝나면 다시 알려 드리겠습니다.";

fn info(session: SessionId, role: &str, recent: Vec<Entry>) -> SessionInfo {
    SessionInfo {
        session,
        project: "/tmp/fake/site".into(),
        member: Some(Member { role: role.into(), name: role.into() }),
        engine: Engine::Claude,
        model: Some("claude-opus-5-5".into()),
        access: Access::EditFiles,
        workspace: None,
        state: SessionState::Ready,
        title: None,
        tokens: 0,
        context: Some((310, 1000)),
        queued_project: None,
        queued_in: vec![],
        steering: vec![],
        lane: None,
        estimate: None,
        recent,
        queued: vec![],
        work: None,
        mode: None,
        effort: None,
    }
}

/// Requests for permission still unanswered; a `decide` removes one. `FAKE_ASKS=0` starts with none.
/// The squad's "always allow" rules, as settings lists them.
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
        std::sync::Mutex::new(vec![phone("a1", "Abel의 iPhone", "ios", true, Some(now - 60)), phone("b2", "iPad Pro", "ipados", false, Some(now - 3 * 3600))])
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
                    let _ = events.send(Event::PairAsked { name: "Abel의 iPhone 16".into(), platform: "ios".into() });
                });
            }
        }
        Command::PairAnswer { approve: true } => {
            tokio::time::sleep(Duration::from_millis(700)).await;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            phones().lock().unwrap().push(equipe_core::protocol::PairedDevice { id: "c3".into(), name: "Abel의 iPhone 16".into(), platform: "ios".into(), added: now, online: true, seen: Some(now) });
            let _ = events.send(Event::Paired { name: "Abel의 iPhone 16".into() });
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
        let on = std::env::var("FAKE_ASKS").map_or(true, |v| v != "0");
        std::sync::Mutex::new(if on {
            vec![
                ask(7, 3, "backend", "Bash", "cargo test -p equipe-core -- --nocapture\ncat > /tmp/out.txt <<EOF\nx\nEOF", Some("테스트를 돌리기 전에 허가가 필요합니다")),
                ask(8, 2, "frontend", "Send", "press return in Slack · #general\n내일 회의 3시로 옮겨요", None),
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
                    text.push_str(&format!("// {branch} line {n}: 결제 흐름을 정리합니다\n"));
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
    let empty = std::env::var("FAKE_EMPTY").is_ok_and(|v| v == "1");
    let mut out = vec![
        ticket(47, "in_progress", "web", "frontend", "상품 상세 페이지 이미지 갤러리", "스와이프와 확대를 지원하는 갤러리.", &[], Some("equipe/t-47-gallery"), None),
        ticket(45, "todo", "web", "frontend", "결제 완료 후 주문 내역으로 이동", "- 결제 완료 화면에서 주문 내역 링크\n- 영수증 메일 발송 확인", &[42], None, None),
        ticket(38, "done", "web", "frontend", "헤더 내비게이션 정리", "", &[], None, Some("정리했습니다.")),
        ticket(36, "done", "web", "designer", "로그인 화면 시안", "", &[], None, None),
        ticket(60, "in_progress", "app", "designer", "온보딩 세 화면 시안", "앱 첫 실행 온보딩 3장.", &[], None, None),
        ticket(61, "todo", "app", "frontend", "푸시 알림 권한 요청 시점 조정", "", &[], None, None),
        ticket(62, "todo", "app", "frontend", "설정 화면 다크 모드", "", &[60], None, None),
        ticket(58, "done", "app", "designer", "앱 아이콘 후보 세 가지", "", &[], None, None),
        ticket(66, "cancelled", "app", "frontend", "태블릿 레이아웃", "", &[], None, None),
        ticket(70, "in_progress", "api", "backend", "주문 API 페이지네이션", "커서 기반 페이지네이션.", &[], Some("equipe/t-70-cursor"), None),
        ticket(72, "todo", "api", "backend", "주문 검색 인덱스", "", &[70], None, None),
        ticket(68, "done", "api", "backend", "헬스 체크 엔드포인트", "", &[], None, None),
    ];
    if !empty {
        out.extend([
            ticket(42, "review", "web", "frontend", "체크아웃 페이지 결제 수단 선택", "- [ ] 카드와 계좌이체 둘 다 선택된다\n- [ ] 선택이 새로고침 후에도 남는다\n- [ ] 모바일 너비에서 깨지지 않는다", &[], Some("equipe/t-42-checkout"), Some("결제 수단 선택을 **라디오 그룹**으로 바꿨습니다.\n\n- 카드 / 계좌이체 / 간편결제\n- 선택은 `localStorage`에 남습니다\n- 결제 API 호출은 `pay.ts`로 모았습니다")),
            ticket(71, "review", "api", "backend", "요청 수 제한 미들웨어", "- [ ] 분당 60회를 넘으면 429\n- [ ] 헤더에 남은 횟수", &[], Some("equipe/t-71-rate-limit"), Some("토큰 버킷으로 분당 60회 제한을 넣었습니다. 테스트 6개 추가.")),
            ticket(55, "waiting", "web", "frontend", "결제 테스트 키 연결", "Stripe 테스트 모드로 결제를 끝까지 돌려 봅니다.", &[], None, Some("Waiting on you:\n- [ ] Stripe 대시보드에서 테스트 키 만들기\n- [ ] `.env.local`에 STRIPE_KEY 넣기\n- [ ] Vercel 프로젝트에도 같은 키 추가")),
            ticket(73, "waiting", "api", "backend", "운영 DB 읽기 전용 계정", "", &[], None, Some("Waiting on you:\n- [ ] 운영 DB 읽기 전용 비밀번호 알려 주기")),
            ticket(52, "proposed", "web", "frontend", "다크 모드 토글", "사용자 설정에 다크 모드를 넣으면 야간 이용 시간이 늘 것 같습니다.\n\n- [ ] 시스템 설정을 따른다\n- [ ] 직접 고를 수 있다", &[], None, None),
            ticket(64, "proposed", "app", "designer", "빈 화면 일러스트 세트", "", &[], None, None),
        ]);
        let mut task = ticket(63, "review", "app", "designer", "경쟁 앱 온보딩 조사", "", &[], None, Some("다섯 개 앱의 온보딩을 정리했습니다. `research/onboarding.md`"));
        task.kind = equipe_core::protocol::TicketKind::Task;
        out.push(task);
        let mut done = ticket(51, "done", "web", "frontend", "푸터 링크 정리", "", &[], Some("equipe/t-51-footer"), Some("링크를 정리했습니다."));
        done.merged = Some(false);
        out.push(done);
        let mut clash = ticket(49, "done", "web", "frontend", "공지 배너", "", &[], Some("equipe/t-49-banner"), Some("상단 공지 배너를 넣었습니다."));
        clash.merged = Some(false);
        clash.off_main = Some(equipe_core::protocol::OffMain::Conflict { detail: "equipe/t-49-banner conflicts with main in README.md — nothing was merged".into() });
        out.push(clash);
    }
    if your_turn().is_some() {
        out.push(turn_ticket());
    }
    out
}

/// `FAKE_TURN`: T-117 waits for the person (`1`), and is answered elsewhere (`elsewhere`).
fn your_turn() -> Option<String> {
    std::env::var("FAKE_TURN").ok().filter(|v| !v.is_empty() && v != "0")
}

/// What T-117 asks: as the person once saw it joined into one line.
const TURN_ASKS: [&str; 3] = [
    "심사 연락처(App Review Information)를 알려 주세요: 이름, 전화번호(국가번호 포함), 이메일. 심사팀만 보고 공개되지 않습니다.",
    "회사 iOS 엔지니어 리뷰를 언제, 누가 할지 정해 주세요. equipe-app 규칙상 스토어 출시 전에 꼭 받아야 합니다.",
    "스크린샷과 문구를 확인해 주세요: outputs/T-117 앱스토어 제출/screenshots-6.9/ (en·ko 5장씩, 실제 앱 화면) 와 listing.json. ko 스크린샷 속 티켓 제목은 데모 데이터라 영어입니다. 그대로 둬도 되면 '괜찮음'이라고만 답해 주세요.",
];

/// T-117, waiting, with the folder and file its third ask names on disk.
fn turn_ticket() -> Ticket {
    let shots = repo("app").join("outputs/T-117 앱스토어 제출/screenshots-6.9");
    let _ = std::fs::create_dir_all(&shots);
    let _ = std::fs::write(repo("app").join("listing.json"), "{ \"name\": \"equipe\" }\n");
    let asks: Vec<String> = TURN_ASKS.iter().map(|a| (*a).to_owned()).collect();
    let note = equipe_core::protocol::asks::waiting_note(&asks, Some("앱스토어 심사 제출에 필요합니다"));
    let mut t = ticket(117, "waiting", "app", "frontend", "앱스토어 제출", "App Store Connect에 1.0을 제출합니다.", &[], None, Some(&note));
    t.kind = equipe_core::protocol::TicketKind::Task;
    t.asks = asks;
    t
}

fn tickets() -> &'static std::sync::Mutex<Vec<Ticket>> {
    static TICKETS: std::sync::OnceLock<std::sync::Mutex<Vec<Ticket>>> = std::sync::OnceLock::new();
    TICKETS.get_or_init(|| std::sync::Mutex::new(board()))
}

/// A command about a ticket, done as the daemon would: the ticket as it is now.
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
        Entry::User { text: format!("T-{ticket} 시작해 주세요.") },
        Entry::Reply { text: "네, 지금 구조부터 보겠습니다. 기존 컴포넌트를 재사용하고 테스트를 먼저 쓰겠습니다.".into() },
        Entry::Tool { tool: tool.0.into(), target: Some(if tool.0 == "Bash" { tool.1.to_owned() } else { format!("{}/{}", repo(project).display(), tool.1) }), outcome: None },
    ];
    info.estimate = estimate;
    info
}

#[path = "fake_daemon/torture.rs"]
mod torture;

type Turns = Arc<Mutex<HashMap<SessionId, tokio::task::JoinHandle<()>>>>;

/// Where a message goes, as the daemon decides: the project in focus, each
/// member's current conversation, and the lanes opened beside them.
#[derive(Default)]
struct Routes {
    focus: Option<String>,
    thread: HashMap<SessionId, Option<String>>,
    /// `(lane session, member session, project)`.
    lanes: Vec<(SessionId, SessionId, Option<String>)>,
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
                            squads: ["demo", "side"].map(|name| rpc::SquadEntry { name: name.into(), projects: vec!["/tmp/fake/site".into()], home: format!("/tmp/fake/{name}-home").into() }).to_vec(),
                            last: Some("demo".into()),
                        };
                        if writer.write_all(Message::response(id, squads).to_line().as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    (Some(rpc::HELLO), Some(id)) => {
                        let params: rpc::HelloParams = message.params_as().unwrap();
                        let history = vec![
                            Entry::User { text: "어제 하던 일 이어서 해 줘".into() },
                            Entry::Reply { text: "네, **T-9**부터 이어서 하겠습니다.".into() },
                            Entry::Tool { tool: "Read".into(), target: Some("/tmp/fake/site/a.rs".into()), outcome: None },
                            Entry::Tool { tool: "Edit".into(), target: Some("/tmp/fake/site/a.rs".into()), outcome: None },
                            Entry::Reply { text: "고쳤습니다.".into() },
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
                        history.extend(std::env::var("FAKE_SAID").ok().filter(|s| !s.is_empty()).map(|text| Entry::User { text }));
                        if let Some(how) = your_turn() {
                            history.push(Entry::Note { text: "frontend handed in T-63 for your review · 경쟁 앱 온보딩 조사.".into() });
                            history.push(Entry::Note { text: "T-117 waits for you — frontend needs something only you can do · 앱스토어 제출.".into() });
                            if how == "elsewhere" {
                                let events = events.clone();
                                tokio::spawn(async move {
                                    let after = std::env::var("FAKE_TURN_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(5000);
                                    tokio::time::sleep(Duration::from_millis(after)).await;
                                    let changed = {
                                        let mut all = tickets().lock().unwrap();
                                        all.iter_mut().find(|t| t.id == 117 && t.status == TicketStatus::Waiting).map(|t| {
                                            t.status = TicketStatus::InProgress;
                                            t.note = Some("Resumed: 1. 심사 연락처(App Review Information)를 알려 주세요: 이름, 전화번호(국가번호 포함), 이메일. 심사팀만…\n→ 고아벨, +82 10 1234 5678, review@equipe.im\n\n3. 스크린샷과 문구를 확인해 주세요: outputs/T-117 앱스토어 제출/screenshots-6.9/ (en·ko 5장씩…\n→ 괜찮음".into());
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
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                        let frontend = on(info(2, "frontend", vec![]), 47, "web", "상품 상세 페이지 이미지 갤러리", true, ("Edit", "src/gallery.tsx"), Some(Estimate { done_at: Some(now + 25 * 60), ..Default::default() }));
                        let backend = on(info(3, "backend", vec![]), 70, "api", "주문 API 페이지네이션", true, ("Bash", "cargo test -p api"), Some(Estimate { done_at: Some(now + 50 * 60), guessed: true, ..Default::default() }));
                        let designer = on(info(4, "designer", vec![]), 60, "app", "온보딩 세 화면 시안", false, ("Read", "brief.md"), Some(Estimate { done_at: Some(now.saturating_sub(12 * 60)), ..Default::default() }));
                        // Settings have something to show: a member on its own mode and effort, rules kept.
                        let mut designer = designer;
                        designer.mode = Some(equipe_core::protocol::Mode::Ask);
                        let mut backend = backend;
                        backend.effort = Some("high".into());
                        let snapshot = Snapshot {
                            squad: params.squad,
                            projects: vec![repo("web"), repo("app"), repo("api")],
                            sessions: vec![info(1, "pm", history), frontend, backend, designer],
                            approvals: asks().lock().unwrap().clone(),
                            tickets: tickets().lock().unwrap().clone(),
                            allowed: allowed().lock().unwrap().clone(),
                            browsers_shown: true,
                            ..Default::default()
                        };
                        let hello = Hello { version: "fake".into(), snapshot };
                        writer.write_all(Message::response(id, hello).to_line().as_bytes()).await.unwrap();
                    }
                    // Kept as the daemon keeps them: under `uploads`, named by equipe.
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
                    (Some(rpc::COMMAND), None) => {
                        let command: Command = message.params_as().unwrap();
                        eprintln!("command: {command:?}");
                        match command {
                            Command::Submit { session, text } => {
                                let working = |id: SessionId| {
                                    let turns = turns.clone();
                                    async move { turns.lock().await.get(&id).is_some_and(|t| !t.is_finished()) }
                                };
                                // Route like the daemon: the conversation about the project in focus,
                                // beside the member's other work if it is at work in another.
                                let mut r = routes.lock().await;
                                let wanted = r.focus.clone();
                                let lane = r.lanes.iter().find(|(_, of, project)| *of == session && *project == wanted).map(|l| l.0);
                                let session = match lane {
                                    Some(lane) => lane,
                                    None => {
                                        let here = r.thread.get(&session).cloned().unwrap_or(None);
                                        if here != wanted && working(session).await {
                                            let id = 100 + r.lanes.len() as SessionId;
                                            r.lanes.push((id, session, wanted.clone()));
                                            let role = ["pm", "frontend", "backend", "designer"][(session as usize - 1).min(3)];
                                            let _ = events.send(Event::SessionStarted { session: id, project: "/tmp/fake/site".into(), engine: Engine::Claude, model: None, workspace: None, member: Some(Member { role: role.into(), name: role.into() }), squad: Some("demo".into()) });
                                            let _ = events.send(Event::LaneOpened { session: id, of: session, project: wanted.clone() });
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
                                    let _ = events.send(Event::SteerQueued { session, text });
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
                            Command::SubmitLater { session, text } => {
                                let _ = events.send(Event::MessageQueued { session, text, project: None });
                            }
                            Command::Unqueue { session, text } => {
                                let _ = events.send(Event::MemoWithdrawn { session, text });
                            }
                            Command::Interrupt { session } => {
                                if let Some(task) = turns.lock().await.remove(&session) {
                                    task.abort();
                                }
                                let _ = events.send(Event::TurnEnded { session, outcome: TurnOutcome::Interrupted, usage: None });
                            }
                            Command::Decide { squad, approval, decision } => {
                                asks().lock().unwrap().retain(|a| a.id != approval);
                                let _ = events.send(Event::ApprovalResolved { squad, approval, decision });
                            }
                            Command::Focus { project, .. } => {
                                routes.lock().await.focus = project;
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
                                if let Some(ticket) = on_ticket_command(&other) {
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
    let read = |p: &str, f: &str| Entry::Tool { tool: "Read".into(), target: Some(format!("/tmp/fake/site/{p}/{f}")), outcome: None };
    vec![
        about(Some("app")),
        user("온보딩 첫 화면 문구를 다듬어 줘"),
        read("app", "src/onboarding.tsx"),
        reply("네, **T-60** 시안에 맞춰 첫 화면 제목을 \"팀이 일하는 곳\"으로 바꾸고 버튼은 \"시작하기\" 하나로 줄이겠습니다."),
        about(Some("web")),
        Entry::Note { text: "frontend이(가) T-47을(를) 검토해 달라고 넘겼습니다 · 상품 상세 페이지 이미지 갤러리.".into() },
        user("상품 상세 갤러리는 어디까지 됐어?"),
        read("web", "src/gallery.tsx"),
        read("web", "src/gallery.test.tsx"),
        reply("**T-47**은 검토를 기다리고 있습니다. 스와이프와 확대는 끝났고, 썸네일 줄은 다음 차례입니다."),
        about(None),
        user("이번 주 우선순위 정리해 줘"),
        reply("이번 주는 세 가지입니다.\n\n1. web — 갤러리 검토 (**T-47**)\n2. api — 주문 페이지네이션 (**T-70**)\n3. app — 온보딩 시안 (**T-60**)"),
        about(Some("app")),
        user("온보딩 두 번째 화면도 같은 톤으로"),
        read("app", "src/onboarding.tsx"),
        reply("두 번째 화면도 같은 톤으로 맞췄습니다. 설명은 두 줄로 줄였어요."),
    ]
}

/// One turn: a couple of steps, then the reply a few characters at a time.
async fn turn(session: SessionId, events: broadcast::Sender<Event>, seconds: f32) {
    let tool = |tool: &str, target: &str| Event::ToolUse { session, tool: tool.into(), target: Some(target.into()) };
    tokio::time::sleep(Duration::from_millis(200)).await;
    for step in [tool("Read", "/tmp/fake/site/a.rs"), tool("Read", "/tmp/fake/site/b.rs"), tool("Edit", "/tmp/fake/site/a.rs")] {
        let _ = events.send(step);
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    let chars: Vec<char> = REPLY.chars().collect();
    let chunks = chars.chunks(4).count().max(1);
    let gap = Duration::from_secs_f32(seconds / chunks as f32);
    for chunk in chars.chunks(4) {
        let _ = events.send(Event::TextDelta { session, text: chunk.iter().collect() });
        tokio::time::sleep(gap).await;
    }
    let usage = Usage { input_tokens: Some(1200), output_tokens: Some(340), context_tokens: Some(420), context_window: Some(1000), ..Default::default() };
    let _ = events.send(Event::TurnEnded { session, outcome: TurnOutcome::Completed, usage: Some(usage) });
    let _ = events.send(Event::Suggestion { session, text: "다른 페이지도 정리해줘".into() });
}
