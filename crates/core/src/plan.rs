//! A plan: what the PM writes after the brief and before the tickets, kept in
//! the squad's own folder — never a project's repository — at
//! `docs/<project>/plans/<YYYY-MM-DD>-<slug>.md` (`crate::briefs::Target`). Its front
//! matter says what it is (`title`, `date`, `status`, `brief`, `scope`,
//! `tickets`); its sections follow in one order — goal, approach,
//! architecture and data flow, edge cases, test plan, design notes, risks,
//! then each reviewer's notes under its own name. The members it asks review
//! it at once, each writing its own notes into it; the person picks the
//! scope on one card; the tickets cut from it carry their slice of its test
//! plan. Pure: reading and writing the file is [`crate::briefs`]'s, the
//! board's the runtime's.

use crate::i18n::{self, Lang};
use equipe_protocol::TicketId;

/// Where a plan stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// Written, not yet put to the members.
    #[default]
    Draft,
    /// Its reviews are asked for.
    InReview,
    /// The scope is the person's pick, and tickets are cut from it.
    Settled,
}

impl Status {
    pub const ALL: [Status; 3] = [Status::Draft, Status::InReview, Status::Settled];

    pub fn label(self) -> &'static str {
        match self {
            Status::Draft => "draft",
            Status::InReview => "in_review",
            Status::Settled => "settled",
        }
    }

    pub fn parse(text: &str) -> Option<Status> {
        let text = text.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        Status::ALL.into_iter().find(|s| s.label() == text)
    }
}

/// How much the plan takes on: the person's pick, among four.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The bigger, better version: more than was asked.
    Expansion,
    /// What was asked, and the few additions worth it.
    Selective,
    /// What was asked, no more.
    Hold,
    /// Less than was asked: the core first.
    Reduction,
}

impl Scope {
    pub const ALL: [Scope; 4] = [Scope::Expansion, Scope::Selective, Scope::Hold, Scope::Reduction];

    pub fn label(self) -> &'static str {
        match self {
            Scope::Expansion => "expansion",
            Scope::Selective => "selective",
            Scope::Hold => "hold",
            Scope::Reduction => "reduction",
        }
    }

    /// Its label, or a way of saying it: `selective expansion`, `hold scope`, `reduce`.
    pub fn parse(text: &str) -> Option<Scope> {
        let text = text.trim().to_ascii_lowercase().replace(['_', '-'], " ");
        match text.as_str() {
            "expansion" | "expand" | "scope expansion" => Some(Scope::Expansion),
            "selective" | "selective expansion" => Some(Scope::Selective),
            "hold" | "hold scope" => Some(Scope::Hold),
            "reduction" | "reduce" | "scope reduction" => Some(Scope::Reduction),
            _ => None,
        }
    }
}

/// A plan's front matter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Front {
    pub title: String,
    /// `YYYY-MM-DD`, the day it was first written.
    pub date: String,
    pub status: Status,
    /// The brief it came from: its path, when it went through one.
    pub brief: Option<String>,
    /// The scope the person picked; none until they have.
    pub scope: Option<Scope>,
    /// The tickets cut from it, once made.
    pub tickets: Vec<TicketId>,
}

impl Front {
    /// Its fields in the file's order, as `key: value` — `brief`, `scope` and
    /// `tickets` only once there is one.
    pub fn fields(&self) -> Vec<(&'static str, String)> {
        let mut out = vec![("title", self.title.clone()), ("date", self.date.clone()), ("status", self.status.label().to_owned())];
        if let Some(brief) = self.brief.as_ref().filter(|b| !b.trim().is_empty()) {
            out.push(("brief", brief.clone()));
        }
        if let Some(scope) = self.scope {
            out.push(("scope", scope.label().to_owned()));
        }
        if !self.tickets.is_empty() {
            out.push(("tickets", ids_text(&self.tickets)));
        }
        out
    }

    /// Read back from its fields; what isn't there, or isn't understood, is left at its default.
    pub fn from_fields(fields: &[(String, String)]) -> Front {
        let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.trim()).filter(|v| !v.is_empty());
        Front {
            title: get("title").unwrap_or_default().to_owned(),
            date: get("date").unwrap_or_default().to_owned(),
            status: get("status").and_then(Status::parse).unwrap_or_default(),
            brief: get("brief").map(str::to_owned),
            scope: get("scope").and_then(Scope::parse),
            tickets: get("tickets").map(ids_from).unwrap_or_default(),
        }
    }

    /// A ticket cut from it: listed once, and the plan settled.
    pub fn add_ticket(&mut self, id: TicketId) {
        if !self.tickets.contains(&id) {
            self.tickets.push(id);
        }
        self.status = Status::Settled;
    }
}

