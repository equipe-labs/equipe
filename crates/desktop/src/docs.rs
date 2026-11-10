//! The squad's briefs and plans all together (`views/docs.rs`), the rules:
//! every project's and the squad's own, newest first, narrowed by whose they
//! are, their kind and how they stand, and found by title at once and by
//! their words when the daemon answers (`Command::SearchDocs`); the tickets a
//! doc names and the docs a ticket came from; the ones saved under more than
//! one project, and what is left of them once the person keeps one.

use equipe_core::protocol::briefs::{DocCopies, DocKind, InRepo, ProjectDoc};
use equipe_core::protocol::{Command, TicketId};

use crate::board::{DocTone, doc_status};

/// Whose docs are shown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Whose {
    #[default]
    All,
    /// One project's, by name.
    Project(String),
    /// The squad's own work, outside the projects.
    Squad,
}

impl Whose {
    fn takes(&self, doc: &ProjectDoc) -> bool {
        match self {
            Whose::All => true,
            Whose::Project(name) => doc.project.as_deref() == Some(name.as_str()),
            Whose::Squad => doc.project.is_none(),
        }
    }
}

/// What the list is narrowed to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filter {
    pub whose: Whose,
    pub kind: Option<DocKind>,
    pub status: Option<DocTone>,
}

impl Filter {
    pub fn takes(&self, doc: &ProjectDoc) -> bool {
        self.whose.takes(doc) && self.kind.is_none_or(|k| k == doc.kind) && self.status.is_none_or(|s| doc_status(&doc.status).1 == s)
    }

    /// Whether it narrows anything.
    pub fn narrows(&self) -> bool {
        *self != Filter::default()
    }
}

/// Newest first; a brief before the plan of the same day.
fn newest_first(a: &&ProjectDoc, b: &&ProjectDoc) -> std::cmp::Ordering {
    b.date.cmp(&a.date).then(a.kind.cmp(&b.kind)).then_with(|| a.title.cmp(&b.title)).then_with(|| a.path.cmp(&b.path))
}

/// The docs shown: those `filter` takes and — with words to find — whose
/// title holds them, or which the daemon found by them (`found`, its answer
/// for these words). Newest first.
pub fn listed<'a>(docs: &'a [ProjectDoc], filter: &Filter, query: &str, found: Option<&[String]>) -> Vec<&'a ProjectDoc> {
    let query = query.trim().to_lowercase();
    let mut out: Vec<&ProjectDoc> = docs
        .iter()
        .filter(|d| filter.takes(d))
        .filter(|d| query.is_empty() || d.title.to_lowercase().contains(&query) || found.is_some_and(|f| f.contains(&d.path)))
        .collect();
    out.sort_by(newest_first);
    out
}

/// Whose docs there are to pick from, in the daemon's order: each project's
/// that has one, then the squad's own when it has any.
pub fn wheres(docs: &[ProjectDoc]) -> Vec<Whose> {
    let mut out: Vec<Whose> = Vec::new();
    for doc in docs {
        let whose = doc.project.clone().map_or(Whose::Squad, Whose::Project);
        if !out.contains(&whose) {
            out.push(whose);
        }
    }
    // The squad's own last, wherever it came.
    if let Some(at) = out.iter().position(|w| *w == Whose::Squad) {
        let own = out.remove(at);
        out.push(own);
    }
    out
}

/// The doc at `path`, when it is one of them.
pub fn doc_at<'a>(docs: &'a [ProjectDoc], path: &str) -> Option<&'a ProjectDoc> {
    docs.iter().find(|d| d.path == path)
}

/// The briefs and plans ticket `id` came from or was cut from: a brief's
/// `ticket`, a plan's `tickets` — briefs first, newest first.
pub fn docs_of_ticket(docs: &[ProjectDoc], id: TicketId) -> Vec<&ProjectDoc> {
    let mut out: Vec<&ProjectDoc> = docs.iter().filter(|d| d.ticket_ids().contains(&id)).collect();
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| newest_first(a, b)));
    out
}

/// The set of copies `path` is one of.
pub fn copies_of<'a>(copies: &'a [DocCopies], path: &str) -> Option<&'a DocCopies> {
    copies.iter().find(|c| c.paths.iter().any(|p| p == path))
}

/// The older copies still in `project`'s repository.
pub fn in_repo_of<'a>(in_repos: &'a [InRepo], project: &str) -> Vec<&'a InRepo> {
    in_repos.iter().filter(|r| r.project == project).collect()
}

/// One copy kept, as the daemon told it: shown with its undo until the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub kept: String,
    pub removed: Vec<String>,
    /// The change that removed them, for `Command::UndoHomeChange`.
    pub undo: Option<String>,
    /// Taken back already: the banner says so, its button gone.
    pub undone: bool,
}

