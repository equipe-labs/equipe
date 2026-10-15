//! A text view for the conversation: the palette's colours, code blocks with
//! a header (language, copy) and highlighting, tables that scroll sideways,
//! links that go where they point, and the plugins — the same everywhere a
//! reply, a note or a callout's inside is drawn.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use equipe_core::t;
use gpui_kit::base::text::{TextView, TextViewMotion, TextViewStyle};
use gpui_kit::component::h_flex;
use gpui_kit::*;

use super::links::{self, Link};
use super::{Open, ShellRef, base_dir, code, is_dark, open_file, plugins, prose, stream, with_shell};
use crate::icons::{Icon, icon};
use crate::theme::{MONO, Palette};

/// How a text view's text looks: its size and line height, its colour, and
/// whether it is still streaming in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub size: f32,
    pub line: f32,
    pub color: Rgba,
    pub live: bool,
}

/// How long "copied" stays on a copy button.
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// The style every conversation text view starts from.
pub fn style(p: Palette, look: &Look) -> TextViewStyle {
    let to_hsla = |c: Rgba| -> Hsla { c.into() };
    let size = look.size;
    // Borderless, a quiet fill; room above the code for its header (language, copy).
    let code_block = StyleRefinement::default().bg(to_hsla(p.code_bg)).rounded(px(10.)).pt(px(30.)).text_size(px(12.5)).line_height(px(19.));
    let mut table = StyleRefinement::default().bg(to_hsla(p.code_bg)).border_color(to_hsla(p.hair)).text_color(to_hsla(p.text)).rounded(px(8.));
    // A wide table scrolls sideways instead of squeezing its columns.
    table.overflow.x = Some(Overflow::Scroll);
    let table_head = StyleRefinement::default().bg(to_hsla(p.raised)).text_color(to_hsla(p.text)).font_weight(FontWeight::SEMIBOLD);
    // Cells keep to one line (a word broken mid-way reads worse than a
    // table to scroll), so a wide table scrolls.
    let mut table_cell = StyleRefinement::default().text_color(to_hsla(p.text2)).border_color(to_hsla(p.hair));
    table_cell.text.white_space = Some(WhiteSpace::Nowrap);
    TextViewStyle::default()
        .with_foreground(to_hsla(look.color))
        .with_muted_foreground(to_hsla(p.muted))
        .with_link(to_hsla(p.accent))
        .with_selection(to_hsla(p.accent_bg))
        .with_border(to_hsla(p.hair))
        .with_code_background(to_hsla(p.code_bg))
        .with_code_block(code_block)
        .with_table(table)
        .with_table_head(table_head)
        .with_table_cell(table_cell)
        .with_inline_code(HighlightStyle { color: Some(to_hsla(p.accent)), background_color: Some(to_hsla(p.code_bg)), ..Default::default() })
        // A conversation is not a document: headings are bold, barely larger.
        .with_heading(move |level| {
            let grow = match level {
                1 => 2.,
                2 => 1.,
                _ => 0.,
            };
            StyleRefinement::default().text_size(px(size + grow)).font_weight(FontWeight::SEMIBOLD)
        })
        .with_dark(is_dark(&p))
}

/// The Markdown a text view is given: a streaming reply's unfinished end held
/// back, long fences cut (unless `expanded`), ticket numbers made links and
/// emphasis next to Korean closed (`prose`). With it, the whole code of each
/// cut fence, for its copy button.
pub fn prepare(raw: &str, live: bool, expanded: &dyn Fn(usize) -> bool) -> (String, HashMap<String, String>) {
    let shown = if live { stream::hold_back(raw) } else { raw };
    let more = |n: usize| t!("render.code_more", count = n);
    let rows = |n: usize| t!("render.table_more_rows", count = n);
    let words = code::Words { more: &more, less: t!("render.code_less"), rows: &rows };
    let fences = code::prepare(shown, expanded, &words);
    (prose::fix(&fences.text), fences.full)
}

