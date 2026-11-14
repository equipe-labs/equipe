//! Design screen · References: the best-in-class examples the designer keeps
//! for each interface pattern (`design/references/<pattern>/`, read through
//! the daemon — `crate::refshelf`). A list of patterns, each with how many
//! pictures and notes it holds and when it last changed; one opened shows its
//! pictures as cards with the source's host and the notes' take, leave and fit.
//! A picture opens big in the design screen's lightbox (←/→ through the
//! pattern), a card's source opens in the browser. A search filters patterns
//! and notes. With none yet, what references are.

use equipe_core::protocol::design::{RefPattern, RefSource};
use equipe_core::t;
use gpui_kit::component::input::Input;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::refshelf::{self, Entry};
use crate::theme::{BODY, BODY_LINE, META, MONO, R_LG, R_MD, SMALL};
use crate::tips::TipExt as _;
use crate::visuals::Size;
use crate::{Shell, motion, ui};

const PAD: f32 = 32.;
const MAX_W: f32 = 1040.;
const CARD_W: f32 = 320.;
const SHOT_H: f32 = 200.;
const COVER_W: f32 = 96.;
const COVER_H: f32 = 64.;
/// How long typing rests before the daemon is asked for the words.
const SEARCH_AFTER: std::time::Duration = std::time::Duration::from_millis(250);

