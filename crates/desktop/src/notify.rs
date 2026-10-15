//! What the person is told when the window is out of sight (T-62 §4): a request
//! for permission, work to review, a ticket put down for them — who needs what,
//! never the command or the content, which would stay in the notification
//! centre. The wording and grouping are plain functions, tested here; only
//! `send` touches the system.

use equipe_core::t;
use equipe_core::protocol::{Approval, SessionId, TicketId};
use tokio::sync::mpsc::UnboundedSender;

use crate::views::approvals::noun;

/// More than this many at once become one notice.
const GROUP_AFTER: usize = 3;

/// Something that needs the person, as the model saw it happen.
#[derive(Debug, Clone, PartialEq)]
pub enum Alert {
    Approval { approval: Approval, squad: String },
    Review { ticket: TicketId, title: String, by: Option<String>, project: Option<String> },
    Waiting { ticket: TicketId, by: Option<String> },
    /// A member's turn failed: its work stopped until someone looks.
    Failed { session: SessionId, member: String },
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
            Alert::Waiting { .. } => self.waiting,
            Alert::Failed { .. } => self.failed,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub title: String,
    pub body: String,
    pub go: Go,
}

/// The notices for `alerts`, `squad`'s. `titles`: whether ticket titles may show.
pub fn toasts(alerts: &[Alert], squad: &str, titles: bool) -> Vec<Toast> {
    if alerts.len() > GROUP_AFTER {
        let approvals = alerts.iter().filter(|a| matches!(a, Alert::Approval { .. })).count();
        let reviews = alerts.iter().filter(|a| matches!(a, Alert::Review { .. })).count();
        let failed = alerts.iter().filter(|a| matches!(a, Alert::Failed { .. })).count();
        let waits = alerts.len() - approvals - reviews - failed;
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
        return vec![Toast { title: t!("notify.grouped_title", squad = squad, n = alerts.len()), body: parts.join(" · "), go: Go::Board }];
    }
    alerts
        .iter()
        .map(|alert| match alert {
            Alert::Approval { approval, squad } => {
                let ticket = approval.ticket.map(|t| format!(" · T-{t}")).unwrap_or_default();
                let send = approval.tool == equipe_core::desktop_gate::SEND;
                Toast {
                    title: t!("notify.approval_title", member = approval.member, what = noun(&approval.tool)),
                    body: format!("{squad}{ticket}{}", if send { t!("notify.send_waits") } else { "" }),
                    go: Go::Member(approval.session),
                }
            }
            Alert::Review { ticket, title, by, project } => {
                let who = by.as_deref().map(|b| t!("notify.submitted_by", b = b)).unwrap_or_else(|| t!("notify.submitted").into());
                let what = if titles { format!(" · {title}") } else { String::new() };
                let project = project.as_deref().map(|p| format!(" · {p}")).unwrap_or_default();
                Toast { title: t!("notify.review_title", ticket = ticket), body: format!("{who}{what}{project}"), go: Go::Ticket(*ticket) }
            }
            Alert::Waiting { ticket, by } => Toast {
                title: t!("notify.waiting_title", ticket = ticket),
                body: t!("notify.waiting_body", by = by.as_deref().map(|b| format!("{b} · ")).unwrap_or_default()),
                go: Go::Ticket(*ticket),
            },
            Alert::Failed { session, member } => Toast { title: t!("notify.failed_title", member = member), body: t!("notify.failed_body", squad = squad), go: Go::Member(*session) },
        })
        .collect()
}

/// A line to say when the daemon runs an older release than this app, else `None`.
/// Builds read `0.3.1+<seconds>`; only the release number is compared.
pub fn stale_daemon(daemon: &str, own: &str) -> Option<String> {
    let release = |v: &str| -> Option<Vec<u64>> { v.split('+').next()?.split('.').map(|n| n.parse().ok()).collect() };
    let (theirs, ours) = (release(daemon)?, release(own)?);
    (theirs < ours).then(|| t!("notify.stale_daemon", version = daemon.split('+').next().unwrap_or(daemon)))
}

/// Show `toast`; when it is clicked, say where on `clicked`. The system call
/// waits for the click, so it runs on a thread of its own. Where the system
/// refuses, the person still sees the badge and cards in the window.
/// `sound`: with the system's sound, or silently.
#[cfg(target_os = "macos")]
pub fn send(toast: Toast, clicked: UnboundedSender<Go>, sound: bool) {
    std::thread::spawn(move || {
        let mut notice = mac_notification_sys::Notification::new();
        notice.title(&toast.title).message(&toast.body).wait_for_click(true);
        if sound {
            notice.default_sound();
        }
        let sent = notice.send();
        if matches!(sent, Ok(mac_notification_sys::NotificationResponse::Click | mac_notification_sys::NotificationResponse::ActionButton(_))) {
            let _ = clicked.send(toast.go);
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn send(_: Toast, _: UnboundedSender<Go>, _: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval(tool: &str, action: &str) -> Approval {
        Approval { id: 1, session: 4, member: "backend".into(), ticket: Some(52), tool: tool.into(), action: action.into(), reason: None, preview: None }
    }

    fn ask(tool: &str, action: &str) -> Alert {
        Alert::Approval { approval: approval(tool, action), squad: "acme".into() }
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
    fn each_kind_of_notice_can_be_turned_off_on_its_own() {
        let mut pings = Pings::default();
        let failed = Alert::Failed { session: 6, member: "qa".into() };
        let waiting = Alert::Waiting { ticket: 3, by: None };
        assert!(Ping::ALL.iter().all(|p| pings.get(*p)), "everything, by default");
        pings.set(Ping::Failed, false);
        assert!(!pings.wants(&failed) && pings.wants(&waiting) && pings.wants(&ask("Bash", "a")));
        pings.set(Ping::Approvals, false);
        assert!(!pings.wants(&ask("Bash", "a")));
        pings.set(Ping::Sound, false);
        assert!(!pings.sound && pings.wants(&waiting), "the sound is not a kind of notice");
    }

    #[test]
    fn only_an_older_release_is_called_stale() {
        assert!(stale_daemon("0.3.0+17", "0.3.1").is_some());
        assert!(stale_daemon("0.3.1+17", "0.3.1").is_none());
        assert!(stale_daemon("0.4.0+1", "0.3.1").is_none());
        assert!(stale_daemon("test", "0.3.1").is_none());
    }
}
