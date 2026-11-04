//! `FAKE_CROWD=1`: a squad where nearly every project waits on the person —
//! six projects with 4, 2, 2, 1, 1 and 0 things put down for them, ten more
//! about no one project (20 in all), and two pieces of work under way. The
//! sidebar at its most crowded, to look at how the counts read when almost
//! every row has one. Nothing else waits: no permission asked, no questions,
//! no cards.

use equipe_core::protocol::Ticket;

pub fn on() -> bool {
    std::env::var("FAKE_CROWD").is_ok_and(|v| v == "1")
}

/// The projects, in the sidebar's order, and how many things in each wait on the person.
pub const PROJECTS: [(&str, usize); 6] = [("equipe", 4), ("equipe-site", 2), ("equipe-app", 2), ("equipe-desktop", 1), ("equipe-design", 1), ("equipe-docs", 0)];

/// What waits on the person about no one project.
const OUTSIDE: usize = 10;

pub fn projects() -> Vec<std::path::PathBuf> {
    PROJECTS.iter().map(|(name, _)| super::repo(name)).collect()
}

/// The board: what waits, by project and outside them, and two under way.
pub fn tickets() -> Vec<Ticket> {
    let make = |id: u32, status: &str, project: Option<&str>, who: &str, title: String| -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": title, "body": "", "status": status, "owner": who, "assignee": who,
            // About no one project: a task, kept in the squad's own folder.
            "kind": if project.is_some() { "code" } else { "task" },
            "project": project.map(super::repo), "depends_on": [], "created_by": "pm",
            "note": (status == "waiting").then_some("Waiting on you:\n- [ ] a look"),
        }))
        .unwrap()
    };
    let owners = ["frontend", "designer", "pm"];
    let mut out = Vec::new();
    let mut id = 200;
    for (project, waiting) in PROJECTS {
        for n in 0..waiting {
            out.push(make(id, "waiting", Some(project), owners[n % owners.len()], format!("{project}: sign-off {}", n + 1)));
            id += 1;
        }
    }
    for n in 0..OUTSIDE {
        out.push(make(id, "waiting", None, owners[n % owners.len()], format!("Squad: sign-off {}", n + 1)));
        id += 1;
    }
    out.push(make(70, "in_progress", Some("equipe"), "backend", "Order API pagination".into()));
    out.push(make(47, "in_progress", Some("equipe-docs"), "frontend", "Docs search".into()));
    out
}
