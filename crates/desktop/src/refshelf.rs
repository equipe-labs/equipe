//! The design screen's References tab: what the daemon last said a design
//! folder's references are (`Event::DesignRefs`) — patterns, each with its
//! pictures and the entries of its notes — which pattern is open, which
//! picture is big, and the search. No UI types here, so the rules can be
//! tested; `views/design_refs.rs` draws it. The daemon reads the folders
//! (this Mac may be a remote one) and pages them; the pictures come through
//! `read_image`, as the rest of the design screen's do.

use equipe_core::protocol::Command;
use equipe_core::protocol::design::{RefNote, RefPattern, RefPicture, RefSource};

/// What a request was for: the project (none: the squad's own), the words
/// searched, and the page's start.
type Key = (Option<String>, String, u32);

/// The daemon's answer, as far as it has come.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Answer {
    pub project: Option<String>,
    pub query: String,
    /// How many patterns match; `patterns` are the ones that came.
    pub total: usize,
    pub patterns: Vec<RefPattern>,
    pub sources: Vec<RefSource>,
    pub folder: String,
}

/// The tab's state (`DesignUi::refs`).
#[derive(Default)]
pub struct RefsUi {
    pub answer: Option<Answer>,
    /// The last request made, until it is answered.
    asked: Option<Key>,
    /// The words the daemon was last asked for: the search once typing rests.
    pub query: String,
    /// Read again at the next look: the tab was opened, or the folder moved.
    pub dirty: bool,
    /// The pattern open, by its folder's name.
    pub pattern: Option<String>,
    /// The picture big: its place among the pictures the open pattern shows.
    pub viewer: Option<usize>,
    pub sources_open: bool,
    /// `notes.md` as written, unfolded.
    pub raw_open: bool,
    pub scroll: gpui_kit::ScrollHandle,
}

/// One card of a pattern: a picture, its note, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub picture: Option<usize>,
    pub note: Option<usize>,
}

/// What is big, and what to say of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewing {
    pub path: String,
    pub name: String,
    pub said: String,
    pub many: bool,
    /// Where it was seen, when the note says and it is a web address.
    pub url: Option<String>,
}

impl RefsUi {
    /// The request for what is on screen, when it isn't what was last asked
    /// (or the folder may have moved): once, until answered.
    pub fn wanted(&mut self, squad: &str, project: Option<&str>) -> Option<Command> {
        let key: Key = (project.map(str::to_owned), self.query.clone(), 0);
        if !self.dirty && self.asked.as_ref() == Some(&key) {
            return None;
        }
        self.dirty = false;
        self.asked = Some(key.clone());
        Some(Command::DesignRefs { squad: squad.to_owned(), project: key.0, query: key.1, offset: 0 })
    }

    /// The next page of the patterns, when there is one and it isn't asked.
    pub fn more(&mut self, squad: &str, project: Option<&str>) -> Option<Command> {
        let answer = self.answer.as_ref()?;
        let offset = answer.patterns.len();
        if offset >= answer.total || answer.project.as_deref() != project || answer.query != self.query {
            return None;
        }
        let key: Key = (project.map(str::to_owned), self.query.clone(), offset as u32);
        if self.asked.as_ref() == Some(&key) {
            return None;
        }
        self.asked = Some(key.clone());
        Some(Command::DesignRefs { squad: squad.to_owned(), project: key.0, query: key.1, offset: key.2 })
    }

    /// The search settled on `query`: asked at the next look.
    pub fn search(&mut self, query: &str) -> bool {
        let query = query.trim();
        if query == self.query {
            return false;
        }
        self.query = query.to_owned();
        true
    }

