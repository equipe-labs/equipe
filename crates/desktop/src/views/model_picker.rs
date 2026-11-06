//! The model picker over the composer (⌘⇧I from the model's control, ⌘⇧E
//! from the effort's): Claude | Codex tabs with what is left of each plan,
//! that engine's models, the effort the member's model takes, and what a pick
//! changes and when. The rules are `crate::picker`'s; this draws them and
//! sends the person's pick as the command the daemon takes.

use std::time::{Duration, Instant};

use equipe_core::protocol::{Command, Engine};
use equipe_core::t;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::picker::{self, Act, Do, EffortPart, Focus, Row, View};
use crate::settings::Category;
use crate::state::Presence;
use crate::switching::{Answer, Tone, engine_name};
use crate::theme::{GAP, ICON, ICON_SM, Palette, R_MD, R_SM, ROW};
use crate::views::composer::MenuKind;
use crate::{Shell, motion};

/// The picker as open: its state, and the control it opened from.
#[derive(Debug, Clone)]
pub struct Open {
    pub ui: picker::Ui,
    pub from: MenuKind,
}

/// How wide it is: room for a model's line and the effort's row.
const WIDTH: f32 = 344.;
/// The list scrolls past this — or sooner, so the whole picker fits above
/// its control: `CHROME` is about what it takes besides the list.
const LIST_MAX: f32 = 380.;
const LIST_MIN: f32 = 96.;
const CHROME: f32 = 300.;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl Shell {
    /// What the picker is built from, for the member on screen.
    fn with_picker<R>(&self, f: impl FnOnce(&picker::Member, &picker::World) -> R) -> Option<R> {
        let m = self.model.selected_member()?;
        let asked = m.asked.as_deref().or(m.model.as_deref());
        let member = picker::Member {
            name: &m.name,
            role: &m.role,
            engine: m.engine,
            asked,
            ran: m.model.as_deref(),
            effort: self.model.efforts.get(&m.session).map(String::as_str),
            working: m.presence == Presence::Working,
            on_ticket: m.ticket.is_some(),
        };
        let meters = self.model.engines.meters(now());
        let role_start = picker::ENGINES.map(|engine| {
            let set = self.model.role_models.iter().find(|d| d.role == m.role && d.engine == engine).and_then(|d| d.model.clone());
            set.or_else(|| {
                let ids: Vec<String> = self.model.listed(Some(engine)).iter().map(|c| c.id.clone()).collect();
                equipe_core::roles::recommended_model_for(equipe_core::roles::role_or_custom(&m.role), engine, &ids)
            })
        });
        let world = picker::World {
            claude: self.model.listed(Some(Engine::Claude)),
            codex: self.model.catalogs.get(&Engine::Codex).map(Vec::as_slice),
            codex_state: self.engines.status(Engine::Codex).map(|s| s.state),
            codex_waited: self.composer.codex_asked.map(|at| at.elapsed()),
            providers: self.engines.providers.as_deref().unwrap_or_default(),
            meters: &meters,
            moving: picker::moving(self.model.engines.open(), &m.name),
            role_start,
        };
        Some(f(&member, &world))
    }

    /// The picker as it draws now.
    pub(crate) fn picker_view(&self) -> Option<View> {
        let open = self.composer.picker.as_ref()?;
        self.with_picker(|member, world| picker::build(member, world, &open.ui))
    }

    /// Open the picker from `from` (the model's control, or the effort's), or
    /// close it when it is open from there already.
    pub(crate) fn open_picker(&mut self, from: MenuKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.picker.as_ref().is_some_and(|o| o.from == from) {
            self.close_picker(cx);
            return;
        }
        let Some(ui) = self.with_picker(|member, world| picker::Ui::new(member, world, from == MenuKind::Effort)) else { return };
        self.composer.menu = None;
        self.composer.leaving = None;
        self.composer.picker = Some(Open { ui, from });
        self.ask_for_codex(false, cx);
        self.composer.choices.clear();
        self.composer.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn close_picker(&mut self, cx: &mut Context<Self>) {
        if self.composer.picker.take().is_some() {
            cx.notify();
        }
    }

    /// Codex's models, asked for when there is no list yet — never asked, or
    /// `again` (the person's "Ask again") — and where the engines stand, when
    /// not known: both read-only. The picker is drawn again once the wait for
    /// an answer is over, to say none came.
    fn ask_for_codex(&mut self, again: bool, cx: &mut Context<Self>) {
        if self.engines.statuses.is_none() {
            self.send_command(Command::Engines);
        }
        let listed = self.model.catalogs.get(&Engine::Codex).is_some_and(|l| !l.is_empty());
        if listed || (!again && self.composer.codex_asked.is_some()) {
            if let Some(at) = self.composer.codex_asked.filter(|_| !listed) {
                self.redraw_after(picker::CODEX_WAIT.saturating_sub(at.elapsed()), cx);
            }
            return;
        }
        self.composer.codex_asked = Some(Instant::now());
        self.send_command(Command::Models { engine: Engine::Codex });
        self.redraw_after(picker::CODEX_WAIT, cx);
    }

    fn redraw_after(&self, wait: Duration, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait + Duration::from_millis(50)).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
    }

    /// A key while the picker is open: true when it took it.
    pub(crate) fn picker_key(&mut self, key: &str, modifiers: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.composer.picker.is_none() {
            return false;
        }
        if key == "escape" {
            self.close_picker(cx);
            return true;
        }
        if modifiers.control || modifiers.alt || modifiers.platform {
            return false;
        }
        let Some(k) = picker::Key::parse(key, modifiers.shift) else { return false };
        let Some(view) = self.picker_view() else { return false };
        let Some(open) = self.composer.picker.as_mut() else { return false };
        let (focus, did) = picker::key(&view, open.ui.focus, k);
        open.ui.focus = focus;
        if let Some(did) = did {
            self.picker_do(did, window, cx);
        }
        cx.notify();
        true
    }

    fn picker_do(&mut self, did: Do, window: &mut Window, cx: &mut Context<Self>) {
        match did {
            Do::Show(engine) => self.picker_show(engine, cx),
            Do::Run(act) => self.picker_run(act, window, cx),
            Do::Effort(effort) => self.picker_effort(effort, cx),
            Do::Link(i) => self.picker_link(i, window, cx),
        }
    }

    /// The other engine's tab, the keys where it starts (or still on the tabs).
    fn picker_show(&mut self, engine: Engine, cx: &mut Context<Self>) {
        let Some(open) = self.composer.picker.as_mut() else { return };
        open.ui.tab = engine;
        let on_tabs = open.ui.focus == Focus::Tabs;
        if let Some(view) = self.picker_view()
            && let Some(open) = self.composer.picker.as_mut()
        {
            open.ui.focus = if on_tabs { Focus::Tabs } else { view.first_focus() };
        }
        if engine == Engine::Codex {
            self.ask_for_codex(false, cx);
        }
        cx.notify();
    }

    fn picker_run(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = self.model.selected_member() else { return };
        let (session, name, engine) = (m.session, m.name.clone(), m.engine);
        match act {
            Act::Versions(family) => {
                if let Some(open) = self.composer.picker.as_mut() {
                    match open.ui.open.iter().position(|f| *f == family) {
                        Some(at) => {
                            open.ui.open.remove(at);
                        }
                        None => open.ui.open.push(family),
                    }
                }
                if let Some(view) = self.picker_view()
                    && let Some(open) = self.composer.picker.as_mut()
                {
                    open.ui.focus = picker::settle(&view, open.ui.focus);
                }
            }
            Act::Retry => self.ask_for_codex(true, cx),
            Act::Model(model) => {
                self.close_picker(cx);
                self.send_command(Command::SetModel { session, model: model.clone() });
                let wanted = crate::models::label(&model, self.model.listed(engine));
                // A provider's model from Claude Code: it moves at its next ticket.
                let provider = equipe_core::model_providers::split_ref(&model).filter(|_| engine != Some(Engine::Codex)).map(|(id, _)| self.engines.provider(id).map_or_else(|| id.to_owned(), |p| p.name.clone()));
                self.composer.flash = Some(match provider {
                    Some(provider) => t!("compose.provider_moving", name = name, wanted = wanted, provider = provider),
                    None => t!("compose.model_set", name = name, wanted = wanted),
                });
            }
            Act::Move { engine: to, model } => {
                // What happens and when, as the picker said it.
                let said = self.with_picker(|member, _| picker::applies(member, to, self.model.listed(Some(to))));
                self.close_picker(cx);
                self.send_command(Command::MoveEngine { squad: self.model.squad.clone(), members: vec![name], engine: to, model });
                self.composer.flash = said;
            }
            Act::SignIn(_) => {
                self.close_picker(cx);
                self.open_settings(Some(Category::Engines), window, cx);
                return;
            }
            Act::UpdateClaude => {
                self.close_picker(cx);
                for command in self.engines.install(Engine::Claude) {
                    self.send_command(command);
                }
                self.composer.flash = Some(t!("models.updating").to_owned());
            }
            Act::Undo(id) => {
                self.close_picker(cx);
                self.engines_decide(id, Answer::Undo, cx);
            }
        }
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// The member's effort, from its next turn; the picker stays open on it.
    fn picker_effort(&mut self, effort: Option<String>, cx: &mut Context<Self>) {
        let Some(m) = self.model.selected_member() else { return };
        let (session, name) = (m.session, m.name.clone());
        self.send_command(Command::SetEffort { session, effort: effort.clone() });
        self.composer.flash = Some(match &effort {
            Some(e) => t!("composer.effort_set", name = name, effort = e),
            None => t!("composer.effort_reset", name = name),
        });
        match effort {
            Some(effort) => self.model.efforts.insert(session, effort),
            None => self.model.efforts.remove(&session),
        };
        cx.notify();
    }

    /// The foot's links: the plan's usage on the tab's engine; new members' defaults.
    fn picker_link(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.composer.picker.as_ref().map_or(Engine::Claude, |o| o.ui.tab);
        self.close_picker(cx);
        if i == 0 {
            let asks = self.plan_use.pick_engine(tab);
            self.plan_use_send(asks, cx);
            self.open_usage(cx);
        } else {
            self.open_settings(Some(Category::Team), window, cx);
        }
    }

    /// The pointer moved onto something the keys can reach: they move there
    /// too — on a move, not on the picker opening under a resting pointer.
    fn picker_point(&mut self, focus: Focus, cx: &mut Context<Self>) {
        if let Some(open) = self.composer.picker.as_mut()
            && open.ui.focus != focus
        {
            open.ui.focus = focus;
            cx.notify();
        }
    }
}

/// A window's colour by how low it runs.
fn tone_color(p: Palette, tone: Tone) -> Rgba {
    match tone {
        Tone::Calm => p.muted,
        Tone::Watch => p.amber,
        Tone::Low => p.orange,
        Tone::Out => p.red,
    }
}

/// How hard a model hits the plan, as four small bars rising.
fn gauge(p: Palette, weight: u8) -> AnyElement {
    h_flex()
        .flex_none()
        .items_end()
        .gap(px(1.5))
        .h(px(10.))
        .children((1..=4u8).map(|i| div().w(px(3.)).h(px(3. + 2. * f32::from(i - 1))).rounded(px(1.)).bg(if i <= weight { p.text2 } else { p.hair })))
        .into_any_element()
}

/// The ring the keys' place wears.
fn ring(p: Palette) -> Vec<BoxShadow> {
    vec![BoxShadow { color: Rgba { a: 0.55, ..p.accent }.into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.), inset: false }]
}

