//! What the text view draws its own way: a path in a code span as a file
//! chip, an image that can be looked at closer, math, Mermaid diagrams, and
//! GitHub's callouts (`> [!NOTE]`). Parsing reads only the Markdown; drawing
//! reaches the window through [`super::ShellRef`].

use std::path::PathBuf;

use equipe_core::t;
use gpui_kit::base::text::{InlineElement, InlineRenderContext, MarkdownNode, MarkdownParseContext, MarkdownPlugin, markdown_ast as md};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::links::{self, looks_like_path};
use super::raster::{self, Kind};
use super::view::Look;
use super::{Open, ShellRef, base_dir, hash_of, open_file, tones, with_shell};
use crate::theme::{MONO, Palette, R_LG, R_SM};

fn hex(c: Rgba) -> u32 {
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    byte(c.r) << 16 | byte(c.g) << 8 | byte(c.b)
}

/// Every plugin, in the order they are asked; `scope`: the conversation's row they draw in.
pub fn all(p: Palette, look: Look, scope: u64) -> (FileChips, Images, InlineMath, BlockMath, Mermaid, Alerts, Cards) {
    (FileChips { p }, Images { p, live: look.live, scope }, InlineMath { p }, BlockMath { p, look }, Mermaid { p }, Alerts { p, look }, Cards { p, scope })
}

// ── file chips ──

pub struct FileChips {
    p: Palette,
}

impl MarkdownPlugin for FileChips {
    fn name(&self) -> &str {
        "file-chip"
    }

    fn parse(&self, node: &md::Node, _: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let md::Node::InlineCode(code) = node else { return None };
        looks_like_path(&code.value).then(|| MarkdownNode::new("file-chip", code.value.clone()).text(code.value.clone()).markdown(format!("`{}`", code.value)))
    }

    fn render_inline(&self, node: &MarkdownNode, context: &InlineRenderContext, _: &mut Window, cx: &mut App) -> Option<InlineElement> {
        let text = node.data::<String>()?.clone();
        let id = ("file-chip", hash_of((&text, node.source_range().map(|r| r.start))));
        let size = context.font_size() * 0.88;
        let found = cx.try_global::<ShellRef>().and_then(|s| s.0.upgrade()).map(|s| locate(s.read(cx), links::path_and_line(&text).0));
        // A document opens in the panel wherever the daemon is.
        let openable = crate::doc::is_doc(&text) || found.is_some_and(|path| exists(&path));
        Some(InlineElement::new(file_chip(self.p, id, &text, openable, size)))
    }
}

/// Where a path written in the conversation is: from where the conversation
/// works, else in the worktree of a ticket not merged yet — the one it names
/// first (`outputs/T-287 …/a.txt`) — as the document panel looks.
pub fn locate(shell: &crate::Shell, written: &str) -> PathBuf {
    let base = base_dir(shell);
    let path = links::resolve(written, &base);
    if exists(&path) {
        return path;
    }
    let places = crate::doc::places(&path, &shell.ticket_places());
    crate::doc::elsewhere(&path, &base, &places).paths.into_iter().find(|p| exists(p)).unwrap_or(path)
}

/// Whether `path` is on this Mac, asked at most every few seconds: a reply
/// draws its paths every frame.
fn exists(path: &std::path::Path) -> bool {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};
    thread_local! {
        static SEEN: RefCell<HashMap<PathBuf, (bool, Instant)>> = RefCell::new(HashMap::new());
    }
    SEEN.with(|seen| {
        let mut seen = seen.borrow_mut();
        if let Some((there, at)) = seen.get(path)
            && at.elapsed() < Duration::from_secs(5)
        {
            return *there;
        }
        let there = path.exists();
        if seen.len() > 512 {
            seen.clear();
        }
        seen.insert(path.to_owned(), (there, Instant::now()));
        there
    })
}

