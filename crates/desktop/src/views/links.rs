//! Links in what a member asks, drawn as rows under the words: an icon, what
//! the link is ("Open AWS sign-in" and its host under it), and what can be
//! done — open it here (a sign-in page, in the sign-in view of the member's
//! browser the person was handed), open it, copy it. A link opens on the Mac
//! the person is looking at (this app's own), not the daemon's.

use equipe_core::protocol::SessionId;
use equipe_core::protocol::links::{self, Link};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::Shell;
use crate::icons::{Icon, icon};
use crate::theme::{MONO, Palette, R_MD};
use crate::ui::{self, Kind};

/// What a link's row can do, the first being the main one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// In the sign-in view of the member's browser, in this window.
    Here,
    /// In the Mac's own browser.
    Open,
    Copy,
}

/// A row's actions: "Open here" leads when the link is a sign-in page and a
/// sign-in view of this member's hand-off is open (`view`); else the Mac's browser.
pub fn acts(link: &Link, view: bool) -> Vec<Act> {
    let sign_in = links::looks(&link.url).is_some_and(|l| l.sign_in);
    if view && sign_in { vec![Act::Here, Act::Open, Act::Copy] } else { vec![Act::Open, Act::Copy] }
}

/// A link's words: what it is, and where it goes. The member's label as
/// given; else named by its host — "Open AWS sign-in" for a sign-in page of a
/// known service, "Open sign-in page", "Open GitHub", "Open link".
pub fn caption(link: &Link) -> (String, String) {
    let looks = links::looks(&link.url);
    let host = looks.as_ref().map(|l| l.host.clone()).unwrap_or_else(|| link.url.clone());
    let label = link.label.trim();
    if !label.is_empty() {
        return (label.to_owned(), host);
    }
    let name = match &looks {
        Some(l) => match (l.sign_in, l.service) {
            (true, Some(service)) => t!("links.open_signin_of", service = service).into(),
            (true, None) => t!("links.open_signin").into(),
            (false, Some(service)) => t!("links.open_service", service = service).into(),
            (false, None) => t!("links.open").into(),
        },
        None => t!("links.open").into(),
    };
    (name, host)
}

/// The links as rows, one under another; none when there are none. `view`:
/// the session whose sign-in view is open (where "Open here" goes).
pub fn rows(p: Palette, key: u64, list: &[Link], view: Option<SessionId>, handle: &WeakEntity<Shell>) -> Option<AnyElement> {
    if list.is_empty() {
        return None;
    }
    let rows = list.iter().enumerate().map(|(n, link)| row(p, key, n, link, view, handle));
    Some(v_flex().w_full().gap_1p5().children(rows).into_any_element())
}

fn row(p: Palette, key: u64, n: usize, link: &Link, view: Option<SessionId>, handle: &WeakEntity<Shell>) -> AnyElement {
    let (name, host) = caption(link);
    let sign_in = links::looks(&link.url).is_some_and(|l| l.sign_in);
    let id = crate::render::hash_of((key, n));
    let buttons = acts(link, view.is_some()).into_iter().enumerate().map(|(k, act)| {
        let url = link.url.clone();
        let handle = handle.clone();
        let (label, glyph, kind, tip) = match act {
            Act::Here => (t!("links.open_here"), Icon::External, Kind::Primary, t!("links.open_here_tip")),
            Act::Open => (t!("links.open_browser"), Icon::External, if k == 0 { Kind::Primary } else { Kind::Secondary }, t!("links.open_browser_tip")),
            Act::Copy => (t!("links.copy"), Icon::Copy, Kind::Ghost, t!("links.copy_tip")),
        };
        ui::button(p, ("link-act", crate::render::hash_of((id, k))), kind, label).field().icon(glyph).tip(tip, None).on_click(move |_, window, cx| {
            let url = url.clone();
            handle
                .update(cx, |shell, cx| match act {
                    Act::Here => shell.open_link_here(view, &url, window, cx),
                    Act::Open => cx.open_url(&url),
                    Act::Copy => {
                        cx.write_to_clipboard(ClipboardItem::new_string(url));
                        shell.composer.flash = Some(t!("links.copied").into());
                        cx.notify();
                    }
                })
                .ok();
        })
    });
    h_flex()
        .id(("link-row", id))
        .w_full()
        .min_w_0()
        .px_3()
        .py(px(6.))
        .gap_2p5()
        .items_center()
        .rounded(px(R_MD))
        .bg(p.wash)
        .border_1()
        .border_color(p.hair)
        .child(icon(if sign_in { Icon::Lock } else { Icon::Globe }, 15., p.accent))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().truncate().text_size(px(13.5)).line_height(px(19.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(name))
                .child(div().truncate().font_family(MONO).text_size(px(11.)).line_height(px(15.)).text_color(p.muted).child(host)),
        )
        .child(h_flex().flex_none().gap_1().children(buttons))
        .into_any_element()
}

impl Shell {
    /// The session whose sign-in view is open in the browser panel, if it is `session`'s.
    pub fn link_view_of(&self, session: SessionId, cx: &App) -> Option<SessionId> {
        self.browser.read(cx).has_view_for(session).then_some(session)
    }

    /// "Open here": the sign-in view of the member's browser goes to `url`.
    /// Without one open, the Mac's browser does.
    pub fn open_link_here(&mut self, session: Option<SessionId>, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        let went = session.is_some_and(|s| self.browser.update(cx, |b, cx| b.open_here(s, url, window, cx)));
        if !went {
            cx.open_url(url);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Act, acts, caption};
    use equipe_core::protocol::links::Link;
    use equipe_core::t;

    fn link(label: &str, url: &str) -> Link {
        Link { label: label.into(), url: url.into() }
    }

    const AWS: &str = "https://ap-northeast-2.signin.aws.amazon.com/v1/authorize?response_type=code&client_id=arn%3Aaws";

    #[test]
    fn a_link_is_named_by_its_label_else_by_what_its_host_says() {
        assert_eq!(caption(&link("AWS sign-in", AWS)), ("AWS sign-in".to_owned(), "ap-northeast-2.signin.aws.amazon.com".to_owned()));
        let (name, host) = caption(&link("", AWS));
        let named: String = t!("links.open_signin_of", service = "AWS").into();
        assert_eq!(name, named);
        assert_eq!(host, "ap-northeast-2.signin.aws.amazon.com");
        let sign_in: String = t!("links.open_signin").into();
        assert_eq!(caption(&link("", "https://example.org/device?c=1")).0, sign_in);
        let github: String = t!("links.open_service", service = "GitHub").into();
        assert_eq!(caption(&link("", "https://github.com/org/repo/pull/3")).0, github);
        let plain: String = t!("links.open").into();
        assert_eq!(caption(&link("", "https://example.org/doc")).0, plain);
    }

    #[test]
    fn open_here_leads_only_for_a_sign_in_page_with_the_members_view_open() {
        assert_eq!(acts(&link("", AWS), true), [Act::Here, Act::Open, Act::Copy]);
        assert_eq!(acts(&link("", AWS), false), [Act::Open, Act::Copy], "no view open: the system browser leads");
        assert_eq!(acts(&link("", "https://example.org/doc"), true), [Act::Open, Act::Copy], "not a sign-in page");
    }
}
