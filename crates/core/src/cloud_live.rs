//! What is live on the equipe platform, for a cloud team.
//!
//! A cloud team's products go live by the platform's `deploy_site`, not by a
//! git push or a go-live command equipe runs, so equipe saw no deploy and took
//! work on main as live. It was not: at Golden Boot (2026-11-25) a tidied
//! booking form and a phone speed fix landed on main at 10:58 and 11:11, the
//! person's request closed as live, the PM told them it was done — and the
//! site went live with it only at 16:00, after someone asked.
//!
//! The platform keeps the commit each app is live from (`live_commit` in
//! `/v1/resources`, read by `crate::sprint::platform`). A project whose
//! repository holds one of those commits goes live by a deploy
//! (`crate::requests::Shipped::by_deploy`), and a landing on main
//! (a [`DeploySource::Main`] record) is live once its commit is an ancestor of
//! a live commit. Plain git, no model; each answer is kept, since whether one
//! commit contains another never changes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use equipe_protocol::deploys::{Deploy, DeploySource};
use equipe_protocol::TicketId;

/// What the platform's live commits say of a squad's landings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Carried {
    /// Tickets whose landing a live version contains.
    pub live: HashSet<TicketId>,
    /// The projects (by path, as `Shipped::by_deploy` keys them) whose
    /// repository holds a live commit: they go live only by a deploy.
    pub by_deploy: HashSet<String>,
    /// Landings on main of a deploy-only project no live version contains:
    /// `(ticket, landed at)`, the oldest first.
    pub waiting: Vec<(TicketId, u64)>,
}

/// [`carried_by`] with git in each project's folder.
pub fn carried(deploys: &[Deploy], commits: &[String]) -> Carried {
    carried_by(deploys, commits, has_commit, contains)
}

/// What `commits` (the apps' live commits) carry of `deploys`' landings on
/// main. `has(path, commit)`: the project's repository has the commit;
/// `ancestor(path, landed, live)`: `live` contains `landed`.
pub fn carried_by(deploys: &[Deploy], commits: &[String], has: impl Fn(&Path, &str) -> bool, ancestor: impl Fn(&Path, &str, &str) -> bool) -> Carried {
    let mut out = Carried::default();
    if commits.is_empty() {
        return out;
    }
    let mut lives: HashMap<PathBuf, Vec<&String>> = HashMap::new();
    for d in deploys.iter().filter(|d| d.source == DeploySource::Main) {
        lives.entry(d.path.clone()).or_insert_with(|| commits.iter().filter(|c| has(&d.path, c)).collect());
    }
    for (path, live) in &lives {
        if !live.is_empty() {
            out.by_deploy.insert(path.to_string_lossy().into_owned());
        }
    }
    let mut waiting: Vec<(TicketId, u64)> = Vec::new();
    for d in deploys.iter().filter(|d| d.source == DeploySource::Main && !d.tickets.is_empty()) {
        let Some(live) = lives.get(&d.path).filter(|l| !l.is_empty()) else { continue };
        if live.iter().any(|c| ancestor(&d.path, &d.to, c)) {
            out.live.extend(d.tickets.iter().copied());
        } else {
            // Only what the person can see waits to go live: the record's items.
            for t in d.items.iter().flat_map(|i| i.tickets.iter().copied()) {
                waiting.push((t, d.at));
            }
        }
    }
    waiting.retain(|(t, _)| !out.live.contains(t));
    waiting.sort_by_key(|(t, at)| (*at, *t));
    waiting.dedup_by_key(|(t, _)| *t);
    out.waiting = waiting;
    out
}

/// Work on main a deploy-only project's live version lacks this long is
/// brought to the PM (unix seconds): long enough for a deploy right after the
/// landing, short enough that the person isn't told "done" for hours first.
pub const NOT_LIVE_AFTER: u64 = 20 * 60;

/// Of `waiting` (ticket, landed at), those on main past [`NOT_LIVE_AFTER`]
/// at `now` that the PM wasn't told of yet.
pub fn due(waiting: &[(TicketId, u64)], told: &HashSet<TicketId>, now: u64) -> Vec<(TicketId, u64)> {
    waiting.iter().filter(|(t, at)| (NOT_LIVE_AFTER..=NOT_LIVE_UNTIL).contains(&now.saturating_sub(*at)) && !told.contains(t)).copied().collect()
}

/// Older landings are not brought up any more: a team that keeps work off
/// the live site on purpose was told already, or decided it long ago.
pub const NOT_LIVE_UNTIL: u64 = 3 * 86_400;

