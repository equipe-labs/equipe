//! Briefs and plans: what the squad settles with the person before it builds.
//!
//! After office hours the PM leaves a brief, and the plan that follows it sits
//! beside it — both in the squad's own folder, never in a project's
//! repository: any repository may be public, and a brief holds the person's
//! plans, the people they name and their numbers. A project's are
//! `docs/<project>/briefs/<YYYY-MM-DD>-<slug>.md` and `docs/<project>/plans/…`
//! there ([`Target`]), work outside the projects `docs/briefs/…` and
//! `docs/plans/…`. Each opens with a front matter of flat `key: value` lines
//! between `---` (a brief's: `title`, `date`, `status` — draft, settled or
//! dropped — `ticket`, `mode` — startup or builder), then the document.
//!
//! One is written in place and saved at once as a change of the squad's
//! folder (`crate::squad_home`), by whoever wrote it — kept, told apart and
//! taken back as every change there is. Members on a ticket read it there by
//! its absolute path: their worktree doesn't have it. Reads stay inside the
//! folder (no links out) and are size-limited, as `design/folder.rs` reads.
//! What equipe once committed into the repositories is copied over, once
//! ([`keep_from_repos`]), and left where it was.
//!
//! A second opinion on a brief comes from the other engine
//! (`equipe brief critique`), the way `design::critique` gets one on pictures.

use std::path::{Path, PathBuf};

pub use equipe_protocol::briefs::{DocKind as Kind, ProjectDoc};
use equipe_protocol::{Engine, TicketId};

use serde_json::{Value, json};

use crate::design::folder::inside;
use crate::worktree;

/// The biggest brief or plan read or written.
pub const MAX: u64 = 256 << 10;
/// How many of each kind a folder lists.
const LISTED: usize = 200;

// ── the front matter ──

/// The front matter's fields, in order, and the document after it — `None`
/// when it doesn't open with one. Flat `key: value` lines; quotes around a
/// value are taken off; a list (`[3, 4]`, or `- 3` lines under its key) is
/// kept as `[3, 4]`.
pub fn split_front(text: &str) -> Option<(Vec<(String, String)>, &str)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n"))?;
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut items: Option<Vec<String>> = None;
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        at += line.len();
        let bare = line.trim_end_matches(['\n', '\r']);
        if bare.trim_end() == "---" {
            close_list(&mut fields, &mut items);
            return Some((fields, &rest[at..]));
        }
        let trimmed = bare.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if bare.starts_with([' ', '\t']) {
            if let (Some(item), Some(list)) = (trimmed.strip_prefix("- ").or(trimmed.strip_prefix('-').filter(|r| r.is_empty())), items.as_mut()) {
                list.push(unquote(item.trim()).to_owned());
            }
            continue;
        }
        close_list(&mut fields, &mut items);
        let Some((key, value)) = bare.split_once(':') else { continue };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            continue;
        }
        let value = value.trim();
        fields.push((key.to_owned(), unquote(value).to_owned()));
        if value.is_empty() {
            items = Some(Vec::new());
        }
    }
    None
}

/// A key left with no value and `- item` lines under it: a list.
fn close_list(fields: &mut [(String, String)], items: &mut Option<Vec<String>>) {
    if let (Some(list), Some(last)) = (items.take(), fields.last_mut())
        && !list.is_empty()
        && last.1.is_empty()
    {
        last.1 = format!("[{}]", list.join(", "));
    }
}

fn unquote(value: &str) -> &str {
    for q in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(q).and_then(|v| v.strip_suffix(q)).filter(|_| value.len() >= 2) {
            return inner;
        }
    }
    value
}

/// A field's value, by its key.
pub fn field<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.trim()).filter(|v| !v.is_empty())
}

/// A front matter of `fields`, then `body`. A value that YAML would read as
/// something else is quoted; a list (`[3, 4]`) is written as it is.
pub fn render(fields: &[(&str, String)], body: &str) -> String {
    let mut out = String::from("---\n");
    for (key, value) in fields {
        let value = value.trim();
        let plain = value.starts_with('[') && value.ends_with(']')
            || !value.is_empty()
                && !value.contains(": ")
                && !value.contains(" #")
                && !value.contains(['"', '\n'])
                && !value.starts_with(['\'', '{', '&', '*', '!', '|', '>', '%', '@', '`', '-', '?', '#', ',', '[', ']']);
        if plain {
            out.push_str(&format!("{key}: {value}\n"));
        } else {
            out.push_str(&format!("{key}: {}\n", serde_json::to_string(value.replace('\n', " ").as_str()).unwrap_or_default()));
        }
    }
    out.push_str("---\n\n");
    out.push_str(body.trim_start_matches('\n'));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

// ── names ──

/// A slug from words: ASCII letters and digits, the rest dashes, at most 60
/// long. Empty when nothing ASCII is in them — then a slug must be given.
pub fn slugify(words: &str) -> String {
    let mut out = String::new();
    for c in words.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let mut out = out.trim_end_matches('-').to_owned();
    if out.len() > 60 {
        out.truncate(60);
        out = out.trim_end_matches('-').to_owned();
    }
    out
}

/// Whether `slug` is one: what [`slugify`] makes.
pub fn is_slug(slug: &str) -> bool {
    !slug.is_empty() && slugify(slug) == slug
}

/// `<date>-<slug>.md`.
pub fn file_name(date: &str, slug: &str) -> String {
    format!("{date}-{slug}.md")
}

/// The date and the slug a file's name holds: `2026-11-04-checkout.md`.
pub fn parse_name(name: &str) -> Option<(&str, &str)> {
    let stem = name.strip_suffix(".md")?;
    let (date, slug) = (stem.get(..10)?, stem.get(11..)?);
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let parts: Vec<&str> = date.split('-').collect();
    (stem.as_bytes().get(10) == Some(&b'-') && parts.len() == 3 && parts.iter().all(|p| digits(p)) && !slug.is_empty()).then_some((date, slug))
}

/// A ticket named in a front matter: `12`, `T-12`, `#12`.
pub fn ticket_of(value: &str) -> Option<TicketId> {
    let v = value.trim();
    let v = v.strip_prefix("T-").or_else(|| v.strip_prefix("t-")).or_else(|| v.strip_prefix('#')).unwrap_or(v);
    v.parse().ok()
}

/// What a brief or plan is, from its file's name and text.
pub fn doc_of(kind: Kind, project: Option<&str>, path: &Path, text: &str) -> ProjectDoc {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (named_date, slug) = parse_name(&name).map_or(("", name.trim_end_matches(".md")), |(d, s)| (d, s));
    let (fields, body) = split_front(text).unwrap_or((Vec::new(), text));
    let heading = body.lines().find_map(|l| l.strip_prefix("# ")).map(str::trim).filter(|h| !h.is_empty());
    let title = field(&fields, "title").or(heading).unwrap_or(slug).to_owned();
    ProjectDoc {
        kind,
        project: project.map(str::to_owned),
        path: path.to_string_lossy().into_owned(),
        title,
        date: field(&fields, "date").unwrap_or(named_date).to_owned(),
        status: field(&fields, "status").unwrap_or("draft").to_ascii_lowercase(),
        ticket: field(&fields, "ticket").and_then(ticket_of),
        mode: field(&fields, "mode").map(str::to_ascii_lowercase),
    }
}

// ── where they are kept ──

/// Their folder, from the top of the squad's.
pub const DOCS: &str = "docs";

/// Where a squad's briefs and plans are kept: in the squad's own folder (its
/// home, a git repository equipe keeps for it), never in a project's
/// repository — any project may be public, and a brief holds the person's
/// plans, people and numbers. A project's are under `docs/<project>/`, the
/// squad's own work's under `docs/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The squad's folder.
    home: PathBuf,
    /// The project they are about, by name; none: the squad's own.
    project: Option<String>,
}

