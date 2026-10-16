//! Which machine's daemon the window is a window onto: this Mac (the socket)
//! or an SSH host. What the person typed or picked is remembered, so the next
//! launch connects to the same one.
//!
//! Nothing here assumes a host, a user, a path or a shell: the names come from
//! the person's own `~/.ssh/config` or from what they type (`user@host[:port]`).

use equipe_core::t;
use std::path::{Path, PathBuf};

/// How many hosts typed by hand are kept to pick again.
const RECENT: usize = 8;

/// An SSH destination, as typed: a name `ssh` resolves (an alias from
/// `~/.ssh/config`, a DNS name, an address), maybe with a user and a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshHost {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
}

impl SshHost {
    /// `host`, `user@host`, `host:port`, `user@host:port`, `[::1]:2222`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err(t!("target.host_empty").into());
        }
        let (user, rest) = match text.rsplit_once('@') {
            Some((user, rest)) => (Some(user), rest),
            None => (None, text),
        };
        if user.is_some_and(str::is_empty) {
            return Err(t!("target.user_empty").into());
        }
        let (host, port) = split_port(rest)?;
        let valid_name = |s: &str| !s.is_empty() && !s.starts_with('-') && !s.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '\'' | '"' | '\\' | '`' | '$' | ';' | '&' | '|' | '<' | '>' | '(' | ')' | '*' | '?' | '!'));
        if !valid_name(&host) || !user.is_none_or(valid_name) {
            return Err(t!("target.bad_chars").into());
        }
        Ok(Self { user: user.map(str::to_owned), host, port })
    }

    /// The text that parses back to this host.
    pub fn spec(&self) -> String {
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        let user = self.user.as_ref().map(|u| format!("{u}@")).unwrap_or_default();
        let port = self.port.map(|p| format!(":{p}")).unwrap_or_default();
        format!("{user}{host}{port}")
    }

    /// The arguments that name this host for `ssh`: `-p` when there is a port,
    /// then `--` so a name can never be read as an option, then the destination.
    pub fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(port) = self.port {
            args.push("-p".to_owned());
            args.push(port.to_string());
        }
        args.push("--".to_owned());
        args.push(match &self.user {
            Some(user) => format!("{user}@{}", self.host),
            None => self.host.clone(),
        });
        args
    }
}

fn split_port(rest: &str) -> Result<(String, Option<u16>), String> {
    let bad_port = || t!("target.bad_port").to_owned();
    if let Some(inner) = rest.strip_prefix('[') {
        let (host, after) = inner.split_once(']').ok_or(t!("target.no_closing_bracket"))?;
        return match after {
            "" => Ok((host.to_owned(), None)),
            _ => {
                let port = after.strip_prefix(':').ok_or_else(bad_port)?;
                Ok((host.to_owned(), Some(port.parse().ok().filter(|p| *p > 0).ok_or_else(bad_port)?)))
            }
        };
    }
    // A bare IPv6 address has several colons and no port.
    if rest.matches(':').count() > 1 {
        return Ok((rest.to_owned(), None));
    }
    match rest.split_once(':') {
        Some((host, port)) => Ok((host.to_owned(), Some(port.parse().ok().filter(|p| *p > 0).ok_or_else(bad_port)?))),
        None => Ok((rest.to_owned(), None)),
    }
}

/// Where the window's daemon is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// The daemon on this Mac, through its socket file.
    Local { socket: PathBuf },
    /// A daemon on another machine, through `ssh <host> equipe attach --stdio`.
    Ssh(SshHost),
}

impl Endpoint {
    pub fn this_mac() -> Self {
        let home = equipe_providers::credentials::equipe_home();
        Self::Local { socket: equipe_core::gateway::socket_path(&home) }
    }

    /// What the window calls it.
    pub fn label(&self) -> String {
        match self {
            Self::Local { .. } => t!("target.this_mac").to_owned(),
            Self::Ssh(host) => host.spec(),
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Ssh(_))
    }