/// What the PM is handed: each ticket with its title and how long it has
/// waited, and what to do — deploy main live and check it, or, where the
/// squad's policy asks before deploys, ask the person.
pub fn nudge_message(due: &[(TicketId, String, u64)], ask_first: bool, now: u64) -> String {
    let lines: Vec<String> = due.iter().map(|(t, title, at)| format!("- T-{t} \"{title}\" — on main for {} min", now.saturating_sub(*at) / 60)).collect();
    let todo = if ask_first {
        "The squad's policy asks the person before a deploy: ask them now, in one plain line, whether to put it live."
    } else {
        "Put main live now (deploy_site from the main checkout, without preview), then open the live address at desktop and phone width and check each change is there."
    };
    format!(
        "[Not live yet] This work landed on main, but the live site doesn't have it: the platform's live version was built before it.\n{}\n\n{todo} \
Until it is live, don't tell the person it is done or live; say it is finished and going live. If it must not go live yet, tell them why in one line.",
        lines.join("\n")
    )
}

/// Git's answers by (repository, commit, live commit — empty for "has it").
type Answers = Mutex<HashMap<(PathBuf, String, String), bool>>;

fn answers() -> &'static Answers {
    static C: OnceLock<Answers> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// At most this many answers are kept; then the oldest ones go all at once.
const KEPT: usize = 4096;

fn kept(key: (PathBuf, String, String), ask: impl FnOnce() -> Option<bool>) -> bool {
    if let Some(v) = answers().lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
        return *v;
    }
    // A git that couldn't answer (no repository, a lock) is asked again next time.
    let Some(v) = ask() else { return false };
    let mut cache = answers().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= KEPT {
        cache.clear();
    }
    cache.insert(key, v);
    v
}