/// Whether a project's name can be a folder of its own under `docs/`.
fn is_folder_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', '\0']) && name.len() <= 200
}

impl Target {
    /// The squad's own, for work outside the projects: `docs/briefs`, `docs/plans`.
    pub fn squad(home: &Path) -> Target {
        Target { home: home.to_owned(), project: None }
    }

    /// `project`'s (its name): `docs/<project>/briefs`, `docs/<project>/plans`.
    pub fn project(home: &Path, project: &str) -> Result<Target, String> {
        if !is_folder_name(project) {
            return Err(format!("A project called {project:?} can't have a folder of its own in the squad's folder."));
        }
        Ok(Target { home: home.to_owned(), project: Some(project.to_owned()) })
    }

    /// Whose a brief or plan named by its absolute path is, and which kind:
    /// `<home>/docs/<project>/plans/<file>`, or `<home>/docs/plans/<file>`.
    pub fn of_path(home: &Path, path: &Path) -> Option<(Target, Kind)> {
        let rel = path.strip_prefix(home.join(DOCS)).ok()?;
        let parts: Vec<String> = rel
            .components()
            .map(|c| match c {
                std::path::Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect::<Option<_>>()?;
        let kind_of = |folder: &str| Kind::ALL.into_iter().find(|k| k.folder() == folder);
        match parts.as_slice() {
            [folder, _file] => Some((Target::squad(home), kind_of(folder)?)),
            [project, folder, _file] => Some((Target::project(home, project).ok()?, kind_of(folder)?)),
            _ => None,
        }
    }

    /// The squad's folder they are in.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The project they are about, by name.
    pub fn project_name(&self) -> Option<&str> {
        self.project.as_deref()
    }

    /// `kind`'s folder, from the top of the squad's.
    pub fn rel(&self, kind: Kind) -> String {
        match &self.project {
            None => kind.dir().to_owned(),
            Some(project) => format!("{DOCS}/{project}/{}", kind.folder()),
        }
    }

    /// The folder their kinds' folders are in: `<home>/docs[/<project>]`.
    pub fn root(&self) -> PathBuf {
        match &self.project {
            None => self.home.join(DOCS),
            Some(project) => self.home.join(DOCS).join(project),
        }
    }

    /// Where `kind`'s file called `name` is.
    pub fn path(&self, kind: Kind, name: &str) -> PathBuf {
        self.home.join(self.rel(kind)).join(name)
    }

    /// The names of the files in `kind`'s folder.
    fn names(&self, kind: Kind) -> Vec<String> {
        names_in(&self.home.join(self.rel(kind)))
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir).map(|e| e.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default()
}

// ── reading ──

/// The briefs and plans `target` keeps, each kind newest first: Markdown
/// files right in each kind's folder that stay inside the squad's folder,
/// read no further than [`MAX`].
pub fn list(target: &Target) -> Vec<ProjectDoc> {
    let mut out = Vec::new();
    for kind in Kind::ALL {
        let mut names: Vec<String> = target.names(kind).into_iter().filter(|n| n.ends_with(".md") && !n.starts_with('.')).collect();
        names.sort_by(|a, b| b.cmp(a));
        let dir = target.rel(kind);
        let mut docs: Vec<ProjectDoc> = names
            .iter()
            .take(LISTED)
            .filter_map(|name| {
                let rel = format!("{dir}/{name}");
                let bytes = inside(target.home(), &rel, MAX)?;
                let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX as usize)]);
                Some(doc_of(kind, target.project_name(), &target.home().join(&rel), &text))
            })
            .collect();
        docs.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.path.cmp(&a.path)));
        out.extend(docs);
    }
    out
}

