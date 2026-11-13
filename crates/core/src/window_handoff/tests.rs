use super::*;
use std::cell::RefCell;

/// The Mac and Chrome, as a test says they are.
#[derive(Default)]
struct Fake {
    chrome: bool,
    /// Folders a Chrome has in use — the automated browser until it exits,
    /// then the plain window while it is up.
    locked: BTreeSet<PathBuf>,
    /// Profiles with a plain window up.
    windows: BTreeSet<String>,
    opened: Vec<(String, String)>,
    closed: Vec<(String, bool)>,
    fail_open: bool,
    /// A window that ignores a polite close.
    stubborn: BTreeSet<String>,
    strays: RefCell<Vec<PathBuf>>,
    dirs: BTreeMap<String, PathBuf>,
}

impl Machine for Fake {
    fn chrome(&self) -> Option<PathBuf> {
        self.chrome.then(|| PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"))
    }

    fn open(&mut self, profile: &str, dir: &Path, url: &str) -> std::io::Result<()> {
        if self.fail_open {
            return Err(std::io::Error::other("won't start"));
        }
        self.opened.push((profile.to_owned(), url.to_owned()));
        self.windows.insert(profile.to_owned());
        self.locked.insert(dir.to_owned());
        self.dirs.insert(profile.to_owned(), dir.to_owned());
        Ok(())
    }

    fn window_open(&mut self, profile: &str) -> bool {
        self.windows.contains(profile)
    }

    fn close(&mut self, profile: &str, force: bool) {
        self.closed.push((profile.to_owned(), force));
        if force || !self.stubborn.contains(profile) {
            self.windows.remove(profile);
            if let Some(dir) = self.dirs.get(profile) {
                self.locked.remove(dir);
            }
        }
    }

    fn locked(&self, dir: &Path) -> bool {
        self.locked.contains(dir)
    }

    fn stop_strays(&self, dir: &Path) -> usize {
        self.strays.borrow_mut().push(dir.to_owned());
        1
    }
}

/// The daemon's side of it, in a few lines: carries out each action and
/// moves the clock.
struct Sim {
    h: Handoffs,
    m: Fake,
    t: Instant,
    /// Every action, in order.
    log: Vec<Action>,
}

impl Sim {
    fn new() -> Self {
        Self { h: Handoffs::default(), m: Fake { chrome: true, ..Fake::default() }, t: Instant::now(), log: Vec::new() }
    }

    fn run(&mut self, actions: Vec<Action>) {
        for action in actions {
            perform(&mut self.m, &mut self.h, self.t, &action);
            self.log.push(action);
        }
    }

    fn begin(&mut self, profile: &str, dir: &str, lane: &str, url: &str) -> Vec<Action> {
        // The automated browser has the folder.
        self.m.locked.insert(PathBuf::from(dir));
        let actions = self.h.begin(self.t, profile, Path::new(dir), lane, url, self.m.chrome().is_some());
        self.run(actions.clone());
        actions
    }

    /// The host closed the profile's browser and its Chrome exited.
    fn host_closed(&mut self, profile: &str, dir: &str) {
        self.m.locked.remove(Path::new(dir));
        self.h.suspended(self.t, profile);
    }

    fn pass(&mut self, secs: u64) {
        for _ in 0..secs {
            self.t += Duration::from_secs(1);
            let actions = self.h.tick(self.t, &mut self.m);
            self.run(actions);
        }
    }

    fn done(&mut self, lane: &str) {
        let actions = self.h.done(self.t, lane);
        self.run(actions);
    }

    /// The automated browser is back and shows its pages: lanes are handed back.
    fn back(&mut self, profile: &str) -> Vec<Action> {
        let actions = self.h.resumed(profile);
        self.run(actions.clone());
        actions
    }

    fn saw(&self, wanted: &Action) -> bool {
        self.log.contains(wanted)
    }

