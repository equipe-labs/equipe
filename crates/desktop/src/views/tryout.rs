//! A decision's options tried live: each option's prototype (`try`) — a
//! loopback page its member serves on the daemon's machine, or an HTML page
//! it saved that the daemon serves (`prototype_url`) — in a real web view of
//! this screen, every option at once and at one size, side by side, for the
//! person to hover, press, tab through and type in. "Try it" on one option's
//! tile opens just that one in the live preview pane (`super::preview`).
//!
//! With the daemon on another Mac each page's port is forwarded here over the
//! window's own SSH link (`crate::tunnel`), as the live preview's is, and
//! taken down when side by side is put away. Pictures stay each option's
//! thumbnail on the card.

use std::cell::RefCell;
use std::rc::Rc;

use equipe_core::live_preview as rules;
use equipe_core::protocol::Command;
use equipe_core::protocol::decide::Choice;
use equipe_core::t;
use gpui_kit::*;

use crate::Shell;
use crate::preview_view::PreviewWebView;
use crate::theme::{Palette, R_MD};

/// Where one option's live page stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Its address being found (asked of the daemon, forwarded here).
    Finding,
    /// Shown from this address, here.
    Ready(String),
    /// Couldn't be had: why.
    Failed(String),
}

/// One option's live page.
pub struct Page {
    pub option: usize,
    pub phase: Phase,
    /// Its address on the daemon's machine, as the member's server or the daemon serves it.
    served: Option<String>,
    tunnel: Option<crate::tunnel::Tunnel>,
    view: Option<PreviewWebView>,
    /// What the page tells: kept, unread — the pane's keys are the preview pane's.
    _events: Option<tokio::sync::mpsc::UnboundedReceiver<crate::preview_view::WebEvent>>,
    /// Where it was last drawn, in the window's points.
    pub area: Rc<RefCell<Option<Bounds<Pixels>>>>,
}

/// A question's options tried side by side.
pub struct Tryout {
    pub set: u64,
    pub step: usize,
    pub pages: Vec<Page>,
}

impl Tryout {
    /// Whether option `n` has a live page here.
    pub fn page(&self, n: usize) -> Option<&Page> {
        self.pages.iter().find(|p| p.option == n)
    }
}

/// What an option's `try` is shown from, as it is asked: its address as it is,
/// or the saved page's path to ask the daemon about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Address(String),
    Saved(String),
}

/// An option's live prototype, if it has one.
pub fn source_of(option: &Choice) -> Option<Source> {
    let given = option.try_it.trim();
    if given.is_empty() {
        None
    } else if equipe_core::prototype::is_address(given) {
        rules::loopback(given).ok().map(|_| Source::Address(given.to_owned()))
    } else {
        Some(Source::Saved(given.to_owned()))
    }
}

/// Each of `n` live pages side by side in `width` (gaps `gap` between): one width for all.
pub fn page_width(width: f32, n: usize, gap: f32) -> f32 {
    let n = n.max(1) as f32;
    ((width - gap * (n - 1.)) / n).floor().max(1.)
}

impl Shell {
    /// Find option `n`'s page address on the daemon's machine (asked of the
    /// daemon for a saved page), then `then` with it.
    fn try_find(&mut self, id: u64, step: usize, n: usize, cx: &mut Context<Self>, then: impl FnOnce(&mut Shell, Result<String, String>, &mut Context<Shell>) + 'static) {
        let source = self.model.asked.get(id).and_then(|s| s.questions.get(step)).and_then(|q| q.options.get(n)).and_then(source_of);
        let Some(source) = source else { return then(self, Err(t!("decide.try_none").to_owned()), cx) };
        match source {
            Source::Address(url) => then(self, Ok(url), cx),
            Source::Saved(path) => {
                let Some(uploader) = self.link.as_ref().map(|l| l.uploader()) else { return then(self, Err(t!("daemon.not_connected").to_owned()), cx) };
                let squad = self.model.squad.clone();
                // The tool's pictures: a made-up address, as if the daemon served it.
                if cfg!(feature = "snapshot") {
                    return then(self, Ok(format!("http://127.0.0.1:4899/{}", crate::visuals::name_of(&path))), cx);
                }
                let asked = cx.background_spawn(async move { uploader.prototype_url(&squad, &path) });
                cx.spawn(async move |this, cx| {
                    let url = asked.await;
                    this.update(cx, |this, cx| then(this, url, cx)).ok();
                })
                .detach();
            }
        }
    }

