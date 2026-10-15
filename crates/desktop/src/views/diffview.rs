//! The diff, as a place to review (D): the files at the left, the lines at
//! the right with a cursor; ]/[ walk files, n/p hunks, c leaves a comment on
//! the line, and R sends them all with the words as one request for changes.
//! A accepts. The bar says where the review stands in 나에게: `검토 2/4`.

use equipe_core::t;
use std::collections::HashSet;

use equipe_core::protocol::{TicketId, TicketStatus};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::{Flow, btn, kbd};
use crate::diffs::{self, Kind};
use crate::state::{Need, Section};
use crate::theme::{BODY, MONO, Palette, SMALL};
use crate::Shell;

const LINE: f32 = 20.;

/// A comment left on a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub path: String,
    pub line: Option<u32>,
    pub text: String,
}

/// The diff open on screen.
pub struct DiffUi {
    pub ticket: TicketId,
    pub branch: String,
    /// `None` while git is asked.
    pub files: Option<Vec<diffs::File>>,
    pub patch: String,
    pub error: Option<String>,
    pub current: usize,
    folded: HashSet<usize>,
    /// What the list shows: `(file, line)`; a file's own row has `usize::MAX`.
    flat: Vec<(usize, usize)>,
    /// The row the cursor is on, in `flat`.
    pub cursor: usize,
    scroll: UniformListScrollHandle,
    pub comments: Vec<Comment>,
    pub comment: Entity<InputState>,
    /// The line a comment is being written for.
    pub commenting: Option<(String, Option<u32>)>,
    _typed: Subscription,
}

impl DiffUi {
    pub fn new(ticket: TicketId, branch: String, window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let comment = cx.new(|cx| InputState::new(window, cx).placeholder(t!("diff.comment_placeholder")));
        let typed = cx.subscribe_in(&comment, window, |this, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let text = input.read(cx).value().trim().to_owned();
                if let Some(d) = this.board.diff.as_mut()
                    && let Some((path, line)) = d.commenting.take()
                    && !text.is_empty()
                {
                    d.comments.push(Comment { path, line, text });
                }
                window.blur(cx);
                cx.notify();
            }
        });
        Self {
            ticket,
            branch,
            files: None,
            patch: String::new(),
            error: None,
            current: 0,
            folded: HashSet::new(),
            flat: Vec::new(),
            cursor: 0,
            scroll: UniformListScrollHandle::new(),
            comments: Vec::new(),
            comment,
            commenting: None,
            _typed: typed,
        }
    }

    pub fn loaded(&mut self, text: Result<String, String>) {
        match text {
            Ok(patch) => {
                let files = diffs::parse(&patch);
                self.folded = if diffs::total_lines(&files) > diffs::BIG { (1..files.len()).collect() } else { HashSet::new() };
                self.files = Some(files);
                self.patch = patch;
                self.rebuild();
            }
            Err(why) => self.error = Some(why),
        }
    }

    fn rebuild(&mut self) {
        self.flat.clear();
        for (i, file) in self.files.iter().flatten().enumerate() {
            self.flat.push((i, usize::MAX));
            if !self.folded.contains(&i) {
                self.flat.extend((0..file.lines.len()).map(|l| (i, l)));
            }
        }
        self.cursor = self.cursor.min(self.flat.len().saturating_sub(1));
    }

    /// Esc inside: a comment being written goes first. True when taken.
    pub fn escape(&mut self) -> bool {
        self.commenting.take().is_some()
    }

    /// The comments, as the words a request for changes carries.
    pub fn comments_text(&self) -> String {
        comments_text(&self.comments)
    }

    fn move_cursor(&mut self, to: usize) {
        if self.flat.is_empty() {
            return;
        }
        self.cursor = to.min(self.flat.len() - 1);
        self.current = self.flat[self.cursor].0;
        self.scroll.scroll_to_item(self.cursor, ScrollStrategy::Center);
    }

    /// The next (or previous) row matching `hit`, from the cursor.
    fn seek(&mut self, forward: bool, hit: impl Fn(&Self, usize, usize) -> bool) {
        let n = self.flat.len();
        let found = if forward { (self.cursor + 1..n).find(|&i| hit(self, self.flat[i].0, self.flat[i].1)) } else { (0..self.cursor).rev().find(|&i| hit(self, self.flat[i].0, self.flat[i].1)) };
        if let Some(i) = found {
            self.move_cursor(i);
        }
    }

    fn is_hunk(&self, f: usize, l: usize) -> bool {
        l != usize::MAX && self.files.as_ref().is_some_and(|files| files[f].lines[l].kind == Kind::Hunk)
    }

    /// Where the cursor is, for a comment: the file, and the new (or old) line number.
    fn spot(&self) -> Option<(String, Option<u32>)> {
        let (f, l) = *self.flat.get(self.cursor)?;
        let file = &self.files.as_ref()?[f];
        let line = (l != usize::MAX).then(|| file.lines[l].new.or(file.lines[l].old)).flatten();
        Some((file.path.clone(), line))
    }

    fn goto_file(&mut self, file: usize) {
        self.current = file;
        self.folded.remove(&file);
        self.rebuild();
        if let Some(at) = self.flat.iter().position(|(f, l)| *f == file && *l == usize::MAX) {
            self.cursor = at;
            self.scroll.scroll_to_item(at, ScrollStrategy::Top);
        }
    }
}

