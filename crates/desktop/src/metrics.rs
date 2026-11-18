//! Metrics: a product's numbers, read from the daemon (`metrics.*`, docs/METRICS.md).
//! This is the screen's model — what the daemon said, turned into what is drawn
//! (cards, banners, sparklines) — and none of its drawing (`views/metrics.rs`).
//!
//! A key lives in a text box until the person connects, then goes to the
//! daemon in `metrics.connect` and out of the box. Nothing here holds one:
//! [`Draft`] keeps the field names and never a value, and what the daemon
//! answers has its keys cut out already (docs/METRICS.md, Rules).

use std::collections::BTreeMap;

use equipe_core::protocol::metrics::{
    Bucket, Connection, ErrorCode, FetchError, MetricValue, PosthogAccount, PosthogLook, PosthogProject, PosthogSetup, QueryResult, Sample, SetupOutcome, Shape, SourceKind, SourceSpec, Sources,
};
use equipe_core::protocol::secrets::SquadSecret;

/// The periods the screen offers, in days.
pub const PERIODS: [u32; 2] = [7, 30];

/// What the person is looking at.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Flow {
    /// The dashboard (or its empty state).
    #[default]
    Dash,
    /// Connecting, or managing what is connected.
    Connect(Draft),
}

/// A connection being set up. Holds no value the person types.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Draft {
    /// The service picked (step 2).
    pub kind: Option<SourceKind>,
    /// The connection whose key or settings are being replaced.
    pub replacing: Option<String>,
    /// The request is out.
    pub sending: bool,
    /// The service connected (step 3).
    pub done: Option<SourceKind>,
    /// Why the last try failed.
    pub failed: Option<String>,
    /// The person pressed connect with a required box empty.
    pub missing: bool,
    /// Opened from the North Star card: only its three boxes show.
    pub north: bool,
    /// PostHog's "Set it up for me", under way (docs/METRICS.md).
    pub setup: Option<Setup>,
    /// PostHog's boxes to paste a key into, picked over "Set it up for me".
    pub paste: bool,
}

/// Where PostHog's "Set it up for me" stands. Holds no key: the key goes
/// from PostHog to the daemon's Keychain and never comes to the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setup {
    /// Asking equipe's browser who is signed in to PostHog.
    Looking,
    /// Not signed in: the browser is open on PostHog's sign-in, for the
    /// person to sign in themselves; looked at again every few seconds.
    SignIn,
    /// More than one project: the person picks.
    Choose(Box<PosthogLook>),
    /// The card that says what equipe will make — the person's yes is what
    /// makes it.
    Confirm(Box<Picked>),
    Creating(Box<Picked>),
    Done { setup: Box<PosthogSetup>, project: String },
    /// PostHog wants a sign-in from the last two hours before it makes a key.
    SignInAgain { host: String },
    /// equipe couldn't make it: the key page, its steps and the paste box.
    Manual { keys_url: String, why: Option<String> },
    /// equipe's browser can't be asked here (members use a Chrome each).
    NoBrowser,
}

/// The project to connect, and the PostHog account it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub account: PosthogAccount,
    pub project: PosthogProject,
}

/// PostHog's sign-in page, where "Set it up for me" sends a person who isn't
/// signed in (its region picker is there too).
pub const POSTHOG_SIGN_IN: &str = "https://us.posthog.com/login";
/// PostHog's key page when no region is known yet.
pub const POSTHOG_KEYS: &str = "https://us.posthog.com/settings/user-api-keys";

/// What follows a look at PostHog: sign in, pick a project, or the card that
/// asks the person's yes — the only project, or the one they picked.
pub fn after_look(look: &PosthogLook) -> Setup {
    if !look.browser {
        return Setup::NoBrowser;
    }
    if !look.signed_in() {
        return Setup::SignIn;
    }
    match look.only_project() {
        Some((account, project)) => Setup::Confirm(Box::new(Picked { account: account.clone(), project: project.clone() })),
        None => Setup::Choose(Box::new(look.clone())),
    }
}

/// What follows the daemon's answer to the person's yes.
pub fn after_setup(done: PosthogSetup, account: &PosthogAccount, project: &PosthogProject) -> Setup {
    match done.outcome {
        SetupOutcome::Connected => Setup::Done { setup: Box::new(done), project: project.name.clone() },
        SetupOutcome::SignIn => Setup::SignIn,
        SetupOutcome::SignInAgain => Setup::SignInAgain { host: account.host.clone() },
        SetupOutcome::Manual => Setup::Manual { keys_url: done.keys_url, why: done.why },
    }
}

/// A PostHog key a member saved in the squad's keys — on a ticket that wired
/// PostHog into the product, say — while Metrics shows no PostHog yet: the
/// one-tap "Show these numbers in Metrics?".
pub fn posthog_suggestion<'a>(squad: &str, secrets: &'a [SquadSecret], connections: &[Connection], dismissed: &[String]) -> Option<&'a SquadSecret> {
    if connections.iter().any(|c| c.kind == SourceKind::Posthog) {
        return None;
    }
    let posthog = |s: &SquadSecret| [s.service.as_str(), s.name.as_str()].iter().any(|t| t.to_ascii_lowercase().contains("posthog"));
    secrets.iter().filter(|s| s.squad.as_str() == squad && posthog(s) && !dismissed.contains(&s.name)).max_by_key(|s| s.created_at)
}

/// The event the North Star counts the people of, when it was set that way
/// ("Set it up for me" on a product without equipe's `work_accepted`): the
/// card names it, and counts people, not squads.
pub fn north_people_event(sources: &Sources) -> Option<String> {
    let c = sources.connections.iter().find(|c| c.kind == SourceKind::Posthog)?;
    let event = c.settings.get("north_star_event")?;
    (c.settings.get("north_star_property").map(String::as_str) == Some("distinct_id")).then(|| event.clone())
}

