//! `FAKE_RETRO=1`: the PM's weekly retro, come in — last week's and this
//! week's runs in the PM's conversation (the facts as the daemon gathers
//! them, and the retro the PM wrote from them), and this week's changes
//! waiting in 확인 필요 as one card, each with the lesson behind it.

use equipe_core::protocol::decide::{Choice, Question, QuestionSet, Retro};
use equipe_core::protocol::{Entry, Ticket, TicketEntry, TicketKind, TicketStatus};
use equipe_core::retro::{self, Turn, Wait, Week};

const DAY: u64 = 24 * 60 * 60;
const HOUR: u64 = 60 * 60;

pub fn on() -> bool {
    std::env::var("FAKE_RETRO").is_ok_and(|v| v == "1")
}

/// This week's changes, one click each: the change first, the last "not this week".
pub fn set(now: u64) -> QuestionSet {
    let option = |label: &str, description: &str| Choice { label: label.into(), description: description.into(), ..Choice::default() };
    let change = |question: &str, header: &str, first: (&str, &str)| Question {
        question: question.into(),
        header: header.into(),
        options: vec![option(first.0, first.1), option("교훈만 남기기", "바꾸지는 않고 배운 것만 기억해 둡니다"), option("이번 주는 안 함", "")],
        ..Question::default()
    };
    QuestionSet {
        id: 9006,
        session: 1,
        member: "pm".into(),
        ticket: None,
        questions: vec![
            change("교훈: 반나절 넘는 티켓은 멈췄어요. → 다음 주부터 티켓을 반나절 크기로 나눌까요?", "티켓 크기", ("반나절로 나누기 (추천)", "T-70처럼 큰 티켓은 둘로 쪼개서 올립니다")),
            change("교훈: 승인이 밤새 기다렸어요. → 승인 요청을 오전·오후 두 번 모아서 드릴까요?", "승인", ("하루 두 번 모아서 (추천)", "급한 것만 바로, 나머지는 10시·16시에")),
        ],
        asked_at: now.saturating_sub(600),
        review: false,
        retro: Some(Retro { lessons: vec!["반나절 넘는 티켓은 멈춘다 — 작게 나눈다".into(), "승인은 모아서 받는다".into()] }),
    }
}

fn ticket(id: u32, title: &str, status: TicketStatus, who: &str, project: &str) -> Ticket {
    Ticket {
        id,
        title: title.into(),
        body: String::new(),
        status,
        kind: TicketKind::Code,
        owner: Some(who.into()),
        assignee: None,
        project: Some(format!("/tmp/fake/{project}").into()),
        depends_on: vec![],
        branch: Some(format!("equipe/t-{id}")),
        worktree: None,
        merged: Some(status == TicketStatus::Done),
        off_main: None,
        design_qa: None,
        created_by: "pm".into(),
        note: None,
        asks: vec![],
        remaining: vec![],
        images: Vec::new(),
        updated_at: None,
        priority: Default::default(),
        rank: 0,
    }
}

/// The week up to `to`, gathered as the daemon gathers it.
fn facts(to: u64) -> String {
    let from = to - retro::WEEK;
    let at = |days: u64, hours: u64| from + days * DAY + hours * HOUR;
    let event = |id: u32, when: u64, by: &str, kind: &str, text: &str| (id, TicketEntry { at: when, by: by.into(), kind: kind.into(), text: text.into() });
    let tickets = vec![
        ticket(47, "상품 상세 이미지 갤러리", TicketStatus::Done, "frontend", "web"),
        ticket(52, "결제 실패 안내 문구", TicketStatus::Done, "frontend", "web"),
        ticket(70, "주문 API 페이지네이션", TicketStatus::InProgress, "backend", "api"),
    ];
    let events = vec![
        event(47, at(1, 3), "you", "accepted", ""),
        event(47, at(1, 4), "you", "merged", "main"),
        event(52, at(3, 2), "you", "accepted", ""),
        event(52, at(3, 2), "you", "merged", "main"),
        event(70, at(2, 1), "backend", retro::ESTIMATED, &at(2, 5).to_string()),
        event(70, at(2, 9), "backend", "handed in", ""),
        event(70, at(3, 1), "you", "reopened", "Reopened: 마지막 페이지에서 빈 목록이 나와요"),
    ];
    let turn = |member: &str, ticket: Option<u32>, day: u64, hours: u64, mins: u64, outcome: &str, tokens: u64| Turn { member: member.into(), role: member.into(), ticket, began: at(day, hours), ended: at(day, hours) + mins * 60, outcome: outcome.into(), tokens };
    let turns = vec![
        turn("frontend", Some(47), 0, 2, 25, "completed", 310_000),
        turn("frontend", Some(52), 2, 3, 18, "completed", 120_000),
        turn("backend", Some(70), 2, 1, 55, "completed", 840_000),
        turn("backend", Some(70), 2, 3, 6, "failed", 30_000),
        turn("pm", None, 0, 1, 12, "completed", 90_000),
        turn("pm", None, 4, 1, 9, "completed", 60_000),
    ];
    let waits = vec![Wait { kind: "permission".into(), member: "backend".into(), ticket: Some(70), what: "git push origin equipe/t-70".into(), asked: at(2, 10), answered: Some(at(3, 0)), outcome: "allowed".into() }];
    let gathered = retro::gather(&Week { from, to, tickets: &tickets, events: &events, turns: &turns, waits: &waits, briefs: vec![retro::Brief { project: "web".into(), file: "docs/briefs/checkout.md".into(), title: "결제 흐름 다듬기".into() }] });
    retro::facts_text(&gathered, &equipe_core::routines::Local)
}

