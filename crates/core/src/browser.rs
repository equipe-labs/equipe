//! A browser for the squad: Playwright's MCP server, headless, signed in
//! wherever the person signed in through `equipe browser`. Members use it the
//! way a person would — open a dashboard, create a project, connect a domain,
//! find a key — on the person's own accounts.
//!
//! The person signs in (and signs up) themselves: terms, CAPTCHAs, one-time
//! codes and payment stay with them. Members see only the resulting session,
//! as Playwright storage state, each in its own in-memory browser.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The MCP server's name: its tools are `mcp__browser__…`.
pub const SERVER: &str = "browser";
/// Pinned, so nothing upstream changes what members run without a release.
pub const MCP_PACKAGE: &str = "@playwright/mcp@0.0.83";
/// What `equipe browser` opens the person's sign-in window with.
pub const PLAYWRIGHT_PACKAGE: &str = "playwright@1.64.0";

/// The person's sign-ins, as Playwright storage state (cookies and local
/// storage), shared by every squad: one person, one set of accounts.
pub fn state_file(home: &Path) -> PathBuf {
    home.join("browser").join("state.json")
}

/// Whether the person has signed in anywhere yet. Until then members have no
/// browser: there would be nothing of theirs to work in.
pub fn signed_in(home: &Path) -> bool {
    state_file(home).is_file()
}

/// One member's browser server: headless, in memory, starting from the
/// person's sign-ins; what it saves (screenshots) goes to `output`.
pub fn server(npx: &Path, home: &Path, output: &Path) -> Value {
    json!({
        "type": "stdio",
        "command": npx,
        "args": [
            "-y", MCP_PACKAGE,
            "--headless",
            "--isolated",
            "--storage-state", state_file(home),
            "--output-dir", output,
        ],
    })
}

/// `npx` on PATH: the browser needs Node.js.
pub fn find_npx() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join("npx")).find(|p| p.is_file())
}

/// The sites in a storage state the person is signed in to, by domain.
pub fn sites(state: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(state) else {
        return Vec::new();
    };
    let mut sites: Vec<String> = value["cookies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["domain"].as_str())
        .map(|d| {
            let d = d.trim_start_matches('.');
            // The registrable part is enough to recognise: vercel.com, not api.vercel.com.
            let parts: Vec<&str> = d.split('.').collect();
            parts[parts.len().saturating_sub(2)..].join(".")
        })
        .collect();
    sites.sort();
    sites.dedup();
    sites
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_are_named_by_their_domain() {
        let state = r#"{"cookies":[{"domain":".vercel.com"},{"domain":"api.vercel.com"},{"domain":"us.posthog.com"}],"origins":[]}"#;
        assert_eq!(sites(state), ["posthog.com", "vercel.com"]);
        assert!(sites("not json").is_empty());
    }

    #[test]
    fn a_member_starts_from_the_persons_sign_ins_in_a_browser_of_its_own() {
        let server = server(Path::new("/bin/npx"), Path::new("/h"), Path::new("/out"));
        let args: Vec<&str> = server["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
        assert!(args.windows(2).any(|w| w == ["--storage-state", "/h/browser/state.json"]));
        assert!(args.contains(&"--isolated"), "members never write over the person's sign-ins");
    }
}
