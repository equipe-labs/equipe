//! Briefs and plans, all in one place (`docs.rs` has the rules): every
//! project's and the squad's own, newest first, narrowed by whose they are,
//! their kind and how they stand, found by their title and words. Above the
//! list, the ones saved under more than one project, side by side — "these
//! differ" when their words do — for the person to keep one; below it, the
//! older copies still sitting in a project's repository, told and left alone.
//! A row opens its doc beside the screen. Each project's page keeps its own
//! tab of them.

use equipe_core::protocol::briefs::{DocCopies, DocKind, ProjectDoc};
use equipe_core::t;
use gpui_kit::component::input::Input;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::project::{copies_badge, date_cell, status_chip};
use crate::board::{DocTone, doc_kind, doc_status};
use crate::docs::{Filter, Whose, doc_at, listed, wheres};
use crate::icons::{Icon, icon};
use crate::theme::{MONO, R_MD, SMALL};
use crate::tips::TipExt as _;
use crate::{Screen, Shell, ui};

/// The screen's column, at its widest.
const COLUMN: f32 = 860.;
/// How long typing rests before the daemon is asked to search the words.
const SEARCH_AFTER: std::time::Duration = std::time::Duration::from_millis(250);

/// Where a doc is kept, in words: its project, or the squad's own.
pub fn where_of(project: Option<&str>) -> String {
    project.map_or_else(|| t!("docs.squad_own").to_owned(), str::to_owned)
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let header = ui::title_bar(h_flex())
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_4()
        .pl(px(shell.header_inset(16., cx)))
        .gap_3()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(div().flex_none().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("docs.title")))
        .child(div().min_w_0().truncate().text_size(px(SMALL)).text_color(p.muted).child(t!("docs.subtitle")));
    let query = shell.docs_query(cx);
    let shown = listed(&shell.docs, &shell.shelf.filter, &query, shell.shelf.found_for(&query));
    let searching = !query.is_empty() && shell.shelf.found_for(&query).is_none();
    let mut column = v_flex().w_full().gap_3();
    column = column.children(kept_banner(shell, cx));
    if !shell.shelf.copies.is_empty() {
        column = column.child(copies_part(shell, cx));
    }
    column = column.child(search_bar(shell, cx)).child(filters(shell, cx));
    column = column.child(if shell.docs.is_empty() {
        empty(shell, t!("docs.empty_title"), t!("docs.empty"))
    } else if shown.is_empty() && !searching {
        empty(shell, t!("docs.none_title"), t!("docs.none"))
    } else {
        let rows: Vec<AnyElement> = shown.iter().enumerate().map(|(i, d)| row(shell, i, d, cx)).collect();
        v_flex().w_full().gap_0p5().children(rows).when(searching, |l| l.child(div().pt_1().text_size(px(12.)).text_color(p.muted).child(t!("docs.searching")))).into_any_element()
    });
    column = column.children(in_repos_part(shell));
    column = column.child(div().pt_2().text_size(px(12.)).text_color(p.muted).child(t!("project.docs_private")));
    v_flex()
        .id("docs-page")
        .flex_1()
        .min_h_0()
        .bg(p.bg)
        .child(header)
        .child(div().id("docs-scroll").flex_1().min_h_0().overflow_y_scroll().child(h_flex().w_full().justify_center().child(v_flex().w_full().max_w(px(COLUMN)).px_6().pt_4().pb_10().child(column))))
        .into_any_element()
}

fn empty(shell: &Shell, title: &str, text: &str) -> AnyElement {
    let p = shell.palette;
    v_flex()
        .w_full()
        .py_10()
        .gap_1p5()
        .items_center()
        .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(title.to_owned()))
        .child(div().max_w(px(440.)).text_center().text_size(px(13.)).line_height(px(20.)).text_color(p.muted).child(text.to_owned()))
        .into_any_element()
}

fn search_bar(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let typed = !shell.docs_query(cx).is_empty();
    h_flex()
        .h(px(34.))
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
        .child(div().flex_1().min_w_0().child(Input::new(&shell.docs_search).appearance(false).bordered(false)))
        .when(typed, |bar| {
            bar.child(ui::icon_button(p, "docs-search-clear", Icon::X, t!("docs.search_clear"), None).on_click(cx.listener(|this, _, window, cx| {
                this.docs_search.update(cx, |s, cx| s.set_value(String::new(), window, cx));
                cx.notify();
            })))
        })
        .into_any_element()
}