    /// The daemon's page. A later page of the same ask is added to the ones
    /// before it; one for another project or other words than are on screen
    /// is an old answer and is let go. Whether anything changed.
    #[allow(clippy::too_many_arguments)]
    pub fn arrived(&mut self, on_screen: Option<&str>, project: Option<String>, query: String, offset: u32, total: u32, patterns: Vec<RefPattern>, sources: Vec<RefSource>, folder: String) -> bool {
        if project.as_deref() != on_screen || query != self.query {
            return false;
        }
        if self.asked.as_ref().is_some_and(|(p, q, o)| *p == project && *q == query && *o == offset) {
            self.asked = None;
        }
        let total = total as usize;
        match self.answer.as_mut() {
            Some(a) if offset > 0 && a.project == project && a.query == query && a.patterns.len() == offset as usize => {
                a.patterns.extend(patterns);
                a.total = total;
            }
            _ => self.answer = Some(Answer { project, query, total, patterns, sources, folder }),
        }
        // What was open may be gone.
        if self.pattern.as_ref().is_some_and(|name| self.answer.as_ref().is_none_or(|a| !a.patterns.iter().any(|p| &p.name == name))) && self.answer.as_ref().is_some_and(|a| a.patterns.len() >= a.total) {
            self.pattern = None;
        }
        if self.viewer.is_some_and(|at| at >= self.pictures().len()) {
            self.viewer = None;
        }
        true
    }

    /// Another folder is on screen: what was read is no longer its.
    pub fn forget(&mut self) {
        self.answer = None;
        self.asked = None;
        self.pattern = None;
        self.viewer = None;
        self.dirty = true;
    }

    pub fn open(&self) -> Option<&RefPattern> {
        let name = self.pattern.as_ref()?;
        self.answer.as_ref()?.patterns.iter().find(|p| &p.name == name)
    }

    /// The pictures the open pattern shows, big, in order: each card's, as filtered.
    pub fn pictures(&self) -> Vec<(usize, Option<usize>)> {
        let Some(pattern) = self.open() else { return Vec::new() };
        entries(pattern).into_iter().filter(|e| entry_matches(pattern, e, &self.query)).filter_map(|e| Some((e.picture?, e.note))).collect()
    }

    pub fn viewing(&self) -> Option<Viewing> {
        let pattern = self.open()?;
        let shown = self.pictures();
        let (pic, note) = *shown.get(self.viewer?)?;
        let picture = pattern.pictures.get(pic)?;
        let note = note.and_then(|n| pattern.notes.get(n));
        let name = note.filter(|n| !n.title.is_empty()).map_or_else(|| picture.name.clone(), |n| n.title.clone());
        let host = note.and_then(|n| n.host.clone());
        let said = [Some(format!("{}/{}", self.viewer? + 1, shown.len())), host].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        Some(Viewing { path: picture.path.clone(), name, said, many: shown.len() > 1, url: note.and_then(|n| n.url.clone()).filter(|u| safe_web(u)) })
    }

    /// ←/→ in the big picture.
    pub fn step(&mut self, forward: bool) {
        let n = self.pictures().len();
        if let Some(at) = self.viewer.filter(|_| n > 0) {
            self.viewer = Some(if forward { (at + 1) % n } else { (at + n - 1) % n });
        }
    }
}

/// The cards of a pattern: each picture with the note that names it (by file
/// name), else — when the notes and pictures are as many and neither has a
/// name to go by — in order; then the notes no picture was found for.
pub fn entries(pattern: &RefPattern) -> Vec<Entry> {
    let by_name = |note: &RefNote| note.picture.as_deref().and_then(|name| pattern.pictures.iter().position(|p| p.name.eq_ignore_ascii_case(name)));
    let mut note_of: Vec<Option<usize>> = vec![None; pattern.pictures.len()];
    let mut loose: Vec<usize> = Vec::new();
    for (n, note) in pattern.notes.iter().enumerate() {
        match by_name(note) {
            Some(p) if note_of[p].is_none() => note_of[p] = Some(n),
            _ => loose.push(n),
        }
    }
    // Notes that name no picture, beside pictures no note names: pair them in order.
    let unnamed: Vec<usize> = (0..pattern.pictures.len()).filter(|p| note_of[*p].is_none()).collect();
    let pairable: Vec<usize> = loose.iter().copied().filter(|n| pattern.notes[*n].picture.is_none()).collect();
    let mut out = Vec::new();
    let mut paired = Vec::new();
    if !pairable.is_empty() && pairable.len() == unnamed.len() {
        for (p, n) in unnamed.iter().zip(&pairable) {
            note_of[*p] = Some(*n);
            paired.push(*n);
        }
    }
    for (p, n) in note_of.iter().enumerate() {
        out.push(Entry { picture: Some(p), note: *n });
    }
    for n in loose.into_iter().filter(|n| !paired.contains(n)) {
        out.push(Entry { picture: None, note: Some(n) });
    }
    out
}