/// `[3, 4]`.
fn ids_text(ids: &[TicketId]) -> String {
    format!("[{}]", ids.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "))
}

/// `[3, 4]`, `3, 4`, `[T-3, T-4]` or a YAML list's `- 3` lines folded in → 3, 4.
fn ids_from(text: &str) -> Vec<TicketId> {
    text.trim_matches(|c| c == '[' || c == ']')
        .split([',', ' ', '\n'])
        .filter_map(|part| crate::board::parse_ticket_id(part.trim().trim_start_matches('-').trim()))
        .collect()
}

/// A plan's sections, in the order the file keeps them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Goal,
    Approach,
    /// Architecture and data flow.
    Architecture,
    EdgeCases,
    TestPlan,
    DesignNotes,
    Risks,
    /// Each reviewer's notes, under its name.
    Reviews,
}

impl Section {
    pub const ALL: [Section; 8] = [
        Section::Goal,
        Section::Approach,
        Section::Architecture,
        Section::EdgeCases,
        Section::TestPlan,
        Section::DesignNotes,
        Section::Risks,
        Section::Reviews,
    ];

    /// The sections the PM writes: all but the reviews.
    pub const WRITTEN: [Section; 7] =
        [Section::Goal, Section::Approach, Section::Architecture, Section::EdgeCases, Section::TestPlan, Section::DesignNotes, Section::Risks];

    /// Its name as `write_plan` takes it, and in its words' keys.
    pub fn key(self) -> &'static str {
        match self {
            Section::Goal => "goal",
            Section::Approach => "approach",
            Section::Architecture => "architecture",
            Section::EdgeCases => "edge_cases",
            Section::TestPlan => "test_plan",
            Section::DesignNotes => "design_notes",
            Section::Risks => "risks",
            Section::Reviews => "reviews",
        }
    }

    fn index(self) -> usize {
        Section::ALL.iter().position(|s| *s == self).unwrap_or_default()
    }

    /// Its heading in `lang`.
    pub fn heading(self, lang: Lang) -> String {
        i18n::core(lang).get(&format!("plan.section.{}", self.key())).to_owned()
    }

    /// The section a heading names, in any language equipe speaks — a plan
    /// written while the screen spoke another is still read.
    pub fn from_heading(text: &str) -> Option<Section> {
        let text = text.trim();
        Section::ALL.into_iter().find(|s| {
            let key = format!("plan.section.{}", s.key());
            Lang::ALL.iter().any(|l| i18n::core(*l).own(&key).is_some_and(|h| h.eq_ignore_ascii_case(text))) || s.key() == text
        })
    }
}

/// A plan's text below its front matter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Body {
    /// What comes before its first section: its `# title`.
    pub preface: String,
    /// Each written section's text, in [`Section::WRITTEN`]'s order.
    pub sections: [String; 7],
    /// What the review notes begin with, before any reviewer's.
    pub reviews_intro: String,
    /// Each reviewer's notes: who (`backend`, `qa-2 (qa)`), what they found.
    pub reviews: Vec<(String, String)>,
}

impl Body {
    /// A new plan's: its title, every section to fill.
    pub fn new(title: &str) -> Body {
        Body { preface: format!("# {}", title.trim()), ..Body::default() }
    }

    pub fn section(&self, section: Section) -> &str {
        self.sections.get(section.index()).map_or("", String::as_str)
    }

    /// A written section's text replaced; the test plan's cases numbered.
    pub fn set(&mut self, section: Section, text: &str) {
        let text = if section == Section::TestPlan { number_cases(text) } else { text.trim().to_owned() };
        if let Some(slot) = self.sections.get_mut(section.index()) {
            *slot = text;
        }
    }

    /// `who`'s review notes: in place of its last ones, else after the others.
    pub fn set_review(&mut self, who: &str, notes: &str) {
        let notes = notes.trim().to_owned();
        match self.reviews.iter_mut().find(|(w, _)| w == who) {
            Some((_, said)) => *said = notes,
            None => self.reviews.push((who.to_owned(), notes)),
        }
    }

