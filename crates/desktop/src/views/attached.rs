//! What your message attached, drawn as itself in the conversation (and
//! small on the memo rail): pictures as rounded thumbnails — one alone at its
//! shape, several as tiles — a video as its first frame with ▶, a PDF or a
//! file as a chip with its kind, name and size. A placeholder breathes while
//! one comes, it fades in when it does, and a file that is gone says so.
//! A picture opens big; ←/→ go through the message's others. What is known of
//! each file is `crate::sent`'s.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use equipe_core::protocol::tags::{Tag, TagKind};
use equipe_core::t;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::attach::{self, Kind};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::sent::{self, Gallery, Shown, Want};
use crate::theme::{CONTROL, ICON, ICON_SM, Palette, R_LG, R_SM, R_XL};
use crate::tips::tip;
use crate::{Screen, Shell};

/// How long a picture that just came fades in.
const FADE: Duration = Duration::from_millis(260);

/// What every attachment of one frame draws with.
#[derive(Clone)]
pub struct Ctx {
    pub p: Palette,
    pub sent: Rc<RefCell<sent::Sent>>,
    pub handle: WeakEntity<Shell>,
    /// The daemon is on another machine: a file can't be opened from here.
    pub remote: bool,
    /// How wide the row is, at most.
    pub room: f32,
}

impl Ctx {
    pub fn of(shell: &Shell, handle: WeakEntity<Shell>, room: f32) -> Self {
        Self { p: shell.palette, sent: shell.chat.sent.clone(), handle, remote: shell.endpoint.is_remote(), room }
    }

    /// What `path` shows; the first time, it is asked for — once the frame
    /// is drawn, so nothing changes while it is.
    fn ask(&self, path: &str, want: Want, cx: &mut App) -> Shown {
        let mut sent = self.sent.borrow_mut();
        let shown = sent.look(path, want);
        if matches!(shown, Shown::Loading) && !sent.scheduled {
            sent.scheduled = true;
            let handle = self.handle.clone();
            cx.defer(move |cx| {
                handle.update(cx, |shell, cx| shell.load_sent(cx)).ok();
            });
        }
        shown
    }
}

/// A tag's kind as the composer knows kinds (for a chip's badge and colour).
fn kind_of(tag: &Tag) -> Kind {
    match tag.kind {
        TagKind::Image => Kind::Image,
        TagKind::Video => Kind::Video,
        TagKind::Audio => Kind::Audio,
        TagKind::Pdf => Kind::Pdf,
        TagKind::File => match Kind::of(Path::new(tag.name())) {
            Kind::Text => Kind::Text,
            Kind::Doc => Kind::Doc,
            _ => Kind::File,
        },
    }
}

/// What a chip says under its name: its size when known, and what the tag
/// tells — pages, lines, a length.
pub fn detail(tag: &Tag, size: Option<u64>) -> String {
    let size_label = size.map(attach::size_label);
    match (tag.kind, size_label) {
        (TagKind::Pdf, Some(size)) => tag.pages.map_or(size.clone(), |pages| t!("composer.pdf_pages", size = size, pages = pages)),
        (TagKind::Pdf, None) => tag.pages.map_or_else(|| "PDF".to_owned(), |n| t!("sent.pages", count = n)),
        (TagKind::File, Some(size)) => tag.lines.map_or(size.clone(), |n| t!("composer.text_lines", size = size, count = n)),
        (TagKind::File, None) => tag.lines.map(|n| t!("sent.lines", count = n)).unwrap_or_default(),
        (TagKind::Audio | TagKind::Video, size) => [tag.duration.clone(), size].into_iter().flatten().collect::<Vec<_>>().join(" · "),
        (TagKind::Image, size) => size.unwrap_or_default(),
    }
}