/// Whether a card holds the words searched (any case): its note's words or its picture's name.
pub fn entry_matches(pattern: &RefPattern, entry: &Entry, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let note = entry.note.and_then(|n| pattern.notes.get(n));
    let picture = entry.picture.and_then(|p| pattern.pictures.get(p));
    let hay = [
        picture.map(|p| p.name.as_str()),
        note.map(|n| n.title.as_str()),
        note.map(|n| n.take.as_str()),
        note.map(|n| n.leave.as_str()),
        note.map(|n| n.fit.as_str()),
        note.and_then(|n| n.host.as_deref()),
        note.and_then(|n| n.source.as_deref()),
    ];
    pattern.name.to_lowercase().contains(&q) || hay.into_iter().flatten().any(|s| s.to_lowercase().contains(&q))
}

/// Whether a pattern shows for the words typed so far, before the daemon's
/// answer for them comes: its name or notes hold them (any case).
pub fn pattern_matches(pattern: &RefPattern, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || pattern.name.to_lowercase().contains(&q) || pattern.raw.to_lowercase().contains(&q) || pattern.agree.to_lowercase().contains(&q)
}

/// A folder's name as a person says it: `kpi-tile` → `KPI tile`.
pub fn title(name: &str) -> String {
    const ACRONYMS: [&str; 10] = ["kpi", "ui", "ux", "cta", "faq", "api", "url", "id", "ai", "seo"];
    let words: Vec<String> = name
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .enumerate()
        .map(|(i, w)| {
            if ACRONYMS.contains(&w.to_lowercase().as_str()) {
                w.to_uppercase()
            } else if i == 0 {
                let mut chars = w.chars();
                chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
            } else {
                w.to_owned()
            }
        })
        .collect();
    if words.is_empty() { name.to_owned() } else { words.join(" ") }
}

/// Whether `url` is a web address to hand to a browser: `http(s)` and one
/// line, whatever a daemon sent.
pub fn safe_web(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://")) && !url.chars().any(|c| c.is_whitespace() || c.is_control()) && url.len() <= 2048
}

