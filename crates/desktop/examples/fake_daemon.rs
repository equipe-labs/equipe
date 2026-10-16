//! A stand-in daemon for looking at the screens without spending a real
//! engine's turns: it answers `hello` with a squad (a PM, frontend, backend and
//! designer, three projects — real git repositories it makes under
//! `$TMPDIR/equipe-fake`, with a branch for each piece of work handed in) and a
//! board with work in every state, and answers every message with a streamed
//! Korean reply — Markdown with a list, a table, a code block and `T-12` —
//! between tool steps. `interrupt` ends the turn at once; `focus` moves the PM's
//! thread; accept, merge, reopen, resume and cancel move tickets as the daemon
//! would — and so do the person's own create, edit, move and comment (by
//! `equipe_core::board`'s rules), each kept in the ticket's record.
//! `FAKE_EMPTY=1` starts with nothing waiting for the person;
//! `FAKE_BUSY=1` adds four members at work in web and more finished today;
//! `FAKE_SUBAGENTS=1` has frontend and backend hand work to subagents that come and go;
//! `FAKE_WORKING=<n>` has only the first n of frontend and backend at work (`0`: nobody).
//!
//! ```text
//! export EQUIPE_HOME=$(mktemp -d) EQUIPE_SQUAD=demo
//! cargo run -p equipe-desktop --example fake_daemon &
//! cargo run -p equipe-desktop
//! ```
//! `FAKE_SECONDS` (default 4) is how long a reply streams; `FAKE_SAID` adds a
//! last message of the person's to the PM's history (attachment tags and all).
//! `FAKE_CARDS=1` ends it on a reply naming T-42 (a review), T-117 and T-999
//! (no such ticket), to rest the pointer on.
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
//! (`fake_daemon/pictures.rs`).
//!
//! `FAKE_HISTORY=<n>`: the PM's history is a long day's, `n` lines of it
//! (`fake_daemon/history.rs`); `FAKE_LONG_REPLY=1`: its answers are long too.
//!
//! `FAKE_TEAM=1`: the PM proposes one more frontend, for the person to
//! approve on a card (`fake_daemon/team.rs`; `remove`, `elsewhere`).
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
use equipe_core::protocol::{Access, Approval, Command, Engine, Entry, Estimate, Event, Member, Priority, SessionId, SessionInfo, SessionState, Snapshot, SubagentInfo, Ticket, TicketEntry, TicketStatus, TurnOutcome, Usage, WorkInfo};
use equipe_providers::credentials::equipe_home;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, broadcast};

const REPLY: &str = "네, 확인했습니다. 지금 보드를 살펴보고 **T-12**를 먼저 처리하겠습니다.\n\n진행 순서는 다음과 같습니다.\n\n- 헤더 컴포넌트 정리\n- 입력창 한글 조합 확인\n- 스트리밍 표시 점검 (`T-13`)\n\n| 항목 | 상태 | 담당 |\n|---|---|---|\n| 헤더 | 진행 중 | frontend |\n| 입력창 | 대기 | frontend |\n| 데몬 연결 | 완료 | backend |\n\n```rust\nfn main() {\n    println!(\"안녕, equipe\");\n}\n```\n\n끝나면 다시 알려 드리겠습니다.";