/// Every brief and plan of `squad`, all in its own folder: each project's,
/// then its own work's. Nothing is read from a project's repository.
pub fn catalog(home: &Path, squad: &str) -> Vec<ProjectDoc> {
    let mut out = Vec::new();
    let folder = crate::squad_home::path(home, squad);
    let db = crate::journal::squad_dir(home, squad).join("squad.db");
    if crate::squads::valid_name(squad)
        && db.exists()
        && let Ok(journal) = crate::journal::Journal::open(&db)
    {
        for project in journal.projects().unwrap_or_default() {
            if let Ok(target) = Target::project(&folder, &crate::board::project_name(&project)) {
                out.extend(list(&target));
            }
        }
    }
    out.extend(list(&Target::squad(&folder)));
    out
}

/// The name of `kind`'s file with `slug`, when there is one — the earliest
/// date first, should two share it.
pub fn find(target: &Target, kind: Kind, slug: &str) -> Option<String> {
    let mut names: Vec<String> = target.names(kind).into_iter().filter(|n| parse_name(n).is_some_and(|(_, s)| s == slug)).collect();
    names.sort();
    names.into_iter().next()
}

/// What `kind`'s file with `slug` says.
pub fn read(target: &Target, kind: Kind, slug: &str) -> Option<String> {
    let name = find(target, kind, slug)?;
    let bytes = inside(target.home(), &format!("{}/{name}", target.rel(kind)), MAX).filter(|b| b.len() as u64 <= MAX)?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

// ── writing ──

/// Write `text` as `kind`'s file with `slug` — the one there already (its
/// date kept), else `<today>-<slug>.md` — in the squad's folder, in place and
/// never through a link, and save it there as a change by `by` that the
/// person can take back, as every change to the folder is. Nothing goes near
/// a project's repository. Returns where it is.
pub fn save(target: &Target, kind: Kind, slug: &str, text: &str, today: &str, by: &str) -> Result<PathBuf, String> {
    if !is_slug(slug) {
        return Err(format!("`{slug}` isn't a slug: a few lowercase ASCII words joined by dashes (`checkout-redesign`)."));
    }
    if text.len() as u64 > MAX {
        return Err(format!("It is {} KB; a {} is at most {} KB.", text.len() >> 10, kind.word(), MAX >> 10));
    }
    let name = find(target, kind, slug).unwrap_or_else(|| file_name(today, slug));
    let dir = target.rel(kind);
    let rel = format!("{dir}/{name}");
    // Whatever the person changed in it themselves is saved as theirs first.
    let _ = crate::squad_home::save_in(target.home(), crate::squad_home::PERSON, "changed in the squad's folder", &[&rel]);
    let at = real_folder(target.home(), &dir)?;
    let file = at.join(&name);
    if std::fs::symlink_metadata(&file).is_ok_and(|m| !m.is_file()) {
        return Err(format!("{} isn't a file of its own.", file.display()));
    }
    let partial = at.join(format!(".{name}.partial"));
    std::fs::write(&partial, text).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, &file).map_err(|e| format!("{}: {e}", file.display()))?;
    let title = split_front(text).and_then(|(f, _)| field(&f, "title").map(str::to_owned)).unwrap_or_else(|| slug.to_owned());
    // A change of its own; should git be busy, the next save takes it in.
    let _ = crate::squad_home::save_in(target.home(), by, &format!("wrote the {} {title}", kind.word()), &[&rel]);
    Ok(file)
}

/// `rel`'s folder under `root`, made where it isn't: each folder on the way a
/// real one inside `root`, never a link elsewhere.
fn real_folder(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let mut at = root.to_path_buf();
    for part in rel.split('/') {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(format!("{} isn't a folder of its own.", at.display())),
            Err(_) => std::fs::create_dir(&at).map_err(|e| format!("{}: {e}", at.display()))?,
        }
    }
    Ok(at)
}

// ── what equipe once wrote into the repositories ──

/// Whether `text`, in a file called `name`, is a brief or plan equipe wrote:
/// named `<date>-<slug>.md`, its front matter equipe's — a title, a date, a
/// status of `kind`'s, and a brief's mode.
pub fn written_by_equipe(kind: Kind, name: &str, text: &str) -> bool {
    let Some((fields, _)) = split_front(text) else { return false };
    let status = field(&fields, "status").unwrap_or_default();
    let mode = field(&fields, "mode").unwrap_or_default().to_ascii_lowercase();
    parse_name(name).is_some()
        && field(&fields, "title").is_some()
        && field(&fields, "date").is_some_and(|d| parse_name(&format!("{d}-x.md")).is_some())
        && match kind {
            Kind::Brief => ["draft", "settled", "dropped"].contains(&status) && ["startup", "builder"].contains(&mode.as_str()),
            Kind::Plan => ["draft", "in_review", "settled"].contains(&status),
        }
}

/// A brief or plan equipe once committed into a project's repository, and
/// where the squad's folder keeps a copy of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftInRepo {
    pub kind: Kind,
    /// Where it is in the repository: `docs/plans/2026-11-04-sign-in.md`.
    pub from: String,
    /// Where its copy goes.
    pub to: PathBuf,
    bytes: Vec<u8>,
}

/// The briefs and plans equipe wrote into `repo` (on its main branch, else
/// in its folder) that `target` doesn't have yet. Only read: the repository
/// is left as it is.
pub fn left_in_repo(repo: &Path, target: &Target) -> Vec<LeftInRepo> {
    let main = worktree::main_branch(repo).ok();
    let mut out = Vec::new();
    for kind in Kind::ALL {
        let names = match &main {
            Some(_) => worktree::names_on_main(repo, kind.dir()).unwrap_or_default(),
            None => names_in(&repo.join(kind.dir())),
        };
        let have = target.names(kind);
        for name in names.into_iter().filter(|n| n.ends_with(".md") && !have.contains(n)) {
            let from = format!("{}/{name}", kind.dir());
            let bytes = match &main {
                Some(main) => worktree::read_blob(repo, main, Path::new(&from), MAX).ok(),
                None => inside(repo, &from, MAX),
            };
            let Some(bytes) = bytes.filter(|b| b.len() as u64 <= MAX) else { continue };
            if written_by_equipe(kind, &name, &String::from_utf8_lossy(&bytes)) {
                out.push(LeftInRepo { kind, from, to: target.path(kind, &name), bytes });
            }
        }
    }
    out
}