impl Shell {
    /// The reviews in 나에게 with a branch to look at, and where this one is among them.
    fn reviews(&self) -> Vec<TicketId> {
        self.model
            .needs_you()
            .into_iter()
            .filter(|(s, _)| *s == Section::Review)
            .filter_map(|(_, n)| if let Need::Ticket(id) = n { Some(id) } else { None })
            .filter(|id| self.model.tickets.iter().any(|t| t.id == *id && t.branch.is_some()))
            .collect()
    }

    /// The diff of another review: the one `by` away in 나에게.
    fn diff_review(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let list = self.reviews();
        let Some(d) = &self.board.diff else { return };
        let at = list.iter().position(|id| *id == d.ticket).map_or(0, |i| i as isize + by);
        if let Some(&next) = usize::try_from(at).ok().and_then(|i| list.get(i)) {
            self.board.view.selected = Some(Need::Ticket(next));
            self.board_open_diff(window, cx);
        }
    }

    /// The diff's keys. True when taken.
    pub fn on_diff_key(&mut self, key: &str, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let plain = !(m.control || m.alt || m.platform || m.shift || m.function);
        if matches!(self.board.flow, Some(Flow::Reply(_))) {
            return false;
        }
        let Some(d) = self.board.diff.as_mut() else { return false };
        match key {
            "down" | "j" if plain => d.move_cursor(d.cursor + 1),
            "up" | "k" if plain => d.move_cursor(d.cursor.saturating_sub(1)),
            "down" if m.alt => {
                self.diff_review(1, window, cx);
                return true;
            }
            "up" if m.alt => {
                self.diff_review(-1, window, cx);
                return true;
            }
            "]" => {
                let next = (d.current + 1).min(d.files.as_ref().map_or(0, |f| f.len().saturating_sub(1)));
                d.goto_file(next);
            }
            "[" => {
                let prev = d.current.saturating_sub(1);
                d.goto_file(prev);
            }
            "n" if plain => d.seek(true, |d, f, l| d.is_hunk(f, l)),
            "p" if plain => d.seek(false, |d, f, l| d.is_hunk(f, l)),
            "c" if plain => {
                if let Some(spot) = d.spot() {
                    d.commenting = Some(spot);
                    d.comment.update(cx, |s, cx| {
                        s.set_value(String::new(), window, cx);
                        s.focus(window, cx);
                    });
                }
            }
            "r" if plain => {
                let id = d.ticket;
                self.board.view.selected = Some(Need::Ticket(id));
                self.board.flow = Some(Flow::Reply(id));
                self.board.reply.update(cx, |s, cx| {
                    s.set_value(String::new(), window, cx);
                    s.focus(window, cx);
                });
            }
            "a" if plain => {
                let id = d.ticket;
                self.board.diff = None;
                self.board.view.selected = Some(Need::Ticket(id));
                self.board_yes(cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn diff_fold(&mut self, file: usize, cx: &mut Context<Self>) {
        if let Some(d) = self.board.diff.as_mut() {
            if !d.folded.remove(&file) {
                d.folded.insert(file);
            }
            d.rebuild();
        }
        cx.notify();
    }
}

pub fn render(shell: &Shell, diff: &DiffUi, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let t = model.tickets.iter().find(|t| t.id == diff.ticket);
    let can = shell.can_act();
    let id = diff.ticket;
    let review = t.is_some_and(|t| t.status == TicketStatus::Review);
    let patch = diff.patch.clone();
    let reviews = shell.reviews();
    let place = reviews.iter().position(|r| *r == id).map(|i| t!("diff.review_place", at = i + 1, of = reviews.len()));
    let stat = diff.files.as_ref().map(|f| {
        let (a, r) = f.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
        t!("diff.stat", n = f.len(), a = a, r = r)
    });
    let bar = h_flex()
        .h(px(48.))
        .flex_none()
        .px_4()
        .gap_3()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(btn(p, "diff-back", "←", Some("Esc"), false, true).on_click(cx.listener(|this, _, _, cx| {
            this.board.diff = None;
            cx.notify();
        })))
        .child(div().flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{id}")))
        .child(div().min_w_0().flex_1().truncate().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(t.map(|t| t.title.clone()).unwrap_or_default()))
        .child(div().flex_none().text_size(px(SMALL)).text_color(p.muted).child(stat.unwrap_or_default()))
        .children(place.map(|place| h_flex().flex_none().gap_1().items_center().text_size(px(SMALL)).text_color(p.text2).child(place).child(kbd(p, "⌥↑↓"))))
        .child(btn(p, "diff-copy", t!("diff.copy_patch"), None, false, !patch.is_empty()).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(patch.clone()))))
        .children(review.then(|| {
            btn(p, "diff-ask", t!("peek.request_changes"), Some("R"), false, can).on_click(cx.listener(move |this, _, w, cx| {
                this.board.view.selected = Some(Need::Ticket(id));
                this.board.flow = Some(Flow::Reply(id));
                this.board.reply.update(cx, |s, cx| s.focus(w, cx));
                cx.notify();
            }))
        }))
        .children(review.then(|| {
            btn(p, "diff-accept", t!("peek.accept"), Some("A"), true, can).on_click(cx.listener(move |this, _, _, cx| {
                this.board.diff = None;
                this.board.view.selected = Some(Need::Ticket(id));
                this.board_yes(cx);
            }))
        }));
    let body: AnyElement = match (&diff.files, &diff.error) {
        (_, Some(why)) => div().p_6().text_size(px(BODY)).text_color(p.muted).child(t!("diff.failed", why = why)).into_any_element(),
        (None, _) => div().p_6().text_size(px(BODY)).text_color(p.muted).child(t!("diff.loading")).into_any_element(),
        (Some(files), _) if files.is_empty() => div().p_6().text_size(px(BODY)).text_color(p.muted).child(t!("diff.none")).into_any_element(),
        (Some(files), _) => {
            let tree = v_flex()
                .id("diff-files")
                .w(px(260.))
                .flex_none()
                .h_full()
                .overflow_y_scroll()
                .border_r_1()
                .border_color(p.hair)
                .p_2()
                .child(h_flex().px_2().pb_1().gap_2().text_size(px(11.)).text_color(p.muted).child(t!("diff.files")).child(div().flex_1()).child(kbd(p, "[")).child(kbd(p, "]")))
                .children(files.iter().enumerate().map(|(i, f)| {
                    let commented = diff.comments.iter().filter(|c| c.path == f.path).count();
                    h_flex()
                        .id(("diff-file", i))
                        .gap_2()
                        .px_2()
                        .h(px(26.))
                        .items_center()
                        .rounded(px(6.))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .when(diff.current == i, |d| d.bg(p.wash))
                        .hover(move |s| s.bg(p.wash))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(d) = this.board.diff.as_mut() {
                                d.goto_file(i);
                            }
                            cx.notify();
                        }))
                        .child(div().flex_1().min_w_0().truncate().font_family(MONO).text_color(if diff.current == i { p.text } else { p.text2 }).child(f.path.clone()))
                        .when(commented > 0, |r| r.child(div().text_color(p.accent).child(format!("● {commented}"))))
                        .child(div().flex_none().text_color(p.muted).child(format!("+{} −{}", f.added, f.removed)))
                }));
            let count = diff.flat.len();
            let lines = uniform_list("diff-lines", count, cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                let p = this.palette;
                let Some(d) = this.board.diff.as_ref() else { return Vec::new() };
                let Some(files) = d.files.as_ref() else { return Vec::new() };
                range.filter_map(|ix| d.flat.get(ix).copied().map(|(f, l)| diff_row(p, files, d, ix, f, l, cx))).collect()
            }))
            .track_scroll(&diff.scroll)
            .flex_1()
            .min_w_0()
            .h_full();
            h_flex().flex_1().min_h_0().child(tree).child(lines).into_any_element()
        }
    };
    // What is being written: a comment on the line, the request for changes.
    let commenting = diff.commenting.clone().map(|(path, line)| {
        h_flex()
            .flex_none()
            .px_4()
            .py_2()
            .gap_3()
            .items_center()
            .border_t_1()
            .border_color(p.hair)
            .bg(p.raised)
            .child(div().flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(match line {
                Some(n) => format!("{path}:{n}"),
                None => path,
            }))
            .child(div().flex_1().px_2().py_1().rounded(px(6.)).bg(p.code_bg).border_1().border_color(p.hair).text_size(px(SMALL)).child(Input::new(&diff.comment).appearance(false).bordered(false)))
            .child(kbd(p, "⏎"))
    });
    let replying = matches!(shell.board.flow, Some(Flow::Reply(t)) if t == id);
    let comments = (!diff.comments.is_empty() || replying).then(|| {
        v_flex()
            .flex_none()
            .max_h(px(260.))
            .px_4()
            .py_3()
            .gap_2()
            .border_t_1()
            .border_color(p.hair)
            .bg(p.raised)
            .child(div().text_size(px(11.)).text_color(p.muted).child(t!("diff.comments", n = diff.comments.len())))
            .children(diff.comments.iter().enumerate().map(|(i, c)| {
                h_flex()
                    .gap_3()
                    .text_size(px(SMALL))
                    .child(div().flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(match c.line {
                        Some(n) => format!("{}:{n}", c.path.rsplit('/').next().unwrap_or(&c.path)),
                        None => c.path.clone(),
                    }))
                    .child(div().flex_1().min_w_0().text_color(p.text).child(c.text.clone()))
                    .child(div().id(("drop-comment", i)).flex_none().text_color(p.muted).cursor_pointer().child("×").on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(d) = this.board.diff.as_mut()
                            && i < d.comments.len()
                        {
                            d.comments.remove(i);
                        }
                        cx.notify();
                    })))
            }))
            .when(replying, |c| {
                c.child(div().w_full().px_3().py_2().rounded(px(8.)).bg(p.code_bg).border_1().border_color(p.hair).text_size(px(SMALL)).child(Textarea::new(&shell.board.reply).appearance(false).bordered(false))).child(
                    h_flex().gap_2().child(div().flex_1()).child(btn(p, "diff-reply-cancel", t!("peek.cancel"), Some("Esc"), false, true).on_click(cx.listener(|this, _, _, cx| {
                        this.board.flow = None;
                        cx.notify();
                    }))).child(btn(p, "diff-reply-send", t!("peek.send_back"), Some("⌘⏎"), true, can).on_click(cx.listener(|this, _, w, cx| this.board_send_reply(w, cx)))),
                )
            })
            .when(!replying, |c| c.child(h_flex().gap_2().text_size(px(12.)).text_color(p.muted).child(t!("diff.comments_hint"))))
    });
    let hint = h_flex()
        .h(px(28.))
        .flex_none()
        .px_4()
        .gap_3()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .text_size(px(11.))
        .text_color(p.muted)
        .child(h_flex().gap_1().items_center().child(kbd(p, "J")).child(kbd(p, "K")).child(t!("diff.hint_lines")))
        .child(h_flex().gap_1().items_center().child(kbd(p, "N")).child(kbd(p, "P")).child(t!("diff.hint_hunks")))
        .child(h_flex().gap_1().items_center().child(kbd(p, "C")).child(t!("diff.hint_comment")))
        .child(h_flex().gap_1().items_center().child(kbd(p, "R")).child(t!("diff.hint_send")));
    v_flex().flex_1().min_h_0().child(bar).child(body).children(commenting).children(comments).child(hint)
}

