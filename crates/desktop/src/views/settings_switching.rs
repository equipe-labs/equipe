//! Settings' engine switching, a section of its own — so the AI platforms'
//! cards (each engine signed in) can sit beside it: how the squad moves
//! members when a plan runs low (off · suggest · auto), each plan as last read,
//! and the recommended setup for both engines, again — and, beside it, which
//! engine reviews the designer's work (auto · the member's own · the other),
//! how many tickets the squad works at once (auto · max, with max's caps),
//! and whether equipe picks each ticket's model (quality first · off).
//! A child of `views/settings.rs`, drawn with its rows and controls.

use equipe_core::protocol::engines::{Reviewer, Switching};
use equipe_core::protocol::lanes::{MOST_PER_ENGINE, MOST_PER_MEMBER, Parallel, ParallelMode};
use equipe_core::protocol::tiers::ModelChoice;
use equipe_core::t;
use gpui_kit::*;

use super::{Line, Pop, Section, picker, segmented, value};
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
    vec![Section::new(t!("engines.settings_section"), lines).foot(t!("engines.settings_foot")), parallel_section(shell, cx), model_choice_section(shell, cx), reviewer_section(shell, cx)]
}

/// The caps "Per member" offers: every one, as each makes a difference.
pub(super) fn per_member_choices() -> Vec<u32> {
    (1..=MOST_PER_MEMBER).collect()
}

/// The caps "Per engine" offers: one at a time near the bottom, wider above.
pub(super) const PER_ENGINE_CHOICES: [u32; 12] = [1, 2, 4, 6, 8, 10, 12, 16, 20, 24, 28, MOST_PER_ENGINE];

/// The rows of "Parallel work": the choice, and on max its two caps.
pub(crate) fn parallel_rows(parallel: &Parallel) -> Vec<(&'static str, String)> {
    let about = if parallel.is_max() { t!("parallel.max_about") } else { t!("parallel.auto_about") };
    let mut rows = vec![(t!("parallel.row"), about.to_owned())];
    if parallel.is_max() {
        rows.push((t!("parallel.per_member"), t!("parallel.per_member_desc").to_owned()));
        rows.push((t!("parallel.per_engine"), t!("parallel.per_engine_desc").to_owned()));
    }
    rows
}

/// "Parallel work": auto by the plans' usage, or max at the person's caps.
fn parallel_section(shell: &Shell, cx: &mut Context<Shell>) -> Section {
    let p = shell.palette;
    let connected = matches!(shell.model.conn, Conn::Connected { .. });
    let now = shell.model.parallel;
    let options: Vec<(ParallelMode, String)> = ParallelMode::ALL.iter().map(|m| (*m, rules::parallel_word(*m).to_owned())).collect();
    let mut rows = parallel_rows(&now).into_iter();
    let mut lines = Vec::new();
    if let Some((label, desc)) = rows.next() {
        lines.push(Line::new(label, desc).words("parallel max booster lanes tickets at once plan usage auto").warn(now.is_max()).control(if connected {
            segmented(shell, "parallel", options, now.mode, |shell, mode, _, cx| shell.set_parallel(Parallel { mode, ..shell.model.parallel }, cx), cx)
        } else {
            value(p, rules::parallel_word(now.mode))
        }));
    }
    for ((label, desc), (pop, n)) in rows.zip([(Pop::PerMember, now.per_member), (Pop::PerEngine, now.per_engine)]) {
        lines.push(Line::new(label, desc).words("parallel max lanes per member per engine").control(if connected { picker(shell, pop, label, n.to_string(), 72., cx) } else { value(p, n.to_string()) }));
    }
    // A card of its own under engine switching, so its footnote reads as its alone.
    Section::new("", lines).foot(t!("parallel.foot"))
}

/// "Model choice": quality first — a ticket's model raised where its work
/// needs it, never below its member's own — or off, members' own models.
fn model_choice_section(shell: &Shell, cx: &mut Context<Shell>) -> Section {
    let p = shell.palette;
    let connected = matches!(shell.model.conn, Conn::Connected { .. });
    let now = shell.model.model_choice;
    let options: Vec<(ModelChoice, String)> = ModelChoice::ALL.iter().map(|c| (*c, rules::model_choice_word(*c).to_owned())).collect();
    let line = Line::new(t!("model_choice.row"), rules::model_choice_about(now)).words("model choice tier quality first off raise escalate opus sonnet effort").control(if connected {
        segmented(shell, "model-choice", options, now, |shell, choice, _, cx| shell.set_model_choice(choice, cx), cx)
    } else {
        value(p, rules::model_choice_word(now))
    });
    Section::new("", vec![line]).foot(t!("model_choice.foot"))
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
    // Not `super::*`: gpui_kit's glob brings its own `test` attribute, which `#[test]` would then mean.
    use super::{ModelChoice, PER_ENGINE_CHOICES, Parallel, Reviewer, parallel_rows, per_member_choices, reviewer_words};
    use equipe_core::protocol::lanes::{MOST_PER_ENGINE, MOST_PER_MEMBER};

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

    #[test]
    fn parallel_work_is_one_row_on_auto_and_shows_max_s_caps_on_max() {
        // The tests' screens speak Korean.
        assert_eq!(parallel_rows(&Parallel::default()), [("병렬 작업", "플랜 사용량이 늘수록 한 번에 진행하는 티켓을 줄여요".to_owned())]);
        let max = parallel_rows(&Parallel::max(6, 16));
        assert_eq!(max.iter().map(|(label, _)| *label).collect::<Vec<_>>(), ["병렬 작업", "멤버당", "엔진당"]);
        assert!(max[0].1.contains("플랜을 더 빨리"), "says it uses the plan faster: {}", max[0].1);
        // Every cap within bounds, the most offered.
        assert_eq!(per_member_choices().first().zip(per_member_choices().last()), Some((&1, &MOST_PER_MEMBER)));
        assert!(PER_ENGINE_CHOICES.windows(2).all(|w| w[0] < w[1]) && PER_ENGINE_CHOICES.last() == Some(&MOST_PER_ENGINE));
        assert!(PER_ENGINE_CHOICES.contains(&equipe_core::protocol::lanes::MAX_PER_ENGINE) && per_member_choices().contains(&equipe_core::protocol::lanes::MAX_PER_MEMBER), "the defaults offered");
    }

    #[test]
    fn model_choice_offers_quality_first_then_off_and_says_what_each_does() {
        use crate::switching::{model_choice_about, model_choice_word};
        // The tests' screens speak Korean.
        assert_eq!(ModelChoice::ALL.map(model_choice_word), ["품질 우선", "끔"]);
        assert_eq!(ModelChoice::ALL[0], ModelChoice::default(), "quality first, the default, first");
        assert!(model_choice_about(ModelChoice::QualityFirst).contains("낮추지는 않아요"));
        assert!(model_choice_about(ModelChoice::Off).contains("멤버 자신의 모델"));
    }
}
