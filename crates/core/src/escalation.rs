//! What stays a bottleneck gets louder, the way a good human assistant would
//! raise it — without ever waking the person.
//!
//! Every thing that waits on the person has an importance ([`crate::importance`]).
//! How long it has held work up, against that, sets its **level**:
//!
//! | level | when (an item holding urgent work or a deploy; others wait three times as long) | what |
//! |---|---|---|
//! | 1 | until [`NotifySettings::level_mins`]`[0]` (2 h) | only in Needs you |
//! | 2 | 2 h | one notification |
//! | 3 | 8 h | one more, and the PM brings it up first in conversation |
//! | 4 | 24 h | the daily summary: "these 2 items have blocked 9 tickets for 18 h" |
//!
//! Around the levels, the rules that keep it kind:
//!
//! - **Quiet hours** (default 23:00–08:00 in the person's own time zone):
//!   nothing goes out; what came due goes out at the end as *one* summary.
//! - **No spam**: one notification per item per level, at most
//!   [`NotifySettings::max_per_hour`] an hour in all (what is held back waits
//!   for the next minute that is allowed, and goes out as one).
//! - **Missed**: when an item has been told [`MISSED_AT`] times and still waits,
//!   the person is told so, plainly, once.
//!
//! [`Planner::step`] is pure over its state: items and the clock in, notices
//! out, so every rule is tested without a clock or a screen. The daemon calls
//! it each minute per squad and publishes what comes out (`Event::Escalated`).

use std::collections::{HashMap, VecDeque};

use equipe_protocol::importance::{Impact, Notice, NoticeItem, NoticeKind, NotifySettings, Only, Waiting};
use equipe_protocol::TicketId;

const HOUR: u64 = 3600;
const DAY: u64 = 24 * HOUR;

/// What waits on the person that holds up neither urgent work nor a deploy
/// waits this many times as long for each level.
pub const WEAK_FACTOR: u64 = 3;
/// How often the daily summary comes, at most.
pub const DAILY_EVERY: u64 = DAY;
/// Notices about one item, all unanswered, before the person is told they were missed.
pub const MISSED_AT: u32 = 3;
/// The highest level that has a notification of its own; level 4 is the daily summary.
const LAST_OWN_LEVEL: u8 = 3;

/// Whether `impact` may escalate under `s` at all.
pub fn eligible(s: &NotifySettings, impact: &Impact) -> bool {
    s.on && impact.is_blocking() && (s.only == Only::Blocking || impact.is_pressing() || impact.holds_deploy)
}

/// The level (1–4) of an item that has held `impact` up for `waited` seconds.
pub fn level(s: &NotifySettings, impact: &Impact, waited: u64) -> u8 {
    if !eligible(s, impact) {
        return 1;
    }
    let strong = impact.is_pressing() || impact.holds_deploy;
    let factor = if strong { 1 } else { WEAK_FACTOR };
    let after = |i: usize| u64::from(s.level_mins[i]) * 60 * factor;
    match waited {
        w if w >= after(2) => 4,
        w if w >= after(1) => 3,
        w if w >= after(0) => 2,
        _ => 1,
    }
}

/// The offset of the machine's own time zone from UTC at `now`, in seconds.
pub fn local_utc_offset(now: u64) -> i32 {
    let at = now as libc::time_t;
    // SAFETY: `tm` is plain data that `localtime_r` fills in; both pointers are valid for the call.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&at, &mut tm).is_null() { 0 } else { tm.tm_gmtoff as i32 }
    }
}

/// The minute of the day (0–1439) at `now` for someone `offset` seconds from UTC.
pub fn local_minute(now: u64, offset: i32) -> u16 {
    ((now as i64 + i64::from(offset)).rem_euclid(DAY as i64) / 60) as u16
}

/// Whether `now` falls in the person's quiet hours. The window may cross
/// midnight (23:00–08:00); equal ends mean no window.
pub fn is_quiet(s: &NotifySettings, now: u64) -> bool {
    if !s.quiet_on || s.quiet_from == s.quiet_to {
        return false;
    }
    let m = local_minute(now, s.utc_offset_secs.unwrap_or_else(|| local_utc_offset(now)));
    if s.quiet_from < s.quiet_to { (s.quiet_from..s.quiet_to).contains(&m) } else { m >= s.quiet_from || m < s.quiet_to }
}

