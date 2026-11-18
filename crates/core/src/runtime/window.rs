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
use crate::handoff_view;
use crate::window_handoff::{self as handoff, Action, PanelWhy};
use equipe_protocol::{HandoffCookie, SignInView};

use super::*;

/// What the runtime keeps for the person's own windows.
pub(super) struct Windows {
    pub(super) handoffs: handoff::Handoffs,
    /// Sign-in views open on the person's screen, for members' hand-offs.
    pub(super) views: handoff_view::Views,
    machine: Box<dyn handoff::Machine>,
    /// Each paused profile's other lanes that were open when its browser
    /// closed — `(squad, key, shown)` — to open again with it.
    reopen: HashMap<String, Vec<(String, String, bool)>>,
    /// What is on disk about the hand-offs, to write only what changed.
    kept: Vec<handoff::Saved>,
    /// Sites whose sign-in made in a sign-in view didn't hold in the members'
    /// Chrome (the site still asked): signed in to in the members' own browser
    /// from then on ([`no_carry_file`]).
    no_carry: std::collections::BTreeSet<String>,
    /// How often the person passed each site's human check in their own window lately.
    rounds: handoff::CheckRounds,
    /// The lanes in the person's window for a human check their page stopped
    /// at (not one their member asked for): handed back as from a check.
    checks: std::collections::BTreeSet<String>,
}

/// What follows a member's page stopping at a human check, once its tabs are told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CheckFollow {
    /// Back right after the person passed it in their own window: that window again.
    Window(String),
    /// Back after that too: the member does without the site (`lane`, `site`).
    GiveUp(String, String),
}

/// Where the sites a sign-in view's sign-in doesn't carry over for are kept.
fn no_carry_file(home: &Path) -> PathBuf {
    home.join("browser").join("view-no-carry")
}

impl Windows {
    pub(super) fn new(home: &Path) -> Self {
        let mut windows = Self::with(Box::new(handoff::System::new(home)));
        windows.no_carry = std::fs::read_to_string(no_carry_file(home)).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
        windows
    }

