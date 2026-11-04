//! `FAKE_CHECKS=1`: the designer's conversation shows its method — a new
//! screen drawn and checked in code (`equipe design lint --render`), the floor
//! failing and fixed, a habit kept with its reason, then fresh eyes on it
//! (`equipe design second-look`): its first screenful alone, then the whole,
//! a fix list answered, and the hand-in. Every card is read from the lines
//! the real commands end with, made here by the same code.
//! `FAKE_CHECKS=fail`: the conversation stops at a check that fails the floor.

use equipe_core::design::lint::record::{Fix, Verdict};
use equipe_core::design::lint::{self, Summary};
use equipe_core::design::second_look;
use equipe_core::protocol::{Entry, ToolOutcome};

pub fn on() -> bool {
    std::env::var("FAKE_CHECKS").is_ok_and(|v| !v.is_empty() && v != "0")
}

fn failing_only() -> bool {
    std::env::var("FAKE_CHECKS").is_ok_and(|v| v == "fail")
}

const SCREEN: &str = "settings/notifications";
const WIDTHS: [u32; 7] = [360, 390, 768, 1024, 1280, 1440, 1920];

fn pair(id: &str, said: &str) -> (String, String) {
    (id.to_owned(), said.to_owned())
}

fn first_check() -> Summary {
    Summary {
        floor: (9, 11),
        habits: 1,
        finish: 1,
        widths: WIDTHS.to_vec(),
        failing: vec![pair("contrast", "p.hint “매일 오전 9시에 보내요” is 3.1:1 (#8A8C93 on #F6F5F1), under 4.5:1 (at 360, 390, 768, 1024, 1280, 1440, 1920 px)"), pair("token-space", "padding: 13px is off the spacing scale (4, 8, 12, 16, 24, 32)")],
        open: vec![pair("eyebrow", "2 of 2 heading(s) have a small label over them: “EMAIL”, “PUSH”")],
        notes: vec![pair("touch-target", "input#digest is 20×20")],
        screen: Some(SCREEN.into()),
        ..Summary::default()
    }
}

fn second_check() -> Summary {
    Summary {
        floor: (11, 11),
        habits: 0,
        waived: vec![pair("tracked-caps", "brand.md의 고정값: 채널 이름은 대문자 라벨로 둔다")],
        finish: 2,
        widths: WIDTHS.to_vec(),
        notes: vec![pair("touch-target", "input#digest is 20×20"), pair("literal-on-scale", "gap: 16px is `space.4`")],
        screen: Some(SCREEN.into()),
        ..Summary::default()
    }
}

fn lint_run(summary: &Summary, report: &str) -> Entry {
    let output = format!("Drawing design/mockups/notifications.html at 360,390,768,1024,1280,1440,1920…\n{report}\n{}", lint::mark_line(summary));
    let passed = summary.passes();
    Entry::Tool {
        tool: "Bash".into(),
        target: Some(format!("equipe design lint design/mockups/notifications.html --render --screen {SCREEN}")),
        outcome: Some(Box::new(ToolOutcome { ok: passed, exit: Some(if passed { 0 } else { 1 }), ms: Some(14_200), output, skipped: 0, diff: None })),
        label: Some("화면 점검 (7개 폭)".into()),
        background: false,
    }
}

fn look_run(first: bool, verdict: &Verdict) -> Entry {
    let output = format!("A fresh Claude Code is looking at 7 picture(s) of {SCREEN}…\n{}{}", second_look::text(verdict), second_look::mark_line(SCREEN, verdict));
    Entry::Tool {
        tool: "Bash".into(),
        target: Some(format!("equipe design second-look {SCREEN}{} --criteria \"다이제스트를 한 번에 끈다\"", if first { " --first" } else { "" })),
        outcome: Some(Box::new(ToolOutcome { ok: verdict.pass, exit: Some(if verdict.pass { 0 } else { 1 }), ms: Some(48_000), output, skipped: 0, diff: None })),
        label: Some(if first { "첫 화면만 새 시선으로" } else { "새 시선 검토" }.into()),
        background: false,
    }
}

fn edit(file: &str) -> Entry {
    Entry::Tool { tool: "Edit".into(), target: Some(format!("/tmp/fake/app/design/mockups/{file}")), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(20), ..ToolOutcome::default() })), label: None, background: false }
}