/// PostHog's metrics counted from page views.
pub const PAGE_VIEW_METRICS: [&str; 4] = ["visits", "languages", "referrers", "utm_sources"];

/// The PostHog connections whose project sends no page views: fetched fine,
/// and not one day of visits. Their page-view cards make way for one that
/// says so, instead of four empty ones.
pub fn without_page_views(query: &QueryResult, sources: &Sources) -> Vec<String> {
    sources
        .connections
        .iter()
        .filter(|c| c.kind == SourceKind::Posthog && c.last_ok.is_some() && c.last.as_ref().is_some_and(|l| l.error.is_none()))
        .filter(|c| query.values.iter().filter(|m| m.connection == c.id && m.metric == "visits").all(|m| m.days_known == 0))
        .filter(|c| query.values.iter().any(|m| m.connection == c.id))
        .map(|c| c.id.clone())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Metrics {
    pub days: u32,
    pub sources: Option<Sources>,
    pub query: Option<QueryResult>,
    pub flow: Flow,
    /// Numbers are being asked for.
    pub loading: bool,
    /// Why they couldn't be (the daemon's own words).
    pub failed: Option<String>,
    /// The screen's own sample (`--features snapshot`): no daemon is asked.
    pub demo: bool,
    /// The width the screen has, in pixels (0 until the shell has drawn once).
    pub room: f32,
    /// Saved PostHog keys whose "Show these numbers in Metrics?" got a "Not now".
    pub dismissed: Vec<String>,
}

impl Metrics {
    pub fn new() -> Self {
        Self { days: PERIODS[0], ..Self::default() }
    }

    pub fn connections(&self) -> &[Connection] {
        self.sources.as_ref().map_or(&[], |s| &s.connections)
    }

    /// Nothing connected yet (once the daemon has said).
    pub fn is_empty(&self) -> bool {
        self.sources.as_ref().is_some_and(|s| s.connections.is_empty())
    }

    pub fn spec(&self, kind: SourceKind) -> Option<&SourceSpec> {
        self.sources.as_ref()?.kinds.iter().find(|k| k.kind == kind)
    }

    pub fn connection_of(&self, kind: SourceKind) -> Option<&Connection> {
        self.connections().iter().find(|c| c.kind == kind)
    }
}

// ---- numbers and days ----

/// `1,284`; a small fractional number keeps one decimal (`4.5`).
pub fn number(v: f64) -> String {
    let whole = v.abs() >= 100. || v.fract().abs() < 0.05;
    let text = if whole { format!("{:.0}", v.abs()) } else { format!("{:.1}", v.abs()) };
    let (int, frac) = text.split_once('.').map_or((text.as_str(), None), |(i, f)| (i, Some(f)));
    let mut out = String::new();
    for (n, ch) in int.chars().enumerate() {
        if n > 0 && (int.len() - n) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if let Some(f) = frac {
        out.push('.');
        out.push_str(f);
    }
    if v < 0. && out != "0" { format!("−{out}") } else { out }
}

/// `(month, day)` of a `YYYY-MM-DD`.
pub fn month_day(day: &str) -> Option<(u32, u32)> {
    let mut parts = day.split('-');
    let _year = parts.next()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some((month, day))
}

/// A day in words, in the person's language order (`Nov 2`, Japanese `11月2日`).
pub fn day_words(day: &str) -> String {
    match month_day(day) {
        Some((month, day)) => equipe_core::t!("metrics.date", month = month, day = day),
        None => day.to_owned(),
    }
}

/// The local `YYYY-MM-DD` and `HH:MM` of unix seconds.
pub fn local_day_time(unix: u64) -> (String, String) {
    let Some(tm) = equipe_providers::platform::local_time(unix) else { return (unix.to_string(), String::new()) };
    (format!("{:04}-{:02}-{:02}", tm.year, tm.month, tm.day), format!("{:02}:{:02}", tm.hour, tm.minute))
}

/// A moment: just the time when it is today, else the day and the time.
pub fn moment(unix: u64, now: u64) -> String {
    let (day, time) = local_day_time(unix);
    if day == local_day_time(now).0 { time } else { format!("{} {time}", day_words(&day)) }
}

// ---- cards ----

/// A rise or a fall against the period before.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub up: Option<bool>,
    /// `+8.2%`, `−3`.
    pub text: String,
}

fn change_of(m: &MetricValue) -> Option<Change> {
    let (sign, body) = match (m.change_pct, m.change) {
        (Some(pct), _) => (pct, format!("{}%", number(pct.abs()))),
        (None, Some(delta)) => (delta, number(delta.abs())),
        (None, None) => return None,
    };
    let (up, mark) = if sign > 0. { (Some(true), "+") } else if sign < 0. { (Some(false), "−") } else { (None, "") };
    Some(Change { up, text: format!("{mark}{body}") })
}

/// One bar of a breakdown.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub label: String,
    pub value: String,
    /// 0–1 of the biggest.
    pub fill: f32,
}

/// How a metric is drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// A number, with its line against the period before.
    Number { value: Option<String>, change: Option<Change>, series: Vec<Option<f64>>, previous: Vec<Option<f64>> },
    Bars(Vec<Bar>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub connection: String,
    pub kind: SourceKind,
    pub metric: String,
    pub label: String,
    pub unit: String,
    pub body: Body,
    /// The service's latest fetch failed: what the card says, dimmed.
    pub error: Option<ErrorCode>,
    /// That failure is the service saying it is busy (a 503): a wait.
    pub busy: bool,
    /// The latest day with a value.
    pub as_of: Option<String>,
    /// Days with a value, of those asked.
    pub days_known: u32,
    pub days: u32,
    /// The latest fetch that worked (unix seconds).
    pub last_ok: Option<u64>,
    /// The latest fetch (unix seconds).
    pub fetched: Option<u64>,
    /// When a refused-for-now fetch is tried again (unix seconds).
    pub retry_at: Option<u64>,
    /// The service itself keeps this many days only.
    pub keeps_days: Option<u32>,
}