pub fn page(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let ui_ = &shell.design.refs;
    let body: AnyElement = match &ui_.answer {
        None => loading(shell),
        Some(a) if a.patterns.is_empty() && ui_.query.is_empty() && a.sources.is_empty() => empty(shell, &a.folder, cx),
        Some(a) if a.patterns.is_empty() => nothing_found(shell, &ui_.query),
        Some(_) => match ui_.open() {
            Some(pattern) => detail(shell, pattern, cx),
            None => list(shell, cx),
        },
    };
    v_flex()
        .id("design-refs")
        .flex_1()
        .min_h_0()
        .child(toolbar(shell, cx))
        .child(
            div()
                .id("design-refs-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&ui_.scroll)
                .child(div().w_full().max_w(px(MAX_W)).mx_auto().px(px(PAD)).pt(px(8.)).pb(px(96.)).child(body)),
        )
        .into_any_element()
}

fn loading(shell: &Shell) -> AnyElement {
    let p = shell.palette;
    h_flex().w_full().py(px(80.)).gap_2().items_center().justify_center().text_size(px(SMALL)).text_color(p.muted).child(motion::spinner(16., p.muted)).child(t!("design.refs_loading")).into_any_element()
}

/// The search, and what the list holds.
fn toolbar(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let typed = !shell.refs_query(cx).is_empty();
    let total = shell.design.refs.answer.as_ref().map_or(0, |a| a.total);
    let searching = shell.design.refs.answer.as_ref().is_some_and(|a| !a.patterns.is_empty() || !shell.design.refs.query.is_empty());
    h_flex()
        .flex_none()
        .w_full()
        .max_w(px(MAX_W))
        .mx_auto()
        .px(px(PAD))
        .pt(px(16.))
        .pb(px(8.))
        .gap_2()
        .items_center()
        .when(searching, |bar| {
            bar.child(
                h_flex()
                    .id("design-refs-search")
                    .h(px(34.))
                    .w(px(340.))
                    .max_w_full()
                    .flex_shrink_1()
                    .pl_2()
                    .pr_1()
                    .gap_1()
                    .items_center()
                    .rounded(px(R_MD))
                    .bg(p.wash)
                    .border_1()
                    .border_color(p.hair)
                    .text_size(px(SMALL))
                    .child(icon(Icon::Search, 14., p.muted))
                    .child(div().flex_1().min_w_0().child(Input::new(&shell.refs_search).appearance(false).bordered(false)))
                    .when(typed, |b| {
                        b.child(ui::icon_button(p, "design-refs-search-clear", Icon::X, t!("design.refs_search_clear"), None).on_click(cx.listener(|this, _, window, cx| {
                            this.refs_search.update(cx, |s, cx| s.set_value(String::new(), window, cx));
                            cx.notify();
                        })))
                    }),
            )
        })
        .child(div().flex_1())
        .when(searching, |bar| bar.child(div().flex_none().text_size(px(META)).text_color(p.muted).child(t!("design.refs_patterns", count = total))))
        .child(ui::icon_button(p, "design-refs-refresh", Icon::Reload, t!("design.refs_refresh"), None).on_click(cx.listener(|this, _, _, cx| this.refs_refresh(cx))))
}

/// A picture of the references, as the daemon read it: drawn, coming, or why not.
fn thumb(shell: &Shell, path: &str, cover: bool) -> AnyElement {
    let p = shell.palette;
    let shots = shell.shots.borrow();
    match shots.best(path) {
        Some(shot) if cover => img(shot.image).size_full().object_fit(ObjectFit::Cover).into_any_element(),
        Some(shot) => img(shot.image).max_w_full().max_h_full().object_fit(ObjectFit::Contain).into_any_element(),
        None if shots.is_gone(path) => icon(Icon::Alert, 16., p.muted).into_any_element(),
        None => motion::spinner(14., p.muted),
    }
}

// ── the patterns ──

fn list(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui_ = &shell.design.refs;
    let Some(answer) = ui_.answer.as_ref() else { return loading(shell) };
    let settled = answer.query == ui_.query;
    let shown: Vec<&RefPattern> = answer.patterns.iter().filter(|pat| settled || refshelf::pattern_matches(pat, &ui_.query)).collect();
    let rows: Vec<AnyElement> = shown.iter().map(|pat| row(shell, pat, cx)).collect();
    let more = answer.patterns.len() < answer.total;
    v_flex()
        .w_full()
        .gap_2()
        .children(rows)
        .when(shown.is_empty(), |c| c.child(nothing_found(shell, &ui_.query)))
        .when(more, |c| {
            c.child(
                h_flex().w_full().pt_2().gap_3().items_center().justify_center().child(div().text_size(px(META)).text_color(p.muted).child(t!("design.refs_showing", shown = answer.patterns.len(), total = answer.total))).child(
                    ui::button(p, "design-refs-more", ui::Kind::Secondary, t!("design.refs_more")).on_click(cx.listener(|this, _, _, cx| this.refs_more(cx))),
                ),
            )
        })
        .children(sources(shell, &answer.sources, cx))
        .into_any_element()
}

fn row(shell: &Shell, pattern: &RefPattern, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let name = pattern.name.clone();
    let cover = refshelf::cover(pattern).map(|c| thumb(shell, &c.path, true));
    let updated = (pattern.updated > 0).then(|| t!("design.refs_updated", when = crate::activity::ago(shell.model.unix.saturating_sub(pattern.updated))));
    let counts = [t!("design.refs_pictures", count = pattern.pictures_total as usize), t!("design.refs_notes", count = pattern.notes.len())].join(" · ");
    h_flex()
        .id(SharedString::from(format!("design-refs-row-{}", pattern.name)))
        .w_full()
        .gap_4()
        .p_3()
        .items_center()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .child(
            div()
                .flex_none()
                .w(px(COVER_W))
                .h(px(COVER_H))
                .rounded(px(R_MD))
                .overflow_hidden()
                .bg(p.wash)
                .border_1()
                .border_color(p.hair)
                .flex()
                .items_center()
                .justify_center()
                .child(cover.unwrap_or_else(|| icon(Icon::Eye, 18., p.muted).into_any_element())),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().min_w_0().truncate().text_size(px(BODY)).line_height(px(BODY_LINE)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(refshelf::title(&pattern.name)))
                        .when(pattern.shared, |r| r.child(shared_chip(shell, &pattern.name))),
                )
                .child(div().text_size(px(META)).text_color(p.muted).child(counts))
                .when(!pattern.agree.is_empty(), |c| c.child(div().truncate().text_size(px(SMALL)).text_color(p.text2).child(pattern.agree.clone()))),
        )
        .children(updated.map(|u| div().flex_none().text_size(px(META)).text_color(p.muted).child(u)))
        .child(icon(Icon::ChevronRight, 14., p.muted))
        .on_click(cx.listener(move |this, _, _, cx| this.refs_open_pattern(&name, cx)))
        .into_any_element()
}

