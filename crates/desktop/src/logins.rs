//! The Logins page's logic, with no drawing (`views/logins.rs` draws it): which
//! logins show for a search, how a login's scope reads, where the sites that
//! ask again most could be helped, which sites members wait on now, and the
//! trend's bars. Everything here reads what screens already hold — logins
//! without passwords (`protocol::vault`), the record's counts
//! (`LoginInsights`), the browser waits — so it can be tested with plain
//! values: no Keychain, no daemon.

use std::time::{Duration, Instant};

use equipe_core::protocol::HumanCheck;
use equipe_core::protocol::vault::{EVERY_SQUAD, Login, LoginInsights};
use equipe_core::vault::serves;

use crate::state::BrowserWait;

/// The look-backs the page offers, in days.
pub const RANGES: [u32; 2] = [7, 30];

/// How long a delete can be taken back before it is sent to the daemon.
pub const UNDO_FOR: Duration = Duration::from_secs(8);

/// Where members may use a login without asking, as the row's chip says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Every squad.
    Every,
    /// Only the squad in view.
    ThisSquad,
    /// One other squad, by name.
    Squad(String),
    /// Several squads.
    Squads(usize),
    /// Nowhere yet: each use asks the person.
    AskFirst,
}

/// What a scope picker offers: the daemon's `SetLoginScope` squads for each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    AskFirst,
    ThisSquad,
    Every,
}

impl Choice {
    pub const ALL: [Choice; 3] = [Choice::AskFirst, Choice::ThisSquad, Choice::Every];

    /// The squads to send for this choice, from the squad in view.
    pub fn squads(self, squad: &str) -> Vec<String> {
        match self {
            Choice::AskFirst => Vec::new(),
            Choice::ThisSquad => vec![squad.to_owned()],
            Choice::Every => vec![EVERY_SQUAD.to_owned()],
        }
    }

    /// Whether `login`'s scope is this choice, seen from `squad`.
    pub fn is(self, login: &Login, squad: &str) -> bool {
        match (self, scope_of(login, squad)) {
            (Choice::AskFirst, Scope::AskFirst) | (Choice::ThisSquad, Scope::ThisSquad) | (Choice::Every, Scope::Every) => true,
            _ => false,
        }
    }
}

/// `login`'s scope, seen from the squad in view. Entries from before
/// squad-wide approvals (`squad/member`) read as their squad.
pub fn scope_of(login: &Login, squad: &str) -> Scope {
    if login.everywhere() {
        return Scope::Every;
    }
    let mut squads: Vec<String> = Vec::new();
    for who in &login.allowed {
        let one = equipe_core::vault::scope_of(who);
        if !one.is_empty() && !squads.contains(&one) {
            squads.push(one);
        }
    }
    match squads.as_slice() {
        [] => Scope::AskFirst,
        [one] if one == squad => Scope::ThisSquad,
        [one] => Scope::Squad(one.clone()),
        many => Scope::Squads(many.len()),
    }
}

/// The logins a search shows: those whose site, username, note or sign-in
/// page contain every word typed, by site then username. `hidden` ids (a
/// delete that can still be taken back) are left out.
pub fn visible<'a>(logins: &'a [Login], query: &str, hidden: &[u32]) -> Vec<&'a Login> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut shown: Vec<&Login> = logins
        .iter()
        .filter(|l| !hidden.contains(&l.id))
        .filter(|l| {
            let hay = format!("{} {} {} {}", l.site, l.username, l.note.as_deref().unwrap_or_default(), l.url.as_deref().unwrap_or_default()).to_lowercase();
            words.iter().all(|w| hay.contains(w))
        })
        .collect();
    shown.sort_by(|a, b| a.site.cmp(&b.site).then(a.username.to_lowercase().cmp(&b.username.to_lowercase())).then(a.id.cmp(&b.id)));
    shown
}

/// The one letter a site's tile shows: its first letter, upper-cased. (No
/// icon is fetched from the site: opening the page must not tell anyone
/// which sites the person has logins for.)
pub fn letter(site: &str) -> String {
    site.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".to_owned())
}

/// A tile's colour slot (0–7, the palette's member hues), steady for a site.
pub fn tint(site: &str) -> usize {
    // FNV-1a: the same site always gets the same colour, run after run.
    let mut h: u32 = 0x811c_9dc5;
    for b in site.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    (h % 8) as usize
}