    /// Read from the text: each heading of a known section starts it; any
    /// other heading stays inside the section it is in.
    pub fn parse(text: &str) -> Body {
        let mut body = Body::default();
        // Where the lines go: the preface, a section, the reviews' start, a review.
        enum At {
            Preface,
            Section(usize),
            Reviews,
            Review(usize),
        }
        let mut at = At::Preface;
        let push = |body: &mut Body, at: &At, line: &str| {
            let slot = match at {
                At::Preface => &mut body.preface,
                At::Section(i) => &mut body.sections[*i],
                At::Reviews => &mut body.reviews_intro,
                At::Review(i) => &mut body.reviews[*i].1,
            };
            slot.push_str(line);
            slot.push('\n');
        };
        for line in text.lines() {
            if let Some(section) = line.strip_prefix("## ").and_then(Section::from_heading) {
                at = match section {
                    Section::Reviews => At::Reviews,
                    other => At::Section(other.index()),
                };
                continue;
            }
            if matches!(at, At::Reviews | At::Review(_))
                && let Some(who) = line.strip_prefix("### ")
            {
                body.reviews.push((who.trim().to_owned(), String::new()));
                at = At::Review(body.reviews.len() - 1);
                continue;
            }
            push(&mut body, &at, line);
        }
        let tidy = |s: &mut String| *s = s.trim().to_owned();
        tidy(&mut body.preface);
        body.sections.iter_mut().for_each(tidy);
        tidy(&mut body.reviews_intro);
        body.reviews.iter_mut().for_each(|(_, notes)| tidy(notes));
        body
    }

    /// As the file has it, its headings in `lang`.
    pub fn render(&self, lang: Lang) -> String {
        let mut out = String::new();
        if !self.preface.is_empty() {
            out.push_str(&self.preface);
            out.push_str("\n\n");
        }
        for section in Section::ALL {
            out.push_str(&format!("## {}\n\n", section.heading(lang)));
            let text = match section {
                Section::Reviews => &self.reviews_intro,
                other => self.section(other),
            };
            if !text.is_empty() {
                out.push_str(text);
                out.push_str("\n\n");
            }
        }
        for (who, notes) in &self.reviews {
            out.push_str(&format!("### {who}\n\n"));
            if !notes.is_empty() {
                out.push_str(notes);
                out.push_str("\n\n");
            }
        }
        format!("{}\n", out.trim_end())
    }
}

/// A plan's file read: its front matter and its body. A file without front
/// matter is all body.
pub fn parse(text: &str) -> (Front, Body) {
    match crate::briefs::split_front(text) {
        Some((fields, body)) => (Front::from_fields(&fields), Body::parse(body)),
        None => (Front::default(), Body::parse(text)),
    }
}

/// A plan's file: its front matter, then its body with headings in `lang`.
pub fn text(front: &Front, body: &Body, lang: Lang) -> String {
    crate::briefs::render(&front.fields(), &body.render(lang))
}

/// What one `write_plan` call changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub title: Option<String>,
    pub brief: Option<String>,
    pub status: Option<Status>,
    pub scope: Option<Scope>,
    /// Sections given, each replacing what was there.
    pub sections: Vec<(Section, String)>,
    /// A reviewer's notes: who, what.
    pub review: Option<(String, String)>,
}