    fn saved(&self) -> String {
        match self {
            Self::Local { .. } => "local".to_owned(),
            Self::Ssh(host) => format!("ssh {}", host.spec()),
        }
    }

    fn from_saved(text: &str) -> Option<Self> {
        match text.trim() {
            "local" => Some(Self::this_mac()),
            other => other.strip_prefix("ssh ").and_then(|spec| SshHost::parse(spec).ok()).map(Self::Ssh),
        }
    }

    /// `local` / `this` / `이 맥`, or an SSH destination.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim() {
            "local" | "this" | "이 맥" => Ok(Self::this_mac()),
            other if other == t!("target.this_mac") => Ok(Self::this_mac()),
            other => SshHost::parse(other).map(Self::Ssh),
        }
    }
}

/// What the app remembers about where it connected: the last target, and the
/// hosts typed by hand to pick again. A small text file next to the app's other
/// state (`<home>/desktop-target`): `last <target>` and `recent ssh <spec>` lines.
/// A view of the work screens the person kept (⌥V): its name, filter words,
/// grouping (by its code) and whether the sidebar shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeptView {
    pub name: String,
    pub query: String,
    pub group: String,
    pub pinned: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Remembered {
    pub last: Option<Endpoint>,
    pub recent: Vec<SshHost>,
    /// How tall the board's list was left, in px (the divider the person dragged).
    pub board_split: Option<f32>,
    /// How wide the browser panel was left, in px.
    pub browser_width: Option<f32>,
    /// What accepting lands, per project (by name), once the person said.
    pub merge_ways: Vec<(String, String)>,
    /// The views the person saved (⌥V), in their order.
    pub views: Vec<KeptView>,
    /// How wide the sidebar was left (px).
    pub sidebar_width: Option<f32>,
    /// The language this app kept before the pick was everyone's (`ko`, `en`, …):
    /// read once, moved to the shared pick (`i18n::migrate`), never written again.
    pub lang: Option<String>,
    /// Light or dark, or as the Mac is.
    pub appearance: Appearance,
    /// What the window opens on.
    pub start: Start,
    /// What the person wants to be told of while the window is out of sight.
    pub pings: Pings,
    /// The person said "나중에" when asked to let equipe notify: not asked
    /// again on its own (settings still offers it).
    pub notify_later: bool,
    /// The PM's conversation shows whole whatever project is in focus — as it
    /// did before it followed the project (settings ▸ 일반).
    pub whole_conversation: bool,
    /// The sidebar's projects folded to their header, by name (the squad's
    /// shared folder: ""): `fold\t<name>` lines.
    pub folded: std::collections::BTreeSet<String>,
    /// 티켓 as a board rather than rows, as last left.
    pub tickets_board: bool,
    /// How large the window draws, in percent (⌘+ / ⌘-, settings ▸ 일반).
    pub zoom: Zoom,
}

/// The window's size in percent (`zoom.rs`): actual size unless the person
/// zoomed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zoom(pub u16);

impl Zoom {
    /// The sizes, in percent, as browsers offer them.
    pub const LEVELS: [u16; 10] = [67, 75, 80, 90, 100, 110, 125, 150, 175, 200];
    /// Actual size.
    pub const ACTUAL: Zoom = Zoom(100);

    /// `pct` as one of the sizes: the nearest (a hand-edited file, an old size).
    pub fn nearest(pct: u16) -> Self {
        Zoom(Self::LEVELS.iter().copied().min_by_key(|&l| l.abs_diff(pct)).unwrap_or(Self::ACTUAL.0))
    }
}

impl Default for Zoom {
    fn default() -> Self {
        Self::ACTUAL
    }
}

/// Light or dark, as the person picked in settings; the Mac's by default
/// (`theme.rs` draws it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    pub fn code(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.code() == code)
    }
}

