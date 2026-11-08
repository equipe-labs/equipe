//! The directions a designer has proposed and the person chose, each by its
//! key choices — layout, type, palette, density, motion — kept in
//! `design/directions/log.json`, so one design doesn't drift into the last.
//!
//! A new proposal differs from each of the recent chosen ones in at least
//! one of those, unless the brand locks every one it could change; and
//! options brought together differ in more than their colour.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DIMENSIONS: [&str; 5] = ["layout", "type", "palette", "density", "motion"];

/// How many chosen directions back a new one is held against.
pub const RECENT: usize = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Direction {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    /// When it was proposed, in unix seconds.
    pub at: u64,
    pub chosen: bool,
    /// The layout's shape: "two columns, list and detail", "one wide column".
    pub layout: String,
    /// The type pairing: "Inter 15/24 with tabular figures".
    #[serde(rename = "type")]
    pub type_: String,
    pub palette: String,
    pub density: String,
    pub motion: String,
    /// The references on the board it draws on (`R1`, `R3`), from
    /// `design/references/board.json`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
}

impl Direction {
    pub fn get(&self, dimension: &str) -> &str {
        match dimension {
            "layout" => &self.layout,
            "type" => &self.type_,
            "palette" => &self.palette,
            "density" => &self.density,
            _ => &self.motion,
        }
    }

    pub fn missing(&self) -> Vec<&'static str> {
        DIMENSIONS.iter().copied().filter(|d| self.get(d).trim().is_empty()).collect()
    }

    /// The dimensions it differs from `other` in.
    pub fn differs(&self, other: &Direction) -> Vec<&'static str> {
        DIMENSIONS.iter().copied().filter(|d| norm(self.get(d)) != norm(other.get(d))).collect()
    }
}

