//! Agents answer in Markdown; the console draws it — bold as bold, code as
//! code, lists indented, tables aligned — instead of showing the asterisks.
//!
//! Wrapping is done here, not by the terminal, so continuation lines keep
//! their indent and wide characters (Hangul, CJK) count as two columns.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

/// The styles Markdown maps onto.
#[derive(Clone, Copy)]
pub struct Styles {
    pub plain: Style,
    /// Inline and block code.
    pub code: Style,
    /// Rules, quote bars, link targets, code-block bars.
    pub dim: Style,
}

/// `text` as lines no wider than `width`.
pub fn lines(text: &str, width: usize, styles: &Styles) -> Vec<Line<'static>> {
    let mut r = Renderer::new(width.max(10), *styles);
    let text = crate::ui::cell_safe(text);
    for event in Parser::new_ext(&text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH) {
        r.event(event);
    }
    r.flush();
    r.out
}

/// A cell: its text runs and their styles.
type Cell = Vec<(String, Style)>;

#[derive(Default)]
struct Table {
    rows: Vec<Vec<Cell>>,
    row: Vec<Cell>,
    cell: Cell,
}

struct Renderer {
    width: usize,
    styles: Styles,
    out: Vec<Line<'static>>,
    /// The paragraph being built: text runs and their styles.
    runs: Vec<(String, Style)>,
    bold: usize,
    italic: usize,
    strike: usize,
    link: Option<(String, usize)>,
    /// One entry per open list: the next number, or `None` for bullets.
    lists: Vec<Option<u64>>,
    /// The marker of an item whose first line is still to come.
    marker: Option<String>,
    /// Where an item's text starts, for its continuation lines.
    indent: usize,
    /// The indents of the items enclosing this one.
    outer: Vec<usize>,
    quote: usize,
    heading: bool,
    /// A heading was just drawn: the text under it follows without a gap.
    under_heading: bool,
    code_block: bool,
    /// The open fence is a diff: its `+` and `-` lines in green and red.
    diff_block: bool,
    table: Option<Table>,
}

impl Renderer {
    fn new(width: usize, styles: Styles) -> Self {
        Self {
            width,
            styles,
            out: Vec::new(),
            runs: Vec::new(),
            bold: 0,
            italic: 0,
            strike: 0,
            link: None,
            lists: Vec::new(),
            marker: None,
            indent: 0,
            outer: Vec::new(),
            quote: 0,
            heading: false,
            under_heading: false,
            code_block: false,
            diff_block: false,
            table: None,
        }
    }

