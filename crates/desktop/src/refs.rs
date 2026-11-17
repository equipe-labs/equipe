//! An equipe link pasted into the composer: it is a chip — the ticket's or
//! conversation's title — instead of a raw address, and what is sent says
//! which ticket or conversation it is: its title and the link, one line each,
//! before the person's own words (what "Copy as text" copies, so a member who
//! reads it knows `T-2391` as it does anywhere on the board).
//!
//! Only a paste that is nothing but a link — or a link with its title, as
//! "Copy as text" makes it — becomes a chip; a link inside a sentence stays
//! words. Plain functions here; the paste and the chips' row are `views`'.

use equipe_core::protocol::deeplink::{self, DeepLink, Target};
use gpui_kit::Context;

use crate::Shell;

/// The longest title a pasted "title · link" may have to be taken as one.
const TITLE_MAX: usize = 200;

/// A link in the composer, and what its chip says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    pub link: DeepLink,
    pub label: String,
}

/// The link a paste is, with the title it came with if it has one
/// ("T-2391 title · equipe://…"): `None` for any other text.
pub fn pasted(text: &str) -> Option<(DeepLink, Option<String>)> {
    let text = text.trim();
    let found = deeplink::find(text);
    let [(range, link)] = found.as_slice() else { return None };
    let (before, after) = (text[..range.start].trim(), text[range.end..].trim());
    // Nothing after it (a closing bracket or a full stop is not words).
    if !after.chars().all(|c| !c.is_alphanumeric()) {
        return None;
    }
    let title = before.trim_end_matches(['·', '-', '–', '—', ':', '<', '(', '[', '|']).trim();
    let wrapped_only = title.is_empty() || title.chars().all(|c| !c.is_alphanumeric());
    if wrapped_only {
        return Some((link.clone(), None));
    }
    if title.chars().count() > TITLE_MAX || title.contains('\n') {
        return None;
    }
    Some((link.clone(), Some(title.to_owned())))
}

/// What goes ahead of the person's words: each reference as "title · link".
pub fn message(refs: &[Ref], typed: &str) -> String {
    let lines = refs.iter().map(|r| r.link.as_text(&r.label)).collect::<Vec<_>>().join("\n");
    if typed.trim().is_empty() { lines } else { format!("{lines}\n\n{}", typed.trim_start()) }
}

impl Shell {
    /// Whether `link` is about this window's squad on this window's machine.
    pub fn is_here(&self, link: &DeepLink) -> bool {
        link.squad.eq_ignore_ascii_case(&self.model.squad) && link.host.as_deref().is_none_or(|h| crate::deeplink::names(&self.endpoint, &self.connect.peers).iter().any(|n| deeplink::host_key(n) == deeplink::host_key(h)))
    }

    /// The chip a pasted `text` makes, if it is a link: its title as this
    /// squad calls it, else the title it came with, else what it points at.
    pub fn link_chip(&self, text: &str) -> Option<Ref> {
        let (link, came_with) = pasted(text)?;
        let known = self.is_here(&link) && match &link.target {
            Target::Ticket(id) => self.model.tickets.iter().any(|t| t.id == *id),
            Target::Conversation(s) => self.model.conversations().iter().any(|l| l.session == *s),
            Target::Member(name) => self.model.open_member(name).is_some(),
            _ => true,
        };
        let label = if known {
            self.link_title(&link.target)
        } else {
            came_with.unwrap_or_else(|| format!("{} · {}", crate::deeplink::what(&link.target), link.squad))
        };
        Some(Ref { link, label })
    }

    /// Put a chip in the box's row (once).
    pub fn add_ref(&mut self, chip: Ref, cx: &mut Context<Self>) {
        if !self.composer.refs.iter().any(|r| r.link == chip.link) {
            self.composer.refs.push(chip);
        }
        cx.notify();
    }

    pub fn remove_ref(&mut self, at: usize, cx: &mut Context<Self>) {
        if at < self.composer.refs.len() {
            self.composer.refs.remove(at);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link() -> DeepLink {
        DeepLink::new("web", Target::Ticket(2391), Some("mini".into()))
    }

    #[test]
    fn a_bare_link_is_a_chip_in_either_form() {
        for text in [link().uri(), link().web(), format!("  <{}> ", link().uri()), format!("{}.", link().uri())] {
            assert_eq!(pasted(&text), Some((link(), None)), "{text}");
        }
    }

    #[test]
    fn a_link_with_its_title_keeps_the_title_for_the_chip() {
        let text = link().as_text("T-2391 구글시트 4개 공유하기");
        assert_eq!(pasted(&text), Some((link(), Some("T-2391 구글시트 4개 공유하기".into()))));
    }

    #[test]
    fn a_link_in_a_sentence_stays_words() {
        for text in [format!("can you look at {} please", link().uri()), format!("{} is stuck", link().uri()), format!("{} {}", link().uri(), "equipe://open/web/status"), "no link here".to_owned(), String::new()] {
            assert_eq!(pasted(&text), None, "{text}");
        }
        let long = format!("{} · {}", "word ".repeat(80), link().uri());
        assert_eq!(pasted(&long), None, "a paragraph with a link at its end is not a title");
        assert_eq!(pasted(&format!("a\nb · {}", link().uri())), None);
    }

    #[test]
    fn what_is_sent_names_the_ticket_before_the_words() {
        let refs = [Ref { link: link(), label: "T-2391 구글시트 4개 공유하기".into() }, Ref { link: DeepLink::new("web", Target::Conversation(7), None), label: "Pricing page".into() }];
        assert_eq!(
            message(&refs, "  what is blocking this?"),
            "T-2391 구글시트 4개 공유하기 · equipe://open/web/ticket/T-2391?host=mini\nPricing page · equipe://open/web/conversation/7\n\nwhat is blocking this?"
        );
        assert_eq!(message(&refs[..1], "  "), "T-2391 구글시트 4개 공유하기 · equipe://open/web/ticket/T-2391?host=mini", "from an empty box: only the reference");
    }
}
