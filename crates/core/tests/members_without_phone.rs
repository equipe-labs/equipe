//! Members of a squad without a mobile project get exactly what they got
//! before the phone tools existed: the same charter, the same skills and the
//! same board tools, byte for byte.
//!
//! `members_without_phone.txt` holds a SHA-256 of each, written from the
//! commit before the phone tools (`EQUIPE_BLESS_PINNED=1` writes it again —
//! only do that for a change members *should* see, and say so in review).

use std::fmt::Write as _;

use equipe_core::browser::BrowserProfile;
use equipe_core::protocol::Engine;
use equipe_core::roles::{self, CUSTOM, Context, ROLES, Subagents, Teammate};
use equipe_core::skills::{self, Delivery};

const PINNED: &str = "tests/members_without_phone.txt";

fn sha(text: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, text.as_bytes());
    digest.as_ref().iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// JSON with every object's keys in sorted order: the same text whether or
/// not something in the build turns on serde_json's `preserve_order` (GPUI
/// does, in a whole-workspace build).
fn canonical(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let fields: Vec<String> = keys.iter().map(|k| format!("{}:{}", serde_json::to_string(k).unwrap(), canonical(&map[k.as_str()]))).collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => format!("[{}]", items.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

/// Every (role, squad, engine) a member is told something in, and what it is told.
fn told_now() -> Vec<(String, String)> {
    let team = [
        Teammate { name: "pm", role: roles::PM },
        Teammate { name: "designer", role: "designer" },
        Teammate { name: "frontend", role: "frontend" },
        Teammate { name: "backend", role: "backend" },
        Teammate { name: "mobile", role: "mobile" },
        Teammate { name: "qa", role: "qa" },
    ];
    let projects = ["web".to_owned(), "server".to_owned()];
    let profiles = [BrowserProfile { name: "jh".into(), label: "jh@x".into() }];
    let memory = "- [tax](tax.md) — a fact\n- [deploy](deploy.md) — another fact";
    let mut out = Vec::new();
    for role in ROLES.iter().chain([&CUSTOM]) {
        let full = Context {
            team: &team,
            projects: &projects,
            hq: Some("web"),
            equipe: Some("/Users/x/.local/bin/equipe"),
            board_tools: true,
            language: Some("English"),
            screen_language: Some("English"),
            browser: true,
            pm_accepts: true,
            profiles: &profiles,
            desktop: false,
            design: role.id == "designer",
            home: Some("/Users/x/.equipe/squads/acme/home"),
            memory: Some(memory),
            facts: 2,
            edits: role.edits,
            skills: Delivery::Plugin("/h/skills/1.0/x"),
            guard: true,
            subagents: Subagents::of(Engine::Claude, true, false, None),
            helpers: false,
            phone: false,
        };
        let bare = Context { board_tools: false, browser: false, design: false, pm_accepts: false, profiles: &[], guard: false, ..full };
        let codex = Context { skills: Delivery::Files("/h/skills/1.0/x"), subagents: None, helpers: true, guard: false, ..full };
        for (variant, ctx) in [("full", full), ("bare", bare), ("codex", codex)] {
            let tag = format!("{}.{variant}", role.id);
            out.push((format!("{tag}.charter"), roles::charter(role, role.id, &ctx)));
            out.push((format!("{tag}.inline"), roles::charter(role, role.id, &Context { skills: Delivery::Inline, ..ctx })));
            let offered = skills::offered(role, &ctx);
            let text = offered.iter().map(|s| format!("{}\n{}\n{:?}", s.name, s.description, s.files)).collect::<Vec<_>>().join("\n\n");
            out.push((format!("{tag}.skills"), text));
        }
        for helped in [false, true] {
            let tools = canonical(&serde_json::Value::Array(equipe_core::helpers::with_board(role.id, helped)));
            out.push((format!("{}.tools{}", role.id, if helped { ".helped" } else { "" }), tools));
        }
    }
    out
}

#[test]
fn a_squad_without_a_mobile_project_tells_its_members_exactly_what_it_did() {
    let now: String = told_now().iter().map(|(tag, text)| format!("{tag}\t{}\n", sha(text))).collect();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(PINNED);
    if std::env::var_os("EQUIPE_BLESS_PINNED").is_some() {
        equipe_core::bless::here(env!("CARGO_MANIFEST_DIR"));
        std::fs::write(&path, &now).unwrap();
        return;
    }
    let pinned = std::fs::read_to_string(&path).expect("the pinned hashes");
    let changed: Vec<&str> = now.lines().zip(pinned.lines()).filter(|(a, b)| a != b).map(|(a, _)| a.split('\t').next().unwrap_or(a)).collect();
    assert!(changed.is_empty(), "members without a mobile project are told something new: {changed:?}");
    assert_eq!(now.lines().count(), pinned.lines().count(), "the same members, told the same things");
}

#[test]
fn in_a_squad_with_a_mobile_app_qa_and_mobile_are_told_of_the_phone_and_given_its_skill() {
    let team = [Teammate { name: "qa", role: "qa" }];
    let projects = ["app".to_owned()];
    for id in ["qa", "mobile"] {
        let role = roles::role(id).unwrap();
        let ctx = Context {
            team: &team,
            projects: &projects,
            hq: None,
            equipe: None,
            board_tools: true,
            language: None,
            screen_language: None,
            browser: false,
            pm_accepts: true,
            profiles: &[],
            desktop: false,
            design: false,
            home: None,
            memory: None,
            facts: 0,
            edits: role.edits,
            skills: Delivery::Plugin("/h/skills/1.0/x"),
            guard: true,
            subagents: Subagents::of(Engine::Claude, true, false, None),
            helpers: false,
            phone: true,
        };
        let charter = roles::charter(role, id, &ctx);
        assert!(charter.contains("the `phone` tools") && charter.contains("`mobile-qa`"), "{id}");
        assert!(skills::offered(role, &ctx).iter().any(|s| s.name == "mobile-qa"), "{id}");
        let without = roles::charter(role, id, &Context { phone: false, ..ctx });
        assert!(!without.contains("mobile-qa"));
    }
}