/// Copy what equipe once wrote into the squad's `projects` into the squad's
/// folder `home`, so the PM's history stays where briefs and plans are kept
/// now — saved there as one change, the repositories left untouched. Returns
/// what was copied.
pub fn keep_from_repos(home: &Path, projects: &[PathBuf]) -> Result<Vec<LeftInRepo>, String> {
    let mut copied = Vec::new();
    for repo in projects {
        let Ok(target) = Target::project(home, &crate::board::project_name(repo)) else { continue };
        for doc in left_in_repo(repo, &target) {
            let at = real_folder(home, &target.rel(doc.kind))?;
            let file = at.join(doc.to.file_name().unwrap_or_default());
            if std::fs::symlink_metadata(&file).is_ok() {
                continue;
            }
            std::fs::write(&file, &doc.bytes).map_err(|e| format!("{}: {e}", file.display()))?;
            copied.push(doc);
        }
    }
    if !copied.is_empty() {
        crate::squad_home::save_in(home, "equipe", "kept the briefs and plans from the projects' repositories in the squad's folder", &[DOCS])?;
    }
    Ok(copied)
}

// ── the PM's tool ──

/// The `write_brief` tool's name.
pub const WRITE_BRIEF: &str = "write_brief";

/// What a brief's front matter says, from `write_brief`'s arguments: the
/// title, the status, the mode and the ticket, checked; the date is the file's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefArgs {
    pub project: Option<String>,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub mode: String,
    pub ticket: Option<TicketId>,
    pub body: String,
}

/// `write_brief`'s arguments, checked.
pub fn brief_args(args: &Value) -> Result<BriefArgs, String> {
    let title = crate::board::arg_str(args, "title").map(str::trim).filter(|t| !t.is_empty()).ok_or("A brief needs a `title`, in the person's language.")?.to_owned();
    let body = crate::board::arg_str(args, "body").map(str::trim).filter(|b| !b.is_empty()).ok_or("Write the brief in `body`: Markdown, in the person's language, without a front matter — equipe writes that.")?.to_owned();
    let status = crate::board::arg_str(args, "status").unwrap_or("draft").to_owned();
    if !["draft", "settled", "dropped"].contains(&status.as_str()) {
        return Err(format!("A brief's status is draft, settled or dropped, not {status}."));
    }
    let mode = crate::board::arg_str(args, "mode").ok_or("Say the `mode`: startup (a product people should want) or builder (something the person wants made).")?.to_owned();
    if !["startup", "builder"].contains(&mode.as_str()) {
        return Err(format!("A brief's mode is startup or builder, not {mode}."));
    }
    let slug = match crate::board::arg_str(args, "slug").map(str::trim).filter(|s| !s.is_empty()) {
        Some(slug) if is_slug(slug) => slug.to_owned(),
        Some(slug) => return Err(format!("`{slug}` isn't a slug: lowercase ASCII words joined by dashes, like `checkout-redesign`.")),
        None => Some(slugify(&title)).filter(|s| !s.is_empty()).ok_or("Give a `slug` — a few lowercase English words joined by dashes (`checkout-redesign`): the title has none to make one from.")?,
    };
    let ticket = match args.get("ticket") {
        Some(Value::Number(n)) => n.as_u64().and_then(|n| TicketId::try_from(n).ok()),
        Some(Value::String(s)) => ticket_of(s),
        _ => None,
    };
    Ok(BriefArgs { project: crate::board::arg_str(args, "project").map(str::to_owned), slug, title, status, mode, ticket, body })
}

/// The brief's text: its front matter, then the body (a front matter the
/// PM wrote itself taken off — equipe's is the one).
pub fn brief_text(brief: &BriefArgs, date: &str) -> String {
    let mut fields = vec![("title", brief.title.clone()), ("date", date.to_owned()), ("status", brief.status.clone())];
    if let Some(ticket) = brief.ticket {
        fields.push(("ticket", ticket.to_string()));
    }
    fields.push(("mode", brief.mode.clone()));
    let body = split_front(&brief.body).map_or(brief.body.as_str(), |(_, rest)| rest);
    render(&fields, body)
}

/// The tool, for the PM.
pub fn tool() -> Value {
    json!({
        "name": WRITE_BRIEF,
        "description": "Write the brief office hours lead to — or rewrite it: the same slug is the same brief. equipe writes \
it with its front matter (title, date, status, ticket, mode) into the squad's own folder — \
`docs/<project>/briefs/<date>-<slug>.md`, or `docs/briefs/…` when it belongs to no project — never into a project's \
repository, which may be public; saves it there as a change the person can take back; and shows it on the project's \
page. Write it in the person's language. status: draft until the person says it is settled; settled only on their \
word; dropped when they let it go.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": "The project it is about; left out: work outside the projects." },
                "title": { "type": "string", "description": "In the person's language." },
                "slug": { "type": "string", "description": "A few lowercase English words joined by dashes (`checkout-redesign`); left out, made from the title." },
                "mode": { "type": "string", "enum": ["startup", "builder"] },
                "status": { "type": "string", "enum": ["draft", "settled", "dropped"] },
                "ticket": { "type": "integer", "description": "The idea or ticket it came from, if any." },
                "body": { "type": "string", "description": "The brief in Markdown, without a front matter." },
            },
            "required": ["title", "mode", "body"],
        },
    })
}

