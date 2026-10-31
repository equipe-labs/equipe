//! The squad's signals, as the daemon reads them: PostHog pageviews rising,
//! App Store downloads, a command that passes, a workflow that fails, and one
//! the person paused. `FAKE_SIGNALS=0`: none, for the empty state.
//! `SetSignalOn` is answered as the daemon does: the whole list again.

use equipe_core::protocol::signals::{SignalReading, SignalSource, SignalView};
use equipe_core::protocol::{Command, Event};
use tokio::sync::broadcast;

/// The signals now.
pub fn signals() -> Vec<SignalView> {
    all().lock().unwrap().clone()
}

fn all() -> &'static std::sync::Mutex<Vec<SignalView>> {
    static ALL: std::sync::OnceLock<std::sync::Mutex<Vec<SignalView>>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| std::sync::Mutex::new(if std::env::var("FAKE_SIGNALS").is_ok_and(|v| v == "0") { Vec::new() } else { samples() }))
}

fn samples() -> Vec<SignalView> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // `n` readings `every` apart, the last `last_ago` ago, each from `read(i)`.
    let series = |n: usize, every: u64, last_ago: u64, read: &dyn Fn(usize) -> (bool, Option<String>, Option<String>)| -> Vec<SignalReading> {
        (0..n)
            .map(|i| {
                let (ok, value, note) = read(i);
                SignalReading { at: now - last_ago - (n - 1 - i) as u64 * every, ok, value, note }
            })
            .collect()
    };
    let pageviews = [412, 398, 455, 470, 438, 502, 561, 540, 603, 655, 640, 702, 768, 815];
    let downloads = [38, 41, 36, 44, 52, 49, 57, 61];
    vec![
        SignalView {
            project: "equipe-site".into(),
            name: "pageviews".into(),
            source: SignalSource::Posthog,
            what: "PostHog 700951 · $pageview · 1d".into(),
            expect: "≥ 1".into(),
            every: 21_600,
            on: true,
            readings: series(pageviews.len(), 86_400, 2 * 3600, &|i| (true, Some(format!("{}", pageviews[i])), None)),
        },
        SignalView {
            project: "equipe-app".into(),
            name: "downloads".into(),
            source: SignalSource::AppStore,
            what: "App Store 6740000000 · 7d".into(),
            expect: "≥ 1".into(),
            every: 86_400,
            on: true,
            readings: series(downloads.len(), 7 * 86_400, 9 * 3600, &|i| (true, Some(format!("{}", downloads[i])), None)),
        },
        SignalView {
            project: "equipe-site".into(),
            name: "site-analytics".into(),
            source: SignalSource::Command,
            what: "curl -s https://equipe.im/ | grep -c analytics-config".into(),
            expect: "≥ 1".into(),
            every: 3600,
            on: true,
            readings: series(6, 3600, 25 * 60, &|_| (true, Some("1".into()), None)),
        },
        SignalView {
            project: "equipe".into(),
            name: "traffic-workflow".into(),
            source: SignalSource::Command,
            what: "gh run list --workflow traffic.yml --limit 1 --json conclusion -q '.[0].conclusion'".into(),
            expect: "= \"success\"".into(),
            every: 21_600,
            on: true,
            readings: series(8, 21_600, 40 * 60, &|i| {
                if i >= 6 { (false, Some("\"failure\"".into()), Some("The traffic job failed: GITHUB_TOKEN can't read repository traffic (403)".into())) } else { (true, Some("\"success\"".into()), None) }
            }),
        },
        SignalView {
            project: "equipe-app".into(),
            name: "crash-free".into(),
            source: SignalSource::Command,
            what: "./scripts/crash-free.sh --days 7".into(),
            expect: "≥ 99".into(),
            every: 86_400,
            on: false,
            readings: series(5, 86_400, 3 * 86_400, &|i| (true, Some(format!("99.{} %", 4 + i)), None)),
        },
    ]
}

/// The person's answer, as the daemon makes it. Whether it was one.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    let Command::SetSignalOn { squad, project, name, on } = command else { return false };
    let signals = {
        let mut all = all().lock().unwrap();
        if let Some(s) = all.iter_mut().find(|s| s.project == *project && s.name == *name) {
            s.on = *on;
        }
        all.clone()
    };
    let _ = events.send(Event::SignalsChanged { squad: squad.clone(), signals });
    true
}