/// What the screen holds of the squad's briefs and plans beside the list
/// itself (`Shell::docs`).
#[derive(Debug, Clone, Default)]
pub struct Shelf {
    /// The ones saved under more than one project.
    pub copies: Vec<DocCopies>,
    /// Older copies still in the projects' repositories.
    pub in_repos: Vec<InRepo>,
    pub filter: Filter,
    /// The words last asked of the daemon.
    pub asked: Option<String>,
    /// Its answer: the words, and the paths it found by them.
    pub found: Option<(String, Vec<String>)>,
    /// The copy the person asked to keep, until the daemon answers.
    pub keeping: Option<String>,
    pub kept: Option<Kept>,
}

impl Shelf {
    /// The daemon's answer for `query`, when it is for these words.
    pub fn found_for(&self, query: &str) -> Option<&[String]> {
        self.found.as_ref().filter(|(q, _)| q.trim() == query.trim()).map(|(_, paths)| paths.as_slice())
    }

    /// The search to ask the daemon for `query`, typed: none when it is
    /// empty or already asked.
    pub fn ask(&mut self, squad: &str, query: &str) -> Option<Command> {
        let query = query.trim();
        if query.is_empty() || self.asked.as_deref() == Some(query) {
            return None;
        }
        self.asked = Some(query.to_owned());
        Some(Command::SearchDocs { squad: squad.to_owned(), query: query.to_owned() })
    }

    /// Keep the copy at `path`, as told in `session`'s conversation: once,
    /// until the daemon answers.
    pub fn keep(&mut self, squad: &str, session: u64, path: &str) -> Option<Command> {
        if self.keeping.is_some() || copies_of(&self.copies, path).is_none() {
            return None;
        }
        self.keeping = Some(path.to_owned());
        self.kept = None;
        Some(Command::KeepDocCopy { squad: squad.to_owned(), session, keep: path.to_owned() })
    }

    /// The daemon kept one.
    pub fn on_kept(&mut self, kept: String, removed: Vec<String>, undo: Option<String>) {
        self.keeping = None;
        self.kept = Some(Kept { kept, removed, undo, undone: false });
    }

