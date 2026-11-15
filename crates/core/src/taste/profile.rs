//! The style profile: the person's style in short, at the top of a project's
//! `design/taste.md`, read first by every design ticket and session.
//!
//! ```text
//! ## Style profile
//! _Updated 2026-11-16 · 24 picks_
//!
//! - Density: airy, generous whitespace
//! - Tone: calm, editorial
//! - Type: large serif headings over a quiet sans
//! - Color use: one accent on warm neutrals
//! - Liked: Product A pricing; Product B onboarding
//! - Disliked: busy dashboards with many colours
//! - Leans: density -60 (70%), contrast +20 (40%)
//! <!-- /style profile -->
//! ```
//!
//! The worded lines are the designer's (after each round of a session, from
//! what the person said); `Leans` is equipe's, worked out from the taste
//! tuner's picks (`super::tuner`). The end mark keeps the notes after it
//! notes (`super::notes`). Pure: no disk here.

use crate::protocol::design::{Lean, StyleProfile, TasteAxis};

pub const HEADING: &str = "Style profile";
pub const END: &str = "<!-- /style profile -->";
/// The most liked and disliked examples kept, newest first.
pub const EXAMPLES_MAX: usize = 8;
/// The longest a worded line may be, in characters.
pub const LINE_MAX: usize = 160;

/// Whether a heading's words are the profile's.
pub fn is_heading(heading: &str) -> bool {
    heading.trim().eq_ignore_ascii_case(HEADING)
}

/// Whether a line ends the profile.
pub fn is_end(line: &str) -> bool {
    line.trim().eq_ignore_ascii_case(END)
}

/// A field of the profile, as its line names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Density,
    Tone,
    Type,
    Color,
    Liked,
    Disliked,
}

impl Field {
    pub const ALL: [Field; 6] = [Field::Density, Field::Tone, Field::Type, Field::Color, Field::Liked, Field::Disliked];

    /// Its label in the file.
    pub fn label(self) -> &'static str {
        match self {
            Field::Density => "Density",
            Field::Tone => "Tone",
            Field::Type => "Type",
            Field::Color => "Color use",
            Field::Liked => "Liked",
            Field::Disliked => "Disliked",
        }
    }

    pub fn parse(word: &str) -> Option<Field> {
        let w = word.trim().to_ascii_lowercase();
        match w.as_str() {
            "density" => Some(Field::Density),
            "tone" => Some(Field::Tone),
            "type" | "typography" => Some(Field::Type),
            "color" | "colour" | "color use" | "colour use" => Some(Field::Color),
            "liked" | "references liked" => Some(Field::Liked),
            "disliked" | "references disliked" => Some(Field::Disliked),
            _ => None,
        }
    }
}

fn clip(text: &str) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > LINE_MAX { one.chars().take(LINE_MAX).collect::<String>().trim_end().to_owned() } else { one }
}

fn list(text: &str) -> Vec<String> {
    text.split(';').map(clip).filter(|s| !s.is_empty()).take(EXAMPLES_MAX).collect()
}

/// `density -60 (70%)`, and back.
fn lean_text(l: &Lean) -> String {
    format!("{} {:+} ({}%)", l.axis.key(), l.lean, l.confidence)
}

fn parse_lean(text: &str) -> Option<Lean> {
    let (head, rest) = text.trim().split_once(' ')?;
    let axis = TasteAxis::parse(head)?;
    let (lean, conf) = rest.trim().split_once('(')?;
    let lean: i32 = lean.trim().trim_start_matches('+').parse().ok()?;
    let confidence: u32 = conf.trim().trim_end_matches(')').trim_end_matches('%').trim().parse().ok()?;
    Some(Lean { axis, lean: lean.clamp(-100, 100) as i8, confidence: confidence.min(100) as u8 })
}

