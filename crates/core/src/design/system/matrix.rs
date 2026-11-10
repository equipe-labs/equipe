//! A component's matrix: every variant its source declares, in every state
//! its pattern has, light and dark, at each width — drawn from its docs page
//! in headless Chrome (`matrix.mjs`) into a contact sheet, with axe-core's
//! accessibility rules and a keyboard pass run on each example, and judged
//! here: what couldn't be shown, a state that looks no different, text under
//! AA in a state, axe's violations, keys that don't do what the pattern
//! says. Said in plain words; kept in `design/checks/components/`.
//!
//! The docs page marks each example `data-example="variant=outline,size=sm"`
//! (any words, the variants as `axis=value`); the part a person uses is
//! found inside by the catalog's selector, or marked `data-target`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::catalog::{self, Component, Open};
use crate::design::{contrast, values};

/// The themes a matrix is drawn in by default.
pub const THEMES: [&str; 2] = ["light", "dark"];

/// The line a run ends with: this word, then the summary as JSON.
pub const MARK: &str = "equipe-design-matrix:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub component: String,
    pub url: String,
    pub target: Option<String>,
    pub open: Option<Open>,
    pub states: Vec<String>,
    pub keyboard: Vec<String>,
    pub themes: Vec<String>,
    pub widths: Vec<u32>,
    /// Each variant axis its source declares, and its values.
    pub variants: Vec<(String, Vec<String>)>,
}

/// The plan for `comp`, drawn from `url`, its variants read from its source.
pub fn plan(comp: &Component, source: Option<&str>, url: &str, widths: &[u32], themes: &[String]) -> Plan {
    Plan {
        component: comp.name.clone(),
        url: url.to_owned(),
        target: comp.target.clone(),
        open: comp.open.clone(),
        states: comp.states.clone(),
        keyboard: comp.keyboard.clone(),
        themes: themes.to_vec(),
        widths: widths.to_vec(),
        variants: source.map(variants).unwrap_or_default(),
    }
}

/// How many pictures the plan asks for, an example at a time.
pub fn per_example(plan: &Plan) -> usize {
    plan.states.len() * plan.themes.len() * plan.widths.len()
}

/// The object after `key` in `text`, braces matched: `{ … }`.
fn object_after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let at = text.find(key)? + key.len();
    let rest = &text[at..];
    let open = rest.find('{')?;
    if !rest[..open].trim().trim_start_matches(':').trim().is_empty() {
        return object_after(rest, key);
    }
    let mut depth = 0;
    let mut quote: Option<char> = None;
    for (i, c) in rest[open..].char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&rest[open + 1..open + i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The keys at the top of an object's body, each with what follows its colon.
fn keys(body: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut quote: Option<char> = None;
    let mut start = 0;
    for (i, c) in body.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                push_key(&body[start..i], &mut out);
                start = i + 1;
            }
            _ => {}
        }
    }
    push_key(&body[start..], &mut out);
    out
}

fn push_key<'a>(entry: &'a str, out: &mut Vec<(String, &'a str)>) {
    let entry = entry.trim();
    let Some((k, v)) = entry.split_once(':') else { return };
    let k = k.trim().trim_matches(['"', '\'', '`']).trim();
    if !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        out.push((k.to_owned(), v));
    }
}

/// The variants a component's source declares, as `cva(…, { variants: { … } })`
/// or tailwind-variants' `tv({ variants: { … } })` write them.
pub fn variants(source: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut rest = source;
    while let Some(body) = object_after(rest, "variants") {
        for (axis, value) in keys(body) {
            let Some(inner) = value.trim().strip_prefix('{').and_then(|v| v.rfind('}').map(|e| &v[..e])) else { continue };
            let names: Vec<String> = keys(inner).into_iter().map(|(k, _)| k).collect();
            if !names.is_empty() && !out.iter().any(|(a, _)| *a == axis) {
                out.push((axis, names));
            }
        }
        let at = rest.find("variants").map_or(rest.len(), |a| a + "variants".len());
        rest = &rest[at..];
    }
    out
}

