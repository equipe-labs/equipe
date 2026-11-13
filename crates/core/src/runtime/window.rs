//! The person's own window, on the daemon (`crate::window_handoff`): a
//! member's hand-off of its browser for a sign-in goes to a normal Chrome
//! window on this Mac — the profile's automated browser steps aside for it
//! and comes back after — instead of the automated browser in the panel,
//! which a "verify you are human" check doesn't let anyone through.
//!
//! Here: starting a hand-off from `show_my_browser`, carrying out what its
//! state machine says against the browser host (close the profile's browser,
//! open it again), and handing the lanes back — through the same path as a
//! person pressing Done in the panel, so the member is told the same way.

use crate::browser::HostEvent;
use crate::window_handoff::{self as handoff, Action, PanelWhy};

use super::*;

/// What the runtime keeps for the person's own windows.
pub(super) struct Windows {
    pub(super) handoffs: handoff::Handoffs,
    machine: Box<dyn handoff::Machine>,
    /// Each paused profile's other lanes that were open when its browser
    /// closed — `(squad, key, shown)` — to open again with it.
    reopen: HashMap<String, Vec<(String, String, bool)>>,
    /// What is on disk about the hand-offs, to write only what changed.
    kept: Vec<handoff::Saved>,
}

impl Windows {
    pub(super) fn new(home: &Path) -> Self {
        Self::with(Box::new(handoff::System::new(home)))
    }

    pub(super) fn with(machine: Box<dyn handoff::Machine>) -> Self {
        Self { handoffs: handoff::Handoffs::default(), machine, reopen: HashMap::new(), kept: Vec::new() }
    }
}

/// The profile a lane's browser is in — `""` the main one — and where its
/// Chrome keeps itself.
fn profile_of(home: &Path, lane: &str) -> (String, PathBuf) {
    let key = lane.split_once('/').map_or("", |(_, key)| key);
    match browser::split_profile(key).1 {
        Some(name) => (name.to_owned(), browser::profile_dir(home, name)),
        None => (String::new(), browser::host_profile(home)),
    }
}

/// The profile of a lane of the host's tabs, as they are told: `""` is the
/// person's own tabs in the main profile, `"#name"` in another's.
fn tabs_profile(lane: &str) -> String {
    if lane.is_empty() {
        return String::new();
    }
    if let Some(name) = lane.strip_prefix('#') {
        return name.to_owned();
    }
    profile_of(Path::new(""), lane).0
}

/// This Mac's name, for telling the person where the window is.
fn this_mac() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "this Mac".to_owned())
}

impl Runtime {
    /// Whether a hand-off for a sign-in goes to a window of its own: the
    /// setting says so (`equipe browser handoff window`; a Mac's default),
    /// and members' browsers are lanes of the shared host. A scratch home —
    /// a test's — keeps the panel unless it asks for the window.
    pub(super) fn window_mode(&self) -> bool {
        if !self.config.browser_host || self.config.npx.is_none() {
            return false;
        }
        let default = if self.config.home == equipe_home() { handoff::default_mode() } else { handoff::Mode::Panel };
        handoff::mode(&self.config.home, default) == handoff::Mode::Window
    }

    /// A member hands its browser to the person for a sign-in (`show_my_browser`):
    /// the browser's profile is given to a normal Chrome window at `url`, in
    /// the person's hands until they press Done. `Err`: not possible — the
    /// panel then, for that reason.
    pub(super) async fn begin_window_handoff(&mut self, call: &ToolCall, url: Option<&str>, why: &str) -> Result<(), PanelWhy> {
        let id = self.member_id(&call.squad, &call.member).ok_or(PanelWhy::NoWindow)?;
        let (squad, key) = self.browser_key(id).ok_or(PanelWhy::NoWindow)?;
        let lane = format!("{squad}/{key}");
        let (profile, dir) = profile_of(&self.config.home, &lane);
        // Where to open it, and to come back to: the address it names, else where its member is.
        let at = url
            .map(str::to_owned)
            .or_else(|| self.lane_page(&lane).map(|p| p.url).filter(|u| u.starts_with("http")))
            .or_else(|| self.last_url.get(&(squad.clone(), key.clone())).cloned())
            .unwrap_or_default();
        let chrome = self.windows.machine.chrome().is_some();
        let actions = self.windows.handoffs.begin(std::time::Instant::now(), &profile, &dir, &lane, &at, chrome);
        if let Some(why) = actions.iter().find_map(|a| if let Action::Panel { why, .. } = a { Some(*why) } else { None }) {
            return Err(why);
        }
        // In the person's hands, as in the panel: every screen is told, and what for —
        // after it is told the window is open, so a screen knows there is no page to wait for.
        self.asked_in.insert(lane.clone(), why.to_owned());
        self.held.insert(lane.clone(), Some(why.to_owned()));
        self.run_window_actions(actions).await;
        let (squad, _) = self.lane_session(&lane);
        self.publish(Event::BrowserControl { squad, lane, by: Some("person".into()), why: Some(why.to_owned()), check: None });
        Ok(())
    }

