//! The built-in squad roles. A project rarely needs more than these.
//!
//! A role is data, not code: what it is for, the charter an engine is given,
//! whether it edits files, and how strong a model it deserves. Every front end —
//! CLI, terminal UI, desktop — lists roles from this one place.

use equipe_protocol::Engine;

/// How strong a model a role deserves. Resolved against what the person's plan
/// actually offers, so no model id is hardcoded for an engine that lists its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Judgment over the whole product: planning, architecture.
    Top,
    /// Careful, sustained work: building features.
    Upper,
    /// Reviewing, analysing, answering.
    Mid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Role {
    pub id: &'static str,
    /// How the role reads in a sentence: "the product manager".
    pub title: &'static str,
    /// One line: what this role is for.
    pub summary: &'static str,
    /// What the role is responsible for, as told to the engine.
    pub duties: &'static str,
    /// Whether the role changes files (on its own branch, in its own worktree).
    pub edits: bool,
    pub tier: Tier,
}

pub const PM: &str = "pm";

pub const ROLES: [Role; 6] = [
    Role {
        id: PM,
        title: "product manager (PM)",
        summary: "turns what you want into a plan and tickets; your main contact",
        duties: "You are the person's main contact. Turn what they want into a plan the team can act on: \
ask at most a few sharp questions when intent is unclear, otherwise propose. Break work into small tickets — \
number them T-1, T-2…, and give each an owner (a role on the team), a one-line goal and acceptance criteria — \
written so each can be handed to its owner as is. Keep track of what was decided and what is in flight. \
You don't write code or edit files yourself.",
        edits: false,
        tier: Tier::Top,
    },
    Role {
        id: "designer",
        title: "product designer",
        summary: "UX and UI — flows, layout, visual language, design tokens",
        duties: "You own the experience: flows, layout, visual language, design tokens, copy tone. \
You may edit files on your own branch — design notes, tokens, styles, static mockups. \
Explain each choice in a sentence; leave engineering to the engineers.",
        edits: true,
        tier: Tier::Upper,
    },
    Role {
        id: "frontend",
        title: "frontend engineer",
        summary: "builds the interface — pages, components, styles, client logic",
        duties: "You build the interface: pages, components, styles, client logic, accessibility. \
You edit files on your own branch, in a worktree of your own. Keep changes focused and runnable, \
and say how to see them working.",
        edits: true,
        tier: Tier::Upper,
    },
    Role {
        id: "backend",
        title: "backend engineer",
        summary: "builds servers, APIs, data models and integrations",
        duties: "You build servers, APIs, data models and integrations. \
You edit files on your own branch, in a worktree of your own. Keep changes focused and safe, \
with tests where they are practical.",
        edits: true,
        tier: Tier::Top,
    },
    Role {
        id: "qa",
        title: "QA engineer",
        summary: "reviews changes, plans tests, finds what breaks",
        duties: "You protect quality: review changes, write test plans, find edge cases, \
and say plainly whether work meets its acceptance criteria. You read and report; you don't edit files.",
        edits: false,
        tier: Tier::Mid,
    },
    Role {
        id: "data",
        title: "data analyst",
        summary: "defines metrics and answers questions with numbers",
        duties: "You are the data analyst: define metrics, read data and code, answer questions with numbers \
and say how sure you are. You read and report; you don't edit files.",
        edits: false,
        tier: Tier::Mid,
    },
];

/// A role the person names themselves (`/add custom writer`): read-only until
/// they say otherwise, with a charter built from its name.
pub const CUSTOM: Role = Role {
    id: "custom",
    title: "team member",
    summary: "any role you name",
    duties: "Do the work your role's name implies, and ask when its scope is unclear. \
You read and report; you don't edit files.",
    edits: false,
    tier: Tier::Upper,
};

/// Look a built-in role up by id. `po` is accepted for the PM.
pub fn role(id: &str) -> Option<&'static Role> {
    let id = if id == "po" { PM } else { id };
    ROLES.iter().find(|r| r.id == id)
}

/// The role a member was given: built in, or custom under its own name.
pub fn role_or_custom(id: &str) -> &'static Role {
    role(id).unwrap_or(&CUSTOM)
}

/// The engines to try for a role, best first. Editing runs on Claude Code for
/// now; everyone else prefers Codex — which also puts QA on a different engine
/// from the engineers, so a second model checks the first one's work.
pub fn engine_preference(role: &Role) -> [Engine; 2] {
    if role.edits { [Engine::Claude, Engine::Codex] } else { [Engine::Codex, Engine::Claude] }
}

/// The model to recommend for a role on an engine. Claude Code takes its own
/// aliases; Codex picks from the account's live catalog (best first).
pub fn recommended_model(tier: Tier, engine: Engine, catalog: &[String]) -> Option<String> {
    match engine {
        Engine::Claude => Some(match tier {
            Tier::Top => "opus",
            Tier::Upper | Tier::Mid => "sonnet",
        }
        .to_owned()),
        Engine::Codex => match tier {
            Tier::Top | Tier::Upper => catalog.first().cloned(),
            // A lighter model when the plan offers one.
            Tier::Mid => catalog.iter().find(|m| m.contains("mini")).or(catalog.first()).cloned(),
        },
    }
}

