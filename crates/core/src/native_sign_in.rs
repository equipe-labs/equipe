//! Sites whose sign-in belongs in a real desktop app, not the automated
//! browser: Slack blocks the automated browser, so where desktop control is
//! on and the app is installed, a member's hand-over of the browser to sign in
//! there is refused and the member is sent to the app through the desktop tools.
//!
//! The mapping is data: [`DEFAULT`] plus `<equipe home>/sign-in-apps`, one
//! `host = App name` per line (`#` starts a comment); a line for a host
//! replaces the default for it, and `host =` with no app removes it.

use std::path::{Path, PathBuf};

/// The file of extra mappings, in equipe's home.
pub const EXTRA_FILE: &str = "sign-in-apps";

/// Built-in host → desktop app.
pub const DEFAULT: [(&str, &str); 1] = [("slack.com", "Slack")];

/// The mappings in force: the defaults, changed by the lines of `extra`.
pub fn mapping(extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut all: Vec<(String, String)> = DEFAULT.iter().map(|(h, a)| ((*h).to_owned(), (*a).to_owned())).collect();
    for (host, app) in extra {
        all.retain(|(h, _)| h != host);
        if !app.is_empty() {
            all.push((host.clone(), app.clone()));
        }
    }
    all
}

/// The extra lines kept in `home`, none when there is no file.
pub fn read_extra(home: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(home.join(EXTRA_FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (host, app) = l.split('#').next().unwrap_or_default().split_once('=')?;
            let host = crate::vault::site_of(host);
            (!host.is_empty()).then(|| (host, app.trim().to_owned()))
        })
        .collect()
}

/// The app that signs in to `host` (the host itself or one of its subdomains).
pub fn app_for(host: &str, mapping: &[(String, String)]) -> Option<String> {
    let host = crate::vault::site_of(host);
    mapping.iter().find(|(h, _)| host == *h || host.ends_with(&format!(".{h}"))).map(|(_, app)| app.clone())
}

/// Where Mac apps are installed: the system's, and the person's own.
pub fn app_dirs(person_home: Option<&Path>, system: bool) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if system {
        dirs.push(PathBuf::from("/Applications"));
    }
    if let Some(home) = person_home {
        dirs.push(home.join("Applications"));
    }
    dirs
}

/// Whether `app` is installed in any of `dirs`.
pub fn installed(app: &str, dirs: &[PathBuf]) -> bool {
    !app.is_empty() && !app.contains('/') && dirs.iter().any(|d| d.join(format!("{app}.app")).exists())
}

/// What a member is told when asked to sign in in the browser where an app does it.
pub fn refusal(host: &str, app: &str) -> String {
    format!(
        "{host} blocks the automated browser, and the {app} desktop app is installed here with desktop control on: use the {app} app \
through the desktop tools instead (open {app}, take a snapshot, read or write there) — it is the person's own, already signed-in app. \
The person was not asked and nothing was handed over. Only if {app} itself asks to sign in, hand it to them in the app, never the browser."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_and_its_subdomains_map_to_an_app_and_a_file_changes_the_mapping() {
        let map = mapping(&[]);
        assert_eq!(app_for("https://app.slack.com/client/T1", &map).as_deref(), Some("Slack"));
        assert_eq!(app_for("acme.slack.com", &map).as_deref(), Some("Slack"));
        assert_eq!(app_for("notslack.com", &map), None);
        let extra = vec![("slack.com".to_owned(), String::new()), ("chat.example.com".to_owned(), "Example Chat".to_owned())];
        let map = mapping(&extra);
        assert_eq!(app_for("app.slack.com", &map), None, "an empty app removes a default");
        assert_eq!(app_for("chat.example.com", &map).as_deref(), Some("Example Chat"));
    }

    #[test]
    fn the_file_is_read_as_host_equals_app() {
        let home = std::env::temp_dir().join(format!("equipe-sign-in-apps-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(EXTRA_FILE), "# apps\nhttps://Chat.Example.com/ = Example Chat # ours\nbroken line\nslack.com =\n").unwrap();
        assert_eq!(read_extra(&home), [("chat.example.com".to_owned(), "Example Chat".to_owned()), ("slack.com".to_owned(), String::new())]);
        std::fs::create_dir_all(home.join("Applications/Example Chat.app")).unwrap();
        let dirs = app_dirs(Some(&home), false);
        assert!(installed("Example Chat", &dirs) && !installed("Slack", &dirs) && !installed("../x", &dirs));
        let _ = std::fs::remove_dir_all(&home);
    }
}