/// The profile in `text`, when it has one.
pub fn parse(text: &str) -> Option<StyleProfile> {
    let mut lines = text.lines().skip_while(|l| !l.trim_start().starts_with('#') || !is_heading(l.trim_start().trim_start_matches('#')));
    lines.next()?;
    let mut p = StyleProfile::default();
    for line in lines {
        let t = line.trim();
        if is_end(t) || t.starts_with('#') {
            break;
        }
        if let Some(meta) = t.strip_prefix('_').and_then(|m| m.strip_suffix('_')) {
            for part in meta.split('·').map(str::trim) {
                if let Some(date) = part.strip_prefix("Updated ").map(str::trim).filter(|d| super::notes::is_date(d)) {
                    p.updated = Some(date.to_owned());
                } else if let Some(n) = part.strip_suffix(" picks").or_else(|| part.strip_suffix(" pick")).and_then(|n| n.trim().parse().ok()) {
                    p.picks = n;
                }
            }
            continue;
        }
        let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) else { continue };
        let Some((key, value)) = item.split_once(':') else { continue };
        if key.trim().eq_ignore_ascii_case("leans") {
            p.leans = value.split(',').filter_map(parse_lean).collect();
            continue;
        }
        let Some(field) = Field::parse(key) else { continue };
        set(&mut p, field, value);
    }
    Some(p)
}

/// Set one worded field: a line's words, or a list's (`;`-separated).
pub fn set(p: &mut StyleProfile, field: Field, value: &str) {
    match field {
        Field::Density => p.density = clip(value),
        Field::Tone => p.tone = clip(value),
        Field::Type => p.typography = clip(value),
        Field::Color => p.color = clip(value),
        Field::Liked => p.liked = list(value),
        Field::Disliked => p.disliked = list(value),
    }
}

/// An example the person liked (or disliked): first in its list, out of the
/// other one, the oldest let go past [`EXAMPLES_MAX`].
pub fn add_example(p: &mut StyleProfile, example: &str, liked: bool) {
    let example = clip(&example.replace(';', ","));
    if example.is_empty() {
        return;
    }
    let same = |e: &String| e.eq_ignore_ascii_case(&example);
    p.liked.retain(|e| !same(e));
    p.disliked.retain(|e| !same(e));
    let to = if liked { &mut p.liked } else { &mut p.disliked };
    to.insert(0, example);
    to.truncate(EXAMPLES_MAX);
}

/// The profile as its section's lines, heading to end mark.
pub fn lines(p: &StyleProfile) -> Vec<String> {
    let mut out = vec![format!("## {HEADING}")];
    let mut meta = Vec::new();
    if let Some(d) = &p.updated {
        meta.push(format!("Updated {d}"));
    }
    if p.picks > 0 {
        meta.push(format!("{} picks", p.picks));
    }
    if !meta.is_empty() {
        out.push(format!("_{}_", meta.join(" · ")));
    }
    out.push(String::new());
    for field in Field::ALL {
        let value = match field {
            Field::Density => p.density.clone(),
            Field::Tone => p.tone.clone(),
            Field::Type => p.typography.clone(),
            Field::Color => p.color.clone(),
            Field::Liked => p.liked.join("; "),
            Field::Disliked => p.disliked.join("; "),
        };
        out.push(if value.is_empty() { format!("- {}:", field.label()) } else { format!("- {}: {value}", field.label()) });
    }
    if !p.leans.is_empty() {
        out.push(format!("- Leans: {}", p.leans.iter().map(lean_text).collect::<Vec<_>>().join(", ")));
    }
    out.push(END.to_owned());
    out
}