fn values_of(samples: &[Sample]) -> Vec<Option<f64>> {
    samples.iter().map(|s| s.value).collect()
}

/// The dashboard's cards: one per metric, in the daemon's order.
pub fn cards(query: &QueryResult, sources: &Sources) -> Vec<Card> {
    query
        .values
        .iter()
        .filter(|m| m.metric != NORTH_STAR)
        .map(|m| {
            let connection = sources.connections.iter().chain(&query.connections).find(|c| c.id == m.connection);
            let spec = sources.kinds.iter().find(|k| k.kind == m.kind);
            let body = match m.shape {
                Shape::Breakdown => {
                    let top = m.breakdown.iter().map(|s| s.value).fold(0., f64::max);
                    Body::Bars(m.breakdown.iter().map(|s| Bar { label: s.label.clone(), value: number(s.value), fill: if top > 0. { (s.value / top) as f32 } else { 0. } }).collect())
                }
                Shape::Count | Shape::Total | Shape::Rolling => Body::Number { value: m.value.map(number), change: change_of(m), series: values_of(&m.series), previous: values_of(&m.previous_series) },
            };
            Card {
                connection: m.connection.clone(),
                kind: m.kind,
                metric: m.metric.clone(),
                label: m.label.clone(),
                unit: m.unit.clone(),
                body,
                error: connection.and_then(|c| c.last.as_ref()?.error.as_ref()).map(|e| e.code),
                busy: connection.and_then(|c| c.last.as_ref()?.error.as_ref()).is_some_and(FetchError::busy),
                as_of: m.as_of.clone(),
                days_known: m.days_known,
                days: query.days,
                last_ok: connection.and_then(|c| c.last_ok),
                fetched: connection.and_then(|c| c.last.as_ref()).map(|l| l.at),
                retry_at: connection.and_then(|c| c.last.as_ref()?.error.as_ref()?.retry_at),
                keeps_days: spec.and_then(|s| s.keeps_days),
            }
        })
        .collect()
}

/// A failed fetch the dashboard says above its cards.
#[derive(Debug, Clone, PartialEq)]
pub struct Banner {
    pub connection: String,
    pub kind: SourceKind,
    pub error: FetchError,
    pub last_ok: Option<u64>,
    pub at: u64,
}

pub fn banners(sources: &Sources) -> Vec<Banner> {
    sources
        .connections
        .iter()
        .filter_map(|c| {
            let last = c.last.as_ref()?;
            Some(Banner { connection: c.id.clone(), kind: c.kind, error: last.error.clone()?, last_ok: c.last_ok, at: last.at })
        })
        .collect()
}

/// What fixes it: a new key, or trying again.
pub fn needs_key(code: ErrorCode) -> bool {
    matches!(code, ErrorCode::Auth | ErrorCode::Keychain)
}

/// The catalog's word for a failure: `busy` for a service that answered "too
/// busy" (a 503), else its code's.
pub fn error_word(e: &FetchError) -> &'static str {
    if e.busy() {
        return "busy";
    }
    match e.code {
        ErrorCode::Auth => "auth",
        ErrorCode::RateLimited => "rate_limited",
        ErrorCode::Network => "network",
        ErrorCode::BadResponse => "bad_response",
        ErrorCode::Keychain => "keychain",
        ErrorCode::Other => "other",
    }
}

/// A banner's title and body, in the person's language: what happened (the
/// service's own words when it answered), when it is tried again, and how
/// old the numbers shown are — that last part only when there are some.
pub fn banner_words(b: &Banner, service: &str, now: u64) -> (String, String) {
    let catalog = crate::i18n::catalog();
    let word = error_word(&b.error);
    let title = catalog.fill(&format!("metrics.err.{word}.title"), &[("service", service.to_owned())]);
    let mut parts = Vec::new();
    match (b.error.code, &b.error.said) {
        // A wait: the service's words say why, the retry when.
        (ErrorCode::RateLimited, said) => {
            if let Some(said) = said {
                parts.push(catalog.fill("metrics.err.said", &[("service", service.to_owned()), ("said", said.clone())]));
            }
            parts.push(match b.error.retry_at.filter(|at| *at > now) {
                Some(at) => catalog.fill("metrics.err.rate_limited.body", &[("retry", moment(at, now))]),
                None => catalog.get("metrics.err.rate_limited.body_soon").to_owned(),
            });
        }
        (code, said) => {
            parts.push(catalog.fill(&format!("metrics.err.{word}.body"), &[("when", moment(b.at, now))]));
            if let (Some(said), false) = (said, needs_key(code)) {
                parts.push(catalog.fill("metrics.err.said", &[("service", service.to_owned()), ("said", said.clone())]));
            }
        }
    }
    if let Some(last) = b.last_ok {
        parts.push(catalog.fill("metrics.err.shown_from", &[("last", moment(last, now))]));
    }
    (title, sentences(&parts))
}