/// The filters: whose, which kind, how they stand — "All" first in each.
fn filters(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let filter = &shell.shelf.filter;
    let chip = |id: SharedString, label: String, on: bool, pick: Filter, cx: &mut Context<Shell>| {
        ui::button(p, id, ui::Kind::Ghost, label).selected(on).on_click(cx.listener(move |this, _, _, cx| {
            this.shelf.filter = pick.clone();
            cx.notify();
        }))
    };
    let label = |text: &str| div().flex_none().pr_1().text_size(px(12.)).text_color(p.muted).child(text.to_owned());
    let mut whose = h_flex().flex_wrap().gap_1().items_center().child(label(t!("docs.filter_where")));
    whose = whose.child(chip("docs-where-all".into(), t!("docs.filter_all").into(), filter.whose == Whose::All, Filter { whose: Whose::All, ..filter.clone() }, cx));
    for (i, w) in wheres(&shell.docs).into_iter().enumerate() {
        let words = match &w {
            Whose::Project(name) => name.clone(),
            _ => t!("docs.squad_own").into(),
        };
        let on = filter.whose == w;
        whose = whose.child(chip(SharedString::from(format!("docs-where-{i}")), words, on, Filter { whose: w, ..filter.clone() }, cx));
    }
    let mut kinds = h_flex().flex_wrap().gap_1().items_center().child(label(t!("docs.filter_kind")));
    for (id, kind) in [("docs-kind-all", None), ("docs-kind-brief", Some(DocKind::Brief)), ("docs-kind-plan", Some(DocKind::Plan))] {
        let words: String = kind.map_or(t!("docs.filter_all"), kind_plural).into();
        kinds = kinds.child(chip(id.into(), words, filter.kind == kind, Filter { kind, ..filter.clone() }, cx));
    }
    let mut states = h_flex().flex_wrap().gap_1().items_center().child(label(t!("docs.filter_status")));
    for (id, tone, word) in [("docs-status-all", None, ""), ("docs-status-draft", Some(DocTone::Draft), "draft"), ("docs-status-review", Some(DocTone::Review), "in_review"), ("docs-status-settled", Some(DocTone::Settled), "settled"), ("docs-status-dropped", Some(DocTone::Dropped), "dropped")] {
        let words = if tone.is_none() { t!("docs.filter_all").to_owned() } else { doc_status(word).0 };
        states = states.child(chip(id.into(), words, filter.status == tone, Filter { status: tone, ..filter.clone() }, cx));
    }
    v_flex().w_full().gap_1().child(whose).child(h_flex().flex_wrap().gap_x_4().gap_y_1().child(kinds).child(states)).into_any_element()
}

/// Briefs, Plans: a kind's filter.
fn kind_plural(kind: DocKind) -> &'static str {
    match kind {
        DocKind::Brief => t!("docs.filter_briefs"),
        DocKind::Plan => t!("docs.filter_plans"),
    }
}

/// The tickets a doc names: `T-42 T-51`.
pub fn tickets_words(doc: &ProjectDoc) -> Option<String> {
    let ids = doc.ticket_ids();
    (!ids.is_empty()).then(|| ids.iter().map(|id| format!("T-{id}")).collect::<Vec<_>>().join(" "))
}

/// A brief or plan: what it is, its title, whose, the tickets it names, its
/// date and how it stands — opened beside the screen.
fn row(shell: &Shell, i: usize, d: &ProjectDoc, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let path = std::path::PathBuf::from(&d.path);
    let dropped = doc_status(&d.status).1 == DocTone::Dropped;
    let copies = crate::docs::copies_of(&shell.shelf.copies, &d.path).map(|c| c.paths.len());
    h_flex()
        .id(("docs-row", i))
        .w_full()
        .h(px(36.))
        .px_2()
        .ml(px(-8.))
        .gap_2()
        .items_center()
        .rounded(px(R_MD))
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .aria_label(d.title.clone())
        .tip(p, t!("project.doc_open_tip"), None)
        .on_click(cx.listener(move |this, _, _, cx| this.open_doc(path.clone(), cx)))
        .child(div().w(px(44.)).flex_none().truncate().text_size(px(12.)).text_color(p.muted).child(doc_kind(d.kind)))
        .child(div().flex_auto().min_w(px(64.)).truncate().text_size(px(14.)).text_color(if dropped { p.text2 } else { p.text }).child(d.title.clone()))
        .children(copies.map(|n| copies_badge(shell, n)))
        // Whose, and the tickets it names, each in a column of its own so they line up.
        .child(div().flex_none().w(px(96.)).truncate().text_size(px(12.)).text_color(p.text2).child(where_of(d.project.as_deref())))
        .child(div().flex_none().w(px(92.)).truncate().font_family(MONO).text_size(px(12.)).text_color(p.muted).children(tickets_words(d)))
        .children((!d.date.is_empty()).then(|| date_cell(shell, &d.date)))
        .child(h_flex().flex_none().min_w(px(76.)).justify_end().child(status_chip(shell, &d.status)))
        .into_any_element()
}

