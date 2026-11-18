//! Settings' autonomy, under Permissions: how far the squad's members go
//! without the person, kind by kind (ask · decide and tell · just do), how
//! long a decide-and-tell choice waits, what always asks whatever the policy
//! (the floor), the standing allowances the person granted from request cards
//! with a way to take each back, and the choices members took on their own
//! with a way to open each again. A child of `views/settings.rs`.

use equipe_core::protocol::Command;
use equipe_core::protocol::autonomy::{Kind, Level, State, WINDOWS};
use equipe_core::t;
use gpui_kit::component::h_flex;
use gpui_kit::*;

use super::{Line, Section, segmented, value};
use crate::Shell;
use crate::state::Conn;
use crate::ui::{self, Kind as ButtonKind};

/// A kind's name, as the person reads it.
pub fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Design => t!("autonomy.kind_design"),
        Kind::Merge => t!("autonomy.kind_merge"),
        Kind::Internal => t!("autonomy.kind_internal"),
        Kind::ProductionData => t!("autonomy.kind_production_data"),
        Kind::SignIn => t!("autonomy.kind_sign_in"),
        Kind::Money => t!("autonomy.kind_money"),
        Kind::Deletion => t!("autonomy.kind_deletion"),
        Kind::Outside => t!("autonomy.kind_outside"),
        Kind::Security => t!("autonomy.kind_security"),
        Kind::Legal => t!("autonomy.kind_legal"),
        Kind::Other => t!("autonomy.kind_other_requests"),
    }
}

/// What a kind covers, in a line.
fn kind_desc(kind: Kind) -> &'static str {
    match kind {
        Kind::Design => t!("autonomy.desc_design"),
        Kind::Merge => t!("autonomy.desc_merge"),
        Kind::Internal => t!("autonomy.desc_internal"),
        _ => t!("autonomy.desc_production_data"),
    }
}

pub fn level_label(level: Level) -> &'static str {
    match level {
        Level::Ask => t!("autonomy.level_ask"),
        Level::Tell => t!("autonomy.level_tell"),
        Level::JustDo => t!("autonomy.level_just_do"),
    }
}

/// A window, in a word: "At once", "30 min", "2 h".
pub fn window_label(minutes: u32) -> String {
    match minutes {
        0 => t!("autonomy.window_now").to_owned(),
        m if m % 60 == 0 => t!("autonomy.window_hours", hours = m / 60),
        m => t!("autonomy.window_minutes", minutes = m),
    }
}

/// How an allowance stands now, in a few words.
pub fn allowance_state(a: &equipe_core::protocol::autonomy::Allowance, now: u64) -> String {
    match a.revoked_at {
        Some(_) => t!("autonomy.allowance_revoked").to_owned(),
        None if a.expires_at <= now => t!("autonomy.allowance_expired").to_owned(),
        None => t!("autonomy.allowance_left", days = a.expires_at.saturating_sub(now).div_ceil(86_400)),
    }
}

/// The sections: the levels, the floor, the allowances, the picks.
pub(super) fn sections(shell: &Shell, cx: &mut Context<Shell>) -> Vec<Section> {
    let p = shell.palette;
    let connected = matches!(shell.model.conn, Conn::Connected { .. });
    let Some(state) = shell.model.autonomy.clone() else {
        return vec![Section::new(t!("autonomy.section"), vec![Line::new(t!("autonomy.unknown"), t!("autonomy.unknown_desc"))])];
    };
    let State { policy, allowances, picks, .. } = state;
    let mut levels: Vec<Line> = Vec::new();
    for kind in Kind::SET {
        let now = policy.level(kind);
        let options = Level::ALL.into_iter().map(|l| (l, level_label(l).to_owned())).collect();
        let control = if connected {
            segmented(shell, &format!("autonomy-{}", kind.code()), options, now, move |this, level, _, _| {
                let squad = this.model.squad.clone();
                this.send_command(Command::SetAutonomy { squad, kind, level });
            }, cx)
        } else {
            value(p, level_label(now))
        };
        levels.push(Line::new(kind_label(kind), kind_desc(kind)).words(format!("autonomy policy {} ask decide tell just do", kind.code())).wrap_control().control(control));
    }
    let windows = WINDOWS.into_iter().map(|m| (m, window_label(m))).collect();
    let window_now = if WINDOWS.contains(&policy.window_minutes) { policy.window_minutes } else { WINDOWS[1] };
    levels.push(
        Line::new(t!("autonomy.window"), t!("autonomy.window_desc"))
            .words("autonomy window wait minutes decide and tell")
            .wrap_control()
            .control(if connected {
                segmented(shell, "autonomy-window", windows, window_now, |this, minutes, _, _| {
                    let squad = this.model.squad.clone();
                    this.send_command(Command::SetAutonomyWindow { squad, minutes });
                }, cx)
            } else {
                value(p, window_label(policy.window_minutes))
            }),
    );
    let floor = Line::new(t!("autonomy.floor"), Kind::FLOOR.iter().map(|k| kind_label(*k)).collect::<Vec<_>>().join(" · ")).words("autonomy floor always ask sign in money delete send publish security legal");

    let now = crate::state::now_unix();
    let mut kept: Vec<Line> = allowances
        .iter()
        .map(|a| {
            let id = a.id;
            let desc = format!("{} · {}", allowance_state(a, now), t!("autonomy.allowance_used", uses = a.uses));
            let line = Line::new(a.scope.words.clone(), desc).words("standing allowance allow for this kind revoke");
            if a.live(now) {
                line.control(ui::button(p, ("revoke-allowance", id), ButtonKind::Secondary, t!("autonomy.revoke")).disabled(!connected).on_click(cx.listener(move |this, _, _, cx| {
                    let squad = this.model.squad.clone();
                    this.send_command(Command::RevokeAllowance { squad, id });
                    cx.notify();
                })))
            } else {
                line
            }
        })
        .collect();
    if kept.is_empty() {
        kept.push(Line::new(t!("autonomy.no_allowances"), t!("autonomy.no_allowances_desc")).words("standing allowance allow for this kind"));
    }
    let mut taken: Vec<Line> = picks
        .iter()
        .take(10)
        .map(|pick| {
            let set = pick.set;
            let mut desc = t!("autonomy.picked", picked = pick.picked.clone());
            if pick.reopened {
                desc = format!("{desc} · {}", t!("autonomy.reopened"));
            }
            let line = Line::new(format!("{} · {}", pick.member, pick.question), desc).words("decide and tell picked change");
            if pick.reopened {
                line
            } else {
                line.control(ui::button(p, ("reopen-pick", set), ButtonKind::Secondary, t!("autonomy.change")).disabled(!connected).on_click(cx.listener(move |this, _, _, cx| this.reopen_pick(set, cx))))
            }
        })
        .collect();
    let mut out = vec![
        Section::new(t!("autonomy.section"), levels).foot(t!("autonomy.foot")),
        Section::new(t!("autonomy.floor_section"), vec![floor]).foot(t!("autonomy.floor_foot")),
        Section::new(t!("autonomy.allowances"), kept).foot(t!("autonomy.allowances_foot")),
    ];
    if !taken.is_empty() {
        taken.truncate(10);
        out.push(Section::new(t!("autonomy.picks"), taken));
    }
    out
}