/// What the person wants to be told of while the window is out of sight
/// (settings → 알림): each kind on or off, whether a notice makes a sound,
/// whether the Dock's icon shows what waits. Everything but merges, by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pings {
    pub approvals: bool,
    pub reviews: bool,
    pub waiting: bool,
    pub failed: bool,
    /// The answer to what the person last wrote.
    pub replies: bool,
    /// Accepted work landing on main.
    pub merged: bool,
    /// A new version of the app, downloaded.
    pub update: bool,
    pub sound: bool,
    /// The number on the Dock's icon: 나에게's.
    pub badge: bool,
}

impl Default for Pings {
    fn default() -> Self {
        let mut pings = Self { approvals: false, reviews: false, waiting: false, failed: false, replies: false, merged: false, update: false, sound: false, badge: false };
        for ping in Ping::ALL {
            pings.set(ping, ping.default_on());
        }
        pings
    }
}

/// The kinds of notice, as `desktop-target` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ping {
    Approvals,
    Reviews,
    Waiting,
    Failed,
    Replies,
    Merged,
    Update,
    Sound,
    Badge,
}

impl Ping {
    pub const ALL: [Ping; 9] = [Ping::Approvals, Ping::Reviews, Ping::Waiting, Ping::Failed, Ping::Replies, Ping::Merged, Ping::Update, Ping::Sound, Ping::Badge];

    pub fn code(self) -> &'static str {
        match self {
            Ping::Approvals => "approvals",
            Ping::Reviews => "reviews",
            Ping::Waiting => "waiting",
            Ping::Failed => "failed",
            Ping::Replies => "replies",
            Ping::Merged => "merged",
            Ping::Update => "update",
            Ping::Sound => "sound",
            Ping::Badge => "badge",
        }
    }

    /// On unless the person turns it off — all but merges, which they turn on.
    pub fn default_on(self) -> bool {
        self != Ping::Merged
    }
}

impl Pings {
    pub fn get(&self, ping: Ping) -> bool {
        match ping {
            Ping::Approvals => self.approvals,
            Ping::Reviews => self.reviews,
            Ping::Waiting => self.waiting,
            Ping::Failed => self.failed,
            Ping::Replies => self.replies,
            Ping::Merged => self.merged,
            Ping::Update => self.update,
            Ping::Sound => self.sound,
            Ping::Badge => self.badge,
        }
    }

    pub fn set(&mut self, ping: Ping, on: bool) {
        match ping {
            Ping::Approvals => self.approvals = on,
            Ping::Reviews => self.reviews = on,
            Ping::Waiting => self.waiting = on,
            Ping::Failed => self.failed = on,
            Ping::Replies => self.replies = on,
            Ping::Merged => self.merged = on,
            Ping::Update => self.update = on,
            Ping::Sound => self.sound = on,
            Ping::Badge => self.badge = on,
        }
    }
}

/// What the window opens on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Start {
    /// The PM's conversation.
    #[default]
    Pm,
    /// 나에게: what waits for the person.
    Inbox,
    /// 모든 일.
    All,
}

impl Start {
    pub const ALL: [Start; 3] = [Start::Pm, Start::Inbox, Start::All];

    pub fn code(self) -> &'static str {
        match self {
            Start::Pm => "pm",
            Start::Inbox => "inbox",
            Start::All => "all",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.code() == code)
    }
}

impl Remembered {
    pub fn path(home: &Path) -> PathBuf {
        home.join("desktop-target")
    }

    pub fn load(home: &Path) -> Self {
        std::fs::read_to_string(Self::path(home)).map(|text| Self::parse(&text)).unwrap_or_default()
    }