/// The picker over its control, when open.
pub fn layer(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let open = shell.composer.picker.as_ref()?;
    let view = shell.picker_view()?;
    let anchor = { let anchors = shell.composer.anchors.borrow(); anchors.get(&open.from).or_else(|| anchors.get(&MenuKind::Model)).copied()? };
    let p = shell.palette;
    let focus = open.ui.focus;
    let name = shell.model.selected_member().map(|m| m.name.clone()).unwrap_or_default();

    // The engines, as tabs: each with what is left of its plan.
    let tabs = h_flex().id("picker-tabs").role(Role::TabList).aria_label(t!("models.tabs_aria")).mx_1().p(px(2.)).gap(px(2.)).rounded(px(R_MD)).bg(p.wash).children(view.tabs.iter().map(|tab| {
        let engine = tab.engine;
        let on = engine == view.tab;
        let keyed = on && focus == Focus::Tabs;
        let (plan, tone) = tab.plan.clone().unwrap_or_else(|| (t!("models.plan_unread").to_owned(), Tone::Calm));
        let mark = if engine == Engine::Claude { Icon::ClaudeMark } else { Icon::OpenAiMark };
        let aria = if tab.here { t!("models.tab_aria_here", engine = engine_name(engine), plan = plan.clone(), name = name.clone()) } else { t!("models.tab_aria", engine = engine_name(engine), plan = plan.clone()) };
        v_flex()
            .id(("picker-tab", engine as usize))
            .flex_1()
            .min_w_0()
            .px_2()
            .py(px(5.))
            .gap(px(1.))
            .rounded(px(R_MD - 2.))
            .cursor_pointer()
            .role(Role::Tab)
            .aria_selected(on)
            .aria_label(aria)
            .when(on, |d| d.bg(p.popover()))
            .when(keyed, |d| d.shadow(ring(p)))
            .child(
                h_flex()
                    .gap(px(6.))
                    .items_center()
                    .child(icon(mark, ICON_SM, if on { p.text } else { p.text2 }))
                    .child(div().min_w_0().truncate().text_size(px(13.)).text_color(if on { p.text } else { p.text2 }).when(on, |d| d.font_weight(FontWeight::SEMIBOLD)).child(engine_name(engine)))
                    .children(tab.here.then(|| div().flex_none().px(px(5.)).rounded(px(4.)).bg(p.accent_bg).text_size(px(10.5)).text_color(p.accent).child(t!("models.tab_here")))),
            )
            .child(div().min_w_0().truncate().text_size(px(11.5)).text_color(if tab.plan.is_some() { tone_color(p, tone) } else { p.muted }).child(plan))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(open) = this.composer.picker.as_mut() {
                    open.ui.focus = Focus::Tabs;
                }
                this.picker_show(engine, cx);
            }))
            .into_any_element()
    }));

    // The list.
    let rows: Vec<AnyElement> = view
        .rows
        .iter()
        .enumerate()
        .map(|(ix, row)| {
            let keyed = focus == Focus::Row(ix);
            let base = || h_flex().id(("picker-row", ix)).min_h(px(ROW)).px_2().gap_2().rounded(px(R_SM));
            let live = |el: Stateful<Div>, cx: &mut Context<Shell>| {
                let act = row.act();
                el.cursor_pointer()
                    .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                        this.picker_point(Focus::Row(ix), cx);
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(act) = act.clone() {
                            this.picker_point(Focus::Row(ix), cx);
                            this.picker_run(act, window, cx);
                        }
                    }))
            };
            match row {
                Row::Heading(text) => div().px_2().pt_2().pb(px(2.)).text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(text.clone()).into_any_element(),
                Row::Note { text, warn } => h_flex()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_start()
                    .when(*warn, |d| d.child(div().pt(px(1.)).flex_none().child(icon(Icon::Warning, ICON_SM, p.amber))))
                    .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(17.)).text_color(if *warn { p.text2 } else { p.muted }).child(text.clone()))
                    .into_any_element(),
                Row::Loading(text) => h_flex()
                    .px_2()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .id("picker-loading")
                    .role(Role::Status)
                    .aria_label(text.clone())
                    .child(Spinner::new().with_size(px(13.)).color(Hsla::from(p.muted)))
                    .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(17.)).text_color(p.muted).child(text.clone()))
                    .into_any_element(),
                Row::Model(m) => {
                    let weight = m.weight.filter(|_| m.about.is_empty()).map(picker::weight_words).unwrap_or_default();
                    let aria = [m.label.as_str(), m.detail.as_str(), m.about.as_str(), weight].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
                    let lead = if m.on { icon(Icon::Check, ICON, p.accent).into_any_element() } else { div().into_any_element() };
                    let el = base()
                        .role(Role::MenuItemRadio)
                        .aria_label(aria)
                        .aria_selected(keyed)
                        .aria_toggled(if m.on { Toggled::True } else { Toggled::False })
                        .py(px(5.))
                        .items_start()
                        .when(m.nested, |d| d.pl(px(8. + 14.)))
                        .child(div().pt(px(1.)).w(px(ICON)).flex_none().flex().justify_center().child(lead))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(1.))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(p.text).when(m.on, |d| d.font_weight(FontWeight::MEDIUM)).child(m.label.clone()))
                                        .children((!m.detail.is_empty()).then(|| div().flex_none().max_w(px(150.)).truncate().text_size(px(12.)).text_color(p.muted).child(m.detail.clone())))
                                        .children(m.weight.map(|w| gauge(p, w))),
                                )
                                .children((!m.about.is_empty()).then(|| div().text_size(px(12.)).line_height(px(16.)).text_color(p.muted).child(m.about.clone()))),
                        );
                    motion::hover_fill(live(el, cx), ("picker-row-fill", ix), p.wash, keyed).into_any_element()
                }
                Row::Versions { family, count, open } => {
                    let words = t!("models.versions", count = count);
                    let family_name = equipe_core::models::family(family).map_or("", |f| f.name);
                    let el = base()
                        .h(px(26.))
                        .min_h(px(26.))
                        .items_center()
                        .role(Role::Button)
                        .aria_label(t!("models.versions_aria", family = family_name, count = count))
                        .aria_expanded(*open)
                        .aria_selected(keyed)
                        .child(div().w(px(ICON)).flex_none().flex().justify_center().child(icon(if *open { Icon::ChevronDown } else { Icon::ChevronRight }, ICON_SM, p.muted)))
                        .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(p.text2).child(words));
                    motion::hover_fill(live(el, cx), ("picker-row-fill", ix), p.wash, keyed).into_any_element()
                }
                Row::Action { label, act } => {
                    let glyph = match act {
                        Act::Retry => Icon::Retry,
                        Act::UpdateClaude => Icon::ArrowUp,
                        Act::Undo(_) => Icon::Withdraw,
                        Act::SignIn(_) => Icon::Settings,
                        Act::Move { engine: Engine::Codex, .. } => Icon::OpenAiMark,
                        Act::Move { .. } => Icon::ClaudeMark,
                        _ => Icon::ChevronRight,
                    };
                    let el = base()
                        .items_center()
                        .role(Role::Button)
                        .aria_label(label.clone())
                        .aria_selected(keyed)
                        .child(div().w(px(ICON)).flex_none().flex().justify_center().child(icon(glyph, ICON_SM, p.text2)))
                        .child(div().flex_1().min_w_0().text_size(px(13.)).text_color(p.text).child(label.clone()));
                    motion::hover_fill(live(el, cx), ("picker-row-fill", ix), p.wash, keyed).into_any_element()
                }
            }
        })
        .collect();
    let list = v_flex().id("picker-list").role(Role::Menu).aria_label(t!("composer.menu_model", name = name.clone())).max_h(px((f32::from(anchor.origin.y) - GAP - 8. - CHROME).clamp(LIST_MIN, LIST_MAX))).overflow_y_scroll().px_1().pt_1().children(rows.into_iter().map(|row| div().flex_none().child(row)));

    // The effort the member's model takes.
    let effort: AnyElement = match &view.effort {
        EffortPart::Pick(e) => {
            let chip = |i: usize, label: String, on: bool, stray: bool, cx: &mut Context<Shell>| {
                let keyed = focus == Focus::Effort(i);
                let value = e.at(i).flatten();
                div()
                    .id(("picker-effort", i))
                    .h(px(24.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(R_MD - 2.))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .role(Role::RadioButton)
                    .aria_selected(on)
                    .aria_toggled(if on { Toggled::True } else { Toggled::False })
                    .aria_label(if stray { format!("{label}, {}", t!("models.effort_not_taken")) } else { label.clone() })
                    .when(on, |d| d.bg(p.popover()).text_color(p.text).font_weight(FontWeight::SEMIBOLD))
                    .when(!on, |d| d.text_color(if stray { p.muted } else { p.text2 }).hover(move |s| s.text_color(p.text)))
                    .when(keyed, |d| d.bg(p.popover()).shadow(ring(p)))
                    .child(label)
                    .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                        this.picker_point(Focus::Effort(i), cx);
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.picker_point(Focus::Effort(i), cx);
                        this.picker_effort(value.clone(), cx);
                    }))
                    .into_any_element()
            };
            let mut chips = vec![chip(0, t!("composer.effort_default").to_owned(), e.now.is_none(), false, cx)];
            for (i, level) in e.levels.iter().enumerate() {
                let stray = e.stray.as_ref() == Some(level);
                chips.push(chip(i + 1, level.clone(), e.now.as_ref() == Some(level), stray, cx));
            }
            let hint = match &e.default {
                Some(d) => t!("models.effort_hint_default", effort = d.clone()),
                None => t!("models.effort_hint").to_owned(),
            };
            v_flex()
                .px_3()
                .pt_2()
                .gap(px(6.))
                .child(h_flex().gap_2().items_center().child(div().flex_none().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(t!("models.effort_title"))).child(div().flex_1().min_w_0().text_size(px(11.5)).text_color(p.muted).child(hint)))
                .child(h_flex().id("picker-efforts").role(Role::RadioGroup).aria_label(t!("composer.menu_effort", name = name.clone())).flex_wrap().p(px(2.)).gap(px(2.)).rounded(px(R_MD)).bg(p.wash).children(chips))
                .into_any_element()
        }
        EffortPart::None(text) | EffortPart::Elsewhere(text) => v_flex()
            .px_3()
            .pt_2()
            .gap(px(2.))
            .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(t!("models.effort_title")))
            .child(div().text_size(px(12.)).line_height(px(17.)).text_color(p.muted).child(text.clone()))
            .into_any_element(),
    };

    // What a pick changes and when; the plan's usage and new members' defaults.
    let link = |i: usize, label: String, cx: &mut Context<Shell>| {
        let keyed = focus == Focus::Link(i);
        div()
            .id(("picker-link", i))
            .h(px(22.))
            .px_1p5()
            .flex()
            .items_center()
            .gap_1()
            .rounded(px(R_SM))
            .cursor_pointer()
            .role(Role::Link)
            .aria_label(label.clone())
            .text_size(px(12.))
            .text_color(p.accent)
            .when(keyed, |d| d.bg(p.popover()).shadow(ring(p)))
            .child(label)
            .child(icon(Icon::ChevronRight, 12., p.accent))
            .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                this.picker_point(Focus::Link(i), cx);
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.picker_link(i, window, cx)))
            .into_any_element()
    };
    let mut links = vec![link(0, t!("models.usage_link", engine = engine_name(view.tab)), cx)];
    if view.role.is_some() {
        links.push(link(1, t!("models.role_link").to_owned(), cx));
    }
    let foot = v_flex()
        .mt_2()
        .px_3()
        .pt_2()
        .pb_1()
        .gap(px(4.))
        .border_t_1()
        .border_color(p.hair)
        .child(div().text_size(px(12.)).line_height(px(17.)).text_color(p.text2).child(view.applies.clone()))
        .children(view.role.clone().map(|role| div().text_size(px(12.)).line_height(px(17.)).text_color(p.muted).child(role)))
        .child(h_flex().flex_wrap().gap_2().ml(px(-6.)).children(links));

    let title = t!("composer.menu_model", name = name);
    let surface = v_flex()
        .id("model-picker")
        .role(Role::Dialog)
        .aria_label(title.clone())
        .w(px(WIDTH))
        .pt_1()
        .pb_1()
        .rounded(px(R_MD + 2.))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .text_color(p.text)
        .child(div().h(px(26.)).px_3().flex().items_center().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(title))
        .child(tabs)
        .child(list)
        .child(div().mx_3().mt_1().h(px(1.)).bg(p.hair))
        .child(effort)
        .child(foot)
        .on_mouse_down_out(cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            if !anchor.contains(&e.position) {
                this.close_picker(cx);
            }
        }));
    let surface = motion::pop(surface, "model-picker-in", true).into_any_element();
    let align = gpui_kit::base::Align::End;
    let positioner = gpui_kit::base::Positioner::side(anchor).placement(gpui_kit::base::Placement::Top).align(align).offset(px(GAP)).margin(px(8.)).occlude();
    Some(deferred(positioner.child(surface)).with_priority(2).into_any_element())
}