/// The ones saved under more than one project, each set side by side.
fn copies_part(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let sets: Vec<AnyElement> = shell.shelf.copies.iter().enumerate().map(|(n, set)| copy_set(shell, n, set, cx)).collect();
    v_flex()
        .w_full()
        .gap_2()
        .child(h_flex().gap_1p5().items_center().child(super::board::dot(p.orange, 6.)).child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("docs.copies_title"))).child(div().text_size(px(12.)).text_color(p.muted).child(sets.len().to_string())))
        .children(sets)
        .into_any_element()
}

/// One set of copies: what it is, whether their words differ, and each copy
/// — whose, its date, its file — to open, or to keep.
fn copy_set(shell: &Shell, n: usize, set: &DocCopies, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let keeping = shell.shelf.keeping.clone();
    let badge = if set.differ {
        div().flex_none().h(px(20.)).px(px(7.)).flex().items_center().rounded(px(10.)).bg(Rgba { a: 0.18, ..p.amber }).text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(t!("docs.copies_differ"))
    } else {
        div().flex_none().h(px(20.)).px(px(7.)).flex().items_center().rounded(px(10.)).bg(p.wash).text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(p.text2).child(t!("docs.copies_same"))
    };
    let rows: Vec<AnyElement> = set
        .paths
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let doc = doc_at(&shell.docs, path);
            let name = path.rsplit('/').next().unwrap_or(path).to_owned();
            let (open_at, keep_at) = (std::path::PathBuf::from(path), path.clone());
            let this_one = keeping.as_deref() == Some(path.as_str());
            h_flex()
                .id(SharedString::from(format!("docs-copy-{n}-{i}")))
                .w_full()
                .h(px(36.))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(R_MD))
                .hover(move |s| s.bg(p.wash))
                .child(div().w(px(130.)).flex_none().truncate().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(where_of(doc.and_then(|d| d.project.as_deref()))))
                .child(div().flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(doc.map(|d| d.date.clone()).unwrap_or_default()))
                .child(div().id(SharedString::from(format!("docs-copy-file-{n}-{i}"))).flex_1().min_w_0().truncate().font_family(MONO).text_size(px(12.)).text_color(p.muted).tip(p, path.clone(), None).child(name))
                .child(ui::button(p, SharedString::from(format!("docs-copy-open-{n}-{i}")), ui::Kind::Ghost, t!("docs.copy_open")).on_click(cx.listener(move |this, _, _, cx| this.open_doc(open_at.clone(), cx))))
                .child(
                    ui::button(p, SharedString::from(format!("docs-copy-keep-{n}-{i}")), ui::Kind::Secondary, if this_one { t!("docs.copy_keeping") } else { t!("docs.copy_keep") })
                        .disabled(keeping.is_some())
                        .tip(t!("docs.copy_keep_tip"), None)
                        .on_click(cx.listener(move |this, _, _, cx| this.keep_doc_copy(&keep_at, cx))),
                )
                .into_any_element()
        })
        .collect();
    v_flex()
        .w_full()
        .p_3()
        .gap_1()
        .rounded(px(R_MD))
        .border_1()
        .border_color(p.hair)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().flex_none().text_size(px(12.)).text_color(p.muted).child(doc_kind(set.kind)))
                .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(set.title.clone()))
                .child(badge),
        )
        .child(div().pb_1().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("docs.copies_explain", n = set.paths.len())))
        .children(rows)
        .into_any_element()
}

/// One copy kept: which, how many went, and its undo — until taken back.
fn kept_banner(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let kept = shell.shelf.kept.as_ref()?;
    let whose = where_of(doc_at(&shell.docs, &kept.kept).and_then(|d| d.project.as_deref()));
    let words = if kept.undone { t!("docs.kept_undone").to_owned() } else { t!("docs.kept", project = whose, n = kept.removed.len()).to_owned() };
    Some(
        h_flex()
            .w_full()
            .min_h(px(40.))
            .px_3()
            .gap_2()
            .items_center()
            .rounded(px(R_MD))
            .bg(p.accent_bg)
            .child(icon(Icon::Check, 14., p.accent))
            .child(div().flex_1().min_w_0().text_size(px(13.)).text_color(p.text).child(words))
            .when(!kept.undone && kept.undo.is_some(), |b| b.child(ui::button(p, "docs-kept-undo", ui::Kind::Secondary, t!("docs.kept_undo")).on_click(cx.listener(|this, _, _, cx| this.undo_doc_copy(cx)))))
            .child(ui::icon_button(p, "docs-kept-close", Icon::X, t!("docs.kept_close"), None).on_click(cx.listener(|this, _, _, cx| {
                this.shelf.kept = None;
                cx.notify();
            })))
            .into_any_element(),
    )
}

