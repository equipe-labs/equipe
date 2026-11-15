//! The design screen's Designs tab: what the daemon last said the squad's
//! design tickets made (`Event::DesignResults`) — one card each — which card
//! is open, which width is compared, and what is new for the person to look
//! at (the Needs-you item and the Status line). No UI types here, so the
//! rules can be tested; `views/design_results.rs` draws it. The daemon reads
//! the folders (this Mac may be a remote one); the pictures come through
//! `read_image`, as the rest of the design screen's do.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use equipe_core::protocol::Command;
use equipe_core::protocol::design::{DesignChange, DesignDirection, DesignShot, DesignState, DesignWork};
use equipe_core::protocol::TicketId;

/// How long a hand-in or an acceptance stays new, unless looked at.
pub const NEW_FOR: u64 = 3 * 86_400;
/// How many new designs are put in front of the person at most.
pub const NEWS_MAX: usize = 5;
/// The answer is asked again after this long while the screen is open, and
/// when the board's design tickets changed.
pub const REFRESH: Duration = Duration::from_secs(90);
/// A render this wide or narrower is a phone's.
pub const MOBILE_UP_TO: u32 = 600;

/// A design the person hasn't looked at: handed in, or accepted by the PM, with a picture to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct News {
    pub ticket: TicketId,
    pub title: String,
    /// Accepted already (by the PM): to look at, nothing to decide.
    pub accepted: bool,
    /// When it was handed in or accepted, Unix seconds.
    pub at: u64,
    pub hero: Option<String>,
}

/// The designs to put in front of the person: handed in or accepted lately
/// with a picture to show, not looked at yet — the latest first.
pub fn news(designs: &[DesignWork], seen: &HashSet<TicketId>, now: u64) -> Vec<News> {
    let mut out: Vec<News> = designs
        .iter()
        .filter(|d| matches!(d.state, DesignState::HandedIn | DesignState::Accepted) && d.hero.is_some() && d.updated > 0 && d.updated + NEW_FOR >= now && !seen.contains(&d.ticket))
        .map(|d| News { ticket: d.ticket, title: d.title.clone(), accepted: d.state == DesignState::Accepted, at: d.updated, hero: d.hero.clone() })
        .collect();
    out.sort_by_key(|n| std::cmp::Reverse((n.at, n.ticket)));
    out.truncate(NEWS_MAX);
    out
}

/// Which renders are compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Viewport {
    #[default]
    Desktop,
    Mobile,
}

impl Viewport {
    pub fn of(width: u32) -> Self {
        if width > 0 && width <= MOBILE_UP_TO { Viewport::Mobile } else { Viewport::Desktop }
    }
}

/// What a card's list row counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badges {
    pub directions: usize,
    pub links: usize,
    pub notes: bool,
}

pub fn badges(work: &DesignWork) -> Badges {
    Badges { directions: work.directions.iter().filter(|d| !d.shots.is_empty()).count(), links: work.links.len(), notes: !work.changes.is_empty() }
}

/// Which widths a card has renders for.
pub fn viewports(work: &DesignWork) -> Vec<Viewport> {
    let all = work.before.iter().chain(work.directions.iter().flat_map(|d| d.shots.iter()));
    let mut out = Vec::new();
    for shot in all {
        let v = Viewport::of(shot.width);
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out.sort_by_key(|v| matches!(v, Viewport::Mobile));
    out
}

/// The render of `shots` for a viewport: the widest desktop one, the phone
/// one nearest a phone's width.
fn pick(shots: &[DesignShot], viewport: Viewport) -> Option<&DesignShot> {
    let of_it = shots.iter().filter(|s| Viewport::of(s.width) == viewport);
    match viewport {
        Viewport::Desktop => of_it.filter(|s| s.width <= 2000).max_by_key(|s| s.width).or_else(|| shots.iter().find(|s| Viewport::of(s.width) == viewport)),
        Viewport::Mobile => of_it.min_by_key(|s| s.width.abs_diff(390)),
    }
}

/// One column of the side-by-side view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// `None`: the screen as it was.
    pub direction: Option<String>,
    pub chosen: bool,
    pub shot: DesignShot,
}

/// The screen as it was and each direction at `viewport`, left to right —
/// a direction with no render at that width is left out.
pub fn columns(work: &DesignWork, viewport: Viewport) -> Vec<Column> {
    let before = pick(&work.before, viewport).map(|shot| Column { direction: None, chosen: false, shot: shot.clone() });
    let directions = work.directions.iter().filter_map(|d| pick(&d.shots, viewport).map(|shot| Column { direction: Some(d.key.clone()), chosen: d.chosen, shot: shot.clone() }));
    before.into_iter().chain(directions).collect()
}

