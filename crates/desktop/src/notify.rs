//! What the person is told when the window is out of sight (T-62 §4): a request
//! for permission, work to review, a ticket put down for them, a reply to what
//! they wrote — who needs what, never the command or the content, which would
//! stay in the notification centre. The wording, grouping, threads and where a
//! click goes are plain functions, tested here; only `system` (macOS's
//! UNUserNotificationCenter, from the app bundle) touches the system.
//!
//! The rules:
//! - Only while the window is out of sight (another app in front, the window
//!   hidden, minimised or on another Space); in sight, the sidebar and the
//!   cards say it. The delegate keeps the rule too (`willPresent`).
//! - Only the kinds the person wants (settings ▸ 일반 ▸ 알림).
//! - Permission is asked when it is first useful — the first thing worth
//!   telling, or a switch turned on — with a sheet saying why first, then
//!   macOS's own question.
//! - Driven by what the window hears from the daemon, so a window onto another
//!   Mac's daemon over SSH tells the person on *this* Mac; the daemon never
//!   shows a notification itself.

use std::cell::Cell;

use equipe_core::protocol::{Approval, Entry, OffMain, SessionId, Ticket, TicketId, TicketKind, TicketStatus};
use equipe_core::t;

use crate::views::approvals::noun;

#[cfg(target_os = "macos")]
pub mod system;

/// More than this many at once become one notice.
const GROUP_AFTER: usize = 3;

/// A reply's first line, at most this many characters.
const LINE_MAX: usize = 100;

/// Something that needs the person, or news they asked for, as the model saw it happen.
#[derive(Debug, Clone, PartialEq)]
pub enum Alert {
    Approval { approval: Approval, squad: String },
    Review { ticket: TicketId, title: String, by: Option<String>, project: Option<String> },
    Waiting { ticket: TicketId, by: Option<String> },
    /// A member's turn failed: its work stopped until someone looks.
    Failed { session: SessionId, member: String },
    /// A member puts a decision to the person: `count` questions.
    Decision { session: SessionId, member: String, count: usize },
    /// A member's browser page waits for the person's own hand (a "verify you
    /// are human" check, a sign-in): `page`, on `site`.
    Browser { page: u64, session: Option<SessionId>, member: String, site: String },
    /// A member's turn ended with an answer: `line`, its first line. Told only
    /// for the conversation the person last wrote in ([`only_awaited`]).
    Reply { session: SessionId, member: String, line: String },
    /// An accepted ticket's work is on the main branch now.
    Merged { ticket: TicketId, title: String, project: Option<String> },
    /// Merging an accepted ticket's work failed — `conflict` with main, or
    /// otherwise; nothing was merged.
    MergeFailed { ticket: TicketId, conflict: bool, project: Option<String> },
    /// A new version of the app is found; it is being downloaded.
    UpdateFound { version: String },
    /// A new version of the app is downloaded and goes in on a restart.
    Update { version: String },
}

impl Alert {
    /// News rather than something waiting: told one by one, never folded into
    /// "n things wait for you".
    fn is_news(&self) -> bool {
        matches!(self, Alert::Reply { .. } | Alert::Merged { .. } | Alert::UpdateFound { .. } | Alert::Update { .. })
    }
}

/// What the person wants to be told of (settings → 알림), kept with the
/// window's other choices in `target.rs`.
pub use crate::target::{Ping, Pings};

impl Pings {
    /// Whether the person wants to be told of `alert`.
    pub fn wants(&self, alert: &Alert) -> bool {
        match alert {
            Alert::Approval { .. } => self.approvals,
            Alert::Review { .. } => self.reviews,
            Alert::Waiting { .. } | Alert::Decision { .. } | Alert::Browser { .. } => self.waiting,
            // A merge that failed waits for the person, as a failed turn does.
            Alert::Failed { .. } | Alert::MergeFailed { .. } => self.failed,
            Alert::Reply { .. } => self.replies,
            Alert::Merged { .. } => self.merged,
            Alert::UpdateFound { .. } | Alert::Update { .. } => self.update,
        }
    }
}

/// Where a click on the notification goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Go {
    /// The member's conversation, where the card to answer is.
    Member(SessionId),
    /// The ticket, on the board.
    Ticket(TicketId),
    Board,
    /// The browser panel, on a page waiting for the person, in their hands.
    Browser(u64),
    /// The update's notice: with the update downloaded, a restart into it;
    /// before, the window, where the sidebar's pill follows the download.
    Update,
    /// The update notice's "다시 시작": install the downloaded update now.
    Relaunch,
}

