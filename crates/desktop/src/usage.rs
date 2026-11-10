//! The Usage screen's figures (`views/usage.rs` draws them): how full a
//! plan's window is and when it starts over, at this pace when it runs out,
//! where it went — the person's own sessions outside equipe, each project,
//! ticket and member — its burn hour by hour, the spikes in it in plain
//! words, and the notice when one burns fast. The daemon reads and adds it
//! up (`Command::PlanUse`, `Event::PlanUse`, `Event::PlanBurn`). The
//! members' context, those that spent the most first, too.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use equipe_core::protocol::plan_use::{Beat, Burn, Mostly, Report, Slice, Span, Who};
use equipe_core::protocol::{Command, Engine, Event, TicketId};

use equipe_core::t;

/// How long until `resets_at`, from `now`, as a person says it: "3h 20m",
/// "2d 4h", "12m".
pub fn time_left(resets_at: i64, now: i64) -> String {
    let left = (resets_at - now).max(0);
    let (days, hours, minutes) = (left / 86_400, left % 86_400 / 3_600, left % 3_600 / 60);
    if days > 0 {
        t!("usage.left_days", days = days, hours = hours)
    } else if hours > 0 {
        t!("usage.left_hours", hours = hours, minutes = minutes)
    } else {
        t!("usage.left_minutes", minutes = minutes.max(1))
    }
}

/// 8400 → "8.4k", 2_800_000 → "2.8M".
pub fn short_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1e3).replace(".0k", "k"),
        _ => format!("{:.1}M", n as f64 / 1e6).replace(".0M", "M"),
    }
}

/// A moment as the person's clock reads it: the time today, with the date when
/// it is further off. Asks `date`, as the console does.
pub fn local_time(unix: i64) -> String {
    let format = |fmt: &str| -> Option<String> {
        // BSD date takes -r seconds; GNU date takes -d @seconds.
        [vec!["-r".to_owned(), unix.to_string()], vec!["-d".to_owned(), format!("@{unix}")]].into_iter().find_map(|args| {
            let out = std::process::Command::new("date").args(&args).arg(fmt).output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        })
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    format(if unix - now < 20 * 60 * 60 { "+%H:%M" } else { "+%m/%d %H:%M" }).unwrap_or_default()
}

// ── where the plan went ──

/// Which cut of equipe's share the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cut {
    #[default]
    Tickets,
    Members,
    Projects,
}

impl Cut {
    pub const ALL: [Cut; 3] = [Cut::Tickets, Cut::Members, Cut::Projects];

    pub fn code(self) -> &'static str {
        match self {
            Cut::Tickets => "tickets",
            Cut::Members => "members",
            Cut::Projects => "projects",
        }
    }

    pub fn parse(code: &str) -> Option<Cut> {
        Cut::ALL.into_iter().find(|c| c.code() == code)
    }

    pub fn label(self) -> &'static str {
        match self {
            Cut::Tickets => t!("plan_use.cut_tickets"),
            Cut::Members => t!("plan_use.cut_members"),
            Cut::Projects => t!("plan_use.cut_projects"),
        }
    }
}

/// How long "Hidden · Undo" stays after a notice is closed.
pub const UNDO_FOR: Duration = Duration::from_secs(8);

/// The screen's state: which engine and stretch, the reports as they came,
/// what is open, and the notices of fast burns.
#[derive(Debug)]
pub struct PlanUseUi {
    pub engine: Engine,
    pub span: Span,
    pub cut: Cut,
    pub reports: HashMap<(Engine, Span), Report>,
    /// Looks asked for and not answered yet.
    pub asked: HashSet<(Engine, Span)>,
    /// The hour picked on the burn (its start).
    pub hour: Option<i64>,
    /// "Outside equipe" opened to its folders.
    pub outside_open: bool,
    pub burns: Vec<Burn>,
    /// Notices closed, by id.
    pub dismissed: HashSet<String>,
    /// The notice just closed, to bring back for a moment.
    pub undo: Option<(String, Instant)>,
}