/// What a direction's key is called: `a` → `A`, `calm-ledger` → `Calm ledger`.
pub fn name_of(key: &str) -> String {
    let words: String = key.split(['-', '_']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
    if words.chars().count() == 1 {
        return words.to_uppercase();
    }
    let mut chars = words.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The notes of one direction (or, with an empty key, those for all).
pub fn notes_for<'a>(work: &'a DesignWork, key: &str) -> Vec<&'a DesignChange> {
    work.changes.iter().filter(|c| c.direction == key).collect()
}

/// The directions that have notes but no render, and the notes for all — in
/// the order the notes come — so none is lost for want of a picture.
pub fn note_keys(work: &DesignWork) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in &work.changes {
        if !out.contains(&c.direction) {
            out.push(c.direction.clone());
        }
    }
    // General notes first.
    out.sort_by_key(|k| !k.is_empty());
    out
}

/// A line of the change notes, as they are drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Heading(String),
    Bullet(String),
    Text(String),
}

/// Markdown's emphasis, code ticks and links as plain words.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let (head, tail) = rest.split_at(open);
        out.push_str(head);
        match tail.find("](").and_then(|mid| tail[mid..].find(')').map(|end| (mid, mid + end))) {
            Some((mid, end)) => {
                out.push_str(&tail[1..mid]);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('[');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace("__", "").replace('`', "").trim().to_owned()
}

/// A section of notes as lines: headings, bullets and paragraphs; a table's
/// row as a bullet of its cells; no rules, no table dividers.
pub fn lines(text: &str) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    let mut paragraph = String::new();
    let flush = |out: &mut Vec<Line>, paragraph: &mut String| {
        if !paragraph.trim().is_empty() {
            out.push(Line::Text(plain(paragraph)));
        }
        paragraph.clear();
    };
    for raw in text.lines() {
        let line = raw.trim();
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if line.is_empty() {
            flush(&mut out, &mut paragraph);
        } else if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            flush(&mut out, &mut paragraph);
            out.push(Line::Heading(plain(&line[hashes..])));
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).or_else(|| line.strip_prefix("• ")) {
            flush(&mut out, &mut paragraph);
            out.push(Line::Bullet(plain(item)));
        } else if line.starts_with(|c: char| c.is_ascii_digit()) && line.split_once(". ").is_some_and(|(n, _)| n.chars().all(|c| c.is_ascii_digit())) {
            flush(&mut out, &mut paragraph);
            out.push(Line::Bullet(plain(line.split_once(". ").map_or(line, |(_, rest)| rest))));
        } else if line.starts_with('|') {
            flush(&mut out, &mut paragraph);
            let cells: Vec<String> = line.trim_matches('|').split('|').map(plain).filter(|c| !c.is_empty()).collect();
            let divider = cells.iter().all(|c| c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')));
            if !cells.is_empty() && !divider {
                out.push(Line::Bullet(cells.join(" · ")));
            }
        } else if line.chars().all(|c| matches!(c, '-' | '*' | '_' | '=')) {
            flush(&mut out, &mut paragraph);
        } else {
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(line);
        }
    }
    flush(&mut out, &mut paragraph);
    out
}

/// A link of a card as the screen opens it: the one check every card's link
/// goes through (https, a real host, no credentials, no port), again here
/// whatever a daemon sent. The address and the host to show next to it.
pub fn live_link(url: &str) -> Option<(String, String)> {
    let url = equipe_core::connections::clean_url(url).ok()?;
    let host = equipe_core::connections::host_of(&url);
    Some((url, host))
}

/// The tab's state (`DesignUi::results`).
#[derive(Default)]
pub struct DesignsUi {
    /// What the daemon last said, newest first.
    pub answer: Option<Vec<DesignWork>>,
    /// Asked at this time, not yet answered.
    asked: Option<Instant>,
    /// When the last answer came.
    answered: Option<Instant>,
    /// Read again at the next look: the tab was opened, a design ticket changed.
    pub dirty: bool,
    /// The card open, by its ticket.
    pub open: Option<TicketId>,
    /// The width compared, when the card has more than one.
    pub viewport: Option<Viewport>,
    pub scroll: gpui_kit::ScrollHandle,
}

