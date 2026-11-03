//! Briefs and plans (`equipe_core::briefs`): written into the fake projects'
//! repositories the way the PM's `write_brief` writes them — committed onto
//! main — and sent when a project page asks. web has two briefs and a plan,
//! api a draft, app none (its page offers office hours). `FAKE_DOCS=none`:
//! no project has any. `FAKE_DECIDE=office`: the PM's office-hours card waits.

use equipe_core::briefs::{self, Kind, Target};
use equipe_core::protocol::decide::{Choice, Question, QuestionSet};
use equipe_core::protocol::{Command, Event};
use tokio::sync::broadcast;

fn none() -> bool {
    std::env::var("FAKE_DOCS").is_ok_and(|v| v == "none")
}

const PROJECTS: [&str; 3] = ["web", "app", "api"];

/// What the PM left, written as it writes them.
pub fn seed() {
    if none() {
        return;
    }
    let write = |project: &str, kind: Kind, slug: &str, date: &str, fields: &[(&str, String)], body: &str| {
        let target = Target::Repo(super::repo(project));
        if let Err(why) = briefs::save(&target, kind, slug, &briefs::render(fields, body), date) {
            eprintln!("fake daemon: {project}'s {slug} wasn't written: {why}");
        }
    };
    let f = |pairs: &[(&'static str, &str)]| pairs.iter().map(|(k, v)| (*k, (*v).to_owned())).collect::<Vec<_>>();
    write(
        "web",
        Kind::Brief,
        "one-tap-checkout",
        "2026-11-02",
        &f(&[("title", "한 번에 끝나는 결제"), ("date", "2026-11-02"), ("status", "settled"), ("ticket", "42"), ("mode", "startup")]),
        "## 목표\n\n장바구니에서 결제까지 한 화면으로 — 첫 구매 이탈을 줄입니다.\n\n## 누구를 위한 것인가\n\n모바일로 처음 사는 사람. 지금은 세 화면을 거치다 38%가 떠납니다(지난 30일 분석).\n\n## 수요\n\n- 고객센터 문의의 1/4이 \"결제가 어디서 끝나요?\"\n- 경쟁 두 곳이 이미 한 화면 결제\n\n## 고른 안\n\n**B. 한 화면 결제 + 저장된 수단** (추천) — 2주, 결제 API는 그대로.\n\n## 범위\n\n- 넣음: 카드·간편결제, 저장된 수단\n- 뺌: 해외 결제, 할부 선택\n\n## 끝났다는 건\n\n첫 구매 전환 +5%p, 4주 안에.\n",
    );
    write(
        "web",
        Kind::Plan,
        "one-tap-checkout",
        "2026-11-03",
        &f(&[("title", "한 번에 끝나는 결제 — 계획"), ("date", "2026-11-03"), ("status", "in_review"), ("brief", "docs/briefs/2026-11-02-one-tap-checkout.md"), ("scope", "selective"), ("tickets", "[42, 51]")]),
        "## 단계\n\n1. 결제 수단 선택을 한 화면으로 (T-42)\n2. 저장된 수단 불러오기\n3. 하단 요약 고정 (T-51)\n\n## 테스트\n\n- 카드 / 간편결제 각각 끝까지\n- 저장된 수단이 없을 때\n",
    );
    write(
        "web",
        Kind::Brief,
        "gift-cards",
        "2026-10-20",
        &f(&[("title", "선물 카드"), ("date", "2026-10-20"), ("status", "dropped"), ("mode", "startup")]),
        "## 왜 접었나\n\n원하는 사람을 한 명도 찾지 못했어요. 지금 쓰는 방법(계좌이체)으로 충분하다는 답이 대부분이었어요.\n",
    );
    write(
        "api",
        Kind::Brief,
        "rate-limits",
        "2026-11-04",
        &f(&[("title", "요청 한도"), ("date", "2026-11-04"), ("status", "draft"), ("mode", "builder")]),
        "## 목표\n\n파트너 한 곳이 몰아 보내도 다른 곳이 느려지지 않게.\n\n## 아직 모르는 것\n\n- 파트너별 한도를 어디서 정할지\n",
    );
}

/// Every project's briefs and plans, as the daemon reads them.
fn docs() -> Vec<equipe_core::protocol::briefs::ProjectDoc> {
    PROJECTS.iter().flat_map(|p| briefs::list(&super::repo(p), Some(p))).collect()
}

/// `ProjectDocs`: answered at once.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    if !matches!(command, Command::ProjectDocs { .. }) {
        return false;
    }
    let _ = events.send(Event::ProjectDocs { squad: "demo".into(), docs: docs() });
    true
}

/// Whether the PM's office-hours card waits (`FAKE_DECIDE=office`).
pub fn office() -> bool {
    std::env::var("FAKE_DECIDE").is_ok_and(|v| v == "office")
}

/// The second forcing question, pushed back on: the person's last answer
/// was vague, so the card says so and asks for what shows it.
pub fn office_hours_set(now: u64) -> QuestionSet {
    let choice = |label: &str, description: &str| Choice { label: label.into(), description: description.into(), ..Choice::default() };
    QuestionSet {
        id: 9101,
        session: 1,
        member: "pm".into(),
        ticket: Some(57),
        questions: vec![Question {
            question: "\"다들 불편해한다\"고 하셨는데, 누가 지금 무엇으로 버티고 있나요? 가장 가까운 것을 골라 주세요 — 숫자나 실제 예가 있으면 덧붙여 주세요.".into(),
            header: "현재 방식 2/6".into(),
            options: vec![
                choice("엑셀로 정리해 메일로 보냄 (추천)", "고객센터 문의 기록에 매주 3~4건 — 가장 많이 보인 방식이에요"),
                choice("카카오톡 단체방에서 직접 조율", "작은 매장 두 곳이 그렇다고 했어요 — 몇 곳인지는 아직 모릅니다"),
                choice("그냥 포기하고 전화로 처리", "증거가 없어요: 이걸 고르면 다음 질문에서 확인해요"),
            ],
            multi_select: false,
            ..Question::default()
        }],
        asked_at: now.saturating_sub(40),
        review: false,
    }
}
