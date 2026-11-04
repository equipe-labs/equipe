//! 디자인 셋업: how far the designer's first setup of the design system on
//! screen has come — 브리프, 로고, 디자인 시스템, 화면 지도, 디자인 QA — in a
//! band over the design screen while a step is left: each step ticked or
//! not, the one it is at and what that step makes, and a way to ask the
//! designer to go on with it (the PM to bring one in, when the team has
//! none) — or, while a ticket is already making it, "진행 중 · T-n", which
//! opens what is done so far. Once every step is done it is gone. The steps are the shown
//! system's (`DesignSystem::setup`, read from its folder): a project's own,
//! not the squad's, when they differ.

use equipe_core::protocol::design::{DesignSystem, Setup};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::state::{Member, Model, Presence};
use crate::theme::{self, ICON_SM, META, SMALL, SMALL_LINE};
use crate::tips::TipExt as _;
use crate::{Shell, ui};

/// The band's side margin: the design screen's.
const PAD: f32 = 32.;
/// The designer's avatar beside what the step makes.
const AVATAR: f32 = 20.;

/// The steps, in the order the designer takes them.
pub const STEPS: [Setup; 5] = [Setup::Brief, Setup::Logo, Setup::System, Setup::Flows, Setup::Qa];

/// Whether `step` is done in `system`.
pub fn done(system: &DesignSystem, step: Setup) -> bool {
    system.setup.iter().any(|s| s.step == step && s.done)
}

/// The step the setup is at: the first not done, whatever came after it.
/// `None`: every step done — or a daemon that doesn't say.
pub fn current(system: &DesignSystem) -> Option<Setup> {
    if system.setup.is_empty() {
        return None;
    }
    STEPS.into_iter().find(|s| !done(system, *s))
}

/// Whether the band shows: while a step is left.
pub fn shows(system: &DesignSystem) -> bool {
    current(system).is_some()
}

pub fn label(step: Setup) -> &'static str {
    match step {
        Setup::Brief => t!("design.setup_brief"),
        Setup::Logo => t!("design.setup_logo"),
        Setup::System => t!("design.setup_system"),
        Setup::Flows => t!("design.setup_flows"),
        Setup::Qa => t!("design.setup_qa"),
    }
}

/// What a step makes, in a line.
pub fn makes(step: Setup) -> &'static str {
    match step {
        Setup::Brief => t!("design.setup_brief_makes"),
        Setup::Logo => t!("design.setup_logo_makes"),
        Setup::System => t!("design.setup_system_makes"),
        Setup::Flows => t!("design.setup_flows_makes"),
        Setup::Qa => t!("design.setup_qa_makes"),
    }
}

/// The line under the steps: the one it is at, and what it makes.
pub fn now_line(step: Setup) -> String {
    t!("design.setup_now", step = label(step), makes = makes(step))
}

/// Which design it is, for a member to find: the project's or the squad's, and its folder.
fn whose(system: &DesignSystem) -> String {
    match &system.project {
        Some(name) => t!("design.setup_of_project", name = name, dir = system.dir),
        None => t!("design.setup_of_squad", dir = system.dir),
    }
}

/// The step's place while a ticket is making it: "진행 중 · T-378".
pub fn working_word(ticket: equipe_core::protocol::TicketId) -> String {
    t!("design.setup_working", id = ticket)
}

/// What the designer is asked, for the step the setup is at.
pub fn request(system: &DesignSystem) -> Option<String> {
    let design = whose(system);
    Some(match current(system)? {
        Setup::Brief => t!("design.setup_ask_brief", design = design),
        Setup::Logo => t!("design.setup_ask_logo", design = design),
        Setup::System => t!("design.setup_ask_system", design = design),
        Setup::Flows => t!("design.setup_ask_flows", design = design),
        Setup::Qa => t!("design.setup_ask_qa", design = design),
    })
}

/// What the PM is asked when the team has no designer: to propose one (a
/// change to the team, the person's to approve), then hand it the step.
pub fn pm_request(system: &DesignSystem) -> Option<String> {
    let step = current(system)?;
    let ask = request(system)?;
    Some(t!("design.setup_ask_pm_message", design = whose(system), step = label(step), request = ask))
}

/// The designer who can be asked: one on the team and not closed.
fn designer(model: &Model) -> Option<&Member> {
    model.members.iter().find(|m| m.role == "designer" && m.presence != Presence::Closed)
}

/// The band, over the design screen's sections: `None` once every step is
/// done, before the daemon has answered, or with no system to show.
/// The narrowest the step's line goes before its ask moves under it.
const STEP_LINE_MIN: f32 = 280.;