impl Go {
    /// How it travels in a notification's identifier, which comes back with a
    /// click — even one that starts the app again.
    pub fn code(self) -> String {
        match self {
            Go::Member(s) => format!("member:{s}"),
            Go::Ticket(t) => format!("ticket:{t}"),
            Go::Board => "board".into(),
            Go::Browser(p) => format!("browser:{p}"),
            Go::Update => "update".into(),
            Go::Relaunch => "relaunch".into(),
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        let (kind, n) = code.split_once(':').unwrap_or((code, ""));
        match kind {
            "member" => n.parse().ok().map(Go::Member),
            "ticket" => n.parse().ok().map(Go::Ticket),
            "browser" => n.parse().ok().map(Go::Browser),
            "board" => Some(Go::Board),
            "update" => Some(Go::Update),
            "relaunch" => Some(Go::Relaunch),
            _ => None,
        }
    }
}

/// The update notice's button, as macOS reports it.
pub const ACTION_RELAUNCH: &str = "relaunch";
/// Every other notice's button.
pub const ACTION_OPEN: &str = "open";
/// The notices that carry "열기", and the update's that carries "다시 시작".
pub const CATEGORY_OPEN: &str = "equipe.open";
pub const CATEGORY_UPDATE: &str = "equipe.update";

/// Where a click goes: `id`, the notice's identifier; `action`, which of its
/// buttons (or the notice itself, or a dismissal).
pub fn clicked(id: &str, action: &str) -> Option<Go> {
    if action == ACTION_RELAUNCH {
        return Some(Go::Relaunch);
    }
    if action.contains("Dismiss") {
        return None;
    }
    Go::parse(id.split('|').next()?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub title: String,
    pub body: String,
    pub go: Go,
    /// Notification Center keeps a thread's notices together: one per member,
    /// else per squad.
    pub thread: String,
}

impl Toast {
    /// The notice's identifier, `n` making it unique — except the update's:
    /// "downloaded" replaces "found", a newer version the older.
    pub fn request_id(&self, n: u64) -> String {
        match self.go {
            Go::Update => "update|0".into(),
            go => format!("{}|{n}", go.code()),
        }
    }

    pub fn category(&self) -> &'static str {
        if self.go == Go::Update { CATEGORY_UPDATE } else { CATEGORY_OPEN }
    }
}

/// The thread a squad's notice goes in when no one member's is.
fn squad_thread(squad: &str) -> String {
    format!("squad/{squad}")
}

fn member_thread(squad: &str, member: &str) -> String {
    format!("squad/{squad}/{member}")
}

/// Which thread `alert` is in, `squad`'s.
pub fn thread(alert: &Alert, squad: &str) -> String {
    match alert {
        Alert::Approval { approval, squad } => member_thread(squad, &approval.member),
        Alert::Review { by, .. } | Alert::Waiting { by, .. } => by.as_deref().map_or_else(|| squad_thread(squad), |b| member_thread(squad, b)),
        Alert::Failed { member, .. } | Alert::Decision { member, .. } | Alert::Browser { member, .. } | Alert::Reply { member, .. } => member_thread(squad, member),
        Alert::Merged { .. } | Alert::MergeFailed { .. } => squad_thread(squad),
        Alert::UpdateFound { .. } | Alert::Update { .. } => "equipe/update".into(),
    }
}

/// The notices for `alerts`, `squad`'s. `titles`: whether ticket titles may show.
pub fn toasts(alerts: &[Alert], squad: &str, titles: bool) -> Vec<Toast> {
    let (news, needs): (Vec<&Alert>, Vec<&Alert>) = alerts.iter().partition(|a| a.is_news());
    let mut out = Vec::new();
    if needs.len() > GROUP_AFTER {
        let approvals = needs.iter().filter(|a| matches!(a, Alert::Approval { .. })).count();
        let reviews = needs.iter().filter(|a| matches!(a, Alert::Review { .. })).count();
        let failed = needs.iter().filter(|a| matches!(a, Alert::Failed { .. } | Alert::MergeFailed { .. })).count();
        let waits = needs.len() - approvals - reviews - failed;
        let parts: Vec<String> = [
            (approvals, t!("notify.count_approvals", n = approvals)),
            (reviews, t!("notify.count_reviews", n = reviews)),
            (waits, t!("notify.count_waits", n = waits)),
            (failed, t!("notify.count_failed", n = failed)),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(_, text)| text)
        .collect();
        out.push(Toast { title: t!("notify.grouped_title", squad = squad, n = needs.len()), body: parts.join(" · "), go: Go::Board, thread: squad_thread(squad) });
    } else {
        out.extend(needs.into_iter().map(|a| toast(a, squad, titles)));
    }
    out.extend(news.into_iter().map(|a| toast(a, squad, titles)));
    out
}

fn toast(alert: &Alert, squad: &str, titles: bool) -> Toast {
    let (title, body, go) = match alert {
        Alert::Approval { approval, squad } => {
            let ticket = approval.ticket.map(|t| format!(" · T-{t}")).unwrap_or_default();
            let send = approval.tool == equipe_core::desktop_gate::SEND;
            (t!("notify.approval_title", member = approval.member, what = noun(&approval.tool)), format!("{squad}{ticket}{}", if send { t!("notify.send_waits") } else { "" }), Go::Member(approval.session))
        }
        Alert::Review { ticket, title, by, project } => {
            let who = by.as_deref().map(|b| t!("notify.submitted_by", b = b)).unwrap_or_else(|| t!("notify.submitted").into());
            let what = if titles { format!(" · {title}") } else { String::new() };
            let project = project.as_deref().map(|p| format!(" · {p}")).unwrap_or_default();
            (t!("notify.review_title", ticket = ticket), format!("{who}{what}{project}"), Go::Ticket(*ticket))
        }
        Alert::Waiting { ticket, by } => (t!("notify.waiting_title", ticket = ticket), t!("notify.waiting_body", by = by.as_deref().map(|b| format!("{b} · ")).unwrap_or_default()), Go::Ticket(*ticket)),
        Alert::Failed { session, member } => (t!("notify.failed_title", member = member), t!("notify.failed_body", squad = squad), Go::Member(*session)),
        // Who asks and how many — never the questions.
        Alert::Decision { session, member, count } => (t!("notify.decision_title", member = member, count = count), squad.to_owned(), Go::Member(*session)),
        // Who and where — the page itself stays in the panel.
        Alert::Browser { page, member, site, .. } => (t!("notify.browser_title", member = member), t!("notify.browser_body", site = site, squad = squad), Go::Browser(*page)),
        // Who answered, and the answer's first line: what the person asked to hear.
        Alert::Reply { session, member, line } => (t!("notify.reply_title", member = member), line.clone(), Go::Member(*session)),
        Alert::Merged { ticket, title, project } => {
            let what = if titles { title.clone() } else { String::new() };
            let body = [what, project.clone().unwrap_or_default(), squad.to_owned()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            (t!("notify.merged_title", ticket = ticket), body, Go::Ticket(*ticket))
        }
        Alert::MergeFailed { ticket, conflict, project } => {
            let why = if *conflict { t!("notify.merge_conflict") } else { t!("notify.merge_failed") };
            let project = project.as_deref().map(|p| format!(" · {p}")).unwrap_or_default();
            (t!("notify.merge_failed_title", ticket = ticket), format!("{why}{project}"), Go::Ticket(*ticket))
        }
        Alert::UpdateFound { version } => (t!("notify.update_found_title", version = version), t!("notify.update_found_body").to_owned(), Go::Update),
        Alert::Update { version } => (t!("notify.update_title").to_owned(), t!("notify.update_body", version = version), Go::Update),
    };
    Toast { title, body, go, thread: thread(alert, squad) }
}

/// The first line of a member's last answer in `entries` (a conversation), as a
/// notice shows it: the words of its first line of prose — not a code block,
/// not markdown's marks — cut short. `None` when the person spoke last.
pub fn reply_line(entries: &[Entry]) -> Option<String> {
    for entry in entries.iter().rev() {
        match entry {
            Entry::Reply { text } => {
                if let Some(line) = first_line(text) {
                    return Some(line);
                }
            }
            Entry::User { .. } => return None,
            _ => {}
        }
    }
    None
}

fn first_line(text: &str) -> Option<String> {
    let mut fenced = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced || line.chars().all(|c| matches!(c, '-' | '=' | '*' | '_' | '|' | ':' | ' ')) {
            continue;
        }
        let line = line.trim_start_matches(['#', '>']).trim_start();
        let line = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).unwrap_or(line);
        let words: String = line.replace("**", "").replace("__", "").replace('`', "").split_whitespace().collect::<Vec<_>>().join(" ");
        if words.is_empty() {
            continue;
        }
        return Some(if words.chars().count() > LINE_MAX { format!("{}…", words.chars().take(LINE_MAX - 1).collect::<String>().trim_end()) } else { words });
    }
    None
}