    fn style(&self) -> Style {
        let mut style = self.styles.plain;
        if self.bold > 0 || self.heading {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.strike > 0 {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if self.link.is_some() {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        style
    }

    /// A blank line between blocks — not before the first, not inside a list.
    fn gap(&mut self) {
        if !self.out.is_empty() && self.lists.is_empty() && self.out.last().is_some_and(|l| l.width() > 0) {
            self.out.push(Line::raw(""));
        }
    }

    fn quote_prefix(&self) -> Vec<Span<'static>> {
        (0..self.quote).map(|_| Span::styled("│ ", self.styles.dim)).collect()
    }

    /// Lay out the paragraph built so far.
    fn flush(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        let runs = std::mem::take(&mut self.runs);
        let mut first = self.quote_prefix();
        match self.marker.take() {
            Some(marker) => {
                first.push(Span::raw(" ".repeat(self.indent - marker.width())));
                first.push(Span::styled(marker, self.styles.dim));
            }
            None => first.push(Span::raw(" ".repeat(self.indent))),
        }
        let mut rest = self.quote_prefix();
        rest.push(Span::raw(" ".repeat(self.indent)));
        let lines = wrap(&runs, first, rest, self.width);
        self.out.extend(lines);
    }

    fn event(&mut self, event: Event<'_>) {
        // Only the paragraph right under a heading sits tight against it.
        if !matches!(event, Event::Start(Tag::Paragraph) | Event::End(TagEnd::Heading(_))) {
            self.under_heading = false;
        }
        if self.table.is_some() {
            let run = match &event {
                Event::Text(t) => Some((t.to_string(), self.style())),
                Event::Code(t) => Some((t.to_string(), self.styles.code)),
                Event::SoftBreak | Event::HardBreak => Some((" ".to_owned(), self.styles.plain)),
                _ => None,
            };
            let table = self.table.as_mut().expect("checked");
            match event {
                _ if run.is_some() => table.cell.extend(run),
                Event::End(TagEnd::TableCell) => {
                    let cell = std::mem::take(&mut table.cell);
                    table.row.push(cell);
                }
                Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                    let row = std::mem::take(&mut table.row);
                    table.rows.push(row);
                }
                Event::End(TagEnd::Table) => {
                    let table = self.table.take().unwrap_or_default();
                    self.table_lines(table);
                }
                // Bold, italics and the like inside a cell.
                other => self.inline(other),
            }
            return;
        }
        match event {
            Event::Start(Tag::Paragraph) => {
                if self.marker.is_none() && !std::mem::take(&mut self.under_heading) {
                    self.gap();
                }
            }
            Event::End(TagEnd::Paragraph) => self.flush(),
            Event::Start(Tag::Heading { .. }) => {
                self.flush();
                self.gap();
                self.heading = true;
            }
            Event::End(TagEnd::Heading(_)) => {
                self.flush();
                self.heading = false;
                self.under_heading = true;
            }
            Event::Start(Tag::BlockQuote(_)) => {
                self.flush();
                self.gap();
                self.quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                self.flush();
                self.quote -= 1;
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                self.flush();
                self.gap();
                self.code_block = true;
                if let CodeBlockKind::Fenced(lang) = kind
                    && !lang.is_empty()
                {
                    let word = lang.split([' ', ',']).next().unwrap_or_default().to_ascii_lowercase();
                    self.diff_block = matches!(word.as_str(), "diff" | "patch" | "udiff");
                    let mut line = self.quote_prefix();
                    line.push(Span::raw(" ".repeat(self.indent)));
                    line.push(Span::styled(format!("┌ {lang}"), self.styles.dim));
                    self.out.push(Line::from(line));
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                self.code_block = false;
                self.diff_block = false;
            }
            Event::Start(Tag::List(start)) => {
                self.flush();
                self.gap();
                self.lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                self.flush();
                self.lists.pop();
            }
            Event::Start(Tag::Item) => {
                self.flush();
                let depth = self.lists.len().saturating_sub(1);
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    _ => "• ".to_owned(),
                };
                self.outer.push(self.indent);
                self.indent = depth * 2 + marker.width();
                self.marker = Some(marker);
            }
            Event::End(TagEnd::Item) => {
                self.flush();
                // Back to the enclosing item's indent.
                self.indent = self.outer.pop().unwrap_or(0);
            }
            Event::Start(Tag::Strong | Tag::Emphasis | Tag::Strikethrough)
            | Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough) => self.inline(event),
            Event::Start(Tag::Link { dest_url, .. }) => self.link = Some((dest_url.to_string(), self.runs.len())),
            Event::End(TagEnd::Link) => {
                if let Some((url, from)) = self.link.take() {
                    let text: String = self.runs[from..].iter().map(|(t, _)| t.as_str()).collect();
                    if text != url && !url.is_empty() {
                        self.runs.push((format!(" ({url})"), self.styles.dim));
                    }
                }
            }
            Event::Start(Tag::Table(_)) => {
                self.flush();
                self.gap();
                self.table = Some(Table::default());
            }
            Event::Text(text) if self.code_block => {
                for row in text.lines() {
                    let mut line = self.quote_prefix();
                    line.push(Span::raw(" ".repeat(self.indent)));
                    line.push(Span::styled("│ ", self.styles.dim));
                    line.push(Span::styled(row.to_owned(), self.code_line_style(row)));
                    self.out.push(Line::from(line));
                }
            }
            Event::Text(text) => {
                let style = self.style();
                self.runs.push((text.into_string(), style));
            }
            Event::Code(code) => {
                let mut style = self.styles.code;
                if self.bold > 0 || self.heading {
                    style = style.add_modifier(Modifier::BOLD);
                }
                self.runs.push((code.into_string(), style));
            }
            Event::SoftBreak => self.runs.push((" ".into(), self.styles.plain)),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.flush();
                self.gap();
                let rule = "─".repeat(self.width.min(40));
                self.out.push(Line::from(Span::styled(rule, self.styles.dim)));
            }
            Event::Html(html) | Event::InlineHtml(html) => self.runs.push((html.into_string(), self.styles.plain)),
            _ => {}
        }
    }

    /// A code line's style: a diff's added lines green, removed red, hunks dim.
    fn code_line_style(&self, row: &str) -> Style {
        if !self.diff_block || row.starts_with("+++") || row.starts_with("---") {
            return self.styles.code;
        }
        match row.chars().next() {
            Some('+') => Style::new().fg(Color::Green),
            Some('-') => Style::new().fg(Color::Red),
            _ if row.starts_with("@@") => self.styles.dim,
            _ => self.styles.code,
        }
    }

    /// Style changes inside a paragraph or a cell.
    fn inline(&mut self, event: Event<'_>) {
        match event {
            Event::Start(Tag::Strong) => self.bold += 1,
            Event::End(TagEnd::Strong) => self.bold = self.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => self.italic += 1,
            Event::End(TagEnd::Emphasis) => self.italic = self.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => self.strike += 1,
            Event::End(TagEnd::Strikethrough) => self.strike = self.strike.saturating_sub(1),
            _ => {}
        }
    }

    /// A grid: columns sized to their content, narrowed to fit the width, each
    /// cell wrapping inside its own column. The header is bold and ruled off;
    /// rows that wrap are ruled off from each other so they stay readable.
    fn table_lines(&mut self, table: Table) {
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }
        let text_width = |cell: &Cell| cell.iter().map(|(t, _)| t.width()).sum::<usize>();
        let longest_word = |cell: &Cell| {
            cell.iter().flat_map(|(t, _)| t.split_whitespace()).map(|w| w.width()).max().unwrap_or(0)
        };
        let column = |c: usize| table.rows.iter().filter_map(move |r| r.get(c));
        let mut widths: Vec<usize> = (0..columns).map(|c| column(c).map(text_width).max().unwrap_or(0).max(1)).collect();
        let floor: Vec<usize> = (0..columns).map(|c| column(c).map(longest_word).max().unwrap_or(1).clamp(1, 12)).collect();
        let available = self.width.saturating_sub(3 * (columns - 1)).max(columns);
        // Narrow the widest column, a column at a time, until the grid fits.
        while widths.iter().sum::<usize>() > available {
            let Some(c) = (0..columns).filter(|&c| widths[c] > floor[c]).max_by_key(|&c| widths[c]) else {
                // Every column is at its floor: let the floors give too.
                let c = (0..columns).max_by_key(|&c| widths[c]).expect("a column");
                if widths[c] <= 1 {
                    break;
                }
                widths[c] -= 1;
                continue;
            };
            widths[c] -= 1;
        }
        let bar = Span::styled(" │ ", self.styles.dim);
        let rule = |cross: &str| {
            let parts: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
            Line::from(Span::styled(parts.join(cross), self.styles.dim))
        };
        let laid_out: Vec<Vec<Vec<Line<'static>>>> = table
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                (0..columns)
                    .map(|c| {
                        let mut cell = row.get(c).cloned().unwrap_or_default();
                        if i == 0 {
                            for (_, style) in &mut cell {
                                *style = style.add_modifier(Modifier::BOLD);
                            }
                        }
                        trim_runs(&mut cell);
                        wrap(&cell, Vec::new(), Vec::new(), widths[c])
                    })
                    .collect()
            })
            .collect();
        let tall = laid_out.iter().skip(1).any(|row| row.iter().any(|cell| cell.len() > 1));
        for (i, row) in laid_out.iter().enumerate() {
            if i > 1 && tall {
                self.out.push(rule("─┼─"));
            }
            let height = row.iter().map(Vec::len).max().unwrap_or(1);
            for r in 0..height {
                let mut spans = Vec::new();
                for (c, cell) in row.iter().enumerate() {
                    if c > 0 {
                        spans.push(bar.clone());
                    }
                    let line = cell.get(r).cloned().unwrap_or_default();
                    let pad = widths[c].saturating_sub(line.width());
                    spans.extend(line.spans);
                    if c + 1 < columns {
                        spans.push(Span::raw(" ".repeat(pad)));
                    }
                }
                self.out.push(Line::from(spans));
            }
            if i == 0 {
                self.out.push(rule("─┼─"));
            }
        }
    }
}