pub fn band(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let ui = &shell.design;
    // The work under way on show: the band is the accepted system's, not its.
    let system = ui.current().filter(|s| ui.loaded && !ui.working && shows(s))?;
    // Already being made: where it is, not a second ask.
    let making = ui.making(&shell.model);
    let at = current(system)?;
    let finished = STEPS.iter().filter(|s| done(system, **s)).count();
    let designer = designer(&shell.model);
    let mut steps = h_flex()
        .flex_wrap()
        .items_center()
        .gap_x_4()
        .gap_y_1()
        .child(
            h_flex()
                .flex_none()
                .gap_1p5()
                .items_baseline()
                .child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("design.setup_title")))
                .child(div().text_size(px(META)).text_color(p.muted).child(format!("{finished}/{}", STEPS.len()))),
        );
    for step in STEPS {
        let (ticked, on) = (done(system, step), step == at);
        // Done: a tick. The one it is at: a ring with a dot, in the words'
        // colour. The rest: a faint ring.
        let mark = if ticked {
            icon(Icon::Check, ICON_SM, p.accent).into_any_element()
        } else if on {
            div().size(px(ICON_SM)).flex_none().rounded_full().border(px(1.5)).border_color(p.text).flex().items_center().justify_center().child(div().size(px(6.)).rounded_full().bg(p.text)).into_any_element()
        } else {
            div().size(px(ICON_SM)).flex_none().rounded_full().border(px(1.5)).border_color(Rgba { a: 0.55, ..p.muted }).into_any_element()
        };
        steps = steps.child(
            h_flex()
                .flex_none()
                .gap_1p5()
                .items_center()
                .h(px(24.))
                .text_size(px(SMALL))
                .text_color(if on { p.text } else if ticked { p.text2 } else { p.muted })
                .when(on, |d| d.font_weight(FontWeight::MEDIUM))
                .child(mark)
                .child(label(step)),
        );
    }
    let ask = if designer.is_some() { t!("design.setup_ask") } else { t!("design.setup_ask_pm") };
    // "로고: 세 가지 시안 중 …", the step's name in the words' colour.
    let line = now_line(at);
    let named = line.find(label(at)).map(|from| from..from + label(at).len());
    let words = StyledText::new(line).with_highlights(named.map(|r| (r, HighlightStyle { color: Some(p.text.into()), font_weight: Some(FontWeight::MEDIUM), ..Default::default() })));
    // The ask beside what the step makes; when the screen is too narrow for
    // both, under it, so the words keep a readable measure.
    let now = h_flex()
        .flex_wrap()
        .gap_x_3()
        .gap_y_2()
        .items_center()
        .child(
            h_flex()
                .flex_1()
                .min_w(px(STEP_LINE_MIN))
                .gap_2()
                .items_start()
                .children(designer.map(|m| div().pt(px((SMALL_LINE - AVATAR) / 2.)).child(theme::avatar(p, &m.role, &m.name, AVATAR))))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(SMALL))
                        .line_height(px(SMALL_LINE))
                        .text_color(p.text2)
                        .child(words),
                ),
        )
        .child(match making {
            Some(m) => h_flex()
                .id("design-setup-working")
                .flex_none()
                .h(px(28.))
                .px_2p5()
                .gap_1p5()
                .items_center()
                .rounded(px(14.))
                .border_1()
                .border_color(p.hair)
                .text_size(px(SMALL))
                .text_color(p.text)
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash))
                .child(if m.busy { crate::motion::spinner(12., p.accent) } else { div().size(px(6.)).rounded_full().bg(p.accent).into_any_element() })
                .child(working_word(m.ticket))
                .tip(p, t!("design.setup_working_tip"), None)
                .on_click(cx.listener(|this, _, _, cx| this.design_view_working(true, cx)))
                .into_any_element(),
            None => div()
                .flex_none()
                .child(
                    ui::button(p, "design-setup-ask", ui::Kind::Primary, ask)
                        .icon(Icon::Message)
                        .on_click(cx.listener(|this, _, window, cx| this.design_ask_setup(window, cx))),
                )
                .into_any_element(),
        });
    Some(v_flex().id("design-setup").flex_none().px(px(PAD)).py_3().gap_2().border_b_1().border_color(p.hair).child(steps).child(now).into_any_element())
}

