//! `FAKE_DESIGN_REVIEW=1`: the designer's weekly design review, come in —
//! its routine's message and the review it brought in the designer's
//! conversation, and its proposal waiting in 나에게 as a visual question
//! (the pictures `fake_daemon/pictures.rs` draws).

use equipe_core::protocol::decide::{Choice, Question, QuestionSet};
use equipe_core::protocol::{Entry, ToolOutcome};

use super::pictures::at;

pub fn on() -> bool {
    std::env::var("FAKE_DESIGN_REVIEW").is_ok_and(|v| v == "1")
}

/// The review's proposal: the sign-in screen's two buttons, put as one
/// click — what is there now over the options, each its picture.
pub fn set(now: u64) -> QuestionSet {
    let option = |label: &str, description: &str, image: &str| Choice { label: label.into(), description: description.into(), images: vec![at(image)], ..Choice::default() };
    QuestionSet {
        id: 9005,
        session: 4,
        member: "designer".into(),
        ticket: None,
        questions: vec![Question {
            question: "로그인 화면의 버튼 위계를 이렇게 다듬을까요?".into(),
            header: "버튼 위계".into(),
            images: vec![at("case-now.png")],
            options: vec![
                option("주 버튼 하나로 (추천)", "Google 로그인은 보조 버튼으로 — 첫눈에 할 일이 하나", "case-wide.png"),
                option("지금처럼 둘 다 주 버튼", "손대지 않습니다", "case-square.png"),
                option("더 과감하게: 카드 전체가 버튼", "추천보다 한 발 더 — 터치 영역이 화면 절반", "case-circle-square.png"),
            ],
            ..Question::default()
        }],
        asked_at: now.saturating_sub(120),
        review: true,
        retro: None,
    }
}

/// The designer's conversation: the routine's message, the checks it ran,
/// and the review — its heading, the three things, the critic's word.
pub fn history() -> Vec<Entry> {
    let routine = equipe_core::design::review::routine(equipe_core::i18n::Lang::Ko, 2);
    let ran = |target: &str| Entry::Tool { tool: "Bash".into(), target: Some(target.into()), outcome: Some(Box::new(ToolOutcome { ok: true, ..ToolOutcome::default() })), label: None, background: false };
    let review = format!(
        "## 주간 디자인 리뷰\n\n\
### 1. 어긋난 것 — 3가지\n\
1. **로그인 › 오류** — 다크 모드 그림이 없습니다. 라이트만 있어요.\n\
2. **주문 › 목록 › 빈 상태** — 지난주 코드가 바뀐 뒤 그림이 오래됐습니다.\n\
3. **설정 › 알림** — 토글 색이 `color.action.primary`가 아닌 하드코딩 값(#3B82F6)입니다.\n\n\
![로그인 › 오류 (라이트만)]({})\n\n\
### 2. 다듬을 제안 — 로그인 버튼 위계\n\
지금은 이메일과 Google 버튼이 같은 무게라 첫눈에 할 일이 둘입니다. 주 버튼 하나로 정리하는 안을 실제 화면 위에 그렸어요. 아래 카드에서 골라 주세요.\n\n\
> 비평(Codex): 추천안은 위계가 분명하지만 보조 버튼의 대비가 3.9:1로 AA에 조금 못 미칩니다 — 테두리를 `color.border.strong`으로 올리면 됩니다.\n\n\
### 3. 볼 만한 레퍼런스\n\
- [Linear 로그인](https://linear.app/login) — 주 버튼 하나, 나머지는 텍스트 링크로 물러나 있는 방식\n\
- [Stripe 대시보드 빈 상태](https://dashboard.stripe.com) — 빈 상태에서 다음 할 일 하나만 보여 주는 문장\n\
- [Toss 설정](https://toss.im) — 토글과 설명 줄의 간격, 한국어 줄바꿈\n\n\
고르실 때까지 아무것도 바꾸지 않을게요.",
        at("case-now.png")
    );
    vec![
        Entry::User { text: equipe_core::routines::message(&routine) },
        ran("equipe design flows check"),
        ran("equipe design critique design/review/sign-in-a.png --reference design/references/linear/login.png"),
        Entry::Reply { text: review },
    ]
}