impl Default for PlanUseUi {
    fn default() -> Self {
        Self {
            engine: Engine::Claude,
            span: Span::Week,
            cut: Cut::default(),
            reports: HashMap::new(),
            asked: HashSet::new(),
            hour: None,
            outside_open: false,
            burns: Vec::new(),
            dismissed: HashSet::new(),
            undo: None,
        }
    }
}

impl PlanUseUi {
    /// The look at what is shown, asked again: it changes as the plan burns.
    pub fn ask(&mut self) -> Vec<Command> {
        let at = (self.engine, self.span);
        if !self.asked.insert(at) {
            return Vec::new();
        }
        vec![Command::PlanUse { engine: self.engine, span: self.span }]
    }

    /// A look at another engine or stretch than the one shown, for a screen
    /// that reads it beside (Orchestration's lanes), leaving what is shown as it is.
    pub fn ask_for(&mut self, engine: Engine, span: Span) -> Vec<Command> {
        if !self.asked.insert((engine, span)) {
            return Vec::new();
        }
        vec![Command::PlanUse { engine, span }]
    }

    pub fn pick_engine(&mut self, engine: Engine) -> Vec<Command> {
        if self.engine != engine {
            self.engine = engine;
            self.hour = None;
        }
        self.ask()
    }

    pub fn pick_span(&mut self, span: Span) -> Vec<Command> {
        if self.span != span {
            self.span = span;
            self.hour = None;
        }
        self.ask()
    }

    /// A bar picked, or picked again to let it go.
    pub fn pick_hour(&mut self, at: i64) {
        self.hour = if self.hour == Some(at) { None } else { Some(at) };
    }

    /// What the daemon said. `true` when something shown changed.
    pub fn on_event(&mut self, event: &Event) -> bool {
        match event {
            Event::PlanUse { report } => {
                let at = (report.engine, report.span);
                self.asked.remove(&at);
                self.reports.insert(at, (**report).clone());
                true
            }
            Event::PlanBurn { burn } => {
                if self.burns.iter().any(|b| b.id == burn.id) {
                    return false;
                }
                self.burns.push(burn.clone());
                true
            }
            _ => false,
        }
    }

    pub fn report(&self) -> Option<&Report> {
        self.reports.get(&(self.engine, self.span))
    }

    /// Whether a look is under way for what is shown.
    pub fn looking(&self) -> bool {
        self.asked.contains(&(self.engine, self.span))
    }

    /// The notice to show: the newest burn not closed.
    pub fn notice(&self) -> Option<&Burn> {
        self.burns.iter().rev().find(|b| !self.dismissed.contains(&b.id))
    }

    pub fn dismiss(&mut self, id: &str, now: Instant) {
        self.dismissed.insert(id.to_owned());
        self.undo = Some((id.to_owned(), now));
    }

    /// The notice closed a moment ago, while it can still be brought back.
    pub fn undoable(&self, now: Instant) -> Option<&str> {
        self.undo.as_ref().filter(|(_, at)| now.duration_since(*at) < UNDO_FOR).map(|(id, _)| id.as_str())
    }

    pub fn undo_dismiss(&mut self) {
        if let Some((id, _)) = self.undo.take() {
            self.dismissed.remove(&id);
        }
    }

    /// The hour picked, as the burn has it.
    pub fn picked<'a>(&self, report: &'a Report) -> Option<&'a Beat> {
        let at = self.hour?;
        report.beats.iter().find(|b| b.at == at)
    }
}

/// A share in basis points as a percent: a decimal below 10%, "<0.1%" for a sliver.
pub fn percent(bp: u32) -> String {
    match bp {
        0 => "0%".into(),
        1..=9 => "<0.1%".into(),
        10..=999 => format!("{:.1}%", f64::from(bp) / 100.0),
        _ => format!("{}%", (bp + 50) / 100),
    }
}

/// 1204 → "1,204", grouped as the language groups digits.
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let sep = t!("plan_use.digits_group");
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push_str(sep);
        }
        out.push(c);
    }
    out
}