    /// "Try it" on option `n`'s tile: its live page in the live preview pane,
    /// opened once its address is known (`decide_try_settle`, with the window).
    pub fn decide_try(&mut self, id: u64, step: usize, n: usize, cx: &mut Context<Self>) {
        let session = self.model.asked.get(id).map(|s| s.session);
        self.try_find(id, step, n, cx, move |this, url, cx| {
            match url {
                Ok(url) => this.decide.try_pending = Some((url, session)),
                Err(why) => this.say(t!("decide.try_failed", why = why)),
            }
            cx.notify();
        });
    }

    /// What "Try it" found, opened in the pane now that a window is at hand.
    pub fn decide_try_settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((url, session)) = self.decide.try_pending.take()
            && !self.open_live_preview(&url, session, false, window, cx)
        {
            self.say(t!("decide.try_failed", why = url));
        }
    }

    /// Side by side opened on a question with live prototypes: each option's page found, forwarded.
    pub fn tryout_start(&mut self, id: u64, step: usize, cx: &mut Context<Self>) {
        self.tryout_stop(cx);
        let Some(q) = self.model.asked.get(id).and_then(|s| s.questions.get(step)).cloned() else { return };
        let options: Vec<usize> = q.options.iter().enumerate().filter(|(_, o)| source_of(o).is_some()).map(|(n, _)| n).collect();
        if options.is_empty() {
            return;
        }
        self.decide.tryout = Some(Tryout { set: id, step, pages: options.iter().map(|&n| Page { option: n, phase: Phase::Finding, served: None, tunnel: None, view: None, _events: None, area: Rc::default() }).collect() });
        for n in options {
            self.try_find(id, step, n, cx, move |this, url, cx| this.tryout_found(id, step, n, url, cx));
        }
        cx.notify();
    }

    /// Option `n`'s page address on the daemon's machine came: shown from here,
    /// forwarded first when the daemon is on another Mac.
    fn tryout_found(&mut self, id: u64, step: usize, n: usize, url: Result<String, String>, cx: &mut Context<Self>) {
        let session = self.model.asked.get(id).map(|s| s.session);
        let squad = self.model.squad.clone();
        let host = match &self.endpoint {
            crate::target::Endpoint::Ssh(host) if !cfg!(feature = "snapshot") => Some(host.clone()),
            _ => None,
        };
        let remote = self.endpoint.is_remote();
        let Some(page) = self.tryout_page(id, step, n) else { return };
        let target = match url.and_then(|u| rules::loopback(&u).map(|t| (u, t)).map_err(|_| t!("decide.try_not_local").to_owned())) {
            Ok(found) => found,
            Err(why) => {
                page.phase = Phase::Failed(why);
                return cx.notify();
            }
        };
        let (url, target) = target;
        page.served = Some(url.clone());
        let ready = match &host {
            Some(_) if remote => None,
            _ if remote => Some(Phase::Failed(t!("decide.try_no_link").to_owned())),
            _ => Some(Phase::Ready(url.clone())),
        };
        if let Some(phase) = ready {
            page.phase = phase;
        }
        // While the person tries it, the member keeps its server running.
        self.send_command(Command::LivePreview { squad, session, url: target.page(), open: true });
        match host {
            Some(host) if remote => {
                let opened = cx.background_spawn(async move {
                    match rules::local_port(target.port, crate::tunnel::port_free, crate::tunnel::any_free_port) {
                        Some(local) => crate::tunnel::open_to(&host, local, target.port).map(|t| (target.at(t.port()), t)),
                        None => Err("no free port on this Mac".to_owned()),
                    }
                });
                cx.spawn(async move |this, cx| {
                    let opened = opened.await;
                    this.update(cx, |this, cx| {
                        let Some(page) = this.tryout_page(id, step, n) else {
                            if let Ok((_, tunnel)) = opened {
                                std::thread::spawn(move || tunnel.close());
                            }
                            return;
                        };
                        match opened {
                            Ok((local, tunnel)) => {
                                page.tunnel = Some(tunnel);
                                page.phase = Phase::Ready(local);
                            }
                            Err(why) => page.phase = Phase::Failed(why),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            _ => {}
        }
        cx.notify();
    }

    fn tryout_page(&mut self, id: u64, step: usize, n: usize) -> Option<&mut Page> {
        self.decide.tryout.as_mut().filter(|t| t.set == id && t.step == step)?.pages.iter_mut().find(|p| p.option == n)
    }

    /// Side by side put away: its web views closed, its forwards taken down, the members told.
    pub fn tryout_stop(&mut self, cx: &mut Context<Self>) {
        let Some(tryout) = self.decide.tryout.take() else { return };
        let session = self.model.asked.get(tryout.set).map(|s| s.session);
        for page in tryout.pages {
            if let Some(tunnel) = page.tunnel {
                std::thread::spawn(move || tunnel.close());
            }
            if let Some(url) = page.served.and_then(|u| rules::loopback(&u).ok()) {
                self.send_command(Command::LivePreview { squad: self.model.squad.clone(), session, url: url.page(), open: false });
            }
        }
        cx.notify();
    }

    /// Each live page's web view where its box was last drawn: opened once it
    /// has an address and a box, shown only while side by side is on top.
    pub fn tryout_sync(&mut self, window: &mut Window) {
        let covered = self.viewer.is_some() || self.overlay_open() || self.decide.side.is_none();
        let zoom = window.content_zoom();
        let Some(tryout) = self.decide.tryout.as_mut() else { return };
        for page in &mut tryout.pages {
            let area = *page.area.borrow();
            let at = area.map(|b| (f32::from(b.origin.x) * zoom, f32::from(b.origin.y) * zoom, f32::from(b.size.width) * zoom, f32::from(b.size.height) * zoom));
            let visible = !covered && at.is_some();
            let ready = match &page.phase {
                Phase::Ready(url) if page.view.is_none() && !cfg!(feature = "snapshot") => Some(url.clone()),
                _ => None,
            };
            if let (Some(url), Some(at)) = (ready, at.filter(|_| visible)) {
                let (events, received) = tokio::sync::mpsc::unbounded_channel();
                match PreviewWebView::open(window, &url, at, events) {
                    Ok(view) => {
                        page.view = Some(view);
                        page._events = Some(received);
                    }
                    Err(why) => page.phase = Phase::Failed(why),
                }
                continue;
            }
            if let Some(view) = page.view.as_mut() {
                if let Some(at) = at.filter(|_| visible) {
                    view.set_bounds(at);
                }
                view.set_visible(visible);
            }
        }
    }
}

/// Option `n`'s live box, `w`×`h`: the web view goes over it; while its page
/// comes, or when it couldn't, the box says so.
pub fn live_box(p: Palette, page: &Page, (w, h): (f32, f32)) -> AnyElement {
    let measured = page.area.clone();
    let words = match &page.phase {
        Phase::Finding => Some(t!("decide.try_finding").to_owned()),
        Phase::Failed(why) => Some(t!("decide.try_failed", why = why)),
        // The tool's pictures have no web view: the box says where it draws.
        Phase::Ready(url) if cfg!(feature = "snapshot") => Some(url.clone()),
        Phase::Ready(_) => None,
    };
    div()
        .relative()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .rounded(px(R_MD))
        .overflow_hidden()
        .bg(p.bg)
        .child(
            canvas(
                move |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    let moved = measured.borrow().is_none_or(|b| b != bounds);
                    *measured.borrow_mut() = Some(bounds);
                    if moved {
                        window.refresh();
                    }
                },
            )
            .absolute()
            .size_full(),
        )
        .children(words.map(|words| div().absolute().inset_0().p_4().flex().items_center().justify_center().text_center().text_size(px(12.5)).text_color(p.muted).child(words)))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude brings a `test` macro of its own.
    use super::{Choice, Source, page_width, source_of};

    #[test]
    fn an_options_prototype_is_a_local_page_or_a_saved_one_and_nothing_else() {
        let option = |t: &str| Choice { label: "A".into(), try_it: t.into(), ..Default::default() };
        assert_eq!(source_of(&option("http://localhost:5173/states")), Some(Source::Address("http://localhost:5173/states".into())));
        assert_eq!(source_of(&option("/w/design/proto/a/index.html")), Some(Source::Saved("/w/design/proto/a/index.html".into())));
        assert_eq!(source_of(&option("https://acme.vercel.app/")), None, "never another host's");
        assert_eq!(source_of(&option(" ")), None);
    }

    #[test]
    fn live_pages_side_by_side_are_one_width() {
        assert_eq!(page_width(1216., 3, 16.), 394.);
        assert_eq!(page_width(1216., 2, 16.), 600.);
        assert_eq!(page_width(10., 0, 16.), 10.);
    }
}