    fn parse(text: &str) -> Self {
        let mut out = Self::default();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("last ") {
                out.last = Endpoint::from_saved(rest);
            } else if let Some(rest) = line.strip_prefix("merge ") {
                if let Some((project, way)) = rest.rsplit_once(' ')
                {
                    out.merge_ways.retain(|(p, _)| p != project);
                    out.merge_ways.push((project.to_owned(), way.to_owned()));
                }
            } else if let Some(rest) = line.strip_prefix("view ") {
                let mut parts = rest.splitn(4, '\t');
                if let (Some(pinned), Some(group), Some(name), Some(query)) = (parts.next(), parts.next(), parts.next(), parts.next()) {
                    out.views.push(KeptView { name: name.to_owned(), query: query.to_owned(), group: group.to_owned(), pinned: pinned == "1" });
                }
            } else if let Some(rest) = line.strip_prefix("sidebar-width ") {
                out.sidebar_width = rest.trim().parse::<f32>().ok().filter(|w| w.is_finite() && *w > 0.);
            } else if let Some(rest) = line.strip_prefix("lang ") {
                out.lang = Some(rest.trim().to_owned()).filter(|l| !l.is_empty());
            } else if let Some(rest) = line.strip_prefix("appearance ") {
                out.appearance = Appearance::parse(rest.trim()).unwrap_or_default();
            } else if let Some(rest) = line.strip_prefix("start ") {
                out.start = Start::parse(rest.trim()).unwrap_or_default();
            } else if let Some(rest) = line.strip_prefix("zoom ") {
                out.zoom = rest.trim().parse::<u16>().map(Zoom::nearest).unwrap_or_default();
            } else if line.trim() == "conversation whole" {
                out.whole_conversation = true;
            } else if let Some(name) = line.strip_prefix("fold\t") {
                out.folded.insert(name.to_owned());
            } else if line.trim() == "tickets board" {
                out.tickets_board = true;
            } else if let Some((on, rest)) = line.strip_prefix("quiet ").map(|r| (false, r)).or_else(|| line.strip_prefix("ping ").map(|r| (true, r))) {
                if let Some(ping) = Ping::ALL.into_iter().find(|p| p.code() == rest.trim()) {
                    out.pings.set(ping, on);
                }
            } else if line.trim() == "notify later" {
                out.notify_later = true;
            } else if let Some(rest) = line.strip_prefix("board-split ") {
                out.board_split = rest.trim().parse::<f32>().ok().filter(|h| h.is_finite() && *h > 0.);
            } else if let Some(rest) = line.strip_prefix("browser-width ") {
                out.browser_width = rest.trim().parse::<f32>().ok().filter(|w| w.is_finite() && *w > 0.);
            } else if let Some(rest) = line.strip_prefix("recent ssh ")
                && let Ok(host) = SshHost::parse(rest)
                && !out.recent.contains(&host)
            {
                out.recent.push(host);
            }
        }
        out.recent.truncate(RECENT);
        out
    }

    fn render(&self) -> String {
        let mut text = String::new();
        if let Some(last) = &self.last {
            text.push_str(&format!("last {}\n", last.saved()));
        }
        for host in &self.recent {
            text.push_str(&format!("recent ssh {}\n", host.spec()));
        }
        if let Some(split) = self.board_split {
            text.push_str(&format!("board-split {split:.0}\n"));
        }
        if let Some(width) = self.browser_width {
            text.push_str(&format!("browser-width {width:.0}\n"));
        }
        if let Some(width) = self.sidebar_width {
            text.push_str(&format!("sidebar-width {width:.0}\n"));
        }
        if let Some(lang) = &self.lang {
            text.push_str(&format!("lang {lang}\n"));
        }
        if self.appearance != Appearance::System {
            text.push_str(&format!("appearance {}\n", self.appearance.code()));
        }
        if self.start != Start::Pm {
            text.push_str(&format!("start {}\n", self.start.code()));
        }
        if self.zoom != Zoom::default() {
            text.push_str(&format!("zoom {}\n", self.zoom.0));
        }
        if self.whole_conversation {
            text.push_str("conversation whole\n");
        }
        for name in &self.folded {
            text.push_str(&format!("fold\t{}\n", name.replace(['\t', '\n'], " ")));
        }
        if self.tickets_board {
            text.push_str("tickets board\n");
        }
        // Only what differs from the default: off as `quiet`, on as `ping`.
        for ping in Ping::ALL.into_iter().filter(|p| self.pings.get(*p) != p.default_on()) {
            text.push_str(&format!("{} {}\n", if self.pings.get(ping) { "ping" } else { "quiet" }, ping.code()));
        }
        if self.notify_later {
            text.push_str("notify later\n");
        }
        for (project, way) in &self.merge_ways {
            text.push_str(&format!("merge {project} {way}\n"));
        }
        for view in &self.views {
            let clean = |s: &str| s.replace(['\t', '\n'], " ");
            text.push_str(&format!("view {}\t{}\t{}\t{}\n", u8::from(view.pinned), clean(&view.group), clean(&view.name), clean(&view.query)));
        }
        text
    }

    /// What accepting lands in `project`, if the person said.
    pub fn merge_way(&self, project: &str) -> Option<&str> {
        self.merge_ways.iter().find(|(p, _)| p == project).map(|(_, w)| w.as_str())
    }

    pub fn set_merge_way(&mut self, project: &str, way: &str) {
        self.merge_ways.retain(|(p, _)| p != project);
        self.merge_ways.push((project.to_owned(), way.to_owned()));
    }

    /// Ask again each time in `project`.
    pub fn clear_merge_way(&mut self, project: &str) {
        self.merge_ways.retain(|(p, _)| p != project);
    }

    /// Remember that `endpoint` is the one in use now.
    pub fn use_endpoint(&mut self, endpoint: &Endpoint) {
        self.last = Some(endpoint.clone());
        if let Endpoint::Ssh(host) = endpoint {
            self.recent.retain(|h| h != host);
            self.recent.insert(0, host.clone());
            self.recent.truncate(RECENT);
        }
    }

    pub fn save(&self, home: &Path) {
        let _ = std::fs::create_dir_all(home);
        let _ = std::fs::write(Self::path(home), self.render());
    }
}