/// A path as a chip shows it: whole when short; else its last folder and
/// name, or its name alone (`…/A linkedin paste.txt`). A chip is one
/// piece on its line — it can't wrap — so a long one would run past the
/// words' edge in a narrow window. The whole path is on hover.
pub fn shown_path(written: &str) -> String {
    const MOST: usize = 34;
    let trimmed = written.trim_end_matches('/');
    if written.chars().count() <= MOST || !trimmed.contains('/') {
        return written.to_owned();
    }
    let slash = if written.ends_with('/') { "/" } else { "" };
    let mut parts = trimmed.rsplit('/');
    let name = parts.next().unwrap_or(trimmed);
    match parts.next() {
        Some(folder) if folder.chars().count() + name.chars().count() + 3 <= MOST => format!("…/{folder}/{name}{slash}"),
        _ => format!("…/{name}{slash}"),
    }
}

/// A path in a reply, as written, like any inline code: a quiet fill, no
/// mark. One on this Mac (`openable`) underlines under the pointer — a click
/// looks at it, ⌘-click opens it, ⌥-click shows it in Finder — and its menu
/// also copies its path. A document (Markdown) opens in the panel at the
/// right instead, from this Mac or another.
pub fn file_chip(p: Palette, id: (&'static str, u64), written: &str, openable: bool, size: Pixels) -> impl IntoElement + use<> {
    let label = shown_path(written);
    let whole = (label != written).then(|| written.to_owned());
    let written = written.to_owned();
    // No kind of file at its end: a folder.
    let folder = {
        let path = links::path_and_line(&written).0.trim_end_matches('/');
        let name = path.rsplit('/').next().unwrap_or(path);
        path.contains('/') && !name.contains('.')
    };
    let resolve = move |cx: &mut App| -> PathBuf {
        let path = links::path_and_line(&written).0;
        cx.try_global::<ShellRef>().and_then(|s| s.0.upgrade()).map_or_else(|| PathBuf::from(path), |s| locate(s.read(cx), path))
    };
    let (click, locate, add_locate) = (resolve.clone(), resolve.clone(), resolve);
    let accent = p.accent;
    div()
        .id(id)
        .px(px(4.))
        .rounded(px(4.))
        .bg(p.code_bg)
        .text_size(size)
        .font_family(MONO)
        .text_color(p.accent)
        .child(label)
        .when(openable, |chip| {
            chip.cursor_pointer().hover(move |s| s.underline().text_decoration_color(accent)).on_click(move |event, _, cx| {
                let how = match event.modifiers() {
                    m if m.secondary() => Open::Edit,
                    m if m.alt => Open::Reveal,
                    _ => Open::Look,
                };
                let path = click(cx);
                if how == Open::Look && crate::doc::is_doc(&path.to_string_lossy()) {
                    return super::with_shell(cx, |shell, cx| shell.open_doc(path, cx));
                }
                open_file(&path, how, cx);
            })
        })
        // Shown short: the whole of it on hover.
        .when_some(whole, |chip, whole| crate::tips::tip(chip, p, whole, None))
        .context_menu(move |menu, _, _| {
            let item = |label: &'static str, how: Option<Open>| {
                let locate = locate.clone();
                PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    let path = locate(cx);
                    match how {
                        Some(Open::Look) if crate::doc::is_doc(&path.to_string_lossy()) => super::with_shell(cx, |shell, cx| shell.open_doc(path, cx)),
                        Some(how) => open_file(&path, how, cx),
                        None => cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string())),
                    }
                })
            };
            let menu = menu.item(item(t!("render.file_look"), Some(Open::Look)))
                .item(item(t!("render.file_open"), Some(Open::Edit)))
                .item(item(t!("render.file_reveal"), Some(Open::Reveal)))
                .separator()
                .item(item(t!("render.file_copy_path"), None));
            // A folder a member made (a new project): one click puts it on the squad.
            if folder {
                let locate = add_locate.clone();
                menu.separator().item(PopupMenuItem::new(t!("render.add_as_project")).on_click(move |_, _, cx| {
                    let path = locate(cx);
                    super::with_shell(cx, |shell, cx| shell.add_project(path.display().to_string(), cx));
                }))
            } else {
                menu
            }
        })
}