impl Change {
    /// Read from `write_plan`'s arguments. A member gives only its review;
    /// the rest is the PM's.
    pub fn read(args: &serde_json::Value, is_pm: bool) -> Result<Change, String> {
        let text = |key: &str| args.get(key).and_then(serde_json::Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        let mut change = Change { title: text("title"), brief: text("brief"), ..Change::default() };
        if let Some(status) = text("status") {
            change.status = Some(Status::parse(&status).ok_or_else(|| format!("A plan's status is draft, in_review or settled, not {status}."))?);
        }
        if let Some(scope) = text("scope") {
            change.scope = Some(Scope::parse(&scope).ok_or_else(|| format!("A plan's scope is expansion, selective, hold or reduction, not {scope}."))?);
        }
        change.sections = Section::WRITTEN.into_iter().filter_map(|s| text(s.key()).map(|t| (s, t))).collect();
        let review = text("review");
        if is_pm {
            if review.is_some() {
                return Err("The plan is yours: put what you take from a review into its sections; `review` is a reviewer's own notes.".into());
            }
        } else {
            let theirs = change.title.is_some() || change.brief.is_some() || change.status.is_some() || change.scope.is_some() || !change.sections.is_empty();
            if theirs {
                return Err("The plan's sections are the PM's: put what you found in `review` — your own notes — and the PM takes it in.".into());
            }
            if review.is_none() {
                return Err("Give your review in `review`.".into());
            }
        }
        change.review = review.map(|r| (String::new(), r));
        Ok(change)
    }

    /// Nothing to write.
    pub fn is_empty(&self) -> bool {
        *self == Change::default()
    }
}

/// The plan's file after `change`: `existing` changed, or a new one — which
/// needs its title — dated `today`.
pub fn apply(existing: Option<&str>, change: &Change, today: &str, lang: Lang) -> Result<String, String> {
    let (mut front, mut body) = match existing {
        Some(text) => parse(text),
        None => {
            let title = change.title.as_deref().ok_or("A new plan needs a `title`.")?;
            (Front { title: title.to_owned(), date: today.to_owned(), ..Front::default() }, Body::new(title))
        }
    };
    if front.date.is_empty() {
        front.date = today.to_owned();
    }
    if let Some(title) = &change.title {
        front.title = title.clone();
    }
    if let Some(brief) = &change.brief {
        front.brief = Some(brief.clone());
    }
    if let Some(status) = change.status {
        front.status = status;
    }
    if let Some(scope) = change.scope {
        front.scope = Some(scope);
    }
    for (section, text) in &change.sections {
        body.set(*section, text);
    }
    if let Some((who, notes)) = &change.review {
        body.set_review(who, notes);
    }
    Ok(text(&front, &body, lang))
}

/// Who a review is by, as its heading says: the member, and its role when
/// the name doesn't say it.
pub fn reviewer(name: &str, role: &str) -> String {
    if name == role || role.is_empty() { name.to_owned() } else { format!("{name} ({role})") }
}

/// One case of a test plan: `TP-3`, and what it checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    pub id: String,
    pub text: String,
}

/// The list marker a line opens with, and what follows it: `- `, `* `, `3. `.
fn item(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        return Some(rest);
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0).then(|| line[digits..].strip_prefix(". ").or_else(|| line[digits..].strip_prefix(") "))).flatten()
}

/// The case id an item opens with — `TP-3`, `**TP-3**`, `[TP-3]` — its
/// number, and what follows it.
fn case_id(rest: &str) -> Option<(u32, &str)> {
    let rest = rest.trim_start();
    let (open, close) = if rest.starts_with("**") {
        ("**", "**")
    } else if rest.starts_with('[') {
        ("[", "]")
    } else {
        ("", "")
    };
    let inner = rest[open.len()..].strip_prefix("TP-").or_else(|| rest[open.len()..].strip_prefix("tp-"))?;
    let digits = inner.bytes().take_while(u8::is_ascii_digit).count();
    let n = inner[..digits].parse().ok()?;
    let after = inner[digits..].strip_prefix(close)?;
    if after.starts_with(|c: char| c.is_alphanumeric()) {
        return None;
    }
    Some((n, after.trim_start_matches([':', '.', ' ']).trim_start_matches(['—', '-']).trim_start()))
}

/// A test plan with every case numbered: each top-level list item without a
/// `TP-` number gets the next one, as `- TP-4: …`; numbered ones keep theirs.
pub fn number_cases(test_plan: &str) -> String {
    let mut next = cases(test_plan).iter().filter_map(|c| c.id.strip_prefix("TP-")?.parse::<u32>().ok()).max().unwrap_or(0);
    let lines: Vec<String> = test_plan
        .trim()
        .lines()
        .map(|line| match item(line) {
            Some(rest) if case_id(rest).is_none() && !rest.trim().is_empty() => {
                next += 1;
                format!("- TP-{next}: {}", rest.trim())
            }
            _ => line.to_owned(),
        })
        .collect();
    lines.join("\n")
}

