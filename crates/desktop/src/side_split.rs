//! The sidebar's three areas, one over the other — the projects (with what
//! was done lately), the plans' usage, the team — and the hairlines between
//! them, which the person drags to give one more room and the other less.
//! The projects take what is left; usage and the team keep the height they
//! were dragged to (none dragged: their own), each scrolling inside when it
//! holds more. Double-clicking a hairline puts both back to their own. Kept
//! per Mac (`Remembered::side_usage`, `side_team`). Plain functions, so the
//! rules are tested without a window.

/// The least each area keeps: a few rows of projects, usage's heading and a
/// line, the team's heading and a member.
pub const PROJECTS_LEAST: f32 = 120.;
pub const USAGE_LEAST: f32 = 48.;
pub const TEAM_LEAST: f32 = 60.;

/// Which hairline is held: over usage (under the projects), or over the
/// team (under usage — under the projects when there's no usage).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    Usage,
    Team,
}

/// The areas' heights as drawn, when a hairline was taken (usage `None`
/// when it isn't shown).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Drawn {
    pub projects: f32,
    pub usage: Option<f32>,
    pub team: f32,
}

/// What a drag of `dy` (down positive) from where it began makes of the
/// heights the person sets: (usage, team). The hairline follows the pointer,
/// pushing the next area along once the one beside it is at its least —
/// usage's hairline down past usage's least shrinks the team, the team's up
/// past usage's least takes from the projects — as far as every area keeps
/// its least.
pub fn dragged(held: Divider, at: Drawn, dy: f32) -> (Option<f32>, f32) {
    let Some(usage) = at.usage else {
        // No usage: the team's hairline is between the projects and the team.
        let dy = dy.clamp((PROJECTS_LEAST - at.projects).min(0.), (at.team - TEAM_LEAST).max(0.));
        return (None, at.team - dy);
    };
    let usage_slack = (usage - USAGE_LEAST).max(0.);
    let team_slack = (at.team - TEAM_LEAST).max(0.);
    let projects_slack = (at.projects - PROJECTS_LEAST).max(0.);
    match held {
        // Up: usage grows from the projects. Down: usage shrinks, then the team.
        Divider::Usage if dy < 0. => (Some(usage + (-dy).min(projects_slack)), at.team),
        Divider::Usage => {
            let dy = dy.min(usage_slack + team_slack);
            let from_usage = dy.min(usage_slack);
            (Some(usage - from_usage), at.team - (dy - from_usage))
        }
        // Down: usage grows from the team. Up: the team grows from usage, then the projects.
        Divider::Team if dy > 0. => {
            let dy = dy.min(team_slack);
            (Some(usage + dy), at.team - dy)
        }
        Divider::Team => {
            let up = (-dy).min(usage_slack + projects_slack);
            let from_usage = up.min(usage_slack);
            (Some(usage - from_usage), at.team + up)
        }
    }
}

/// The set heights within `room` (the three areas' together): usage and
/// the team as set, as far as the projects keep their least — the team
/// giving way first, then usage — never under their own least.
pub fn fit(usage: Option<f32>, team: Option<f32>, room: f32) -> (Option<f32>, Option<f32>) {
    let mut usage = usage.map(|u| u.max(USAGE_LEAST));
    let mut team = team.map(|t| t.max(TEAM_LEAST));
    let over = usage.unwrap_or(0.) + team.unwrap_or(0.) + PROJECTS_LEAST - room;
    if over > 0. {
        let mut left = over;
        if let Some(t) = team.as_mut() {
            let give = (*t - TEAM_LEAST).min(left).max(0.);
            *t -= give;
            left -= give;
        }
        if let Some(u) = usage.as_mut() {
            let give = (*u - USAGE_LEAST).min(left).max(0.);
            *u -= give;
        }
    }
    (usage, team)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Drawn = Drawn { projects: 400., usage: Some(90.), team: 200. };

    #[test]
    fn the_hairline_over_usage_gives_the_projects_room_pushing_the_team_too() {
        assert_eq!(dragged(Divider::Usage, AT, -30.), (Some(120.), 200.), "up: usage grows from the projects");
        assert_eq!(dragged(Divider::Usage, AT, -500.), (Some(90. + 400. - PROJECTS_LEAST), 200.), "the projects keep their least");
        assert_eq!(dragged(Divider::Usage, AT, 20.), (Some(70.), 200.), "down: usage shrinks");
        assert_eq!(dragged(Divider::Usage, AT, 100.), (Some(USAGE_LEAST), 200. - (100. - (90. - USAGE_LEAST))), "past usage's least, the team gives");
        assert_eq!(dragged(Divider::Usage, AT, 1000.), (Some(USAGE_LEAST), TEAM_LEAST), "both down to their least");
    }

    #[test]
    fn the_hairline_over_the_team_moves_room_from_usage_then_the_projects() {
        assert_eq!(dragged(Divider::Team, AT, 60.), (Some(150.), 140.), "down: usage grows, the team gives");
        assert_eq!(dragged(Divider::Team, AT, 500.), (Some(90. + 200. - TEAM_LEAST), TEAM_LEAST), "the team keeps its least");
        assert_eq!(dragged(Divider::Team, AT, -40.), (Some(50.), 240.), "up: the team grows, usage gives");
        assert_eq!(dragged(Divider::Team, AT, -100.), (Some(USAGE_LEAST), 300.), "past usage's least, the projects give");
        assert_eq!(dragged(Divider::Team, AT, -1000.), (Some(USAGE_LEAST), 200. + (90. - USAGE_LEAST) + (400. - PROJECTS_LEAST)), "as far as the projects keep their least");
    }

    #[test]
    fn with_no_usage_the_team_takes_room_from_the_projects() {
        let at = Drawn { usage: None, ..AT };
        assert_eq!(dragged(Divider::Team, at, -100.), (None, 300.));
        assert_eq!(dragged(Divider::Team, at, -1000.), (None, 200. + 400. - PROJECTS_LEAST));
    }

    #[test]
    fn a_shorter_window_takes_from_the_team_first_then_usage() {
        assert_eq!(fit(Some(100.), Some(300.), 1000.), (Some(100.), Some(300.)), "room enough: as set");
        assert_eq!(fit(Some(100.), Some(300.), 400.), (Some(100.), Some(180.)), "the team gives first");
        assert_eq!(fit(Some(100.), Some(300.), 200.), (Some(USAGE_LEAST), Some(TEAM_LEAST)), "both down to their least");
        assert_eq!(fit(None, None, 100.), (None, None), "none set: their own");
        assert_eq!(fit(Some(10.), None, 1000.), (Some(USAGE_LEAST), None), "never under the least");
    }
}
