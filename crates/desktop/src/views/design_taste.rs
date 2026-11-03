//! 취향과 결정: the person's design taste as notes the designer follows —
//! each editable where it stands, removable, with a muted line on where it
//! came from — and below, the decisions made: a timeline of the date, the
//! chosen picture, the question, the choice and why (`crate::taste` in core
//! keeps both). With nothing yet, an invitation to write the first note.

use equipe_core::protocol::Command;
use equipe_core::protocol::design::{Decision, EvidenceKind, TasteChange, TasteNote};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::design::{self as ds, Section};
use crate::icons::{Icon, icon};
use crate::theme::{CONTROL, ICON_SM, META, MONO, Palette, R_LG, R_MD, SMALL, UI};
use crate::{Shell, ui};

/// The column the notes and the timeline read in.
const WIDTH: f32 = 720.;
const PAD: f32 = 32.;
/// A decision's picture in the timeline.
const THUMB_W: f32 = 112.;
const THUMB_H: f32 = 72.;

/// The box a note is written in: a new one (`was` none) or one rewritten.
#[derive(Default)]
pub struct Inputs {
    editing: Option<(Option<String>, Entity<InputState>)>,
    subscriptions: Vec<Subscription>,
}

impl Inputs {
    /// Whether the box has the keyboard: what is typed is the note's.
    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        self.editing.as_ref().is_some_and(|(_, b)| b.read(cx).focus_handle(cx).is_focused(window))
    }

    pub fn editing(&self) -> bool {
        self.editing.is_some()
    }

    fn was(&self) -> Option<Option<&str>> {
        self.editing.as_ref().map(|(was, _)| was.as_deref())
    }
}

pub fn page(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.design;
    let taste = ui.current_taste();
    let notes: &[TasteNote] = taste.map_or(&[], |t| &t.notes);
    let decisions: &[Decision] = taste.map_or(&[], |t| &t.decisions);
    let content = v_flex()
        .w_full()
        .max_w(px(WIDTH))
        .gap(px(36.))
        // No system yet: the way back to what one is.
        .when(ui.current().is_none(), |d| {
            d.child(h_flex().child(ui::button(p, "design-taste-back", ui::Kind::Ghost, t!("design.taste_back")).icon(Icon::ArrowLeft).on_click(cx.listener(|this, _, _, cx| this.design_section(Section::Overview, cx)))))
        })
        .child(notes_section(shell, notes, taste.map(|t| t.notes_at.as_str()), cx))
        .child(decisions_section(shell, decisions));
    div()
        .id("design-taste")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&shell.design.scroll)
        .child(div().w_full().flex().justify_center().child(div().w_full().max_w(px(WIDTH + 2. * PAD)).px(px(PAD)).pt(px(24.)).pb(px(64.)).flex().justify_center().child(content)))
        .into_any_element()
}

fn heading(p: Palette, title: &'static str, note: &'static str) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .child(div().text_size(px(UI)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title))
        .child(div().text_size(px(SMALL)).text_color(p.muted).child(note))
}

/// The box to write a note in, and its two buttons.
fn editor(shell: &Shell, input: &Entity<InputState>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let focus = input.read(cx).focus_handle(cx);
    let ring = BoxShadow { color: p.accent.into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: false };
    h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .child(
            div()
                .id("design-taste-box")
                .flex_1()
                .min_w_0()
                .h(px(CONTROL + 6.))
                .px_3()
                .flex()
                .items_center()
                .rounded(px(R_MD))
                .border_1()
                .border_color(p.hair)
                .bg(p.bg)
                .track_focus(&focus)
                .in_focus(move |s| s.border_color(ring.color).shadow(vec![ring]))
                .text_size(px(UI))
                .child(Input::new(input).appearance(false).bordered(false)),
        )
        .child(ui::button(p, "design-taste-save", ui::Kind::Primary, t!("design.taste_save")).on_click(cx.listener(|this, _, window, cx| this.design_taste_save(window, cx))))
        .child(ui::button(p, "design-taste-cancel", ui::Kind::Ghost, t!("design.taste_cancel")).on_click(cx.listener(|this, _, _, cx| this.design_taste_cancel(cx))))
        .into_any_element()
}

/// A date as the screen says it.
fn date(date: &str) -> String {
    ds::ymd(date).map(|(year, month, day)| t!("design.taste_date", year = year, month = month, day = day)).unwrap_or_else(|| date.to_owned())
}

/// Who wrote a note, where it came from, and when, in a muted line.
fn origin(note: &TasteNote) -> String {
    let mut parts: Vec<String> = ds::note_origin(note).into_iter().map(|(key, args)| crate::i18n::catalog().fill(key, &args)).collect();
    parts.extend(note.date.as_deref().map(date));
    parts.join(" · ")
}