/// Whether a ticket that changed from `old` to `new` landed on main, or failed
/// to: news to tell. Nothing for a ticket that already had.
pub fn merge_alert(old: &Ticket, new: &Ticket, project: Option<String>) -> Option<Alert> {
    let landed = |t: &Ticket| t.kind == TicketKind::Code && t.status == TicketStatus::Done && t.branch.is_some() && t.merged == Some(true);
    let failed = |t: &Ticket| match &t.off_main {
        Some(OffMain::Conflict { .. }) => Some(true),
        Some(OffMain::Failed { .. }) => Some(false),
        _ => None,
    };
    if landed(new) && !landed(old) {
        return Some(Alert::Merged { ticket: new.id, title: new.title.clone(), project });
    }
    match (failed(old), failed(new)) {
        (None, Some(conflict)) => Some(Alert::MergeFailed { ticket: new.id, conflict, project }),
        _ => None,
    }
}

/// Of `alerts`, the replies only from the conversation the person last wrote
/// in (`awaiting`), and only the first: that one is told, and nothing more is
/// awaited until they write again.
pub fn only_awaited(alerts: Vec<Alert>, awaiting: &Cell<Option<SessionId>>) -> Vec<Alert> {
    alerts
        .into_iter()
        .filter(|alert| match alert {
            Alert::Reply { session, .. } if awaiting.get() == Some(*session) => {
                awaiting.set(None);
                true
            }
            Alert::Reply { .. } => false,
            _ => true,
        })
        .collect()
}