/// The picture to stand for a pattern in the list: its first.
pub fn cover(pattern: &RefPattern) -> Option<&RefPicture> {
    pattern.pictures.first()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pic(name: &str) -> RefPicture {
        RefPicture { path: format!("/d/references/p/{name}"), name: name.into(), size: 10, modified: 1 }
    }

    fn note(title: &str, picture: Option<&str>, take: &str) -> RefNote {
        RefNote { title: title.into(), picture: picture.map(str::to_owned), take: take.into(), host: Some("example.com".into()), url: Some("https://example.com/a".into()), ..Default::default() }
    }

    fn pattern(name: &str, pictures: &[&str], notes: Vec<RefNote>) -> RefPattern {
        RefPattern { name: name.into(), pictures: pictures.iter().map(|n| pic(n)).collect(), pictures_total: pictures.len() as u32, notes, ..Default::default() }
    }

    fn answer(ui: &mut RefsUi, project: Option<&str>, query: &str, offset: u32, total: u32, names: &[&str]) -> bool {
        ui.arrived(project, project.map(str::to_owned), query.to_owned(), offset, total, names.iter().map(|n| pattern(n, &[], vec![])).collect(), vec![], "/d/references".into())
    }

    #[test]
    fn a_look_is_asked_once_and_again_when_the_words_the_project_or_the_folder_change() {
        let mut ui = RefsUi::default();
        let ask = Command::DesignRefs { squad: "acme".into(), project: None, query: String::new(), offset: 0 };
        assert_eq!(ui.wanted("acme", None), Some(ask.clone()));
        assert_eq!(ui.wanted("acme", None), None, "asked, not asked again");
        assert!(ui.search("  table "));
        assert!(!ui.search("table"), "the same words");
        assert_eq!(ui.wanted("acme", None), Some(Command::DesignRefs { squad: "acme".into(), project: None, query: "table".into(), offset: 0 }));
        assert!(matches!(ui.wanted("acme", Some("web")), Some(Command::DesignRefs { project: Some(p), .. }) if p == "web"));
        ui.dirty = true;
        assert!(ui.wanted("acme", Some("web")).is_some(), "the folder moved");
        assert_eq!(ui.wanted("acme", Some("web")), None);
    }

    #[test]
    fn pages_are_added_in_order_and_an_old_answer_is_let_go() {
        let mut ui = RefsUi::default();
        ui.wanted("acme", None);
        assert!(answer(&mut ui, None, "", 0, 3, &["a", "b"]));
        assert_eq!(ui.answer.as_ref().unwrap().patterns.len(), 2);
        let more = ui.more("acme", None);
        assert_eq!(more, Some(Command::DesignRefs { squad: "acme".into(), project: None, query: String::new(), offset: 2 }));
        assert_eq!(ui.more("acme", None), None, "asked once");
        assert!(answer(&mut ui, None, "", 2, 3, &["c"]));
        assert_eq!(ui.answer.as_ref().unwrap().patterns.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(ui.more("acme", None), None, "all of them are here");
        // A page that doesn't continue what is here starts over; one for other words or another project is dropped.
        assert!(answer(&mut ui, None, "", 40, 90, &["z"]));
        assert_eq!(ui.answer.as_ref().unwrap().patterns.len(), 1);
        assert!(!ui.arrived(None, Some("web".into()), String::new(), 0, 1, vec![pattern("w", &[], vec![])], vec![], String::new()), "another project's");
        assert!(!answer(&mut ui, None, "old words", 0, 1, &["o"]), "words no longer typed");
        assert_eq!(ui.answer.as_ref().unwrap().patterns[0].name, "z");
    }

    #[test]
    fn an_open_pattern_that_is_gone_closes_and_one_still_to_come_stays() {
        let mut ui = RefsUi::default();
        answer(&mut ui, None, "", 0, 2, &["a", "b"]);
        ui.pattern = Some("b".into());
        ui.viewer = Some(0);
        assert!(answer(&mut ui, None, "", 0, 2, &["a", "b"]));
        assert_eq!(ui.pattern.as_deref(), Some("b"));
        assert!(answer(&mut ui, None, "", 0, 1, &["a"]));
        assert_eq!(ui.pattern, None, "gone from the folder");
        assert_eq!(ui.viewer, None);
        ui.pattern = Some("far".into());
        answer(&mut ui, None, "", 0, 90, &["a"]);
        assert_eq!(ui.pattern.as_deref(), Some("far"), "it may be on a later page");
    }

    #[test]
    fn cards_pair_each_picture_with_the_note_that_names_it_then_by_order() {
        let p = pattern("kpi", &["a.png", "b.png", "c.png"], vec![note("B", Some("B.PNG"), "x"), note("A", Some("a.png"), "y"), note("Gone", Some("none.png"), "z")]);
        let cards = entries(&p);
        assert_eq!(cards, [Entry { picture: Some(0), note: Some(1) }, Entry { picture: Some(1), note: Some(0) }, Entry { picture: Some(2), note: None }, Entry { picture: None, note: Some(2) }]);
        let unnamed = pattern("t", &["x.png", "y.png"], vec![note("One", None, ""), note("Two", None, "")]);
        assert_eq!(entries(&unnamed), [Entry { picture: Some(0), note: Some(0) }, Entry { picture: Some(1), note: Some(1) }], "as many as the pictures, in order");
        let uneven = pattern("t", &["x.png"], vec![note("One", None, ""), note("Two", None, "")]);
        assert_eq!(entries(&uneven), [Entry { picture: Some(0), note: None }, Entry { picture: None, note: Some(0) }, Entry { picture: None, note: Some(1) }]);
        assert_eq!(entries(&pattern("empty", &[], vec![])), []);
    }

    #[test]
    fn the_big_picture_steps_through_the_cards_that_have_one_and_shows_where_it_was_seen() {
        let mut ui = RefsUi::default();
        let mut p = pattern("table", &["a.png", "b.png", "c.png"], vec![note("A", Some("a.png"), "dense"), note("B", Some("b.png"), "airy")]);
        p.notes[1].url = Some("javascript:alert(1)".into());
        ui.answer = Some(Answer { patterns: vec![p], total: 1, ..Default::default() });
        ui.pattern = Some("table".into());
        ui.viewer = Some(0);
        let v = ui.viewing().unwrap();
        assert_eq!((v.name.as_str(), v.said.as_str(), v.many), ("A", "1/3 · example.com", true));
        assert_eq!(v.url.as_deref(), Some("https://example.com/a"));
        ui.step(true);
        assert_eq!(ui.viewing().unwrap().url, None, "only a web address is handed to a browser");
        ui.step(true);
        let v = ui.viewing().unwrap();
        assert_eq!((v.name.as_str(), v.said.as_str()), ("c.png", "3/3"), "a picture no note names is its file name");
        ui.step(true);
        assert_eq!(ui.viewer, Some(0), "round");
        ui.step(false);
        assert_eq!(ui.viewer, Some(2));
        // With words searched, the cards left are the ones stepped through.
        ui.query = "airy".into();
        ui.viewer = Some(0);
        assert_eq!((ui.pictures().len(), ui.viewing().unwrap().name.as_str(), ui.viewing().unwrap().many), (1, "B", false));
        ui.pattern = None;
        assert_eq!(ui.viewing(), None);
    }

    #[test]
    fn searching_reads_names_notes_hosts_and_picture_names_in_any_case() {
        let mut p = pattern("data-table", &["acme-1440.png"], vec![note("Acme — Orders", Some("acme-1440.png"), "36px rows")]);
        p.raw = "## Acme — Orders\n- Take: 36px rows\n".into();
        let card = entries(&p)[0];
        for q in ["", "  ", "ROWS", "acme", "example.com", "1440"] {
            assert!(entry_matches(&p, &card, q), "{q}");
        }
        assert!(!entry_matches(&p, &card, "sparkline"));
        assert!(pattern_matches(&p, "data") && pattern_matches(&p, "36PX") && !pattern_matches(&p, "toast"));
    }

    #[test]
    fn names_read_as_people_say_them_and_only_web_addresses_open() {
        assert_eq!(title("kpi-tile"), "KPI tile");
        assert_eq!(title("data_table"), "Data table");
        assert_eq!(title("filter-bar"), "Filter bar");
        assert_eq!(title("api-key-form"), "API key form");
        assert_eq!(title("--"), "--");
        assert_eq!(title("테이블"), "테이블");
        assert!(safe_web("https://example.com/a?b=1") && safe_web("http://example.com"));
        for bad in ["javascript:alert(1)", "file:///etc/passwd", "ftp://x", "https://a b", "https://a\nb", "example.com", ""] {
            assert!(!safe_web(bad), "{bad}");
        }
    }
}
