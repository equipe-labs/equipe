//! The person's Chrome profiles, and which one a squad uses for a site.
//!
//! Profiles come from Chrome's `Local State` file — the profile list Chrome
//! shows in its own profile menu: each profile's folder name, display name
//! and the Google account it is signed in to. Nothing else in Chrome's folder
//! is read: no cookies, no passwords, no history. The account's address is
//! masked before any member sees it.
//!
//! A squad's choice of profile per site is kept in
//! `~/.equipe/desktop/chrome-profiles.json`, beside the helper's other
//! settings (only the person's to open), so the person is asked once.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::page;

/// One of the person's Chrome profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Its folder in Chrome's user data: `Default`, `Profile 1`… What
    /// `--profile-directory` takes.
    pub dir: String,
    /// The name Chrome shows for it ("Work").
    pub name: String,
    /// The Google account it is signed in to, as Chrome lists it.
    pub email: Option<String>,
    /// Whether it is the profile Chrome used last.
    pub last_used: bool,
}

impl Profile {
    /// The account, masked: `j***@example.com`.
    pub fn masked(&self) -> Option<String> {
        self.email.as_deref().map(mask)
    }

    /// One line for a member: folder, name, masked account.
    pub fn line(&self) -> String {
        let mut line = format!("{} · {:?}", self.dir, self.name);
        match self.masked() {
            Some(m) => line.push_str(&format!(" · {m}")),
            None => line.push_str(" · not signed in to Google"),
        }
        if self.last_used {
            line.push_str(" · used last");
        }
        line
    }
}

/// Where Chrome keeps `Local State` for the person whose home is `user_home`.
pub fn local_state_path(user_home: &Path) -> PathBuf {
    user_home.join("Library/Application Support/Google/Chrome/Local State")
}

/// An address with all but its first character before `@` hidden.
pub fn mask(email: &str) -> String {
    let email = email.trim();
    match email.split_once('@') {
        Some((local, domain)) => {
            let first: String = local.chars().take(1).collect();
            format!("{first}***@{domain}")
        }
        None => {
            let first: String = email.chars().take(1).collect();
            format!("{first}***")
        }
    }
}

/// The profiles listed in a `Local State` file's text: `Default` first, then
/// `Profile 1`, `Profile 2`… in number order.
pub fn parse(local_state: &str) -> Result<Vec<Profile>, String> {
    let value: Value = serde_json::from_str(local_state).map_err(|_| "Chrome's profile list couldn't be read.".to_owned())?;
    let profile = &value["profile"];
    let cache = profile["info_cache"].as_object().ok_or("Chrome lists no profiles yet: the person hasn't used Chrome on this Mac.")?;
    let last = profile["last_used"].as_str().unwrap_or("Default");
    let mut list: Vec<Profile> = cache
        .iter()
        .map(|(dir, info)| Profile {
            dir: dir.clone(),
            name: info["name"].as_str().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(dir).to_owned(),
            email: info["user_name"].as_str().map(str::trim).filter(|e| !e.is_empty()).map(str::to_owned),
            last_used: dir == last,
        })
        .collect();
    list.sort_by_key(|p| order(&p.dir));
    Ok(list)
}

fn order(dir: &str) -> (u8, u64, String) {
    if dir == "Default" {
        return (0, 0, String::new());
    }
    match dir.strip_prefix("Profile ").and_then(|n| n.trim().parse::<u64>().ok()) {
        Some(n) => (1, n, String::new()),
        None => (2, 0, dir.to_owned()),
    }
}

/// The profile a member means: by folder (`Profile 1`), by name ("Work") or
/// by masked account (`j***@example.com`), ignoring case. A name two
/// profiles share must be said by folder.
pub fn resolve<'a>(list: &'a [Profile], want: &str) -> Result<&'a Profile, String> {
    let want = want.trim();
    if let Some(p) = list.iter().find(|p| p.dir.eq_ignore_ascii_case(want)) {
        return Ok(p);
    }
    let by: Vec<&Profile> = list
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case(want) || p.masked().is_some_and(|m| m.eq_ignore_ascii_case(want)))
        .collect();
    match by.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("No Chrome profile {want:?}. The person's profiles:\n{}", lines(list))),
        _ => Err(format!("More than one Chrome profile is called {want:?}: say which by its folder.\n{}", lines(list))),
    }
}

/// Every profile, one per line.
pub fn lines(list: &[Profile]) -> String {
    list.iter().map(Profile::line).collect::<Vec<_>>().join("\n")
}

