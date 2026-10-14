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
use equipe_core::protocol::{Access, Command, Engine, Entry, Event, Member, SessionId, SessionInfo, SessionState, Snapshot, TurnOutcome, Usage};
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
        recent,
        queued: vec![],
        work: None,
        mode: None,
    }
}

type Turns = Arc<Mutex<HashMap<SessionId, tokio::task::JoinHandle<()>>>>;

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
    loop {
        let (stream, _) = listener.accept().await.unwrap();
        tokio::spawn(serve(stream, events.clone(), turns.clone(), seconds));
    }
}

async fn serve(stream: UnixStream, events: broadcast::Sender<Event>, turns: Turns, seconds: f32) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let mut feed = events.subscribe();
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { return };
                let Ok(message) = Message::from_line(&line) else { continue };
                match (message.method.as_deref(), message.id) {
                    (Some(rpc::HELLO), Some(id)) => {
                        let params: rpc::HelloParams = message.params_as().unwrap();
                        let history = vec![
                            Entry::User { text: "어제 하던 일 이어서 해 줘".into() },
                            Entry::Reply { text: "네, **T-9**부터 이어서 하겠습니다.".into() },
                            Entry::Tool { tool: "Read".into(), target: Some("/tmp/fake/site/a.rs".into()) },
                            Entry::Tool { tool: "Edit".into(), target: Some("/tmp/fake/site/a.rs".into()) },
                            Entry::Reply { text: "고쳤습니다.".into() },
                        ];
                        let snapshot = Snapshot {
                            squad: params.squad,
                            projects: vec!["/tmp/fake/site".into(), "/tmp/fake/app".into()],
                            sessions: vec![info(1, "pm", history), info(2, "frontend", vec![]), info(3, "backend", vec![])],
                            ..Default::default()
                        };
                        let hello = Hello { version: "fake".into(), snapshot };
                        writer.write_all(Message::response(id, hello).to_line().as_bytes()).await.unwrap();
                    }
                    (Some(rpc::COMMAND), None) => {
                        let command: Command = message.params_as().unwrap();
                        eprintln!("command: {command:?}");
                        match command {
                            Command::Submit { session, text } => {
                                let busy = turns.lock().await.get(&session).is_some_and(|t| !t.is_finished());
                                if busy {
                                    let _ = events.send(Event::MessageQueued { session, text });
                                } else {
                                    let _ = events.send(Event::UserMessage { session, text });
                                    let task = tokio::spawn(turn(session, events.clone(), seconds));
                                    turns.lock().await.insert(session, task);
                                }
                            }
                            Command::Interrupt { session } => {
                                if let Some(task) = turns.lock().await.remove(&session) {
                                    task.abort();
                                }
                                let _ = events.send(Event::TurnEnded { session, outcome: TurnOutcome::Interrupted, usage: None });
                            }
                            Command::Focus { project, .. } => {
                                let _ = events.send(Event::Thread { session: 1, project });
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
}