/// A test plan's cases: each top-level item with its `TP-` number, with the
/// indented lines under it.
pub fn cases(test_plan: &str) -> Vec<Case> {
    let mut out: Vec<Case> = Vec::new();
    let mut open = false;
    for line in test_plan.lines() {
        if let Some((n, rest)) = item(line).and_then(case_id) {
            out.push(Case { id: format!("TP-{n}"), text: rest.trim().to_owned() });
            open = true;
        } else if open && line.starts_with([' ', '\t']) && !line.trim().is_empty() {
            if let Some(case) = out.last_mut() {
                case.text.push('\n');
                case.text.push_str(line.trim_end());
            }
        } else if !line.trim().is_empty() {
            open = false;
        }
    }
    out
}

/// A case's id as it may be named: `TP-3`, `tp-3`, `3` → `TP-3`.
fn case_name(text: &str) -> String {
    let t = text.trim();
    let digits = t.strip_prefix("TP-").or_else(|| t.strip_prefix("tp-")).unwrap_or(t);
    format!("TP-{digits}")
}

/// The cases a ticket carries, in the order named, or the first id the test plan doesn't have.
pub fn slice(test_plan: &str, ids: &[String]) -> Result<Vec<Case>, String> {
    let all = cases(test_plan);
    let mut out: Vec<Case> = Vec::new();
    for id in ids {
        let name = case_name(id);
        let case = all.iter().find(|c| c.id == name).ok_or(name)?;
        if !out.contains(case) {
            out.push(case.clone());
        }
    }
    Ok(out)
}

/// What a ticket cut from a plan says at the end of its body, in the
/// person's language: the plan's path, and its slice of the test plan.
pub fn ticket_block(path: &str, cases: &[Case], lang: Lang) -> String {
    let words = i18n::core(lang);
    let mut out = words.fill("plan.ticket_from", &[("path", path.to_owned())]);
    if !cases.is_empty() {
        out.push_str(&format!("\n\n{}\n", words.get("plan.ticket_tests")));
        for case in cases {
            let text = case.text.replace('\n', "\n  ");
            out.push_str(&format!("\n- {}: {text}", case.id));
        }
    }
    out
}

/// The angle a member reviews a plan from, by its role: the engineers' what
/// breaks, the designer's the screens, QA's whether it can be tested.
pub fn angle(role: &str) -> &'static str {
    match role {
        "frontend" | "backend" | "mobile" => "engineer",
        "designer" => "designer",
        "qa" => "qa",
        _ => "general",
    }
}

/// The review ticket a reviewer is given: its title and body, in the person's language.
pub fn review_ticket(title: &str, path: &str, role: &str, lang: Lang) -> (String, String) {
    let words = i18n::core(lang);
    let head = words.fill("plan.review_title", &[("title", title.trim().to_owned())]);
    let body = words.fill(&format!("plan.review_body.{}", angle(role)), &[("path", path.to_owned())]);
    (head, body)
}

/// The slug a `plan` argument names: a path (`docs/plans/2026-11-04-sign-in.md`),
/// a file name, or the slug itself.
pub fn slug_of(arg: &str) -> Option<String> {
    let name = arg.trim().rsplit('/').next()?.trim();
    let stem = name.strip_suffix(".md").unwrap_or(name);
    // `YYYY-MM-DD-` before it.
    let dated = stem.len() > 11 && stem.as_bytes()[..10].iter().enumerate().all(|(i, b)| if i == 4 || i == 7 { *b == b'-' } else { b.is_ascii_digit() }) && stem.as_bytes()[10] == b'-';
    let slug = if dated { &stem[11..] } else { stem };
    (!slug.is_empty()).then(|| slug.to_owned())
}

