//! Settings' engine switching, a section of its own — so the AI platforms'
//! cards (each engine signed in) can sit beside it: how the squad moves
//! members when a plan runs low (off · suggest · auto), each plan as last read,
//! and the recommended setup for both engines, again — and, beside it, which
//! engine reviews the designer's work (auto · the member's own · the other).
//! A child of `views/settings.rs`, drawn with its rows and controls.

use equipe_core::protocol::engines::{Reviewer, Switching};
use equipe_core::t;
use gpui_kit::*;

use super::{Line, Section, segmented, value};
use crate::Shell;
use crate::switching as rules;
use crate::state::Conn;
use crate::ui::{self, Kind};

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// "Engine switching": the squad's choice, both plans, the setup again.
pub(super) fn sections(shell: &Shell, cx: &mut Context<Shell>) -> Vec<Section> {
    let p = shell.palette;
    let model = &shell.model;
    let connected = matches!(model.conn, Conn::Connected { .. });
    let mode = model.engines.mode;
    let options: Vec<(Switching, String)> = Switching::ALL.iter().map(|m| (*m, rules::mode_word(*m).to_owned())).collect();
    let choice = Line::new(t!("engines.settings_mode"), rules::mode_about(mode))
        .words("off suggest auto engines Claude Codex")
        .control(if connected { segmented(shell, "switching", options, mode, |shell, mode, _, cx| shell.set_switching(mode, cx), cx) } else { value(p, rules::mode_word(mode)) });
    let now = now();
    let plans = model.engines.meters(now).into_iter().map(|m| {
        let read = rules::read_words(&m, now);
        let windows: Vec<String> = m.gauges.iter().map(|g| format!("{} {}%", rules::window_word(Some(g.window)), g.percent)).collect();
        let shown = if windows.is_empty() { t!("engines.not_yet").to_owned() } else { windows.join(" · ") };
        Line::new(rules::engine_name(m.engine), read).control(value(p, shown))
    });
    let handle = cx.entity().downgrade();
    let setup = Line::new(t!("engines.settings_setup"), t!("engines.settings_setup_desc")).words("setup recommended").control(
        ui::button(p, "engines-setup-again", Kind::Secondary, t!("engines.settings_setup_button")).disabled(!connected).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.show_engine_setup(cx)).ok();
        }),
    );
    let lines = std::iter::once(choice).chain(plans).chain(std::iter::once(setup)).collect();
    vec![Section::new(t!("engines.settings_section"), lines).foot(t!("engines.settings_foot")), reviewer_section(shell, cx)]
}

/// The design reviewer's choice in the person's words: its name, and what it does.
pub(crate) fn reviewer_words(reviewer: Reviewer) -> (&'static str, &'static str) {
    match reviewer {
        Reviewer::Auto => (t!("engines.reviewer_auto"), t!("engines.reviewer_auto_about")),
        Reviewer::Same => (t!("engines.reviewer_same"), t!("engines.reviewer_same_about")),
        Reviewer::Other => (t!("engines.reviewer_other"), t!("engines.reviewer_other_about")),
    }
}

/// "Design reviews": which engine reviews the designer's work.
fn reviewer_section(shell: &Shell, cx: &mut Context<Shell>) -> Section {
    let p = shell.palette;
    let connected = matches!(shell.model.conn, Conn::Connected { .. });
    let now = shell.model.design_reviewer;
    let options: Vec<(Reviewer, String)> = Reviewer::ALL.iter().map(|r| (*r, reviewer_words(*r).0.to_owned())).collect();
    let line = Line::new(t!("engines.reviewer"), reviewer_words(now).1)
        .words("design reviewer critique second look review engine auto same other Claude Codex")
        .control(if connected { segmented(shell, "design-reviewer", options, now, |shell, reviewer, _, cx| shell.set_design_reviewer(reviewer, cx), cx) } else { value(p, reviewer_words(now).0) });
    Section::new(t!("engines.reviewer_section"), vec![line]).foot(t!("engines.reviewer_foot"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_design_reviewer_choice_has_its_name_and_says_what_it_does() {
        let words: Vec<(&str, &str)> = Reviewer::ALL.iter().map(|r| reviewer_words(*r)).collect();
        // The tests' screens speak Korean.
        assert_eq!(words, [
            ("자동", "다른 엔진이 로그인돼 있고 한도에 여유가 있으면 그 엔진이, 아니면 디자이너 자신의 엔진이 새 세션으로 검토해요"),
            ("멤버와 같은 엔진", "디자이너 자신의 엔진이 작업 과정을 보지 않은 새 세션으로 검토해요"),
            ("다른 엔진", "한도와 상관없이 늘 다른 엔진이 검토해요 — 다른 엔진을 쓸 수 없을 때만 디자이너 자신의 엔진이 해요"),
        ]);
        assert_eq!(Reviewer::ALL[0], Reviewer::default(), "auto first, the default");
    }
}
