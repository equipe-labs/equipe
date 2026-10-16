//! Settings' engine switching, a section of its own — so the AI platforms'
//! cards (each engine signed in) can sit beside it: how the squad moves
//! members when a plan runs low (끄기 · 제안 · 자동), each plan as last read,
//! and the recommended setup for both engines, again. A child of
//! `views/settings.rs`, drawn with its rows and controls.

use equipe_core::protocol::engines::Switching;
use equipe_core::t;
use gpui_kit::*;

use super::{Line, Section, segmented, value};
use crate::Shell;
use crate::engines as rules;
use crate::state::Conn;
use crate::ui::{self, Kind};

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// "엔진 자동 전환": the squad's choice, both plans, the setup again.
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
    vec![Section::new(t!("engines.settings_section"), lines).foot(t!("engines.settings_foot"))]
}