/// What a note was learned from, a line each: the picture or the person's words, and when.
fn evidence_list(shell: &Shell, note: &TasteNote) -> AnyElement {
    let p = shell.palette;
    let mut list = v_flex().w_full().mt_2().gap_2().pl_3().border_l_2().border_color(p.hair);
    for e in &note.evidence {
        let kind = match e.kind {
            EvidenceKind::Chose => t!("design.taste_evidence_chose"),
            EvidenceKind::SentBack => t!("design.taste_evidence_sent_back"),
        };
        let mut meta = vec![kind.to_owned(), date(&e.date)];
        meta.extend(e.ticket.map(|t| format!("T-{t}")));
        list = list.child(
            h_flex()
                .w_full()
                .gap_3()
                .items_start()
                .children(e.picture.as_deref().map(|path| {
                    div().w(px(64.)).h(px(42.)).flex_none().rounded(px(6.)).border_1().border_color(p.hair).bg(p.wash).overflow_hidden().flex().items_center().justify_center().child(super::design::picture(shell, path, p.muted))
                }))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(div().text_size(px(SMALL)).text_color(p.text).child(e.what.clone()))
                        .children(e.words.as_ref().map(|w| div().text_size(px(SMALL)).text_color(p.text2).child(t!("design.taste_why", why = w))))
                        .child(div().text_size(px(META)).text_color(p.muted).child(meta.join(" · "))),
                ),
        );
    }
    list.into_any_element()
}