/// Set a text view up the conversation's way. `scope` names the row, for the
/// controls its links carry (a code block's "show the rest").
pub fn configure(view: TextView, p: Palette, look: Look, scope: u64, full: Arc<HashMap<String, String>>) -> TextView {
    let muted = p.muted;
    let (wash, text) = (p.wash, p.text);
    let (a, b, c, d, e, f) = plugins::all(p, look);
    let motion = if look.live { TextViewMotion::default().with_stream_fade(Duration::from_millis(150)).with_stream_fade_stagger(Duration::from_millis(20)) } else { TextViewMotion::default() };
    view.style(style(p, &look))
        .selectable(true)
        .shared_code_block_highlighter(code::highlighter(is_dark(&p)))
        .code_block_actions(move |block, window, cx| {
            let shown = block.code();
            let whole = full.get(shown.as_ref()).cloned().unwrap_or_else(|| shown.to_string());
            let label = block.lang().and_then(|l| code::label(&l));
            let copied = window.use_keyed_state(("copied", super::hash_of(&whole)), cx, |_, _| None::<Instant>);
            let is_copied = copied.read(cx).is_some_and(|at| at.elapsed() < COPIED_FOR);
            h_flex()
                .gap_1()
                .items_center()
                .children(label.map(|l| div().px_1().font_family(MONO).text_size(px(11.5)).text_color(muted).child(l)))
                .child(
                    div()
                        .id("copy")
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(wash))
                        .child(icon(if is_copied { Icon::Check } else { Icon::Copy }, 13., if is_copied { text } else { muted }))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(whole.clone()));
                            copied.update(cx, |at, cx| {
                                *at = Some(Instant::now());
                                cx.notify();
                            });
                            let copied = copied.downgrade();
                            cx.spawn(async move |cx| {
                                cx.background_executor().timer(COPIED_FOR).await;
                                let _ = copied.update(cx, |_, cx| cx.notify());
                            })
                            .detach();
                        }),
                )
        })
        .on_link_click(move |url, event, _, cx| follow(url, event.modifiers(), scope, cx))
        .image_source(|uri| {
            let url = uri.to_string();
            match url.strip_prefix("file://") {
                Some(path) => ImageSource::from(std::path::PathBuf::from(path)),
                None if url.starts_with('/') => ImageSource::from(std::path::PathBuf::from(url)),
                None => ImageSource::Resource(Resource::Uri(uri.clone())),
            }
        })
        .plugin(a)
        .plugin(b)
        .plugin(c)
        .plugin(d)
        .plugin(e)
        .plugin(f)
        .motion(motion)
}

/// A text view of its own, inside another's custom block (a callout's body).
pub fn nested(p: Palette, id: (&'static str, u64), markdown: &str, look: Look) -> impl IntoElement {
    let (text, full) = prepare(markdown, false, &|_| true);
    div().text_size(px(look.size)).line_height(px(look.line)).text_color(look.color).child(configure(
        TextView::markdown(ElementId::NamedInteger(id.0.into(), id.1), text),
        p,
        look,
        0,
        Arc::new(full),
    ))
}

/// A link in a reply was clicked: a ticket opens on the board, a page in the
/// browser, a file in Quick Look (⌘: in its app, ⌥: in Finder), a control
/// does what it says.
pub fn follow(url: &str, modifiers: Modifiers, scope: u64, cx: &mut App) {
    let base = cx.try_global::<ShellRef>().and_then(|s| s.0.upgrade()).map(|s| base_dir(s.read(cx))).unwrap_or_default();
    match links::classify(url, &base) {
        Link::Ticket(n) => with_shell(cx, |shell, cx| shell.open_ticket(n, cx)),
        Link::Web(url) => cx.open_url(&url),
        Link::File { path, .. } => {
            let how = if modifiers.platform {
                Open::Edit
            } else if modifiers.alt {
                Open::Reveal
            } else {
                Open::Look
            };
            open_file(&path, how, cx);
        }
        Link::Control(what) => with_shell(cx, |shell, cx| shell.chat_control(scope, &what, cx)),
        Link::Other(_) => {}
    }
}