fn shared_chip(shell: &Shell, name: &str) -> impl IntoElement + use<> {
    let p = shell.palette;
    div().id(SharedString::from(format!("design-refs-shared-{name}"))).flex_none().h(px(18.)).px(px(6.)).flex().items_center().rounded(px(9.)).bg(p.wash).border_1().border_color(p.hair).text_size(px(10.5)).text_color(p.text2).child(t!("design.refs_shared")).tip(p, t!("design.refs_shared_tip").to_owned(), None)
}

/// Where the designer looks (`sources.md`), folded under the list.
fn sources(shell: &Shell, list: &[RefSource], cx: &mut Context<Shell>) -> Option<AnyElement> {
    if list.is_empty() {
        return None;
    }
    let p = shell.palette;
    let open = shell.design.refs.sources_open;
    let rows = open.then(|| {
        v_flex()
            .gap_1p5()
            .pl(px(20.))
            .children(list.iter().enumerate().map(|(i, s)| {
                let url = s.url.clone().filter(|u| refshelf::safe_web(u));
                let detail = [(!s.good_for.is_empty()).then(|| format!("{}: {}", t!("design.refs_good_for"), s.good_for)), (!s.fits.is_empty()).then(|| s.fits.clone())].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                h_flex()
                    .gap_2()
                    .items_baseline()
                    .child(div().flex_none().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(s.name.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(META)).text_color(p.muted).child(detail))
                    .children(url.map(|u| {
                        ui::button(p, SharedString::from(format!("design-refs-source-{i}")), ui::Kind::Ghost, t!("design.refs_open_source")).icon(Icon::External).on_click(cx.listener(move |this, _, window, cx| this.refs_open_source(&u, window, cx)))
                    }))
            }))
    });
    Some(
        v_flex()
            .w_full()
            .pt_4()
            .gap_2()
            .child(
                h_flex()
                    .id("design-refs-sources")
                    .gap_1p5()
                    .items_center()
                    .cursor_pointer()
                    .text_size(px(SMALL))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.text2)
                    .child(icon(if open { Icon::ChevronDown } else { Icon::ChevronRight }, 12., p.muted))
                    .child(t!("design.refs_sources", count = list.len()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.design.refs.sources_open = !this.design.refs.sources_open;
                        cx.notify();
                    })),
            )
            .children(rows)
            .into_any_element(),
    )
}

// ── a pattern ──