// ── what matrix.mjs measured ──

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Text {
    pub text: String,
    pub fg: String,
    pub bg: Option<String>,
    pub size: f64,
    pub weight: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StateShot {
    pub state: String,
    /// It could be put in that state.
    pub applied: bool,
    pub why: Option<String>,
    pub picture: Option<String>,
    /// How it looked: the computed style that tells states apart.
    pub look: String,
    pub texts: Vec<Text>,
    /// Focus showed (an outline, or a look of its own) when focused.
    pub focus_shows: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shot {
    pub example: String,
    /// The part a person uses was found inside it.
    pub target: bool,
    pub states: Vec<StateShot>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AxeNode {
    pub target: Vec<String>,
    #[serde(rename = "failureSummary")]
    pub failure_summary: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Violation {
    pub id: String,
    pub impact: Option<String>,
    pub help: String,
    #[serde(rename = "helpUrl")]
    pub help_url: Option<String>,
    pub nodes: Vec<AxeNode>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Axe {
    pub ran: bool,
    pub why: Option<String>,
    pub violations: Vec<Violation>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Key {
    pub example: String,
    pub check: String,
    pub pass: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Run {
    pub width: u32,
    pub theme: String,
    pub examples: Vec<Shot>,
    pub axe: Axe,
    pub keyboard: Vec<Key>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Measured {
    pub url: String,
    pub runs: Vec<Run>,
    /// The contact sheets drawn, one per width.
    pub sheets: Vec<String>,
}

// ── judged ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Fix before the component is released.
    Fail,
    /// Worth a look; never blocking.
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub level: Level,
    /// `coverage`, `state`, `contrast`, `axe:<rule>`, `key:<check>`, `tokens`.
    pub check: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    pub words: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Report {
    pub component: String,
    pub url: String,
    /// When it was checked, in unix seconds.
    pub at: u64,
    pub widths: Vec<u32>,
    pub themes: Vec<String>,
    pub examples: Vec<String>,
    /// Pictures taken, of those the plan asked for.
    pub shown: (usize, usize),
    pub axe: bool,
    pub findings: Vec<Finding>,
    pub sheets: Vec<String>,
}

impl Report {
    pub fn passes(&self) -> bool {
        !self.findings.iter().any(|f| f.level == Level::Fail)
    }

    pub fn fails(&self) -> usize {
        self.findings.iter().filter(|f| f.level == Level::Fail).count()
    }

    /// As a person reads it.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for level in [Level::Fail, Level::Note] {
            let found: Vec<&Finding> = self.findings.iter().filter(|f| f.level == level).collect();
            if found.is_empty() {
                continue;
            }
            out.push_str(if level == Level::Fail { "To fix before it is released:\n" } else { "Worth a look:\n" });
            for f in found {
                let mut at = Vec::new();
                if let Some(e) = &f.example {
                    at.push(e.clone());
                }
                if let Some(t) = &f.theme {
                    at.push(t.clone());
                }
                if let Some(w) = f.width {
                    at.push(format!("@{w}"));
                }
                let at = if at.is_empty() { String::new() } else { format!(" [{}]", at.join(" ")) };
                out.push_str(&format!("  {} {}{at}\n", if level == Level::Fail { "✗" } else { "·" }, f.words));
            }
        }
        out.push_str(&format!(
            "\n{}: {} example(s), {}/{} picture(s), {} · {} · axe {} · {} to fix\n",
            self.component,
            self.examples.len(),
            self.shown.0,
            self.shown.1,
            self.themes.join(" and "),
            self.widths.iter().map(u32::to_string).collect::<Vec<_>>().join(", "),
            if self.axe { "run" } else { "not run" },
            self.fails()
        ));
        for s in &self.sheets {
            out.push_str(&format!("sheet: {s}\n"));
        }
        out
    }

    /// What a screen reads: [`MARK`] and a short summary.
    pub fn mark_line(&self) -> String {
        let summary = serde_json::json!({ "component": self.component, "passes": self.passes(), "fails": self.fails(), "shown": self.shown, "axe": self.axe });
        format!("{MARK} {summary}")
    }
}

fn colour(text: &str) -> Option<values::Color> {
    values::color(&serde_json::Value::String(text.to_owned()))
}

/// Whether an example shows an axis's value: `variant=outline` in its
/// words; an example that names no value of the axis shows its default.
fn shows(example: &str, axis: &str, value: &str) -> bool {
    let e = example.to_ascii_lowercase();
    let parts: Vec<&str> = e.split([',', ' ', ';']).map(str::trim).collect();
    let axis = axis.to_ascii_lowercase();
    let value = value.to_ascii_lowercase();
    parts.iter().any(|p| *p == format!("{axis}={value}") || *p == value) || value == "default" && !parts.iter().any(|p| p.starts_with(&format!("{axis}=")))
}

/// States that should look different from `default` once applied.
const DIFFERS: [&str; 5] = ["hover", "focus-visible", "active", "checked", "open"];

/// The plan's results judged.
pub fn judge(plan: &Plan, measured: &Measured, at: u64) -> Report {
    let cat = catalog::catalog();
    let mut report = Report { component: plan.component.clone(), url: plan.url.clone(), at, widths: plan.widths.clone(), themes: plan.themes.clone(), sheets: measured.sheets.clone(), ..Report::default() };
    let push = |level, check: String, example: Option<&str>, theme: Option<&str>, width: Option<u32>, words: String, out: &mut Vec<Finding>| {
        let f = Finding { level, check, example: example.map(str::to_owned), theme: theme.map(str::to_owned), width, words };
        if !out.contains(&f) {
            out.push(f);
        }
    };
    let mut findings = Vec::new();
    for run in &measured.runs {
        for shot in &run.examples {
            if !report.examples.contains(&shot.example) {
                report.examples.push(shot.example.clone());
            }
        }
    }
    if measured.runs.is_empty() || report.examples.is_empty() {
        push(
            Level::Fail,
            "coverage".into(),
            None,
            None,
            None,
            format!("{} shows no example to check: mark each one `data-example=\"variant=…\"` on its docs page.", plan.url),
            &mut findings,
        );
    }
    for (axis, values) in &plan.variants {
        for value in values {
            if !report.examples.is_empty() && !report.examples.iter().any(|e| shows(e, axis, value)) {
                push(Level::Fail, "coverage".into(), None, None, None, format!("the docs page doesn't show {axis}={value}, which the component declares."), &mut findings);
            }
        }
    }
    let expected = per_example(plan) * report.examples.len().max(1);
    let mut taken = 0;
    for run in &measured.runs {
        let theme = Some(run.theme.as_str());
        let width = Some(run.width);
        for shot in &run.examples {
            let ex = Some(shot.example.as_str());
            if plan.target.is_some() && !shot.target {
                push(Level::Fail, "coverage".into(), ex, None, None, format!("no part to use found in the example (`{}`, or mark it `data-target`).", plan.target.as_deref().unwrap_or("")), &mut findings);
            }
            let default = shot.states.iter().find(|s| s.state == "default" && s.applied);
            for state in &plan.states {
                let Some(s) = shot.states.iter().find(|s| s.state == *state) else {
                    push(Level::Fail, "state".into(), ex, theme, width, format!("{} wasn't drawn.", cat.state_words(state)), &mut findings);
                    continue;
                };
                if !s.applied {
                    push(Level::Fail, "state".into(), ex, theme, width, format!("couldn't show it {}: {}", cat.state_words(state), s.why.as_deref().unwrap_or("no reason given")), &mut findings);
                    continue;
                }
                if s.picture.is_some() {
                    taken += 1;
                }
                if let Some(d) = default
                    && DIFFERS.contains(&state.as_str())
                    && s.look == d.look
                    && !s.look.is_empty()
                {
                    let level = if state == "focus-visible" { Level::Fail } else { Level::Note };
                    push(level, "state".into(), ex, theme, None, format!("{} looks the same as at rest.", first_upper(cat.state_words(state))), &mut findings);
                }
                if state == "focus-visible" && s.focus_shows == Some(false) {
                    push(Level::Fail, "state".into(), ex, theme, None, "focus doesn't show when the keyboard reaches it.".into(), &mut findings);
                }
                // WCAG leaves disabled controls' text out of contrast.
                if state == "disabled" {
                    continue;
                }
                for t in &s.texts {
                    let (Some(fg), Some(bg)) = (colour(&t.fg), t.bg.as_deref().and_then(colour)) else { continue };
                    let ratio = contrast::ratio(contrast::over(fg, bg), bg);
                    let large = t.size >= 24. || t.size >= 18.66 && t.weight >= 700;
                    let need = if large { 3. } else { 4.5 };
                    if ratio + 0.005 < need {
                        push(
                            Level::Fail,
                            "contrast".into(),
                            ex,
                            theme,
                            None,
                            format!("“{}” is {ratio:.1}:1 {} — text needs {need}:1 ({} on {}).", t.text, cat.state_words(state), fg.hex(), bg.hex()),
                            &mut findings,
                        );
                    }
                }
            }
        }
        if !run.axe.ran {
            push(Level::Note, "axe".into(), None, None, None, format!("accessibility rules weren't run: {}", run.axe.why.as_deref().unwrap_or("axe-core isn't installed")), &mut findings);
        }
        for v in &run.axe.violations {
            let level = if matches!(v.impact.as_deref(), Some("critical" | "serious")) { Level::Fail } else { Level::Note };
            let nodes: Vec<String> = v.nodes.iter().take(3).map(|n| n.target.join(" ")).collect();
            push(
                level,
                format!("axe:{}", v.id),
                None,
                theme,
                None,
                format!("{} ({}{}) — {}", v.help.trim_end_matches('.'), v.id, v.impact.as_deref().map(|i| format!(", {i}")).unwrap_or_default(), nodes.join("; ")),
                &mut findings,
            );
        }
        for k in &run.keyboard {
            if !plan.keyboard.contains(&k.check) || k.pass {
                continue;
            }
            push(
                Level::Fail,
                format!("key:{}", k.check),
                Some(&k.example),
                theme,
                None,
                format!("{} — it didn't{}", first_upper(cat.keyboard_words(&k.check)), if k.detail.is_empty() { ".".to_owned() } else { format!(": {}", k.detail) }),
                &mut findings,
            );
        }
        for check in &plan.keyboard {
            if !report.examples.is_empty() && !run.keyboard.iter().any(|k| k.check == *check) {
                push(Level::Note, format!("key:{check}"), None, theme, None, format!("{} wasn't tried (no example it applies to).", first_upper(cat.keyboard_words(check))), &mut findings);
            }
        }
    }
    report.shown = (taken, expected);
    report.axe = measured.runs.iter().any(|r| r.axe.ran);
    findings.sort_by_key(|f| f.level);
    report.findings = findings;
    report
}

fn first_upper(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
}

/// Where a component's matrix is kept: `design/checks/components/<name>.json`, its pictures beside.
pub fn record_path(design: &Path, component: &str) -> PathBuf {
    design.join("checks").join("components").join(format!("{}.json", catalog::normal(component)))
}

pub fn pictures_dir(design: &Path, component: &str) -> PathBuf {
    design.join("checks").join("components").join(catalog::normal(component))
}

pub fn read(design: &Path, component: &str) -> Option<Report> {
    serde_json::from_str(&std::fs::read_to_string(record_path(design, component)).ok()?).ok()
}

pub fn write(design: &Path, report: &Report) -> std::io::Result<PathBuf> {
    let path = record_path(design, &report.component);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(report).unwrap_or_default()))?;
    Ok(path)
}