/// What the person can do about a site that keeps asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// No login is kept for it: save one.
    SaveLogin,
    /// A login is kept, but equipe doesn't sign in with it by itself.
    TurnOnAuto { id: u32 },
    /// Signing in by itself is on, but the password stopped working.
    ReplacePassword { id: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub site: String,
    /// How many times it asked the person to sign in in the period.
    pub asked: u64,
    pub fix: Fix,
}

/// Sign-ins asked for: the causes a kept login could have saved.
pub fn sign_in_asks(by_cause: &[equipe_core::protocol::vault::CauseCount]) -> u64 {
    by_cause.iter().filter(|c| matches!(c.cause.as_str(), "session_expired" | "no_login_kept")).map(|c| c.count).sum()
}

/// The sites that asked for a sign-in most, each with the one step that would
/// help — at most `limit`. A site that is fixed (a login kept and signing in
/// by itself, working), one the person said never to, and a machine with no
/// secret store to keep a password in, get none.
pub fn suggestions(insights: &LoginInsights, logins: &[Login], limit: usize) -> Vec<Suggestion> {
    let mut found = Vec::new();
    for site in &insights.sites {
        let asked = sign_in_asks(&site.by_cause);
        if asked == 0 {
            continue;
        }
        let kept: Vec<&Login> = logins.iter().filter(|l| serves(&l.site, &site.site)).collect();
        let fix = match kept.as_slice() {
            [] if insights.can_keep && !insights.never.iter().any(|n| serves(n, &site.site)) => Some(Fix::SaveLogin),
            [] => None,
            // Two accounts for a site: signing in by itself is the person's choice, not a suggestion.
            [one] if one.auto && one.health() == equipe_core::protocol::vault::Health::Failed => Some(Fix::ReplacePassword { id: one.id }),
            [one] if !one.auto => Some(Fix::TurnOnAuto { id: one.id }),
            _ => None,
        };
        if let Some(fix) = fix {
            found.push(Suggestion { site: site.site.clone(), asked, fix });
        }
        if found.len() == limit {
            break;
        }
    }
    found
}

/// How long a sign-in usually lasts before a site asks again: the median of
/// the sites' own medians, seconds. `None` when no site asked again after one.
pub fn typical_reask(insights: &LoginInsights) -> Option<u64> {
    let mut spans: Vec<u64> = insights.sites.iter().filter_map(|s| s.reask_after).collect();
    spans.sort_unstable();
    spans.get(spans.len() / 2).copied()
}

/// A site members wait on the person to sign in to, now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingSite {
    pub site: String,
    /// The members waiting, once each.
    pub members: Vec<String>,
    /// A page to open the browser on.
    pub page: u64,
    /// Whether a login is kept for it already.
    pub kept: bool,
}

/// Sites waiting on a sign-in, one row each however many members wait on it
/// (the daemon joins them into one card), in the order they began.
pub fn waiting(waits: &[BrowserWait], logins: &[Login]) -> Vec<WaitingSite> {
    let mut out: Vec<WaitingSite> = Vec::new();
    for w in waits.iter().filter(|w| w.check == Some(HumanCheck::SignIn) && !w.site.is_empty()) {
        match out.iter_mut().find(|s| s.site == w.site) {
            Some(one) => {
                if !one.members.contains(&w.member) {
                    one.members.push(w.member.clone());
                }
            }
            None => out.push(WaitingSite { site: w.site.clone(), members: vec![w.member.clone()], page: w.page, kept: logins.iter().any(|l| serves(&l.site, &w.site)) }),
        }
    }
    out
}

/// Bar heights for the trend, 0..=1 of the tallest; a day with no asks is a
/// hairline (`floor`), so the row of days stays readable.
pub fn bars(trend: &[u64], floor: f32) -> Vec<f32> {
    let top = trend.iter().copied().max().unwrap_or(0);
    trend.iter().map(|n| if top == 0 || *n == 0 { floor } else { (*n as f32 / top as f32).max(floor) }).collect()
}

/// A delete that can still be taken back: the login stays in the daemon until
/// the time is up — nothing is sent, so nothing can be lost to a quit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDelete {
    pub id: u32,
    pub site: String,
    pub until: Instant,
}

impl PendingDelete {
    pub fn new(login: &Login, now: Instant) -> Self {
        Self { id: login.id, site: login.site.clone(), until: now + UNDO_FOR }
    }

    pub fn due(&self, now: Instant) -> bool {
        now >= self.until
    }
}

/// The address to send for a sign-in page: what was typed, or the site when
/// the box is empty — the daemon reads a host as the site and a longer
/// address as the sign-in page.
pub fn draft_site(site: &str, url: &str) -> String {
    let url = url.trim();
    if url.is_empty() { site.trim().to_owned() } else { url.to_owned() }
}

