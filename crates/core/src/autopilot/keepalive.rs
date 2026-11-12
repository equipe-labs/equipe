//! Keep-alive: for logins whose keep-alive the person turned on, equipe looks
//! at the site now and then — a page that shows whether it's signed in —
//! in a tab of its own behind the others, so an expired sign-in is caught
//! and signed back in (password, code, passkey: whatever the person let it
//! do) before a member needs the site. Gently: hours apart, each site at
//! its own jittered time, one look every few minutes at most across every
//! site, none for a login stopped after a failed sign-in.

use crate::protocol::vault::Login;

/// Hours between two looks at one site, unless the person says otherwise.
pub const DEFAULT_HOURS: u32 = 6;
/// At most one look this often, across every site.
pub const GAP_SECS: u64 = 3 * 60;

/// A small, steady hash: each site's own jitter, the same every run.
fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

/// Seconds between two looks at `login`, `hours` apart give or take a fifth
/// (its own share of the jitter, changing with each look).
pub fn interval(login: &Login, hours: u32, looks: u64) -> u64 {
    let base = u64::from(hours.max(1)) * 3600;
    let spread = base / 5;
    let j = mix(u64::from(login.id) << 32 | looks) % (2 * spread + 1);
    base - spread + j
}

/// The login to look at now, if any: keep-alive on, signing in by itself on
/// and not stopped, its last look (or its saving) longer ago than its
/// interval, and the last look at any site at least [`GAP_SECS`] ago. The
/// one most overdue first.
pub fn due(logins: &[Login], hours: u32, now: u64, last_any: u64) -> Option<&Login> {
    if hours == 0 || now.saturating_sub(last_any) < GAP_SECS {
        return None;
    }
    logins
        .iter()
        .filter(|l| l.autopilot.keepalive && l.auto && l.stopped.is_none())
        .filter_map(|l| {
            let last = l.autopilot.checked.as_ref().map_or(l.created_at, |c| c.at);
            let looks = l.autopilot.checked.as_ref().map_or(0, |c| c.at / 3600);
            let wait = interval(l, hours, looks);
            let late = now.saturating_sub(last).checked_sub(wait)?;
            Some((late, l))
        })
        .max_by_key(|(late, l)| (*late, std::cmp::Reverse(l.id)))
        .map(|(_, l)| l)
}

/// The page keep-alive looks at for `login`: the one the person named, else
/// the site's home (which shows a signed-in page, or sends a signed-out
/// browser to its sign-in).
pub fn check_url(login: &Login) -> String {
    login.autopilot.check_url.clone().filter(|u| u.starts_with("https://")).unwrap_or_else(|| format!("https://{}/", login.site))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::vault::{Autopilot, Use};

    fn login(id: u32, keepalive: bool, checked: Option<u64>) -> Login {
        Login {
            id,
            site: "github.com".into(),
            url: None,
            username: "a".into(),
            note: None,
            allowed: Vec::new(),
            auto: true,
            stopped: None,
            created_at: 0,
            uses: Vec::new(),
            autopilot: Autopilot { keepalive, checked: checked.map(|at| Use { at, squad: String::new(), member: String::new(), outcome: "signed_in".into() }), ..Autopilot::default() },
        }
    }

    #[test]
    fn a_site_is_looked_at_hours_apart_jittered_and_never_two_at_once() {
        let l = login(1, true, Some(1_000_000));
        let i = interval(&l, 6, 0);
        assert!((6 * 3600 * 4 / 5..=6 * 3600 * 6 / 5).contains(&i), "{i}");
        assert_ne!(interval(&l, 6, 0), interval(&login(2, true, None), 6, 0), "each site its own time");
        let logins = [l.clone()];
        assert!(due(&logins, 6, 1_000_000 + 3600, 0).is_none(), "not yet");
        assert_eq!(due(&logins, 6, 1_000_000 + 8 * 3600, 0).map(|l| l.id), Some(1));
        assert!(due(&logins, 6, 1_000_000 + 8 * 3600, 1_000_000 + 8 * 3600 - 60).is_none(), "another look a minute ago");
        assert!(due(&logins, 0, 1_000_000 + 8 * 3600, 0).is_none(), "the person turned it off");
        assert!(due(&[login(1, false, None)], 6, 9_999_999, 0).is_none(), "off for that login");
        let mut stopped = login(1, true, None);
        stopped.stopped = Some("form came back".into());
        assert!(due(&[stopped], 6, 9_999_999, 0).is_none(), "stopped after a failed sign-in");
        assert_eq!(check_url(&l), "https://github.com/");
    }
}