fn notes_section(shell: &Shell, notes: &[TasteNote], kept_at: Option<&str>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let editing = shell.design_taste.was();
    let adding = editing == Some(None);
    let input = shell.design_taste.editing.as_ref().map(|(_, b)| b.clone());
    let mut list = v_flex().w_full().rounded(px(R_LG)).border_1().border_color(p.hair).bg(p.raised).overflow_hidden();
    for (n, note) in notes.iter().enumerate() {
        let here = editing == Some(Some(note.text.as_str()));
        let group = SharedString::from(format!("design-taste-note-{n}"));
        let row = h_flex().id(group.clone()).group(group.clone()).w_full().px_4().py(px(10.)).gap_3().items_start().border_b_1().border_color(p.hair);
        let row = match (&input, here) {
            (Some(input), true) => row.child(div().flex_1().min_w_0().child(editor(shell, input, cx))),
            _ => {
                let (edit, remove, confirm, toggle) = (note.text.clone(), note.text.clone(), note.text.clone(), note.text.clone());
                let waiting = !note.confirmed;
                let open = shell.design.evidence_open.contains(&note.text);
                let amber = p.amber;
                // Confirmed: a filled dot; waiting for the person: a ring.
                let dot = div().mt(px(8.)).size(px(7.)).flex_none().rounded_full().when(!waiting, |d| d.bg(p.accent)).when(waiting, |d| d.border_1().border_color(amber));
                let evidence = (!note.evidence.is_empty()).then(|| {
                    h_flex()
                        .id(SharedString::from(format!("design-taste-evidence-{n}")))
                        .flex_none()
                        .gap_0p5()
                        .items_center()
                        .cursor_pointer()
                        .text_size(px(META))
                        .text_color(p.text2)
                        .hover(move |s| s.text_color(p.text))
                        .child(t!("design.taste_evidence", n = note.evidence.len()))
                        .child(icon(if open { Icon::ChevronUp } else { Icon::ChevronDown }, 12., p.muted))
                        .on_click(cx.listener(move |this, _, _, cx| this.design_taste_evidence(&toggle, cx)))
                });
                row.child(dot)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .flex_wrap()
                                    .child(div().text_size(px(UI)).line_height(px(22.)).text_color(p.text).child(note.text.clone()))
                                    .when(waiting, |r| r.child(div().px_1p5().rounded(px(4.)).bg(Rgba { a: 0.14, ..amber }).text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(t!("design.taste_unconfirmed")))),
                            )
                            .child(h_flex().gap_2().items_center().flex_wrap().child(div().text_size(px(META)).text_color(p.muted).child(origin(note))).children(evidence))
                            .when(open && !note.evidence.is_empty(), |d| d.child(evidence_list(shell, note))),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .items_center()
                            .when(waiting, |r| {
                                r.child(ui::button(p, SharedString::from(format!("design-taste-confirm-{n}")), ui::Kind::Secondary, t!("design.taste_confirm")).icon(Icon::Check).on_click(cx.listener(move |this, _, _, cx| this.design_taste_send(TasteChange::Confirm { text: confirm.clone() }, cx))))
                            })
                            .child(
                                h_flex()
                                    .gap_0p5()
                                    .when(!waiting, |r| r.invisible().group_hover(group, |s| s.visible()))
                                    .child(ui::icon_button(p, SharedString::from(format!("design-taste-edit-{n}")), Icon::Pencil, t!("design.taste_edit"), None).on_click(cx.listener(move |this, _, window, cx| this.design_taste_begin(Some(edit.clone()), window, cx))))
                                    .child(ui::icon_button(p, SharedString::from(format!("design-taste-remove-{n}")), Icon::X, t!("design.taste_remove"), None).on_click(cx.listener(move |this, _, _, cx| this.design_taste_send(TasteChange::Remove { text: remove.clone() }, cx)))),
                            ),
                    )
            }
        };
        list = list.child(row);
    }
    // The last row: the box for a new note, or the way to open it.
    let last = match (&input, adding) {
        (Some(input), true) => h_flex().w_full().px_4().py(px(10.)).child(editor(shell, input, cx)).into_any_element(),
        _ => h_flex()
            .id("design-taste-add")
            .w_full()
            .px_4()
            .h(px(44.))
            .gap_2()
            .items_center()
            .cursor_pointer()
            .text_size(px(SMALL))
            .text_color(p.text2)
            .hover(move |s| s.bg(p.wash).text_color(p.text))
            .child(icon(Icon::Plus, ICON_SM, p.text2))
            .child(t!("design.taste_add"))
            .on_click(cx.listener(|this, _, window, cx| this.design_taste_begin(None, window, cx)))
            .into_any_element(),
    };
    let body: AnyElement = if notes.is_empty() && !adding {
        // None yet: what a note does, two to start from, and the way to write one.
        let example = |text: &'static str| div().px_2().py_0p5().rounded(px(6.)).bg(p.wash).text_size(px(SMALL)).text_color(p.text2).child(text);
        v_flex()
            .w_full()
            .p(px(20.))
            .gap_3()
            .rounded(px(R_LG))
            .border_1()
            .border_color(p.hair)
            .bg(p.raised)
            .child(div().text_size(px(UI)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(t!("design.taste_empty_title")))
            .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(t!("design.taste_empty_text")))
            .child(h_flex().flex_wrap().gap_1p5().child(example(t!("design.taste_example_1"))).child(example(t!("design.taste_example_2"))).child(example(t!("design.taste_example_3"))))
            .child(h_flex().pt_1().child(ui::button(p, "design-taste-first", ui::Kind::Primary, t!("design.taste_add")).icon(Icon::Plus).on_click(cx.listener(|this, _, window, cx| this.design_taste_begin(None, window, cx)))))
            .into_any_element()
    } else {
        list.child(last).into_any_element()
    };
    let waiting = notes.iter().filter(|n| !n.confirmed).count();
    v_flex()
        .w_full()
        .gap_3()
        .child(heading(p, t!("design.taste_notes_title"), t!("design.taste_notes_note")))
        .when(waiting > 0, |d| {
            d.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_start()
                    .px_3()
                    .py_2()
                    .rounded(px(R_MD))
                    .bg(Rgba { a: 0.10, ..p.amber })
                    .child(div().mt(px(6.)).size(px(7.)).flex_none().rounded_full().border_1().border_color(p.amber))
                    .child(div().flex_1().min_w_0().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text).child(t!("design.taste_unconfirmed_hint", n = waiting))),
            )
        })
        .child(body)
        .children(kept_at.filter(|k| !k.is_empty()).map(|k| {
            h_flex().gap_1p5().text_size(px(META)).text_color(p.muted).child(t!("design.taste_kept_in")).child(div().min_w_0().font_family(MONO).text_color(p.text2).truncate().child(super::design::short_home(k)))
        }))
        .into_any_element()
}

/// A decision's picture, small: drawn, coming, or a quiet frame without one.
fn thumb(shell: &Shell, picture: Option<&str>) -> AnyElement {
    let p = shell.palette;
    let frame = div().w(px(THUMB_W)).h(px(THUMB_H)).flex_none().rounded(px(R_MD)).border_1().border_color(p.hair).bg(p.wash).overflow_hidden().flex().items_center().justify_center();
    match picture {
        Some(path) => frame.child(super::design::picture(shell, path, p.muted)).into_any_element(),
        None => frame.child(icon(Icon::Check, ICON_SM, p.muted)).into_any_element(),
    }
}