fn detail(shell: &Shell, pattern: &RefPattern, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui_ = &shell.design.refs;
    let cards: Vec<Entry> = refshelf::entries(pattern).into_iter().filter(|e| refshelf::entry_matches(pattern, e, &ui_.query)).collect();
    let mut at = 0;
    let mut grid = h_flex().w_full().flex_wrap().gap_4().items_start();
    for entry in &cards {
        let place = entry.picture.map(|_| {
            at += 1;
            at - 1
        });
        grid = grid.child(card(shell, pattern, entry, place, cx));
    }
    let hidden = (pattern.pictures_total as usize).saturating_sub(pattern.pictures.len());
    let updated = (pattern.updated > 0).then(|| t!("design.refs_updated", when = crate::activity::ago(shell.model.unix.saturating_sub(pattern.updated))));
    let meta = [Some(t!("design.refs_pictures", count = pattern.pictures_total as usize)), Some(t!("design.refs_notes", count = pattern.notes.len())), updated].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    v_flex()
        .w_full()
        .gap_4()
        .child(
            h_flex().child(
                ui::button(p, "design-refs-back", ui::Kind::Ghost, t!("design.refs_back")).icon(Icon::ArrowLeft).tip(t!("design.refs_back_tip"), Some("Esc")).on_click(cx.listener(|this, _, _, cx| this.refs_back(cx))),
            ),
        )
        .child(
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().text_size(px(20.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(refshelf::title(&pattern.name)))
                        .when(pattern.shared, |r| r.child(shared_chip(shell, &pattern.name))),
                )
                .child(div().text_size(px(META)).text_color(p.muted).child(meta)),
        )
        .when(!pattern.agree.is_empty(), |c| {
            c.child(
                v_flex()
                    .gap_1()
                    .p_3()
                    .rounded(px(R_LG))
                    .bg(p.raised)
                    .border_1()
                    .border_color(p.hair)
                    .child(div().text_size(px(META)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("design.refs_agree")))
                    .child(div().text_size(px(BODY)).line_height(px(BODY_LINE)).text_color(p.text).child(pattern.agree.clone())),
            )
        })
        .child(grid)
        .when(cards.is_empty(), |c| c.child(nothing_found(shell, &ui_.query)))
        .when(hidden > 0, |c| c.child(div().text_size(px(META)).text_color(p.muted).child(t!("design.refs_more_pictures", count = hidden))))
        .children(raw_notes(shell, pattern, cx))
        .into_any_element()
}