/// Sentences one after the other: a space between them, none after a
/// sentence that ends in a full-width stop (Japanese, Chinese).
pub fn sentences(parts: &[String]) -> String {
    let mut out = String::new();
    for part in parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        if !out.is_empty() && !out.ends_with(['。', '！', '？']) {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}

/// The days of a period, as the screen words them: `Oct 27 – Nov 2`.
pub fn range_words(query: &QueryResult) -> String {
    equipe_core::t!("metrics.range", from = day_words(&query.from), to = day_words(&query.to), previous_from = day_words(&query.previous_from), previous_to = day_words(&query.previous_to))
}

// ---- the North Star ----

/// The metric the card at the top shows: squads that approved work in the last
/// 7 days, as 12 windows of 7 days (docs/TELEMETRY.md, "The North Star").
pub const NORTH_STAR: &str = "north_star";
pub const NORTH_BARS: usize = 12;

/// What the North Star card is in.
#[derive(Debug, Clone, PartialEq)]
pub enum NorthStar {
    /// Asked, not answered yet.
    Loading,
    /// Connected to PostHog, which has no definition for it.
    Undefined,
    Ready(NorthCard),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NorthCard {
    pub value: Option<String>,
    /// The last window against the one 7 days before; `None` with nothing to compare.
    pub change: Option<Change>,
    pub previous: Option<String>,
    /// Twelve windows, the oldest first; `None` is a window not known.
    pub bars: Vec<Option<f64>>,
    pub error: Option<ErrorCode>,
    pub busy: bool,
    pub fetched: Option<u64>,
    pub last_ok: Option<u64>,
    pub as_of: Option<String>,
}

impl NorthCard {
    /// Windows with a value.
    pub fn known(&self) -> usize {
        self.bars.iter().flatten().count()
    }

    /// Fewer windows than the twelve the card draws.
    pub fn short(&self) -> bool {
        self.known() < NORTH_BARS
    }
}

/// The card's state, or nothing when PostHog is not connected (there is no
/// number to look for).
pub fn north_star(query: Option<&QueryResult>, sources: &Sources, loading: bool) -> Option<NorthStar> {
    let connection = sources.connections.iter().find(|c| c.kind == SourceKind::Posthog)?;
    let Some(m) = query.and_then(|q| q.values.iter().find(|m| m.metric == NORTH_STAR)) else {
        return Some(if loading { NorthStar::Loading } else { NorthStar::Undefined });
    };
    let mut bars: Vec<Option<f64>> = m.series.iter().map(|s| s.value).collect();
    if bars.len() > NORTH_BARS {
        bars.drain(..bars.len() - NORTH_BARS);
    }
    while bars.len() < NORTH_BARS {
        bars.insert(0, None);
    }
    Some(NorthStar::Ready(NorthCard {
        value: m.value.map(number),
        change: change_of(m).map(|ch| match (m.change, m.change_pct) {
            // `3 (+27%)`: the count, then the share.
            (Some(delta), Some(_)) => Change { text: format!("{} ({})", number(delta.abs()), ch.text), ..ch },
            _ => ch,
        }),
        previous: m.previous.map(number),
        bars,
        error: connection.last.as_ref().and_then(|l| l.error.as_ref()).map(|e| e.code),
        busy: connection.last.as_ref().and_then(|l| l.error.as_ref()).is_some_and(FetchError::busy),
        fetched: connection.last.as_ref().map(|l| l.at),
        last_ok: connection.last_ok,
        as_of: m.as_of.clone(),
    }))
}

/// A bar's height in a box `h` tall: a window with a value is at least 3 px
/// (zero stays visible), the biggest fills the box.
pub fn bar_height(value: f64, top: f64, h: f32) -> f32 {
    if top <= 0. { 3. } else { ((value / top) as f32 * h).max(3.) }
}

/// The source's words in a card's title line, in the three forms the screen
/// uses: `updated today 07:00` (today), `as of Nov 1` (an earlier day) — the
/// bare time is for a card with no room.
pub fn updated(at: u64, now: u64) -> Updated {
    let (day, time) = local_day_time(at);
    if day == local_day_time(now).0 { Updated::Today(time) } else { Updated::Day(day_words(&day)) }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Updated {
    Today(String),
    Day(String),
}

// ---- sparkline ----

/// A line's points in its box, a gap where a day isn't known.
pub type Points = Vec<Option<(f32, f32)>>;

/// The points of two lines in a box `w` × `h`, scaled together (so the period
/// and the one before compare) and with gaps kept: `None` is a day not known.
pub fn spark(series: &[Option<f64>], previous: &[Option<f64>], w: f32, h: f32) -> (Points, Points) {
    let all = || series.iter().chain(previous).flatten().copied();
    let (lo, hi) = all().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let place = |line: &[Option<f64>]| -> Points {
        let last = line.len().saturating_sub(1).max(1) as f32;
        line.iter()
            .enumerate()
            .map(|(n, v)| {
                let v = (*v)?;
                let t = if hi > lo { ((v - lo) / (hi - lo)) as f32 } else { 0.5 };
                Some((w * n as f32 / last, h - t * h))
            })
            .collect()
    };
    (place(series), place(previous))
}

// ---- the connect form ----

/// The required boxes of a form that are empty: `values` by field key. A key
/// the daemon already keeps (`kept`, when replacing settings) may stay empty.
pub fn missing(spec: &SourceSpec, values: &BTreeMap<String, String>, kept: &[String]) -> Vec<String> {
    spec.fields.iter().filter(|f| f.required && !kept.contains(&f.key) && values.get(&f.key).is_none_or(|v| v.trim().is_empty())).map(|f| f.key.clone()).collect()
}

/// The settings to send: every box with something in it, trimmed.
pub fn settings(values: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    values.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k.clone(), v.trim().to_owned())).collect()
}

/// The query's bucket for a period: days up to a month, then weeks.
pub fn bucket(days: u32) -> Bucket {
    if days > 31 { Bucket::Week } else { Bucket::Day }
}

#[cfg(any(test, feature = "snapshot"))]
pub mod demo {
    //! A sample of the daemon's answers, for the screen's pictures and tests.
    use super::*;
    use equipe_core::protocol::metrics::{Field, Fetched, MetricDef, Share};

    fn field(key: &str, label: &str, secret: bool, hint: Option<&str>) -> Field {
        Field { key: key.into(), label: label.into(), secret, required: true, hint: hint.map(Into::into) }
    }

    fn optional(key: &str, label: &str, hint: &str) -> Field {
        Field { key: key.into(), label: label.into(), secret: false, required: false, hint: Some(hint.into()) }
    }

    fn def(name: &str, label: &str, unit: &str, shape: Shape) -> MetricDef {
        MetricDef { name: name.into(), label: label.into(), unit: unit.into(), shape }
    }

    pub fn kinds() -> Vec<SourceSpec> {
        vec![
            SourceSpec { kind: SourceKind::Posthog, label: "PostHog".into(), fields: vec![field("api_key", "Personal API key", true, None), field("project_id", "Project ID", false, None), optional("north_star_event", "North Star event", "The event of an approval: work_accepted when empty"), optional("north_star_property", "North Star counts", "The event property counted once each: squad when empty"), optional("north_star_where", "North Star only", "Event properties, comma-separated: by=person when empty")], metrics: vec![def("visits", "Visits", "page_loads", Shape::Count), def("languages", "Languages", "page_loads", Shape::Breakdown)], keeps_days: None },
            SourceSpec { kind: SourceKind::Github, label: "GitHub".into(), fields: vec![field("token", "Token", true, None), field("repo", "Repository", false, None)], metrics: vec![def("views", "Views", "views", Shape::Count), def("stars", "Stars", "stars", Shape::Total)], keeps_days: Some(14) },
            SourceSpec { kind: SourceKind::AppStore, label: "App Store Connect".into(), fields: vec![field("private_key", "Private key", true, None), field("app_id", "App ID", false, None)], metrics: vec![def("downloads", "Downloads", "downloads", Shape::Count)], keeps_days: None },
        ]
    }

    fn connection(id: &str, kind: SourceKind, error: Option<FetchError>) -> Connection {
        // 2026-11-02 07:00 UTC, near enough to any zone's morning.
        let at = 1_793_602_800;
        Connection { id: id.into(), squad: "demo".into(), project: Some("equipe-site".into()), kind, settings: BTreeMap::new(), keys: vec!["token".into()], connected_at: at - 86_400 * 9, last: Some(Fetched { at, error: error.clone() }), last_ok: Some(if error.is_some() { at - 86_400 } else { at }) }
    }

    fn line(values: &[f64]) -> Vec<Sample> {
        values.iter().enumerate().map(|(n, v)| Sample { day: format!("2026-10-{:02}", 27 + n), value: Some(*v) }).collect()
    }

    fn share(label: &str, value: f64, previous: f64) -> Share {
        Share { label: label.into(), value, previous: Some(previous) }
    }

    fn visits(connection: &str) -> MetricValue {
        MetricValue { connection: connection.into(), kind: SourceKind::Posthog, metric: "visits".into(), label: "Visits".into(), unit: "page_loads".into(), shape: Shape::Count, value: Some(1284.), previous: Some(1187.), change: Some(97.), change_pct: Some(8.2), as_of: Some("2026-11-02".into()), days_known: 7, series: line(&[160., 172., 181., 176., 190., 188., 217.]), previous_series: line(&[150., 158., 162., 170., 168., 175., 180.]), breakdown: vec![] }
    }

    fn languages(connection: &str) -> MetricValue {
        MetricValue { metric: "languages".into(), label: "Languages".into(), shape: Shape::Breakdown, value: Some(1284.), previous: Some(1187.), change: None, change_pct: None, series: vec![], previous_series: vec![], breakdown: vec![share("English", 742., 700.), share("Korean", 283., 240.), share("日本語", 112., 100.), share("Español", 64., 60.), share("Other", 83., 87.)], ..visits(connection) }
    }

    fn spots(connection: &str, metric: &str, label: &str, unit: &str, rows: &[(&str, f64)]) -> MetricValue {
        let all: f64 = rows.iter().map(|r| r.1).sum();
        MetricValue { metric: metric.into(), label: label.into(), unit: unit.into(), value: Some(all), previous: Some(all * 0.9), change_pct: Some(12.), change: Some(all * 0.1), breakdown: rows.iter().map(|(l, v)| share(l, *v, v * 0.9)).collect(), ..languages(connection) }
    }

    fn downloads(connection: &str) -> MetricValue {
        MetricValue { connection: connection.into(), kind: SourceKind::AppStore, metric: "downloads".into(), label: "Downloads".into(), unit: "downloads".into(), shape: Shape::Count, value: Some(63.), previous: Some(54.), change: Some(9.), change_pct: Some(17.), as_of: Some("2026-11-01".into()), days_known: 7, series: line(&[7., 9., 8., 11., 10., 9., 9.]), previous_series: line(&[6., 8., 7., 9., 8., 8., 8.]), breakdown: vec![] }
    }

    fn github(connection: &str, metric: &str, label: &str, unit: &str, shape: Shape, value: f64, pct: f64) -> MetricValue {
        MetricValue { connection: connection.into(), kind: SourceKind::Github, metric: metric.into(), label: label.into(), unit: unit.into(), shape, value: Some(value), previous: Some(value / (1. + pct / 100.)), change: Some(value - value / (1. + pct / 100.)), change_pct: Some(pct), as_of: Some("2026-11-02".into()), days_known: 7, series: line(&[310., 322., 318., 340., 335., 351., 334.]), previous_series: line(&[300., 305., 312., 309., 318., 321., 325.]), breakdown: vec![] }
    }

    fn query(values: Vec<MetricValue>, connections: Vec<Connection>) -> QueryResult {
        QueryResult { from: "2026-10-27".into(), to: "2026-11-02".into(), previous_from: "2026-10-20".into(), previous_to: "2026-10-26".into(), days: 7, bucket: Bucket::Day, values, connections }
    }

    /// The North Star: 12 windows of 7 days, the oldest first.
    pub fn north(connection: &str, windows: &[Option<f64>]) -> MetricValue {
        let last = windows.iter().flatten().last().copied();
        let before = windows.len().checked_sub(2).and_then(|n| windows[n]);
        let series = windows.iter().enumerate().map(|(n, v)| Sample { day: format!("2026-w{n:02}"), value: *v }).collect();
        MetricValue { connection: connection.into(), kind: SourceKind::Posthog, metric: NORTH_STAR.into(), label: "Squads approving each week".into(), unit: "squads".into(), shape: Shape::Rolling, value: last, previous: before, change: last.zip(before).map(|(a, b)| a - b), change_pct: last.zip(before).filter(|(_, b)| *b > 0.).map(|(a, b)| ((a - b) / b * 100.).round()), as_of: Some("2026-11-02".into()), days_known: 7, series, previous_series: vec![], breakdown: vec![] }
    }

    pub fn north_full() -> Vec<Option<f64>> {
        [2., 3., 3., 5., 4., 6., 7., 6., 8., 11., 11., 14.].iter().map(|v| Some(*v)).collect()
    }

    /// Everything connected and well.
    pub fn healthy() -> (Sources, QueryResult) {
        let connections = vec![connection("m1", SourceKind::Posthog, None), connection("m2", SourceKind::Github, None), connection("m3", SourceKind::AppStore, None)];
        let values = vec![
            visits("m1"),
            languages("m1"),
            spots("m1", "referrers", "Referrers", "page_loads", &[("Direct", 514.), ("X", 308.), ("GitHub", 231.), ("LinkedIn", 128.), ("Other", 103.)]),
            spots("m1", "download_clicks", "Download click spots", "clicks", &[("Hero button", 96.), ("Top menu", 34.), ("Install guide", 19.), ("Footer", 7.)]),
            github("m2", "views", "Views", "views", Shape::Count, 2310., 5.1),
            github("m2", "clones", "Clones", "clones", Shape::Count, 418., -2.3),
            github("m2", "stars", "Stars", "stars", Shape::Total, 25., 19.0),
            downloads("m3"),
            north("m1", &north_full()),
        ];
        (Sources { kinds: kinds(), connections: connections.clone() }, query(values, connections))
    }

    /// The key of one service refused, the other asked too often.
    pub fn failing() -> (Sources, QueryResult) {
        let auth = FetchError::new(ErrorCode::Auth, "key ••• was refused");
        let limit = FetchError { retry_at: Some(1_793_602_800 + 7 * 3600), ..FetchError::new(ErrorCode::RateLimited, "slow down") };
        let connections = vec![connection("m1", SourceKind::Posthog, Some(auth)), connection("m2", SourceKind::Github, Some(limit))];
        let values = vec![north("m1", &north_full()), visits("m1"), languages("m1"), github("m2", "stars", "Stars", "stars", Shape::Total, 25., 19.0)];
        (Sources { kinds: kinds(), connections: connections.clone() }, query(values, connections))
    }

    pub fn nothing() -> Sources {
        Sources { kinds: kinds(), connections: vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_get_separators_and_small_ones_keep_a_decimal() {
        assert_eq!(number(1284.), "1,284");
        assert_eq!(number(1_234_567.), "1,234,567");
        assert_eq!(number(14.), "14");
        assert_eq!(number(8.24), "8.2");
        assert_eq!(number(0.), "0");
        assert_eq!(number(-3.), "−3");
    }

    #[test]
    fn the_north_star_is_its_own_card_in_six_states() {
        let (s, q) = demo::healthy();
        assert!(cards(&q, &s).iter().all(|c| c.metric != NORTH_STAR));
        let Some(NorthStar::Ready(c)) = north_star(Some(&q), &s, false) else { panic!("ready") };
        assert_eq!((c.value.as_deref(), c.bars.len(), c.short()), (Some("14"), NORTH_BARS, false));
        assert_eq!(c.change.as_ref().map(|c| c.text.as_str()), Some("3 (+27%)"));
        // Asked, not answered: loading; answered without it: no definition.
        let mut none = q.clone();
        none.values.retain(|m| m.metric != NORTH_STAR);
        assert_eq!(north_star(Some(&none), &s, true), Some(NorthStar::Loading));
        assert_eq!(north_star(Some(&none), &s, false), Some(NorthStar::Undefined));
        // Nothing from PostHog connected: no card at all.
        assert_eq!(north_star(Some(&q), &Sources { connections: vec![], ..s.clone() }, false), None);
        // Few windows: short, padded to twelve with unknowns at the left.
        let mut few = q.clone();
        few.values.iter_mut().find(|m| m.metric == NORTH_STAR).unwrap().series.truncate(2);
        let Some(NorthStar::Ready(c)) = north_star(Some(&few), &s, false) else { panic!("ready") };
        assert_eq!((c.known(), c.short(), c.bars.len()), (2, true, NORTH_BARS));
        // A key refused keeps the last value and says so.
        let (s, q) = demo::failing();
        let Some(NorthStar::Ready(c)) = north_star(Some(&q), &s, false) else { panic!("ready") };
        assert_eq!((c.error, c.value.as_deref()), (Some(ErrorCode::Auth), Some("14")));
    }

    #[test]
    fn a_zero_window_keeps_three_pixels() {
        assert_eq!(bar_height(0., 14., 72.), 3.);
        assert_eq!(bar_height(14., 14., 72.), 72.);
        assert_eq!(bar_height(0., 0., 72.), 3.);
    }

    #[test]
    fn a_change_is_signed_and_a_rise_is_up() {
        let (_, q) = demo::healthy();
        let c = change_of(&q.values[0]).unwrap();
        assert_eq!((c.up, c.text.as_str()), (Some(true), "+8.2%"));
        let mut fall = q.values[0].clone();
        fall.change_pct = Some(-2.3);
        assert_eq!(change_of(&fall).unwrap(), Change { up: Some(false), text: "−2.3%".into() });
        fall.change_pct = None;
        fall.change = Some(-4.);
        assert_eq!(change_of(&fall).unwrap().text, "−4");
        fall.change = None;
        assert_eq!(change_of(&fall), None);
    }

    #[test]
    fn a_breakdown_is_bars_scaled_to_the_biggest() {
        let (s, q) = demo::healthy();
        let cards = cards(&q, &s);
        let Body::Bars(bars) = &cards[1].body else { panic!("languages are bars") };
        assert_eq!((bars[0].label.as_str(), bars[0].value.as_str(), bars[0].fill), ("English", "742", 1.0));
        assert!((bars[1].fill - 283. / 742.).abs() < 1e-6);
        assert!(matches!(cards[0].body, Body::Number { .. }));
    }

    #[test]
    fn a_card_of_a_failed_connection_says_so_and_keeps_the_last_value() {
        let (s, q) = demo::failing();
        let cards = cards(&q, &s);
        assert_eq!(cards[0].error, Some(ErrorCode::Auth));
        assert_eq!(cards[2].error, Some(ErrorCode::RateLimited));
        assert!(cards[0].last_ok.is_some());
        let Body::Number { value, .. } = &cards[0].body else { panic!() };
        assert_eq!(value.as_deref(), Some("1,284"));
        let (healthy_s, healthy_q) = demo::healthy();
        assert!(super::cards(&healthy_q, &healthy_s).iter().all(|c| c.error.is_none()));
    }

    #[test]
    fn banners_come_from_failed_fetches_and_a_key_error_asks_for_a_new_key() {
        let (s, _) = demo::failing();
        let b = banners(&s);
        assert_eq!(b.iter().map(|b| b.error.code).collect::<Vec<_>>(), [ErrorCode::Auth, ErrorCode::RateLimited]);
        assert!(needs_key(ErrorCode::Auth) && needs_key(ErrorCode::Keychain));
        assert!(!needs_key(ErrorCode::RateLimited) && !needs_key(ErrorCode::Network));
        assert!(banners(&demo::healthy().0).is_empty());
    }

    #[test]
    fn a_busy_service_is_a_wait_in_its_own_words_never_the_internet() {
        let at = 1_793_602_800;
        let said = "Queries are a little too busy right now. We're working to free up resources. Please try again later.";
        let busy = FetchError { status: Some(503), said: Some(said.into()), retry_at: Some(at + 300), ..FetchError::new(ErrorCode::RateLimited, format!("PostHog said 503: {said}")) };
        let b = Banner { connection: "m1".into(), kind: SourceKind::Posthog, error: busy.clone(), last_ok: None, at };
        let (title, body) = banner_words(&b, "PostHog", at);
        assert_eq!(error_word(&busy), "busy");
        assert!(title.contains("PostHog") && !title.contains('{'), "{title}");
        assert!(body.contains(said), "PostHog's own words: {body}");
        assert!(!body.contains('{') && !body.trim_end().ends_with(" ."), "{body}");
        let retry = moment(at + 300, at);
        assert!(body.contains(&retry), "when it tries again: {body}");
        // A first fetch that failed: nothing earlier is shown, so nothing says where it is from.
        let shown = crate::i18n::catalog().fill("metrics.err.shown_from", &[("last", String::new())]);
        assert!(!body.contains(shown.trim()), "{body}");
        let earlier = Banner { last_ok: Some(at - 86_400), ..b.clone() };
        assert!(banner_words(&earlier, "PostHog", at).1.len() > body.len(), "with numbers from before, it says how old they are");
        // Every code with no numbers yet reads whole.
        for code in [ErrorCode::Network, ErrorCode::BadResponse, ErrorCode::Other, ErrorCode::RateLimited, ErrorCode::Auth, ErrorCode::Keychain] {
            let b = Banner { error: FetchError::new(code, "x"), ..b.clone() };
            let (title, body) = banner_words(&b, "PostHog", at);
            assert!(!title.contains('{') && !body.contains('{') && !body.contains("  "), "{code:?}: {title} / {body}");
        }
    }

    fn look(projects: &[(u64, &str)]) -> PosthogLook {
        let account = PosthogAccount { host: "https://us.posthog.com".into(), region: "us".into(), organization: "Atinum".into(), projects: projects.iter().map(|(id, name)| PosthogProject { id: *id, name: (*name).into() }).collect(), current: None };
        PosthogLook { browser: true, accounts: if projects.is_empty() { vec![] } else { vec![account] } }
    }

    #[test]
    fn set_it_up_signs_in_asks_only_with_a_choice_and_always_asks_the_yes() {
        assert_eq!(after_look(&PosthogLook::default()), Setup::NoBrowser);
        assert_eq!(after_look(&look(&[])), Setup::SignIn, "signed out: sign in, in equipe's browser");
        let Setup::Confirm(one) = after_look(&look(&[(735144, "Atinum MCP")])) else { panic!("one project: straight to the yes") };
        assert_eq!(one.project.id, 735144);
        assert!(matches!(after_look(&look(&[(1, "Site"), (2, "App")])), Setup::Choose(_)), "two: a choice card");
        // What the daemon answers the yes.
        let (account, project) = (one.account.clone(), one.project.clone());
        let answer = |outcome| PosthogSetup { outcome, keys_url: "https://us.posthog.com/settings/user-api-keys".into(), ..PosthogSetup::default() };
        assert!(matches!(after_setup(answer(SetupOutcome::Connected), &account, &project), Setup::Done { .. }));
        assert_eq!(after_setup(answer(SetupOutcome::SignIn), &account, &project), Setup::SignIn);
        assert_eq!(after_setup(answer(SetupOutcome::SignInAgain), &account, &project), Setup::SignInAgain { host: "https://us.posthog.com".into() });
        assert!(matches!(after_setup(answer(SetupOutcome::Manual), &account, &project), Setup::Manual { keys_url, .. } if keys_url.ends_with("/settings/user-api-keys")));
    }

    #[test]
    fn a_saved_posthog_key_suggests_metrics_until_it_is_connected_or_waved_off() {
        use equipe_core::protocol::secrets::SecretKind;
        let key = |name: &str, service: &str, at| SquadSecret { squad: "demo".into(), name: name.into(), kind: SecretKind::ApiKey, service: service.into(), projects: vec![], any_project: false, created_by: "dev".into(), created_ticket: Some(12), created_at: at, note: String::new(), last_used: None };
        let secrets = vec![key("vercel-token", "vercel", 1), key("posthog-project-key", "app.posthog.com", 2), key("ph", "PostHog", 3)];
        assert_eq!(posthog_suggestion("demo", &secrets, &[], &[]).map(|s| s.name.as_str()), Some("ph"), "the latest PostHog key");
        assert_eq!(posthog_suggestion("other", &secrets, &[], &[]), None, "another squad's keys aren't this one's");
        assert_eq!(posthog_suggestion("demo", &secrets, &[], &["ph".into(), "posthog-project-key".into()]), None, "not now");
        let (s, _) = demo::healthy();
        assert_eq!(posthog_suggestion("demo", &secrets, &s.connections, &[]), None, "PostHog is in Metrics already");
    }

    #[test]
    fn a_project_without_page_views_gets_one_card_that_says_so() {
        let (s, mut q) = demo::healthy();
        assert!(without_page_views(&q, &s).is_empty());
        for m in q.values.iter_mut().filter(|m| m.metric == "visits") {
            m.days_known = 0;
            m.value = None;
        }
        assert_eq!(without_page_views(&q, &s), ["m1"]);
        // A failed fetch isn't "no page views": it says it failed.
        let (failing, _) = demo::failing();
        assert!(without_page_views(&q, &failing).is_empty());
        // The North Star counting people of the project's own event is named so.
        let mut s = s;
        let c = s.connections.iter_mut().find(|c| c.kind == SourceKind::Posthog).unwrap();
        c.settings.insert("north_star_event".into(), "mcp_tool_called".into());
        assert_eq!(north_people_event(&s), None, "squads still, until it counts people");
        s.connections.iter_mut().find(|c| c.kind == SourceKind::Posthog).unwrap().settings.insert("north_star_property".into(), "distinct_id".into());
        assert_eq!(north_people_event(&s).as_deref(), Some("mcp_tool_called"));
    }

    #[test]
    fn sentences_join_with_a_space_except_after_a_full_width_stop() {
        assert_eq!(sentences(&["A.".into(), "B.".into()]), "A. B.");
        assert_eq!(sentences(&["あ。".into(), "い。".into()]), "あ。い。");
        assert_eq!(sentences(&["A.".into(), " ".into(), "B.".into()]), "A. B.");
    }

    #[test]
    fn nothing_connected_is_the_empty_state_only_once_the_daemon_said() {
        let mut m = Metrics::new();
        assert!(!m.is_empty());
        m.sources = Some(demo::nothing());
        assert!(m.is_empty());
        m.sources = Some(demo::healthy().0);
        assert!(!m.is_empty());
        assert_eq!(m.connection_of(SourceKind::Github).map(|c| c.id.as_str()), Some("m2"));
        assert!(m.connection_of(SourceKind::Fake).is_none());
    }

    #[test]
    fn the_form_asks_for_every_required_box_and_sends_only_what_is_filled() {
        let spec = demo::kinds().remove(0);
        let mut values = BTreeMap::new();
        assert_eq!(missing(&spec, &values, &[]), ["api_key", "project_id"]);
        values.insert("api_key".into(), "  phx_x  ".into());
        values.insert("project_id".into(), " ".into());
        assert_eq!(missing(&spec, &values, &[]), ["project_id"]);
        values.insert("project_id".into(), "700951".into());
        assert!(missing(&spec, &values, &[]).is_empty());
        values.insert("api_key".into(), String::new());
        assert!(missing(&spec, &values, &["api_key".to_owned()]).is_empty(), "a kept key may stay empty");
        values.insert("api_key".into(), "  phx_x  ".into());
        assert_eq!(settings(&values).get("api_key").map(String::as_str), Some("phx_x"));
    }

    #[test]
    fn a_draft_is_debug_printed_without_any_value_to_leak() {
        // It has no field to hold one: what is typed stays in the boxes.
        let shown = format!("{:?}", Draft { kind: Some(SourceKind::Posthog), ..Draft::default() });
        assert!(shown.contains("Posthog") && !shown.contains("phx"));
    }

    #[test]
    fn the_sparkline_scales_both_lines_together_and_keeps_gaps() {
        let (a, b) = spark(&[Some(0.), None, Some(10.)], &[Some(5.), Some(5.), Some(5.)], 100., 20.);
        assert_eq!(a[0], Some((0., 20.)));
        assert_eq!(a[1], None);
        assert_eq!(a[2], Some((100., 0.)));
        assert_eq!(b[1], Some((50., 10.)));
        // One value, all the same: a line across the middle, no division by zero.
        let (flat, _) = spark(&[Some(3.), Some(3.)], &[], 10., 10.);
        assert_eq!(flat[0], Some((0., 5.)));
    }

    #[test]
    fn months_and_days_read_off_a_day() {
        assert_eq!(month_day("2026-11-02"), Some((11, 2)));
        assert_eq!(month_day("nope"), None);
        assert_eq!(day_words("2026-11-02"), "11월 2일");
        assert_eq!(bucket(7), Bucket::Day);
        assert_eq!(bucket(90), Bucket::Week);
    }
}