    /// The person pressed Done for `lane`: its window closes once no one is
    /// left in it. `true`: this was a hand-off of a window, nothing else to do.
    pub(super) async fn window_done(&mut self, lane: &str, take: bool) -> bool {
        if !self.windows.handoffs.busy_lane(lane) {
            return false;
        }
        if !take {
            let actions = self.windows.handoffs.done(std::time::Instant::now(), lane);
            self.run_window_actions(actions).await;
        }
        true
    }

    /// Time passing: the browser closed, the folder free, the window gone, the
    /// automated browser back at its pages.
    pub(super) async fn window_tick(&mut self) {
        let now = std::time::Instant::now();
        let mut actions = self.windows.handoffs.tick(now, &mut *self.windows.machine);
        for (profile, waited) in self.windows.handoffs.resuming(now) {
            let back = self.windows.handoffs.lanes_of(&profile).iter().all(|lane| self.lane_is_back(lane));
            if back || waited >= handoff::RESUME_WAIT {
                actions.extend(self.windows.handoffs.resumed(&profile));
            }
        }
        self.run_window_actions(actions).await;
    }

    /// The host closed a profile's automated browser, as asked.
    pub(super) fn window_suspended(&mut self, profile: &str) {
        self.windows.handoffs.suspended(std::time::Instant::now(), profile);
    }

    /// Whether a lane's browser shows a page again, after coming back.
    fn lane_is_back(&self, lane: &str) -> bool {
        matches!(self.tabs.get(lane), Some(Event::BrowserTabs { pages, .. }) if !pages.is_empty())
    }

    /// Where the person's windows are, for a screen that just came: each
    /// lane's placeholder.
    pub(super) fn window_lanes(&self, squad: &str) -> Vec<String> {
        self.windows.handoffs.window_lanes().into_iter().filter(|l| l.split_once('/').is_some_and(|(q, _)| q == squad)).collect()
    }

    /// Carry out what the hand-offs' state machine says, in order.
    pub(super) async fn run_window_actions(&mut self, actions: Vec<Action>) {
        let mut queue: std::collections::VecDeque<Action> = actions.into();
        while let Some(action) = queue.pop_front() {
            match action {
                Action::Suspend { ref profile } => self.suspend_profile(profile),
                Action::Open { .. } | Action::OpenMore { .. } | Action::CloseWindow { .. } => {
                    let now = std::time::Instant::now();
                    handoff::perform(&mut *self.windows.machine, &mut self.windows.handoffs, now, &action);
                }
                Action::Resume { profile, lanes } => self.resume_profile(&profile, &lanes),
                Action::HandBack { lanes } => {
                    for lane in lanes {
                        // The same as the person pressing Done in the panel: the
                        // member is told, with where its page is now.
                        Box::pin(self.on_host(HostEvent::Control { lane, by: None, refused: false })).await;
                    }
                }
                Action::Panel { lane, url, why } => self.window_to_panel(&lane, &url, why),
                Action::Window { lane, open } => {
                    let (squad, _) = self.lane_session(&lane);
                    self.publish(Event::BrowserWindow { squad, lane, open });
                }
            }
        }
        let saved = self.windows.handoffs.saved();
        if saved != self.windows.kept {
            handoff::keep(&self.config.home, &saved);
            self.windows.kept = saved;
        }
    }