/// One card: the picture (big on a click), where it was seen, what to take, leave and how it fits.
fn card(shell: &Shell, pattern: &RefPattern, entry: &Entry, place: Option<usize>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let note = entry.note.and_then(|n| pattern.notes.get(n));
    let picture = entry.picture.and_then(|i| pattern.pictures.get(i));
    let key = format!("{}-{}-{}", pattern.name, entry.picture.map_or(-1, |i| i as i64), entry.note.map_or(-1, |n| n as i64));
    let shot = picture.map(|pic| {
        let at = place.unwrap_or(0);
        div()
            .id(SharedString::from(format!("design-refs-shot-{key}")))
            .w_full()
            .h(px(SHOT_H))
            .flex_none()
            .bg(p.wash)
            .border_b_1()
            .border_color(p.hair)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .p_2()
            .child(thumb(shell, &pic.path, false))
            .on_click(cx.listener(move |this, _, _, cx| this.refs_view(Some(at), cx)))
    });
    let title = note.map(|n| n.title.clone()).filter(|t| !t.is_empty()).or_else(|| picture.map(|pic| pic.name.clone())).unwrap_or_default();
    let meta = note.map(|n| [n.host.clone(), n.source.clone(), n.date.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ")).unwrap_or_default();
    let url = note.and_then(|n| n.url.clone()).filter(|u| refshelf::safe_web(u));
    let line = |label: String, text: &str| {
        (!text.is_empty()).then(|| {
            h_flex()
                .gap_2()
                .items_start()
                .child(div().flex_none().w(px(38.)).pt(px(1.)).text_size(px(META)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(label))
                .child(div().flex_1().min_w_0().text_size(px(SMALL)).line_height(px(18.)).text_color(p.text2).child(text.to_owned()))
        })
    };
    v_flex()
        .id(SharedString::from(format!("design-refs-card-{key}")))
        .w(px(CARD_W))
        .flex_none()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .overflow_hidden()
        .children(shot)
        .child(
            v_flex()
                .p_3()
                .gap_2()
                .child(div().text_size(px(SMALL + 0.5)).line_height(px(19.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title))
                .when(!meta.is_empty() || url.is_some(), |c| {
                    c.child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().min_w_0().truncate().font_family(MONO).text_size(px(11.)).text_color(p.muted).child(meta))
                            .children(url.map(|u| {
                                let host = note.and_then(|n| n.host.clone()).unwrap_or_default();
                                ui::button(p, SharedString::from(format!("design-refs-open-{key}")), ui::Kind::Ghost, t!("design.refs_open_source"))
                                    .icon(Icon::External)
                                    .tip(t!("design.refs_open_source_tip", host = host), None)
                                    .on_click(cx.listener(move |this, _, window, cx| this.refs_open_source(&u, window, cx)))
                            })),
                    )
                })
                .children(note.and_then(|n| line(t!("design.refs_take").to_string(), &n.take)))
                .children(note.and_then(|n| line(t!("design.refs_leave").to_string(), &n.leave)))
                .children(note.and_then(|n| line(t!("design.refs_fit").to_string(), &n.fit))),
        )
        .into_any_element()
}

/// `notes.md` as written, folded: for what the cards read differently.
fn raw_notes(shell: &Shell, pattern: &RefPattern, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if pattern.raw.trim().is_empty() {
        return None;
    }
    let p = shell.palette;
    let open = shell.design.refs.raw_open;
    Some(
        v_flex()
            .w_full()
            .pt_2()
            .gap_2()
            .child(
                h_flex()
                    .id("design-refs-raw")
                    .gap_1p5()
                    .items_center()
                    .cursor_pointer()
                    .text_size(px(SMALL))
                    .text_color(p.text2)
                    .child(icon(if open { Icon::ChevronDown } else { Icon::ChevronRight }, 12., p.muted))
                    .child(div().font_family(MONO).child("notes.md"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.design.refs.raw_open = !this.design.refs.raw_open;
                        cx.notify();
                    })),
            )
            .when(open, |c| c.child(div().w_full().p_3().rounded(px(R_MD)).bg(p.wash).border_1().border_color(p.hair).font_family(MONO).text_size(px(11.5)).line_height(px(17.)).text_color(p.text2).child(pattern.raw.clone())))
            .into_any_element(),
    )
}

// ── nothing yet ──

fn nothing_found(shell: &Shell, query: &str) -> AnyElement {
    let p = shell.palette;
    div().w_full().py(px(60.)).flex().justify_center().text_size(px(SMALL)).text_color(p.muted).child(t!("design.refs_none_match", query = query)).into_any_element()
}

/// None yet: what references are, and where the designer keeps them.
fn empty(shell: &Shell, folder: &str, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let folder = if folder.is_empty() { shell.design.folder.clone() } else { folder.to_owned() };
    let point = |text: String| h_flex().gap_2().items_start().child(div().mt(px(9.)).size(px(4.)).flex_none().rounded_full().bg(p.muted)).child(div().flex_1().child(text));
    let dir = folder.clone();
    v_flex()
        .id("design-refs-empty")
        .w_full()
        .pt(px(56.))
        .items_center()
        .child(
            motion::rise(
                v_flex()
                    .max_w(px(520.))
                    .gap_4()
                    .child(div().size(px(44.)).rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).flex().items_center().justify_center().child(icon(Icon::Eye, 22., p.text2)))
                    .child(div().text_size(px(20.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("design.refs_empty_title")))
                    .child(div().text_size(px(BODY)).line_height(px(BODY_LINE)).text_color(p.text2).child(t!("design.refs_empty_text")))
                    .child(
                        v_flex()
                            .gap_1()
                            .text_size(px(SMALL))
                            .line_height(px(22.))
                            .text_color(p.text2)
                            .child(point(t!("design.refs_empty_pictures").into()))
                            .child(point(t!("design.refs_empty_notes").into()))
                            .child(point(t!("design.refs_empty_before").into())),
                    )
                    .when(!folder.is_empty(), |d| {
                        d.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .text_size(px(META))
                                .text_color(p.muted)
                                .child(t!("design.lives_in"))
                                .child(div().min_w_0().truncate().font_family(MONO).text_color(p.text2).child(super::design::short_home(&folder)))
                                .child(
                                    ui::button(p, "design-refs-folder", ui::Kind::Ghost, if shell.endpoint.is_remote() { t!("design.refs_copy_path") } else { t!("design.refs_open_folder") })
                                        .on_click(cx.listener(move |this, _, _, cx| this.design_open_folder(&dir, cx))),
                                ),
                        )
                    }),
                "design-refs-empty-in",
            ),
        )
        .into_any_element()
}