fn choices_path(home: &Path) -> PathBuf {
    crate::helper::dir(home).join("chrome-profiles.json")
}

fn load(home: &Path) -> Value {
    std::fs::read_to_string(choices_path(home)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}))
}

/// The profile folder `squad` uses for the site of `host`, if one was chosen.
pub fn remembered(home: &Path, squad: &str, host: &str) -> Option<String> {
    load(home)[squad][page::site(host)].as_str().map(str::to_owned)
}

/// Remember that `squad` uses profile `dir` for the site of `host`.
pub fn remember(home: &Path, squad: &str, host: &str, dir: &str) -> std::io::Result<()> {
    let mut all = load(home);
    if !all[squad].is_object() {
        all[squad] = json!({});
    }
    all[squad][page::site(host)] = json!(dir);
    crate::helper::ensure_dir(home)?;
    std::fs::write(choices_path(home), serde_json::to_string_pretty(&all).unwrap_or_default())
}

/// Every site → profile folder `squad` has chosen.
pub fn chosen(home: &Path, squad: &str) -> BTreeMap<String, String> {
    load(home)[squad].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect()).unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The shape of Chrome's `Local State` profile list (hand-written; other
    /// keys of the real file left out). Example accounts only.
    pub(crate) const LOCAL_STATE: &str = r#"{
        "browser": { "enabled_labs_experiments": [] },
        "profile": {
            "last_used": "Profile 2",
            "info_cache": {
                "Profile 2": { "name": "Work", "user_name": "jordan.lee@acme.example", "gaia_name": "Jordan Lee" },
                "Default": { "name": "Personal", "user_name": "jordan@example.com" },
                "Profile 10": { "name": "Testing", "user_name": "" },
                "Guest Profile": { "name": "" }
            }
        }
    }"#;

    #[test]
    fn profiles_list_in_chromes_order_with_masked_accounts() {
        let list = parse(LOCAL_STATE).unwrap();
        let dirs: Vec<&str> = list.iter().map(|p| p.dir.as_str()).collect();
        assert_eq!(dirs, ["Default", "Profile 2", "Profile 10", "Guest Profile"]);
        assert_eq!(list[1].masked().as_deref(), Some("j***@acme.example"));
        assert!(list[1].last_used && !list[0].last_used);
        assert_eq!(list[2].email, None, "an empty account is no account");
        assert_eq!(list[3].name, "Guest Profile", "no name: its folder");
        let text = lines(&list);
        assert!(!text.contains("jordan.lee") && !text.contains("jordan@"), "{text}");
        assert!(text.contains("Profile 2 · \"Work\" · j***@acme.example · used last"), "{text}");
        assert!(text.contains("not signed in to Google"));
        assert!(parse("not json").is_err());
        assert!(parse("{}").is_err());
    }

    #[test]
    fn masking_keeps_only_the_first_letter() {
        assert_eq!(mask("someone@example.com"), "s***@example.com");
        assert_eq!(mask(" x@y.z "), "x***@y.z");
        assert_eq!(mask("plain"), "p***");
    }

    #[test]
    fn a_profile_is_found_by_folder_name_or_masked_account() {
        let list = parse(LOCAL_STATE).unwrap();
        assert_eq!(resolve(&list, "profile 2").unwrap().name, "Work");
        assert_eq!(resolve(&list, "work").unwrap().dir, "Profile 2");
        assert_eq!(resolve(&list, "j***@example.com").unwrap().dir, "Default");
        let missing = resolve(&list, "jordan@example.com").unwrap_err();
        assert!(missing.contains("No Chrome profile") && !missing.contains("jordan.lee@"), "the full address never matches: {missing}");
        let mut twins = list.clone();
        twins[2].name = "Work".into();
        assert!(resolve(&twins, "Work").unwrap_err().contains("say which by its folder"));
    }

    #[test]
    fn a_squads_choice_is_kept_per_site() {
        let home = std::env::temp_dir().join(format!("eq-chrome-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(remembered(&home, "acme", "github.com"), None);
        remember(&home, "acme", "accounts.google.com", "Profile 2").unwrap();
        assert_eq!(remembered(&home, "acme", "mail.google.com").as_deref(), Some("Profile 2"), "the same site");
        assert_eq!(remembered(&home, "other", "mail.google.com"), None, "another squad chooses for itself");
        remember(&home, "acme", "github.com", "Default").unwrap();
        assert_eq!(chosen(&home, "acme").len(), 2);
        let _ = std::fs::remove_dir_all(&home);
    }
}