    pub(super) fn with(machine: Box<dyn handoff::Machine>) -> Self {
        Self {
            handoffs: handoff::Handoffs::default(),
            views: handoff_view::Views::default(),
            machine,
            reopen: HashMap::new(),
            kept: Vec::new(),
            no_carry: Default::default(),
            rounds: Default::default(),
            checks: Default::default(),
        }
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

/// This Mac's name, for telling the person where the window is (or the
/// dialog that waits): the one name every such line uses
/// ([`crate::screens::machine_name`]).
pub(super) fn this_mac() -> String {
    crate::screens::machine_name().to_owned()
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
    pub(super) async fn begin_window_handoff(&mut self, call: &ToolCall, url: Option<&str>, why: &str) -> Result<bool, PanelWhy> {
        let id = self.member_id(&call.squad, &call.member).ok_or(PanelWhy::NoWindow)?;
        let (squad, key) = self.browser_key(id).ok_or(PanelWhy::NoWindow)?;
        let lane = format!("{squad}/{key}");
        // Where to open it, and to come back to: the address it names, else where its member is.
        let at = url
            .map(str::to_owned)
            .or_else(|| self.lane_page(&lane).map(|p| p.url).filter(|u| u.starts_with("http")))
            .or_else(|| self.last_url.get(&(squad.clone(), key.clone())).cloned())
            .unwrap_or_default();
        // A screen that can take the sign-in in a view of its own, wherever it is: there —
        // unless the site's sign-in holds only in the browser that made it (Google's), or
        // the page is a human check: what passes one belongs to the browser that passed it,
        // so that has to be the members' own profile, in the person's window.
        let walled = self.lane_page(&lane).and_then(|p| p.check).is_some_and(handoff::needs_real_browser);
        let carries = !walled && !handoff_view::binds_to_browser(&at) && !self.windows.no_carry.iter().any(|s| vault::serves(s, &at));
        if self.hub.has_web_view() && carries {
            self.begin_view_handoff(&squad, &key, url, why).await;
            return Ok(true);
        }
        let (profile, dir) = profile_of(&self.config.home, &lane);
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
        Ok(false)
    }

    /// A member's page stopped at a human check the person answers by clicking
    /// (Cloudflare, Turnstile, reCAPTCHA, hCaptcha) and the person reaches for
    /// it — Take over, the Chrome button — or it came back right after a round:
    /// the person gets a normal Chrome window on the members' profile at that
    /// page, the automated browser stepping aside for it, instead of the panel.
    /// The panel shows the automated browser and hands it their clicks over
    /// DevTools, which such a check is built to refuse, however human the
    /// click. Done (or the window closed) brings the automated browser back
    /// to the page and gives it back to its member. `false`: not such a check
    /// in that lane, or no window to be had — the panel then, as before.
    pub(super) async fn check_to_window(&mut self, lane: &str) -> bool {
        let Some((_, url, check)) = self.checking.get(lane).cloned() else { return false };
        if !handoff::needs_real_browser(check) || !self.window_mode() || self.windows.handoffs.busy_lane(lane) {
            return false;
        }
        let now = std::time::Instant::now();
        let (profile, dir) = profile_of(&self.config.home, lane);
        let chrome = self.windows.machine.chrome().is_some();
        let actions = self.windows.handoffs.begin(now, &profile, &dir, lane, &url, chrome);
        if let Some(why) = actions.iter().find_map(|a| if let Action::Panel { why, .. } = a { Some(*why) } else { None }) {
            crate::daemon_log!("equipe: {lane}'s human check stays in the browser panel: no normal Chrome window for it ({why:?})");
            // No Chrome to open one in: said plainly, since the panel may not get anyone through.
            if why == PanelWhy::NoChrome
                && let (_, Some(session)) = self.lane_session(lane)
            {
                let member = lane.split_once('/').map(|(_, key)| browser::member_of(key).to_owned()).unwrap_or_default();
                let text = self.window_fallback_note(&member, "", why);
                self.publish(Event::Notice { undo: None, session, text });
            }
            return false;
        }
        let site = vault::site_of(&url);
        self.windows.rounds.began(now, &site);
        self.windows.checks.insert(lane.to_owned());
        let host = this_mac();
        let why = say!(self, "notice.browser_check_window_why", site = site, host = host);
        self.held.insert(lane.to_owned(), Some(why.clone()));
        self.run_window_actions(actions).await;
        let (squad, session) = self.lane_session(lane);
        self.publish(Event::BrowserControl { squad, lane: lane.to_owned(), by: Some("person".into()), why: Some(why), check: Some(check) });
        if let Some(session) = session {
            let text = say!(self, "notice.browser_check_window", site = site, host = host);
            self.publish(Event::Notice { undo: None, session, text });
        }
        crate::daemon_log!("equipe: {lane}'s human check on {site} went to a normal Chrome window on this Mac (nothing automated attached)");
        true
    }

    /// Whether a page that just stopped at a human check goes on as usual (the
    /// card), straight to the person's window (it came back right after they
    /// passed it there), or not to the person at all (it came back after that
    /// too): from the site's rounds lately.
    pub(super) fn check_round(&mut self, url: &str, check: equipe_protocol::HumanCheck) -> handoff::CheckRound {
        if !handoff::needs_real_browser(check) {
            return handoff::CheckRound::Ask;
        }
        self.windows.rounds.step(std::time::Instant::now(), &vault::site_of(url))
    }

    /// The page a lane's automated browser came back to from the person's
    /// window, if it shows a check that refuses automated browsers again.
    fn walled_again(&self, lane: &str) -> Option<(u64, String, equipe_protocol::HumanCheck)> {
        let Some(Event::BrowserTabs { pages, agent_page, .. }) = self.tabs.get(lane) else { return None };
        let page = agent_page.and_then(|a| pages.iter().find(|p| p.id == a)).or(pages.first())?;
        page.check.filter(|c| handoff::needs_real_browser(*c)).map(|c| (page.id, page.url.clone(), c))
    }

    /// Carry out what follows a page stopping at a check, once its tabs are told.
    pub(super) async fn check_follow(&mut self, follow: CheckFollow) {
        match follow {
            CheckFollow::Window(lane) => {
                self.check_to_window(&lane).await;
            }
            CheckFollow::GiveUp(lane, site) => {
                if !self.windows.rounds.tell(&lane, &site) {
                    return;
                }
                crate::daemon_log!("equipe: {site} kept showing {lane}'s browser a human check after the person passed it twice; its member does without it");
                let (_, Some(session)) = self.lane_session(&lane) else { return };
                let member = lane.split_once('/').map(|(_, key)| browser::member_of(key).to_owned()).unwrap_or_default();
                let text = say!(self, "notice.browser_check_gave_up", site = site, name = member);
                let prompt = format!(
                    "[equipe: {site} keeps showing your browser a \"verify you are human\" check, even right after the person passed it \
twice in a normal Chrome window. The site refuses automated browsers like yours, and equipe won't hand it to the person again. Stop using \
{site} for this task: carry on without it, and say plainly in your hand-in what you couldn't do or check there because of it. Don't \
try to get past the check, and don't ask the person to do it again.]"
                );
                self.carry_on(session, text, prompt).await;
            }
        }
    }

    /// The page a hand-off is for: the address asked for, else where the member is.
    fn handoff_page(&self, squad: &str, key: &str, url: Option<&str>) -> String {
        let lane = format!("{squad}/{key}");
        url.map(str::to_owned)
            .or_else(|| self.lane_page(&lane).map(|p| p.url).filter(|u| u.starts_with("http")))
            .or_else(|| self.last_url.get(&(squad.to_owned(), key.to_owned())).cloned())
            .unwrap_or_default()
    }

    /// The person signs in in a view of their screen's panel — a plain web
    /// view, nothing driving it — and the cookies of that sign-in come back to
    /// the member's browser (`crate::handoff_view`).
    async fn begin_view_handoff(&mut self, squad: &str, key: &str, url: Option<&str>, why: &str) {
        let lane = format!("{squad}/{key}");
        let at = self.handoff_page(squad, key, url);
        let site = vault::site_of(&at);
        // The member's browser stays up: the cookies go into it.
        self.open_browser(squad, key, false);
        self.windows.views.begin(std::time::Instant::now(), &lane, &at, &site);
        self.asked_in.insert(lane.clone(), why.to_owned());
        self.held.insert(lane.clone(), Some(why.to_owned()));
        let (sq, _) = self.lane_session(&lane);
        self.publish(Event::BrowserWindow { squad: sq.clone(), lane: lane.clone(), open: true, view: Some(SignInView { url: at, site }), challenged: false });
        self.publish(Event::BrowserControl { squad: sq, lane, by: Some("person".into()), why: Some(why.to_owned()), check: None });
    }

    /// A screen's sign-in cookies (`Command::BrowserCookies`): the ones of the
    /// sign-in's own sites go into the member's browser and its page loads
    /// again. From a screen with no hand-off under way: nothing.
    pub(super) async fn view_cookies(&mut self, lane: String, hosts: Vec<String>, cookies: Vec<HandoffCookie>, failed: bool) {
        match self.windows.views.cookies(std::time::Instant::now(), &lane, &hosts, cookies, failed) {
            handoff_view::Taken::Ignored => {}
            handoff_view::Taken::Failed => self.view_fallback(&lane).await,
            handoff_view::Taken::Apply { url, scoped } => {
                // What went in is the host's to say (`HostEvent::Cookies`): Chrome may refuse some.
                crate::daemon_log!("equipe: {} sign-in cookies are for {lane}'s browser ({} of other sites left out)", scoped.cookies.len(), scoped.dropped);
                let cookies: Vec<serde_json::Value> = scoped.cookies.iter().map(super::sign_ins::host_cookie).collect();
                self.hub.op(&json!({ "op": "cookies", "lane": lane, "cookies": cookies, "url": url }));
                // Kept for the squad: no member of it is asked for this site again while the site takes it.
                if let Some((squad, key)) = lane.split_once('/') {
                    let profile = browser::split_profile(key).1.unwrap_or_default().to_owned();
                    self.keep_sign_in(vec![squad.to_owned()], &profile, &url, scoped.cookies);
                }
            }
        }
    }

    /// A view that can't go on (its screen couldn't open it, or went away): the
    /// person's own Chrome window on the daemon's Mac if there is one, else the panel.
    async fn view_fallback(&mut self, lane: &str) {
        let url = self.windows.views.take(lane).unwrap_or_default();
        self.finish_view(lane, false);
        let (profile, dir) = profile_of(&self.config.home, lane);
        let chrome = self.windows.machine.chrome().is_some();
        let actions = self.windows.handoffs.begin(std::time::Instant::now(), &profile, &dir, lane, &url, chrome);
        self.run_window_actions(actions).await;
    }

    /// A view's sign-in that the site didn't take in the member's Chrome: the
    /// member's own browser into the person's hands in the panel, at `url`.
    /// Whether it went there.
    fn view_to_panel(&mut self, lane: &str, url: &str) -> bool {
        let Some((squad, key)) = lane.split_once('/') else { return false };
        let (squad, member) = (squad.to_owned(), browser::member_of(key).to_owned());
        let site = vault::site_of(url);
        if !site.is_empty() && self.windows.no_carry.insert(site.clone()) {
            let all: Vec<&str> = self.windows.no_carry.iter().map(String::as_str).collect();
            let _ = std::fs::write(no_carry_file(&self.config.home), all.join("\n") + "\n");
        }
        // What the view handed over is no sign-in there: not kept to be put back later.
        self.forget_sign_in(&squad, browser::split_profile(key).1.unwrap_or_default(), &site, 0);
        let why = self.asked_in.get(lane).cloned().unwrap_or_default();
        if !self.ask_in_panel(&squad, &member, Some(url), &why) {
            return false;
        }
        crate::daemon_log!("equipe: the sign-in to {site} from the view didn't hold in {lane}'s browser; it is in the person's hands in the panel");
        let (_, session) = self.lane_session(lane);
        if let Some(session) = session {
            let text = say!(self, "notice.view_didnt_carry", name = member, site = site);
            self.publish(Event::Notice { undo: None, session, text });
        }
        true
    }

    /// A view is over: its screens drop it.
    fn finish_view(&mut self, lane: &str, challenged: bool) {
        let (squad, _) = self.lane_session(lane);
        self.publish(Event::BrowserWindow { squad, lane: lane.to_owned(), open: false, view: None, challenged });
    }

    /// Whether a lane's browser shows its page again after its cookies went in.
    fn lane_loaded(&self, lane: &str) -> bool {
        self.lane_page(lane).is_some_and(|p| !p.loading && p.url != "about:blank")
    }

    /// The views whose person is done and whose page is back: handed back —
    /// saying so when the page still meets a human check.
    async fn hand_back_views(&mut self) {
        let now = std::time::Instant::now();
        let loaded: std::collections::HashSet<String> = self.windows.views.lanes().into_iter().filter(|l| self.lane_loaded(l)).collect();
        for (lane, _stale) in self.windows.views.ready(now, |l| loaded.contains(l)) {
            let page = self.lane_page(&lane);
            let challenged = page.as_ref().is_some_and(|p| p.check.is_some());
            self.finish_view(&lane, challenged);
            // The view's sign-in didn't hold in the member's Chrome (the site still asks
            // for it): the browser itself goes into the person's hands, in the panel, at
            // once — the same hand-off, not a new question — and that site goes there
            // directly from now on.
            let still = page.filter(|p| matches!(p.check, Some(equipe_protocol::HumanCheck::SignIn | equipe_protocol::HumanCheck::TwoFactor)));
            if let Some(page) = still
                && self.view_to_panel(&lane, &page.url)
            {
                continue;
            }
            Box::pin(self.on_host(HostEvent::Control { lane, by: None, refused: false })).await;
        }
        // The screen that took them went: each goes where a person can still sign in.
        if !self.hub.has_web_view() && !self.windows.views.is_empty() {
            for lane in self.windows.views.lanes() {
                self.view_fallback(&lane).await;
            }
        }
    }

    /// The person pressed Done for `lane`: its window closes once no one is
    /// left in it. `true`: this was a hand-off of a window, nothing else to do.
    pub(super) async fn window_done(&mut self, lane: &str, take: bool) -> bool {
        // A sign-in view: Done after its cookies; the browser goes back once its page is loaded again.
        if self.windows.views.holds(lane) {
            if !take {
                self.windows.views.done(std::time::Instant::now(), lane);
            }
            return true;
        }
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
        self.hand_back_views().await;
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

    /// Whether any hand-off waits on the clock: a window, or a sign-in view.
    pub(super) fn window_busy(&self) -> bool {
        !self.windows.handoffs.is_empty() || !self.windows.views.is_empty()
    }

    /// Sign-in views open now in `squad`, as a screen that just came is told.
    pub(super) fn view_asks(&self, squad: &str) -> Vec<Event> {
        self.windows
            .views
            .lanes()
            .into_iter()
            .filter(|l| l.split_once('/').is_some_and(|(q, _)| q == squad))
            .filter_map(|lane| {
                let (url, site) = self.windows.views.ask_of(&lane)?;
                Some(Event::BrowserWindow { squad: Some(squad.to_owned()), lane, open: true, view: Some(SignInView { url, site }), challenged: false })
            })
            .collect()
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
                        // member is told, with where its page is now — and once
                        // that page shows the site past its sign-in, the sign-in
                        // is on the record, kept, and offered to save
                        // (`Runtime::verify_back`). Back from a check its page
                        // stopped at: its tools were turned away meanwhile, so
                        // it carries on.
                        let from_check = self.windows.checks.remove(&lane);
                        // The automated browser came back to the check: once more in the
                        // person's window, by itself, while the member still waits.
                        let again = if from_check { self.walled_again(&lane) } else { None };
                        if let Some((page, url, check)) = again.clone()
                            && self.check_round(&url, check) == handoff::CheckRound::Again
                        {
                            self.checking.insert(lane.clone(), (page, url, check));
                            if Box::pin(self.check_to_window(&lane)).await {
                                continue;
                            }
                        }
                        Box::pin(self.on_host(HostEvent::Control { lane: lane.clone(), by: None, refused: from_check })).await;
                        // …and after that, no more rounds: the member does without the site.
                        if let Some((page, url, check)) = again
                            && self.check_round(&url, check) == handoff::CheckRound::GiveUp
                        {
                            let site = vault::site_of(&url);
                            self.let_go.insert((lane.clone(), page, url));
                            Box::pin(self.check_follow(CheckFollow::GiveUp(lane, site))).await;
                        }
                    }
                }
                Action::Panel { lane, url, why } => self.window_to_panel(&lane, &url, why),
                Action::Window { lane, open } => {
                    let (squad, _) = self.lane_session(&lane);
                    self.publish(Event::BrowserWindow { squad, lane, open, view: None, challenged: false });
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
        self.host_pages.retain(|lane, _| tabs_profile(lane) != profile);
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
        // The profile its Chrome opens again: where the person was, should their sign-in have moved to another.
        let dir = if profile.is_empty() { browser::host_profile(&self.config.home) } else { browser::profile_dir(&self.config.home, profile) };
        browser::chrome_profile(&dir);
        self.tell_profile_separation();
        self.hub.op(&json!({ "op": "resume", "profile": profile }));
        // Its Chrome starts again: the kept sign-ins go in again as its lanes open.
        self.sign_ins_restart(Some(profile));
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

    /// The person's Google Workspace keeps each work account in a Chrome
    /// profile of its own (enterprise profile separation): told once, plainly,
    /// which account members work as, how to choose another, and the setting
    /// their Workspace admin can change so Chrome stops making a profile per
    /// account — optional: equipe works either way.
    pub(super) fn tell_profile_separation(&mut self) {
        let home = self.config.home.clone();
        let dir = browser::host_profile(&home);
        let told = home.join("browser").join("told-profile-separation");
        if told.exists() || !browser::separated(&dir) {
            return;
        }
        let _ = std::fs::write(&told, "");
        let pinned = browser::pinned_profile(&dir);
        let account = browser::chrome_profiles(&dir).into_iter().find(|p| p.dir == pinned).and_then(|p| p.account).unwrap_or_else(|| pinned.clone());
        crate::daemon_log!("equipe: Chrome keeps each work account in a profile of its own in {}; members use {pinned:?}", dir.display());
        let text = say!(self, "notice.profile_separation", account = account);
        let line = format!(
            "equipe: the person's Google Workspace requires a separate Chrome profile for each work account, so equipe's browser has one per \
account signed in to; members work as {account}. The person was told how to choose another account and what their admin can change. \
Nothing for you to do; if a page needs another Google account, say so — don't ask the person to sign in again."
        );
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            self.brief_pm(&squad, line.clone(), Some(text.clone()));
        }
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