// ── what the person does ──

impl Shell {
    /// The words typed in the search.
    pub fn refs_query(&self, cx: &App) -> String {
        self.refs_search.read(cx).value().trim().to_owned()
    }

    /// The search was typed in: asked of the daemon once typing rests.
    pub fn refs_typed(&mut self, cx: &mut Context<Self>) {
        let typed = self.refs_query(cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_AFTER).await;
            let _ = this.update(cx, |this, cx| {
                if this.refs_query(cx) == typed && this.design.refs.search(&typed) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub fn refs_open_pattern(&mut self, name: &str, cx: &mut Context<Self>) {
        self.design.refs.pattern = Some(name.to_owned());
        self.design.refs.viewer = None;
        self.design.refs.raw_open = false;
        self.design.refs.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    /// Back to the patterns.
    pub fn refs_back(&mut self, cx: &mut Context<Self>) {
        self.design.refs.pattern = None;
        self.design.refs.viewer = None;
        self.design.refs.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    /// A picture big (its place among the pattern's cards), or closed.
    pub fn refs_view(&mut self, at: Option<usize>, cx: &mut Context<Self>) {
        self.design.refs.viewer = at;
        cx.notify();
    }

    /// Read again: a designer may have added some.
    pub fn refs_refresh(&mut self, cx: &mut Context<Self>) {
        self.design.refs.dirty = true;
        cx.notify();
    }

    pub fn refs_more(&mut self, cx: &mut Context<Self>) {
        let project = self.design.source_project();
        if let Some(ask) = self.design.refs.more(&self.model.squad, project.as_deref()) {
            self.send_command(ask);
        }
        cx.notify();
    }

    /// A source opened where the person is — the browser they chose, on this Mac, not the daemon's.
    pub fn refs_open_source(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if refshelf::safe_web(url) {
            self.open_link(url, false, window, cx);
        }
    }

    /// The daemon's page of references.
    #[allow(clippy::too_many_arguments)]
    pub fn refs_arrived(&mut self, squad: String, project: Option<String>, query: String, offset: u32, total: u32, patterns: Vec<RefPattern>, sources: Vec<RefSource>, folder: String) -> bool {
        if squad != self.model.squad {
            return false;
        }
        let on_screen = self.design.source_project();
        self.design.refs.arrived(on_screen.as_deref(), project, query, offset, total, patterns, sources, folder)
    }

    /// Each draw, while the tab is on show: the references asked for when
    /// they are not here yet, and the pictures on screen.
    pub fn refs_prepare(&mut self, cx: &mut Context<Self>) {
        if !self.design.loaded || self.model.squad.is_empty() {
            return;
        }
        let project = self.design.source_project();
        // What is here is another folder's (the source changed under it).
        if self.design.refs.answer.as_ref().is_some_and(|a| a.project != project) {
            self.design.refs.forget();
        }
        if let Some(ask) = self.design.refs.wanted(&self.model.squad, project.as_deref()) {
            self.send_command(ask);
        }
        let wanted = {
            let ui_ = &self.design.refs;
            let mut shots = self.shots.borrow_mut();
            match ui_.open() {
                Some(pattern) => {
                    for pic in &pattern.pictures {
                        let _ = shots.look(&pic.path, Size::Thumb);
                    }
                    if let Some(v) = ui_.viewing() {
                        let _ = shots.look(&v.path, Size::Full);
                    }
                }
                None => {
                    for pattern in ui_.answer.iter().flat_map(|a| &a.patterns) {
                        if let Some(c) = refshelf::cover(pattern) {
                            let _ = shots.look(&c.path, Size::Thumb);
                        }
                    }
                }
            }
            shots.has_wanted() && !shots.scheduled
        };
        if wanted {
            self.load_shots(cx);
        }
    }
}
