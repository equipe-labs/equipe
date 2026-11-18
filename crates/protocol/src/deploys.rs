//! What went live, as screens show it: each time a project's main branch
//! reaches its users — a production deployment (Vercel posts them to GitHub),
//! a deploy workflow that succeeded, or, for a project with neither, its
//! commits landing on main — with the person's work in it said in plain
//! words. The daemon finds them and writes the notes (`equipe_core::deploys`);
//! a screen only draws them.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{SessionId, TicketId};

/// How equipe knows the work is live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploySource {
    /// A production deployment on GitHub succeeded for the commit (Vercel,
    /// Netlify, anything that reports deployments).
    Deployment,
    /// The project's deploy workflow (GitHub Actions) succeeded for it.
    Workflow,
    /// No deploy signal: its commits landed on the main branch, which is as
    /// far as equipe can see.
    #[default]
    Main,
}

impl DeploySource {
    /// Live for the project's users, rather than only on main.
    pub fn is_live(self) -> bool {
        self != DeploySource::Main
    }
}

/// One line of a release note: something the person can see now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteItem {
    /// What changed, in the words of the ticket it came from.
    pub title: String,
    /// The tickets behind it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tickets: Vec<TicketId>,
    /// The person's request it answers, when equipe knows it: its number and
    /// its name in their words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_title: Option<String>,
    /// The conversation the request was made in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
}

/// One deploy of one project, or a few close together told as one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deploy {
    /// Unique within the squad, growing.
    pub id: u64,
    /// The project, as the board names it, and where it is.
    pub project: String,
    pub path: PathBuf,
    #[serde(default)]
    pub source: DeploySource,
    /// The environment it went to (`Production`), when one was named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// Where to see it: the site's address, or the deploy's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The commits it brought: after `from` (none: unknown) up to `to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub to: String,
    /// When it went live, and when the last deploy folded into it did (unix seconds).
    pub at: u64,
    #[serde(default)]
    pub last_at: u64,
    /// What the person can see now, one line each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<NoteItem>,
    /// How many changes nobody sees — checks, docs, tests, tidying — it also brought.
    #[serde(default)]
    pub internal: usize,
    /// Every ticket in it, seen or not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tickets: Vec<TicketId>,
    /// Its news went out (the conversations and the notification).
    #[serde(default)]
    pub told: bool,
}

impl Deploy {
    /// The site's host, for "Live now on example.com": its address without
    /// the scheme or a trailing slash.
    pub fn host(&self) -> Option<&str> {
        let url = self.url.as_deref()?;
        let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
        let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        (!host.is_empty()).then_some(host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deploy(url: Option<&str>) -> Deploy {
        Deploy {
            id: 1,
            project: "site".into(),
            path: "/c/site".into(),
            source: DeploySource::Deployment,
            environment: Some("Production".into()),
            url: url.map(str::to_owned),
            from: None,
            to: "abc".into(),
            at: 10,
            last_at: 10,
            items: Vec::new(),
            internal: 0,
            tickets: Vec::new(),
            told: false,
        }
    }

    #[test]
    fn a_deploys_host_is_its_address_without_the_scheme_or_path() {
        assert_eq!(deploy(Some("https://example.com/")).host(), Some("example.com"));
        assert_eq!(deploy(Some("https://site-git-main.vercel.app/a?b")).host(), Some("site-git-main.vercel.app"));
        assert_eq!(deploy(Some("example.com")).host(), Some("example.com"));
        assert_eq!(deploy(None).host(), None);
    }

    #[test]
    fn an_older_deploy_without_the_newer_fields_still_reads() {
        let json = r#"{"id":3,"project":"site","path":"/c/site","to":"abc","at":5}"#;
        let d: Deploy = serde_json::from_str(json).unwrap();
        assert_eq!((d.source, d.last_at, d.told, d.items.len()), (DeploySource::Main, 0, false, 0));
        assert!(!d.source.is_live());
    }
}