/// Someone on the team, as a charter names them.
pub struct Teammate<'a> {
    pub name: &'a str,
    pub role: &'a str,
}

/// What an engine is told about who it is on the team.
pub fn charter(role: &Role, name: &str, team: &[Teammate<'_>]) -> String {
    let title = if role.id == CUSTOM.id { format!("{name} (a custom role)") } else { role.title.to_owned() };
    let mates: Vec<String> = team
        .iter()
        .filter(|m| m.name != name)
        .map(|m| if m.name == m.role { m.name.to_owned() } else { format!("{} ({})", m.name, m.role) })
        .collect();
    let mates = if mates.is_empty() { "nobody else yet".to_owned() } else { mates.join(", ") };
    let pm = team.iter().find(|m| m.role == PM).map(|m| m.name);
    let reporting = match pm {
        Some(pm) if pm != name => format!(
            "{pm} (the PM) keeps the plan. The person may also talk to you directly — follow them, and when \
what they ask goes beyond your current task, say so and propose it as a new ticket rather than quietly \
widening your work."
        ),
        _ => String::new(),
    };
    format!(
        "You are {name}, the {title} on a software squad that equipe runs for one person, who leads it. \
Your teammates: {mates}. {reporting}\n\n{}\n\nReply in the language the person writes in. Be concise.",
        role.duties
    )
    .replace(".  ", ". ")
    .replace(". \n", ".\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_lowercase_words() {
        let mut ids: Vec<_> = ROLES.iter().map(|r| r.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ROLES.len());
        assert!(ROLES.iter().all(|r| r.id.chars().all(|c| c.is_ascii_lowercase())));
    }

    #[test]
    fn finds_a_role_by_id_and_po_means_pm() {
        assert_eq!(role("qa").map(|r| r.id), Some("qa"));
        assert_eq!(role("po").map(|r| r.id), Some("pm"));
        assert_eq!(role("ceo"), None);
        assert_eq!(role_or_custom("writer").id, "custom");
    }

    #[test]
    fn the_pm_plans_and_does_not_edit() {
        let pm = role(PM).unwrap();
        assert!(!pm.edits);
        assert_eq!(pm.tier, Tier::Top);
        assert!(role("frontend").unwrap().edits);
    }

    #[test]
    fn editors_run_on_claude_and_qa_checks_on_the_other_engine() {
        assert_eq!(engine_preference(role("backend").unwrap())[0], Engine::Claude);
        assert_eq!(engine_preference(role("qa").unwrap())[0], Engine::Codex);
    }

    #[test]
    fn models_are_recommended_from_what_the_plan_offers() {
        let catalog = vec!["gpt-6".to_owned(), "gpt-6-mini".to_owned()];
        assert_eq!(recommended_model(Tier::Top, Engine::Claude, &[]).as_deref(), Some("opus"));
        assert_eq!(recommended_model(Tier::Mid, Engine::Claude, &[]).as_deref(), Some("sonnet"));
        assert_eq!(recommended_model(Tier::Top, Engine::Codex, &catalog).as_deref(), Some("gpt-6"));
        assert_eq!(recommended_model(Tier::Mid, Engine::Codex, &catalog).as_deref(), Some("gpt-6-mini"));
        assert_eq!(recommended_model(Tier::Mid, Engine::Codex, &catalog[..1]).as_deref(), Some("gpt-6"));
        assert_eq!(recommended_model(Tier::Top, Engine::Codex, &[]), None);
    }

    #[test]
    fn a_charter_names_the_member_its_team_and_who_keeps_the_plan() {
        let team = [Teammate { name: "pm", role: "pm" }, Teammate { name: "frontend", role: "frontend" }, Teammate { name: "frontend-2", role: "frontend" }];
        let text = charter(role("frontend").unwrap(), "frontend", &team);
        assert!(text.starts_with("You are frontend, the frontend engineer"), "{text}");
        assert!(text.contains("Your teammates: pm, frontend-2 (frontend)."), "{text}");
        assert!(text.contains("pm (the PM) keeps the plan"), "{text}");
        assert!(text.contains("new ticket"), "{text}");
        assert!(text.contains("language the person writes in"), "{text}");
    }

    #[test]
    fn the_pms_own_charter_does_not_point_at_itself() {
        let team = [Teammate { name: "pm", role: "pm" }];
        let text = charter(role(PM).unwrap(), "pm", &team);
        assert!(text.contains("nobody else yet"), "{text}");
        assert!(!text.contains("keeps the plan. The person may"), "{text}");
        assert!(text.contains("T-1"), "{text}");
    }

    #[test]
    fn a_custom_role_is_named_by_the_person() {
        let text = charter(&CUSTOM, "writer", &[]);
        assert!(text.starts_with("You are writer, the writer (a custom role)"), "{text}");
    }
}