/// The hosts `~/.ssh/config` names (and the files it includes), in file order,
/// without wildcard patterns and negations — the ones a person could pick.
pub fn config_hosts(ssh_dir: &Path) -> Vec<String> {
    let mut hosts = Vec::new();
    read_config(&ssh_dir.join("config"), ssh_dir, 0, &mut hosts);
    let mut seen = std::collections::HashSet::new();
    hosts.retain(|h| seen.insert(h.clone()));
    hosts
}

fn read_config(file: &Path, ssh_dir: &Path, depth: usize, hosts: &mut Vec<String>) {
    let Ok(text) = std::fs::read_to_string(file) else { return };
    for line in text.lines() {
        let line = line.trim();
        let (key, rest) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((key, rest)) => (key, rest.trim_start_matches(|c: char| c.is_whitespace() || c == '=')),
            None => continue,
        };
        if key.eq_ignore_ascii_case("host") {
            hosts.extend(rest.split_whitespace().filter(|h| !h.contains(['*', '?', '!'])).map(str::to_owned));
        } else if key.eq_ignore_ascii_case("include") && depth < 3 {
            for pattern in rest.split_whitespace() {
                for path in expand_include(pattern, ssh_dir) {
                    read_config(&path, ssh_dir, depth + 1, hosts);
                }
            }
        }
    }
}

/// An `Include` pattern: `~/`, relative to the ssh folder, `*` in the file name.
fn expand_include(pattern: &str, ssh_dir: &Path) -> Vec<PathBuf> {
    let path = match pattern.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest),
        None if pattern.starts_with('/') => PathBuf::from(pattern),
        None => ssh_dir.join(pattern),
    };
    let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { return Vec::new() };
    if !name.contains('*') {
        return vec![path];
    }
    let Some(dir) = path.parent() else { return Vec::new() };
    let (head, tail) = name.split_once('*').unwrap_or((&name, ""));
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.len() >= head.len() + tail.len() && n.starts_with(head) && n.ends_with(tail)))
        .map(|e| e.path())
        .collect();
    found.sort();
    found
}

