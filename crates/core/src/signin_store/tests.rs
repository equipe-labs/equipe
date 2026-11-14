use super::*;

const NOW: u64 = 1_800_000_000;
const DAY: u64 = 24 * 60 * 60;

fn cookie(name: &str, domain: &str, expires: Option<i64>) -> HandoffCookie {
    HandoffCookie { name: name.into(), value: format!("{name}-s3cret"), domain: domain.into(), path: "/".into(), expires, secure: true, http_only: true, same_site: Some("Lax".into()) }
}

fn names(cookies: &[HandoffCookie]) -> Vec<&str> {
    let mut n: Vec<&str> = cookies.iter().map(|c| c.name.as_str()).collect();
    n.sort_unstable();
    n
}

#[derive(Default)]
struct MemKeys(std::collections::HashMap<String, String>);

impl Keys for MemKeys {
    fn get(&self, account: &str) -> Result<String, String> {
        self.0.get(account).cloned().ok_or_else(|| "nothing kept".into())
    }

    fn set(&mut self, account: &str, value: &str) -> Result<(), String> {
        self.0.insert(account.to_owned(), value.to_owned());
        Ok(())
    }
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("equipe-sign-ins-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_sealed_record_opens_only_with_its_key_and_shows_no_value() {
    let mut record = Record::default();
    record.remember("", "atinum.example", vec![cookie("session", ".atinum.example", None)], NOW);
    let key = new_key().unwrap();
    let sealed = seal(&key, &record).unwrap();
    assert!(!String::from_utf8_lossy(&sealed).contains("s3cret") && !String::from_utf8_lossy(&sealed).contains("atinum"), "nothing readable on disk");
    assert_eq!(open(&key, &sealed).unwrap(), record);
    assert!(open(&new_key().unwrap(), &sealed).is_err(), "another key");
    let mut changed = sealed.clone();
    *changed.last_mut().unwrap() ^= 1;
    assert!(open(&key, &changed).is_err(), "a changed byte");
    assert!(open(&key, b"{\"profiles\":{}}").is_err(), "not sealed");
    assert!(!format!("{record:?}").contains("s3cret"), "never printed");
}

#[test]
fn session_cookies_are_kept_and_expired_ones_are_not() {
    let mut record = Record::default();
    let later = i64::try_from(NOW + 60 * DAY).unwrap();
    let gone = i64::try_from(NOW - 1).unwrap();
    record.remember("", "app.example", vec![cookie("session", "app.example", None), cookie("remember", ".app.example", Some(later)), cookie("old", ".app.example", Some(gone))], NOW);
    assert_eq!(names(&record.live("", NOW + 60)), ["remember", "session"], "a session cookie lasts past a restart; one already expired never went in");
    assert_eq!(names(&record.live("", NOW + SESSION_KEEP + 1)), ["remember"], "the session cookie, after a month, is the site's to renew");
    assert!(record.live("", NOW + 61 * DAY).is_empty());
    record.prune(NOW + 61 * DAY);
    assert!(record.is_empty(), "nothing left that counts: nothing kept");
}

#[test]
fn profiles_stay_apart_and_the_newest_sign_in_wins_a_shared_cookie() {
    let mut record = Record::default();
    record.remember("", "chatgpt.com", vec![cookie("SID", ".google.com", None), cookie("chat", "chatgpt.com", None)], NOW);
    let mut newer = cookie("SID", ".google.com", None);
    newer.value = "newer".into();
    record.remember("", "app.example", vec![newer, cookie("app", "app.example", None)], NOW + 10);
    record.remember("jh", "app.example", vec![cookie("theirs", "app.example", None)], NOW);
    let main = record.live("", NOW + 20);
    assert_eq!(names(&main), ["SID", "app", "chat"]);
    assert_eq!(main.iter().find(|c| c.name == "SID").unwrap().value, "newer");
    assert_eq!(names(&record.live("jh", NOW + 20)), ["theirs"], "another person's profile: their accounts only");
    assert!(record.live("nobody", NOW).is_empty());
}

#[test]
fn a_sign_in_begun_at_google_is_found_for_the_app_it_went_on_to() {
    let mut record = Record::default();
    // The view opened at Google's sign-in and went on to the app: kept under Google's host.
    record.remember("", "https://accounts.google.com/v3/signin", vec![cookie("__Host-GAPS", ".accounts.google.com", None), cookie("SID", ".google.com", None), cookie("app_session", ".atinum.example", None)], NOW);
    let (made, cookies) = record.for_site("", "atinum.example", NOW + 5).expect("found for the app");
    assert_eq!(made, NOW);
    assert_eq!(names(&cookies), ["SID", "__Host-GAPS", "app_session"]);
    let gaps = cookies.iter().find(|c| c.name == "__Host-GAPS").unwrap();
    assert_eq!(gaps.domain, "accounts.google.com", "kept as Chrome takes it");
    assert!(record.for_site("", "other.example", NOW).is_none());
    assert!(record.for_site("jh", "atinum.example", NOW).is_none(), "not in another profile");
}

#[test]
fn a_site_that_ended_its_sign_in_loses_it_and_others_that_went_through_it_stay() {
    let mut record = Record::default();
    record.remember("", "app.example", vec![cookie("SID", ".google.com", None), cookie("app", "app.example", None)], NOW);
    record.remember("", "accounts.google.com", vec![cookie("SID", ".google.com", None)], NOW + 5);
    assert!(record.forget("", "google.com", NOW + 5));
    assert_eq!(record.sites(""), ["app.example"], "the app's sign-in that went through Google stays");
    assert!(record.forget("", "app.example", NOW));
    assert!(record.is_empty());
    assert!(!record.forget("", "app.example", NOW), "nothing left to forget");
}

#[test]
fn the_same_sign_in_told_twice_is_one_and_a_later_one_replaces_it() {
    let mut record = Record::default();
    record.remember("", "app.example", vec![cookie("SID", ".google.com", None), cookie("app", "app.example", None)], NOW);
    // The site's own cookies read back from the browser a minute later: merged.
    record.remember("", "app.example", vec![cookie("app", "app.example", None), cookie("csrf", "app.example", None)], NOW + 60);
    assert_eq!(names(&record.live("", NOW + 61)), ["SID", "app", "csrf"]);
    // A sign-in days later: what it holds, alone.
    record.remember("", "app.example", vec![cookie("app", "app.example", None)], NOW + 3 * DAY);
    assert_eq!(names(&record.live("", NOW + 3 * DAY)), ["app"]);
}

#[test]
fn kept_on_disk_a_restart_finds_them_session_cookies_and_all() {
    let home = home("disk");
    let mut keys = MemKeys::default();
    assert!(load(&home, "acme", &keys).unwrap().is_empty(), "nothing kept yet");
    update(&home, "acme", &mut keys, NOW, |r| r.remember("", "app.example", vec![cookie("session", "app.example", None)], NOW)).unwrap();
    assert!(keys.0.contains_key(&key_account("acme")), "the key is in the secret store");
    let file = path(&home, "acme");
    let bytes = std::fs::read(&file).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("s3cret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600, "the person's alone");
    }
    // The daemon starts again: read anew from disk.
    let again = load(&home, "acme", &keys).unwrap();
    assert_eq!(names(&again.live("", NOW + 3600)), ["session"]);
    // Another squad's: its own file, its own key.
    assert!(load(&home, "beta", &keys).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_file_whose_key_is_gone_is_left_as_it_is() {
    let home = home("nokey");
    let mut keys = MemKeys::default();
    update(&home, "acme", &mut keys, NOW, |r| r.remember("", "app.example", vec![cookie("session", "app.example", None)], NOW)).unwrap();
    let before = std::fs::read(path(&home, "acme")).unwrap();
    let mut other = MemKeys::default();
    assert!(load(&home, "acme", &other).is_err());
    assert!(update(&home, "acme", &mut other, NOW, |r| r.remember("", "b.example", vec![cookie("b", "b.example", None)], NOW)).is_err());
    assert_eq!(std::fs::read(path(&home, "acme")).unwrap(), before, "never written over with a new key");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn cleaning_members_scratch_folders_never_takes_the_kept_sign_ins() {
    let home = home("clean");
    let mut keys = MemKeys::default();
    update(&home, "acme", &mut keys, NOW, |r| r.remember("", "app.example", vec![cookie("session", "app.example", None)], NOW)).unwrap();
    for name in ["t7", "qa-2", "designer"] {
        let dir = home.join("tmp").join("acme").join(name).join("playwright_chromiumdev_profile-x");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cookies"), b"x").unwrap();
    }
    let done = crate::disk::clear_ticket_tmp(&home, "acme", 7, &[]);
    assert!(done.iter().any(|l| l.starts_with("removed ")), "{done:?}");
    crate::disk::clear_lane_tmp(&home, &home.join("tmp").join("acme").join("qa-2"), &[]);
    crate::disk::tidy_lane_tmp(&home, &[("acme".to_owned(), Vec::new())], &[]);
    // Even a folder handed to the tmp cleanup outside `tmp` is refused.
    crate::disk::clear_lane_tmp(&home, &crate::journal::squad_dir(&home, "acme"), &[]);
    assert!(!home.join("tmp").join("acme").join("qa-2").exists(), "the member's scratch folder went");
    assert!(path(&home, "acme").is_file(), "the kept sign-ins stay");
    assert_eq!(names(&load(&home, "acme", &keys).unwrap().live("", NOW)), ["session"]);
    let _ = std::fs::remove_dir_all(&home);
}