/// Whether macOS lets equipe show notifications.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Permission {
    /// Not known yet (being asked).
    #[default]
    Unknown,
    /// Never asked: macOS asks the first time.
    NotAsked,
    Allowed,
    /// Turned off, in System Settings or when macOS asked.
    Denied,
    /// Not from the app bundle (`cargo run`): macOS has no app to ask for.
    NoBundle,
}

impl Permission {
    /// From UNAuthorizationStatus: not determined, denied, authorized,
    /// provisional, ephemeral.
    pub fn from_status(status: isize) -> Self {
        match status {
            0 => Permission::NotAsked,
            1 => Permission::Denied,
            2..=4 => Permission::Allowed,
            _ => Permission::Unknown,
        }
    }
}

/// Whether to show the sheet that says why equipe asks to notify, now that
/// there is something worth telling: never asked, and the person didn't say
/// "나중에" already.
pub fn explain_now(permission: Permission, later: bool) -> bool {
    permission == Permission::NotAsked && !later
}

/// The number the Dock's icon shows: what waits for the person, when they want it.
pub fn badge(needs_you: usize, pings: &Pings) -> Option<usize> {
    (pings.badge && needs_you > 0).then_some(needs_you)
}

/// The notification state the window keeps.
#[derive(Debug, Default)]
pub struct Bell {
    pub permission: Permission,
    /// The sheet saying why equipe asks, over the window.
    pub sheet: bool,
    /// The conversation the person last wrote in, until its answer is told.
    pub awaiting: Cell<Option<SessionId>>,
    /// What the Dock shows now (`None`: no number), once set.
    pub badge: Option<Option<usize>>,
}

/// A line to say when the daemon runs an older release than this app, else `None`.
/// Builds read `0.3.1+<seconds>`; only the release number is compared.
pub fn stale_daemon(daemon: &str, own: &str) -> Option<String> {
    let release = |v: &str| -> Option<Vec<u64>> { v.split('+').next()?.split('.').map(|n| n.parse().ok()).collect() };
    let (theirs, ours) = (release(daemon)?, release(own)?);
    (theirs < ours).then(|| t!("notify.stale_daemon", version = daemon.split('+').next().unwrap_or(daemon)))
}

// ── the system's side, wherever there is one ──

/// Start: the notification centre's delegate (clicks, and the rule for a
/// window in sight), the buttons, what macOS allows. Call once, on the main
/// thread, as the app starts. From a dev build (no bundle) it says so once.
pub fn start() {
    #[cfg(target_os = "macos")]
    system::start();
}

/// Follow what macOS allows: now, and whenever it changes.
pub fn permission() -> tokio::sync::watch::Receiver<Permission> {
    permission_channel().subscribe()
}

fn permission_channel() -> &'static tokio::sync::watch::Sender<Permission> {
    static PERMISSION: std::sync::OnceLock<tokio::sync::watch::Sender<Permission>> = std::sync::OnceLock::new();
    PERMISSION.get_or_init(|| tokio::sync::watch::channel(Permission::Unknown).0)
}

pub(crate) fn set_permission(permission: Permission) {
    permission_channel().send_if_modified(|now| std::mem::replace(now, permission) != permission);
}

/// Ask macOS again what it allows (the person may have changed it in System Settings).
pub fn refresh() {
    #[cfg(target_os = "macos")]
    system::refresh();
}

/// macOS's own question: may equipe show notifications (banners, sounds, the badge)?
pub fn ask() {
    #[cfg(target_os = "macos")]
    system::ask();
}

/// Show `toast`, with the system's sound or silently. Nothing when macOS
/// doesn't allow it — the badge and the window's cards still say it.
pub fn send(toast: &Toast, sound: bool) {
    #[cfg(target_os = "macos")]
    system::send(toast, sound);
    #[cfg(not(target_os = "macos"))]
    let _ = (toast, sound);
}

/// Where clicks go: sent to `to`, from now on (and those that came before, at once).
pub fn follow_clicks(to: tokio::sync::mpsc::UnboundedSender<Go>) {
    #[cfg(target_os = "macos")]
    system::follow_clicks(to);
    #[cfg(not(target_os = "macos"))]
    let _ = to;
}