/// What still sits in the projects' repositories from before, project by
/// project: told, nothing done to it.
fn in_repos_part(shell: &Shell) -> Option<AnyElement> {
    let p = shell.palette;
    if shell.shelf.in_repos.is_empty() {
        return None;
    }
    let mut projects: Vec<&str> = Vec::new();
    for r in &shell.shelf.in_repos {
        if !projects.contains(&r.project.as_str()) {
            projects.push(&r.project);
        }
    }
    Some(
        v_flex()
            .w_full()
            .pt_2()
            .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("docs.in_repos_title")))
            .children(projects.into_iter().filter_map(|project| super::project::in_repo_note(shell, project)))
            .into_any_element(),
    )
}

impl Shell {
    /// The words typed in the docs screen's search.
    pub fn docs_query(&self, cx: &App) -> String {
        self.docs_search.read(cx).value().trim().to_owned()
    }

    /// The docs screen, from the sidebar's "More": every brief and plan, as they are now.
    pub fn open_docs(&mut self, cx: &mut Context<Self>) {
        self.send_command(equipe_core::protocol::Command::ProjectDocs { squad: self.model.squad.clone() });
        self.show(Screen::Docs, cx);
        cx.notify();
    }

    /// A brief or plan a ticket names, opened beside the screen — on the docs
    /// screen when this one has no room for it beside (the work screens).
    pub fn open_doc_of_ticket(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Chat | Screen::Project | Screen::Docs) {
            self.open_docs(cx);
        }
        self.open_doc(path, cx);
    }

    /// The search was typed in: its words asked of the daemon once typing rests.
    pub fn docs_typed(&mut self, cx: &mut Context<Self>) {
        let typed = self.docs_query(cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_AFTER).await;
            let _ = this.update(cx, |this, cx| {
                if this.docs_query(cx) != typed {
                    return;
                }
                let squad = this.model.squad.clone();
                if let Some(ask) = this.shelf.ask(&squad, &typed) {
                    this.send_command(ask);
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// The daemon found these for these words.
    pub fn docs_found(&mut self, squad: String, query: String, paths: Vec<String>) -> bool {
        if squad != self.model.squad {
            return false;
        }
        self.shelf.found = Some((query, paths));
        true
    }

    /// Keep the copy at `path`; the others go from the squad's folder.
    pub fn keep_doc_copy(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(session) = self.model.pm() else { return };
        let squad = self.model.squad.clone();
        if let Some(keep) = self.shelf.keep(&squad, session, path) {
            self.send_command(keep);
        }
        cx.notify();
    }

    /// The daemon kept one.
    pub fn doc_copy_kept(&mut self, squad: String, kept: String, removed: Vec<String>, undo: Option<String>) -> bool {
        if squad != self.model.squad {
            return false;
        }
        self.shelf.on_kept(kept, removed, undo);
        true
    }

    /// Take back what keeping one removed.
    pub fn undo_doc_copy(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.model.pm() else { return };
        let squad = self.model.squad.clone();
        if let Some(undo) = self.shelf.undo(&squad, session) {
            self.send_command(undo);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use equipe_core::protocol::briefs::{DocKind, ProjectDoc};
    use equipe_core::t;

    use super::{kind_plural, tickets_words, where_of};

    fn doc(project: Option<&str>, ticket: Option<u32>, tickets: &[u32]) -> ProjectDoc {
        ProjectDoc { kind: DocKind::Plan, project: project.map(str::to_owned), path: "/h/docs/plans/2026-11-04-x.md".into(), title: "X".into(), date: "2026-11-04".into(), status: "draft".into(), ticket, mode: None, tickets: tickets.to_vec() }
    }

    #[test]
    fn a_row_says_whose_it_is_and_the_tickets_it_names() {
        assert_eq!(where_of(Some("web")), "web");
        assert_eq!(where_of(None), "스쿼드 자체");
        assert_eq!(tickets_words(&doc(None, None, &[42, 51])).as_deref(), Some("T-42 T-51"));
        assert_eq!(tickets_words(&doc(Some("web"), Some(7), &[7, 8])).as_deref(), Some("T-7 T-8"), "each once");
        assert_eq!(tickets_words(&doc(None, None, &[])), None);
    }

    #[test]
    fn the_filters_and_the_chooser_speak_the_persons_words() {
        assert_eq!((kind_plural(DocKind::Brief), kind_plural(DocKind::Plan)), ("브리프", "계획"));
        assert_eq!(t!("docs.copies_explain", n = 2), "2곳에 저장돼 있어요. 하나만 남기면 나머지는 스쿼드 폴더에서 지워지고, 되돌릴 수 있어요. 저장소는 건드리지 않아요.");
        assert_eq!(t!("docs.kept", project = "sf", n = 1), "sf의 사본을 남기고 1개를 스쿼드 폴더에서 지웠어요.");
        assert_eq!(t!("docs.in_repo", project = "web", n = 2), "web 저장소에 예전 사본 2개가 아직 남아 있어요");
    }
}