/// The attachments of one message, over its words: pictures first, then
/// the chips — on the right (`right`) as your message is; `None` when it has none.
pub fn attachments(ctx: &Ctx, key: u64, tags: &[Tag], more: usize, right: bool, cx: &mut App) -> Option<AnyElement> {
    if tags.is_empty() {
        return None;
    }
    let p = ctx.p;
    let reduce = cx.reduce_motion();
    let gallery = Rc::new(sent::gallery_of(tags));
    let is_picture = |t: &Tag| matches!(t.kind, TagKind::Image | TagKind::Video);
    let pictures: Vec<(usize, &Tag)> = tags.iter().enumerate().filter(|(_, t)| is_picture(t)).collect();
    let files: Vec<(usize, &Tag)> = tags.iter().enumerate().filter(|(_, t)| !is_picture(t)).collect();
    let alone = pictures.len() == 1;
    let mut tiles: Vec<AnyElement> = Vec::new();
    for (n, tag) in pictures {
        let id = key << 6 | n as u64;
        let shown = sent::want_of(tag).map(|(path, want)| ctx.ask(path, want, cx));
        let size = match (&shown, alone) {
            (_, false) => (sent::TILE, sent::TILE),
            (Some(Shown::Picture(picture)), true) => sent::single_size(Some(picture.dims), ctx.room),
            (_, true) => sent::single_size(tag.dims, ctx.room),
        };
        // The picture the lightbox opens at: the image, or the video's sheet.
        let opens = match tag.kind {
            TagKind::Image => Some(tag.path.clone()),
            _ => tag.sheet.clone().or_else(|| tag.picture().map(str::to_owned)),
        }
        .filter(|_| !matches!(shown, Some(Shown::Gone)));
        let click = opens.map(|at| {
            let (handle, gallery) = (ctx.handle.clone(), gallery.clone());
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let (gallery, at) = ((*gallery).clone(), at.clone());
                handle.update(cx, |shell, cx| shell.open_gallery(gallery, &at, cx)).ok();
            }
        });
        tiles.push(picture(p, id, size, tag, shown.as_ref(), !alone, reduce, click));
    }
    let chips: Vec<AnyElement> = files.into_iter().map(|(n, tag)| chip(ctx, key << 6 | n as u64, tag, cx)).collect();
    Some(
        v_flex()
            .gap_2()
            .max_w(relative(0.86))
            .when(right, |c| c.items_end())
            .when(!tiles.is_empty(), |c| c.child(h_flex().flex_wrap().when(right, |r| r.justify_end()).gap_2().children(tiles)))
            .when(!chips.is_empty(), |c| c.child(h_flex().flex_wrap().when(right, |r| r.justify_end()).gap_2().children(chips)))
            .when(more > 0, |c| c.child(div().text_size(px(12.)).font_weight(FontWeight::NORMAL).text_color(p.muted).child(t!("sent.more", count = more))))
            .into_any_element(),
    )
}