fn git(path: &Path, args: &[&str]) -> Option<bool> {
    let status = Command::new("git").arg("-C").arg(path).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().ok()?;
    // 0: yes; 1: no (merge-base --is-ancestor); anything else: git couldn't say.
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

fn hex(c: &str) -> bool {
    c.len() >= 7 && c.len() <= 64 && c.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// Whether the repository at `path` has `commit`.
pub fn has_commit(path: &Path, commit: &str) -> bool {
    if !hex(commit) || !path.is_dir() {
        return false;
    }
    kept((path.to_path_buf(), commit.to_owned(), String::new()), || {
        let spec = format!("{commit}^{{commit}}");
        // cat-file exits 128 for a missing object: a plain "no".
        let status = Command::new("git").arg("-C").arg(path).args(["cat-file", "-e", &spec]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().ok()?;
        Some(status.success())
    })
}

/// Whether `live` contains `landed` in the repository at `path`.
pub fn contains(path: &Path, landed: &str, live: &str) -> bool {
    if !hex(landed) || !hex(live) {
        return false;
    }
    if landed == live {
        return true;
    }
    kept((path.to_path_buf(), landed.to_owned(), live.to_owned()), || git(path, &["merge-base", "--is-ancestor", landed, live]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::deploys::NoteItem;

    const APP: &str = "/home/tenki/data/product/golden-boot";
    const NOTES: &str = "/home/tenki/data/product";

    fn landing(id: u64, path: &str, to: &str, at: u64, tickets: &[TicketId]) -> Deploy {
        Deploy {
            id,
            project: Path::new(path).file_name().unwrap().to_string_lossy().into_owned(),
            path: PathBuf::from(path),
            source: DeploySource::Main,
            environment: None,
            url: None,
            from: None,
            to: to.into(),
            at,
            last_at: at,
            items: if tickets.is_empty() { Vec::new() } else { vec![NoteItem { title: "a change".into(), tickets: tickets.to_vec(), request: None, request_title: None, session: None }] },
            internal: 0,
            tickets: tickets.to_vec(),
            told: false,
        }
    }

    /// A small history: the app repository's main went a1 → b2 → c3 → d4; the
    /// notes repository (no app) has its own commits.
    fn history() -> (impl Fn(&Path, &str) -> bool, impl Fn(&Path, &str, &str) -> bool) {
        let order = ["a1a1a1a", "b2b2b2b", "c3c3c3c", "d4d4d4d"];
        let has = move |p: &Path, c: &str| p == Path::new(APP) && order.contains(&c);
        let ancestor = move |p: &Path, x: &str, y: &str| {
            p == Path::new(APP) && order.iter().position(|o| *o == x).zip(order.iter().position(|o| *o == y)).is_some_and(|(i, j)| i <= j)
        };
        (has, ancestor)
    }

    /// Golden Boot's case: T-37 and T-38 landed (c3, d4) while the site was
    /// live from b2. They are on main, not live, and wait; the notes
    /// project, which no app is built from, keeps "on main is live".
    #[test]
    fn a_landing_is_live_only_once_a_live_version_contains_it() {
        let (has, ancestor) = history();
        let deploys = [landing(1, APP, "b2b2b2b", 100, &[33]), landing(2, APP, "c3c3c3c", 200, &[37]), landing(3, APP, "d4d4d4d", 300, &[38]), landing(4, NOTES, "e5e5e5e", 310, &[])];
        let c = carried_by(&deploys, &["b2b2b2b".into()], &has, &ancestor);
        assert_eq!(c.live, HashSet::from([33]));
        assert_eq!(c.by_deploy, HashSet::from([APP.to_owned()]), "only the app's project waits for deploys");
        assert_eq!(c.waiting, vec![(37, 200), (38, 300)]);
        // The deploy of main at d4: everything landed is live.
        let c = carried_by(&deploys, &["d4d4d4d".into()], &has, &ancestor);
        assert_eq!(c.live, HashSet::from([33, 37, 38]));
        assert!(c.waiting.is_empty());
    }

    /// No live commit known (a desktop squad, the platform not read yet, an
    /// app deployed before commits were recorded): nothing changes.
    #[test]
    fn without_live_commits_nothing_is_held_back() {
        let (has, ancestor) = history();
        let deploys = [landing(1, APP, "c3c3c3c", 200, &[37])];
        assert_eq!(carried_by(&deploys, &[], &has, &ancestor), Carried::default());
        // A live commit no repository of the squad has (another team's? a branch never fetched): no project waits.
        let c = carried_by(&deploys, &["ffffffff".into()], &has, &ancestor);
        assert!(c.by_deploy.is_empty() && c.live.is_empty() && c.waiting.is_empty(), "{c:?}");
    }

    /// The PM hears of work on main the live site lacks once, after a while.
    #[test]
    fn work_left_off_the_live_site_is_brought_up_once_after_a_while() {
        let waiting = [(37, 1_000), (38, 1_800)];
        assert!(due(&waiting, &HashSet::new(), 1_000 + NOT_LIVE_AFTER - 1).is_empty(), "a deploy right after the landing is fine");
        assert_eq!(due(&waiting, &HashSet::new(), 1_000 + NOT_LIVE_AFTER), vec![(37, 1_000)]);
        assert!(due(&waiting, &HashSet::from([37]), 1_000 + NOT_LIVE_AFTER + 60).is_empty(), "told once");
        assert!(due(&waiting, &HashSet::new(), 1_800 + NOT_LIVE_UNTIL + 1).is_empty(), "days-old landings aren't brought up");
        let m = nudge_message(&[(37, "Tidy the match review request form".into(), 1_000)], false, 1_000 + 25 * 60);
        assert!(m.starts_with("[Not live yet]") && m.contains("T-37 \"Tidy the match review request form\" — on main for 25 min") && m.contains("deploy_site") && m.contains("don't tell the person it is done"), "{m}");
        let ask = nudge_message(&[(37, "x".into(), 1_000)], true, 1_000 + 25 * 60);
        assert!(ask.contains("asks the person before a deploy") && !ask.contains("Put main live now"), "{ask}");
    }

    /// Real git: a repository with two commits, the live one the first.
    #[test]
    fn git_answers_whether_a_live_commit_contains_a_landing() {
        let dir = std::env::temp_dir().join(format!("equipe-cloud-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(&dir).args(args).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@example.com").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@example.com").output().unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["commit", "-q", "--allow-empty", "-m", "one"]);
        let one = run(&["rev-parse", "HEAD"]);
        run(&["commit", "-q", "--allow-empty", "-m", "two"]);
        let two = run(&["rev-parse", "HEAD"]);
        assert!(has_commit(&dir, &one) && has_commit(&dir, &two));
        assert!(!has_commit(&dir, "0123456789abcdef0123456789abcdef01234567"));
        assert!(contains(&dir, &one, &two) && !contains(&dir, &two, &one));
        let deploys = [landing(1, &dir.to_string_lossy(), &two, 50, &[7])];
        let c = carried(&deploys, std::slice::from_ref(&one));
        assert_eq!((c.live.len(), c.waiting.clone()), (0, vec![(7, 50)]));
        let c = carried(&deploys, std::slice::from_ref(&two));
        assert_eq!((c.live, c.waiting.len()), (HashSet::from([7]), 0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