/// `HH:MM` as minutes after midnight (`"9"` and `"9:30"` too).
pub fn parse_clock(text: &str) -> Option<u16> {
    let text = text.trim();
    let (h, m) = text.split_once(':').unwrap_or((text, "0"));
    let (h, m): (u16, u16) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Minutes after midnight as `HH:MM`.
pub fn clock(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60 % 24, minutes % 60)
}

/// One waiting item as the planner sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Stable between looks: `t<ticket>`, else its kind and words.
    pub key: String,
    pub ticket: Option<TicketId>,
    pub title: String,
    pub impact: Impact,
    /// How long it has waited, seconds (0: not known).
    pub waited: u64,
}

/// The planner's items from the daemon's list of what waits, `now` unix seconds.
pub fn items_of(list: &[Waiting], now: u64) -> Vec<Item> {
    list.iter()
        .map(|w| Item {
            key: w.ticket.map_or_else(|| format!("{:?}:{}", w.kind, w.title), |id| format!("t{id}")),
            ticket: w.ticket,
            title: w.title.clone(),
            impact: w.impact.clone(),
            waited: w.since.map_or(0, |s| now.saturating_sub(s)),
        })
        .collect()
}

/// What was told about one item.
#[derive(Debug, Clone, Default)]
struct Told {
    /// The highest level with its own notification that went out.
    level: u8,
    /// Notifications about it, all still unanswered.
    sends: u32,
    last: u64,
    missed_said: bool,
}

/// Why an item is in the next notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Due {
    Level(u8),
    Daily,
    Again,
}

/// The state of one squad's escalation.
#[derive(Debug, Default)]
pub struct Planner {
    told: HashMap<String, Told>,
    /// When notifications went out in the last hour.
    log: VecDeque<u64>,
    last_daily: u64,
    /// Something came due but was held back (quiet hours, the hourly cap).
    held: bool,
}

impl Planner {
    /// The notices due at `now` for `items`, and the state moved on for what they say.
    pub fn step(&mut self, now: u64, items: &[Item], s: &NotifySettings) -> Vec<Notice> {
        self.told.retain(|key, _| items.iter().any(|i| &i.key == key));
        if !s.on {
            self.held = false;
            return Vec::new();
        }
        let mut due: Vec<(&Item, Due)> = Vec::new();
        let mut missed: Vec<&Item> = Vec::new();
        for item in items {
            let told = self.told.get(&item.key).cloned().unwrap_or_default();
            let lvl = level(s, &item.impact, item.waited);
            if lvl >= 2 {
                let own = lvl.min(LAST_OWN_LEVEL);
                if own > told.level {
                    due.push((item, Due::Level(own)));
                } else if item.impact.holds_deploy
                    && told.sends > 0
                    && s.remind_deploy_mins.is_some_and(|m| m > 0 && now.saturating_sub(told.last) >= u64::from(m) * 60)
                {
                    due.push((item, Due::Again));
                }
            }
            if lvl == 4 && now.saturating_sub(self.last_daily) >= DAILY_EVERY && !due.iter().any(|(i, _)| i.key == item.key) {
                due.push((item, Due::Daily));
            }
            if told.sends >= MISSED_AT && !told.missed_said && eligible(s, &item.impact) {
                missed.push(item);
            }
        }
        if due.is_empty() && missed.is_empty() {
            self.held = false;
            return Vec::new();
        }
        self.log.retain(|at| now.saturating_sub(*at) < HOUR);
        if is_quiet(s, now) || self.log.len() >= s.max_per_hour.max(1) as usize {
            // Nothing is told and nothing is marked: the next allowed minute finds it all still due.
            self.held = true;
            return Vec::new();
        }
        due.sort_by_key(|(i, _)| std::cmp::Reverse(i.impact.score));
        let mut out = Vec::new();
        if !due.is_empty() {
            let daily_only = due.iter().all(|(_, d)| *d == Due::Daily);
            let kind = match (due.len(), self.held, daily_only) {
                (_, _, true) => NoticeKind::Daily,
                (1, false, _) => NoticeKind::Escalation,
                _ => NoticeKind::Batch,
            };
            let notice_items: Vec<NoticeItem> = due.iter().map(|(i, d)| notice_item(i, match d { Due::Level(l) => *l, _ => level(s, &i.impact, i.waited) }, 0)).collect();
            for (item, d) in &due {
                let told = self.told.entry(item.key.clone()).or_default();
                if let Due::Level(l) = d {
                    told.level = told.level.max(*l);
                }
                told.sends += 1;
                told.last = now;
            }
            if due.iter().any(|(_, d)| *d == Due::Daily) {
                self.last_daily = now;
            }
            out.push(Notice { kind, items: notice_items, phone: s.phone });
        }
        if !missed.is_empty() {
            let items = missed.iter().map(|i| notice_item(i, level(s, &i.impact, i.waited), self.told[&i.key].sends)).collect();
            for item in &missed {
                if let Some(told) = self.told.get_mut(&item.key) {
                    told.missed_said = true;
                }
            }
            out.push(Notice { kind: NoticeKind::Missed, items, phone: s.phone });
        }
        self.log.push_back(now);
        self.held = false;
        out
    }
}

