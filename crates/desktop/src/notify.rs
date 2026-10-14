//! What the person is told when the window is out of sight (T-62 §4): a request
//! for permission, work to review, a ticket put down for them — who needs what,
//! never the command or the content, which would stay in the notification
//! centre. The wording and grouping are plain functions, tested here; only
//! `send` touches the system.

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
        let waits = alerts.len() - approvals - reviews;
        let parts: Vec<String> = [("승인", approvals), ("리뷰", reviews), ("사람 대기", waits)]
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .map(|(what, n)| format!("{what} {n}"))
            .collect();
        return vec![Toast { title: format!("{squad}에서 {}건이 당신을 기다립니다", alerts.len()), body: parts.join(" · "), go: Go::Board }];
    }
    alerts
        .iter()
        .map(|alert| match alert {
            Alert::Approval { approval, squad } => {
                let ticket = approval.ticket.map(|t| format!(" · T-{t}")).unwrap_or_default();
                let send = approval.tool == equipe_core::desktop_gate::SEND;
                Toast {
                    title: format!("{}: {}을 요청합니다", approval.member, noun(&approval.tool)),
                    body: format!("{squad}{ticket}{}", if send { " · 전송은 사람의 허락을 기다립니다" } else { "" }),
                    go: Go::Member(approval.session),
                }
            }
            Alert::Review { ticket, title, by, project } => {
                let who = by.as_deref().map(|b| format!("{b} 제출")).unwrap_or_else(|| "제출됨".into());
                let what = if titles { format!(" · {title}") } else { String::new() };
                let project = project.as_deref().map(|p| format!(" · {p}")).unwrap_or_default();
                Toast { title: format!("T-{ticket} 검토를 기다립니다"), body: format!("{who}{what}{project}"), go: Go::Ticket(*ticket) }
            }
            Alert::Waiting { ticket, by } => Toast {
                title: format!("T-{ticket} · 당신을 기다립니다"),
                body: format!("{}사람만 할 수 있는 일이 남았습니다", by.as_deref().map(|b| format!("{b} · ")).unwrap_or_default()),
                go: Go::Ticket(*ticket),
            },
        })
        .collect()
}

/// A line to say when the daemon runs an older release than this app, else `None`.
/// Builds read `0.3.1+<seconds>`; only the release number is compared.
pub fn stale_daemon(daemon: &str, own: &str) -> Option<String> {
    let release = |v: &str| -> Option<Vec<u64>> { v.split('+').next()?.split('.').map(|n| n.parse().ok()).collect() };
    let (theirs, ours) = (release(daemon)?, release(own)?);
    (theirs < ours).then(|| format!("데몬이 더 오래된 버전({})으로 실행 중입니다 — 멤버가 쉴 때 `equipe daemon restart`로 다시 시작하세요", daemon.split('+').next().unwrap_or(daemon)))
}

/// Show `toast`; when it is clicked, say where on `clicked`. The system call
/// waits for the click, so it runs on a thread of its own. Where the system
/// refuses, the person still sees the badge and cards in the window.
#[cfg(target_os = "macos")]
pub fn send(toast: Toast, clicked: UnboundedSender<Go>) {
    std::thread::spawn(move || {
        let sent = mac_notification_sys::Notification::new().title(&toast.title).message(&toast.body).default_sound().wait_for_click(true).send();
        if matches!(sent, Ok(mac_notification_sys::NotificationResponse::Click | mac_notification_sys::NotificationResponse::ActionButton(_))) {
            let _ = clicked.send(toast.go);
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn send(_: Toast, _: UnboundedSender<Go>) {}

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
    fn only_an_older_release_is_called_stale() {
        assert!(stale_daemon("0.3.0+17", "0.3.1").is_some());
        assert!(stale_daemon("0.3.1+17", "0.3.1").is_none());
        assert!(stale_daemon("0.4.0+1", "0.3.1").is_none());
        assert!(stale_daemon("test", "0.3.1").is_none());
    }
}
