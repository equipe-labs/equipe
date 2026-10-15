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

use super::links::{self, chip_label, file_mark, looks_like_path};
use super::raster::{self, Kind};
use super::view::Look;
use super::{Open, ShellRef, base_dir, hash_of, open_file, tones, with_shell};
use crate::theme::{MONO, Palette};

fn hex(c: Rgba) -> u32 {
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    byte(c.r) << 16 | byte(c.g) << 8 | byte(c.b)
}

/// Every plugin, in the order they are asked.
pub fn all(p: Palette, look: Look) -> (FileChips, Images, InlineMath, BlockMath, Mermaid, Alerts) {
    (FileChips { p }, Images { p }, InlineMath { p }, BlockMath { p, look }, Mermaid { p }, Alerts { p, look })
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

    fn render_inline(&self, node: &MarkdownNode, context: &InlineRenderContext, _: &mut Window, _: &mut App) -> Option<InlineElement> {
        let text = node.data::<String>()?.clone();
        let (path, line) = links::path_and_line(&text);
        let path = PathBuf::from(path);
        let id = ("file-chip", hash_of((&text, node.source_range().map(|r| r.start))));
        let size = context.font_size() * 0.88;
        Some(InlineElement::new(file_chip(self.p, id, &path, line, &text, size)))
    }
}

/// A file as a chip: its mark and name; a click looks at it, ⌘-click opens
/// it, ⌥-click shows it in Finder; the menu also copies its path.
pub fn file_chip(p: Palette, id: (&'static str, u64), path: &std::path::Path, line: Option<u32>, written: &str, size: Pixels) -> impl IntoElement {
    let written = written.to_owned();
    let resolve = move |cx: &mut App| -> PathBuf {
        let base = cx.try_global::<ShellRef>().and_then(|s| s.0.upgrade()).map(|s| base_dir(s.read(cx))).unwrap_or_default();
        links::resolve(links::path_and_line(&written).0, &base)
    };
    let (click, locate) = (resolve.clone(), resolve);
    h_flex()
        .id(id)
        .gap(px(4.))
        .px(px(5.))
        .rounded(px(5.))
        .bg(p.code_bg)
        .border_1()
        .border_color(p.hair)
        .text_size(size)
        .font_family(MONO)
        .text_color(p.text2)
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash).text_color(p.text))
        .child(div().text_color(p.muted).child(file_mark(path)))
        .child(chip_label(path, line))
        .on_click(move |event, _, cx| {
            let how = match event.modifiers() {
                m if m.platform => Open::Edit,
                m if m.alt => Open::Reveal,
                _ => Open::Look,
            };
            let path = click(cx);
            open_file(&path, how, cx);
        })
        .context_menu(move |menu, _, _| {
            let item = |label: &'static str, how: Option<Open>| {
                let locate = locate.clone();
                PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    let path = locate(cx);
                    match how {
                        Some(how) => open_file(&path, how, cx),
                        None => cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string())),
                    }
                })
            };
            menu.item(item(t!("render.file_look"), Some(Open::Look)))
                .item(item(t!("render.file_open"), Some(Open::Edit)))
                .item(item(t!("render.file_reveal"), Some(Open::Reveal)))
                .separator()
                .item(item(t!("render.file_copy_path"), None))
        })
}

// ── images ──

pub struct Images {
    p: Palette,
}

#[derive(Clone)]
struct ImageRef {
    url: String,
    alt: String,
}

impl MarkdownPlugin for Images {
    fn name(&self) -> &str {
        "image"
    }

    fn parse(&self, node: &md::Node, _: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let md::Node::Image(image) = node else { return None };
        let alt = image.alt.clone();
        Some(MarkdownNode::new("image", ImageRef { url: image.url.clone(), alt: alt.clone() }).text(alt))
    }

    fn render_inline(&self, node: &MarkdownNode, context: &InlineRenderContext, _: &mut Window, cx: &mut App) -> Option<InlineElement> {
        let image = node.data::<ImageRef>()?.clone();
        let p = self.p;
        let id = hash_of(&image.url);
        let source = image_source(&image.url, cx);
        let Some(source) = source else {
            // Not on this Mac: a chip that says so, and copies where it is.
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
                    .child(div().child(t!("render.image_elsewhere")))
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
            Err(_) => source_box(p, &source),
        })
    }
}

/// Source shown as code, when it couldn't be drawn.
fn source_box(p: Palette, source: &str) -> AnyElement {
    div()
        .w_full()
        .p_3()
        .rounded(px(8.))
        .bg(p.code_bg)
        .border_1()
        .border_color(p.hair)
        .font_family(MONO)
        .text_size(px(12.5))
        .line_height(px(19.))
        .text_color(p.text2)
        .children(source.lines().map(|l| div().child(l.to_owned())))
        .into_any_element()
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
        let failed = picture.is_err();
        let toggle = {
            let state = showing_source.clone();
            div()
                .id(("mermaid-toggle", key))
                .px_2()
                .rounded(px(5.))
                .text_size(px(12.))
                .text_color(p.muted)
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash).text_color(p.text))
                .child(if as_source { t!("render.show_diagram") } else { t!("render.show_source") })
                .on_click(move |_, _, cx| state.update(cx, |s, cx| {
                    *s = !*s;
                    cx.notify();
                }))
        };
        let copy = {
            let source = source.clone();
            div()
                .id(("mermaid-copy", key))
                .px_2()
                .rounded(px(5.))
                .text_size(px(12.))
                .text_color(p.muted)
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash).text_color(p.text))
                .child(t!("chat.copy"))
                .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(source.clone())))
        };
        v_flex()
            .w_full()
            .my_1()
            .rounded(px(8.))
            .border_1()
            .border_color(p.hair)
            .bg(p.code_bg)
            .child(
                h_flex()
                    .px_2()
                    .pt_1()
                    .gap_1()
                    .text_size(px(12.))
                    .child(div().flex_1().pl_1().font_family(MONO).text_color(p.muted).child("mermaid"))
                    .when(!failed, |row| row.child(toggle))
                    .child(copy),
            )
            .child(match picture {
                Ok(pic) if !as_source => div()
                    .w_full()
                    .p_3()
                    .flex()
                    .justify_center()
                    .overflow_hidden()
                    .child(img(ImageSource::Render(pic.image)).max_w_full().w(px(pic.width)).h(px(pic.height)).flex_none())
                    .into_any_element(),
                _ => v_flex()
                    .child(div().p_1().child(source_box(p, &source)))
                    .when(failed, |col| col.child(div().px_3().pb_2().text_size(px(12.)).text_color(tones(&p).danger).child(t!("render.diagram_failed"))))
                    .into_any_element(),
            })
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
    use super::{Alert, alert_of, looks_like_math};

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