fn notice_item(i: &Item, level: u8, missed: u32) -> NoticeItem {
    NoticeItem { ticket: i.ticket, title: i.title.clone(), level, blocks: i.impact.blocks, priority: i.impact.priority, holds_deploy: i.impact.holds_deploy, waited_secs: i.waited, missed }
}

/// A span as the notices say it: `45m`, `18h`, `3d`.
pub fn span(secs: u64) -> String {
    match secs {
        s if s >= DAY => format!("{}d", s / DAY),
        s if s >= HOUR => format!("{}h", s / HOUR),
        s => format!("{}m", (s / 60).max(1)),
    }
}

/// What the PM is told about a notice that reached level 3 or was missed, to
/// bring it up first in conversation. `None` for the others.
pub fn pm_line(notice: &Notice) -> Option<String> {
    let first = notice.items.first()?;
    let what = |i: &NoticeItem| i.ticket.map_or_else(|| i.title.clone(), |id| format!("T-{id} {}", i.title));
    match notice.kind {
        NoticeKind::Missed => Some(format!(
            "The person has missed {} notices about {} — it blocks {} ticket{}. Bring it up first, plainly, and offer to open it.",
            first.missed,
            what(first),
            first.blocks,
            if first.blocks == 1 { "" } else { "s" }
        )),
        _ => {
            let loud: Vec<String> = notice.items.iter().filter(|i| i.level >= 3).map(|i| format!("{} (blocks {}, waiting {})", what(i), i.blocks, span(i.waited_secs))).collect();
            (!loud.is_empty()).then(|| format!("These have blocked work for a long time and wait on the person: {}. Bring them up first in your next message to them.", loud.join("; ")))
        }
    }
}

// ── the person's settings ──

/// Where the person's own default lives, in equipe's home: every squad
/// without settings of its own follows it.
pub fn default_path(home: &std::path::Path) -> std::path::PathBuf {
    home.join("notify.json")
}

