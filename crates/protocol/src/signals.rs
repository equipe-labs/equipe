//! Signals as screens show them: each project's checks of what the squad
//! measures and ships — a shell command, a PostHog event count, App Store
//! downloads — with their latest readings. The PM proposes one and the
//! person turns it on on a card; the person may pause one or start it again
//! from a screen ([`crate::Command::SetSignalOn`]).

use serde::{Deserialize, Serialize};

/// Where a signal reads its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalSource {
    /// A shell command, run in the project's folder inside a fence.
    #[default]
    Command,
    /// A count of PostHog events.
    Posthog,
    /// App Store Connect downloads.
    AppStore,
}

/// One run of a signal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalReading {
    /// Unix seconds.
    pub at: u64,
    pub ok: bool,
    /// What it read, on one line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Why it fails, when it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalView {
    /// The project it watches, as the board names it.
    pub project: String,
    pub name: String,
    #[serde(default)]
    pub source: SignalSource,
    /// What it reads, in one line: the command, or `PostHog 700951 · $pageview · 1d`.
    pub what: String,
    /// What it is expected to give: `≥ 1`, `exit 0`, `= "success"`.
    pub expect: String,
    /// Seconds between runs.
    pub every: u64,
    /// Paused by the person: kept, not run.
    #[serde(default = "on")]
    pub on: bool,
    /// The latest readings, oldest first (at most [`VIEW_READINGS`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readings: Vec<SignalReading>,
}

fn on() -> bool {
    true
}

/// Readings a screen is sent per signal: enough for a trend.
pub const VIEW_READINGS: usize = 30;

impl SignalReading {
    /// The number its value starts with: `1,234`, `25`, `-3.5 stars`.
    pub fn number(&self) -> Option<f64> {
        let first = self.value.as_deref()?.split_whitespace().next()?.trim_matches('"').replace(',', "");
        first.parse().ok().filter(|n: &f64| n.is_finite())
    }
}

impl SignalView {
    pub fn last(&self) -> Option<&SignalReading> {
        self.readings.last()
    }

    /// Failing now: its latest reading failed.
    pub fn failing(&self) -> bool {
        self.last().is_some_and(|r| !r.ok)
    }

    /// How far the number moved from the first reading shown to the last, in percent.
    pub fn trend_pct(&self) -> Option<f64> {
        let first = self.readings.iter().find_map(SignalReading::number)?;
        let last = self.readings.iter().rev().find_map(SignalReading::number)?;
        if self.readings.iter().filter(|r| r.number().is_some()).count() < 2 || first == 0.0 {
            return None;
        }
        Some((last - first) / first.abs() * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_reads_back_and_says_how_it_trends() {
        let r = |at, n: u32| SignalReading { at, ok: true, value: Some(format!("{n}")), note: None };
        let view = SignalView {
            project: "equipe-site".into(),
            name: "pageviews".into(),
            source: SignalSource::Posthog,
            what: "PostHog 700951 · $pageview · 1d".into(),
            expect: "≥ 1".into(),
            every: 21600,
            on: true,
            readings: vec![r(1, 100), r(2, 150)],
        };
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["source"], "posthog");
        assert_eq!(serde_json::from_value::<SignalView>(json).unwrap(), view);
        assert_eq!(view.trend_pct(), Some(50.0));
        assert!(!view.failing());
        let bare: SignalView = serde_json::from_str(r#"{"project":"p","name":"n","what":"true","expect":"exit 0","every":60}"#).unwrap();
        assert!(bare.on && bare.readings.is_empty() && bare.source == SignalSource::Command);
        assert_eq!(bare.trend_pct(), None);
    }
}