pub fn history() -> Vec<Entry> {
    let read = |f: &str| Entry::Tool { tool: "Read".into(), target: Some(format!("/tmp/fake/app/design/{f}")), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(30), ..ToolOutcome::default() })), label: None, background: false };
    let mut out = vec![
        Entry::Note { text: "── T-61 · 알림 설정 화면 시안 — on branch equipe/t-61".into() },
        Entry::User { text: "알림 설정 화면 시안 만들어 줘. 다이제스트를 한 번에 끌 수 있어야 해.".into() },
        Entry::Reply { text: "브리프와 디자인 시스템부터 읽고, 첫 화면을 먼저 만들어 점검할게요.".into() },
        read("brand.md"),
        read("guidelines.md"),
        read("tokens.json"),
        Entry::Tool { tool: "Write".into(), target: Some("/tmp/fake/app/design/mockups/notifications.html".into()), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(40), ..ToolOutcome::default() })), label: None, background: false },
        lint_run(&first_check(), "Floor — never waived; fix before handing in:\n  ✗ contrast …\n  ✗ token-space …"),
    ];
    if failing_only() {
        return out;
    }
    let first_look = Verdict { pass: true, summary: "첫 화면이 다이제스트 스위치로 시작해서 할 일이 바로 보여요. 이 위에 나머지를 쌓아도 됩니다.".into(), by: "claude".into(), widths: WIDTHS.to_vec(), first: true, ..Verdict::default() };
    let fixes = Verdict {
        pass: false,
        summary: "한 번에 끄는 흐름은 맞지만, 좁은 폭에서 저장 상태가 보이지 않아요.".into(),
        fixes: vec![
            Fix { place: "다이제스트 줄, 360·390".into(), what: "스위치 옆 “저장됨” 표시가 줄바꿈으로 밀려 안 보여요 — 스위치 아래로 내리세요".into(), why: "껐는지 확신할 수 없어요".into() },
            Fix { place: "푸시 카드".into(), what: "꺼진 채널의 설명 글자를 muted로 맞추세요".into(), why: String::new() },
        ],
        by: "claude".into(),
        widths: WIDTHS.to_vec(),
        ..Verdict::default()
    };
    let passed = Verdict { pass: true, summary: "다이제스트를 한 번에 끄고, 바뀐 상태가 모든 폭에서 보여요.".into(), by: "claude".into(), widths: WIDTHS.to_vec(), ..Verdict::default() };
    out.extend([
        Entry::Reply { text: "기준선 두 개가 걸렸어요 — 보조 문구 대비와 간격 하나. 고치고, 제목 위 라벨은 brand.md에 고정된 대문자 라벨이라 이유를 남겨 유지할게요.".into() },
        edit("notifications.html"),
        Entry::Tool {
            tool: "Bash".into(),
            target: Some("equipe design lint --waive tracked-caps --why \"brand.md의 고정값: 채널 이름은 대문자 라벨로 둔다\" --ticket T-61".into()),
            outcome: Some(Box::new(ToolOutcome { ok: true, exit: Some(0), ms: Some(60), output: "wrote design/decisions/2026-11-05-keep-tracked-caps.md — `tracked-caps` is kept with its reason.".into(), skipped: 0, diff: None })),
            label: Some("습관 유지 이유 기록".into()),
            background: false,
        },
        lint_run(&second_check(), "Finish — worth doing, never blocking:\n  · touch-target …"),
        look_run(true, &first_look),
        look_run(false, &fixes),
        Entry::Reply { text: "새 시선이 두 가지를 짚었어요. 좁은 폭의 저장 표시를 스위치 아래로 내리고, 꺼진 채널 설명 색을 맞출게요.".into() },
        edit("notifications.html"),
        lint_run(&second_check(), "Finish — worth doing, never blocking:\n  · touch-target …"),
        look_run(false, &passed),
        Entry::Tool { tool: "mcp__equipe__submit_for_review".into(), target: Some("알림 설정 화면 시안".into()), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(80), ..ToolOutcome::default() })), label: None, background: false },
        Entry::Reply { text: "시안을 넘겼어요. 기준선 11/11, 대문자 라벨은 brand.md 고정값이라 유지했고, 새 시선이 짚은 두 가지는 고쳐서 통과했어요.".into() },
    ]);
    out
}
