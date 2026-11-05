//! The rules checked in the page as a browser draws it. `lint.mjs` opens
//! it in Chrome at each width and measures — what runs off the side, text
//! cut off, each text's colour and what it sits on, whether focus shows,
//! where the primary action is, how much is centred, the blocks' radius and
//! shadow, line lengths, target sizes — and writes them as JSON. The
//! judging is here, so it is the same rules and tested like the rest.

use serde::{Deserialize, Serialize};

use super::{Finding, Report, rules};
use crate::design::contrast;
use crate::design::values::{self, Color};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Measured {
    pub url: String,
    pub widths: Vec<AtWidth>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AtWidth {
    pub width: u32,
    /// The window's height: the first screenful.
    pub height: u32,
    pub scroll_width: f64,
    pub client_width: f64,
    /// What reaches past the window's right edge.
    pub wide: Vec<Wide>,
    pub clipped: Vec<Clipped>,
    pub texts: Vec<Text>,
    pub focus: Vec<Focus>,
    pub primary: Vec<Primary>,
    pub centred: Centred,
    pub blocks: Vec<Block>,
    pub lines: Vec<Line>,
    pub targets: Vec<Target>,
    /// The picture drawn at this width.
    pub picture: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wide {
    pub selector: String,
    pub right: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clipped {
    pub selector: String,
    pub text: String,
    /// `x` or `y`.
    pub axis: String,
    /// Cut with an ellipsis, on purpose.
    pub ellipsis: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Text {
    pub selector: String,
    pub text: String,
    /// Its colour, as the browser computed it: `rgb(…)`/`rgba(…)`.
    pub fg: String,
    /// What it sits on, the layers composed; `None` over a picture or gradient.
    pub bg: Option<String>,
    pub size: f64,
    pub weight: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Focus {
    pub selector: String,
    /// Whether focusing it changed how it looks (outline, shadow, border, background).
    pub visible: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Primary {
    pub selector: String,
    pub top: f64,
    pub bottom: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Centred {
    pub total: u32,
    pub centred: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Block {
    pub radius: String,
    pub shadow: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Line {
    pub selector: String,
    /// About how many characters a line of it holds.
    pub chars: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Target {
    pub selector: String,
    pub w: f64,
    pub h: f64,
}

/// Large text: 24px, or 18.66px bold (WCAG's 18pt and 14pt bold).
fn large(size: f64, weight: u32) -> bool {
    size >= 24. || (size >= 18.66 && weight >= 700)
}

fn colour(text: &str) -> Option<Color> {
    values::color(&serde_json::Value::String(text.trim().to_owned()))
}

/// The page's measures judged: the findings, and which rules ran.
pub fn judge(m: &Measured, report: &mut Report) {
    let file = m.url.clone();
    let mut found: Vec<Finding> = Vec::new();
    let mut primary_marked = false;
    for at in &m.widths {
        let w = Some(at.width);
        let mut push = |rule: &'static str, evidence: String| {
            let mut f = Finding::new(rule, &file, None, evidence);
            f.width = w;
            found.push(f);
        };
        if at.scroll_width > at.client_width + 1. {
            let what = at.wide.iter().take(2).map(|x| format!("{} (to {:.0}px)", x.selector, x.right)).collect::<Vec<_>>().join(", ");
            push("no-overflow", format!("the page is {:.0}px wide in a {}px window{}", at.scroll_width, at.width, if what.is_empty() { String::new() } else { format!(": {what}") }));
        }
        for c in at.clipped.iter().filter(|c| !c.ellipsis) {
            push("text-clipped", format!("{} cuts off “{}”", c.selector, clip(&c.text, 40)));
        }
        for t in &at.texts {
            let (Some(fg), Some(bg)) = (colour(&t.fg), t.bg.as_deref().and_then(colour)) else { continue };
            let bg = contrast::over(bg, Color::WHITE);
            let ratio = contrast::ratio(contrast::over(fg, bg), bg);
            let need = if large(t.size, t.weight) { 3. } else { 4.5 };
            if ratio + 0.005 < need {
                push("contrast", format!("{} “{}” is {ratio:.2}:1 ({} on {}), under {need}:1", t.selector, clip(&t.text, 30), fg.hex(), bg.hex()));
            }
        }
        for f in at.focus.iter().filter(|f| !f.visible) {
            push("focus-visible", format!("{} shows nothing when focused", f.selector));
        }
        for p in &at.primary {
            primary_marked = true;
            if p.top >= f64::from(at.height) {
                push("primary-in-view", format!("{} starts {:.0}px down, below the first {}px", p.selector, p.top, at.height));
            }
        }
        if at.width >= 768 && at.centred.total >= 6 && at.centred.centred * 5 >= at.centred.total * 4 {
            push("all-centred", format!("{} of {} text blocks are centred", at.centred.centred, at.centred.total));
        }
        if at.blocks.len() >= 4 {
            let first = &at.blocks[0];
            if first.shadow != "none" && at.blocks.iter().all(|b| b.radius == first.radius && b.shadow == first.shadow) {
                push("uniform-blocks", format!("all {} blocks have radius {} and the same shadow", at.blocks.len(), first.radius));
            }
        }
        if at.width >= 1024 {
            for l in at.lines.iter().filter(|l| l.chars > 90) {
                push("line-length", format!("{} runs to about {} characters a line", l.selector, l.chars));
            }
        }
        if at.width <= 430 {
            for t in at.targets.iter().filter(|t| t.w < 44. || t.h < 44.) {
                push("touch-target", format!("{} is {:.0}×{:.0}", t.selector, t.w, t.h));
            }
        }
    }
    report.findings.extend(collapse(found));
    report.widths = m.widths.iter().map(|w| w.width).collect();
    report.pictures.extend(m.widths.iter().filter_map(|w| w.picture.clone()));
    for r in rules::RULES.iter().filter(|r| r.how.in_render()) {
        if r.id == "primary-in-view" && !primary_marked {
            report.skipped.push((r.id.to_owned(), "nothing is marked data-primary".into()));
            continue;
        }
        if !report.checked.iter().any(|c| c == r.id) {
            report.checked.push(r.id.to_owned());
        }
    }
}

/// The same thing seen at several widths, said once with the widths.
fn collapse(found: Vec<Finding>) -> Vec<Finding> {
    let mut out: Vec<(Finding, Vec<u32>)> = Vec::new();
    for f in found {
        match out.iter_mut().find(|(g, _)| g.rule == f.rule && g.evidence == f.evidence) {
            Some((_, widths)) => widths.extend(f.width.filter(|w| !widths.contains(w))),
            None => {
                let widths = f.width.into_iter().collect();
                out.push((f, widths));
            }
        }
    }
    out.into_iter()
        .map(|(mut f, widths)| {
            if widths.len() > 1 {
                f.evidence.push_str(&format!(" (at {} px)", widths.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")));
            }
            f
        })
        .collect()
}

fn clip(text: &str, n: usize) -> String {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= n { t } else { format!("{}…", t.chars().take(n).collect::<String>()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::lint::Tier;

    fn page(widths: Vec<AtWidth>) -> Report {
        let mut r = Report::default();
        judge(&Measured { url: "design/mockups/a.html".into(), widths }, &mut r);
        r
    }

    fn at(width: u32) -> AtWidth {
        AtWidth { width, height: if width <= 480 { 844 } else { 800 }, scroll_width: f64::from(width), client_width: f64::from(width), ..AtWidth::default() }
    }

    fn rules_of(r: &Report) -> Vec<&str> {
        r.findings.iter().map(|f| f.rule.as_str()).collect()
    }

    #[test]
    fn a_clean_page_passes_every_render_rule_but_the_primary_it_wasnt_told_about() {
        let r = page(vec![at(360), at(1280)]);
        assert!(r.findings.is_empty(), "{:#?}", r.findings);
        assert!(r.checked.contains(&"no-overflow".to_owned()) && r.checked.contains(&"contrast".to_owned()));
        assert!(r.skipped.iter().any(|(id, _)| id == "primary-in-view"));
        assert_eq!(r.widths, vec![360, 1280]);
    }

    #[test]
    fn overflow_clipping_contrast_focus_and_a_low_primary_are_floor_findings_said_once_across_widths() {
        let mut narrow = at(360);
        narrow.scroll_width = 420.;
        narrow.wide = vec![Wide { selector: "table.prices".into(), right: 420. }];
        narrow.clipped = vec![Clipped { selector: "button.save".into(), text: "Save changes".into(), axis: "x".into(), ellipsis: false }, Clipped { selector: "td.name".into(), text: "A long name".into(), axis: "x".into(), ellipsis: true }];
        narrow.texts = vec![
            Text { selector: "p.hint".into(), text: "Optional".into(), fg: "rgb(170, 170, 170)".into(), bg: Some("rgb(255, 255, 255)".into()), size: 14., weight: 400 },
            Text { selector: "h1".into(), text: "Settings".into(), fg: "rgb(118, 118, 118)".into(), bg: Some("rgb(255, 255, 255)".into()), size: 32., weight: 700 },
            Text { selector: "p.over".into(), text: "On a photo".into(), fg: "rgb(200, 200, 200)".into(), bg: None, size: 14., weight: 400 },
        ];
        narrow.focus = vec![Focus { selector: "a.more".into(), visible: false }, Focus { selector: "button.save".into(), visible: true }];
        narrow.primary = vec![Primary { selector: "button.save".into(), top: 900., bottom: 944. }];
        let mut wide = at(1280);
        wide.texts = narrow.texts.clone();
        wide.primary = vec![Primary { selector: "button.save".into(), top: 300., bottom: 344. }];
        let r = page(vec![narrow, wide]);
        let rules = rules_of(&r);
        assert_eq!(rules.iter().filter(|r| **r == "contrast").count(), 1, "the grey hint, once for both widths; the large heading passes at 3:1; over a photo isn't measured: {:#?}", r.findings);
        let contrast = r.findings.iter().find(|f| f.rule == "contrast").unwrap();
        assert!(contrast.evidence.contains("p.hint") && contrast.evidence.contains("(at 360, 1280 px)"), "{}", contrast.evidence);
        for rule in ["no-overflow", "text-clipped", "focus-visible", "primary-in-view"] {
            assert!(rules.contains(&rule), "{rule}: {:#?}", r.findings);
        }
        assert!(!r.findings.iter().any(|f| f.evidence.contains("td.name")), "an ellipsis is a cut made on purpose");
        assert!(r.findings.iter().filter(|f| f.rule == "primary-in-view").all(|f| f.width == Some(360)));
        assert!(r.findings.iter().all(|f| f.tier == Tier::Floor));
        assert!(!r.skipped.iter().any(|(id, _)| id == "primary-in-view"));
    }

    #[test]
    fn centring_uniform_blocks_long_lines_and_small_targets_are_habits_and_notes_at_their_widths() {
        let mut phone = at(390);
        phone.centred = Centred { total: 10, centred: 10 };
        phone.targets = vec![Target { selector: "a.icon".into(), w: 24., h: 24. }];
        let mut desk = at(1440);
        desk.centred = Centred { total: 10, centred: 9 };
        desk.blocks = (0..4).map(|_| Block { radius: "16px".into(), shadow: "0 4px 12px rgba(0,0,0,.1)".into() }).collect();
        desk.lines = vec![Line { selector: "main p".into(), chars: 140 }];
        desk.targets = phone.targets.clone();
        let r = page(vec![phone, desk]);
        let found: Vec<(&str, Option<u32>, Tier)> = r.findings.iter().map(|f| (f.rule.as_str(), f.width, f.tier)).collect();
        assert!(found.contains(&("all-centred", Some(1440), Tier::Habit)), "centred on a phone is often right: {found:?}");
        assert!(!found.iter().any(|(r, w, _)| *r == "all-centred" && *w == Some(390)));
        assert!(found.contains(&("uniform-blocks", Some(1440), Tier::Habit)));
        assert!(found.contains(&("line-length", Some(1440), Tier::Finish)));
        assert!(found.contains(&("touch-target", Some(390), Tier::Finish)) && !found.contains(&("touch-target", Some(1440), Tier::Finish)));
    }

    #[test]
    fn measures_read_from_the_scripts_json() {
        let json = r#"{ "url": "http://localhost:3000/", "widths": [ { "width": 360, "height": 844, "scrollWidth": 360, "clientWidth": 360, "picture": "design/checks/home/360.png", "centred": { "total": 2, "centred": 1 } } ] }"#;
        let m: Measured = serde_json::from_str(json).unwrap();
        let r = page(m.widths);
        assert_eq!(r.pictures, vec!["design/checks/home/360.png"]);
    }
}