/// The update notice for where the update stands, if it wasn't told yet:
/// each version is told once found and once downloaded — `told` remembers
/// (`found <v>`, `ready <v>`; the last few), across launches.
pub fn update_alert(now: &crate::relaunch::Update, told: &mut Vec<String>) -> Option<Alert> {
    use crate::relaunch::Update;
    let (key, alert) = match now {
        Update::Found(v) | Update::Downloading(v) => (format!("found {v}"), Alert::UpdateFound { version: v.clone() }),
        Update::Ready(v) => (format!("ready {v}"), Alert::Update { version: v.clone() }),
        _ => return None,
    };
    if told.contains(&key) {
        return None;
    }
    told.push(key);
    let extra = told.len().saturating_sub(4);
    told.drain(..extra);
    Some(alert)
}

/// Whether the window is in front of the person now: then a notice that
/// arrives anyway (a race) isn't shown as a banner.
pub fn set_in_sight(in_sight: bool) {
    #[cfg(target_os = "macos")]
    system::set_in_sight(in_sight);
    #[cfg(not(target_os = "macos"))]
    let _ = in_sight;
}

/// The number on the Dock's icon; `None` takes it off.
pub fn set_badge(count: Option<usize>) {
    #[cfg(target_os = "macos")]
    system::set_badge(count);
    #[cfg(not(target_os = "macos"))]
    let _ = count;
}