/// What a member on a ticket cut from a plan is told: where the plan is and
/// how to read it, and on a review, where its notes go. A plan is kept in the
/// squad's folder, named by its absolute path; one from before, committed in
/// a repository (`repo`), is read on `main` there.
pub fn work_note(path: &str, repo: Option<(&std::path::Path, &str)>) -> String {
    let read = match repo {
        Some((repo, main)) => format!(
            "`{path}` in {} — on {main}; if your checkout doesn't have it, `git -C {} show {main}:{path}`",
            repo.display(),
            repo.display()
        ),
        None => format!("`{path}` — in the squad's folder, not in your worktree: read it there, by that absolute path"),
    };
    format!(
        "\nThis ticket comes from the plan {read}. Read it before you start: its goal, approach, edge cases and test \
plan bound the work. When the ticket asks you to review the plan, write your notes into it with `write_plan` (`plan`, \
`review`) — only your own notes, never its other sections — then hand in with a two-line summary.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_says_what_is_there_in_the_contracts_order_and_reads_back() {
        let front = Front { title: "Sign in".into(), date: "2026-11-04".into(), status: Status::InReview, brief: Some("docs/briefs/2026-11-03-sign-in.md".into()), scope: Some(Scope::Selective), tickets: vec![3, 4] };
        let fields = front.fields();
        let keys: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, ["title", "date", "status", "brief", "scope", "tickets"]);
        assert_eq!(fields[2].1, "in_review");
        assert_eq!(fields[5].1, "[3, 4]");
        let owned: Vec<(String, String)> = fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        assert_eq!(Front::from_fields(&owned), front);
        // A new one: no brief, no scope, no tickets yet — left out, not written empty.
        let fresh = Front { title: "x".into(), date: "2026-11-04".into(), ..Front::default() };
        assert_eq!(fresh.fields().iter().map(|(k, _)| *k).collect::<Vec<_>>(), ["title", "date", "status"]);
        assert_eq!(fresh.fields()[2].1, "draft");
    }

    #[test]
    fn a_plans_file_is_its_front_matter_then_its_sections_and_reads_back() {
        let change = Change {
            title: Some("Magic-link sign in".into()),
            brief: Some("docs/briefs/2026-11-03-sign-in.md".into()),
            sections: vec![(Section::Goal, "Sign in without a password.".into()), (Section::TestPlan, "- expired link".into())],
            ..Change::default()
        };
        let text = apply(None, &change, "2026-11-04", Lang::En).unwrap();
        assert!(text.starts_with("---\ntitle: Magic-link sign in\ndate: 2026-11-04\nstatus: draft\nbrief: docs/briefs/2026-11-03-sign-in.md\n---\n\n# Magic-link sign in\n\n## Goal\n\nSign in without a password.\n"), "{text}");
        assert!(text.contains("## Test plan\n\n- TP-1: expired link\n"), "{text}");
        let (front, body) = parse(&text);
        assert_eq!((front.title.as_str(), front.date.as_str(), front.status), ("Magic-link sign in", "2026-11-04", Status::Draft));
        assert_eq!(body.section(Section::Goal), "Sign in without a password.");
        // Reviewed, the scope picked, a ticket cut: the date and what was there stay.
        let reviewed = apply(Some(&text), &Change { review: Some(("backend".into(), "Index the token.".into())), ..Change::default() }, "2026-11-09", Lang::En).unwrap();
        let picked = apply(Some(&reviewed), &Change { scope: Some(Scope::Hold), status: Some(Status::InReview), ..Change::default() }, "2026-11-09", Lang::En).unwrap();
        let (front, body) = parse(&picked);
        assert_eq!((front.date.as_str(), front.scope, front.status), ("2026-11-04", Some(Scope::Hold), Status::InReview));
        assert_eq!((body.section(Section::Goal), body.reviews.len()), ("Sign in without a password.", 1));
        assert!(picked.contains("scope: hold\n---"), "{picked}");
        // A plan needs its title; a file without front matter is all body.
        assert!(apply(None, &Change::default(), "2026-11-04", Lang::En).unwrap_err().contains("`title`"));
        assert_eq!(parse("## Goal\n\nx").1.section(Section::Goal), "x");
    }

    #[test]
    fn the_pm_writes_the_plan_and_a_reviewer_only_its_own_notes() {
        use serde_json::json;
        let pm = Change::read(&json!({ "title": "x", "goal": "g", "scope": "hold", "status": "in_review" }), true).unwrap();
        assert_eq!((pm.scope, pm.status, pm.sections.len()), (Some(Scope::Hold), Some(Status::InReview), 1));
        assert!(Change::read(&json!({ "scope": "everything" }), true).unwrap_err().contains("expansion, selective, hold or reduction"));
        assert!(Change::read(&json!({ "review": "mine" }), true).unwrap_err().contains("`review` is a reviewer's"));
        let member = Change::read(&json!({ "plan": "p", "review": "Index the token." }), false).unwrap();
        assert_eq!(member.review, Some((String::new(), "Index the token.".into())));
        assert!(Change::read(&json!({ "review": "x", "goal": "mine now" }), false).unwrap_err().contains("the PM's"));
        assert!(Change::read(&json!({ "plan": "p" }), false).unwrap_err().contains("`review`"));
        assert!(Change::read(&json!({ "plan": "p" }), true).unwrap().is_empty());
    }

    #[test]
    fn front_matter_written_by_hand_is_read_as_it_was_meant() {
        let fields = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect::<Vec<_>>();
        let f = Front::from_fields(&fields(&[("status", "In Review"), ("scope", "selective expansion"), ("tickets", "[T-3, 4]")]));
        assert_eq!((f.status, f.scope, f.tickets), (Status::InReview, Some(Scope::Selective), vec![3, 4]));
        let f = Front::from_fields(&fields(&[("status", "someday"), ("scope", "everything"), ("tickets", "[]")]));
        assert_eq!((f.status, f.scope, f.tickets), (Status::Draft, None, vec![]), "not understood: the default");
        assert_eq!(Scope::parse("hold scope"), Some(Scope::Hold));
        assert_eq!(Scope::parse("reduce"), Some(Scope::Reduction));
        assert!(Scope::ALL.iter().all(|s| Scope::parse(s.label()) == Some(*s)));
    }

    #[test]
    fn a_ticket_cut_from_a_plan_is_listed_once_and_settles_it() {
        let mut f = Front::default();
        f.add_ticket(7);
        f.add_ticket(7);
        f.add_ticket(8);
        assert_eq!((f.tickets.as_slice(), f.status), (&[7, 8][..], Status::Settled));
    }

    #[test]
    fn a_plan_keeps_its_sections_in_order_and_each_reviewer_under_its_name() {
        let mut body = Body::new("Sign in");
        body.set(Section::Risks, "Rate limits.");
        body.set(Section::Goal, "People sign in with a magic link.");
        body.set(Section::TestPlan, "- An expired link says so\n- A used link can't sign in twice");
        body.set_review("backend", "Token table needs an index.");
        body.set_review("qa", "TP-2 needs two browsers.");
        let text = body.render(Lang::En);
        let at = |s: &str| text.find(s).unwrap_or_else(|| panic!("{s}: {text}"));
        let order = ["# Sign in", "## Goal", "## Approach", "## Architecture and data flow", "## Edge cases", "## Test plan", "## Design notes", "## Risks", "## Review notes", "### backend", "### qa"];
        assert!(order.windows(2).all(|w| at(w[0]) < at(w[1])), "{text}");
        assert!(text.contains("- TP-1: An expired link says so\n- TP-2: A used link"), "numbered: {text}");
        // Read back as written; a review rewritten in place, not added twice.
        let mut back = Body::parse(&text);
        assert_eq!(back, body);
        back.set_review("backend", "Index added to the approach.");
        assert_eq!(back.reviews, [("backend".to_owned(), "Index added to the approach.".to_owned()), ("qa".to_owned(), "TP-2 needs two browsers.".to_owned())]);
    }

    #[test]
    fn a_plan_written_in_another_language_is_read_and_a_heading_of_its_own_stays_in_its_section() {
        let mut body = Body::new("로그인");
        body.set(Section::Approach, "매직 링크.\n\n## 대안\n\n비밀번호.");
        body.set_review("designer", "UI 맵에 오류 상태가 없음.");
        let korean = body.render(Lang::Ko);
        assert!(korean.contains("## 목표") && korean.contains("## 리뷰 노트"), "{korean}");
        let back = Body::parse(&korean);
        assert_eq!(back.section(Section::Approach), "매직 링크.\n\n## 대안\n\n비밀번호.", "an unknown heading is the section's own");
        assert_eq!(back.reviews.len(), 1);
        // Rewritten while the screen speaks English: the same plan, English headings.
        assert_eq!(Body::parse(&back.render(Lang::En)), back);
        for lang in Lang::ALL {
            for section in Section::ALL {
                assert_eq!(Section::from_heading(&section.heading(lang)), Some(section), "{lang:?} {section:?}");
            }
        }
    }

    #[test]
    fn test_plan_cases_are_numbered_once_and_kept_with_their_lines() {
        let plan = "Cases:\n- TP-2: kept\n- new one\n  with a second line\n* another\n\nNot a case.\n1. a numbered list item";
        let numbered = number_cases(plan);
        assert_eq!(numbered, "Cases:\n- TP-2: kept\n- TP-3: new one\n  with a second line\n- TP-4: another\n\nNot a case.\n- TP-5: a numbered list item");
        assert_eq!(number_cases(&numbered), numbered, "numbered once");
        let all = cases(&numbered);
        assert_eq!(all.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["TP-2", "TP-3", "TP-4", "TP-5"]);
        assert_eq!(all[1].text, "new one\n  with a second line");
        for written in ["- **TP-9** bold", "- [TP-9] bracketed", "- TP-9 — dashed", "3. TP-9. dotted"] {
            assert_eq!(cases(written).first().map(|c| c.id.as_str()), Some("TP-9"), "{written}");
        }
        assert!(cases("- TP-9x not an id").is_empty() && cases("- TPS report").is_empty());
    }

    #[test]
    fn a_ticket_carries_its_slice_of_the_test_plan_and_a_missing_case_is_named() {
        let plan = "- TP-1: expired link\n- TP-2: used twice\n  two browsers\n- TP-3: wrong email";
        let slice = super::slice(plan, &["3".into(), "tp-2".into(), "TP-3".into()]).unwrap();
        assert_eq!(slice.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["TP-3", "TP-2"], "as named, once each");
        assert_eq!(super::slice(plan, &["TP-7".into()]), Err("TP-7".into()));
        let block = ticket_block("docs/plans/2026-11-04-sign-in.md", &slice, Lang::En);
        assert!(block.starts_with("Plan: `docs/plans/2026-11-04-sign-in.md`"), "{block}");
        assert!(block.contains("Test plan for this ticket:\n\n- TP-3: wrong email\n- TP-2: used twice\n    two browsers"), "{block}");
        assert!(!ticket_block("p.md", &[], Lang::En).contains("Test plan"), "no cases, no heading");
        assert!(ticket_block("p.md", &slice, Lang::Ko).contains("테스트 계획"));
    }

    #[test]
    fn a_review_ticket_asks_each_role_from_its_own_angle() {
        let path = "docs/plans/2026-11-04-sign-in.md";
        let (title, engineer) = review_ticket("Sign in", path, "backend", Lang::En);
        assert_eq!(title, "Review the plan: Sign in");
        assert!(engineer.contains(path) && engineer.contains("what breaks") && engineer.contains("simpler"), "{engineer}");
        let (_, designer) = review_ticket("Sign in", path, "designer", Lang::En);
        assert!(designer.contains("UI map") && designer.contains("0 to 10"), "{designer}");
        let (_, qa) = review_ticket("Sign in", path, "qa", Lang::En);
        assert!(qa.contains("tested"), "{qa}");
        assert_eq!(angle("mobile"), "engineer");
        assert_eq!(angle("writer"), "general");
        assert_eq!(reviewer("qa-2", "qa"), "qa-2 (qa)");
        assert_eq!(reviewer("backend", "backend"), "backend");
    }

    #[test]
    fn a_plan_is_named_by_its_path_its_file_or_its_slug() {
        for arg in ["docs/plans/2026-11-04-sign-in.md", "2026-11-04-sign-in.md", "sign-in", "2026-11-04-sign-in"] {
            assert_eq!(slug_of(arg).as_deref(), Some("sign-in"), "{arg}");
        }
        assert_eq!(slug_of("2026-11-04").as_deref(), Some("2026-11-04"), "a slug that only looks like a date");
        assert_eq!(slug_of(" "), None);
    }

    #[test]
    fn a_member_on_a_plans_ticket_is_told_where_it_is_and_writes_only_its_own_notes() {
        // Kept in the squad's folder: its absolute path, outside the worktree.
        let path = "/h/squads/acme/home/docs/site/plans/2026-11-04-sign-in.md";
        let note = work_note(path, None);
        assert!(note.contains(&format!("`{path}`")), "{note}");
        assert!(note.contains("in the squad's folder, not in your worktree") && note.contains("by that absolute path"), "{note}");
        assert!(!note.contains("git -C"), "{note}");
        assert!(note.contains("`write_plan` (`plan`, \n`review`)") || note.contains("`write_plan` (`plan`, `review`)"), "{note}");
        assert!(note.contains("only your own notes"), "{note}");
        // One from before plans left the repositories: where it was committed.
        let old = work_note("docs/plans/2026-11-04-sign-in.md", Some((std::path::Path::new("/p/site"), "main")));
        assert!(old.contains("`git -C /p/site show main:docs/plans/2026-11-04-sign-in.md`"), "{old}");
    }
}