// ── images ──

/// An image a reply embeds: from the web as it is; a file — on this Mac or
/// the daemon's, alike — asked of the daemon (`read_image`) and drawn in the
/// column (`views::visuals::inline`), opened big among the reply's others.
pub struct Images {
    p: Palette,
    live: bool,
    scope: u64,
}

#[derive(Clone)]
struct ImageRef {
    url: String,
    alt: String,
    /// Every file the text embeds, as written, and what each is called: the big view goes through them.
    all: std::sync::Arc<Vec<(String, String)>>,
}

impl MarkdownPlugin for Images {
    fn name(&self) -> &str {
        "image"
    }

    fn parse(&self, node: &md::Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let md::Node::Image(image) = node else { return None };
        let alt = image.alt.clone();
        let all = equipe_core::visuals::markdown::local_images(cx.source()).into_iter().map(|i| (i.path, i.alt)).collect();
        Some(MarkdownNode::new("image", ImageRef { url: image.url.clone(), alt: alt.clone(), all: std::sync::Arc::new(all) }).text(alt))
    }

    fn render_inline(&self, node: &MarkdownNode, context: &InlineRenderContext, _: &mut Window, cx: &mut App) -> Option<InlineElement> {
        let image = node.data::<ImageRef>()?.clone();
        let p = self.p;
        let id = hash_of(&image.url);
        if !equipe_core::visuals::markdown::is_web(&image.url)
            && let Some(shell) = cx.try_global::<ShellRef>().and_then(|s| s.0.upgrade())
        {
            let (shots, base) = {
                let read = shell.read(cx);
                (crate::views::visuals::Shots::of(read, shell.downgrade(), cx), base_dir(read))
            };
            let lexical = |written: &str| equipe_core::visuals::markdown::lexical(written, &base).map(|p| p.to_string_lossy().into_owned());
            // A folder in a picture's place: its pictures come as the strip under the reply.
            if !image.url.trim_end_matches('/').rsplit('/').next().is_some_and(|n| n.contains('.')) {
                return Some(InlineElement::new(div()));
            }
            if let Some(path) = lexical(&image.url) {
                let pictures: Vec<(String, String)> = image.all.iter().filter_map(|(w, alt)| Some((lexical(w)?, alt.clone()))).collect();
                return Some(InlineElement::new(crate::views::visuals::inline(&shots, self.scope, &path, &image.alt, &pictures, self.live, cx)));
            }
        }
        let source = image_source(&image.url, cx);
        let Some(source) = source else {
            // A path that can't be read (`~`, `..`): a chip that says so, and copies where it is.
            let label = if image.alt.is_empty() { image.url.clone() } else { image.alt.clone() };
            let url = image.url.clone();
            return Some(InlineElement::new(
                h_flex()
                    .id(("image-missing", id))
                    .gap_1()
                    .px(px(6.))
                    .rounded(px(5.))
                    .bg(p.code_bg)
                    .border_1()
                    .border_color(p.hair)
                    .text_size(context.font_size() * 0.88)
                    .text_color(p.muted)
                    .cursor_pointer()
                    .child("▣")
                    .child(label)
                    .child(div().child(t!("visuals.inline_failed")))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                        super::flash(t!("render.path_copied").to_owned(), cx);
                    }),
            ));
        };
        let zoom = source.clone();
        Some(InlineElement::new(
            div()
                .id(("image", id))
                .py_1()
                .cursor_pointer()
                .child(img(source).max_w(px(560.)).max_h(px(420.)).rounded(px(6.)).border_1().border_color(p.hair))
                .on_click(move |_, _, cx| {
                    let zoom = zoom.clone();
                    with_shell(cx, move |shell, cx| {
                        shell.chat.lightbox = Some(zoom);
                        cx.notify();
                    });
                }),
        ))
    }
}