/// System Settings ▸ Notifications, on equipe when the app knows its bundle.
pub fn settings_url() -> String {
    #[cfg(target_os = "macos")]
    if let Some(id) = system::bundle_id() {
        return format!("x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={id}");
    }
    "x-apple.systempreferences:com.apple.Notifications-Settings.extension".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval(tool: &str, action: &str) -> Approval {
        Approval { id: 1, session: 4, member: "backend".into(), ticket: Some(52), tool: tool.into(), action: action.into(), reason: None, preview: None }
    }

    fn ask(tool: &str, action: &str) -> Alert {
        Alert::Approval { approval: approval(tool, action), squad: "acme".into() }
    }

    fn reply(session: SessionId, line: &str) -> Alert {
        Alert::Reply { session, member: "pm".into(), line: line.into() }
    }

    #[test]
    fn a_request_says_who_and_what_kind_but_never_the_command() {
        let t = toasts(&[ask("Bash", "rm -rf /secret")], "acme", true);
        assert_eq!(t.len(), 1);
        assert!(t[0].title.contains("backend") && t[0].title.contains("명령"));
        assert_eq!(t[0].body, "acme · T-52");
        assert!(!format!("{:?}", t[0]).contains("secret"));
        assert_eq!(t[0].go, Go::Member(4));
    }

    #[test]
    fn a_decision_says_who_asks_and_how_many_and_opens_their_conversation() {
        let t = toasts(&[Alert::Decision { session: 3, member: "backend".into(), count: 2 }], "acme", true);
        assert_eq!((t[0].title.as_str(), t[0].body.as_str(), t[0].go), ("backend이(가) 2가지 결정을 기다립니다", "acme", Go::Member(3)));
    }

    #[test]
    fn a_send_says_it_waits_for_a_yes() {
        let t = toasts(&[ask(equipe_core::desktop_gate::SEND, "press return in Slack\nhi")], "acme", true);
        assert!(t[0].body.contains("허락"));
        assert!(!t[0].body.contains("hi"));
    }

    #[test]
    fn a_review_names_the_ticket_and_hides_the_title_when_asked() {
        let alert = Alert::Review { ticket: 54, title: "화면 설계".into(), by: Some("designer".into()), project: Some("desktop".into()) };
        let shown = toasts(std::slice::from_ref(&alert), "acme", true);
        assert_eq!(shown[0].title, "T-54 검토를 기다립니다");
        assert_eq!(shown[0].body, "designer 제출 · 화면 설계 · desktop");
        assert_eq!(shown[0].go, Go::Ticket(54));
        assert!(!toasts(&[alert], "acme", false)[0].body.contains("화면 설계"));
    }

    #[test]
    fn many_at_once_become_one() {
        let alerts = vec![ask("Bash", "a"), ask("Bash", "b"), Alert::Waiting { ticket: 3, by: None }, Alert::Review { ticket: 4, title: "t".into(), by: None, project: None }];
        let t = toasts(&alerts, "acme", true);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].title, "acme에서 4건이 당신을 기다립니다");
        assert_eq!(t[0].body, "승인 2 · 리뷰 1 · 사람 대기 1");
        assert_eq!(t[0].go, Go::Board);
        assert_eq!(t[0].thread, "squad/acme");
    }

    #[test]
    fn news_is_never_folded_into_what_waits() {
        let mut alerts = vec![ask("Bash", "a"), ask("Bash", "b"), Alert::Waiting { ticket: 3, by: None }, Alert::MergeFailed { ticket: 9, conflict: true, project: None }];
        alerts.push(reply(1, "끝났어요"));
        alerts.push(Alert::Merged { ticket: 8, title: "로그인".into(), project: None });
        let t = toasts(&alerts, "acme", true);
        assert_eq!(t.len(), 3, "{t:?}");
        assert_eq!(t[0].body, "승인 2 · 사람 대기 1 · 실패 1", "a failed merge counts as a failure");
        assert_eq!((t[1].title.as_str(), t[1].body.as_str(), t[1].go), ("pm의 답장이 왔어요", "끝났어요", Go::Member(1)));
        assert_eq!(t[2].go, Go::Ticket(8));
        // Three needs and a reply: the needs aren't grouped.
        let few = toasts(&[ask("Bash", "a"), ask("Bash", "b"), Alert::Waiting { ticket: 3, by: None }, reply(1, "네")], "acme", true);
        assert_eq!(few.len(), 4);
    }

    #[test]
    fn a_page_waiting_for_the_person_says_who_and_where_and_opens_the_panel_on_it() {
        let t = toasts(&[Alert::Browser { page: 42, session: Some(3), member: "backend".into(), site: "dash.cloudflare.com".into() }], "acme", true);
        assert_eq!(t[0].title, "backend: 사람 확인이 필요해요");
        assert_eq!(t[0].body, "dash.cloudflare.com · 브라우저 패널에서 직접 눌러 주세요 · acme");
        assert_eq!(t[0].go, Go::Browser(42));
        let pings = Pings::default();
        assert_eq!(pings.wants(&Alert::Browser { page: 1, session: None, member: String::new(), site: String::new() }), pings.waiting);
    }

    #[test]
    fn a_failed_turn_names_the_member_and_goes_to_its_conversation() {
        let t = toasts(&[Alert::Failed { session: 6, member: "qa".into() }], "acme", true);
        assert!(t[0].title.contains("qa"), "{:?}", t[0]);
        assert!(t[0].body.contains("acme"));
        assert_eq!(t[0].go, Go::Member(6));
        let many = vec![ask("Bash", "a"), ask("Bash", "b"), Alert::Failed { session: 6, member: "qa".into() }, Alert::Waiting { ticket: 3, by: None }];
        assert_eq!(toasts(&many, "acme", true)[0].body, "승인 2 · 사람 대기 1 · 실패 1");
    }

    #[test]
    fn merges_and_updates_are_worded_and_go_where_they_belong() {
        let merged = toasts(&[Alert::Merged { ticket: 12, title: "로그인 화면".into(), project: Some("web".into()) }], "acme", true);
        assert_eq!((merged[0].title.as_str(), merged[0].body.as_str()), ("T-12 · main에 머지됐어요", "로그인 화면 · web · acme"));
        assert_eq!(toasts(&[Alert::Merged { ticket: 12, title: "로그인 화면".into(), project: None }], "acme", false)[0].body, "acme", "the title stays hidden when asked");
        let failed = toasts(&[Alert::MergeFailed { ticket: 12, conflict: true, project: Some("web".into()) }], "acme", true);
        assert_eq!((failed[0].title.as_str(), failed[0].body.as_str(), failed[0].go), ("T-12 · 머지하지 못했어요", "main과 충돌해 아무것도 머지되지 않았어요 · web", Go::Ticket(12)));
        let update = toasts(&[Alert::Update { version: "0.48.0".into() }], "acme", true);
        assert_eq!((update[0].title.as_str(), update[0].body.as_str(), update[0].go), ("새 버전이 준비됐어요", "다시 시작해 업데이트 · 0.48.0", Go::Update));
        assert_eq!(update[0].category(), CATEGORY_UPDATE);
        assert_eq!(merged[0].category(), CATEGORY_OPEN);
    }

    #[test]
    fn notices_are_threaded_by_member_else_by_squad() {
        assert_eq!(thread(&ask("Bash", "a"), "acme"), "squad/acme/backend");
        assert_eq!(thread(&reply(1, "x"), "acme"), "squad/acme/pm");
        assert_eq!(thread(&Alert::Review { ticket: 1, title: String::new(), by: Some("qa".into()), project: None }, "acme"), "squad/acme/qa");
        assert_eq!(thread(&Alert::Waiting { ticket: 1, by: None }, "acme"), "squad/acme");
        assert_eq!(thread(&Alert::Merged { ticket: 1, title: String::new(), project: None }, "acme"), "squad/acme");
        assert_eq!(thread(&Alert::Update { version: "1".into() }, "acme"), "equipe/update");
        let t = toasts(&[Alert::Failed { session: 6, member: "qa".into() }], "acme", true);
        assert_eq!(t[0].thread, "squad/acme/qa");
    }

    #[test]
    fn a_click_comes_back_to_where_the_notice_pointed() {
        for go in [Go::Member(4), Go::Ticket(52), Go::Board, Go::Browser(9_000_001), Go::Update, Go::Relaunch] {
            assert_eq!(Go::parse(&go.code()), Some(go));
        }
        let toast = Toast { title: String::new(), body: String::new(), go: Go::Member(4), thread: String::new() };
        assert_eq!(toast.request_id(7), "member:4|7");
        assert_eq!(clicked(&toast.request_id(7), "com.apple.UNNotificationDefaultActionIdentifier"), Some(Go::Member(4)));
        assert_eq!(clicked(&toast.request_id(7), ACTION_OPEN), Some(Go::Member(4)));
        assert_eq!(clicked(&toast.request_id(7), "com.apple.UNNotificationDismissActionIdentifier"), None);
        let update = Toast { go: Go::Update, ..toast };
        assert_eq!(update.request_id(1), update.request_id(2), "a newer update notice replaces the older");
        assert_eq!(clicked(&update.request_id(1), ACTION_RELAUNCH), Some(Go::Relaunch));
        assert_eq!(clicked(&update.request_id(1), "com.apple.UNNotificationDefaultActionIdentifier"), Some(Go::Update));
        assert_eq!(clicked("nonsense|1", ACTION_OPEN), None);
        assert_eq!(Go::parse("member:x"), None);
    }

    #[test]
    fn a_reply_is_its_first_line_of_prose_cut_short() {
        let said = |text: &str| vec![Entry::User { text: "고쳐 줘".into() }, Entry::Reply { text: text.into() }];
        assert_eq!(reply_line(&said("## **끝났어요.** 테스트도 통과\n자세히는…")), Some("끝났어요. 테스트도 통과".into()));
        assert_eq!(reply_line(&said("```sh\nrm -rf /tmp/x\n```\n\n- `main`에 올렸어요")), Some("main에 올렸어요".into()), "never a code block");
        let long = "가".repeat(300);
        let line = reply_line(&said(&long)).unwrap();
        assert_eq!(line.chars().count(), LINE_MAX);
        assert!(line.ends_with('…'));
        assert_eq!(reply_line(&[Entry::Reply { text: "old".into() }, Entry::User { text: "new".into() }]), None, "the person spoke last");
        // A turn's last answer is the one told; an empty one falls back.
        let turn = vec![Entry::User { text: "q".into() }, Entry::Reply { text: "살펴볼게요".into() }, Entry::Reply { text: "다 됐어요".into() }];
        assert_eq!(reply_line(&turn), Some("다 됐어요".into()));
        let turn = vec![Entry::User { text: "q".into() }, Entry::Reply { text: "살펴볼게요".into() }, Entry::Reply { text: "```\nx\n```".into() }];
        assert_eq!(reply_line(&turn), Some("살펴볼게요".into()));
    }

    #[test]
    fn only_the_conversation_last_written_in_is_told_its_answer_once() {
        let awaiting = Cell::new(Some(1));
        let kept = only_awaited(vec![reply(2, "other"), reply(1, "mine"), ask("Bash", "a")], &awaiting);
        assert_eq!(kept, vec![reply(1, "mine"), ask("Bash", "a")]);
        assert_eq!(awaiting.get(), None, "told: nothing more awaited");
        assert_eq!(only_awaited(vec![reply(1, "again")], &awaiting), vec![], "a later turn of its own isn't an answer");
    }

    fn code_ticket(status: &str, merged: Option<bool>, off_main: Option<serde_json::Value>) -> Ticket {
        serde_json::from_value(serde_json::json!({"id": 12, "title": "로그인", "body": "", "status": status, "kind": "code", "owner": "backend", "assignee": "backend", "project": null, "depends_on": [], "branch": "eq/t12", "merged": merged, "off_main": off_main, "created_by": "pm", "note": null})).unwrap()
    }

    #[test]
    fn landing_on_main_or_failing_to_is_news_once() {
        let review = code_ticket("review", None, None);
        let landed = code_ticket("done", Some(true), None);
        assert_eq!(merge_alert(&review, &landed, Some("web".into())), Some(Alert::Merged { ticket: 12, title: "로그인".into(), project: Some("web".into()) }), "accepted and merged at once");
        assert_eq!(merge_alert(&landed, &landed, None), None, "already said");
        let accepted = code_ticket("done", Some(false), None);
        assert_eq!(merge_alert(&review, &accepted, None), None, "accepted, not merged: nothing yet");
        assert!(matches!(merge_alert(&accepted, &landed, None), Some(Alert::Merged { .. })), "merged later");
        let conflict = code_ticket("done", Some(false), Some(serde_json::json!({"why": "conflict", "detail": "x"})));
        assert_eq!(merge_alert(&accepted, &conflict, None), Some(Alert::MergeFailed { ticket: 12, conflict: true, project: None }));
        assert_eq!(merge_alert(&conflict, &conflict, None), None);
        let failed = code_ticket("done", Some(false), Some(serde_json::json!({"why": "failed", "detail": "x"})));
        assert_eq!(merge_alert(&accepted, &failed, None), Some(Alert::MergeFailed { ticket: 12, conflict: false, project: None }));
        let kept = code_ticket("done", Some(false), Some(serde_json::json!({"why": "kept"})));
        assert_eq!(merge_alert(&accepted, &kept, None), None, "left on its branch by the person: no news");
    }

    #[test]
    fn each_kind_of_notice_can_be_turned_off_on_its_own() {
        let mut pings = Pings::default();
        let failed = Alert::Failed { session: 6, member: "qa".into() };
        let waiting = Alert::Waiting { ticket: 3, by: None };
        assert!(Ping::ALL.iter().all(|p| pings.get(*p) == p.default_on()), "as by default");
        assert!(pings.wants(&reply(1, "x")) && pings.wants(&Alert::Update { version: "1".into() }) && pings.badge, "replies, updates and the badge are on");
        assert!(!pings.wants(&Alert::Merged { ticket: 1, title: String::new(), project: None }), "merges are off until asked for");
        assert!(pings.wants(&Alert::MergeFailed { ticket: 1, conflict: false, project: None }), "a failed merge is a failure");
        pings.set(Ping::Failed, false);
        assert!(!pings.wants(&failed) && pings.wants(&waiting) && pings.wants(&ask("Bash", "a")));
        assert!(!pings.wants(&Alert::MergeFailed { ticket: 1, conflict: false, project: None }));
        pings.set(Ping::Approvals, false);
        assert!(!pings.wants(&ask("Bash", "a")));
        pings.set(Ping::Sound, false);
        assert!(!pings.sound && pings.wants(&waiting), "the sound is not a kind of notice");
        pings.set(Ping::Merged, true);
        pings.set(Ping::Replies, false);
        assert!(pings.wants(&Alert::Merged { ticket: 1, title: String::new(), project: None }) && !pings.wants(&reply(1, "x")));
    }

    #[test]
    fn the_dock_shows_what_waits_unless_the_person_said_not_to() {
        let mut pings = Pings::default();
        assert_eq!(badge(3, &pings), Some(3));
        assert_eq!(badge(0, &pings), None, "nothing waits: no number");
        pings.set(Ping::Badge, false);
        assert_eq!(badge(3, &pings), None);
    }

    #[test]
    fn the_sheet_asks_only_when_macos_never_has_and_the_person_didnt_say_later() {
        assert!(explain_now(Permission::NotAsked, false));
        assert!(!explain_now(Permission::NotAsked, true));
        for p in [Permission::Unknown, Permission::Allowed, Permission::Denied, Permission::NoBundle] {
            assert!(!explain_now(p, false), "{p:?}");
        }
        assert_eq!((0..=5).map(Permission::from_status).collect::<Vec<_>>(), [Permission::NotAsked, Permission::Denied, Permission::Allowed, Permission::Allowed, Permission::Allowed, Permission::Unknown]);
    }

    #[test]
    fn each_version_is_told_once_found_and_once_downloaded() {
        use crate::relaunch::{Update, Why};
        let mut told = Vec::new();
        assert_eq!(update_alert(&Update::Found("0.49.0".into()), &mut told), Some(Alert::UpdateFound { version: "0.49.0".into() }));
        assert_eq!(update_alert(&Update::Downloading("0.49.0".into()), &mut told), None, "the same download");
        assert_eq!(update_alert(&Update::Ready("0.49.0".into()), &mut told), Some(Alert::Update { version: "0.49.0".into() }));
        assert_eq!(update_alert(&Update::Ready("0.49.0".into()), &mut told), None, "not again, even after a restart (told is kept)");
        assert_eq!(update_alert(&Update::Idle, &mut told), None);
        assert_eq!(update_alert(&Update::Failed { version: None, why: Why::Download }, &mut told), None, "a failure shows in the window, not as a notice");
        assert!(update_alert(&Update::Found("0.50.0".into()), &mut told).is_some());
        for v in ["0.51.0", "0.52.0", "0.53.0"] {
            update_alert(&Update::Ready(v.into()), &mut told);
        }
        assert_eq!(told.len(), 4, "only the last few are kept");
        let found = toasts(&[Alert::UpdateFound { version: "0.49.0".into() }], "acme", true);
        assert_eq!((found[0].title.as_str(), found[0].body.as_str(), found[0].go), ("equipe 0.49.0 나왔어요", "받는 중이에요 — 다 받으면 다시 알려 드릴게요", Go::Update));
        let ready = toasts(&[Alert::Update { version: "0.49.0".into() }], "acme", true);
        assert_eq!(found[0].request_id(1), ready[0].request_id(2), "downloaded replaces found in Notification Center");
        assert_eq!((found[0].category(), ready[0].category()), (CATEGORY_UPDATE, CATEGORY_UPDATE));
    }

    #[test]
    fn only_an_older_release_is_called_stale() {
        assert!(stale_daemon("0.3.0+17", "0.3.1").is_some());
        assert!(stale_daemon("0.3.1+17", "0.3.1").is_none());
        assert!(stale_daemon("0.4.0+1", "0.3.1").is_none());
        assert!(stale_daemon("test", "0.3.1").is_none());
    }
}