/// `previous` with its profile made `p`: in the place of the one there, else
/// at the top — after the file's title and the paragraph under it.
pub fn write(previous: &str, p: &StyleProfile) -> String {
    let all: Vec<&str> = previous.lines().collect();
    let start = all.iter().position(|l| l.trim_start().starts_with('#') && is_heading(l.trim_start().trim_start_matches('#')));
    let mut out: Vec<String> = Vec::new();
    match start {
        Some(start) => {
            let end = all[start + 1..].iter().position(|l| is_end(l) || l.trim_start().starts_with('#')).map_or(all.len(), |n| {
                let at = start + 1 + n;
                if is_end(all[at]) { at + 1 } else { at }
            });
            out.extend(all[..start].iter().map(|l| (*l).to_owned()));
            out.extend(lines(p));
            out.extend(all[end..].iter().map(|l| (*l).to_owned()));
        }
        None => {
            // After a `# Title` and the paragraph under it, else first.
            let mut at = 0;
            if all.first().is_some_and(|l| l.starts_with("# ")) {
                at = 1;
                while at < all.len() && all[at].trim().is_empty() {
                    at += 1;
                }
                while at < all.len() && !all[at].trim().is_empty() && !all[at].trim_start().starts_with('#') && !all[at].trim_start().starts_with("- ") {
                    at += 1;
                }
            }
            out.extend(all[..at].iter().map(|l| (*l).to_owned()));
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.extend(lines(p));
            if at < all.len() {
                out.push(String::new());
            }
            out.extend(all[at..].iter().map(|l| (*l).to_owned()));
        }
    }
    // No run of blank lines wider than one.
    let mut tidy: Vec<String> = Vec::new();
    for line in out {
        if line.trim().is_empty() && tidy.last().is_some_and(|l: &String| l.trim().is_empty()) {
            continue;
        }
        tidy.push(line);
    }
    while tidy.last().is_some_and(|l| l.trim().is_empty()) {
        tidy.pop();
    }
    format!("{}\n", tidy.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taste::notes;

    const FILE: &str = "# Taste\n\nThe person's taste, a note a line.\n\n- Round the corners _(the person, 2026-11-03)_\n- Never pure black _(noticed on T-4, 2026-11-05)_\n";

    #[test]
    fn a_profile_is_written_at_the_top_and_read_back_as_it_was() {
        assert_eq!(parse(FILE), None, "none yet");
        let mut p = StyleProfile { density: "airy".into(), tone: "calm, editorial".into(), picks: 24, updated: Some("2026-11-16".into()), ..Default::default() };
        p.leans = vec![Lean { axis: TasteAxis::Density, lean: -60, confidence: 70 }, Lean { axis: TasteAxis::Contrast, lean: 20, confidence: 40 }];
        add_example(&mut p, "Product A pricing", true);
        add_example(&mut p, "Busy dashboards; many colours", false);
        let written = write(FILE, &p);
        assert!(written.starts_with("# Taste\n\nThe person's taste, a note a line.\n\n## Style profile\n_Updated 2026-11-16 · 24 picks_\n"), "{written}");
        assert!(written.contains("- Leans: density -60 (70%), contrast +20 (40%)\n<!-- /style profile -->\n\n- Round the corners"), "{written}");
        let back = parse(&written).unwrap();
        assert_eq!(back.disliked, ["Busy dashboards, many colours"], "a `;` inside an example can't split it");
        assert_eq!(back, StyleProfile { disliked: vec!["Busy dashboards, many colours".into()], ..p.clone() });
        // The notes are still the notes: the profile's lines are none of them.
        assert_eq!(notes::parse(&written).iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["Round the corners", "Never pure black"]);
        // Written again, it takes the old one's place; the rest stays.
        let again = write(&written, &StyleProfile { tone: "warm".into(), ..p.clone() });
        assert_eq!(again.matches("## Style profile").count(), 1);
        assert!(again.contains("- Tone: warm\n") && again.ends_with("- Never pure black _(noticed on T-4, 2026-11-05)_\n"), "{again}");
    }

    #[test]
    fn notes_written_after_a_profile_stay_notes() {
        let with = write("# Taste\n", &StyleProfile { density: "dense".into(), ..Default::default() });
        let changed = notes::change(&notes::parse(&with), &crate::protocol::design::TasteChange::Add { text: "More contrast".into() }, "2026-11-16").unwrap();
        let written = notes::write(&with, &changed);
        assert_eq!(notes::parse(&written).len(), 1, "{written}");
        assert_eq!(parse(&written).unwrap().density, "dense");
        // A profile someone left without its end mark ends at the next heading.
        let open = "## Style profile\n- Tone: calm\n## Notes\n- Big type\n";
        assert_eq!(parse(open).unwrap().tone, "calm");
        assert_eq!(notes::parse(open).len(), 1);
    }

    #[test]
    fn examples_keep_newest_first_once_each_and_move_between_lists() {
        let mut p = StyleProfile::default();
        for n in 0..12 {
            add_example(&mut p, &format!("ex {n}"), true);
        }
        assert_eq!(p.liked.len(), EXAMPLES_MAX);
        assert_eq!(p.liked[0], "ex 11");
        add_example(&mut p, "EX 11", false);
        assert!(!p.liked.iter().any(|e| e.eq_ignore_ascii_case("ex 11")) && p.disliked == ["EX 11"]);
        add_example(&mut p, "   ", true);
        assert_eq!(p.liked.len(), EXAMPLES_MAX - 1);
        let mut q = StyleProfile::default();
        set(&mut q, Field::parse("colour").unwrap(), "  one   accent ");
        assert_eq!(q.color, "one accent");
        assert_eq!(parse_lean("type_scale +130 (250%)"), Some(Lean { axis: TasteAxis::TypeScale, lean: 100, confidence: 100 }), "held to its range");
        assert_eq!(parse_lean("nonsense 3 (4%)"), None);
    }
}