/// `~/.ssh`, when `$HOME` is known.
pub fn ssh_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".ssh"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(text: &str) -> SshHost {
        SshHost::parse(text).unwrap()
    }

    #[test]
    fn parses_what_people_type() {
        assert_eq!(host("mini"), SshHost { user: None, host: "mini".into(), port: None });
        assert_eq!(host(" me@mini.local "), SshHost { user: Some("me".into()), host: "mini.local".into(), port: None });
        assert_eq!(host("me@10.0.0.5:2222"), SshHost { user: Some("me".into()), host: "10.0.0.5".into(), port: Some(2222) });
        assert_eq!(host("[::1]:2200"), SshHost { user: None, host: "::1".into(), port: Some(2200) });
        assert_eq!(host("fe80::1").port, None);
        assert_eq!(host("a@b").user.as_deref(), Some("a"));
    }

    #[test]
    fn refuses_what_could_become_an_option_or_a_command() {
        for bad in ["", "  ", "-oProxyCommand=x", "me@-bad", "a b", "a;b", "host:99999", "host:0", "@host", "x$(y)", "'a'"] {
            assert!(SshHost::parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn spec_round_trips_and_args_end_options() {
        for text in ["mini", "me@mini:22", "me@[::1]:2222", "::1"] {
            assert_eq!(host(&host(text).spec()), host(text));
        }
        assert_eq!(host("me@mini:2222").args(), ["-p", "2222", "--", "me@mini"]);
        assert_eq!(host("mini").args(), ["--", "mini"]);
    }

    #[test]
    fn remembers_the_last_target_and_recent_hosts() {
        let dir = std::env::temp_dir().join(format!("eq-target-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Remembered::load(&dir), Remembered::default());

        let mut remembered = Remembered::default();
        remembered.use_endpoint(&Endpoint::Ssh(host("me@a")));
        remembered.use_endpoint(&Endpoint::Ssh(host("b:2222")));
        remembered.use_endpoint(&Endpoint::Ssh(host("me@a")));
        remembered.board_split = Some(212.4);
        remembered.sidebar_width = Some(301.6);
        remembered.lang = Some("ko".into());
        remembered.set_merge_way("my site", "merge");
        remembered.set_merge_way("app", "push");
        remembered.set_merge_way("app", "branch");
        remembered.views.push(KeptView { name: "막힌\t것".into(), query: "is:blocked project:app".into(), group: "deps".into(), pinned: true });
        remembered.save(&dir);

        let loaded = Remembered::load(&dir);
        assert_eq!((loaded.sidebar_width, loaded.lang.as_deref()), (Some(302.), Some("ko")));
        assert_eq!(loaded.merge_way("my site"), Some("merge"), "a project's way is kept, spaces and all");
        assert_eq!(loaded.merge_way("app"), Some("branch"), "the last word wins");
        assert_eq!(loaded.merge_way("web"), None);
        assert_eq!(loaded.views, vec![KeptView { name: "막힌 것".into(), query: "is:blocked project:app".into(), group: "deps".into(), pinned: true }]);
        assert_eq!(loaded.board_split, Some(212.0), "the board's divider is kept, to the pixel");
        assert_eq!(loaded.last, Some(Endpoint::Ssh(host("me@a"))));
        assert_eq!(loaded.recent, vec![host("me@a"), host("b:2222")]);

        let mut local = loaded;
        local.use_endpoint(&Endpoint::this_mac());
        local.save(&dir);
        assert_eq!(Remembered::load(&dir).last, Some(Endpoint::this_mac()));
        assert_eq!(Remembered::load(&dir).recent.len(), 2, "this Mac isn't a host to remember");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_kept_on_this_mac_round_trip_and_defaults_are_not_written() {
        let dir = std::env::temp_dir().join(format!("eq-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let fresh = Remembered::load(&dir);
        assert_eq!((fresh.appearance, fresh.start, fresh.pings), (Appearance::System, Start::Pm, Pings::default()), "the system's look, the PM, every notice but merges");
        assert!(!fresh.notify_later);

        assert!(!fresh.whole_conversation, "the conversation follows the project by default");
        assert!(!fresh.tickets_board, "티켓 opens as a list");
        assert_eq!(fresh.zoom, Zoom(100), "actual size");
        let mut prefs = Remembered { appearance: Appearance::Dark, start: Start::Inbox, whole_conversation: true, tickets_board: true, zoom: Zoom(125), ..Remembered::default() };
        prefs.pings.set(Ping::Failed, false);
        prefs.pings.set(Ping::Sound, false);
        prefs.set_merge_way("app", "push");
        prefs.set_merge_way("site", "merge");
        prefs.clear_merge_way("app");
        prefs.folded.extend(["my site".to_owned(), String::new()]);
        prefs.save(&dir);
        let loaded = Remembered::load(&dir);
        assert_eq!((loaded.appearance, loaded.start, loaded.whole_conversation), (Appearance::Dark, Start::Inbox, true));
        assert_eq!(loaded.zoom, Zoom(125), "the window's size, from one launch to the next");
        assert_eq!(loaded.folded, prefs.folded, "the sidebar's folds, the shared folder's too");
        assert!(loaded.tickets_board, "the board, as last left");
        assert!(!loaded.pings.failed && !loaded.pings.sound && loaded.pings.approvals && loaded.pings.reviews && loaded.pings.waiting);
        assert_eq!((loaded.merge_way("app"), loaded.merge_way("site")), (None, Some("merge")), "back to asking each time");

        // The kinds that are off by default are written when on, and the rest when off.
        let mut prefs = Remembered { notify_later: true, ..Remembered::default() };
        prefs.pings.set(Ping::Merged, true);
        prefs.pings.set(Ping::Badge, false);
        prefs.pings.set(Ping::Replies, false);
        prefs.save(&dir);
        let text = std::fs::read_to_string(Remembered::path(&dir)).unwrap();
        assert_eq!(text, "quiet replies\nping merged\nquiet badge\nnotify later\n");
        let loaded = Remembered::load(&dir);
        assert_eq!(loaded.pings, prefs.pings);
        assert!(loaded.notify_later && loaded.pings.update && loaded.pings.sound);
        let old = Remembered::parse("quiet sound\n");
        assert!(!old.pings.sound && !old.pings.merged && old.pings.replies && old.pings.badge, "an older file reads with the new kinds as by default");

        Remembered::default().save(&dir);
        assert_eq!(std::fs::read_to_string(Remembered::path(&dir)).unwrap(), "", "defaults take no lines");
        for (text, look) in [("appearance light\n", Appearance::Light), ("appearance sepia\n", Appearance::System)] {
            assert_eq!(Remembered::parse(text).appearance, look);
        }
        // A size by hand is taken to the nearest step; nonsense is actual size.
        for (text, zoom) in [("zoom 150\n", 150), ("zoom 1000\n", 200), ("zoom 30\n", 67), ("zoom 112\n", 110), ("zoom big\n", 100)] {
            assert_eq!(Remembered::parse(text).zoom, Zoom(zoom), "{text}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lists_hosts_from_the_ssh_config_without_patterns() {
        let dir = std::env::temp_dir().join(format!("eq-sshcfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("conf.d")).unwrap();
        std::fs::write(
            dir.join("config"),
            "Include conf.d/*.conf\nHost mini nas\n  HostName 10.0.0.2\nHost *\n  ServerAliveInterval 30\nhost = dev !bad *.corp\n",
        )
        .unwrap();
        std::fs::write(dir.join("conf.d/work.conf"), "Host work-box\n  User me\n").unwrap();
        std::fs::write(dir.join("conf.d/skip.txt"), "Host skipped\n").unwrap();
        assert_eq!(config_hosts(&dir), ["work-box", "mini", "nas", "dev"]);
        assert!(config_hosts(&dir.join("missing")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