    fn count(&self, pick: impl Fn(&Action) -> bool) -> usize {
        self.log.iter().filter(|a| pick(a)).count()
    }
}

const MAIN: &str = "/h/browser/chrome";
const JH: &str = "/h/browser/chromes/jh";

fn resume(profile: &str, lanes: &[(&str, &str)]) -> Action {
    Action::Resume { profile: profile.into(), lanes: lanes.iter().map(|(l, u)| (l.to_string(), u.to_string())).collect() }
}

#[test]
fn a_hand_off_closes_the_automated_browser_opens_a_plain_window_and_gives_the_browser_back_on_done() {
    let mut s = Sim::new();
    let first = s.begin("", MAIN, "acme/pm", "https://auth.openai.com/log-in");
    assert_eq!(first, [Action::Window { lane: "acme/pm".into(), open: true }, Action::Suspend { profile: "".into() }]);
    assert!(s.h.paused("") && s.h.holds("acme/pm") && s.h.busy_lane("acme/pm"));
    // The host hasn't answered; the window does not come up on top of the automated browser.
    s.pass(3);
    assert!(s.m.opened.is_empty(), "the profile is in use");
    // It closed, but its Chrome still had the folder for a moment.
    s.h.suspended(s.t, "");
    s.pass(2);
    assert!(s.m.opened.is_empty(), "locked by the automated browser: wait until it has exited");
    s.m.locked.remove(Path::new(MAIN));
    s.pass(1);
    assert_eq!(s.m.opened, [("".to_owned(), "https://auth.openai.com/log-in".to_owned())]);
    assert!(s.h.window_lanes() == ["acme/pm"]);
    // The person works in the window for a while.
    s.pass(60);
    assert!(s.m.closed.is_empty() && s.h.holds("acme/pm"));
    // Done, with the window still open: it is closed, then the browser comes back.
    s.done("acme/pm");
    assert_eq!(s.m.closed, [("".to_owned(), false)]);
    assert!(!s.m.windows.contains(""));
    s.pass(1);
    assert!(s.saw(&resume("", &[("acme/pm", "https://auth.openai.com/log-in")])));
    assert!(s.saw(&Action::Window { lane: "acme/pm".into(), open: false }));
    // Not handed back until its pages are there.
    assert!(!s.log.iter().any(|a| matches!(a, Action::HandBack { .. })));
    assert!(!s.h.paused("") && s.h.busy_lane("acme/pm"), "its browser may open again, but the hand-off isn't over");
    assert_eq!(s.back(""), [Action::HandBack { lanes: vec!["acme/pm".into()] }]);
    assert!(s.h.is_empty());
}

#[test]
fn a_window_the_person_closes_without_done_counts_as_done_after_a_grace() {
    let mut s = Sim::new();
    s.begin("", MAIN, "acme/pm", "https://example.com/login");
    s.host_closed("", MAIN);
    s.pass(1);
    assert!(s.m.windows.contains(""));
    // They close the window (and Chrome lets go of the folder).
    s.m.windows.remove("");
    s.m.locked.remove(Path::new(MAIN));
    s.pass(GRACE.as_secs());
    assert!(s.h.holds("acme/pm"), "not yet: a moment, in case it was Chrome restarting");
    assert_eq!(s.count(|a| matches!(a, Action::Resume { .. })), 0);
    s.pass(1);
    assert!(!s.h.holds("acme/pm"));
    s.pass(1);
    assert_eq!(s.count(|a| matches!(a, Action::Resume { .. })), 1);
    assert_eq!(s.back("").len(), 1);
}

#[test]
fn a_window_that_comes_back_within_the_grace_is_not_done() {
    let mut s = Sim::new();
    s.begin("", MAIN, "acme/pm", "https://example.com/login");
    s.host_closed("", MAIN);
    s.pass(1);
    s.m.windows.remove("");
    s.pass(GRACE.as_secs() - 1);
    // Chrome relaunched its window.
    s.m.windows.insert("".into());
    s.pass(GRACE.as_secs() + 5);
    assert!(s.h.holds("acme/pm"));
    assert_eq!(s.count(|a| matches!(a, Action::Resume { .. })), 0);
}

#[test]
fn done_before_the_window_ever_opened_never_opens_it() {
    let mut s = Sim::new();
    s.begin("", MAIN, "acme/pm", "https://example.com/login");
    s.done("acme/pm");
    // The host answers late; the folder is free.
    s.host_closed("", MAIN);
    s.pass(1);
    assert!(s.m.opened.is_empty());
    s.pass(1);
    assert_eq!(s.count(|a| matches!(a, Action::Resume { .. })), 1);
    assert_eq!(s.back(""), [Action::HandBack { lanes: vec!["acme/pm".into()] }]);
}

#[test]
fn a_profile_the_automated_browser_will_not_let_go_of_goes_to_the_panel_with_its_browser_back() {
    let mut s = Sim::new();
    s.begin("", MAIN, "acme/pm", "https://example.com/login");
    s.h.suspended(s.t, "");
    // Its Chrome never exits.
    s.pass(UNLOCK_WAIT.as_secs() - 1);
    assert!(s.m.opened.is_empty() && s.count(|a| matches!(a, Action::Resume { .. })) == 0);
    s.pass(1);
    assert!(s.m.opened.is_empty(), "no window on a folder in use");
    assert!(s.saw(&resume("", &[("acme/pm", "https://example.com/login")])));
    assert_eq!(s.back(""), [Action::Panel { lane: "acme/pm".into(), url: "https://example.com/login".into(), why: PanelWhy::Locked }]);
}

#[test]
fn a_host_that_never_answers_is_gone_on_without() {
    let mut s = Sim::new();
    s.begin("", MAIN, "acme/pm", "https://example.com/login");
    s.m.locked.remove(Path::new(MAIN));
    s.pass(SUSPEND_WAIT.as_secs() - 1);
    assert!(s.m.opened.is_empty());
    s.pass(2);
    assert_eq!(s.m.opened.len(), 1, "the host is gone, so is its Chrome: the window opens");
}

#[test]
fn two_members_on_two_profiles_each_have_their_own_window() {
    let mut s = Sim::new();
    s.begin("", MAIN, "acme/pm", "https://a.example/login");
    s.begin("jh", JH, "acme/qa#jh", "https://b.example/login");
    s.host_closed("", MAIN);
    s.host_closed("jh", JH);
    s.pass(1);
    assert_eq!(s.m.opened.len(), 2);
    assert!(s.m.windows.contains("") && s.m.windows.contains("jh"));
    // One is done: only its window closes, its folder is its own.
    s.done("acme/pm");
    assert!(!s.m.windows.contains("") && s.m.windows.contains("jh"));
    assert!(s.h.holds("acme/qa#jh") && !s.h.holds("acme/pm"));
    s.pass(2);
    assert_eq!(s.back("").len(), 1);
    assert!(s.h.paused("jh") && !s.h.paused(""));
    // The other closes its window by itself.
    s.m.windows.remove("jh");
    s.m.locked.remove(Path::new(JH));
    s.pass(GRACE.as_secs() + 2);
    assert_eq!(s.back("jh"), [Action::HandBack { lanes: vec!["acme/qa#jh".into()] }]);
    assert!(s.h.is_empty());
}

#[test]
fn two_members_of_one_profile_share_the_window_until_both_are_done() {
    let mut s = Sim::new();
    s.begin("", MAIN, "acme/pm", "https://a.example/login");
    // The second asks while the first's window is on its way.
    let second = s.begin("", MAIN, "acme/qa", "https://b.example/login");
    assert_eq!(second, [Action::Window { lane: "acme/qa".into(), open: true }]);
    s.host_closed("", MAIN);
    s.pass(1);
    assert_eq!(s.count(|a| matches!(a, Action::Open { .. })), 1, "one window");
    assert_eq!(s.m.opened.len(), 2, "the second's page is a tab of it");
    // A third asks with the window up.
    let third = s.begin("", MAIN, "acme/backend", "https://c.example/login");
    assert!(third.iter().any(|a| matches!(a, Action::OpenMore { url, .. } if url == "https://c.example/login")));
    s.done("acme/pm");
    s.done("acme/qa");
    assert!(s.m.windows.contains(""), "one is still in it");
    s.done("acme/backend");
    assert!(!s.m.windows.contains(""));
    s.pass(1);
    match s.back("").as_slice() {
        [Action::HandBack { lanes }] => assert_eq!(lanes.len(), 3, "every one is given its browser back: {lanes:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_window_that_ignores_being_closed_is_made_to() {
    let mut s = Sim::new();
    s.m.stubborn.insert("".into());
    s.begin("", MAIN, "acme/pm", "https://example.com/login");
    s.host_closed("", MAIN);
    s.pass(1);
    s.done("acme/pm");
    s.pass(CLOSE_WAIT.as_secs() - 1);
    assert_eq!(s.m.closed, [("".to_owned(), false)]);
    s.pass(1);
    assert_eq!(s.m.closed.last(), Some(&("".to_owned(), true)));
    s.pass(1);
    assert_eq!(s.count(|a| matches!(a, Action::Resume { .. })), 1);
}

#[test]
fn no_chrome_goes_to_the_panel_at_once_and_closes_nothing() {
    let mut s = Sim::new();
    s.m.chrome = false;
    let said = s.begin("", MAIN, "acme/pm", "https://example.com/login");
    assert_eq!(said, [Action::Panel { lane: "acme/pm".into(), url: "https://example.com/login".into(), why: PanelWhy::NoChrome }]);
    assert!(s.h.is_empty() && !s.h.paused("") && s.m.opened.is_empty());
}

#[test]
fn a_window_that_will_not_start_brings_the_browser_back_and_goes_to_the_panel() {
    let mut s = Sim::new();
    s.m.fail_open = true;
    s.begin("", MAIN, "acme/pm", "https://example.com/login");
    s.host_closed("", MAIN);
    s.pass(1);
    assert!(s.m.opened.is_empty());
    s.pass(1);
    assert_eq!(s.count(|a| matches!(a, Action::Resume { .. })), 1);
    assert_eq!(s.back(""), [Action::Panel { lane: "acme/pm".into(), url: "https://example.com/login".into(), why: PanelWhy::NoWindow }]);
}

#[test]
fn a_hand_off_asked_while_the_window_closes_or_the_browser_comes_back_goes_to_the_panel() {
    let mut s = Sim::new();
    s.begin("", MAIN, "acme/pm", "https://example.com/login");
    s.host_closed("", MAIN);
    s.pass(1);
    s.done("acme/pm");
    let busy = s.begin("", MAIN, "acme/qa", "https://b.example/login");
    assert!(matches!(busy.as_slice(), [Action::Panel { why: PanelWhy::Busy, .. }]));
    s.m.locked.remove(Path::new(MAIN));
    s.pass(2);
    let busy = s.begin("", MAIN, "acme/qa", "https://b.example/login");
    assert!(matches!(busy.as_slice(), [Action::Panel { why: PanelWhy::Busy, .. }]), "resuming");
}

#[test]
fn what_is_in_a_window_is_kept_and_a_daemon_that_comes_back_closes_it() {
    let home = std::env::temp_dir().join(format!("equipe-window-handoff-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let mut s = Sim::new();
    s.begin("", MAIN, "acme/pm", "https://example.com/login");
    // Closing the automated browser: no Chrome on the folder from us yet.
    assert!(s.h.saved().is_empty());
    s.host_closed("", MAIN);
    s.pass(1);
    let saved = s.h.saved();
    assert_eq!(saved, [Saved { profile: "".into(), dir: PathBuf::from(MAIN), lanes: vec!["acme/pm".into()] }]);
    keep(&home, &saved);
    // The daemon dies; the next one finds it, closes the window, forgets it.
    let machine = Fake::default();
    let found = recover(&home, &machine);
    assert_eq!(found, saved);
    assert_eq!(*machine.strays.borrow(), [PathBuf::from(MAIN)]);
    assert!(recover(&home, &machine).is_empty(), "once");
    // And nothing left to find when nothing is open.
    keep(&home, &[]);
    assert!(!home.join("browser").join("window-handoffs.json").exists());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_setting_is_panel_or_window_and_the_default_stands_until_it_is_set() {
    let home = std::env::temp_dir().join(format!("equipe-window-mode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(mode(&home, Mode::Window), Mode::Window);
    assert_eq!(mode(&home, Mode::Panel), Mode::Panel);
    set_mode(&home, Mode::Panel).unwrap();
    assert_eq!(mode(&home, Mode::Window), Mode::Panel);
    set_mode(&home, Mode::Window).unwrap();
    assert_eq!(mode(&home, Mode::Panel), Mode::Window);
    assert_eq!(Mode::parse(" Window\n"), Some(Mode::Window));
    assert_eq!(Mode::parse("sideways"), None);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_window_is_a_plain_launch_with_nothing_of_automation_in_it() {
    let args = chrome_args(Path::new("/h/browser/chrome"), "https://auth.openai.com/log-in");
    assert_eq!(args[0], "--user-data-dir=/h/browser/chrome");
    assert_eq!(args.last().map(String::as_str), Some("https://auth.openai.com/log-in"));
    for arg in &args {
        for automated in ["remote-debugging", "enable-automation", "disable-blink-features", "headless", "test-type", "user-agent", "no-sandbox"] {
            assert!(!arg.contains(automated), "{arg}");
        }
    }
    assert!(chrome_args(Path::new("/d"), "").iter().all(|a| a.starts_with("--")), "no page asked: none opened");
}

#[test]
fn a_chrome_is_found_where_it_is_named_and_not_where_it_isnt() {
    let home = std::env::temp_dir().join(format!("equipe-window-chrome-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("browser")).unwrap();
    let fake = home.join("chrome");
    std::fs::write(&fake, "#!/bin/sh\n").unwrap();
    std::fs::write(home.join("browser").join("plain-chrome"), format!("{}\n", fake.display())).unwrap();
    assert_eq!(chrome_path(&home), Some(fake.clone()));
    // Named, and not there: not found — never the usual path instead.
    std::fs::write(home.join("browser").join("plain-chrome"), "/nonexistent/chrome\n").unwrap();
    assert_eq!(chrome_path(&home), None);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn a_folder_is_locked_while_the_chrome_named_in_its_lock_lives() {
    let dir = std::env::temp_dir().join(format!("equipe-window-lock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(!profile_locked(&dir), "no lock");
    // Chrome's lock: a link named host-pid.
    std::os::unix::fs::symlink(format!("mac-{}", std::process::id()), dir.join("SingletonLock")).unwrap();
    assert!(profile_locked(&dir), "a live process");
    std::fs::remove_file(dir.join("SingletonLock")).unwrap();
    let mut gone = std::process::Command::new("true").spawn().unwrap();
    let pid = gone.id();
    gone.wait().unwrap();
    std::os::unix::fs::symlink(format!("mac-{pid}"), dir.join("SingletonLock")).unwrap();
    assert!(!profile_locked(&dir), "a lock a Chrome that is gone left");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn strays_are_the_main_processes_on_exactly_that_folder() {
    let ps = "\
  101 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome --user-data-dir=/h/browser/chrome --no-first-run https://x.example
  102 /Applications/Google Chrome.app/Contents/Frameworks/Helper --type=renderer --user-data-dir=/h/browser/chrome
  103 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome --user-data-dir=/h/browser/chromes/jh
  104 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome --user-data-dir=/h/browser/chrome
  105 /usr/bin/vim notes";
    assert_eq!(stray_pids(ps, Path::new("/h/browser/chrome")), [101, 104]);
    assert_eq!(stray_pids(ps, Path::new("/h/browser/chromes/jh")), [103]);
}

#[test]
fn bringing_the_window_forward_hands_chrome_no_page() {
    let mut s = Sim::new();
    s.begin("", MAIN, "acme/pm", "https://example.com/login");
    assert!(s.h.raise().is_empty(), "nothing is open yet");
    s.host_closed("", MAIN);
    s.pass(1);
    let opened = s.m.opened.len();
    let raise = s.h.raise();
    assert_eq!(raise, [Action::OpenMore { profile: "".into(), dir: PathBuf::from(MAIN), url: String::new() }]);
    s.run(raise);
    assert_eq!(s.m.opened.len(), opened + 1);
    assert_eq!(s.m.opened.last(), Some(&("".to_owned(), String::new())));
}
