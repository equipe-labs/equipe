//! A stand-in daemon for looking at the conversation screen without spending a
//! real engine's turns: it answers `hello` with a small squad (a PM, a frontend
//! and a backend member, two projects), and answers every message with a streamed
//! Korean reply — Markdown with a list, a table, a code block and `T-12` — between
//! tool steps. `interrupt` ends the turn at once; `focus` moves the PM's thread.
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
use equipe_core::protocol::{Access, Approval, Command, Engine, Entry, Event, Member, SessionId, SessionInfo, SessionState, Snapshot, TurnOutcome, Usage};
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
            id,
            session,
            member: member.into(),
            ticket: Some(52),
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

fn ticket(id: u32, status: &str, note: Option<&str>) -> equipe_core::protocol::Ticket {
    ticket_in(id, status, note, "site")
}

fn ticket_in(id: u32, status: &str, note: Option<&str>, project: &str) -> equipe_core::protocol::Ticket {
    serde_json::from_value(serde_json::json!({"id": id, "title": format!("티켓 {id}의 제목"), "body": "", "status": status, "owner": "backend", "assignee": "backend", "project": format!("/tmp/fake/{project}"), "depends_on": [], "branch": null, "created_by": "pm", "note": note})).unwrap()
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
                        let snapshot = Snapshot {
                            squad: params.squad,
                            projects: vec!["/tmp/fake/site".into(), "/tmp/fake/app".into()],
                            sessions: vec![info(1, "pm", history), info(2, "frontend", vec![]), info(3, "backend", vec![])],
                            approvals: asks().lock().unwrap().clone(),
                            tickets: vec![ticket_in(60, "in_progress", None, "app"), ticket_in(61, "todo", None, "app"), ticket(54, "review", None), ticket(35, "waiting", Some("Waiting on you:\n- [ ] 화면 기록 권한")), ticket(30, "review", None), ticket(31, "review", None)],
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
                                            let role = ["pm", "frontend", "backend"][session as usize - 1];
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
                            _ => {}
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