/// Where an image in a reply comes from: the web, or a file on this Mac.
/// `None`: a path this Mac doesn't have.
pub fn image_source(url: &str, cx: &App) -> Option<ImageSource> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("data:") {
        return Some(ImageSource::Resource(Resource::Uri(SharedUri::from(url.to_owned()))));
    }
    let base = cx.try_global::<ShellRef>().and_then(|s| s.0.upgrade()).map(|s| base_dir(s.read(cx))).unwrap_or_default();
    let path = match links::classify(url, &base) {
        links::Link::File { path, .. } => path,
        _ => links::resolve(url, &base),
    };
    path.is_file().then(|| ImageSource::from(path))
}

// ── math ──

/// `$…$` reads as math, not money: no space just inside the dollars, not a
/// number after them, and something only math has.
pub fn looks_like_math(inner: &str, after: Option<char>) -> bool {
    if inner.is_empty() || inner.starts_with(char::is_whitespace) || inner.ends_with(char::is_whitespace) {
        return false;
    }
    if after.is_some_and(|c| c.is_ascii_digit()) {
        return false;
    }
    let mathy = inner.chars().any(|c| matches!(c, '\\' | '^' | '_' | '{' | '=' | '+' | '<' | '>'));
    let one_symbol = inner.chars().count() <= 3 && inner.chars().all(|c| c.is_alphabetic() || c == '\'');
    mathy || one_symbol
}

pub struct InlineMath {
    p: Palette,
}

impl MarkdownPlugin for InlineMath {
    fn name(&self) -> &str {
        "math-inline"
    }

    fn parse(&self, node: &md::Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let md::Node::InlineMath(math) = node else { return None };
        let end = node.position()?.end.offset;
        let after = cx.source().get(end..).and_then(|rest| rest.chars().next());
        looks_like_math(&math.value, after).then(|| MarkdownNode::new("math-inline", math.value.clone()).text(math.value.clone()).markdown(format!("${}$", math.value)))
    }

    fn render_inline(&self, node: &MarkdownNode, context: &InlineRenderContext, _: &mut Window, cx: &mut App) -> Option<InlineElement> {
        let source = node.data::<String>()?;
        let size = f32::from(context.font_size());
        match raster::draw(Kind::InlineMath, source, size, hex(self.p.text), super::is_dark(&self.p), cx) {
            Ok(pic) => Some(
                InlineElement::new(img(ImageSource::Render(pic.image.clone())).w(px(pic.width)).h(px(pic.height)).flex_none())
                    .with_baseline(px(pic.baseline)),
            ),
            Err(_) => Some(InlineElement::new(div().font_family(MONO).text_size(context.font_size() * 0.9).text_color(self.p.text2).child(format!("${source}$")))),
        }
    }
}

pub struct BlockMath {
    p: Palette,
    look: Look,
}

impl MarkdownPlugin for BlockMath {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "math-block"
    }

    fn parse(&self, node: &md::Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let source = match node {
            md::Node::Math(math) => math.value.clone(),
            md::Node::Code(code) if matches!(code.lang.as_deref(), Some("math" | "latex" | "tex")) && fence_closed(node, cx) => code.value.clone(),
            _ => return None,
        };
        Some(MarkdownNode::new("math-block", source.clone()).text(source.clone()).markdown(format!("$$\n{source}\n$$")))
    }

    fn render(&self, node: &MarkdownNode, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let source = node.data::<String>().cloned().unwrap_or_default();
        let p = self.p;
        let picture = raster::draw(Kind::DisplayMath, &source, self.look.size * 1.1, hex(p.text), super::is_dark(&p), cx);
        div().w_full().py_2().flex().justify_center().overflow_hidden().child(match picture {
            Ok(pic) => img(ImageSource::Render(pic.image)).w(px(pic.width)).h(px(pic.height)).flex_none().into_any_element(),
            Err(_) => source_box(p, &source, true),
        })
    }
}

