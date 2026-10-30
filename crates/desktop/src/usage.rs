//! `/usage`: both engines' plan windows, the squad's engine switching, and each member's context and tokens,
//! as the console's usage screen words them (`tui/view.rs` `render_usage`) —
//! rows of text, without a window.

use equipe_core::protocol::PlanUsage;

use crate::state::{Model, Presence};
use equipe_core::t;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub text: String,
    /// Share of the bar to fill, 0–100, where the row has one.
    pub percent: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub plan: Vec<Row>,
    /// Codex's plan, as last read.
    pub codex: Vec<Row>,
    /// How the squad moves members between engines.
    pub switching: Row,
    pub members: Vec<Row>,
}

/// The plan's windows, each as "N% used · starts over <time>"; `when` words a
/// reset time (local time, which this module doesn't know).
pub fn plan_rows(plan: Option<PlanUsage>, when: impl Fn(i64) -> String) -> Vec<Row> {
    let Some(plan) = plan else {
        return vec![Row { label: "—".into(), text: t!("usage.not_yet").into(), percent: None }];
    };
    [(t!("usage.five_hour"), plan.five_hour), (t!("usage.week"), plan.seven_day)]
        .into_iter()
        .filter_map(|(label, window)| {
            let w = window?;
            Some(Row { label: label.into(), text: t!("usage.window", used = w.used_percent, when = when(w.resets_at)), percent: Some(w.used_percent.min(100)) })
        })
        .collect()
}

pub fn view(model: &Model, when: impl Fn(i64) -> String) -> View {
    let members = model
        .members
        .iter()
        .filter(|m| m.presence != Presence::Closed)
        .map(|m| {
            let spent = model.spent.get(&m.session).copied().unwrap_or_default();
            let (context, percent) = match spent.context.filter(|(_, w)| *w > 0) {
                Some((u, w)) => (t!("usage.context", pct = u * 100 / w, used = u / 1000, window = w / 1000), Some((u * 100 / w).min(100) as u8)),
                None => (t!("usage.context_unknown").to_owned(), None),
            };
            let cache = (spent.seen.1 * 100).checked_div(spent.seen.0).map_or_else(String::new, |p| t!("usage.from_cache", pct = p));
            Row { label: m.name.clone(), text: t!("usage.tokens", context = context, tokens = short_count(spent.tokens), cache = cache), percent }
        })
        .collect();
    let codex = match model.engines.plan(equipe_core::protocol::Engine::Codex) {
        Some(p) => plan_rows(Some(p.usage), &when),
        None => vec![Row { label: "—".into(), text: equipe_core::t!("engines.codex_not_read").into(), percent: None }],
    };
    let mode = model.engines.mode;
    let switching = Row { label: crate::switching::mode_word(mode).into(), text: format!("{} · /engines", crate::switching::mode_about(mode)), percent: None };
    View { plan: plan_rows(model.plan, &when), codex, switching, members }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::Update;
    use crate::state::Clock;
    use equipe_core::protocol::rpc::Hello;
    use equipe_core::protocol::{Access, Engine, Event, Member as Who, PlanWindow, SessionInfo, SessionState, Snapshot, TurnOutcome, Usage};

    const T0: Clock = Clock { hour: 14, minute: 2, second: 10 };

    fn model() -> Model {
        let session = |id, name: &str| SessionInfo {
            session: id,
            project: "/p".into(),
            member: Some(Who { role: name.into(), name: name.into() }),
            engine: Engine::Claude,
            model: None,
            access: Access::ReadOnly,
            workspace: None,
            state: SessionState::Ready,
            title: None,
            tokens: 0,
            context: None,
            recent: vec![],
            queued: vec![],
            queued_project: None,
            queued_in: vec![],
            steering: vec![],
            steering_in: vec![],
            lane: None,
            estimate: None,
            work: None,
            mode: None,
            effort: None,
            subagents: Vec::new(),
        };
        let mut m = Model::new("acme");
        m.apply(Update::Attached(Hello { version: "x".into(), snapshot: Snapshot { squad: "acme".into(), sessions: vec![session(1, "pm"), session(2, "frontend")], ..Default::default() } }), T0);
        m
    }

    #[test]
    fn the_plan_says_when_it_isnt_known_yet_and_then_each_window() {
        let mut m = model();
        let when = |t: i64| format!("@{t}");
        assert_eq!(view(&m, when).plan[0].text, "멤버가 Claude Code에서 다음 턴을 마치면 알 수 있어요");
        m.apply(Update::Event(Event::PlanUsage { usage: PlanUsage { five_hour: Some(PlanWindow { used_percent: 42, resets_at: 100 }), seven_day: None }, engine: equipe_core::protocol::Engine::Claude }), T0);
        let plan = view(&m, when).plan;
        assert_eq!(plan.len(), 1);
        assert_eq!((plan[0].label.as_str(), plan[0].text.as_str(), plan[0].percent), ("5시간", "42% 사용 · @100에 초기화", Some(42)));
    }

    #[test]
    fn members_show_context_tokens_and_the_share_from_cache() {
        let mut m = model();
        assert_eq!(view(&m, |_| String::new()).members[0].text, "컨텍스트 — · 토큰 0");
        let usage = Usage { input_tokens: Some(100_000), output_tokens: Some(2_000), cache_tokens: Some(900_000), context_tokens: Some(126_000), context_window: Some(200_000) };
        m.apply(Update::Event(Event::TurnEnded { session: 2, outcome: TurnOutcome::Completed, usage: Some(usage) }), T0);
        let rows = view(&m, |_| String::new()).members;
        assert_eq!(rows[1].label, "frontend");
        assert_eq!(rows[1].text, "컨텍스트 63% (126k / 200k) · 토큰 1M · 89% 캐시");
        assert_eq!(rows[1].percent, Some(63));
    }

    #[test]
    fn counts_are_short() {
        assert_eq!([short_count(999), short_count(8_400), short_count(12_000), short_count(2_800_000)], ["999", "8.4k", "12k", "2.8M"]);
    }
}