/// A breathing placeholder while a picture comes.
fn placeholder(p: Palette, id: ElementId, reduce: bool) -> AnyElement {
    let base = div().size_full().bg(p.wash);
    if reduce {
        return base.into_any_element();
    }
    base.with_animation(id, Animation::new(Duration::from_millis(1400)).repeat().with_easing(bounce(ease_in_out)), |el, t| el.opacity(1. - 0.45 * t)).into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn picture(p: Palette, n: u64, size: (f32, f32), tag: &Tag, shown: Option<&Shown>, cover: bool, reduce: bool, click: Option<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>) -> AnyElement {
    let face: AnyElement = match shown {
        Some(Shown::Picture(picture)) => {
            let image = img(picture.thumb.clone()).size_full().object_fit(if cover { ObjectFit::Cover } else { ObjectFit::Contain });
            if !reduce && picture.at.elapsed() < FADE * 2 {
                motion::fade(div().size_full().child(image), ElementId::NamedInteger("sent-in".into(), n), FADE).into_any_element()
            } else {
                image.into_any_element()
            }
        }
        Some(Shown::Gone) => gone_face(p, tag, size.0 >= 100.),
        // A video without frames: its kind, dark.
        None => div().size_full().bg(rgb(0x1C1C1E)).into_any_element(),
        _ => placeholder(p, ElementId::NamedInteger("sent-wait".into(), n), reduce),
    };
    let video = tag.kind == TagKind::Video;
    let gone = matches!(shown, Some(Shown::Gone));
    div()
        .id(ElementId::NamedInteger("sent-picture".into(), n))
        .relative()
        .flex_none()
        .w(px(size.0))
        .h(px(size.1))
        .rounded(px(R_LG))
        .overflow_hidden()
        .bg(p.wash)
        .border_1()
        .border_color(p.hair)
        .child(face)
        .when(video && !gone, |d| {
            d.child(
                div().absolute().inset_0().flex().items_center().justify_center().child(
                    div().size(px(34.)).rounded_full().bg(rgba(0x00000099)).flex().items_center().justify_center().text_size(px(13.)).text_color(rgb(0xFFFFFF)).child("▶"),
                ),
            )
            .children(tag.duration.clone().map(|d| div().absolute().right(px(6.)).bottom(px(5.)).px_1().rounded(px(4.)).bg(rgba(0x0000008C)).text_size(px(10.5)).font_weight(FontWeight::NORMAL).text_color(rgb(0xFFFFFF)).child(d)))
        })
        .when_some(click, |d, click| d.cursor_pointer().on_click(click))
        .into_any_element()
}

/// Where a picture was, gone: the file isn't there any more.
fn gone_face(p: Palette, tag: &Tag, roomy: bool) -> AnyElement {
    v_flex()
        .size_full()
        .p_2()
        .gap_1()
        .items_center()
        .justify_center()
        .bg(p.wash)
        .text_color(p.muted)
        .font_weight(FontWeight::NORMAL)
        .child(icon(Icon::File, ICON, p.muted))
        .child(div().max_w_full().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(11.5)).child(tag.name().to_owned()))
        .when(roomy, |c| c.child(div().text_size(px(11.)).child(t!("sent.gone"))))
        .into_any_element()
}

/// A file that isn't a picture: its kind, its name, what it holds.
fn chip(ctx: &Ctx, n: u64, tag: &Tag, cx: &mut App) -> AnyElement {
    let p = ctx.p;
    let kind = kind_of(tag);
    let shown = sent::want_of(tag).map(|(path, want)| ctx.ask(path, want, cx));
    let gone = matches!(shown, Some(Shown::Gone));
    let size = match shown {
        Some(Shown::Sized(size)) => Some(size),
        Some(Shown::Picture(ref picture)) => Some(picture.size),
        _ => None,
    };
    let loading = matches!(shown, Some(Shown::Loading));
    let text = if gone { t!("sent.gone").to_owned() } else { detail(tag, size) };
    let opens = (!ctx.remote && !gone).then(|| PathBuf::from(&tag.path));
    let badge = div()
        .size(px(32.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_SM))
        .bg(p.wash)
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .text_color(if gone { p.muted } else { super::composer::kind_color(p, kind) })
        .child(attach::badge(tag.name(), kind));
    let face = h_flex()
        .id(ElementId::NamedInteger("sent-chip".into(), n))
        .flex_none()
        .w(px(224.))
        .h(px(46.))
        .pl(px(7.))
        .pr_2()
        .gap_2()
        .items_center()
        .rounded(px(R_LG))
        .bg(p.bg)
        .border_1()
        .border_color(p.hair)
        .font_weight(FontWeight::NORMAL)
        .child(badge)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.5)).line_height(px(17.)).text_color(if gone { p.muted } else { p.text }).when(gone, |d| d.line_through()).child(tag.name().to_owned()))
                .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(11.)).line_height(px(15.)).text_color(p.muted).child(if loading && text.is_empty() { "…".to_owned() } else { text })),
        )
        .when_some(opens, |d, path| d.cursor_pointer().hover(move |s| s.bg(p.wash)).on_click(move |_, _, cx| cx.open_with_system(&path)));
    if gone { tip(face, p, t!("sent.gone_tip"), None).into_any_element() } else { face.into_any_element() }
}

// ── the lightbox ──