/// Source shown as code, when it couldn't be drawn.
fn source_box(p: Palette, source: &str, framed: bool) -> AnyElement {
    // Lines don't wrap (a `<br/` cut from its `>` read as broken): a long one scrolls sideways.
    div()
        .id(("source", hash_of(source)))
        .w_full()
        .overflow_x_scroll()
        .when(framed, |d| d.p_3().rounded(px(8.)).bg(p.code_bg).border_1().border_color(p.hair))
        .font_family(MONO)
        .text_size(px(12.5))
        .line_height(px(19.))
        .text_color(p.text2)
        .children(source.lines().map(|l| div().whitespace_nowrap().child(l.to_owned())))
        .into_any_element()
}

/// The face of a diagram's block, as the diagram theme has it: white in
/// light, `color.surface` in dark — the colour of its plain nodes and of an
/// HTML card, so the three sit as one.
fn diagram_face(p: &Palette) -> Rgba {
    if super::is_dark(p) { rgb(0x16171A) } else { rgb(0xFFFFFF) }
}

/// Whether a fence's closing line has come (a diagram is drawn only then).
fn fence_closed(node: &md::Node, cx: &MarkdownParseContext<'_>) -> bool {
    let Some(source) = cx.node_source(node) else { return false };
    let mut lines = source.trim_end().lines();
    let open = lines.next().unwrap_or_default().trim_start();
    let marker = open.chars().next().unwrap_or('`');
    let run = open.chars().take_while(|c| *c == marker).count();
    lines.next_back().is_some_and(|last| {
        let last = last.trim();
        last.len() >= run && last.chars().all(|c| c == marker)
    })
}

// ── diagrams ──

pub struct Mermaid {
    p: Palette,
}

impl MarkdownPlugin for Mermaid {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "mermaid"
    }

    fn parse(&self, node: &md::Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let md::Node::Code(code) = node else { return None };
        // Drawn once the fence has closed; until then it streams as code.
        (code.lang.as_deref().is_some_and(|l| l.eq_ignore_ascii_case("mermaid")) && fence_closed(node, cx))
            .then(|| MarkdownNode::new("mermaid", code.value.clone()).text(code.value.clone()).markdown(format!("```mermaid\n{}\n```", code.value)))
    }

    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let source = node.data::<String>().cloned().unwrap_or_default();
        let p = self.p;
        let key = hash_of(&source);
        let showing_source = window.use_keyed_state(("mermaid-source", key), cx, |_, _| false);
        let as_source = *showing_source.read(cx);
        let picture = raster::draw(Kind::Mermaid, &source, 14., hex(p.text), super::is_dark(&p), cx);
        // A diagram that couldn't be drawn is its source, with no card around it.
        let Ok(pic) = picture else {
            return v_flex()
                .w_full()
                .my_1()
                .gap_1()
                .child(source_box(p, &source, true))
                .child(div().px_1().text_size(px(12.)).text_color(tones(&p).danger).child(t!("render.diagram_failed")))
                .into_any_element();
        };
        let name = crate::views::visuals::diagram_title(&source).unwrap_or_else(|| t!("render.show_diagram").to_owned());
        // Source and copy show under the pointer or at the keys (and stay while the source is up, which is the way back).
        let copy_focus = window.use_keyed_state(("mermaid-copy-focus", key), cx, |_, cx| cx.focus_handle().tab_stop(true)).read(cx).clone();
        let reveal = crate::views::visuals::Reveal::of("mermaid", key, as_source, Some(&copy_focus), window, cx);
        let toggle = {
            let state = showing_source.clone();
            let label = if as_source { t!("render.show_diagram") } else { t!("render.show_source") };
            reveal.button(p, ElementId::NamedInteger("mermaid-toggle".into(), key), label, format!("{label}: {name}"), None).on_click(move |_, _, cx| {
                state.update(cx, |s, cx| {
                    *s = !*s;
                    cx.notify();
                })
            })
        };
        let copy = {
            let source = source.clone();
            reveal
                .button(p, ElementId::NamedInteger("mermaid-copy".into(), key), t!("chat.copy"), format!("{}: {name}", t!("chat.copy")), Some(&copy_focus))
                .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(source.clone())))
        };
        let block = div()
            .id(("mermaid", key))
            .relative()
            .w_full()
            .my_1()
            .pt(px(48.))
            .px(px(16.))
            .pb(px(16.))
            .rounded(px(R_LG))
            .border_1()
            .border_color(p.hair)
            .bg(diagram_face(&p))
            .role(Role::Image)
            .aria_label(name)
            .child(h_flex().absolute().top(px(8.)).right(px(8.)).gap_1().child(toggle).child(copy))
            .child(if as_source {
                source_box(p, &source, false)
            } else {
                div().w_full().flex().justify_center().overflow_hidden().child(img(ImageSource::Render(pic.image)).max_w_full().w(px(pic.width)).h(px(pic.height)).flex_none()).into_any_element()
            });
        reveal.watch(block).into_any_element()
    }
}

