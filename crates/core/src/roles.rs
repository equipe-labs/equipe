//! The built-in squad roles. A project rarely needs more than these.
//!
//! A role is data, not code: an id and a mandate here, and later a charter,
//! toolset and preferred engine. Every front end — CLI, desktop — lists roles
//! from this one place.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Role {
    pub id: &'static str,
    /// One line: what this role is for.
    pub summary: &'static str,
}

pub const ROLES: [Role; 6] = [
    Role { id: "po", summary: "product owner — turns intent into specs and tickets" },
    Role { id: "designer", summary: "UX/UI — flows, wireframes, design tokens" },
    Role { id: "frontend", summary: "frontend engineer" },
    Role { id: "backend", summary: "backend engineer" },
    Role { id: "qa", summary: "quality — tests, review, acceptance" },
    Role { id: "devops", summary: "build, deploy, infra" },
];

/// Look a role up by id.
pub fn role(id: &str) -> Option<&'static Role> {
    ROLES.iter().find(|r| r.id == id)
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
    fn finds_a_role_by_id() {
        assert_eq!(role("qa").map(|r| r.id), Some("qa"));
        assert_eq!(role("ceo"), None);
    }
}