/// Trim the whitespace around a cell's text.
fn trim_runs(cell: &mut Cell) {
    if let Some((first, _)) = cell.first_mut() {
        *first = first.trim_start().to_owned();
    }
    if let Some((last, _)) = cell.last_mut() {
        *last = last.trim_end().to_owned();
    }
}

/// Fill lines no wider than `width` with styled runs, breaking between words —
/// or inside a word longer than a line.
fn wrap(runs: &[(String, Style)], first: Vec<Span<'static>>, rest: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    let rest_width: usize = rest.iter().map(Span::width).sum();
    let mut lines = Vec::new();
    let mut spans = first;
    let mut used: usize = spans.iter().map(Span::width).sum();
    let start = used;
    let mut line_start = start;
    let mut space: Option<Style> = None;
    for (text, style) in runs {
        for word in split_words(text) {
            if word.chars().all(char::is_whitespace) {
                if used > line_start {
                    space = Some(*style);
                }
                continue;
            }
            let w = word.width();
            let gap = usize::from(space.is_some());
            if used + gap + w > width && used > line_start {
                lines.push(Line::from(std::mem::replace(&mut spans, rest.clone())));
                used = rest_width;
                line_start = rest_width;
                space = None;
            }
            if let Some(style) = space.take() {
                spans.push(Span::styled(" ", style));
                used += 1;
            }
            if used + w <= width {
                spans.push(Span::styled(word.to_owned(), *style));
                used += w;
                continue;
            }
            // Longer than a line: break it where it overflows.
            let mut piece = String::new();
            for c in word.chars() {
                let cw = c.width().unwrap_or(0);
                if used + cw > width && used > line_start {
                    spans.push(Span::styled(std::mem::take(&mut piece), *style));
                    lines.push(Line::from(std::mem::replace(&mut spans, rest.clone())));
                    used = rest_width;
                    line_start = rest_width;
                }
                piece.push(c);
                used += cw;
            }
            spans.push(Span::styled(piece, *style));
        }
    }
    lines.push(Line::from(spans));
    lines
}