// ── HTML cards ──

/// How long a card waits for the daemon's picture before it shows its source.
const CARD_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// A fenced block ```` ```html card ````: the daemon's picture of it (light or
/// dark as the window is), a place held while it is drawn, and — when it
/// isn't coming (the daemon draws no cards, or it failed) — the source,
/// folded.
pub struct Cards {
    p: Palette,
    scope: u64,
}

impl MarkdownPlugin for Cards {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "html-card"
    }

    fn parse(&self, node: &md::Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let md::Node::Code(code) = node else { return None };
        let card = code.lang.as_deref().is_some_and(|l| l.eq_ignore_ascii_case("html")) && code.meta.as_deref().is_some_and(|m| m.trim().eq_ignore_ascii_case("card"));
        (card && fence_closed(node, cx)).then(|| MarkdownNode::new("html-card", code.value.clone()).text(code.value.clone()).markdown(format!("```html card\n{}\n```", code.value)))
    }

    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let source = node.data::<String>().cloned().unwrap_or_default();
        let p = self.p;
        let key = equipe_core::visuals::diagram::key(&source);
        let drawn = cx.try_global::<ShellRef>().and_then(|s| s.0.upgrade()).and_then(|shell| {
            shell.read(cx).model.convos.values().flat_map(|c| c.rendered.iter()).find(|r| r.kind == "card" && r.key == key).cloned()
        });
        if let Some(drawn) = drawn
            && let Some(shell) = cx.try_global::<ShellRef>().and_then(|s| s.0.upgrade())
        {
            let shots = crate::views::visuals::Shots::of(shell.read(cx), shell.downgrade(), cx);
            if let Some(card) = crate::views::visuals::card(&shots, self.scope, &drawn, crate::views::visuals::card_title(&source), window, cx) {
                return card;
            }
        }
        // Not drawn yet: a place held for a while, then the source.
        let since = card_seen(&key);
        let waiting = since.elapsed() < CARD_WAIT;
        if waiting {
            self.recheck(&key, since, cx);
            return h_flex()
                .my_1()
                .w_full()
                .h(px(96.))
                .items_center()
                .justify_center()
                .rounded(px(R_LG))
                .border_1()
                .border_color(p.hair)
                .bg(p.wash)
                .text_size(px(12.5))
                .text_color(p.muted)
                .child(t!("render.card_waiting"))
                .into_any_element();
        }
        let open = window.use_keyed_state(("card-source", hash_of(&key)), cx, |_, _| false);
        let shown = *open.read(cx);
        let ring = Rgba { a: 0.55, ..p.accent };
        let focus = window.use_keyed_state(("card-fold-focus", hash_of(&key)), cx, |_, cx| cx.focus_handle().tab_stop(true)).read(cx).clone();
        let label = if shown { t!("render.card_hide") } else { t!("render.show_source") };
        v_flex()
            .w_full()
            .my_1()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(div().child(t!("render.card_failed")))
                    .child(
                        div()
                            .id(("card-fold", hash_of(&key)))
                            .h(px(24.))
                            .px_2()
                            .flex()
                            .items_center()
                            .rounded(px(R_SM))
                            .bg(p.code_bg)
                            .cursor_pointer()
                            .hover(move |s| s.text_color(p.text))
                            .track_focus(&focus)
                            .focus_visible(move |s| s.shadow(vec![BoxShadow { color: ring.into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.), inset: false }]))
                            .role(Role::Button)
                            .aria_label(label)
                            .child(label)
                            .on_click(move |_, _, cx| open.update(cx, |s, cx| {
                                *s = !*s;
                                cx.notify();
                            })),
                    ),
            )
            .when(shown, |col| col.child(source_box(p, &source, true)))
            .into_any_element()
    }
}