/// What it went mostly on, in words.
pub fn mostly_words(mostly: Mostly) -> &'static str {
    match mostly {
        Mostly::Rereading => t!("plan_use.mostly_rereading"),
        Mostly::Loading => t!("plan_use.mostly_loading"),
        Mostly::Writing => t!("plan_use.mostly_writing"),
        Mostly::Input => t!("plan_use.mostly_input"),
    }
}

/// A part's name: "Outside equipe", a folder, "T-427 Checkout", a member.
pub fn who_words(who: &Who, label: Option<&str>) -> String {
    match who {
        Who::Outside { folder: None } => t!("plan_use.outside").to_owned(),
        Who::Outside { folder: Some(f) } if f.is_empty() => t!("plan_use.unknown_folder").to_owned(),
        Who::Outside { folder: Some(f) } => f.clone(),
        Who::Ticket { ticket, .. } => match label {
            Some(title) => format!("T-{ticket} {title}"),
            None => format!("T-{ticket}"),
        },
        Who::Member { member, .. } => member.clone(),
        Who::Project { project, .. } => project.clone(),
        Who::Home { squad } => t!("plan_use.home", squad = squad),
        Who::Equipe => t!("plan_use.equipe_other").to_owned(),
    }
}

/// A part's share: of the week, once the plan is read, else of what was seen.
pub fn share_words(slice: &Slice) -> String {
    match slice.week_bp {
        Some(bp) => t!("plan_use.of_week", pct = percent(bp)),
        None => t!("plan_use.of_seen", pct = percent(slice.share_bp)),
    }
}

/// The ticket a part is, when it is one of `squad`'s: it opens.
pub fn ticket_of(who: &Who, squad: &str) -> Option<TicketId> {
    match who {
        Who::Ticket { squad: of, ticket } if of.as_deref().is_none_or(|s| s == squad) => Some(*ticket),
        _ => None,
    }
}

/// A spike in plain words: "T-427 used 7% of the week in 3 hours: 1,204
/// calls, mostly re-reading a long context".
pub fn spike_words(slice: &Slice, from: i64, to: i64) -> String {
    let hours = ((to - from) / 3600).max(1);
    let who = who_words(&slice.who, None);
    let why = mostly_words(slice.mostly);
    let calls = grouped(slice.calls);
    match slice.week_bp {
        Some(bp) => t!("plan_use.spike_week", who = who, pct = percent(bp), count = hours, calls = calls, why = why),
        None => t!("plan_use.spike_seen", who = who, pct = percent(slice.share_bp), count = hours, calls = calls, why = why),
    }
}

/// The notice of a fast burn, in words.
pub fn burn_words(burn: &Burn) -> String {
    let engine = crate::switching::engine_name(burn.engine);
    let who = who_words(&burn.who, None);
    t!("plan_use.burn", who = who, pct = percent(burn.week_bp), engine = engine, count = burn.hours, why = mostly_words(burn.mostly))
}

/// "At this pace it runs out …" against the reset, or that it lasts; and
/// whether it is a warning.
pub fn pace_words(report: &Report, now: i64) -> Option<(String, bool)> {
    let pace = report.pace?;
    let week = report.week?;
    Some(if week.used_percent >= 100 {
        (t!("plan_use.used_up", left = time_left(week.resets_at, now)), true)
    } else if pace.before_reset {
        (t!("plan_use.runs_out", when = local_time(pace.runs_out_at), left = time_left(pace.runs_out_at, now), early = time_left(week.resets_at, pace.runs_out_at)), true)
    } else {
        (t!("plan_use.lasts", per_day = percent(pace.per_day_bp)), false)
    })
}

/// The tallest bar's load, for scaling the rest.
pub fn tallest(beats: &[Beat]) -> u64 {
    beats.iter().map(|b| b.load).max().unwrap_or(0)
}

/// Whether `at` falls in one of the report's spikes.
pub fn in_spike(report: &Report, at: i64) -> bool {
    report.spikes.iter().any(|s| at >= s.from && at < s.to)
}

/// An hour as its bar's list heads it: "14:00–15:00", the day too when it isn't today.
pub fn hour_words(at: i64, now: i64) -> String {
    hours_words(at, at + 3600, now)
}

