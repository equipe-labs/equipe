//! `/usage`'s figures (`views/usage.rs` draws them): how full a plan's
//! window is and when it starts over, and the members, those that spent the
//! most first.

use crate::state::{Model, Presence};
use equipe_core::t;

/// How full a plan's window is, as its bar is coloured: room, getting full,
/// nearly out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    Room,
    Full,
    Out,
}

pub fn fill(percent: u8) -> Fill {
    match percent {
        0..=69 => Fill::Room,
        70..=89 => Fill::Full,
        _ => Fill::Out,
    }
}

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

/// A member as the usage panel lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberUse {
    pub name: String,
    pub role: String,
    pub engine: Option<equipe_core::protocol::Engine>,
    /// Its context's fill, once known.
    pub context: Option<u8>,
    pub tokens: u64,
    /// The share of what it read that came from the prompt cache.
    pub cache: Option<u8>,
}

/// The squad's open members, those that spent the most first.
pub fn members(model: &Model) -> Vec<MemberUse> {
    let mut all: Vec<MemberUse> = model
        .members
        .iter()
        .filter(|m| m.presence != Presence::Closed)
        .map(|m| {
            let spent = model.spent.get(&m.session).copied().unwrap_or_default();
            MemberUse {
                name: m.name.clone(),
                role: m.role.clone(),
                engine: m.engine,
                context: spent.context.filter(|(_, w)| *w > 0).map(|(u, w)| (u * 100 / w).min(100) as u8),
                tokens: spent.tokens,
                cache: (spent.seen.1 * 100).checked_div(spent.seen.0).map(|p| p.min(100) as u8),
            }
        })
        .collect();
    all.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.name.cmp(&b.name)));
    all
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
    use equipe_core::protocol::{Access, Engine, Event, Member as Who, SessionInfo, SessionState, Snapshot, TurnOutcome, Usage};

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
            compacting: None,
            visuals: Vec::new(),
        };
        let mut m = Model::new("acme");
        m.apply(Update::Attached(Hello { version: "x".into(), snapshot: Snapshot { squad: "acme".into(), sessions: vec![session(1, "pm"), session(2, "frontend")], ..Default::default() } }), T0);
        m
    }



    #[test]
    fn a_plan_is_coloured_by_how_full_it_is_and_says_how_long_until_it_starts_over() {
        assert_eq!([fill(0), fill(53), fill(70), fill(89), fill(90), fill(100)], [Fill::Room, Fill::Room, Fill::Full, Fill::Full, Fill::Out, Fill::Out]);
        assert_eq!(time_left(1_000 + 3 * 3_600 + 20 * 60, 1_000), "3시간 20분");
        assert_eq!(time_left(1_000 + 2 * 86_400 + 4 * 3_600, 1_000), "2일 4시간");
        assert_eq!(time_left(1_030, 1_000), "1분", "never \"0\"");
        assert_eq!(time_left(500, 1_000), "1분", "past: it starts over any moment");
    }

    #[test]
    fn members_are_listed_by_what_they_spent_most() {
        let mut m = model();
        let usage = Usage { input_tokens: Some(100_000), output_tokens: Some(2_000), cache_tokens: Some(900_000), context_tokens: Some(126_000), context_window: Some(200_000) };
        m.apply(Update::Event(Event::TurnEnded { session: 2, outcome: TurnOutcome::Completed, usage: Some(usage) }), T0);
        let all = members(&m);
        assert_eq!(all.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["frontend", "pm"]);
        assert_eq!((all[0].context, all[0].cache), (Some(63), Some(89)));
        assert_eq!((all[1].context, all[1].tokens), (None, 0), "nothing yet: no bar, no guess");
    }

    #[test]
    fn counts_are_short() {
        assert_eq!([short_count(999), short_count(8_400), short_count(12_000), short_count(2_800_000)], ["999", "8.4k", "12k", "2.8M"]);
    }
}