/// The person's default; the built-in one when they set none.
pub fn person_default(home: &std::path::Path) -> NotifySettings {
    std::fs::read_to_string(default_path(home)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save_person_default(home: &std::path::Path, s: &NotifySettings) -> std::io::Result<()> {
    std::fs::create_dir_all(home)?;
    std::fs::write(default_path(home), serde_json::to_string_pretty(s).unwrap_or_default())
}

/// What a squad escalates by: its own settings, else the person's default.
pub fn effective(home: &std::path::Path, squad_own: Option<NotifySettings>) -> NotifySettings {
    squad_own.unwrap_or_else(|| person_default(home))
}

/// The settings `apply` reads, as the PM and the command line name them.
pub const KEYS: [&str; 9] = ["on", "only", "quiet", "quiet_from", "quiet_to", "levels", "max_per_hour", "phone", "remind_deploy"];

fn on_off(v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "on" | "true" | "yes" => Some(true),
        "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

/// A wait as minutes: `90m`, `2h`, `1d`, or a bare number of minutes.
pub fn parse_minutes(text: &str) -> Option<u32> {
    let text = text.trim().to_lowercase();
    let (n, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((at, _)) => text.split_at(at),
        None => (text.as_str(), "m"),
    };
    let n: u32 = n.parse().ok()?;
    let per = match unit.trim() {
        "m" | "min" | "mins" => 1,
        "h" | "hr" | "hrs" | "hour" | "hours" => 60,
        "d" | "day" | "days" => 24 * 60,
        _ => return None,
    };
    n.checked_mul(per).filter(|m| *m > 0)
}

/// One setting changed, from its `key` and `value` as the person or the PM
/// gave them. Quiet hours' times are the person's local time.
pub fn apply(s: &mut NotifySettings, key: &str, value: &str, words: &crate::i18n::Catalog) -> Result<(), String> {
    let bad = |expect: &str| words.fill("notify.bad_value", &[("key", key.to_owned()), ("value", value.to_owned()), ("expect", expect.to_owned())]);
    match key {
        "on" => s.on = on_off(value).ok_or_else(|| bad("on, off"))?,
        "only" => {
            s.only = match value.trim().to_lowercase().as_str() {
                "blocking" | "all" => Only::Blocking,
                "urgent" => Only::Urgent,
                _ => return Err(bad("blocking, urgent")),
            }
        }
        "quiet" => s.quiet_on = on_off(value).ok_or_else(|| bad("on, off"))?,
        "quiet_from" => s.quiet_from = parse_clock(value).ok_or_else(|| bad("HH:MM"))?,
        "quiet_to" => s.quiet_to = parse_clock(value).ok_or_else(|| bad("HH:MM"))?,
        "levels" => {
            let parts: Vec<u32> = value.split([',', '/', ' ']).filter(|p| !p.trim().is_empty()).filter_map(parse_minutes).collect();
            match parts[..] {
                [a, b, c] if a < b && b < c => s.level_mins = [a, b, c],
                _ => return Err(bad("2h,8h,24h")),
            }
        }
        "max_per_hour" => s.max_per_hour = value.trim().parse().ok().filter(|n| (1..=60).contains(n)).ok_or_else(|| bad("1-60"))?,
        "phone" => s.phone = on_off(value).ok_or_else(|| bad("on, off"))?,
        "remind_deploy" => s.remind_deploy_mins = if on_off(value) == Some(false) || value.trim() == "none" { None } else { Some(parse_minutes(value).ok_or_else(|| bad("2h, off"))?) },
        _ => return Err(words.fill("notify.bad_key", &[("key", key.to_owned()), ("keys", KEYS.join(", "))])),
    }
    Ok(())
}

/// A wait as the settings say it: `2h`, `90m`.
pub fn minutes_word(mins: u32) -> String {
    if mins.is_multiple_of(24 * 60) {
        format!("{}d", mins / (24 * 60))
    } else if mins.is_multiple_of(60) {
        format!("{}h", mins / 60)
    } else {
        format!("{mins}m")
    }
}

/// The settings in a line, in the person's language.
pub fn describe(s: &NotifySettings, words: &crate::i18n::Catalog) -> String {
    let quiet = if s.quiet_on && s.quiet_from != s.quiet_to { words.fill("notify.quiet_range", &[("from", clock(s.quiet_from)), ("to", clock(s.quiet_to))]) } else { words.get("notify.quiet_none").to_owned() };
    let again = s.remind_deploy_mins.map(|m| words.fill("notify.again", &[("every", minutes_word(m))])).unwrap_or_default();
    words.fill(
        "notify.show",
        &[
            ("state", words.get(if s.on { "notify.on" } else { "notify.off" }).to_owned()),
            ("only", words.get(if s.only == Only::Urgent { "notify.only_urgent" } else { "notify.only_blocking" }).to_owned()),
            ("l2", minutes_word(s.level_mins[0])),
            ("l3", minutes_word(s.level_mins[1])),
            ("l4", minutes_word(s.level_mins[2])),
            ("quiet", quiet),
            ("max", s.max_per_hour.to_string()),
            ("phone", words.get(if s.phone { "notify.on" } else { "notify.off" }).to_owned()),
            ("again", again),
        ],
    )
}

// ── what a notice says ──

/// What an item holds up, in the person's words: `blocks 5 · urgent · holds deploys`.
pub fn impact_line(item: &NoticeItem, words: &crate::i18n::Catalog) -> String {
    let mut parts = Vec::new();
    if item.blocks > 0 {
        parts.push(words.fill("notice.blocks", &[("n", item.blocks.to_string())]));
    }
    match item.priority {
        Some(equipe_protocol::Priority::Urgent) => parts.push(words.get("notice.urgent").to_owned()),
        Some(equipe_protocol::Priority::High) => parts.push(words.get("notice.high").to_owned()),
        _ => {}
    }
    if item.holds_deploy {
        parts.push(words.get("notice.holds_deploys").to_owned());
    }
    parts.join(" · ")
}

fn what(item: &NoticeItem) -> String {
    item.ticket.map_or_else(|| item.title.clone(), |id| format!("T-{id} {}", item.title))
}

/// A notice's title and body in the person's words — the one text every
/// channel (the desktop's, a phone's, Telegram's) tells it with.
pub fn notice_text(notice: &Notice, words: &crate::i18n::Catalog) -> (String, String) {
    let Some(first) = notice.items.first() else { return (String::new(), String::new()) };
    let list = || notice.items.iter().take(3).map(|i| format!("{} ({})", what(i), impact_line(i, words))).collect::<Vec<_>>().join(" · ");
    let longest = notice.items.iter().map(|i| i.waited_secs).max().unwrap_or(0);
    match notice.kind {
        NoticeKind::Escalation => (words.fill("notice.one_title", &[("what", what(first))]), words.fill("notice.one_body", &[("impact", impact_line(first, words)), ("span", span(first.waited_secs))])),
        NoticeKind::Batch => (words.fill("notice.batch_title", &[("n", notice.items.len().to_string())]), list()),
        NoticeKind::Daily => {
            let blocks = notice.items.iter().map(|i| i.blocks).sum::<u32>();
            (words.fill("notice.daily_title", &[("n", notice.items.len().to_string()), ("blocks", blocks.to_string()), ("span", span(longest))]), list())
        }
        NoticeKind::Missed => (words.fill("notice.missed_title", &[("n", first.missed.to_string()), ("what", what(first))]), words.fill("notice.missed_body", &[("impact", impact_line(first, words))])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::Priority;

    const NOON: u64 = 1_800_000_000 - 1_800_000_000 % DAY + 12 * HOUR;

    fn blocking(blocks: u32, urgent: bool) -> Impact {
        Impact { score: blocks * 10, blocks, priority: urgent.then_some(Priority::Urgent), ..Impact::default() }
    }

    fn item(key: &str, impact: Impact, waited: u64) -> Item {
        Item { key: key.into(), ticket: key.strip_prefix('t').and_then(|n| n.parse().ok()), title: format!("title {key}"), impact, waited }
    }

    /// Never quiet, never capped: only the rule under test.
    fn open() -> NotifySettings {
        NotifySettings { quiet_on: false, max_per_hour: 100, utc_offset_secs: Some(0), ..NotifySettings::default() }
    }

    #[test]
    fn the_level_follows_how_long_an_item_has_held_work_up_and_how_much_it_matters() {
        let s = open();
        let urgent = blocking(2, true);
        assert_eq!([0, 2 * HOUR - 1, 2 * HOUR, 8 * HOUR, 24 * HOUR].map(|w| level(&s, &urgent, w)), [1, 1, 2, 3, 4]);
        let deploy = Impact { holds_deploy: true, ..Impact::default() };
        assert_eq!(level(&s, &deploy, 2 * HOUR), 2, "a held deploy is as strong as urgent work");
        let plain = blocking(1, false);
        assert_eq!([2 * HOUR, 6 * HOUR, 24 * HOUR, 72 * HOUR].map(|w| level(&s, &plain, w)), [1, 2, 3, 4], "other blockers wait three times as long");
        assert_eq!(level(&s, &Impact::default(), 99 * DAY), 1, "what blocks nothing never escalates");
        let urgent_only = NotifySettings { only: Only::Urgent, ..open() };
        assert_eq!(level(&urgent_only, &plain, 99 * DAY), 1);
        assert_eq!(level(&urgent_only, &deploy, 2 * HOUR), 2);
        assert_eq!(level(&NotifySettings { on: false, ..open() }, &urgent, 99 * DAY), 1);
        let slower = NotifySettings { level_mins: [60, 120, 180], ..open() };
        assert_eq!(level(&slower, &urgent, 3 * HOUR), 4, "the thresholds are the person's");
    }

    #[test]
    fn each_level_is_told_once_per_item() {
        let s = open();
        let mut p = Planner::default();
        let it = |w| vec![item("t1", blocking(3, true), w)];
        assert!(p.step(NOON, &it(HOUR), &s).is_empty(), "level 1: only in Needs you");
        let n = p.step(NOON, &it(2 * HOUR), &s);
        assert_eq!((n.len(), n[0].kind, n[0].items[0].level), (1, NoticeKind::Escalation, 2));
        assert!(p.step(NOON + 60, &it(2 * HOUR + 60), &s).is_empty(), "never twice for a level");
        let n = p.step(NOON + 7 * HOUR, &it(8 * HOUR), &s);
        assert_eq!(n[0].items[0].level, 3);
        assert!(pm_line(&n[0]).unwrap().contains("T-1 title t1 (blocks 3, waiting 8h)"), "level 3 is the PM's to bring up first");
        assert!(pm_line(&Notice { kind: NoticeKind::Escalation, items: vec![NoticeItem { level: 2, ..notice_item(&it(0)[0], 2, 0) }], phone: true }).is_none());
        // An item that was answered and comes back starts again.
        assert!(p.step(NOON + 8 * HOUR, &[], &s).is_empty());
        let n = p.step(NOON + 9 * HOUR, &it(2 * HOUR), &s);
        assert_eq!(n[0].items[0].level, 2);
    }

    #[test]
    fn the_daily_summary_comes_once_a_day_with_what_has_blocked_the_longest() {
        let s = open();
        let mut p = Planner::default();
        let items = |w| vec![item("t1", blocking(5, true), w), item("t2", blocking(4, true), w)];
        p.step(NOON, &items(2 * HOUR), &s);
        p.step(NOON + HOUR, &items(8 * HOUR), &s);
        let n = p.step(NOON + 2 * HOUR, &items(24 * HOUR + HOUR), &s);
        assert_eq!(n.len(), 1);
        assert_eq!((n[0].kind, n[0].items.len()), (NoticeKind::Daily, 2));
        assert_eq!(n[0].items.iter().map(|i| i.blocks).sum::<u32>(), 9, "these 2 items have blocked 9 tickets");
        assert!(p.step(NOON + 3 * HOUR, &items(26 * HOUR), &s).iter().all(|n| n.kind != NoticeKind::Daily), "not again the same day");
        let next = p.step(NOON + 2 * HOUR + DAY, &items(50 * HOUR), &s);
        assert_eq!(next[0].kind, NoticeKind::Daily, "and again a day later");
    }

    #[test]
    fn quiet_hours_follow_the_persons_time_zone_and_may_cross_midnight() {
        let at = |hour: u64| NOON - 12 * HOUR + hour * HOUR; // hour of the day in UTC
        let s = NotifySettings { utc_offset_secs: Some(0), ..NotifySettings::default() };
        assert!([22, 23, 0, 7, 8, 12].map(|h| is_quiet(&s, at(h))) == [false, true, true, true, false, false]);
        // Seoul (UTC+9): 14:00 UTC is 23:00 there, quiet; 23:00 UTC is 08:00, not.
        let seoul = NotifySettings { utc_offset_secs: Some(9 * 3600), ..NotifySettings::default() };
        assert!(is_quiet(&seoul, at(14)) && !is_quiet(&seoul, at(23)) && !is_quiet(&seoul, at(13)));
        // New York (UTC-5): 04:00 UTC is 23:00 the evening before.
        let ny = NotifySettings { utc_offset_secs: Some(-5 * 3600), ..NotifySettings::default() };
        assert!(is_quiet(&ny, at(4)) && !is_quiet(&ny, at(3)));
        let day = NotifySettings { quiet_from: 12 * 60, quiet_to: 13 * 60, utc_offset_secs: Some(0), ..NotifySettings::default() };
        assert!(is_quiet(&day, at(12)) && !is_quiet(&day, at(13)));
        assert!(!is_quiet(&NotifySettings { quiet_on: false, ..s.clone() }, at(0)));
        assert!(!is_quiet(&NotifySettings { quiet_from: 60, quiet_to: 60, ..s }, at(1)), "equal ends: no window");
        assert_eq!((parse_clock("9"), parse_clock("9:30"), parse_clock("23:59"), parse_clock("24:00"), parse_clock("x")), (Some(540), Some(570), Some(1439), None, None));
        assert_eq!(clock(570), "09:30");
    }

    #[test]
    fn what_comes_due_in_quiet_hours_goes_out_after_as_one_summary() {
        let s = NotifySettings { utc_offset_secs: Some(0), max_per_hour: 100, ..NotifySettings::default() };
        let midnight = NOON - 12 * HOUR + DAY; // 00:00 UTC
        let mut p = Planner::default();
        let items = |extra: u64| vec![item("t1", blocking(5, true), 3 * HOUR + extra), item("t2", blocking(2, true), 3 * HOUR + extra)];
        assert!(p.step(midnight, &items(0), &s).is_empty(), "00:00 is quiet");
        assert!(p.step(midnight + 4 * HOUR, &items(4 * HOUR), &s).is_empty());
        let n = p.step(midnight + 8 * HOUR, &items(8 * HOUR), &s);
        assert_eq!(n.len(), 1, "one combined notice, not one each");
        assert_eq!((n[0].kind, n[0].items.len()), (NoticeKind::Batch, 2));
        assert_eq!(n[0].items[0].ticket, Some(1), "the most blocking first");
        assert!(n[0].items.iter().all(|i| i.level >= 2));
        assert!(p.step(midnight + 8 * HOUR + 60, &items(8 * HOUR + 60), &s).is_empty());
    }

    #[test]
    fn at_most_so_many_notifications_an_hour_and_the_rest_wait_for_the_next() {
        let s = NotifySettings { max_per_hour: 1, ..open() };
        let mut p = Planner::default();
        let first = vec![item("t1", blocking(3, true), 2 * HOUR)];
        assert_eq!(p.step(NOON, &first, &s).len(), 1);
        let both = vec![item("t1", blocking(3, true), 2 * HOUR + 600), item("t2", blocking(3, true), 2 * HOUR)];
        assert!(p.step(NOON + 600, &both, &s).is_empty(), "held back: one an hour");
        let n = p.step(NOON + HOUR + 1, &both, &s);
        assert_eq!((n.len(), n[0].items.len(), n[0].items[0].ticket), (1, 1, Some(2)), "t1 was told already; t2 is due, told as what was held");
        assert_eq!(n[0].kind, NoticeKind::Batch);
    }

    #[test]
    fn an_item_told_three_times_and_still_waiting_is_said_to_be_missed_once() {
        let s = open();
        let mut p = Planner::default();
        let it = |w| vec![item("t2343", blocking(5, true), w)];
        p.step(NOON, &it(2 * HOUR), &s);
        p.step(NOON + 6 * HOUR, &it(8 * HOUR), &s);
        let third = p.step(NOON + 22 * HOUR, &it(24 * HOUR), &s);
        assert_eq!(third[0].kind, NoticeKind::Daily);
        let n = p.step(NOON + 22 * HOUR + 60, &it(24 * HOUR + 60), &s);
        assert_eq!((n.len(), n[0].kind, n[0].items[0].missed), (1, NoticeKind::Missed, 3));
        let line = pm_line(&n[0]).unwrap();
        assert!(line.contains("missed 3 notices about T-2343") && line.contains("blocks 5 tickets"), "{line}");
        assert!(p.step(NOON + 23 * HOUR, &it(25 * HOUR), &s).is_empty(), "said once");
    }

    #[test]
    fn what_holds_a_deploy_up_is_told_again_as_often_as_the_person_asked() {
        let s = NotifySettings { remind_deploy_mins: Some(120), ..open() };
        let mut p = Planner::default();
        let it = |w| vec![item("t1", Impact { holds_deploy: true, score: 50, ..Impact::default() }, w)];
        assert_eq!(p.step(NOON, &it(2 * HOUR), &s).len(), 1);
        assert!(p.step(NOON + HOUR, &it(3 * HOUR), &s).is_empty());
        assert_eq!(p.step(NOON + 2 * HOUR, &it(4 * HOUR), &s).len(), 1, "every 2 hours");
        assert!(p.step(NOON + 3 * HOUR, &it(5 * HOUR), &s).is_empty());
        let off = NotifySettings { remind_deploy_mins: None, ..open() };
        let mut q = Planner::default();
        q.step(NOON, &it(2 * HOUR), &off);
        assert!(q.step(NOON + 3 * HOUR, &it(5 * HOUR), &off).is_empty());
    }

    #[test]
    fn turned_off_nothing_is_told_and_the_list_reads_from_the_daemons_waits() {
        let mut p = Planner::default();
        assert!(p.step(NOON, &[item("t1", blocking(9, true), 9 * DAY)], &NotifySettings { on: false, ..open() }).is_empty());
        let list = vec![Waiting {
            kind: equipe_protocol::importance::WaitKind::Ticket,
            ticket: Some(7),
            status: None,
            title: "AWS sign-in".into(),
            since: Some(NOON - 3 * HOUR),
            impact: blocking(5, false),
        }];
        let items = items_of(&list, NOON);
        assert_eq!((items[0].key.as_str(), items[0].waited, items[0].ticket), ("t7", 3 * HOUR, Some(7)));
        assert_eq!(span(45 * 60), "45m");
        assert_eq!(span(18 * HOUR), "18h");
        assert_eq!(span(3 * DAY), "3d");
    }

    fn en() -> &'static crate::i18n::Catalog {
        crate::i18n::core(crate::i18n::Lang::En)
    }

    #[test]
    fn settings_are_changed_one_key_at_a_time_and_a_bad_value_says_what_fits() {
        let mut s = NotifySettings::default();
        let mut set = |k: &str, v: &str| apply(&mut s, k, v, en());
        assert!(set("quiet_from", "22:30").is_ok() && set("quiet_to", "9").is_ok());
        assert!(set("only", "urgent").is_ok() && set("phone", "off").is_ok() && set("max_per_hour", "2").is_ok());
        assert!(set("levels", "1h, 4h, 12h").is_ok() && set("remind_deploy", "2h").is_ok());
        assert_eq!((s.quiet_from, s.quiet_to, s.only, s.phone, s.max_per_hour, s.level_mins, s.remind_deploy_mins), (1350, 540, Only::Urgent, false, 2, [60, 240, 720], Some(120)));
        let mut set = |k: &str, v: &str| apply(&mut s, k, v, en());
        assert!(set("remind_deploy", "off").is_ok() && set("quiet", "off").is_ok() && set("on", "off").is_ok());
        assert_eq!((s.remind_deploy_mins, s.quiet_on, s.on), (None, false, false));
        let mut set = |k: &str, v: &str| apply(&mut s, k, v, en());
        assert_eq!(set("quiet_from", "25:00").unwrap_err(), "quiet_from doesn\u{2019}t take \u{201c}25:00\u{201d} \u{2014} use HH:MM.");
        assert!(set("levels", "8h,2h,24h").is_err(), "levels go up");
        assert!(set("max_per_hour", "0").is_err() && set("only", "some").is_err());
        assert!(set("volume", "11").unwrap_err().contains("No setting called volume"));
        assert_eq!((parse_minutes("90m"), parse_minutes("2h"), parse_minutes("1d"), parse_minutes("45"), parse_minutes("0h"), parse_minutes("x")), (Some(90), Some(120), Some(1440), Some(45), None, None));
    }

    #[test]
    fn a_squads_own_settings_win_over_the_persons_default_which_wins_over_the_built_in() {
        let dir = std::env::temp_dir().join(format!("equipe-notify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(effective(&dir, None), NotifySettings::default());
        let mine = NotifySettings { quiet_from: 22 * 60, ..NotifySettings::default() };
        save_person_default(&dir, &mine).unwrap();
        assert_eq!(effective(&dir, None), mine, "the person's default");
        let squads = NotifySettings { only: Only::Urgent, ..mine.clone() };
        assert_eq!(effective(&dir, Some(squads.clone())), squads, "the squad's own");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_settings_and_the_notices_read_in_words() {
        let s = NotifySettings { remind_deploy_mins: Some(120), utc_offset_secs: None, ..NotifySettings::default() };
        let line = describe(&s, en());
        assert!(line.contains("Notifications are on") && line.contains("2h, 8h and 1d") && line.contains("23:00\u{2013}08:00") && line.contains("Deploy blockers again every 2h."), "{line}");
        assert!(describe(&NotifySettings { quiet_on: false, on: false, ..s }, en()).contains("Quiet hours: none"));
        let it = |ticket, blocks, urgent: bool, deploy: bool, waited, missed| NoticeItem { ticket, title: "AWS sign-in".into(), level: 3, blocks, priority: urgent.then_some(Priority::Urgent), holds_deploy: deploy, waited_secs: waited, missed };
        let note = |kind, items| Notice { kind, items, phone: true };
        let (title, body) = notice_text(&note(NoticeKind::Escalation, vec![it(Some(7), 5, true, false, 6 * HOUR, 0)]), en());
        assert_eq!((title.as_str(), body.as_str()), ("Still waiting on you: T-7 AWS sign-in", "blocks 5 \u{b7} urgent \u{b7} waiting 6h"));
        let (title, _) = notice_text(&note(NoticeKind::Missed, vec![it(Some(2343), 5, false, false, DAY, 3)]), en());
        assert_eq!(title, "You\u{2019}ve missed this 3 times: T-2343 AWS sign-in");
        let (title, body) = notice_text(&note(NoticeKind::Daily, vec![it(Some(1), 5, false, true, 18 * HOUR, 0), it(Some(2), 4, false, false, 9 * HOUR, 0)]), en());
        assert_eq!(title, "2 items have blocked 9 tickets for 18h");
        assert!(body.contains("T-1 AWS sign-in (blocks 5 \u{b7} holds deploys)"), "{body}");
        assert_eq!(notice_text(&note(NoticeKind::Batch, Vec::new()), en()), (String::new(), String::new()));
        // Every language has the words.
        for lang in crate::i18n::Lang::ALL {
            let (title, _) = notice_text(&note(NoticeKind::Batch, vec![it(Some(1), 1, false, false, HOUR, 0)]), crate::i18n::core(lang));
            assert!(!title.is_empty() && !title.contains("notice."), "{lang:?}: {title}");
        }
    }
}