    /// Close a profile's automated browser, keeping its sign-ins, and drop what
    /// pointed at it: the lanes' endpoints and tabs. `handoffs` hears when
    /// the host says it is closed.
    fn suspend_profile(&mut self, profile: &str) {
        let keys: Vec<(String, String)> = self.browsers.iter().filter(|(k, b)| b.is_lane() && profile_of(&self.config.home, &format!("{}/{}", k.0, k.1)).0 == profile).map(|(k, _)| k.clone()).collect();
        let mut reopen = Vec::new();
        for key in keys {
            let shown = self.browsers.get(&key).is_some_and(|b| b.is_shown());
            self.browsers.remove(&key);
            if let Some(host) = &self.host
                && let Ok(mut host) = host.lock()
            {
                host.forget_lane(&format!("{}/{}", key.0, key.1));
            }
            reopen.push((key.0, key.1, shown));
        }
        self.windows.reopen.entry(profile.to_owned()).or_default().extend(reopen);
        // Its tabs went with it.
        let gone: Vec<String> = self.tabs.keys().filter(|lane| tabs_profile(lane) == profile).cloned().collect();
        for lane in gone {
            if let Some(Event::BrowserTabs { squad, session, member, color, .. }) = self.tabs.remove(&lane) {
                self.publish(Event::BrowserTabs { squad, lane, session, member, color, pages: Vec::new(), agent_page: None });
            }
        }
        if !self.hub.op(&json!({ "op": "suspend", "profile": profile })) {
            // No host running: nothing to close.
            self.window_suspended(profile);
        }
    }

    /// Open a profile's automated browser again, on the folder the sign-in
    /// went into, and take each lane back to where it was.
    fn resume_profile(&mut self, profile: &str, lanes: &[(String, String)]) {
        self.hub.op(&json!({ "op": "resume", "profile": profile }));
        let mut again = self.windows.reopen.remove(profile).unwrap_or_default();
        for (lane, _) in lanes {
            if let Some((squad, key)) = lane.split_once('/')
                && !again.iter().any(|(q, k, _)| q == squad && k == key)
            {
                again.push((squad.to_owned(), key.to_owned(), false));
            }
        }
        for (squad, key, shown) in again {
            self.open_browser(&squad, &key, shown);
        }
        for (lane, url) in lanes {
            if !url.is_empty() {
                self.hub.op(&json!({ "op": "goto", "lane": lane, "url": url }));
            }
        }
    }

    /// A hand-off that couldn't be a window — no Chrome, one that wouldn't
    /// start, a folder that stayed locked: the browser panel, as before, with
    /// the reason said plainly.
    pub(super) fn window_to_panel(&mut self, lane: &str, url: &str, why: PanelWhy) {
        let Some((squad, key)) = lane.split_once('/') else { return };
        let member = browser::member_of(key).to_owned();
        let asked = self.asked_in.remove(lane).unwrap_or_default();
        self.held.remove(lane);
        let (_, session) = self.lane_session(lane);
        if let Some(session) = session {
            let text = self.window_fallback_note(&member, &asked, why);
            self.publish(Event::Notice { undo: None, session, text });
        }
        let url = (!url.is_empty()).then_some(url);
        if !self.ask_in_panel(squad, &member, url, &asked) {
            if let Some(url) = url {
                self.show_url.insert((squad.to_owned(), member.clone()), url.to_owned());
            }
            self.show_browser(squad, Some(member), true);
        }
    }

    /// What the person is told when a hand-off that was to be a window goes to the panel, and why.
    pub(super) fn window_fallback_note(&self, member: &str, asked: &str, why: PanelWhy) -> String {
        match why {
            PanelWhy::NoChrome => say!(self, "notice.browser_window_no_chrome", name = member, why = asked),
            other => {
                let reason = match other {
                    PanelWhy::Locked => say!(self, "notice.browser_window_reason.locked"),
                    PanelWhy::Busy => say!(self, "notice.browser_window_reason.busy"),
                    _ => say!(self, "notice.browser_window_reason.no_window"),
                };
                say!(self, "notice.browser_window_other", name = member, reason = reason, why = asked)
            }
        }
    }

    /// What the daemon says in the member's conversation when the window opened.
    pub(super) fn window_notice(&self, member: &str, why: &str) -> String {
        say!(self, "notice.browser_window", name = member, host = this_mac(), why = why)
    }

    /// Paused profiles told to a host that started again: it knows nothing of them.
    pub(super) fn suspend_again(&mut self) {
        for profile in self.windows.handoffs.paused_profiles() {
            self.hub.op(&json!({ "op": "suspend", "profile": profile }));
        }
    }

    /// Hand-offs an earlier daemon left in a window end: the window closes
    /// and the folder is free again.
    pub(super) fn recover_windows(home: &Path) {
        let found = handoff::recover(home, &handoff::System::new(home));
        for saved in found {
            crate::daemon_log!("equipe: closed the sign-in window an earlier run left open ({})", saved.dir.display());
        }
    }
}