/// The squad's routines: the weekly check-in and the retro, as a PM brings it.
pub fn routines() -> Vec<equipe_core::protocol::routines::Routine> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut check_in = equipe_core::heartbeat::weekly_check_in(equipe_core::i18n::Lang::Ko);
    let mut weekly = retro::routine(equipe_core::i18n::Lang::Ko, 2);
    for r in [&mut check_in, &mut weekly] {
        r.next_run = Some(equipe_core::routines::next_run(r.when, now, &equipe_core::routines::Local));
    }
    weekly.last_run = Some(now.saturating_sub(HOUR));
    vec![check_in, weekly]
}

/// Last week's run and its retro, then this week's: the PM's conversation.
pub fn history(now: u64) -> Vec<Entry> {
    let routine = retro::routine(equipe_core::i18n::Lang::Ko, 2);
    // This week's ran an hour ago; last week's, a week before.
    let this = now.saturating_sub(HOUR);
    let last = this - retro::WEEK;
    let earlier = "## 주간 회고\n\n이번 주 메인에 4건이 들어갔고, 막힌 건 하나였어요.\n\n### 1. 출시\n- **T-31** 회원가입 이메일 인증 — frontend\n- **T-33** 주문 내역 정렬 — backend".to_owned();
    let week = "## 주간 회고\n\n\
이번 주 메인에 2건이 들어갔고, 주문 API(T-70)가 사흘째 멈춰 있어요.\n\n\
### 1. 출시\n\
- **T-47** 상품 상세 이미지 갤러리 — frontend, web\n\
- **T-52** 결제 실패 안내 문구 — frontend, web\n\n\
### 2. 막힌 것과 이유\n\
- **T-70** 주문 API 페이지네이션 — 말한 시간보다 4시간 늦게 넘어왔고, 마지막 페이지의 빈 목록 때문에 되돌려 보내셨어요. 턴 하나는 실패했어요(테스트 DB 연결).\n\
- `git push` 승인을 밤새 14시간 기다렸어요.\n\n\
### 3. 멤버별 시간\n\
| 멤버 | 턴 | 시간 | 사용량 |\n|---|---|---|---|\n| backend | 2 | 1시간 1분 | 870k |\n| frontend | 2 | 43분 | 430k |\n| pm | 2 | 21분 | 150k |\n\n\
마찰은 backend에 몰렸어요: 실패 1, 되돌림 1, 지연 1, 승인 대기 1.\n\n\
**브리프와 비교** — `docs/briefs/checkout.md`는 결제 흐름 세 가지를 기대했는데 이번 주에는 실패 문구 하나만 나갔어요.\n\n\
### 교훈과 바꿀 것\n\
1. 반나절 넘는 티켓은 멈춘다 → 티켓을 반나절 크기로\n\
2. 승인은 모아서 받는다 → 하루 두 번 모아서\n\n\
아래 카드에서 하나씩 골라 주세요. 고르시기 전에는 아무것도 바꾸지 않을게요."
        .to_owned();
    vec![
        Entry::User { text: retro::message(&routine, &facts(last)) },
        Entry::Reply { text: earlier },
        Entry::User { text: "좋아요, 다음 주도 부탁해요".into() },
        Entry::Reply { text: "네, 금요일에 다시 가져올게요.".into() },
        Entry::User { text: retro::message(&routine, &facts(this)) },
        Entry::Reply { text: week },
    ]
}