impl DesignsUi {
    /// The request, when the answer is missing, stale or wanted again — once, until answered.
    pub fn wanted(&mut self, squad: &str, now: Instant) -> Option<Command> {
        if self.asked.is_some_and(|at| now.duration_since(at) < Duration::from_secs(10)) {
            return None;
        }
        let stale = self.answered.is_none_or(|at| now.duration_since(at) >= REFRESH);
        if !self.dirty && self.answer.is_some() && !stale {
            return None;
        }
        self.dirty = false;
        self.asked = Some(now);
        Some(Command::DesignResults { squad: squad.to_owned() })
    }

    /// The daemon's answer. Whether anything changed.
    pub fn arrived(&mut self, designs: Vec<DesignWork>, now: Instant) -> bool {
        self.asked = None;
        self.answered = Some(now);
        if self.answer.as_ref() == Some(&designs) {
            return false;
        }
        // What was open may be gone.
        if self.open.is_some_and(|t| !designs.iter().any(|d| d.ticket == t)) {
            self.open = None;
        }
        self.answer = Some(designs);
        true
    }

    /// Another squad is on screen: what was read is no longer its.
    pub fn forget(&mut self) {
        *self = DesignsUi { scroll: std::mem::take(&mut self.scroll), ..DesignsUi::default() };
    }

    pub fn get(&self, ticket: TicketId) -> Option<&DesignWork> {
        self.answer.as_ref()?.iter().find(|d| d.ticket == ticket)
    }

    pub fn current(&self) -> Option<&DesignWork> {
        self.get(self.open?)
    }

    /// The width on show for `work`: the person's pick when the card has it, else the first it has.
    pub fn viewport_of(&self, work: &DesignWork) -> Viewport {
        let have = viewports(work);
        self.viewport.filter(|v| have.contains(v)).or_else(|| have.first().copied()).unwrap_or_default()
    }
}

