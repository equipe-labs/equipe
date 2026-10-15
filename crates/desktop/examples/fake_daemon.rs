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
//! `FAKE_SECONDS` (default 4) is how long a reply streams.

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
    }
}

/// Requests for permission still unanswered; a `decide` removes one. `FAKE_ASKS=0` starts with none.
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
        ("web", vec![("equipe/t-42-checkout", vec![("src/checkout.tsx", 48), ("src/cart.ts", 12), ("src/api/pay.ts", 21)]), ("equipe/t-51-footer", vec![("src/footer.tsx", 9)])]),
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
    }
    out
}

fn tickets() -> &'static std::sync::Mutex<Vec<Ticket>> {
    static TICKETS: std::sync::OnceLock<std::sync::Mutex<Vec<Ticket>>> = std::sync::OnceLock::new();
    TICKETS.get_or_init(|| std::sync::Mutex::new(board()))
}

/// A command about a ticket, done as the daemon would: the ticket as it is now.
fn on_ticket_command(command: &Command) -> Option<Ticket> {
    let mut all = tickets().lock().unwrap();
    let id = match command {
        Command::Accept { ticket, .. } | Command::Merge { ticket, .. } | Command::Reopen { ticket, .. } | Command::Resume { ticket, .. } | Command::Cancel { ticket, .. } => *ticket,
        _ => return None,
    };
    let t = all.iter_mut().find(|t| t.id == id)?;
    match command {
        Command::Accept { .. } if t.status == TicketStatus::Proposed => t.status = TicketStatus::Todo,
        Command::Accept { .. } => {
            t.status = TicketStatus::Done;
            t.merged = t.branch.as_ref().map(|_| false);
        }
        Command::Merge { .. } => t.merged = Some(true),
        Command::Reopen { note, .. } => {
            t.status = TicketStatus::InProgress;
            t.note = Some(note.clone());
        }
        Command::Resume { .. } => t.status = TicketStatus::InProgress,
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
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                        let frontend = on(info(2, "frontend", vec![]), 47, "web", "상품 상세 페이지 이미지 갤러리", true, ("Edit", "src/gallery.tsx"), Some(Estimate { done_at: Some(now + 25 * 60), ..Default::default() }));
                        let backend = on(info(3, "backend", vec![]), 70, "api", "주문 API 페이지네이션", true, ("Bash", "cargo test -p api"), Some(Estimate { done_at: Some(now + 50 * 60), guessed: true, ..Default::default() }));
                        let designer = on(info(4, "designer", vec![]), 60, "app", "온보딩 세 화면 시안", false, ("Read", "brief.md"), Some(Estimate { done_at: Some(now.saturating_sub(12 * 60)), ..Default::default() }));
                        let snapshot = Snapshot {
                            squad: params.squad,
                            projects: vec![repo("web"), repo("app"), repo("api")],
                            sessions: vec![info(1, "pm", history), frontend, backend, designer],
                            approvals: asks().lock().unwrap().clone(),
                            tickets: tickets().lock().unwrap().clone(),
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