/// A picture of a message shown big over the window: ←/→ through the
/// message's others, Esc or a click outside closes.
pub fn gallery(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let gallery = shell.chat.gallery.as_ref().filter(|_| shell.screen == Screen::Chat)?;
    let path = gallery.path().to_owned();
    let (at, n) = (gallery.at, gallery.paths.len());
    let remote = shell.endpoint.is_remote();
    let shown = shell.chat.sent.borrow().get(&path).cloned();
    let source: Option<ImageSource> = match (&shown, remote) {
        (Some(Shown::Gone), _) => None,
        (_, false) => Some(PathBuf::from(&path).into()),
        (Some(Shown::Picture(picture)), true) => Some(picture.full.clone().unwrap_or_else(|| picture.thumb.clone()).into()),
        _ => None,
    };
    let name = equipe_core::protocol::tags::display_name(&path).to_owned();
    let p = shell.palette;
    let round = |b: Stateful<Div>, glyph: Icon, label: &'static str, keys: &'static str| {
        tip(b.size(px(CONTROL)).flex().items_center().justify_center().rounded_full().cursor_pointer().hover(|s| s.bg(rgba(0xFFFFFF1F))).child(icon(glyph, ICON, rgb(0xFFFFFF))), p, label, Some(keys))
    };
    let open_in = (!remote && source.is_some()).then(|| PathBuf::from(&path));
    let open_label = t!("composer.open_system");
    let face = match source {
        Some(source) => img(source).max_w_full().max_h_full().object_fit(ObjectFit::Contain).rounded(px(8.)).into_any_element(),
        None => v_flex()
            .size(px(200.))
            .gap_2()
            .rounded(px(R_XL))
            .bg(rgba(0xFFFFFF14))
            .items_center()
            .justify_center()
            .text_size(px(13.))
            .text_color(rgba(0xFFFFFFB3))
            .child(icon(Icon::File, 28., rgba(0xFFFFFFB3)))
            .child(if matches!(shown, Some(Shown::Gone)) { t!("sent.gone") } else { "…" })
            .into_any_element(),
    };
    let backdrop = div()
        .id("sent-lightbox")
        .absolute()
        .inset_0()
        .bg(rgba(0x000000F0))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .p_8()
        .on_click(cx.listener(|this, _, _, cx| {
            this.chat.gallery = None;
            cx.notify();
        }))
        .child(motion::rise(
            div().id("sent-lightbox-picture").flex_1().min_h_0().w_full().flex().items_center().justify_center().on_click(|_, _, cx| cx.stop_propagation()).child(face),
            ElementId::Name(format!("sent-lightbox-{path}").into()),
        ))
        .child(
            h_flex()
                .id("sent-lightbox-bar")
                .flex_none()
                .h(px(44.))
                .px_2()
                .gap_3()
                .items_center()
                .rounded(px(22.))
                .bg(rgba(0x2A2A2EF2))
                .border_1()
                .border_color(rgba(0xFFFFFF1F))
                .on_click(|_, _, cx| cx.stop_propagation())
                .when(n > 1, |b| {
                    let prev = div().id("sent-lightbox-prev").on_click(cx.listener(|this, _, _, cx| {
                        this.step_gallery(false);
                        cx.notify();
                    }));
                    b.child(round(prev, Icon::ArrowLeft, t!("composer.aria_previous"), "←"))
                })
                .child(div().text_size(px(13.)).text_color(rgb(0xFFFFFF)).child(name))
                .when(n > 1, |b| b.child(div().text_size(px(12.)).text_color(rgba(0xFFFFFFB3)).child(format!("{}/{n}", at + 1))))
                .children(open_in.map(|path| {
                    div()
                        .id("sent-lightbox-open")
                        .px_3()
                        .h(px(CONTROL))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .rounded_full()
                        .border_1()
                        .border_color(rgba(0xFFFFFF4D))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgba(0xFFFFFF1F)))
                        .text_size(px(12.))
                        .text_color(rgb(0xFFFFFF))
                        .child(icon(Icon::External, ICON_SM, rgb(0xFFFFFF)))
                        .child(open_label)
                        .on_click(move |_, _, cx| cx.open_with_system(&path))
                }))
                .when(n > 1, |b| {
                    let next = div().id("sent-lightbox-next").on_click(cx.listener(|this, _, _, cx| {
                        this.step_gallery(true);
                        cx.notify();
                    }));
                    b.child(round(next, Icon::ArrowRight, t!("composer.aria_next"), "→"))
                })
                .child(round(
                    div().id("sent-lightbox-close").on_click(cx.listener(|this, _, _, cx| {
                        this.chat.gallery = None;
                        cx.notify();
                    })),
                    Icon::X,
                    t!("composer.aria_close"),
                    "Esc",
                )),
        );
    Some(motion::fade(backdrop, "sent-lightbox-in", motion::BASE))
}