    /// Take back what keeping one removed: the command, once.
    pub fn undo(&mut self, squad: &str, session: u64) -> Option<Command> {
        let kept = self.kept.as_mut().filter(|k| !k.undone)?;
        let change = kept.undo.clone()?;
        kept.undone = true;
        Some(Command::UndoHomeChange { squad: squad.to_owned(), session, change })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(kind: DocKind, project: Option<&str>, date: &str, title: &str, status: &str) -> ProjectDoc {
        let dir = project.map_or(String::new(), |p| format!("{p}/"));
        ProjectDoc {
            kind,
            project: project.map(str::to_owned),
            path: format!("/h/docs/{dir}{}/{date}-{}.md", kind.folder(), title.to_lowercase().replace(' ', "-")),
            title: title.into(),
            date: date.into(),
            status: status.into(),
            ticket: None,
            mode: None,
            tickets: Vec::new(),
        }
    }

    fn docs() -> Vec<ProjectDoc> {
        let mut checkout = doc(DocKind::Brief, Some("web"), "2026-11-02", "Checkout", "settled");
        checkout.ticket = Some(42);
        let mut plan = doc(DocKind::Plan, Some("web"), "2026-11-03", "Checkout plan", "in_review");
        plan.tickets = vec![42, 51];
        vec![checkout, plan, doc(DocKind::Brief, Some("api"), "2026-11-04", "Rate limits", "draft"), doc(DocKind::Plan, None, "2026-11-05", "Offsite", "settled"), doc(DocKind::Brief, Some("web"), "2026-10-20", "Gift cards", "dropped")]
    }

    fn titles(docs: &[&ProjectDoc]) -> Vec<String> {
        docs.iter().map(|d| d.title.clone()).collect()
    }

    #[test]
    fn every_project_and_the_squads_own_are_listed_together_newest_first() {
        let docs = docs();
        assert_eq!(titles(&listed(&docs, &Filter::default(), "", None)), ["Offsite", "Rate limits", "Checkout plan", "Checkout", "Gift cards"]);
        assert_eq!(wheres(&docs), [Whose::Project("web".into()), Whose::Project("api".into()), Whose::Squad]);
    }

    #[test]
    fn the_list_narrows_by_whose_kind_and_status_together() {
        let docs = docs();
        let only = |filter: Filter| titles(&listed(&docs, &filter, "", None));
        assert_eq!(only(Filter { whose: Whose::Project("web".into()), ..Filter::default() }), ["Checkout plan", "Checkout", "Gift cards"]);
        assert_eq!(only(Filter { whose: Whose::Squad, ..Filter::default() }), ["Offsite"], "the squad's own: shown nowhere else");
        assert_eq!(only(Filter { kind: Some(DocKind::Plan), ..Filter::default() }), ["Offsite", "Checkout plan"]);
        assert_eq!(only(Filter { status: Some(DocTone::Settled), ..Filter::default() }), ["Offsite", "Checkout"]);
        assert_eq!(only(Filter { whose: Whose::Project("web".into()), kind: Some(DocKind::Brief), status: Some(DocTone::Dropped) }), ["Gift cards"]);
        assert!(only(Filter { whose: Whose::Project("shop".into()), ..Filter::default() }).is_empty());
        assert!(!Filter::default().narrows() && Filter { kind: Some(DocKind::Plan), ..Filter::default() }.narrows());
    }

    #[test]
    fn words_find_titles_at_once_and_the_rest_when_the_daemon_answers() {
        let docs = docs();
        let mut shelf = Shelf::default();
        assert_eq!(titles(&listed(&docs, &Filter::default(), "CHECKOUT", None)), ["Checkout plan", "Checkout"], "by title, before any answer");
        assert_eq!(shelf.ask("acme", " burst "), Some(Command::SearchDocs { squad: "acme".into(), query: "burst".into() }));
        assert_eq!(shelf.ask("acme", "burst"), None, "asked once");
        assert_eq!(shelf.ask("acme", "  "), None);
        assert!(listed(&docs, &Filter::default(), "burst", shelf.found_for("burst")).is_empty(), "nothing yet");
        shelf.found = Some(("burst".into(), vec![docs[2].path.clone()]));
        assert_eq!(titles(&listed(&docs, &Filter::default(), "burst", shelf.found_for("burst"))), ["Rate limits"], "found by its words");
        assert!(shelf.found_for("bursts").is_none(), "an answer for other words is no answer");
        let web = Filter { whose: Whose::Project("web".into()), ..Filter::default() };
        assert!(listed(&docs, &web, "burst", shelf.found_for("burst")).is_empty(), "and the filters still hold");
    }

    #[test]
    fn a_ticket_shows_the_docs_it_came_from_and_a_doc_its_tickets() {
        let docs = docs();
        assert_eq!(titles(&docs_of_ticket(&docs, 42)), ["Checkout", "Checkout plan"], "the brief, then the plan");
        assert_eq!(titles(&docs_of_ticket(&docs, 51)), ["Checkout plan"]);
        assert!(docs_of_ticket(&docs, 7).is_empty());
        let plan = doc_at(&docs, &docs[1].path).unwrap();
        assert_eq!(plan.ticket_ids(), [42, 51]);
        assert!(doc_at(&docs, "/elsewhere.md").is_none());
    }

    #[test]
    fn keeping_one_copy_is_asked_once_and_its_undo_sent_once() {
        let copies = vec![DocCopies { kind: DocKind::Plan, title: "Your block".into(), paths: vec!["/h/docs/sf/plans/a.md".into(), "/h/docs/block/plans/a.md".into()], differ: true }];
        let mut shelf = Shelf { copies, ..Shelf::default() };
        assert_eq!(copies_of(&shelf.copies, "/h/docs/block/plans/a.md").map(|c| c.paths.len()), Some(2));
        assert_eq!(shelf.keep("acme", 1, "/h/docs/web/plans/b.md"), None, "only one of a set");
        assert_eq!(shelf.keep("acme", 1, "/h/docs/sf/plans/a.md"), Some(Command::KeepDocCopy { squad: "acme".into(), session: 1, keep: "/h/docs/sf/plans/a.md".into() }));
        assert_eq!(shelf.keep("acme", 1, "/h/docs/block/plans/a.md"), None, "one at a time, until the daemon answers");
        assert_eq!(shelf.undo("acme", 1), None, "nothing kept yet");
        shelf.on_kept("/h/docs/sf/plans/a.md".into(), vec!["/h/docs/block/plans/a.md".into()], Some("c0ffee".into()));
        assert!(shelf.keeping.is_none());
        assert_eq!(shelf.undo("acme", 1), Some(Command::UndoHomeChange { squad: "acme".into(), session: 1, change: "c0ffee".into() }));
        assert_eq!(shelf.undo("acme", 1), None, "taken back once");
        assert!(shelf.kept.as_ref().unwrap().undone);
    }

    #[test]
    fn the_older_copies_in_a_repository_are_each_projects_own() {
        let in_repos = vec![
            InRepo { project: "web".into(), kind: DocKind::Plan, path: "docs/plans/2026-11-03-checkout.md".into(), kept: Some("/h/docs/web/plans/2026-11-03-checkout.md".into()) },
            InRepo { project: "api".into(), kind: DocKind::Brief, path: "docs/briefs/2026-11-04-rate-limits.md".into(), kept: None },
        ];
        assert_eq!(in_repo_of(&in_repos, "web").len(), 1);
        assert_eq!(in_repo_of(&in_repos, "api")[0].path, "docs/briefs/2026-11-04-rate-limits.md");
        assert!(in_repo_of(&in_repos, "app").is_empty());
    }
}
