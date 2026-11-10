//! Briefs and plans (`equipe_core::briefs`): written into the fake squad's
//! folder the way the PM's `write_brief` writes them — never into the fake
//! projects' repositories — and sent when a project page or the docs screen asks. web has
//! two briefs and two plans, api a draft, app one plan — the same magic-link plan as web's,
//! saved under both and since edited in one (a pair of copies that differ) — and the squad
//! its own launch-week plan. web's repository still holds an older copy of its checkout
//! plan (told only). A search finds them by their words; keeping one copy removes the other
//! from the fake folder, and taking that back writes them all again. `FAKE_DOCS=none`: no
//! project has any. `FAKE_DECIDE=office`: the PM's office-hours card waits.

use equipe_core::briefs::{self, InRepo, Kind, ProjectDoc, Target};
use equipe_core::protocol::decide::{Choice, Question, QuestionSet};
use equipe_core::protocol::{Command, Event};
use tokio::sync::broadcast;

fn none() -> bool {
    std::env::var("FAKE_DOCS").is_ok_and(|v| v == "none")
}

const PROJECTS: [&str; 3] = ["web", "app", "api"];

/// What the PM left, written as it writes them.
pub fn seed() {
    if none() {
        return;
    }
    // The folder itself, as the daemon makes it for a squad: a save makes only what is under it.
    let _ = std::fs::create_dir_all(folder());
    let write = |project: &str, kind: Kind, slug: &str, date: &str, fields: &[(&str, String)], body: &str| {
        let target = if project.is_empty() {
            Target::squad(&folder())
        } else if let Ok(target) = Target::project(&folder(), project) {
            target
        } else {
            return;
        };
        if let Err(why) = briefs::save(&target, kind, slug, &briefs::render(fields, body), date, "pm") {
            eprintln!("fake daemon: {project}'s {slug} wasn't written: {why}");
        }
    };
    let brief_path = folder().join("docs/web/briefs/2026-11-02-one-tap-checkout.md").to_string_lossy().into_owned();
    let f = |pairs: &[(&'static str, &str)]| pairs.iter().map(|(k, v)| (*k, (*v).to_owned())).collect::<Vec<_>>();
    write(
        "web",
        Kind::Brief,
        "one-tap-checkout",
        "2026-11-02",
        &f(&[("title", "One-tap checkout"), ("date", "2026-11-02"), ("status", "settled"), ("ticket", "42"), ("mode", "startup")]),
        "## Goal\n\nFrom cart to paid on one screen — fewer people dropping out of their first purchase.\n\n## Who it is for\n\nSomeone buying for the first time on a phone. Today 38% leave somewhere in the three screens (last 30 days of analytics).\n\n## Demand\n\n- A quarter of support questions are \"Where does checkout end?\"\n- Two competitors already check out on one screen\n\n## The option picked\n\n**B. One-screen checkout + saved methods** (Recommended) — 2 weeks, the payment API unchanged.\n\n## Scope\n\n- In: cards and wallets, saved methods\n- Out: international payments, instalment choice\n\n## Done means\n\nFirst-purchase conversion +5 pp, within 4 weeks.\n",
    );
    write(
        "web",
        Kind::Plan,
        "one-tap-checkout",
        "2026-11-03",
        &f(&[("title", "One-tap checkout — plan"), ("date", "2026-11-03"), ("status", "in_review"), ("brief", brief_path.as_str()), ("scope", "selective"), ("tickets", "[42, 51]")]),
        "## Steps\n\n1. Payment method choice on one screen (T-42)\n2. Load saved methods\n3. Pin the summary to the bottom (T-51)\n\n## Tests\n\n- Card and wallet, each all the way through\n- With no saved method\n",
    );
    write(
        "web",
        Kind::Brief,
        "gift-cards",
        "2026-10-20",
        &f(&[("title", "Gift cards"), ("date", "2026-10-20"), ("status", "dropped"), ("mode", "startup")]),
        "## Why we dropped it\n\nWe couldn't find a single person who wanted it. Most said what they use now (bank transfer) is enough.\n",
    );
    write(
        "api",
        Kind::Brief,
        "rate-limits",
        "2026-11-04",
        &f(&[("title", "Rate limits"), ("date", "2026-11-04"), ("status", "draft"), ("mode", "builder")]),
        "## Goal\n\nOne partner sending a burst never slows the others down.\n\n## Not known yet\n\n- Where each partner's limit is set\n",
    );
    // The same plan saved under web and app, edited since under app.
    let magic = f(&[("title", "Magic-link sign-in — plan"), ("date", "2026-11-05"), ("status", "settled"), ("tickets", "[70]")]);
    let steps = "## Steps\n\n1. Send the link by email (T-70)\n2. A used or expired link says so\n";
    write("web", Kind::Plan, "magic-link-sign-in", "2026-11-05", &magic, steps);
    write("app", Kind::Plan, "magic-link-sign-in", "2026-11-05", &magic, &format!("{steps}3. Open the link in the app when it is installed\n"));
    // The squad's own work, outside the projects.
    write(
        "",
        Kind::Plan,
        "launch-week",
        "2026-11-06",
        &f(&[("title", "Launch week checklist"), ("date", "2026-11-06"), ("status", "draft"), ("tickets", "[117]")]),
        "## Before the day\n\n- App Store submission (T-117)\n- The press note\n",
    );
}

/// The fake squad's folder, where its briefs and plans are kept.
pub fn folder() -> std::path::PathBuf {
    super::root().join("home")
}

/// Every project's briefs and plans, then the squad's own, as the daemon reads them.
fn docs() -> Vec<ProjectDoc> {
    let mut out: Vec<ProjectDoc> = PROJECTS.iter().filter_map(|p| Target::project(&folder(), p).ok()).flat_map(|t| briefs::list(&t)).collect();
    out.extend(briefs::list(&Target::squad(&folder())));
    out
}

/// What the daemon sends of them: the docs, their copies, and the older copy
/// web's repository still holds (made up: the fake repository has none).
fn catalog() -> Event {
    let docs = docs();
    let copies = briefs::copies(&folder(), &docs);
    let checkout = folder().join("docs/web/plans/2026-11-03-one-tap-checkout.md");
    let in_repos = if none() {
        Vec::new()
    } else {
        vec![InRepo { project: "web".into(), kind: Kind::Plan, path: "docs/plans/2026-11-03-one-tap-checkout.md".into(), kept: checkout.is_file().then(|| checkout.to_string_lossy().into_owned()) }]
    };
    Event::ProjectDocs { squad: "demo".into(), docs, copies, in_repos }
}

/// `ProjectDocs`, `SearchDocs`, `KeepDocCopy` — and taking a kept copy back
/// (`UndoHomeChange` with [`UNDO`]): answered at once.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    match command {
        Command::ProjectDocs { .. } => {}
        Command::SearchDocs { query, .. } => {
            let paths = briefs::search(&folder(), &docs(), query);
            let _ = events.send(Event::DocsFound { squad: "demo".into(), query: query.clone(), paths });
            return true;
        }
        Command::KeepDocCopy { keep, .. } => match briefs::keep_copy(&folder(), &docs(), keep) {
            Ok(kept) => {
                let undo = Some(kept.change.map_or_else(|| UNDO.to_owned(), |c| c.id));
                let _ = events.send(Event::DocCopyKept { squad: "demo".into(), kept: kept.kept, removed: kept.removed, undo });
            }
            Err(why) => eprintln!("fake daemon: no copy kept: {why}"),
        },
        Command::UndoHomeChange { change, .. } if change == UNDO => seed(),
        _ => return false,
    }
    let _ = events.send(catalog());
    true
}