fn diff_row(p: Palette, files: &[diffs::File], d: &DiffUi, ix: usize, f: usize, l: usize, cx: &mut Context<Shell>) -> AnyElement {
    let file = &files[f];
    let at_cursor = ix == d.cursor;
    if l == usize::MAX {
        let folded = d.folded.contains(&f);
        return h_flex()
            .id(("diff-head", f))
            .w_full()
            .h(px(LINE + 8.))
            .px_4()
            .gap_2()
            .items_center()
            .bg(if at_cursor { p.accent_bg } else { p.raised })
            .border_b_1()
            .border_color(p.hair)
            .font_family(MONO)
            .text_size(px(12.))
            .text_color(p.text)
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.diff_fold(f, cx)))
            .child(if folded { "▸" } else { "▾" })
            .child(file.path.clone())
            .child(div().text_color(p.muted).font_weight(FontWeight::NORMAL).child(format!("+{} −{}", file.added, file.removed)))
            .into_any_element();
    }
    let line = &file.lines[l];
    let number = |n: Option<u32>| div().w(px(40.)).flex_none().text_align(TextAlign::Right).text_color(p.muted).child(n.map(|n| n.to_string()).unwrap_or_default());
    let (glyph, bg, color) = match line.kind {
        Kind::Add => ("+", Some(p.accent_bg), p.text),
        Kind::Del => ("−", Some(p.wash), p.muted),
        Kind::Hunk => ("", Some(p.wash), p.muted),
        Kind::Context => (" ", None, p.text2),
        Kind::Note => ("", None, p.muted),
    };
    let n = line.new.or(line.old);
    let commented = n.is_some() && d.comments.iter().any(|c| c.path == file.path && c.line == n);
    h_flex()
        .id(("diff-line", ix))
        .w_full()
        .h(px(LINE))
        .gap_2()
        .pr_2()
        .items_center()
        .font_family(MONO)
        .text_size(px(12.))
        .when_some(bg, |d, bg| d.bg(bg))
        .when(at_cursor, |d| d.bg(p.hair))
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(d) = this.board.diff.as_mut() {
                d.cursor = ix;
                d.current = f;
            }
            cx.notify();
        }))
        .child(div().w(px(3.)).h_full().flex_none().when(at_cursor, |d| d.bg(p.accent)))
        .child(div().w(px(10.)).flex_none().text_color(p.accent).child(if commented { "●" } else { "" }))
        .child(number(line.old))
        .child(number(line.new))
        .child(div().w(px(10.)).flex_none().text_color(color).child(glyph))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_color(color).when(line.kind == Kind::Del, |d| d.line_through()).child(if line.kind == Kind::Hunk || line.kind == Kind::Note { line.text.clone() } else { line.text.replace('\t', "    ") }))
        .into_any_element()
}

/// Comments as the words a request for changes carries: a line a place.
pub fn comments_text(comments: &[Comment]) -> String {
    comments
        .iter()
        .map(|c| match c.line {
            Some(n) => format!("- {}:{n} — {}", c.path, c.text),
            None => format!("- {} — {}", c.path, c.text),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::{Comment, comments_text};

    #[test]
    fn comments_read_as_a_list_of_places() {
        let comments = [Comment { path: "src/a.rs".into(), line: Some(12), text: "이름을 바꿔 주세요".into() }, Comment { path: "b.md".into(), line: None, text: "빠졌어요".into() }];
        assert_eq!(comments_text(&comments), "- src/a.rs:12 — 이름을 바꿔 주세요\n- b.md — 빠졌어요");
    }
}
