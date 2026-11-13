//! The sidebar's conversations in the days they were last used, as the Claude
//! app's history groups them: Today, Yesterday, Previous 7 days, Older — by
//! the person's own midnight, from each conversation's last activity.

use std::ops::Range;

use crate::state::Lane;

const DAY: u64 = 24 * 60 * 60;

/// How long ago a conversation was last used, in the person's days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Day {
    Today,
    Yesterday,
    /// Before yesterday, within the 7 days before today.
    Week,
    Older,
}

impl Day {
    pub fn label_key(self) -> &'static str {
        match self {
            Day::Today => "sidebar.day_today",
            Day::Yesterday => "sidebar.day_yesterday",
            Day::Week => "sidebar.day_week",
            Day::Older => "sidebar.day_older",
        }
    }
}

/// The day `active` (unix seconds) falls in, as of `now`. `midnight` gives the
/// start of the local day a moment is in. A conversation whose activity isn't
/// known (0) is Older.
pub fn day_of(active: u64, now: u64, midnight: impl Fn(u64) -> u64) -> Day {
    if active == 0 {
        return Day::Older;
    }
    let today = midnight(now);
    // A day's start is found from the one before, so a 23- or 25-hour day is right.
    let yesterday = midnight(today.saturating_sub(1));
    let week = midnight(today.saturating_sub(7 * DAY).saturating_add(DAY / 2));
    if active >= today {
        Day::Today
    } else if active >= yesterday {
        Day::Yesterday
    } else if active >= week {
        Day::Week
    } else {
        Day::Older
    }
}

/// The first `shown` conversations (newest first) as runs of one day each:
/// `(day, the range of `talks` it covers)`. The cap counts across the days,
/// so a day may show only some of its conversations.
pub fn groups(talks: &[&Lane], shown: usize, now: u64, midnight: impl Fn(u64) -> u64) -> Vec<(Day, Range<usize>)> {
    let shown = shown.min(talks.len());
    let mut out: Vec<(Day, Range<usize>)> = Vec::new();
    for (i, lane) in talks[..shown].iter().enumerate() {
        let day = day_of(lane.active, now, &midnight);
        match out.last_mut() {
            Some((last, range)) if *last == day => range.end = i + 1,
            _ => out.push((day, i..i + 1)),
        }
    }
    out
}

/// The conversations whose title contains `filter`, in any case, in order. An
/// empty filter keeps all. One without a title is matched by `untitled`, the
/// words the list shows for it.
pub fn matching<'a>(talks: &[&'a Lane], filter: &str, untitled: &str) -> Vec<&'a Lane> {
    let needle = filter.trim().to_lowercase();
    talks
        .iter()
        .copied()
        .filter(|lane| needle.is_empty() || lane.title.as_deref().unwrap_or(untitled).to_lowercase().contains(&needle))
        .collect()
}

/// How many conversations equipe put away by itself on the day of `now`.
pub fn put_away_today(put_away: &[equipe_core::protocol::PutAway], now: u64, midnight: impl Fn(u64) -> u64) -> usize {
    let today = midnight(now);
    put_away.iter().filter(|c| c.auto && c.at > 0 && c.at as u64 >= today).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Presence, Verb};

    /// Midnight at UTC.
    fn utc(unix: u64) -> u64 {
        unix - unix % DAY
    }

    /// A day's 15:00.
    const NOW: u64 = 20_400 * DAY + 15 * 3600;

    fn lane(session: u64, active: u64, title: Option<&str>) -> Lane {
        Lane { session, of: 1, project: None, presence: Presence::Ready, verb: Verb::Thinking, kept: true, title: title.map(str::to_owned), active, pinned: false }
    }

    #[test]
    fn a_conversation_is_grouped_by_the_day_it_was_last_used() {
        let day = |ago: u64| day_of(NOW - ago, NOW, utc);
        assert_eq!(day(0), Day::Today);
        assert_eq!(day(15 * 3600), Day::Today, "today's first second");
        assert_eq!(day(15 * 3600 + 1), Day::Yesterday, "the last second of yesterday");
        assert_eq!(day(15 * 3600 + DAY), Day::Yesterday, "yesterday's first second");
        assert_eq!(day(15 * 3600 + DAY + 1), Day::Week);
        assert_eq!(day(15 * 3600 + 7 * DAY), Day::Week, "seven days before today still counts");
        assert_eq!(day(15 * 3600 + 7 * DAY + 1), Day::Older);
        assert_eq!(day_of(0, NOW, utc), Day::Older, "when it was last used isn't known");
    }

    #[test]
    fn the_days_follow_the_persons_midnight() {
        // Midnight where the day starts at 05:00 UTC.
        let there = |unix: u64| (unix + DAY - 5 * 3600) / DAY * DAY - DAY + 5 * 3600;
        let early = utc(NOW) + 3 * 3600;
        let now = utc(NOW) + 6 * 3600;
        assert_eq!(day_of(early, now, there), Day::Yesterday, "03:00 UTC is still yesterday there");
        assert_eq!(day_of(early, now, utc), Day::Today);
    }

    #[test]
    fn the_cap_counts_across_the_days_and_show_all_gives_the_rest_grouped_the_same_way() {
        // 25 conversations over 12 days, newest first, two a day.
        let talks: Vec<Lane> = (0..25).map(|i| lane(100 - i, NOW - i * (DAY / 2) - 60, None)).collect();
        let refs: Vec<&Lane> = talks.iter().collect();
        let capped = groups(&refs, 10, NOW, utc);
        assert_eq!(capped.iter().map(|(_, r)| r.len()).sum::<usize>(), 10, "ten rows in all");
        assert_eq!(capped.first().map(|g| g.0), Some(Day::Today));
        assert_eq!(capped.last().map(|g| g.1.end), Some(10));
        let all = groups(&refs, usize::MAX, NOW, utc);
        assert_eq!(all.iter().map(|(_, r)| r.len()).sum::<usize>(), 25);
        assert_eq!(all.iter().map(|g| g.0).collect::<Vec<_>>(), [Day::Today, Day::Yesterday, Day::Week, Day::Older], "each day once, in order");
        assert!(capped.iter().zip(&all).all(|(a, b)| a.0 == b.0 && a.1.start == b.1.start), "the first ten are the first ten of all");
    }

    #[test]
    fn the_filter_matches_titles_in_any_case_and_an_untitled_one_by_its_placeholder() {
        let talks = [lane(1, NOW, Some("Pricing page")), lane(2, NOW, Some("Login bug")), lane(3, NOW, None)];
        let refs: Vec<&Lane> = talks.iter().collect();
        let sessions = |filter| matching(&refs, filter, "New conversation").iter().map(|l| l.session).collect::<Vec<_>>();
        assert_eq!(sessions("LOG"), [2]);
        assert_eq!(sessions("  page "), [1]);
        assert_eq!(sessions("new"), [3]);
        assert_eq!(sessions(""), [1, 2, 3]);
        assert!(sessions("nothing").is_empty());
    }

    #[test]
    fn the_note_counts_what_equipe_put_away_today_and_not_what_the_person_did_or_earlier_days() {
        let away = |auto, at: u64| equipe_core::protocol::PutAway { session: 1, project: None, title: None, at: at as i64, auto };
        let list = [away(true, NOW - 3600), away(true, utc(NOW)), away(false, NOW - 60), away(true, utc(NOW) - 1), away(true, 0)];
        assert_eq!(put_away_today(&list, NOW, utc), 2);
        assert_eq!(put_away_today(&[], NOW, utc), 0);
    }
}