/// Hours from `from` to `to` as a clock reads them: "23:00–02:00", the day
/// they start on too when it isn't today.
pub fn hours_words(from: i64, to: i64, now: i64) -> String {
    let start = crate::state::local_time(from.max(0) as u64);
    let today = crate::state::local_time(now.max(0) as u64);
    let end = crate::state::local_time(to.max(0) as u64);
    let span = format!("{:02}:00–{:02}:00", start.hour, end.hour);
    if (start.year, start.month, start.day) == (today.year, today.month, today.day) {
        span
    } else {
        format!("{}/{} {span}", start.month, start.day)
    }
}

/// Where a bar's label goes: the date at local midnight on a week, every
/// sixth hour on a day.
pub fn beat_label(at: i64, span: Span) -> Option<String> {
    let local = crate::state::local_time(at.max(0) as u64);
    match span {
        Span::Week if local.hour == 0 => Some(format!("{}/{}", local.month, local.day)),
        Span::Day if local.hour.is_multiple_of(6) => Some(format!("{:02}:00", local.hour)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_says_how_long_until_it_starts_over() {
        assert_eq!(time_left(1_000 + 3 * 3_600 + 20 * 60, 1_000), "3시간 20분");
        assert_eq!(time_left(1_000 + 2 * 86_400 + 4 * 3_600, 1_000), "2일 4시간");
        assert_eq!(time_left(1_030, 1_000), "1분", "never \"0\"");
        assert_eq!(time_left(500, 1_000), "1분", "past: it starts over any moment");
    }

    #[test]
    fn counts_are_short() {
        assert_eq!([short_count(999), short_count(8_400), short_count(12_000), short_count(2_800_000)], ["999", "8.4k", "12k", "2.8M"]);
    }

    use equipe_core::protocol::PlanWindow;
    use equipe_core::protocol::plan_use::{Pace, Spike, Tokens, Who as P};

    fn slice(who: P, week_bp: Option<u32>, calls: u64) -> Slice {
        Slice { who, label: None, load: 10, share_bp: 2500, week_bp, tokens: Tokens::default(), calls, mostly: Mostly::Rereading, children: vec![] }
    }

    fn report(engine: Engine, span: Span) -> Report {
        Report { engine, span, from: 0, to: 0, week: None, pace: None, load: 0, week_load: 0, tokens: Tokens::default(), calls: 0, outside: None, projects: vec![], tickets: vec![], members: vec![], beats: vec![], spikes: vec![], reading: false }
    }

    #[test]
    fn the_screen_asks_once_for_what_it_shows_and_keeps_each_answer() {
        let mut ui = PlanUseUi::default();
        assert_eq!(ui.ask(), vec![Command::PlanUse { engine: Engine::Claude, span: Span::Week }]);
        assert!(ui.ask().is_empty(), "asked already");
        assert!(ui.looking());
        assert_eq!(ui.pick_span(Span::Day), vec![Command::PlanUse { engine: Engine::Claude, span: Span::Day }]);
        assert!(ui.on_event(&Event::PlanUse { report: Box::new(report(Engine::Claude, Span::Week)) }));
        assert!(ui.report().is_none(), "the day's is what is shown");
        // Back on the week: what came is shown while a fresh look comes.
        assert_eq!(ui.pick_span(Span::Week).len(), 1);
        assert!(ui.report().is_some() && ui.looking());
        ui.pick_hour(3600);
        ui.pick_hour(3600);
        assert_eq!(ui.hour, None, "picked again: let go");
        ui.pick_hour(7200);
        ui.pick_engine(Engine::Codex);
        assert_eq!(ui.hour, None, "another engine's burn has other hours");
    }

    fn burn(id: &str) -> Burn {
        Burn { id: id.into(), engine: Engine::Claude, who: P::Ticket { squad: Some("acme".into()), ticket: 427 }, label: None, week_bp: 700, hours: 3, at: 0, calls: 1204, mostly: Mostly::Rereading }
    }

    #[test]
    fn a_burn_notice_shows_once_closes_and_comes_back_on_undo() {
        let mut ui = PlanUseUi::default();
        assert!(ui.on_event(&Event::PlanBurn { burn: burn("a") }));
        assert!(!ui.on_event(&Event::PlanBurn { burn: burn("a") }), "told again to a screen that attached: once");
        assert_eq!(ui.notice().map(|b| b.id.as_str()), Some("a"));
        let at = Instant::now();
        ui.dismiss("a", at);
        assert!(ui.notice().is_none());
        assert_eq!(ui.undoable(at + Duration::from_secs(2)), Some("a"));
        assert_eq!(ui.undoable(at + UNDO_FOR), None, "only for a moment");
        ui.undo_dismiss();
        assert_eq!(ui.notice().map(|b| b.id.as_str()), Some("a"));
        assert_eq!(burn_words(&burn("a")), "T-427이 최근 3시간 동안 Claude 주간 사용량의 7.0%를 썼어요 · 대부분 긴 맥락 다시 읽기");
    }

    #[test]
    fn a_spike_says_who_how_much_of_the_week_how_fast_and_on_what() {
        let s = slice(P::Ticket { squad: Some("acme".into()), ticket: 427 }, Some(700), 1204);
        assert_eq!(spike_words(&s, 0, 3 * 3600), "T-427이 3시간 동안 주간 사용량의 7.0%를 썼어요: 호출 1,204번, 대부분 긴 맥락 다시 읽기");
        let unread = slice(P::Outside { folder: None }, None, 9);
        assert_eq!(spike_words(&unread, 0, 3600), "equipe 밖 — 내 Claude Code·Codex 세션이 1시간 동안 보인 사용량의 25%를 썼어요: 호출 9번, 대부분 긴 맥락 다시 읽기");
    }

    #[test]
    fn shares_and_counts_read_short() {
        assert_eq!([percent(0), percent(5), percent(42), percent(700), percent(3349)], ["0%", "<0.1%", "0.4%", "7.0%", "33%"]);
        assert_eq!([grouped(9), grouped(1204), grouped(4_200_000_000)], ["9", "1,204", "4,200,000,000"]);
        assert_eq!(who_words(&P::Ticket { squad: None, ticket: 4 }, Some("Checkout")), "T-4 Checkout");
        assert_eq!(ticket_of(&P::Ticket { squad: Some("acme".into()), ticket: 4 }, "acme"), Some(4));
        assert_eq!(ticket_of(&P::Ticket { squad: Some("zeta".into()), ticket: 4 }, "acme"), None, "another squad's: not this board's");
    }

    #[test]
    fn the_pace_says_when_it_runs_out_or_that_it_lasts() {
        let now = 1_000_000;
        let mut r = report(Engine::Claude, Span::Week);
        r.week = Some(PlanWindow { used_percent: 61, resets_at: now + 3 * 86_400 });
        r.pace = Some(Pace { runs_out_at: now + 86_400 + 3 * 3600, before_reset: true, per_day_bp: 3000 });
        let (words, warn) = pace_words(&r, now).unwrap();
        assert!(warn && words.contains("1일 3시간") && words.contains("1일 21시간"), "{words}");
        r.pace = Some(Pace { runs_out_at: now + 9 * 86_400, before_reset: false, per_day_bp: 900 });
        let (words, warn) = pace_words(&r, now).unwrap();
        assert!(!warn && words.contains("9.0%"), "{words}");
        r.pace = None;
        assert_eq!(pace_words(&r, now), None);
    }

    #[test]
    fn a_bar_in_a_spike_stands_out_and_its_hour_reads_as_a_clock_does() {
        let mut r = report(Engine::Claude, Span::Week);
        r.spikes = vec![Spike { from: 7200, to: 3 * 3600, slice: slice(P::Equipe, None, 1) }];
        assert!(in_spike(&r, 7200) && !in_spike(&r, 3 * 3600));
        let words = hour_words(1_793_635_200, 1_793_635_200);
        assert!(words.contains(":00–"), "{words}");
    }
}