/// When this card was first drawn on screen.
fn card_seen(key: &str) -> std::time::Instant {
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static SEEN: RefCell<HashMap<String, std::time::Instant>> = RefCell::new(HashMap::new());
    }
    SEEN.with(|seen| *seen.borrow_mut().entry(key.to_owned()).or_insert_with(std::time::Instant::now))
}

impl Cards {
    /// Draw this row again when the wait is over (once per card).
    fn recheck(&self, key: &str, since: std::time::Instant, cx: &mut App) {
        use std::cell::RefCell;
        use std::collections::HashSet;
        thread_local! {
            static SET: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
        }
        if !SET.with(|s| s.borrow_mut().insert(key.to_owned())) {
            return;
        }
        let (scope, left) = (self.scope, CARD_WAIT.saturating_sub(since.elapsed()));
        cx.spawn(async move |cx| {
            cx.background_executor().timer(left + std::time::Duration::from_millis(50)).await;
            cx.update(|cx| with_shell(cx, |shell, cx| shell.chat.remeasure_row(scope, cx)));
        })
        .detach();
    }
}

// ── callouts ──

/// GitHub's callout kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alert {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl Alert {
    pub fn parse(marker: &str) -> Option<Self> {
        Some(match marker.trim().to_ascii_uppercase().as_str() {
            "[!NOTE]" => Self::Note,
            "[!TIP]" => Self::Tip,
            "[!IMPORTANT]" => Self::Important,
            "[!WARNING]" => Self::Warning,
            "[!CAUTION]" => Self::Caution,
            _ => return None,
        })
    }

    fn title(self) -> &'static str {
        match self {
            Self::Note => t!("render.alert_note"),
            Self::Tip => t!("render.alert_tip"),
            Self::Important => t!("render.alert_important"),
            Self::Warning => t!("render.alert_warning"),
            Self::Caution => t!("render.alert_caution"),
        }
    }

    fn mark(self) -> &'static str {
        match self {
            Self::Note => "ⓘ",
            Self::Tip => "✦",
            Self::Important => "❖",
            Self::Warning | Self::Caution => "⚠",
        }
    }

    fn color(self, p: &Palette) -> Rgba {
        let t = tones(p);
        match self {
            Self::Note => t.note,
            Self::Tip => t.tip,
            Self::Important => t.important,
            Self::Warning => t.warning,
            Self::Caution => t.danger,
        }
    }
}

/// A blockquote's source as a callout: its kind and the Markdown inside.
pub fn alert_of(quote: &str) -> Option<(Alert, String)> {
    let mut lines = quote.lines().map(|l| {
        let l = l.trim_start();
        let l = l.strip_prefix('>').unwrap_or(l);
        l.strip_prefix(' ').unwrap_or(l)
    });
    let first = lines.next()?;
    let (marker, rest) = first.split_at(first.find(']')? + 1);
    let kind = Alert::parse(marker)?;
    let mut inner: Vec<&str> = Vec::new();
    if !rest.trim().is_empty() {
        inner.push(rest.trim());
    }
    inner.extend(lines);
    Some((kind, inner.join("\n")))
}

