//! `FAKE_PRIORITY=1`: a squad with many things waiting on the person, of every
//! kind, where a few hold a lot of work up — an admin sign-in that blocks five
//! tickets, a production migration that blocks four and every deploy, design
//! picks that block one each, and approvals and sign-offs that block nothing.
//! Needs you by importance, against Needs you by kind.

use equipe_core::protocol::Ticket;

pub fn on() -> bool {
    std::env::var("FAKE_PRIORITY").is_ok_and(|v| v == "1")
}

pub fn tickets() -> Vec<Ticket> {
    let make = |id: u32, status: &str, who: &str, title: &str, deps: &[u32], priority: &str| -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": title, "body": "", "status": status, "owner": who, "assignee": who,
            "kind": "code", "project": super::repo("web"), "depends_on": deps, "created_by": "pm",
            "priority": priority, "updated_at": 1_700_000_000u64 + u64::from(id) * 600,
            "note": (status == "waiting").then_some("Waiting on you:\n- [ ] your part"),
        }))
        .unwrap()
    };
    let mut out = vec![
        // What waits on the person.
        make(100, "waiting", "backend", "AWS admin sign-in", &[], "normal"),
        make(101, "waiting", "backend", "Approve the production DB migration", &[], "normal"),
        make(102, "waiting", "designer", "Pick the hero image", &[], "normal"),
        make(103, "waiting", "designer", "Pick the font pairing", &[], "normal"),
        make(104, "waiting", "pm", "Confirm the launch date with the client", &[], "urgent"),
        make(105, "waiting", "frontend", "Review the copy tone", &[], "normal"),
        make(106, "waiting", "designer", "Share the logo files", &[], "low"),
        make(107, "waiting", "backend", "Choose an analytics vendor", &[], "normal"),
        make(130, "review", "frontend", "Checkout page redesign", &[], "normal"),
        make(131, "proposed", "frontend", "Add a dark mode toggle", &[], "normal"),
        make(132, "proposed", "backend", "Cache the product list", &[], "low"),
        make(134, "done", "frontend", "Header navigation tidy", &[], "normal"),
        // What waits on them.
        make(110, "todo", "backend", "Provision the staging network", &[100], "normal"),
        make(111, "todo", "backend", "Create the build roles", &[100], "normal"),
        make(112, "todo", "backend", "Set up the log bucket", &[100], "normal"),
        make(113, "todo", "backend", "Move the queue to the new account", &[100], "normal"),
        make(114, "todo", "backend", "Rotate the access keys", &[100], "normal"),
        make(115, "todo", "backend", "Backfill the order totals", &[101], "normal"),
        make(116, "todo", "backend", "Drop the old coupon column", &[101], "normal"),
        make(117, "todo", "backend", "Reindex the search tables", &[101], "normal"),
        make(119, "todo", "backend", "Deploy the API servers to production", &[101], "normal"),
        make(120, "todo", "backend", "Deploy the web app to production", &[119], "normal"),
        make(121, "todo", "frontend", "Build the landing hero", &[102], "normal"),
        make(122, "todo", "frontend", "Apply the type scale", &[103], "high"),
        make(123, "todo", "backend", "Wire the analytics events", &[107], "normal"),
        make(124, "todo", "backend", "Build the funnel report", &[107], "normal"),
        make(70, "in_progress", "backend", "Order API pagination", &[], "normal"),
        make(47, "in_progress", "frontend", "Product detail page image gallery", &[], "normal"),
    ];
    // A run that stopped without a hand-in, and accepted work not on main yet.
    let mut stalled = make(133, "in_progress", "frontend", "Gallery zoom gestures", &[], "normal");
    stalled.execution = serde_json::from_value(serde_json::json!({ "state": "stalled", "session": 4, "operation": "run", "generation": 1, "heartbeat_at": 0, "lease_expires_at": 0, "reason": "lease_expired" })).unwrap();
    out.push(stalled);
    for t in out.iter_mut().filter(|t| t.id == 134) {
        t.branch = Some("equipe/t-134-nav".into());
        t.merged = Some(false);
    }
    out
}