/// `FAKE_ASKLIST`'s reply: what the PM needs to know, as a numbered list in prose.
const ASKLIST: &str = "EU 디지털 서비스법(DSA) 때문에 EU 스토어에 앱을 내려면 판매자 연락처를 공개해야 합니다. App Store Connect에 아직 비어 있어서 **T-117** 제출이 여기서 멈춰 있어요.\n\n## 알려 주세요\n\n1. EU 스토어에 공개할 전화번호: 국가번호 포함 (예: +82 10-1234-5678)\n2. 공개할 이메일: 스토어 페이지에 그대로 보입니다\n3. T-117 심사 연락처: 위와 같은 번호와 메일을 써도 되는지\n\n받으면 바로 입력합니다.";

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
        steering_in: vec![],
        lane: None,
        estimate: None,
        recent,
        queued: vec![],
        work: None,
        mode: None,
        effort: None,
        subagents: Vec::new(),
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
    // `FAKE_DECIDE=logo`: the designer's brand guide, handed in with its pictures.
    if pictures::on() {
        let note = "브랜드 가이드를 정리했습니다 — 이름과 로고는 그대로 두고 쓰임새를 맞췄습니다.\n\n- 로고 시트: 흰 바탕과 어두운 바탕\n- 앱 아이콘 1024px\n- 색상 다섯 가지 (`brand/palette.md`)";
        out.push(pictures::hand_in(ticket(144, "review", "app", "designer", "equipe 브랜드 정리·체계화 (이름·로고 유지)", "- [ ] 로고가 작은 크기에서도 읽힌다\n- [ ] 색상 대비가 충분하다", &[], None, Some(note))));
    }
    if busy() {
        out.extend(BUSY.iter().map(|(id, _, who, title)| ticket(*id, "in_progress", "web", who, title, "", &[], None, None)));
        out.extend([
            ticket(84, "waiting", "web", "frontend", "도메인 DNS 레코드 확인", "", &[], None, Some("Waiting on you:\n- [ ] DNS에 CNAME 추가")),
            ticket(85, "todo", "web", "frontend", "장바구니 수량 버튼 접근성", "", &[], None, None),
            ticket(86, "todo", "web", "qa", "결제 회귀 테스트 목록", "", &[], None, None),
            ticket(87, "done", "web", "qa", "로그인 오류 메시지 문구", "", &[], None, None),
            ticket(88, "done", "app", "designer", "스플래시 화면 색 조정", "", &[], None, None),
            ticket(89, "done", "api", "backend", "주문 상태 웹훅", "", &[], None, None),
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
                        question: "로그인은 어떤 방식으로 만들까요?".into(),
                        header: "로그인".into(),
                        options: vec![
                            choice("이메일 매직 링크 (추천)", "비밀번호 없이 메일 링크로 로그인 — 가장 빨리 만들 수 있습니다"),
                            choice("Google 로그인", "한 번에 가입되지만 Google 계정이 있어야 합니다"),
                            choice("이메일 + 비밀번호", "익숙하지만 재설정 흐름까지 만들어야 합니다"),
                        ],
                        multi_select: false,
                        ..Question::default()
                    },
                    Question {
                        question: "첫 릴리스에 어떤 화면을 넣을까요?".into(),
                        header: "범위".into(),
                        options: vec![
                            choice("온보딩", "세 화면 — 시안이 T-60에 있습니다"),
                            choice("주문 내역", "api의 페이지네이션(T-70) 다음에"),
                            choice("설정", "알림·계정 관리"),
                        ],
                        multi_select: true,
                        ..Question::default()
                    },
                ],
                asked_at: now.saturating_sub(90),
            },
            QuestionSet {
                id: 9002,
                session: 3,
                member: "backend".into(),
                ticket: Some(70),
                questions: vec![Question {
                    question: "주문 목록 페이지 크기는 얼마로 할까요?".into(),
                    header: "페이지".into(),
                    options: vec![choice("20개 (추천)", "앱 목록 화면 한 번에 맞습니다"), choice("50개", "요청은 줄지만 첫 화면이 느려집니다")],
                    multi_select: false,
                    ..Question::default()
                }],
                asked_at: now.saturating_sub(30),
            },
        ];
        let sets = if pictures::on() { std::iter::once(pictures::question_set(now)).chain(sets).collect() } else { sets };
        std::sync::Mutex::new(if deciding().is_some() { sets } else { Vec::new() })
    })
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
    (80, 5, "frontend-2", "검색 결과 무한 스크롤"),
    (81, 6, "frontend-3", "상품 카드 스켈레톤 로딩과 이미지 지연 로딩, 접근성 라벨까지 한 번에 정리"),
    (82, 7, "qa", "체크아웃 E2E 테스트"),
    (83, 8, "data", "구매 전환 대시보드 이벤트"),
];

/// `FAKE_SUBAGENTS=1`: frontend and backend hand side work to subagents, which
/// start, take steps and hand back over time, round and round — backend's
/// first already at it when a screen attaches (in the snapshot, as a daemon
/// lists them). `FAKE_SUBAGENTS_MS` (default 1500) is the beat.
fn subagents_on() -> bool {
    std::env::var("FAKE_SUBAGENTS").is_ok_and(|v| v == "1")
}

/// The subagents at work now, by session: what a snapshot lists.
fn live_subagents() -> &'static std::sync::Mutex<Vec<(SessionId, SubagentInfo, std::time::Instant)>> {
    static LIVE: std::sync::OnceLock<std::sync::Mutex<Vec<(SessionId, SubagentInfo, std::time::Instant)>>> = std::sync::OnceLock::new();
    LIVE.get_or_init(|| {
        // Backend's, at it since before anyone looked.
        let since = std::time::Instant::now() - Duration::from_secs(48);
        std::sync::Mutex::new(vec![(3, SubagentInfo { id: "sub-b0".into(), what: "API 에러 로그에서 타임아웃 원인 찾기".into(), step: Some("Grep TimeoutError logs/".into()), secs: 0 }, since)])
    })
}