fn decisions_section(shell: &Shell, decisions: &[Decision]) -> AnyElement {
    let p = shell.palette;
    let mut timeline = v_flex().w_full().gap_4();
    for ((year, month), entries) in ds::by_month(decisions) {
        let label = if year == 0 { t!("design.taste_undated").to_owned() } else { t!("design.taste_month", year = year, month = month) };
        let mut group = v_flex().w_full().gap_0().child(div().pb_2().text_size(px(META)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child(label));
        let last = entries.len().saturating_sub(1);
        for (i, at) in entries.into_iter().enumerate() {
            let d = &decisions[at];
            let day = ds::ymd(&d.date).map(|(_, _, day)| t!("design.taste_day", day = day)).unwrap_or_default();
            let rail = v_flex()
                .w(px(14.))
                .flex_none()
                .items_center()
                .child(div().mt(px(6.)).size(px(9.)).rounded_full().border_2().border_color(p.accent).bg(p.bg))
                .when(i < last, |r| r.child(div().flex_1().w(px(1.)).bg(p.hair)));
            let text = v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_size(px(SMALL)).text_color(p.muted).child(d.question.clone()))
                .child(div().text_size(px(UI)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(d.chosen.clone()))
                .children(d.why.as_ref().map(|why| div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(t!("design.taste_why", why = why))))
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .text_size(px(META))
                        .text_color(p.muted)
                        .children(d.ticket.map(|t| div().px_1p5().rounded(px(4.)).bg(p.wash).font_family(MONO).child(format!("T-{t}"))))
                        .when(!d.passed.is_empty(), |r| r.child(t!("design.taste_passed", options = d.passed.join(", ")))),
                );
            group = group.child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_stretch()
                    .child(div().w(px(36.)).flex_none().pt(px(2.)).text_size(px(META)).text_color(p.text2).child(day))
                    .child(rail)
                    .child(h_flex().flex_1().min_w_0().gap_4().items_start().pb(px(20.)).child(thumb(shell, d.picture.as_deref())).child(text)),
            );
        }
        timeline = timeline.child(group);
    }
    let body: AnyElement = if decisions.is_empty() {
        v_flex()
            .w_full()
            .p(px(20.))
            .gap_1p5()
            .rounded(px(R_LG))
            .border_1()
            .border_color(p.hair)
            .child(div().text_size(px(UI)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(t!("design.taste_no_decisions_title")))
            .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(t!("design.taste_no_decisions_text")))
            .into_any_element()
    } else {
        timeline.into_any_element()
    };
    v_flex().w_full().gap_3().child(heading(p, t!("design.taste_decisions_title"), t!("design.taste_decisions_note"))).child(body).into_any_element()
}

// ── what the tab does ──

impl Shell {
    /// Write a note: a new one (`was` none) or `was` rewritten, in a box that has the keyboard.
    pub fn design_taste_begin(&mut self, was: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let text = was.clone().unwrap_or_default();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder(t!("design.taste_placeholder"));
            state.set_value(text, window, cx);
            state
        });
        // ⏎ keeps it.
        let subscription = cx.subscribe_in(&input, window, |shell, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                shell.design_taste_save(window, cx);
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        self.design_taste.editing = Some((was, input));
        self.design_taste.subscriptions = vec![subscription];
        cx.notify();
    }

    /// Type into the open box (`snapshot.rs`).
    pub fn design_taste_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, input)) = &self.design_taste.editing {
            input.update(cx, |s, cx| s.set_value(text.to_owned(), window, cx));
        }
        cx.notify();
    }

    pub fn design_taste_cancel(&mut self, cx: &mut Context<Self>) {
        self.design_taste.editing = None;
        self.design_taste.subscriptions.clear();
        cx.notify();
    }

    /// The box's words to the daemon, as a new note or the rewritten one; an
    /// empty or unchanged box just closes.
    pub fn design_taste_save(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((was, input)) = self.design_taste.editing.take() else { return };
        self.design_taste.subscriptions.clear();
        let text = input.read(cx).value().trim().to_owned();
        let change = match was {
            _ if text.is_empty() => None,
            Some(was) if was == text => None,
            Some(was) => Some(TasteChange::Edit { was, text }),
            None => Some(TasteChange::Add { text }),
        };
        if let Some(change) = change {
            self.design_taste_send(change, cx);
        }
        cx.notify();
    }

    /// A change to the notes of the source on screen.
    pub fn design_taste_send(&mut self, change: TasteChange, cx: &mut Context<Self>) {
        if self.model.squad.is_empty() {
            return;
        }
        let project = self.design.current_taste().and_then(|t| t.project.clone());
        self.send_command(Command::Taste { squad: self.model.squad.clone(), project, change });
        cx.notify();
    }

    /// A note's evidence, opened or put away.
    pub fn design_taste_evidence(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.design.evidence_open.remove(text) {
            self.design.evidence_open.insert(text.to_owned());
        }
        cx.notify();
    }

    /// The pictures on show, as thumbnails: the decisions', and the open evidence's.
    pub fn design_taste_pictures(&self) -> Vec<String> {
        if self.design.section != Section::Taste {
            return Vec::new();
        }
        let Some(taste) = self.design.current_taste() else { return Vec::new() };
        let open = taste.notes.iter().filter(|n| self.design.evidence_open.contains(&n.text)).flat_map(|n| n.evidence.iter());
        taste.decisions.iter().filter_map(|d| d.picture.clone()).chain(open.filter_map(|e| e.picture.clone())).collect()
    }
}