impl Shell {
    /// "디자이너에게 이어서 부탁하기": the step the shown system's setup is at
    /// goes to the designer as the person's message, the way the empty
    /// screen's ask does — or, with no designer, to the PM to propose one.
    pub fn design_ask_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(system) = self.design.current() else { return };
        let designer = designer(&self.model).map(|m| m.session);
        let pm = self.model.members.iter().find(|m| Model::is_pm(m)).map(|m| m.session);
        let (to, text) = match (designer, pm) {
            (Some(d), _) => (d, request(system)),
            (None, Some(pm)) => (pm, pm_request(system)),
            (None, None) => {
                self.composer.flash = Some(t!("design.no_one_to_ask").into());
                cx.notify();
                return;
            }
        };
        let Some(text) = text else { return };
        self.select_member(to, window, cx);
        self.composer.set_text(&text, window, cx);
        self.send(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{STEPS, current, label, makes, now_line, pm_request, request, shows, working_word};
    use equipe_core::protocol::design::{DesignSystem, Setup, SetupStep};

    fn system(project: Option<&str>, done: &[Setup]) -> DesignSystem {
        let setup = STEPS.iter().map(|s| SetupStep { step: *s, done: done.contains(s) }).collect();
        DesignSystem { project: project.map(Into::into), dir: "/h/web/design".into(), setup, ..Default::default() }
    }

    #[test]
    fn the_setup_is_at_its_first_step_not_done() {
        assert_eq!(current(&system(None, &[])), Some(Setup::Brief));
        assert_eq!(current(&system(None, &[Setup::Brief, Setup::Logo, Setup::System])), Some(Setup::Flows));
        assert_eq!(current(&system(None, &[Setup::Brief, Setup::Flows])), Some(Setup::Logo), "a later step done first doesn't skip one left");
        assert_eq!(current(&system(None, &STEPS)), None);
        // Steps as sent, out of order or some missing: the order is the setup's own.
        let mut odd = system(None, &[Setup::Logo, Setup::Brief]);
        odd.setup.reverse();
        odd.setup.retain(|s| s.step != Setup::System);
        assert_eq!(current(&odd), Some(Setup::System));
    }

    #[test]
    fn the_band_shows_while_a_step_is_left_and_not_for_a_daemon_that_doesnt_say() {
        assert!(shows(&system(Some("web"), &[Setup::Brief])));
        assert!(shows(&system(Some("web"), &[Setup::Brief, Setup::Logo, Setup::System, Setup::Flows])));
        assert!(!shows(&system(Some("web"), &STEPS)), "all done: gone");
        assert!(!shows(&DesignSystem::default()), "no steps sent: nothing to say");
    }

    #[test]
    fn the_designer_is_asked_for_the_step_it_is_at_in_the_shown_system() {
        let web = system(Some("web"), &[Setup::Brief, Setup::Logo, Setup::System]);
        assert_eq!(
            request(&web).unwrap(),
            "web 프로젝트 디자인(/h/web/design)의 셋업을 이어서 해 주세요. 다음은 화면 지도예요: 서비스의 흐름과 화면을 flows/flows.json에 정리하고, 화면마다 상태(기본·빈 상태·로딩·오류 등)별 그림을 라이트와 다크로 찍어 그 옆에 두세요."
        );
        let logo = request(&system(None, &[Setup::Brief])).unwrap();
        assert!(logo.starts_with("스쿼드 디자인(/h/web/design)의 셋업을 이어서 해 주세요. 다음은 로고예요"), "{logo}");
        assert!(logo.contains("라이트·다크 버전과 앱 아이콘"), "{logo}");
        assert!(request(&system(None, &[])).unwrap().contains("셋업을 시작해 주세요. 첫 단계는 브리프예요"));
        assert!(request(&system(None, &[Setup::Brief, Setup::Logo, Setup::System, Setup::Flows])).unwrap().contains("마지막은 디자인 QA예요"));
        assert_eq!(request(&system(None, &STEPS)), None, "nothing left to ask for");
    }

    #[test]
    fn with_no_designer_the_pm_is_asked_to_propose_one_and_hand_it_the_step() {
        let web = system(Some("web"), &[Setup::Brief]);
        let ask = pm_request(&web).unwrap();
        assert!(ask.starts_with("web 프로젝트 디자인(/h/web/design)의 셋업을 이어 갈 디자이너가 팀에 없어요. 한 명 추가하자고 제안해 주고"), "{ask}");
        assert!(ask.contains("「로고」부터"), "{ask}");
        assert!(ask.ends_with(&request(&web).unwrap()), "the designer's words, for the PM to pass on");
        assert_eq!(pm_request(&system(None, &STEPS)), None);
    }

    #[test]
    fn a_step_being_made_says_so_and_by_which_ticket() {
        assert_eq!(working_word(378), "진행 중 · T-378");
    }

    #[test]
    fn every_step_has_a_name_and_says_what_it_makes() {
        assert_eq!(STEPS.map(label), ["브리프", "로고", "디자인 시스템", "화면 지도", "디자인 QA"]);
        assert_eq!(makes(Setup::Logo), "세 가지 시안 중 하나를 고르면 라이트·다크와 앱 아이콘까지 만들어요");
        assert!(STEPS.iter().all(|s| !makes(*s).is_empty()));
        assert_eq!(now_line(Setup::Logo), "로고: 세 가지 시안 중 하나를 고르면 라이트·다크와 앱 아이콘까지 만들어요");
    }
}