/// A session's subagents as a snapshot carries them, and the delegations
/// that started them, for its conversation.
fn subagents_of(session: SessionId) -> (Vec<SubagentInfo>, Vec<Entry>) {
    let live = live_subagents().lock().unwrap();
    let subs: Vec<SubagentInfo> = live.iter().filter(|(s, ..)| *s == session).map(|(_, info, since)| SubagentInfo { secs: since.elapsed().as_secs(), ..info.clone() }).collect();
    let calls = subs.iter().map(|s| Entry::Tool { tool: "Agent".into(), target: Some(s.what.clone()), outcome: None }).collect();
    (subs, calls)
}

/// Starts, steps and ends, `beat` apart, round and round: (session, id, what), a step, or an end.
async fn subagents_play(events: broadcast::Sender<Event>) {
    let beat = Duration::from_millis(std::env::var("FAKE_SUBAGENTS_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500));
    let tasks: [(SessionId, &str, &[&str]); 6] = [
        (2, "갤러리 썸네일 크기와 비율 조사", &["Read src/gallery.tsx", "Grep aspect-ratio", "Read src/thumb.tsx"]),
        (2, "모바일 스와이프 제스처 동작 확인", &["Read src/swipe.ts", "Bash npm test swipe"]),
        (3, "페이지네이션 커서 인코딩 비교", &["Read api/cursor.rs", "WebFetch RFC 4648"]),
        (2, "이미지 지연 로딩 테스트 작성", &["Write src/gallery.test.tsx", "Bash npm test gallery"]),
        (2, "접근성 라벨과 키보드 이동 점검", &["Read src/gallery.tsx", "Grep aria-label"]),
        (2, "번들 크기 변화 측정", &["Bash npm run build", "Read dist/stats.json"]),
    ];
    let mut round = 0u32;
    loop {
        let mut started: Vec<(SessionId, String, usize)> = Vec::new();
        for (n, (session, what, _)) in tasks.iter().enumerate() {
            let id = format!("sub-{round}-{n}");
            let _ = events.send(Event::ToolUse { session: *session, tool: "Agent".into(), target: Some((*what).into()) });
            live_subagents().lock().unwrap().push((*session, SubagentInfo { id: id.clone(), what: (*what).into(), step: None, secs: 0 }, std::time::Instant::now()));
            let _ = events.send(Event::SubagentStarted { session: *session, id: id.clone(), what: (*what).into() });
            started.push((*session, id, n));
            tokio::time::sleep(beat).await;
            // Everyone at work takes a step.
            for (s, id, task) in &started {
                let steps = tasks[*task].2;
                let step = steps[(round as usize + n + task) % steps.len()];
                if let Some(entry) = live_subagents().lock().unwrap().iter_mut().find(|(_, info, _)| info.id == *id) {
                    entry.1.step = Some(step.into());
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
    let _ = events.send(Event::ToolResult { session, tool: "Agent".into(), target: what, outcome });
    let _ = events.send(Event::SubagentEnded { session, id: id.into() });
}

/// `FAKE_TURN`: T-117 waits for the person (`1`), and is answered elsewhere (`elsewhere`).
fn your_turn() -> Option<String> {
    std::env::var("FAKE_TURN").ok().filter(|v| !v.is_empty() && v != "0")
}

/// What T-117 asks: as the person saw it — one question, one thing to do.
const TURN_ASKS: [&str; 2] = [
    "App Review 연락처를 알려 주세요: 전화번호(국가번호 포함)와 이메일. 심사팀만 보고 공개되지 않습니다.",
    "App Store Connect에서 EU DSA 상태를 trader(사업자)로 바꿔 주세요 — 앱 정보 › 디지털 서비스법.",
];

/// What T-117 asked before (`FAKE_TURN=files`): as the person once saw it joined into one line.
const TURN_ASKS_FILES: [&str; 3] = [
    "심사 연락처(App Review Information)를 알려 주세요: 이름, 전화번호(국가번호 포함), 이메일. 심사팀만 보고 공개되지 않습니다.",
    "회사 iOS 엔지니어 리뷰를 언제, 누가 할지 정해 주세요. equipe-app 규칙상 스토어 출시 전에 꼭 받아야 합니다.",
    "스크린샷과 문구를 확인해 주세요: outputs/T-117 앱스토어 제출/screenshots-6.9/ (en·ko 5장씩, 실제 앱 화면) 와 listing.json. ko 스크린샷 속 티켓 제목은 데모 데이터라 영어입니다. 그대로 둬도 되면 '괜찮음'이라고만 답해 주세요.",
];

/// T-117, waiting, with the folder and file its third ask names on disk.
fn turn_ticket() -> Ticket {
    let shots = repo("app").join("outputs/T-117 앱스토어 제출/screenshots-6.9");
    let _ = std::fs::create_dir_all(&shots);
    let _ = std::fs::write(repo("app").join("listing.json"), "{ \"name\": \"equipe\" }\n");
    let files = your_turn().as_deref() == Some("files");
    let asks: Vec<String> = if files { TURN_ASKS_FILES.iter().map(|a| (*a).to_owned()).collect() } else { TURN_ASKS.iter().map(|a| (*a).to_owned()).collect() };
    let note = equipe_core::protocol::asks::waiting_note(&asks, Some("앱스토어 심사 제출에 필요합니다"));
    let mut t = ticket(117, "waiting", "app", "frontend", "앱스토어 제출", "App Store Connect에 1.0을 제출합니다.", &[], None, Some(&note));
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
                e(3 * 86_400, "pm", "created", "체크아웃 페이지 결제 수단 선택"),
                e(3 * 86_400 - 60, "equipe", "assigned", "frontend"),
                e(2 * 86_400, "you", "edited", "우선순위 보통 → 긴급"),
                e(86_400, "you", "comment", "간편결제는 토스페이만 먼저 붙여 주세요."),
                e(5_400, "frontend", "handed in", "결제 수단 선택을 라디오 그룹으로 바꿨습니다."),
            ],
        );
        out.insert(47, vec![e(2 * 86_400, "pm", "created", "상품 상세 페이지 이미지 갤러리"), e(86_400, "equipe", "assigned", "frontend")]);
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
                return Some(refuse("티켓에는 제목이 있어야 합니다.".into()));
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
                return Some(refuse(format!("서로를 기다리는 고리가 생깁니다: {}.", path.join(" → "))));
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
        Entry::User { text: format!("T-{ticket} 시작해 주세요.") },
        Entry::Reply { text: "네, 지금 구조부터 보겠습니다. 기존 컴포넌트를 재사용하고 테스트를 먼저 쓰겠습니다.".into() },
        Entry::Tool { tool: tool.0.into(), target: Some(if tool.0 == "Bash" { tool.1.to_owned() } else { format!("{}/{}", repo(project).display(), tool.1) }), outcome: None },
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

#[path = "fake_daemon/pictures.rs"]
mod pictures;
#[path = "fake_daemon/team.rs"]
mod team;

#[path = "fake_daemon/engines.rs"]
mod engines;

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
    if pictures::on() {
        pictures::make();
    }
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
                        // `FAKE_HISTORY=<n>`: a long day's history, `n` lines (`fake_daemon/history.rs`).
                        if let Some(n) = std::env::var("FAKE_HISTORY").ok().and_then(|n| n.parse().ok()) {
                            history = history::history(n);
                        }
                        history.extend(std::env::var("FAKE_SAID").ok().filter(|s| !s.is_empty()).map(|text| Entry::User { text }));
                        // `FAKE_ASKLIST=1`: the PM's last reply asks three things in a list of its own words.
                        if std::env::var("FAKE_ASKLIST").is_ok_and(|v| v == "1") {
                            history.push(Entry::User { text: "EU 스토어에도 바로 낼 수 있어?".into() });
                            history.push(Entry::Tool { tool: "WebFetch".into(), target: Some("https://developer.apple.com/help/app-store-connect/manage-compliance-information/manage-european-union-digital-services-act-trader-requirements".into()), outcome: None });
                            history.push(Entry::Reply { text: ASKLIST.into() });
                        }
                        if pictures::on() {
                            history.push(Entry::Note { text: pictures::NEWS.into() });
                        }
                        if browser::handover() {
                            history.push(Entry::User { text: "App Store Connect에서 EU DSA 거래자 상태를 바꿔 줘".into() });
                            history.push(Entry::Reply { text: "제 브라우저는 App Store Connect에 로그인되어 있지 않아서, 로그인만 먼저 해 주셔야 합니다.".into() });
                            history.push(Entry::Note { text: format!("🖥 pm's browser comes up on this Mac's screen: {}", browser::PM_WHY) });
                        }
                        // `FAKE_CARDS=1`: a last reply naming a review, a wait and a number
                        // long gone, for the tickets' cards on the pointer.
                        if std::env::var_os("FAKE_CARDS").is_some() {
                            history.push(Entry::User { text: "지금 볼 거 있어?".into() });
                            history.push(Entry::Reply { text: "T-42가 검토를 기다리고 있어요. T-117은 당신 차례이고, 예전 T-999는 이제 없습니다.".into() });
                        }
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
                                            use equipe_core::protocol::asks::{self, Answers, Reply};
                                            let answers = Answers { replies: vec![Reply::Text("+82 10 1234 5678, review@equipe.im".into()), Reply::Later], ..Answers::default() };
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
                                close_questions(9001, Some(vec![Answer { picked: vec![1], other: None }, Answer { picked: vec![0, 2], other: Some("설정은 알림만".into()) }]), &events);
                            });
                        }
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                        let frontend = on(info(2, "frontend", vec![]), 47, "web", "상품 상세 페이지 이미지 갤러리", working() >= 1, ("Edit", "src/gallery.tsx"), Some(Estimate { done_at: Some(now + 25 * 60), ..Default::default() }));
                        let backend = on(info(3, "backend", vec![]), 70, "api", "주문 API 페이지네이션", working() >= 2, ("Bash", "cargo test -p api"), Some(Estimate { done_at: Some(now + 50 * 60), guessed: true, ..Default::default() }));
                        let designer = on(info(4, "designer", vec![]), 60, "app", "온보딩 세 화면 시안", false, ("Read", "brief.md"), Some(Estimate { done_at: Some(now.saturating_sub(12 * 60)), ..Default::default() }));
                        // Settings have something to show: a member on its own mode and effort, rules kept.
                        let mut designer = designer;
                        designer.mode = Some(equipe_core::protocol::Mode::Ask);
                        let mut backend = backend;
                        backend.effort = Some("high".into());
                        let mut frontend = frontend;
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
                        let mut sessions = vec![info(1, "pm", history), frontend, backend, designer];
                        if busy() {
                            for (ticket, session, name, title) in BUSY {
                                let role = name.split('-').next().unwrap_or(name);
                                let mut member = on(info(session, role, vec![]), ticket, "web", title, true, ("Edit", "src/search.tsx"), None);
                                member.member = Some(Member { role: role.into(), name: name.into() });
                                sessions.push(member);
                            }
                        }
                        engines::mix(&mut sessions);
                        let snapshot = Snapshot {
                            plans: engines::plans(),
                            plan: engines::claude(),
                            engine_switching: engines::switching(),
                            engine_proposals: engines::proposals(),
                            squad: params.squad,
                            projects: vec![repo("web"), repo("app"), repo("api")],
                            sessions,
                            approvals: asks().lock().unwrap().clone(),
                            questions: questions().lock().unwrap().clone(),
                            team_changes: team::changes(),
                            tickets: tickets().lock().unwrap().clone(),
                            allowed: allowed().lock().unwrap().clone(),
                            browsers_shown: true,
                            ..Default::default()
                        };
                        team::on_hello(&events);
                        let hello = Hello { version: "fake".into(), snapshot };
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
                    // A member's picture: from the pictures' folder only.
                    (Some(rpc::READ_IMAGE), Some(id)) => {
                        let ask: rpc::ReadImage = message.params_as().unwrap();
                        let reply = match pictures::read(&ask) {
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
                                let _ = events.send(Event::QueueSent { session, texts: order.clone() });
                                if let Some(task) = turns.lock().await.remove(&session) {
                                    task.abort();
                                }
                                let _ = events.send(Event::TurnEnded { session, outcome: TurnOutcome::Interrupted, usage: None });
                                let _ = events.send(Event::UserMessage { session, text: order.join("\n\n") });
                                turns.lock().await.insert(session, tokio::spawn(turn(session, events.clone(), seconds)));
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
                                    let _ = events.send(Event::Notice { session: 1, text: "🖐 브라우저를 돌려줬습니다.".into() });
                                    turns.lock().await.insert(1, tokio::spawn(turn(1, events.clone(), seconds)));
                                }
                                if browser::on_command(&other, &events) || team::on_command(&other, &events) || engines::on_command(&other, &events) {
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
    let _ = events.send(Event::Suggestion { session, text: "다른 페이지도 정리해줘".into() });
}