/// What a form can say is wrong with what was typed, before anything is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    NeedSite,
    NeedUsername,
    NeedPassword,
}

/// Checks for a new login: a site, a username, a password.
pub fn check_new(site: &str, username: &str, password: &str) -> Option<Problem> {
    if site.trim().is_empty() || equipe_core::vault::site_of(site).is_empty() {
        Some(Problem::NeedSite)
    } else if username.trim().is_empty() {
        Some(Problem::NeedUsername)
    } else if password.is_empty() {
        Some(Problem::NeedPassword)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::vault::{CauseCount, SiteAsks, Use};

    fn login(id: u32, site: &str, user: &str) -> Login {
        Login { id, site: site.into(), url: None, username: user.into(), note: None, allowed: Vec::new(), auto: false, stopped: None, created_at: 0, password_at: None, uses: Vec::new() }
    }

    fn wait(page: u64, member: &str, site: &str, check: Option<HumanCheck>) -> BrowserWait {
        BrowserWait { page, lane: format!("acme/{member}"), session: None, member: member.into(), site: site.into(), check, why: None }
    }

    fn asks(site: &str, session_expired: u64, captcha: u64) -> SiteAsks {
        SiteAsks {
            site: site.into(),
            asked: session_expired + captcha,
            by_cause: vec![CauseCount { cause: "session_expired".into(), count: session_expired }, CauseCount { cause: "captcha".into(), count: captcha }],
            reask_after: None,
            signed_in: 0,
            signed_in_by_equipe: 0,
        }
    }

    #[test]
    fn a_scope_reads_from_the_squad_in_view() {
        let mut l = login(1, "github.com", "abel");
        assert_eq!(scope_of(&l, "acme"), Scope::AskFirst);
        l.allowed = vec!["acme".into()];
        assert_eq!(scope_of(&l, "acme"), Scope::ThisSquad);
        assert_eq!(scope_of(&l, "shop"), Scope::Squad("acme".into()));
        l.allowed = vec!["acme/qa".into(), "acme".into(), "shop".into()];
        assert_eq!(scope_of(&l, "acme"), Scope::Squads(2), "an old squad/member entry is its squad, once");
        l.allowed.push("*".into());
        assert_eq!(scope_of(&l, "acme"), Scope::Every);
        assert!(Choice::Every.is(&l, "acme") && !Choice::ThisSquad.is(&l, "acme"));
    }

    #[test]
    fn each_choice_sends_its_squads() {
        assert!(Choice::AskFirst.squads("acme").is_empty());
        assert_eq!(Choice::ThisSquad.squads("acme"), ["acme"]);
        assert_eq!(Choice::Every.squads("acme"), ["*"]);
    }

    #[test]
    fn a_search_matches_every_word_anywhere_and_leaves_out_a_delete_under_way() {
        let mut a = login(1, "github.com", "Abel");
        a.note = Some("work account".into());
        let mut b = login(2, "erp.example.com", "pm@example.com");
        b.url = Some("https://erp.example.com/sso/login".into());
        let c = login(3, "github.com", "another");
        let all = [a, b, c];
        assert_eq!(visible(&all, "", &[]).iter().map(|l| l.id).collect::<Vec<_>>(), [2, 1, 3], "by site, then username");
        assert_eq!(visible(&all, "GITHUB work", &[]).iter().map(|l| l.id).collect::<Vec<_>>(), [1]);
        assert_eq!(visible(&all, "sso", &[]).iter().map(|l| l.id).collect::<Vec<_>>(), [2], "its sign-in page counts");
        assert!(visible(&all, "nothing like it", &[]).is_empty());
        assert_eq!(visible(&all, "github", &[3]).iter().map(|l| l.id).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn a_tile_has_a_letter_and_a_steady_colour() {
        assert_eq!(letter("github.com"), "G");
        assert_eq!(letter("1password.com"), "1");
        assert_eq!(letter("..."), "?");
        assert_eq!(tint("github.com"), tint("github.com"));
        assert!((0..8).contains(&tint("erp.example.com")));
    }

    #[test]
    fn the_sites_that_ask_most_get_the_step_that_helps() {
        let insights = LoginInsights {
            days: 7,
            can_keep: true,
            never: vec!["slack.com".into()],
            sites: vec![asks("github.com", 6, 0), asks("slack.com", 5, 0), asks("erp.example.com", 4, 1), asks("bank.example.com", 0, 9), asks("mail.example.com", 3, 0), asks("docs.example.com", 2, 0)],
            ..Default::default()
        };
        let mut kept_off = login(1, "erp.example.com", "pm");
        kept_off.auto = false;
        let mut failing = login(2, "mail.example.com", "pm");
        failing.auto = true;
        failing.stopped = Some("form came back".into());
        failing.uses = vec![Use { at: 5, squad: "acme".into(), member: "qa".into(), outcome: "failed".into() }];
        let mut fine = login(3, "docs.example.com", "pm");
        fine.auto = true;
        let found = suggestions(&insights, &[kept_off, failing, fine], 10);
        assert_eq!(
            found,
            [
                Suggestion { site: "github.com".into(), asked: 6, fix: Fix::SaveLogin },
                Suggestion { site: "erp.example.com".into(), asked: 4, fix: Fix::TurnOnAuto { id: 1 } },
                Suggestion { site: "mail.example.com".into(), asked: 3, fix: Fix::ReplacePassword { id: 2 } },
            ],
            "never-for-this-site, a CAPTCHA-only site and a login that already works get nothing"
        );
        assert_eq!(suggestions(&insights, &[], 1).len(), 1, "limited");
        // No secret store here: nothing to save a login into.
        assert!(suggestions(&LoginInsights { can_keep: false, ..insights }, &[], 10).is_empty());
    }

    #[test]
    fn one_row_per_site_however_many_members_wait() {
        let logins = [login(1, "github.com", "abel")];
        let waits = [
            wait(4, "qa", "github.com", Some(HumanCheck::SignIn)),
            wait(5, "backend", "github.com", Some(HumanCheck::SignIn)),
            wait(6, "backend", "github.com", Some(HumanCheck::SignIn)),
            wait(7, "pm", "erp.example.com", Some(HumanCheck::SignIn)),
            wait(8, "pm", "bank.example.com", Some(HumanCheck::Captcha)),
            wait(9, "pm", "code.example.com", Some(HumanCheck::TwoFactor)),
            wait(10, "pm", "", Some(HumanCheck::SignIn)),
        ];
        assert_eq!(
            waiting(&waits, &logins),
            [
                WaitingSite { site: "github.com".into(), members: vec!["qa".into(), "backend".into()], page: 4, kept: true },
                WaitingSite { site: "erp.example.com".into(), members: vec!["pm".into()], page: 7, kept: false },
            ]
        );
        assert!(waiting(&[], &logins).is_empty());
    }

    #[test]
    fn the_typical_time_to_ask_again_is_the_middle_of_the_sites() {
        let mut insights = LoginInsights { sites: vec![asks("a.com", 1, 0), asks("b.com", 1, 0), asks("c.com", 1, 0)], ..Default::default() };
        assert_eq!(typical_reask(&insights), None);
        for (site, secs) in insights.sites.iter_mut().zip([7_200, 86_400, 3_600]) {
            site.reask_after = Some(secs);
        }
        assert_eq!(typical_reask(&insights), Some(7_200));
    }

    #[test]
    fn the_trend_is_scaled_to_its_tallest_day_with_a_hairline_for_none() {
        assert_eq!(bars(&[0, 2, 4, 1], 0.05), [0.05, 0.5, 1.0, 0.25]);
        assert_eq!(bars(&[0, 0], 0.05), [0.05, 0.05]);
        assert!(bars(&[], 0.05).is_empty());
    }

    #[test]
    fn a_delete_waits_out_its_undo_time_before_it_is_due() {
        let now = Instant::now();
        let pending = PendingDelete::new(&login(1, "github.com", "abel"), now);
        assert!(!pending.due(now) && !pending.due(now + UNDO_FOR - Duration::from_millis(1)));
        assert!(pending.due(now + UNDO_FOR));
    }

    #[test]
    fn a_login_page_address_wins_over_the_site_and_new_logins_are_checked() {
        assert_eq!(draft_site("github.com", ""), "github.com");
        assert_eq!(draft_site("github.com", " https://github.com/login "), "https://github.com/login");
        assert_eq!(check_new("", "abel", "pw"), Some(Problem::NeedSite));
        assert_eq!(check_new("https://", "abel", "pw"), Some(Problem::NeedSite));
        assert_eq!(check_new("github.com", " ", "pw"), Some(Problem::NeedUsername));
        assert_eq!(check_new("github.com", "abel", ""), Some(Problem::NeedPassword));
        assert_eq!(check_new("github.com", "abel", "pw"), None);
    }
}