/// What the PM is told when the person starts an idea: it waits, held,
/// until the PM releases it — at once when it is small, after office hours
/// and a settled brief when it isn't.
pub fn idea_start_message(id: TicketId, title: &str) -> String {
    format!(
        "[Idea start · T-{id}] The person started their idea T-{id} ({title}). It is on the board as todo but held: equipe hands \
it to nobody until you release it with `update_ticket` (status todo). If it is small and clear — a fix, a tweak, a task you \
can state in a line — release it now and say so in one line. If it is of real size, run office hours with them first, \
one card at a time; write the brief with `write_brief` (ticket {id}); and release it, or replace it with the tickets the \
settled brief leads to, only once they have settled it."
    )
}

// ── a second opinion ──

/// What the other engine is asked of a brief: to find where it is weak.
const RUBRIC: &str = "You are a sceptical senior product lead reading another lead's brief, written after office hours \
with the person who wants this built. Find where it is weak; do not praise, and do not soften. You did not write it.\n\
\n\
Check, in order: the problem — is the demand real and evidenced (what people do today, what they pay or put up \
with), or assumed? Who exactly is it for — a person you could name, or a category? Is the wedge the narrowest \
thing that could work? Words used without being defined; assumptions nobody named; what happens if nothing is \
built. Existing code or tools that already solve part of it — read the repository when you can. How users will \
actually get to it. Are the alternatives genuinely different, and does the recommendation follow from the \
evidence? Is done measurable?\n\
\n\
Answer in this shape, in the language of the brief:\n\
## Verdict\n\
`settle` or `revise`, in one line, and the one change that would help most.\n\
## Gaps\n\
One line each, worst first: `[blocker|gap] <section> — what is missing or wrong — the question that would settle it`.\n\
## Blind spots\n\
What the brief doesn't see at all, two or three lines.";

/// The prompt for the brief's second opinion.
pub fn critique_prompt(brief: &str, path: &Path) -> String {
    format!("{RUBRIC}\n\nThe brief ({}):\n<brief>\n{}\n</brief>", path.display(), brief.trim())
}