/// The direction of a card a column stands for, for its label.
pub fn direction_in<'a>(work: &'a DesignWork, key: &str) -> Option<&'a DesignDirection> {
    work.directions.iter().find(|d| d.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::design::DesignLink;

    fn shot(name: &str, width: u32) -> DesignShot {
        DesignShot { path: format!("/p/design/directions/t7-x/{name}"), name: name.into(), width }
    }

    fn work() -> DesignWork {
        DesignWork {
            ticket: 7,
            title: "Dashboard".into(),
            state: DesignState::HandedIn,
            updated: 1_000_000,
            hero: Some(shot("mock-b-1440.png", 1440).path),
            before: vec![shot("before-1440.png", 1440), shot("before-390.png", 390)],
            directions: vec![
                DesignDirection { key: "a".into(), chosen: false, shots: vec![shot("mock-a-1440.png", 1440), shot("mock-a-390.png", 390)] },
                DesignDirection { key: "b".into(), chosen: true, shots: vec![shot("mock-b-1440.png", 1440)] },
                DesignDirection { key: "calm-ledger".into(), chosen: false, shots: vec![shot("direction-calm-ledger-1440.png", 1440), shot("direction-calm-ledger-390.png", 390)] },
            ],
            changes: vec![
                DesignChange { direction: "a".into(), title: "A".into(), text: "- x".into() },
                DesignChange { direction: String::new(), title: "Unchanged".into(), text: "Copy.".into() },
                DesignChange { direction: "d".into(), title: "D".into(), text: "- y".into() },
            ],
            links: vec![DesignLink { url: "https://x.vercel.app/a".into(), ..Default::default() }],
            ..Default::default()
        }
    }

    #[test]
    fn the_screen_as_it_was_and_each_direction_are_columns_at_one_width() {
        let w = work();
        let names = |v: Viewport| columns(&w, v).iter().map(|c| (c.direction.clone(), c.shot.name.clone())).collect::<Vec<_>>();
        assert_eq!(
            names(Viewport::Desktop),
            [(None, "before-1440.png".to_owned()), (Some("a".into()), "mock-a-1440.png".to_owned()), (Some("b".into()), "mock-b-1440.png".to_owned()), (Some("calm-ledger".into()), "direction-calm-ledger-1440.png".to_owned())]
        );
        // B has no phone render: left out, not shown at the wrong width.
        assert_eq!(names(Viewport::Mobile).iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(), [None, Some("a".into()), Some("calm-ledger".into())]);
        assert!(columns(&w, Viewport::Desktop)[2].chosen);
        assert_eq!(viewports(&w), [Viewport::Desktop, Viewport::Mobile]);
        let only_desktop = DesignWork { before: vec![shot("before.png", 0)], directions: vec![DesignDirection { key: String::new(), chosen: false, shots: vec![shot("after-1440.png", 1440)] }], ..Default::default() };
        assert_eq!(viewports(&only_desktop), [Viewport::Desktop], "a render with no width is a desktop's");
        assert_eq!(columns(&only_desktop, Viewport::Desktop).len(), 2);
    }

    #[test]
    fn badges_count_what_a_card_holds() {
        assert_eq!(badges(&work()), Badges { directions: 3, links: 1, notes: true });
        assert_eq!(badges(&DesignWork::default()), Badges { directions: 0, links: 0, notes: false });
    }

    #[test]
    fn a_direction_is_named_as_a_person_would_and_notes_without_a_render_are_kept() {
        assert_eq!((name_of("a").as_str(), name_of("calm-ledger").as_str(), name_of("").as_str()), ("A", "Calm ledger", ""));
        let w = work();
        assert_eq!(note_keys(&w), ["", "a", "d"], "the notes for all first, then each direction with notes, rendered or not");
        assert_eq!(notes_for(&w, "a").len(), 1);
        assert!(direction_in(&w, "d").is_none());
    }

    #[test]
    fn new_designs_are_the_handed_in_or_accepted_ones_with_a_picture_not_looked_at_and_not_old() {
        let now = 1_000_000 + 3600;
        let mut accepted = work();
        accepted.ticket = 8;
        accepted.state = DesignState::Accepted;
        accepted.updated += 10;
        let mut working = work();
        working.ticket = 9;
        working.state = DesignState::Working;
        let mut bare = work();
        bare.ticket = 10;
        bare.hero = None;
        let mut old = work();
        old.ticket = 11;
        old.updated = 1;
        let designs = [work(), accepted, working, bare, old];
        let list = news(&designs, &HashSet::new(), now);
        assert_eq!(list.iter().map(|n| (n.ticket, n.accepted)).collect::<Vec<_>>(), [(8, true), (7, false)], "latest first");
        assert_eq!(news(&designs, &HashSet::from([8]), now).iter().map(|n| n.ticket).collect::<Vec<_>>(), [7], "looked at: gone");
        assert!(news(&designs, &HashSet::new(), now + NEW_FOR + 100).is_empty(), "new for a few days only");
    }

    #[test]
    fn notes_read_as_headings_bullets_and_paragraphs_without_markdown_marks() {
        let md = "### Before → After → Why\n- **Rows** 56px → 40px, [references](https://x.example.com/r)\n\n| Area | After |\n|---|---|\n| Type | `Inter 15/24` |\n\nOne wrapped\nparagraph.\n---\n1. First\n";
        assert_eq!(
            lines(md),
            [
                Line::Heading("Before → After → Why".into()),
                Line::Bullet("Rows 56px → 40px, references".into()),
                Line::Bullet("Area · After".into()),
                Line::Bullet("Type · Inter 15/24".into()),
                Line::Text("One wrapped paragraph.".into()),
                Line::Bullet("First".into()),
            ]
        );
        assert_eq!(plain("a [b](c) [d"), "a b [d");
        assert!(lines("").is_empty());
    }

    #[test]
    fn a_live_link_is_https_with_a_real_host_whatever_a_daemon_sent() {
        assert_eq!(live_link("https://t7-a-git-x-team.vercel.app/a"), Some(("https://t7-a-git-x-team.vercel.app/a".into(), "t7-a-git-x-team.vercel.app".into())));
        for bad in ["http://x.vercel.app/", "javascript:alert(1)", "file:///etc/passwd", "https://user:pw@x.vercel.app/", "https://localhost/", "https://x.vercel.app:8443/", "https://x.vercel.app/a b", "equipe://open/x/status", ""] {
            assert_eq!(live_link(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_answer_is_asked_once_and_again_when_stale_or_wanted_and_a_card_that_went_closes() {
        let mut ui = DesignsUi::default();
        let t0 = Instant::now();
        assert_eq!(ui.wanted("acme", t0), Some(Command::DesignResults { squad: "acme".into() }));
        assert_eq!(ui.wanted("acme", t0 + Duration::from_secs(1)), None, "asked, not answered");
        ui.open = Some(7);
        assert!(ui.arrived(vec![work()], t0 + Duration::from_secs(2)));
        assert!(!ui.arrived(vec![work()], t0 + Duration::from_secs(3)), "the same answer changes nothing");
        assert_eq!(ui.wanted("acme", t0 + Duration::from_secs(30)), None, "fresh");
        ui.dirty = true;
        assert!(ui.wanted("acme", t0 + Duration::from_secs(30)).is_some(), "wanted again");
        assert!(ui.arrived(vec![], t0 + Duration::from_secs(31)));
        assert_eq!(ui.open, None, "the card open is gone");
        assert!(ui.wanted("acme", t0 + REFRESH + Duration::from_secs(40)).is_some(), "stale");
    }
}