fn norm(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn path(design: &Path) -> PathBuf {
    design.join("directions").join("log.json")
}

pub fn read(design: &Path) -> Vec<Direction> {
    std::fs::read_to_string(path(design)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn write(design: &Path, log: &[Direction]) -> std::io::Result<()> {
    let file = path(design);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, format!("{}\n", serde_json::to_string_pretty(log).unwrap_or_default()))
}

/// The dimensions `brand.md` locks, from what its `## Locked` lines name.
pub fn locked(design: &Path) -> Vec<&'static str> {
    let lines = super::check::locked(design);
    let mut out = Vec::new();
    let says = |words: &[&str]| lines.iter().any(|l| words.iter().any(|w| l.contains(w)));
    if says(&["font", "type", "typeface", "family"]) {
        out.push("type");
    }
    if says(&["colour", "color", "palette", "#"]) {
        out.push("palette");
    }
    if says(&["layout", "grid", "column"]) {
        out.push("layout");
    }
    if says(&["density", "spacing"]) {
        out.push("density");
    }
    if says(&["motion", "animation", "easing"]) {
        out.push("motion");
    }
    out
}

/// What keeps `proposal` from joining `log`, a line each; empty when nothing does.
pub fn check(log: &[Direction], proposal: &Direction, locked: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let missing = proposal.missing();
    if !missing.is_empty() {
        out.push(format!("Name its {}: a direction is its key choices, each said.", missing.join(", ")));
        return out;
    }
    let free: Vec<&str> = DIMENSIONS.iter().copied().filter(|d| !locked.contains(d)).collect();
    let recent: Vec<&Direction> = log.iter().rev().filter(|d| d.chosen && d.name != proposal.name).take(RECENT).collect();
    if !free.is_empty() {
        for before in recent {
            let differs: Vec<&str> = proposal.differs(before).into_iter().filter(|d| free.contains(d)).collect();
            if differs.is_empty() {
                out.push(format!(
                    "It repeats “{}”{} in every free dimension ({}): change at least one, and say how it differs.",
                    before.name,
                    before.ticket.as_deref().map(|t| format!(" ({t})")).unwrap_or_default(),
                    free.join(", ")
                ));
            }
        }
    }
    if let Some(ticket) = &proposal.ticket {
        for other in log.iter().filter(|d| d.ticket.as_ref() == Some(ticket) && d.name != proposal.name) {
            let differs = proposal.differs(other);
            if differs.iter().all(|d| *d == "palette") && free.iter().any(|d| *d != "palette") {
                out.push(format!("It differs from “{}” only in colour: options differ in structure — layout, type, density or motion.", other.name));
            }
        }
    }
    out
}

/// `proposal` in the log: new, or over its earlier self.
pub fn record(log: &mut Vec<Direction>, proposal: Direction) {
    match log.iter_mut().find(|d| d.name == proposal.name && d.ticket == proposal.ticket) {
        Some(d) => *d = Direction { chosen: d.chosen, ..proposal },
        None => log.push(proposal),
    }
}

/// The person chose `name` (of `ticket`, if given): it is what the next is held against.
pub fn choose(log: &mut [Direction], name: &str, ticket: Option<&str>) -> bool {
    let Some(at) = log.iter().rposition(|d| d.name == name && ticket.is_none_or(|t| d.ticket.as_deref() == Some(t))) else { return false };
    log[at].chosen = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(name: &str, ticket: &str, dims: [&str; 5], chosen: bool) -> Direction {
        Direction {
            name: name.into(),
            ticket: Some(ticket.into()).filter(|t: &String| !t.is_empty()),
            at: 0,
            chosen,
            layout: dims[0].into(),
            type_: dims[1].into(),
            palette: dims[2].into(),
            density: dims[3].into(),
            motion: dims[4].into(),
            references: Vec::new(),
        }
    }

    const QUIET: [&str; 5] = ["list and detail", "Inter 15/24", "ink on bone, blue accent", "compact", "150ms fades"];

    #[test]
    fn a_new_direction_differs_from_each_recent_chosen_one_unless_the_brand_locks_it() {
        let log = vec![d("Quiet", "T-3", QUIET, true), d("Loud", "T-3", ["one column", "Fraunces", "red", "airy", "springs"], false)];
        let same = d("Again", "T-9", ["List and  detail", "inter 15/24", "Ink on bone, blue accent", "compact", "150ms fades"], false);
        let said = check(&log, &same, &[]);
        assert!(said.len() == 1 && said[0].contains("repeats “Quiet” (T-3)"), "{said:?}");
        let moved = d("Dense", "T-9", ["list and detail", "Inter 15/24", "ink on bone, blue accent", "dense table", "150ms fades"], false);
        assert!(check(&log, &moved, &[]).is_empty());
        // Everything it could change is the brand's.
        assert!(check(&log, &same, &DIMENSIONS).is_empty());
        // What is locked doesn't count as a difference.
        let only_type = d("Serif", "T-9", ["list and detail", "Söhne", "ink on bone, blue accent", "compact", "150ms fades"], false);
        assert!(check(&log, &only_type, &["type"]).iter().any(|s| s.contains("repeats")));
    }

    #[test]
    fn options_brought_together_differ_in_more_than_colour_and_each_says_all_five() {
        let mut log = vec![d("A", "T-9", QUIET, false)];
        let recoloured = d("B", "T-9", ["list and detail", "Inter 15/24", "green", "compact", "150ms fades"], false);
        assert!(check(&log, &recoloured, &[]).iter().any(|s| s.contains("only in colour")));
        assert!(check(&log, &recoloured, &["layout", "type", "density", "motion"]).is_empty(), "colour is all the brand leaves free");
        let half = Direction { name: "C".into(), layout: "grid".into(), ..Direction::default() };
        assert!(check(&log, &half, &[])[0].contains("type, palette, density, motion"));
        record(&mut log, d("B", "T-9", ["cards", "Inter", "green", "airy", "none"], false));
        assert!(choose(&mut log, "B", Some("T-9")));
        assert!(log[1].chosen && !choose(&mut log, "Z", None));
        record(&mut log, d("B", "T-9", ["cards", "Inter", "green", "airy", "slow"], false));
        assert!(log[1].chosen && log[1].motion == "slow" && log.len() == 2, "a proposal written again keeps its choice");
    }

    #[test]
    fn the_brands_locked_lines_lock_dimensions() {
        let dir = std::env::temp_dir().join(format!("equipe-directions-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brand.md"), "# Brand\n## Locked\n- page #F6F5F1, ink #15161A\n- Inter, the only typeface\n## Voice\nplain").unwrap();
        assert_eq!(locked(&dir), vec!["type", "palette"]);
    }
}