/// The program and its arguments that ask `engine` once, reading only: Codex
/// in its read-only sandbox, its last message written to `answer`; Claude
/// with its reading tools alone, its answer printed.
pub fn critique_command(engine: Engine, brief: &str, path: &Path, answer: &Path) -> (String, Vec<String>) {
    let text = critique_prompt(brief, path);
    match engine {
        Engine::Codex => {
            let mut args: Vec<String> = ["exec", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral", "--color", "never"].map(String::from).into();
            args.extend(["--output-last-message".into(), answer.display().to_string(), text]);
            (crate::engines::codex_binary(), args)
        }
        Engine::Claude => {
            let args = ["-p", &text, "--allowedTools", "Read,Grep,Glob", "--disallowedTools", "Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch"].map(String::from).into();
            (equipe_providers::claude_code::DEFAULT_BINARY.to_owned(), args)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-briefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn repo(name: &str) -> PathBuf {
        let dir = scratch(name);
        git(&dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("README.md"), "hi\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "start"]);
        dir
    }

    const BRIEF: &str = "---\ntitle: \"Checkout: one tap\"\ndate: 2026-11-04\nstatus: settled\nticket: T-12\nmode: Startup\n---\n\n# Checkout\n\nWhy.\n";

    #[test]
    fn a_front_matter_reads_as_its_fields_then_the_document() {
        let (fields, body) = split_front(BRIEF).unwrap();
        assert_eq!(field(&fields, "title"), Some("Checkout: one tap"), "quotes off, the colon kept");
        assert_eq!(field(&fields, "date"), Some("2026-11-04"));
        assert_eq!(field(&fields, "ticket"), Some("T-12"));
        assert!(body.starts_with("\n# Checkout"), "{body:?}");
        let listed = "---\ntitle: Plan\ntickets:\n  - 3\n  - 4\nbrief: docs/briefs/2026-11-04-checkout.md\nscope: [a, b]\n---\nbody";
        let (fields, body) = split_front(listed).unwrap();
        assert_eq!(field(&fields, "tickets"), Some("[3, 4]"), "a list under its key");
        assert_eq!(field(&fields, "brief"), Some("docs/briefs/2026-11-04-checkout.md"));
        assert_eq!(field(&fields, "scope"), Some("[a, b]"));
        assert_eq!(body, "body");
        assert!(split_front("# No front matter\n---\n").is_none());
        assert!(split_front("---\ntitle: never closed\n").is_none());
        assert!(split_front("\u{feff}---\r\ntitle: x\r\n---\r\n").is_some(), "a BOM and CRLF");
    }

    #[test]
    fn a_rendered_front_matter_reads_back_the_same() {
        let fields = [("title", "결제: 한 번에 #1".to_owned()), ("date", "2026-11-04".into()), ("status", "draft".into()), ("ticket", "12".into()), ("tickets", "[3, 4]".into()), ("mode", "builder".into())];
        let text = render(&fields, "\n\n# 결제\n");
        assert!(text.starts_with("---\ntitle: \"결제: 한 번에 #1\"\ndate: 2026-11-04\n"), "{text}");
        assert!(text.contains("\ntickets: [3, 4]\n") && text.ends_with("---\n\n# 결제\n"), "{text}");
        let (back, _) = split_front(&text).unwrap();
        for (key, value) in fields {
            assert_eq!(field(&back, key), Some(value.as_str()), "{key}");
        }
    }

    #[test]
    fn a_name_is_a_date_and_a_slug() {
        assert_eq!(slugify("Checkout — one tap, v2!"), "checkout-one-tap-v2");
        assert_eq!(slugify("결제 개선"), "", "nothing ASCII: a slug must be given");
        assert_eq!(slugify(&"a".repeat(80)).len(), 60);
        assert!(is_slug("checkout-v2") && !is_slug("Checkout") && !is_slug("../x") && !is_slug(""));
        assert_eq!(file_name("2026-11-04", "checkout"), "2026-11-04-checkout.md");
        assert_eq!(parse_name("2026-11-04-checkout-v2.md"), Some(("2026-11-04", "checkout-v2")));
        assert_eq!(parse_name("notes.md"), None);
        assert_eq!(parse_name("2026-11-04.md"), None);
        assert_eq!(parse_name("2026-1x-04-a.md"), None);
        assert_eq!((ticket_of("T-12"), ticket_of("#7"), ticket_of("3"), ticket_of("soon")), (Some(12), Some(7), Some(3), None));
    }

    #[test]
    fn a_doc_says_what_its_front_matter_says_and_falls_back_on_its_name() {
        let doc = doc_of(Kind::Brief, Some("web"), Path::new("/r/docs/briefs/2026-11-04-checkout.md"), BRIEF);
        assert_eq!((doc.title.as_str(), doc.date.as_str(), doc.status.as_str()), ("Checkout: one tap", "2026-11-04", "settled"));
        assert_eq!((doc.ticket, doc.mode.as_deref(), doc.project.as_deref()), (Some(12), Some("startup"), Some("web")));
        let bare = doc_of(Kind::Plan, None, Path::new("/r/docs/plans/2026-11-05-checkout.md"), "# Checkout plan\n\nSteps.");
        assert_eq!((bare.title.as_str(), bare.date.as_str(), bare.status.as_str()), ("Checkout plan", "2026-11-05", "draft"), "the heading, the name's date, a draft");
        let nameless = doc_of(Kind::Plan, None, Path::new("/r/docs/plans/odd.md"), "text");
        assert_eq!((nameless.title.as_str(), nameless.date.as_str()), ("odd", ""));
    }

    /// A squad's folder as equipe makes it: a git repository of its own.
    fn squad_folder(name: &str) -> PathBuf {
        let home = scratch(name);
        crate::squad_home::ensure(&home, "acme", None).unwrap();
        crate::squad_home::path(&home, "acme")
    }

    #[test]
    fn a_projects_briefs_then_plans_are_listed_from_the_squads_folder_newest_first_and_nothing_from_outside() {
        let root = scratch("list");
        let web = Target::project(&root, "web").unwrap();
        let briefs = root.join("docs/web/briefs");
        let plans = root.join("docs/web/plans");
        assert_eq!((web.rel(Kind::Brief), web.root()), ("docs/web/briefs".to_owned(), root.join("docs/web")));
        std::fs::create_dir_all(&briefs).unwrap();
        std::fs::create_dir_all(&plans).unwrap();
        std::fs::write(briefs.join("2026-11-01-old.md"), "---\ntitle: Old\nstatus: dropped\n---\n").unwrap();
        std::fs::write(briefs.join("2026-11-04-checkout.md"), BRIEF).unwrap();
        std::fs::write(briefs.join("notes.txt"), "not markdown").unwrap();
        std::fs::write(plans.join("2026-11-05-checkout.md"), "---\ntitle: Plan\nstatus: in_review\nbrief: docs/briefs/2026-11-04-checkout.md\n---\n").unwrap();
        let outside = scratch("list-outside");
        std::fs::write(outside.join("secret.md"), "---\ntitle: Secret\n---\n").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.md"), briefs.join("2026-11-09-link.md")).unwrap();
        std::fs::write(briefs.join("2026-11-08-huge.md"), format!("---\ntitle: Huge\n---\n{}", "x".repeat(MAX as usize + 10))).unwrap();
        let docs = list(&web);
        let titles: Vec<(Kind, &str)> = docs.iter().map(|d| (d.kind, d.title.as_str())).collect();
        assert_eq!(titles, [(Kind::Brief, "Huge"), (Kind::Brief, "Checkout: one tap"), (Kind::Brief, "Old"), (Kind::Plan, "Plan")], "a link out is left out; a big one is read only so far");
        assert_eq!(docs[3].status, "in_review");
        assert!(docs.iter().all(|d| d.project.as_deref() == Some("web")));
        assert_eq!(docs[1].path, root.join("docs/web/briefs/2026-11-04-checkout.md").to_string_lossy());
        // The squad's own work beside them, and another project's: each only its own.
        assert!(list(&Target::squad(&root)).is_empty());
        assert!(list(&Target::project(&root, "api").unwrap()).is_empty());
        assert!(list(&Target::project(&outside, "web").unwrap()).is_empty());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_project_needs_a_name_that_can_be_a_folder_and_a_path_says_whose_it_is() {
        let root = Path::new("/h/squads/acme/home");
        for bad in ["", "..", ".hidden", "a/b", "a\\b"] {
            assert!(Target::project(root, bad).is_err(), "{bad:?}");
        }
        let web = Target::project(root, "web").unwrap();
        assert_eq!(web.path(Kind::Plan, "2026-11-04-x.md"), root.join("docs/web/plans/2026-11-04-x.md"));
        assert_eq!(Target::squad(root).path(Kind::Brief, "2026-11-04-x.md"), root.join("docs/briefs/2026-11-04-x.md"));
        assert_eq!(Target::of_path(root, &root.join("docs/web/plans/2026-11-04-x.md")), Some((web, Kind::Plan)));
        assert_eq!(Target::of_path(root, &root.join("docs/briefs/2026-11-04-x.md")), Some((Target::squad(root), Kind::Brief)));
        assert_eq!(Target::of_path(root, &root.join("docs/plans/briefs/x.md")).map(|(t, k)| (t.project, k)), Some((Some("plans".into()), Kind::Brief)), "a project called plans");
        assert!(Target::of_path(root, &root.join("docs/web/notes/x.md")).is_none());
        assert!(Target::of_path(root, &root.join("docs/web/../../x/plans/a.md")).is_none());
        assert!(Target::of_path(root, Path::new("/elsewhere/docs/plans/x.md")).is_none());
        assert!(Target::of_path(root, Path::new("docs/plans/x.md")).is_none(), "a path in a repository is no folder's");
    }

    #[test]
    fn a_brief_goes_into_the_squads_folder_as_a_change_that_can_be_taken_back_and_never_into_the_repository() {
        let dir = repo("repo");
        let head = git(&dir, &["rev-parse", "HEAD"]);
        let folder = squad_folder("folder-home");
        let target = Target::project(&folder, "web").unwrap();
        let at = save(&target, Kind::Brief, "checkout", BRIEF, "2026-11-04", "pm").unwrap();
        assert_eq!(at, folder.join("docs/web/briefs/2026-11-04-checkout.md"));
        assert_eq!(std::fs::read_to_string(&at).unwrap(), BRIEF);
        // The repository: not a commit, not a file, not a change.
        assert_eq!(git(&dir, &["rev-parse", "HEAD"]), head);
        assert!(git(&dir, &["status", "--porcelain", "--untracked-files=all"]).is_empty());
        assert!(!dir.join("docs").exists());
        // The squad's folder: saved at once, by the PM, for what it is.
        let change = crate::squad_home::changes(&folder, 1).unwrap().remove(0);
        assert_eq!((change.by.as_str(), change.why.as_str()), ("pm", "wrote the brief Checkout: one tap"));
        assert_eq!(change.files, [("saved", "docs/web/briefs/2026-11-04-checkout.md".to_owned())]);
        // A rewrite: the same file, its first date, a change of its own.
        let v2 = BRIEF.replace("settled", "draft");
        assert_eq!(save(&target, Kind::Brief, "checkout", &v2, "2026-11-07", "pm").unwrap(), at, "the same file, its first date");
        assert_eq!(read(&target, Kind::Brief, "checkout").as_deref(), Some(v2.as_str()));
        assert!(read(&target, Kind::Plan, "checkout").is_none());
        // What the person changed in it themselves is kept as theirs, first.
        std::fs::write(&at, "my own words\n").unwrap();
        save(&target, Kind::Brief, "checkout", BRIEF, "2026-11-08", "pm").unwrap();
        let two = crate::squad_home::changes(&folder, 2).unwrap();
        assert_eq!((two[0].by.as_str(), two[1].by.as_str()), ("pm", crate::squad_home::PERSON));
        // Taken back like any change to the folder.
        crate::squad_home::undo(&folder, 1).unwrap();
        assert_eq!(std::fs::read_to_string(&at).unwrap(), "my own words\n");
        // The squad's own work: under docs/ itself.
        let own = save(&Target::squad(&folder), Kind::Plan, "trip", "---\ntitle: Trip\n---\n", "2026-11-04", "pm").unwrap();
        assert_eq!(own, folder.join("docs/plans/2026-11-04-trip.md"));
        assert!(git(&folder, &["status", "--porcelain"]).is_empty(), "everything saved");
        assert!(save(&target, Kind::Brief, "../escape", BRIEF, "2026-11-04", "pm").is_err());
        assert!(save(&target, Kind::Brief, "big", &"x".repeat(MAX as usize + 1), "2026-11-04", "pm").is_err());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(folder.parent().unwrap().parent().unwrap().parent().unwrap());
    }

    #[test]
    fn a_folder_on_the_way_that_is_a_link_elsewhere_is_refused() {
        let root = scratch("linked");
        let elsewhere = scratch("linked-elsewhere");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("docs/web")).unwrap();
        assert!(save(&Target::project(&root, "web").unwrap(), Kind::Plan, "checkout", BRIEF, "2026-11-04", "pm").is_err());
        assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none(), "nothing written through it");
        // No git under it (a bare folder): written in place all the same.
        assert!(save(&Target::squad(&root), Kind::Brief, "x", BRIEF, "2026-11-04", "pm").unwrap().is_file());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&elsewhere);
    }

    #[test]
    fn what_equipe_once_committed_into_a_repository_is_copied_into_the_squads_folder_once_and_left_where_it_was() {
        let dir = repo("web");
        std::fs::create_dir_all(dir.join("docs/briefs")).unwrap();
        std::fs::create_dir_all(dir.join("docs/plans")).unwrap();
        let plan = "---\ntitle: Plan\ndate: 2026-11-05\nstatus: in_review\nbrief: docs/briefs/2026-11-04-checkout.md\ntickets: [3]\n---\n\n# Plan\n";
        std::fs::write(dir.join("docs/briefs/2026-11-04-checkout.md"), BRIEF).unwrap();
        std::fs::write(dir.join("docs/plans/2026-11-05-checkout.md"), plan).unwrap();
        // The project's own docs that equipe didn't write: no front matter, another one, another name.
        std::fs::write(dir.join("docs/briefs/2026-11-01-press.md"), "# Press brief\n").unwrap();
        std::fs::write(dir.join("docs/plans/roadmap.md"), "---\ntitle: Roadmap\ndate: 2026-11-01\nstatus: draft\n---\n").unwrap();
        std::fs::write(dir.join("docs/plans/2026-11-02-launch.md"), "---\ntitle: Launch\nstatus: shipped\n---\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "docs"]);
        let head = git(&dir, &["rev-parse", "HEAD"]);
        let folder = squad_folder("kept-home");
        let target = Target::project(&folder, &crate::board::project_name(&dir)).unwrap();
        let left = left_in_repo(&dir, &target);
        assert_eq!(left.iter().map(|d| d.from.as_str()).collect::<Vec<_>>(), ["docs/briefs/2026-11-04-checkout.md", "docs/plans/2026-11-05-checkout.md"]);
        assert_eq!(left[1].to, target.path(Kind::Plan, "2026-11-05-checkout.md"));
        let copied = keep_from_repos(&folder, std::slice::from_ref(&dir)).unwrap();
        assert_eq!(copied.len(), 2);
        assert_eq!(std::fs::read_to_string(target.path(Kind::Plan, "2026-11-05-checkout.md")).unwrap(), plan, "word for word");
        assert_eq!(list(&target).len(), 2);
        assert_eq!(git(&folder, &["log", "-1", "--format=%s"]), "equipe: kept the briefs and plans from the projects' repositories in the squad's folder", "saved as equipe's");
        assert!(git(&folder, &["status", "--porcelain"]).is_empty());
        // The repository as it was; a second pass finds nothing left to copy.
        assert_eq!(git(&dir, &["rev-parse", "HEAD"]), head);
        assert!(git(&dir, &["status", "--porcelain"]).is_empty());
        assert!(dir.join("docs/briefs/2026-11-04-checkout.md").is_file());
        assert!(keep_from_repos(&folder, std::slice::from_ref(&dir)).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(folder.parent().unwrap().parent().unwrap().parent().unwrap());
    }

    #[test]
    fn a_started_idea_goes_to_the_pm_first_who_releases_it_when_small_or_settled() {
        let text = idea_start_message(12, "One-tap checkout");
        assert!(text.starts_with("[Idea start · T-12]") && text.contains("(One-tap checkout)"), "{text}");
        for part in ["held", "`update_ticket` (status todo)", "small and clear", "release it now", "office hours", "one card at a time", "`write_brief` (ticket 12)", "settled"] {
            assert!(text.contains(part), "{part}: {text}");
        }
    }

    #[test]
    fn the_other_engine_reads_the_brief_sceptically_and_changes_nothing() {
        let p = critique_prompt("# Checkout\nWhy.", Path::new("/r/docs/briefs/2026-11-04-checkout.md"));
        for part in ["do not praise", "evidenced", "Words used without being defined", "assumptions nobody named", "if nothing is built", "Existing code", "`settle` or `revise`", "[blocker|gap]", "in the language of the brief"] {
            assert!(p.contains(part), "{part}");
        }
        assert!(p.contains("<brief>\n# Checkout\nWhy.\n</brief>") && p.contains("2026-11-04-checkout.md"));
        let (bin, args) = critique_command(Engine::Codex, "b", Path::new("/r/b.md"), Path::new("/tmp/a.md"));
        assert!(bin.ends_with("codex"));
        assert_eq!(&args[..3], ["exec", "--sandbox", "read-only"]);
        assert!(args.windows(2).any(|w| w[0] == "--output-last-message" && w[1] == "/tmp/a.md"));
        let (bin, args) = critique_command(Engine::Claude, "b", Path::new("/r/b.md"), Path::new("/tmp/a.md"));
        assert_eq!((bin.as_str(), args[0].as_str()), ("claude", "-p"));
        let denied = args.iter().position(|a| a == "--disallowedTools").unwrap();
        assert!(["Bash", "Edit", "Write"].iter().all(|t| args[denied + 1].contains(t)));
        assert!(!args.iter().any(|a| a.contains("dangerously") || a == "--bare"));
    }

    #[test]
    fn a_brief_needs_a_title_a_mode_and_a_body_and_its_slug_comes_from_the_title() {
        let ok = brief_args(&json!({ "title": "Checkout in one tap", "mode": "startup", "body": "## Problem\nWhy.", "ticket": 12, "project": "web" })).unwrap();
        assert_eq!((ok.slug.as_str(), ok.status.as_str(), ok.ticket, ok.project.as_deref()), ("checkout-in-one-tap", "draft", Some(12), Some("web")), "a draft until the person says");
        assert!(brief_args(&json!({ "title": "결제 개선", "mode": "builder", "body": "x" })).unwrap_err().contains("`slug`"), "nothing ASCII: a slug is asked for");
        let korean = brief_args(&json!({ "title": "결제 개선", "slug": "checkout", "mode": "builder", "body": "x", "ticket": "T-3" })).unwrap();
        assert_eq!((korean.slug.as_str(), korean.ticket), ("checkout", Some(3)));
        assert!(brief_args(&json!({ "title": "x", "mode": "startup" })).is_err(), "no body");
        assert!(brief_args(&json!({ "title": "x", "body": "y" })).unwrap_err().contains("mode"));
        assert!(brief_args(&json!({ "title": "x", "mode": "startup", "body": "y", "status": "done" })).is_err());
        assert!(brief_args(&json!({ "title": "x", "mode": "other", "body": "y" })).is_err());
        assert!(brief_args(&json!({ "title": "x", "slug": "../up", "mode": "startup", "body": "y" })).is_err());
    }

    #[test]
    fn a_briefs_text_is_equipes_front_matter_then_the_body() {
        let brief = brief_args(&json!({ "title": "결제: 한 번에", "slug": "checkout", "mode": "startup", "status": "settled", "ticket": 4, "body": "---\ntitle: mine\n---\n## 문제\n왜." })).unwrap();
        let text = brief_text(&brief, "2026-11-04");
        let (fields, body) = split_front(&text).unwrap();
        let get = |k| field(&fields, k);
        assert_eq!((get("title"), get("date"), get("status"), get("ticket"), get("mode")), (Some("결제: 한 번에"), Some("2026-11-04"), Some("settled"), Some("4"), Some("startup")));
        assert!(body.contains("## 문제") && !body.contains("title: mine"), "the PM's own front matter gives way: {body}");
        let schema = tool();
        assert_eq!(schema["name"], WRITE_BRIEF);
        assert!(schema["description"].as_str().unwrap().contains("settled only on their word"));
    }
}