impl Shell {
    /// Load what the conversation asked about: each file off the UI thread,
    /// from this Mac or over the line; the rows that show it measured again.
    pub fn load_sent(&mut self, cx: &mut Context<Self>) {
        let wanted = self.chat.sent.borrow_mut().take_wanted();
        if wanted.is_empty() {
            return;
        }
        let from = if self.endpoint.is_remote() {
            match self.link.as_ref() {
                Some(link) => sent::Where::Remote(link.uploader()),
                None => return,
            }
        } else {
            sent::Where::Local
        };
        let generation = self.chat.sent.borrow().generation;
        for (path, want) in wanted {
            let from = from.clone();
            let asked = path.clone();
            let loaded = cx.background_spawn(async move { sent::load(&from, &asked, want) });
            cx.spawn(async move |this, cx| {
                let shown = loaded.await;
                this.update(cx, |this, cx| this.sent_arrived(generation, &path, shown, cx)).ok();
            })
            .detach();
        }
    }

    /// What was known of `path` changed: keep it, and draw again the rows that show it.
    pub fn sent_arrived(&mut self, generation: u64, path: &str, shown: Shown, cx: &mut Context<Self>) {
        {
            let mut sent = self.chat.sent.borrow_mut();
            if sent.generation != generation {
                return;
            }
            sent.put(path, shown);
        }
        self.chat.remeasure_mentioning(path, cx);
        cx.notify();
    }

    /// Show a message's picture big, among its others.
    pub fn open_gallery(&mut self, paths: Vec<String>, at: &str, cx: &mut Context<Self>) {
        self.chat.gallery = Gallery::open(paths, at);
        cx.notify();
    }

    pub fn step_gallery(&mut self, forward: bool) {
        if let Some(gallery) = self.chat.gallery.as_mut() {
            gallery.step(forward);
        }
    }

    /// ←/→ go through the pictures shown big, Esc and space put them away.
    /// True when the key was taken.
    pub fn on_gallery_key(&mut self, key: &str, modifiers: &Modifiers, cx: &mut Context<Self>) -> bool {
        let plain = !modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.shift;
        if self.chat.gallery.is_none() || self.screen != Screen::Chat || !plain {
            return false;
        }
        match key {
            "left" | "right" => self.step_gallery(key == "right"),
            "escape" | "space" => self.chat.gallery = None,
            _ => return false,
        }
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Tag, detail, kind_of};
    use equipe_core::protocol::tags::find;

    fn tag(text: &str) -> Tag {
        find(text).remove(0)
    }

    #[test]
    fn a_chip_says_what_is_known_of_its_file() {
        let pdf = tag("[pdf: /u/1792073093606-spec.pdf · 12 pages]");
        assert_eq!(detail(&pdf, Some(2 << 20)), "2.0 MB · 12쪽");
        assert_eq!(detail(&pdf, None), "12쪽");
        let text = tag("[file: /u/a.rs · 3 lines]");
        assert_eq!(detail(&text, Some(100)), "1 KB · 3줄");
        assert_eq!(detail(&text, None), "3줄");
        let doc = tag("[file: /u/x.docx]");
        assert_eq!(detail(&doc, Some(4096)), "4 KB");
        assert_eq!(detail(&doc, None), "");
        assert_eq!(detail(&tag("[audio: /u/m.m4a · 0:31]"), Some(2048)), "0:31 · 2 KB");
        assert_eq!(kind_of(&doc), Kind::Doc);
        assert_eq!(kind_of(&text), Kind::Text);
        assert_eq!(kind_of(&pdf), Kind::Pdf);
    }
}