/// Words and the whitespace between them, in order.
fn split_words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space = None;
    for (i, c) in text.char_indices() {
        let space = c.is_whitespace();
        if in_space.is_some_and(|was| was != space) {
            out.push(&text[start..i]);
            start = i;
        }
        in_space = Some(space);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn styles() -> Styles {
        Styles { plain: Style::new(), code: Style::new().fg(Color::Cyan), dim: Style::new().fg(Color::DarkGray) }
    }

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim_end().to_owned()).collect()
    }

    #[test]
    fn bold_is_bold_and_the_asterisks_are_gone() {
        let out = lines("**권한 모드**는 설정이에요.", 80, &styles());
        assert_eq!(text(&out), ["권한 모드는 설정이에요."]);
        let bold = out[0].spans.iter().find(|s| s.content == "권한").unwrap();
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn inline_code_keeps_its_text_in_its_own_style() {
        let out = lines("run `sysctl -n hw.model` now", 80, &styles());
        assert_eq!(text(&out), ["run sysctl -n hw.model now"]);
        assert!(out[0].spans.iter().any(|s| s.content == "sysctl" && s.style.fg == Some(Color::Cyan)));
    }

    #[test]
    fn lists_get_markers_and_continuation_lines_line_up() {
        let out = lines("1. first item that is long enough to wrap\n2. second\n\n- a\n- b", 24, &styles());
        assert_eq!(
            text(&out),
            ["1. first item that is", "   long enough to wrap", "2. second", "", "• a", "• b"]
        );
    }

    #[test]
    fn nested_lists_indent() {
        let out = lines("- outer\n  - inner", 40, &styles());
        assert_eq!(text(&out), ["• outer", "  • inner"]);
    }

    #[test]
    fn code_blocks_keep_their_lines_behind_a_bar() {
        let out = lines("Look:\n\n```rust\nfn main() {}\n    let x = 1;\n```\n\nDone.", 40, &styles());
        assert_eq!(text(&out), ["Look:", "", "┌ rust", "│ fn main() {}", "│     let x = 1;", "", "Done."]);
    }

    #[test]
    fn headings_are_bold_paragraphs_have_a_gap() {
        let out = lines("## Plan\nFirst.\n\nSecond.", 40, &styles());
        assert_eq!(text(&out), ["Plan", "First.", "", "Second."]);
        assert!(out[0].spans.iter().all(|s| s.style.add_modifier.contains(Modifier::BOLD) || s.content.trim().is_empty()));
    }

    #[test]
    fn tables_align_when_they_fit() {
        let out = lines("| mode | does |\n|---|---|\n| default | asks |\n| dontAsk | denies |", 40, &styles());
        assert_eq!(text(&out), ["mode    │ does", "────────┼───────", "default │ asks", "dontAsk │ denies"]);
    }

    #[test]
    fn a_wide_table_wraps_inside_its_columns() {
        let md = "| 모드 | 동작 |\n|---|---|\n| **default** (기본) | 작업할 때마다 허락할지 물어봐요. 처음엔 이게 가장 무난해요. |\n| **plan** | 읽기만 해요. |";
        let out = lines(md, 40, &styles());
        let rows = text(&out);
        for line in &out {
            assert!(line.width() <= 40, "{rows:#?}");
        }
        assert!(rows[0].starts_with("모드") && rows[0].contains("│ 동작"), "{rows:#?}");
        assert!(rows[1].contains("┼"), "{rows:#?}");
        // The long cell wraps inside its column; its first column stays put.
        assert!(rows[2].starts_with("default (기본) │ 작업할"), "{rows:#?}");
        assert!(rows[3].starts_with("               │ "), "{rows:#?}");
        // Rows that wrap are ruled off from each other.
        assert!(rows.iter().filter(|r| r.contains("┼")).count() >= 2, "{rows:#?}");
        let bold = out[2].spans.iter().find(|s| s.content == "default").unwrap();
        assert!(bold.style.add_modifier.contains(Modifier::BOLD), "bold survives inside a cell");
    }

    #[test]
    fn hangul_wraps_by_its_width_and_nothing_overflows() {
        let out = lines("에이전트가 파일을 고쳤어요 이제 리뷰해 주세요 브랜치에 커밋했어요", 20, &styles());
        for line in &out {
            assert!(line.width() <= 20, "{:?}", text(&out));
        }
        assert_eq!(text(&out).join(" "), "에이전트가 파일을 고쳤어요 이제 리뷰해 주세요 브랜치에 커밋했어요");
    }

    #[test]
    fn a_word_longer_than_the_line_is_broken() {
        let out = lines("https://example.com/a/very/long/path/that/never/ends", 20, &styles());
        assert!(out.iter().all(|l| l.width() <= 20));
        assert_eq!(text(&out).concat(), "https://example.com/a/very/long/path/that/never/ends");
    }

    #[test]
    fn a_diff_fence_colours_what_came_and_went() {
        let out = lines("```diff\n@@ -1 +1 @@\n-old\n+new\n same\n```", 40, &styles());
        let colour = |row: usize| out[row].spans.last().and_then(|s| s.style.fg);
        assert_eq!(text(&out)[0], "┌ diff");
        assert_eq!((colour(2), colour(3), colour(4)), (Some(Color::Red), Some(Color::Green), Some(Color::Cyan)));
        let plain = lines("```rust\n-1\n```", 40, &styles());
        assert_eq!(plain[1].spans.last().and_then(|s| s.style.fg), Some(Color::Cyan), "only a diff is coloured so");
    }

    #[test]
    fn links_show_where_they_go() {
        let out = lines("see [the docs](https://code.claude.com)", 80, &styles());
        assert_eq!(text(&out), ["see the docs (https://code.claude.com)"]);
    }
}