impl Shell {
    /// Open a choice a member took on its own again, for the person to choose.
    pub fn reopen_pick(&mut self, set: u64, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        self.model.picks_seen.insert(set);
        self.send_command(Command::ReopenAutoPick { squad, set });
        cx.notify();
    }
}

/// The picks a dock card tells of: the last day's, not yet looked at here, not opened again.
pub fn unseen_picks(model: &crate::state::Model, now: u64) -> Vec<equipe_core::protocol::autonomy::PickView> {
    model
        .autonomy
        .as_ref()
        .map(|s| s.picks.iter().filter(|p| !p.reopened && !model.picks_seen.contains(&p.set) && now.saturating_sub(p.at) < 86_400).cloned().collect())
        .unwrap_or_default()
}

/// The card over the composer that tells of choices members took on their
/// own: one line each — who, what they picked and why — with "Change" (the
/// question comes back to choose) and "OK".
pub fn docked(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let picks = unseen_picks(&shell.model, crate::state::now_unix());
    if picks.is_empty() {
        return None;
    }
    let rows: Vec<AnyElement> = picks
        .iter()
        .take(3)
        .map(|pick| {
            let set = pick.set;
            let why = if pick.why.is_empty() { String::new() } else { format!(" ({})", pick.why) };
            h_flex()
                .gap_2()
                .items_center()
                .child(div().flex_1().min_w_0().text_size(px(14.)).text_color(p.text).child(t!("autonomy.dock_line", name = pick.member.clone(), picked = pick.picked.clone(), why = why)))
                .child(ui::button(p, ("dock-pick-change", set), ButtonKind::Secondary, t!("autonomy.change")).on_click(cx.listener(move |this, _, _, cx| this.reopen_pick(set, cx))))
                .child(ui::button(p, ("dock-pick-ok", set), ButtonKind::Ghost, t!("autonomy.ok")).on_click(cx.listener(move |this, _, _, cx| {
                    this.model.picks_seen.insert(set);
                    cx.notify();
                })))
                .into_any_element()
        })
        .collect();
    Some(
        gpui_kit::component::v_flex()
            .w_full()
            .mb_3()
            .p_3()
            .gap_2()
            .rounded(px(crate::theme::R_LG))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .child(div().text_size(px(12.)).text_color(p.muted).child(t!("autonomy.dock_title", n = picks.len())))
            .children(rows)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{unseen_picks, window_label};
    use equipe_core::protocol::autonomy::{PickView, State};

    #[test]
    fn windows_read_as_words() {
        assert_eq!(window_label(0), equipe_core::t!("autonomy.window_now"));
        assert!(window_label(120).contains('2'));
        assert!(window_label(30).contains("30"));
    }

    #[test]
    fn the_dock_tells_of_the_last_days_picks_not_yet_seen_or_opened_again() {
        let pick = |set: u64, at: u64, reopened: bool| PickView { set, session: 1, member: "designer".into(), ticket: None, question: "Which?".into(), picked: "B".into(), why: String::new(), at, reopened };
        let mut model = crate::state::Model::new("acme");
        model.autonomy = Some(State { picks: vec![pick(1, 100_000, false), pick(2, 100_000, true), pick(3, 1_000, false), pick(4, 100_000, false)], ..State::default() });
        model.picks_seen.insert(4);
        let shown: Vec<u64> = unseen_picks(&model, 100_100).iter().map(|p| p.set).collect();
        assert_eq!(shown, vec![1], "opened again, a day old and looked at are off the dock");
    }
}