pub struct Alerts {
    p: Palette,
    look: Look,
}

impl MarkdownPlugin for Alerts {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "alert"
    }

    fn parse(&self, node: &md::Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let md::Node::Blockquote(_) = node else { return None };
        let (kind, inner) = alert_of(cx.node_source(node)?)?;
        Some(MarkdownNode::new("alert", (kind, inner.clone())).text(inner.clone()).markdown(cx.node_source(node)?.to_owned()))
    }

    fn render(&self, node: &MarkdownNode, _: &mut Window, _: &mut App) -> impl IntoElement {
        let (kind, inner) = node.data::<(Alert, String)>().cloned().unwrap_or((Alert::Note, String::new()));
        let p = self.p;
        let color = kind.color(&p);
        let tint = Rgba { a: 0.08, ..color };
        let look = Look { size: self.look.size, line: self.look.line, color: self.look.color, live: false };
        v_flex()
            .w_full()
            .my_1()
            .pl_3()
            .pr_3()
            .py_2()
            .gap_1()
            .rounded_r(px(6.))
            .border_l_2()
            .border_color(color)
            .bg(tint)
            .child(h_flex().gap_2().text_size(px(self.look.size - 1.)).font_weight(FontWeight::SEMIBOLD).text_color(color).child(kind.mark()).child(kind.title()))
            .child(super::view::nested(p, ("alert", hash_of(&inner)), &inner, look))
    }
}

#[cfg(test)]
mod tests {
    use super::{Alert, alert_of, looks_like_math, shown_path};

    #[test]
    fn a_long_path_is_shown_short_as_one_piece_that_fits_a_narrow_line() {
        assert_eq!(shown_path("notes/plan.md"), "notes/plan.md");
        assert_eq!(shown_path("outputs/T-287 홍보 글 올리기/A linkedin 붙여넣기.txt"), "…/A linkedin 붙여넣기.txt");
        assert_eq!(shown_path("crates/desktop/src/views/settings_signals.rs"), "…/views/settings_signals.rs");
        assert_eq!(shown_path("outputs/T-117 앱스토어 제출/shots/"), "outputs/T-117 앱스토어 제출/shots/", "short enough: whole");
        assert_eq!(shown_path("outputs/T-117 앱스토어 제출 자료 모음/screenshots/"), "…/T-117 앱스토어 제출 자료 모음/screenshots/");
        assert_eq!(shown_path("a_very_long_file_name_without_any_folder_at_all.txt"), "a_very_long_file_name_without_any_folder_at_all.txt");
    }

    #[test]
    fn callouts_are_read_from_their_marker() {
        assert_eq!(alert_of("> [!NOTE]\n> Read **this**.\n> Twice."), Some((Alert::Note, "Read **this**.\nTwice.".into())));
        assert_eq!(alert_of("> [!warning] Mind the gap"), Some((Alert::Warning, "Mind the gap".into())));
        assert_eq!(alert_of(">[!TIP]\n>\n> spaced"), Some((Alert::Tip, "\nspaced".into())));
        assert_eq!(alert_of("> just a quote"), None);
        assert_eq!(alert_of("> [!NOPE]\n> x"), None);
    }

    #[test]
    fn dollars_are_money_unless_they_read_as_math() {
        assert!(looks_like_math("x^2", None));
        assert!(looks_like_math(r"\frac{a}{b}", Some(' ')));
        assert!(looks_like_math("x", Some('.')));
        assert!(!looks_like_math("5 and ", Some('1')), "$5 and $10");
        assert!(!looks_like_math("5", Some('0')));
        assert!(!looks_like_math(" x ", None));
        assert!(!looks_like_math("price", None));
    }
}