/// The change a kept copy is taken back by: the fake folder keeps no history,
/// so taking it back writes every doc again.
const UNDO: &str = "fake-docs-kept";

/// Whether the PM's office-hours card waits (`FAKE_DECIDE=office`).
pub fn office() -> bool {
    std::env::var("FAKE_DECIDE").is_ok_and(|v| v == "office")
}

/// The second forcing question, pushed back on: the person's last answer
/// was vague, so the card says so and asks for what shows it.
pub fn office_hours_set(now: u64) -> QuestionSet {
    let choice = |label: &str, description: &str| Choice { label: label.into(), description: description.into(), ..Choice::default() };
    QuestionSet {
        id: 9101,
        session: 1,
        member: "pm".into(),
        ticket: Some(57),
        questions: vec![Question {
            question: "You said \"everyone finds it painful\" — who is getting by today, and with what? Pick the closest — and add a number or a real example if you have one.".into(),
            header: "Today 2/6".into(),
            options: vec![
                choice("A spreadsheet sent by email (Recommended)", "3–4 a week in the support log — the way seen most"),
                choice("Sorted out by hand in a group chat", "Two small shops said so — how many more, we don't know yet"),
                choice("They give up and phone instead", "No evidence yet: pick this and the next question checks it"),
            ],
            multi_select: false,
            ..Question::default()
        }],
        asked_at: now.saturating_sub(40),
        review: false,
        retro: None,
    }
}
