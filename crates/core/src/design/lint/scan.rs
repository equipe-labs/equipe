//! The rules checked in a source as written: its declarations, its
//! elements and its words. Each check says where and what it saw.

use std::collections::HashMap;

use super::source::{Decl, Kind, Node, Source, Tag};
use super::{Finding, System};
use crate::design::values::{self, Color};

/// How many findings of one rule a file reports before it counts the rest.
const SHOWN: usize = 12;

/// Why a source rule can't be run against this system, if it can't.
pub fn skipped(rule: &str, system: &System) -> Option<String> {
    let none = || "no design/tokens.json found beside or above the files".to_owned();
    match rule {
        "token-colour" if !system.found => Some(none()),
        "token-colour" if system.colours.is_empty() => Some("tokens.json has no colours".into()),
        "token-space" if !system.found => Some(none()),
        "token-space" if system.spacing.is_empty() => Some("tokens.json has no spacing scale".into()),
        "token-type" if !system.found => Some(none()),
        "token-type" if system.sizes.is_empty() && system.families.is_empty() => Some("tokens.json has no type scale or families".into()),
        "literal-on-scale" if !system.found => Some(none()),
        _ => None,
    }
}

pub fn scan(source: &Source, system: &System, token_file: bool) -> Vec<Finding> {
    let mut out = Vec::new();
    if !token_file {
        tokens_used(source, system, &mut out);
    }
    focus(source, &mut out);
    motion(source, system, &mut out);
    if source.kind != Kind::Css {
        invented(source, &mut out);
        names(source, &mut out);
        eyebrows(source, &mut out);
        coloured_word(source, &mut out);
        nested_cards(source, &mut out);
        emoji(source, &mut out);
        centred(source, &mut out);
        dots(source, &mut out);
        forms(source, &mut out);
        headings(source, &mut out);
        images(source, &mut out);
        figures(source, &mut out);
    }
    tracked_caps(source, &mut out);
    gradient_hero(source, &mut out);
    uniform(source, &mut out);
    cap(out)
}

/// At most [`SHOWN`] of each rule, the last saying how many more.
fn cap(findings: Vec<Finding>) -> Vec<Finding> {
    let mut count: HashMap<String, usize> = HashMap::new();
    for f in &findings {
        *count.entry(f.rule.clone()).or_default() += 1;
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();
    for mut f in findings {
        let n = seen.entry(f.rule.clone()).or_default();
        *n += 1;
        let total = count[&f.rule];
        if *n > SHOWN {
            continue;
        }
        if *n == SHOWN && total > SHOWN {
            f.evidence.push_str(&format!(" (and {} more like it)", total - SHOWN));
        }
        out.push(f);
    }
    out
}

// ── walking the elements ──

/// An element open around a node: its tag, and the words in it so far.
struct Open<'a> {
    tag: &'a Tag,
    words: String,
    expr: bool,
    /// What it holds that names it: an image's alt, an svg's title, a child's label.
    named: bool,
}

/// A step of a walk: a node reached, or an element just closed with what it held.
enum Step<'s, 'a> {
    At(&'a Node),
    Done(&'s Open<'a>),
}

/// Each node with the elements open around it, outermost first; then each
/// element as it closes.
fn walk<'a>(source: &'a Source, mut each: impl FnMut(Step<'_, 'a>, &[Open<'a>])) {
    let mut stack: Vec<Open<'a>> = Vec::new();
    for node in &source.nodes {
        each(Step::At(node), &stack);
        match node {
            Node::Open(tag) => {
                let names = tag.attr("aria-label").is_some_and(|v| !v.trim().is_empty()) || (tag.name == "img" && tag.attr("alt").is_some_and(|a| !a.trim().is_empty())) || tag.name == "title";
                if names && let Some(parent) = stack.last_mut() {
                    parent.named = true;
                }
                let void = tag.self_closing || (source.kind != Kind::Script && super::source::is_void(&tag.name));
                if !void {
                    stack.push(Open { tag, words: String::new(), expr: false, named: false });
                }
            }
            Node::Close { name, .. } => {
                if let Some(at) = stack.iter().rposition(|o| o.tag.name == *name) {
                    while stack.len() > at {
                        let done = stack.pop().expect("there");
                        each(Step::Done(&done), &stack);
                        if let Some(parent) = stack.last_mut() {
                            parent.words.push(' ');
                            parent.words.push_str(&done.words);
                            parent.expr |= done.expr;
                            parent.named |= done.named;
                        }
                    }
                }
            }
            Node::Text { text, expr, .. } => {
                if let Some(top) = stack.last_mut() {
                    top.words.push_str(text);
                    top.expr |= *expr;
                }
            }
            Node::Comment { .. } => {}
        }
    }
}

fn tags(source: &Source) -> impl Iterator<Item = &Tag> {
    source.nodes.iter().filter_map(|n| if let Node::Open(t) = n { Some(t) } else { None })
}

/// A utility class without its variants: `md:hover:bg-red-500` → `bg-red-500`.
fn base_class(class: &str) -> &str {
    class.rsplit(':').next().unwrap_or(class).trim_start_matches('!')
}

fn words_of(text: &str) -> usize {
    text.split_whitespace().count()
}

fn clip(text: &str, n: usize) -> String {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= n { t } else { format!("{}…", t.chars().take(n).collect::<String>()) }
}

// ── token-colour, token-type, token-space, literal-on-scale ──

const COLOUR_PROPS: [&str; 18] = [
    "color", "background", "background-color", "background-image", "border", "border-color", "border-top", "border-bottom", "border-left", "border-right", "outline", "outline-color", "fill", "stroke", "box-shadow", "text-shadow", "caret-color", "accent-color",
];

const SPACE_PROPS: [&str; 18] = [
    "margin", "margin-top", "margin-bottom", "margin-left", "margin-right", "margin-inline", "margin-block", "margin-inline-start", "margin-inline-end", "padding", "padding-top", "padding-bottom", "padding-left", "padding-right", "padding-inline", "padding-block", "gap", "row-gap",
];

const NAMED: [&str; 24] = [
    "white", "black", "red", "blue", "green", "gray", "grey", "yellow", "orange", "purple", "pink", "navy", "teal", "silver", "maroon", "olive", "lime", "aqua", "fuchsia", "indigo", "violet", "crimson", "gold", "beige",
];

const GENERIC_FAMILIES: [&str; 14] = [
    "inherit", "initial", "unset", "system-ui", "sans-serif", "serif", "monospace", "cursive", "ui-sans-serif", "ui-serif", "ui-monospace", "-apple-system", "blinkmacsystemfont", "emoji",
];

/// The colour literals in a value, with where each starts: hex, the colour
/// functions, and (for a colour property) a named colour. What sits in a
/// `var(…, fallback)` isn't counted.
fn colour_literals(value: &str, colour_prop: bool) -> Vec<String> {
    let mut out = Vec::new();
    let lower = value.to_ascii_lowercase();
    let mut skip_until = 0;
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if lower[i..].starts_with("var(") {
            let mut depth = 0;
            for (j, c) in lower[i..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            skip_until = i + j;
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        if i < skip_until {
            i += 1;
            continue;
        }
        let c = bytes[i];
        if c == b'#' {
            let hex: String = lower[i + 1..].chars().take_while(char::is_ascii_hexdigit).collect();
            let before = if i == 0 { b' ' } else { bytes[i - 1] };
            if matches!(hex.len(), 3 | 4 | 6 | 8) && !before.is_ascii_alphanumeric() && !lower[i + 1 + hex.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                out.push(value[i..i + 1 + hex.len()].to_owned());
                i += 1 + hex.len();
                continue;
            }
        }
        for f in ["rgba(", "rgb(", "hsla(", "hsl(", "oklch(", "oklab(", "lab(", "lch(", "hwb(", "color("] {
            let before = if i == 0 { b' ' } else { bytes[i - 1] };
            if lower[i..].starts_with(f) && !before.is_ascii_alphanumeric() && before != b'-' {
                let end = lower[i..].find(')').map_or(lower.len(), |e| i + e + 1);
                out.push(value[i..end].to_owned());
                i = end;
                break;
            }
        }
        i += 1;
    }
    if colour_prop {
        for word in lower.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
            if NAMED.contains(&word) {
                out.push(word.to_owned());
            }
        }
    }
    out
}

/// A length in px: `13px`, `0.8rem` (em and the relative units aren't: they follow their text).
fn px(text: &str) -> Option<f64> {
    let t = text.trim().trim_start_matches('-');
    if let Some(n) = t.strip_suffix("px") {
        return n.parse().ok();
    }
    if let Some(n) = t.strip_suffix("rem") {
        return n.parse::<f64>().ok().map(|r| r * 16.);
    }
    None
}

fn tokens_used(source: &Source, system: &System, out: &mut Vec<Finding>) {
    let file = &source.path;
    let colours = skipped("token-colour", system).is_none();
    let spacing = skipped("token-space", system).is_none();
    let typing = skipped("token-type", system).is_none();
    let on_scale = skipped("literal-on-scale", system).is_none();
    let colour = |literal: &str, at: &str, line: u32, out: &mut Vec<Finding>| {
        match colour_of(literal).and_then(|c| system.colour_token(c).map(|t| (c, t))) {
            Some((_, token)) if on_scale => out.push(Finding::new("literal-on-scale", file, Some(line), format!("{at} is `{token}`"))),
            Some(_) => {}
            None => out.push(Finding::new("token-colour", file, Some(line), format!("{at} isn't a colour of the system"))),
        }
    };
    for d in &source.decls {
        if d.prop.starts_with("--") {
            // A custom property set in a screen is a colour of its own.
            if colours {
                for lit in colour_literals(&d.value, false) {
                    colour(&lit, &format!("{}: {lit}", d.prop), d.line, out);
                }
            }
            continue;
        }
        if colours && (COLOUR_PROPS.contains(&d.prop.as_str()) || d.prop.ends_with("-color")) {
            for lit in colour_literals(&d.value, true) {
                if lit == "transparent" {
                    continue;
                }
                colour(&lit, &format!("{}: {lit}", d.prop), d.line, out);
            }
        }
        if spacing && (SPACE_PROPS.contains(&d.prop.as_str()) || d.prop == "column-gap") && !d.value.contains("var(") && !d.value.contains("calc(") {
            for part in d.value.split_whitespace() {
                let Some(v) = px(part) else { continue };
                if v.abs() <= 2. {
                    continue;
                }
                match system.space_token(v.abs()) {
                    Some(token) if on_scale => out.push(Finding::new("literal-on-scale", file, Some(d.line), format!("{}: {part} is `{token}`", d.prop))),
                    Some(_) => {}
                    None => out.push(Finding::new("token-space", file, Some(d.line), format!("{}: {part} is off the spacing scale ({})", d.prop, scale(&system.spacing)))),
                }
            }
        }
        if typing && d.prop == "font-size" && !system.sizes.is_empty() && let Some(v) = px(&d.value) {
            match system.size_token(v) {
                Some(token) if on_scale => out.push(Finding::new("literal-on-scale", file, Some(d.line), format!("font-size: {} is `{token}`", d.value))),
                Some(_) => {}
                None => out.push(Finding::new("token-type", file, Some(d.line), format!("font-size: {} is off the type scale ({})", d.value, scale(&system.sizes)))),
            }
        }
        if typing && d.prop == "font-family" && !system.families.is_empty() && !d.value.contains("var(") {
            family(&d.value, system, file, d.line, out);
        }
    }
    for tag in tags(source) {
        for class in tag.classes() {
            utility(base_class(class), tag.line, system, file, (colours, spacing, typing, on_scale), out);
        }
    }
}

fn scale(steps: &[(String, f64)]) -> String {
    let mut v: Vec<f64> = steps.iter().map(|(_, p)| *p).collect();
    v.sort_by(|a, b| a.total_cmp(b));
    v.dedup();
    v.iter().take(12).map(|p| format!("{p}")).collect::<Vec<_>>().join(", ")
}

fn family(value: &str, system: &System, file: &str, line: u32, out: &mut Vec<Finding>) {
    let named: Vec<String> = value.split(',').map(|f| f.trim().trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase()).filter(|f| !f.is_empty()).collect();
    let Some(first) = named.iter().find(|f| !GENERIC_FAMILIES.contains(&f.as_str())) else { return };
    if system.families.iter().any(|s| s == first || s.contains(first.as_str()) || first.contains(s.as_str())) {
        out.push(Finding::new("literal-on-scale", file, Some(line), format!("font-family: {value} names the system's family: use its token")));
    } else {
        out.push(Finding::new("token-type", file, Some(line), format!("font-family: {value} — the system's families are {}", system.families.iter().take(4).cloned().collect::<Vec<_>>().join(", "))));
    }
}

const TAILWIND_PALETTE: [&str; 22] = [
    "slate", "gray", "zinc", "neutral", "stone", "red", "orange", "amber", "yellow", "lime", "green", "emerald", "teal", "cyan", "sky", "blue", "indigo", "violet", "purple", "fuchsia", "pink", "rose",
];

const COLOUR_UTILITIES: [&str; 16] = ["bg", "text", "border", "ring", "from", "via", "to", "fill", "stroke", "outline", "decoration", "divide", "accent", "caret", "shadow", "placeholder"];

const SPACE_UTILITIES: [&str; 16] = ["p", "px", "py", "pt", "pb", "pl", "pr", "ps", "pe", "m", "mx", "my", "mt", "mb", "gap", "space-y"];

/// A utility class's literal: `bg-[#123456]`, `p-[13px]`, `text-[15px]`,
/// `font-['Inter']`, or a utility palette the system doesn't have (`bg-indigo-500`).
fn utility(class: &str, line: u32, system: &System, file: &str, on: (bool, bool, bool, bool), out: &mut Vec<Finding>) {
    let (colours, spacing, typing, on_scale) = on;
    let class = class.trim_start_matches('-');
    if let Some((prefix, rest)) = class.split_once("-[") {
        let inner = rest.trim_end_matches(']').replace('_', " ");
        if COLOUR_UTILITIES.contains(&prefix) && colours {
            for lit in colour_literals(&inner, false) {
                match colour_of(&lit).and_then(|c| system.colour_token(c)) {
                    Some(token) if on_scale => out.push(Finding::new("literal-on-scale", file, Some(line), format!("{class} is `{token}`"))),
                    Some(_) => {}
                    None => out.push(Finding::new("token-colour", file, Some(line), format!("{class} isn't a colour of the system"))),
                }
            }
        }
        if prefix == "text" && typing && !system.sizes.is_empty() && let Some(v) = px(&inner) {
            if system.size_token(v).is_none() {
                out.push(Finding::new("token-type", file, Some(line), format!("{class} is off the type scale ({})", scale(&system.sizes))));
            } else if on_scale {
                out.push(Finding::new("literal-on-scale", file, Some(line), format!("{class} is on the type scale: use its token")));
            }
        }
        if prefix == "font" && typing && !system.families.is_empty() {
            family(&inner, system, file, line, out);
        }
        if SPACE_UTILITIES.contains(&prefix)
            && spacing
            && let Some(v) = px(&inner)
            && v.abs() > 2.
            && system.space_token(v.abs()).is_none()
        {
            out.push(Finding::new("token-space", file, Some(line), format!("{class} is off the spacing scale ({})", scale(&system.spacing))));
        }
        return;
    }
    if !colours {
        return;
    }
    let parts: Vec<&str> = class.split('-').collect();
    if parts.len() >= 3 && COLOUR_UTILITIES.contains(&parts[0]) && TAILWIND_PALETTE.contains(&parts[1]) {
        let shade = parts[2].split('/').next().unwrap_or("");
        if shade.parse::<u16>().is_ok_and(|s| s == 50 || s == 950 || (s % 100 == 0 && (100..=900).contains(&s))) && !system.palettes.iter().any(|p| p == parts[1]) {
            out.push(Finding::new("token-colour", file, Some(line), format!("{class}: the system has no `{}` palette", parts[1])));
        }
    }
}

// ── focus-visible ──

fn focus(source: &Source, out: &mut Vec<Finding>) {
    let removes = |d: &Decl| (d.prop == "outline" && matches!(d.value.trim(), "none" | "0" | "0px")) || (d.prop == "outline-style" && d.value.trim() == "none") || (d.prop == "outline-width" && matches!(d.value.trim(), "0" | "0px"));
    let restores = |d: &Decl| {
        d.selector.as_deref().is_some_and(|s| s.contains(":focus"))
            && (d.prop.starts_with("outline") || d.prop == "box-shadow" || d.prop.starts_with("border") || d.prop == "text-decoration" || d.prop == "background" || d.prop == "background-color")
            && !matches!(d.value.trim(), "none" | "0" | "0px" | "transparent")
    };
    let restored = source.decls.iter().any(restores);
    for d in source.decls.iter().filter(|d| removes(d)) {
        if !restored {
            out.push(Finding::new("focus-visible", &source.path, Some(d.line), format!("{} {{ {}: {} }} with no :focus-visible style in its place", d.selector.as_deref().unwrap_or("style"), d.prop, d.value)));
        }
    }
    for tag in tags(source) {
        let classes: Vec<&str> = tag.classes();
        let removed = classes.iter().any(|c| base_class(c) == "outline-none" || base_class(c) == "outline-0");
        if !removed {
            continue;
        }
        let ring = classes.iter().any(|c| {
            let (variant, base) = c.rsplit_once(':').unwrap_or(("", c));
            variant.contains("focus") && (base.starts_with("ring") || base.starts_with("outline") && base != "outline-none" || base.starts_with("shadow") || base.starts_with("border") || base.starts_with("bg-") || base.starts_with("underline"))
        });
        if !ring && !restored {
            out.push(Finding::new("focus-visible", &source.path, Some(tag.line), format!("<{}> has `outline-none` and no focus-visible ring", tag.name)));
        }
    }
}

// ── reduced-motion ──

fn moving_transition(value: &str) -> bool {
    let v = value.to_ascii_lowercase();
    if v.trim() == "none" {
        return false;
    }
    // Colour and fade only: nothing moves.
    let quiet = ["color", "background", "background-color", "opacity", "border-color", "box-shadow", "fill", "stroke", "outline", "text-decoration", "visibility", "filter"];
    !v.split(',').all(|part| part.split_whitespace().next().is_some_and(|p| quiet.contains(&p)))
}

pub(super) fn has_reduced_fallback(text: &str) -> bool {
    text.contains("prefers-reduced-motion") || text.contains("motion-reduce:") || text.contains("motion-safe:") || text.contains("useReducedMotion") || text.contains("reducedMotion") || text.contains("reduced-motion")
}

fn motion(source: &Source, system: &System, out: &mut Vec<Finding>) {
    if has_reduced_fallback(&source.text) || global_motion(system) {
        return;
    }
    let mut moves: Vec<(u32, String)> = Vec::new();
    for d in &source.decls {
        let animated = matches!(d.prop.as_str(), "animation" | "animation-name") && !matches!(d.value.trim(), "none" | "0s" | "0");
        let transition = matches!(d.prop.as_str(), "transition" | "transition-property") && moving_transition(&d.value);
        let smooth = d.prop == "scroll-behavior" && d.value.trim() == "smooth";
        if animated || transition || smooth {
            moves.push((d.line, format!("{}: {}", d.prop, clip(&d.value, 40))));
        }
    }
    if let Some(at) = source.text.find("@keyframes") {
        moves.push((source.line_at(at), "@keyframes".into()));
    }
    for tag in tags(source) {
        for c in tag.classes() {
            let b = base_class(c);
            if (b.starts_with("animate-") && b != "animate-none") || b == "transition-all" || b == "transition-transform" {
                moves.push((tag.line, format!("<{}> {b}", tag.name)));
            }
        }
        if tag.name.starts_with("motion.") && (tag.has("animate") || tag.has("whileHover") || tag.has("whileInView") || tag.has("initial")) {
            moves.push((tag.line, format!("<{}> animates", tag.name)));
        }
    }
    for (line, what) in moves {
        out.push(Finding::new("reduced-motion", &source.path, Some(line), format!("{what} moves with no reduced-motion fallback")));
    }
}

/// The system's own stylesheets stop motion for everyone who asks for less.
fn global_motion(system: &System) -> bool {
    let dir = system.dir.join("build").join("css");
    std::fs::read_dir(&dir).map(|d| d.filter_map(|e| e.ok()).any(|e| std::fs::read_to_string(e.path()).is_ok_and(|t| t.contains("prefers-reduced-motion")))).unwrap_or(false)
}

// ── invented-data ──

/// Whether `text` claims what nobody measured: audience counts, ratings,
/// uptime, trust lines, placeholder names, lorem ipsum.
pub fn invented_claim(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    if lower.contains("lorem ipsum") || lower.contains("dolor sit amet") {
        return Some("lorem ipsum".into());
    }
    for name in crate::i18n::every("match.placeholder_names") {
        if contains_word(&lower, name) {
            return Some(format!("a placeholder name (“{}”)", clip(text, 60)));
        }
    }
    for phrase in crate::i18n::every("match.claim_trust") {
        if lower.contains(phrase) {
            return Some(format!("a trust claim (“{}”)", clip(text, 60)));
        }
    }
    if text.contains("★★★") || text.contains("⭐⭐") {
        return Some(format!("a star rating (“{}”)", clip(text, 60)));
    }
    let words: Vec<&str> = lower.split_whitespace().collect();
    let lead = crate::i18n::every("match.claim_lead");
    let measure = crate::i18n::every("match.claim_measure");
    for (i, w) in words.iter().enumerate() {
        let Some(rounded) = figure(w) else { continue };
        let after = words[i + 1..words.len().min(i + 4)].join(" ");
        let before = words[i.saturating_sub(2)..i].join(" ");
        // An exact count (`1,204 users`) is a fact a screen shows; a rounded
        // one (`10k+`, "over 5,000") is a claim someone has to stand behind.
        let claimed = rounded || lead.iter().any(|l| contains_word(&before, l) || contains_word(&after, l) || (!l.is_ascii() && (w.contains(l) || after.contains(l))));
        if claimed && crate::i18n::every("match.claim_audience").iter().any(|a| contains_word(&after, a) || w.contains(a) && !a.is_ascii() || after.starts_with(a)) {
            return Some(format!("an audience figure (“{}”)", clip(text, 60)));
        }
        if w.contains('%') && measure.iter().any(|m| after.starts_with(m) || w.contains(m) && !m.is_ascii()) {
            return Some(format!("a performance figure (“{}”)", clip(text, 60)));
        }
        if (w.contains("/5") || after.starts_with("out of 5") || after.starts_with("/ 5")) && w.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return Some(format!("a rating (“{}”)", clip(text, 60)));
        }
    }
    None
}

/// A figure a claim is made of — `10,000+`, `2m`, `99.9%`, `4.9/5`, Korean `1,000명` —
/// and whether it is rounded (`+`, `k`, `m`, `b`).
fn figure(w: &str) -> Option<bool> {
    let w = w.trim_matches(|c: char| matches!(c, '(' | ')' | ',' | '.' | '!' | '“' | '"' | '”'));
    let digits = w.chars().filter(char::is_ascii_digit).count();
    if digits == 0 || !w.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '$' || c == '+') {
        return None;
    }
    let rest: String = w.chars().filter(|c| !c.is_ascii_digit() && !matches!(c, ',' | '.')).collect();
    let rounded = rest.contains('+') || matches!(rest.trim_end_matches('+'), "k" | "m" | "b" | "$k" | "$m" | "$b");
    let shape = rest.chars().all(|c| !c.is_ascii_alphabetic()) || rounded;
    shape.then_some(rounded)
}

fn contains_word(text: &str, word: &str) -> bool {
    let mut at = 0;
    while let Some(found) = text[at..].find(word) {
        let start = at + found;
        let end = start + word.len();
        let before = text[..start].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = text[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        if before && after {
            return true;
        }
        at = end;
    }
    false
}

const PROOF_CLASSES: [&str; 9] = ["testimonial", "logo-cloud", "logo-wall", "logos", "customer-logos", "client-logos", "trusted-by", "social-proof", "reviews"];

fn invented(source: &Source, out: &mut Vec<Finding>) {
    // Comments saying where words come from, by line.
    let sourced: Vec<u32> = source.nodes.iter().filter_map(|n| if let Node::Comment { text, line } = n { text.to_ascii_lowercase().contains("source:").then_some(*line) } else { None }).collect();
    let near_source = |line: u32| sourced.iter().any(|&l| l <= line && line - l <= 2);
    walk(source, |step, stack| {
            let Step::At(node) = step else { return };
            let sourced_here = stack.iter().any(|o| o.tag.has("data-source"));
            match node {
                Node::Text { text, line, .. } if !sourced_here && !near_source(*line) => {
                    if let Some(what) = invented_claim(text) {
                        out.push(Finding::new("invented-data", &source.path, Some(*line), what));
                    }
                }
                Node::Open(tag) if !sourced_here && !tag.has("data-source") && !near_source(tag.line) => {
                    if let Some(c) = tag.classes().iter().find(|c| PROOF_CLASSES.iter().any(|p| base_class(c).contains(p))) {
                        out.push(Finding::new("invented-data", &source.path, Some(tag.line), format!("<{} class=\"{c}\">: proof of customers the brief didn't give", tag.name)));
                    }
                }
                _ => {}
            }
    });
}

// ── control-name ──

fn names(source: &Source, out: &mut Vec<Finding>) {
    // The ids a label points at.
    let labelled: Vec<String> = tags(source).filter(|t| t.name == "label").filter_map(|t| t.attr("for").or_else(|| t.attr("htmlFor")).map(str::to_owned)).collect();
    let named_by_attr = |t: &Tag| ["aria-label", "aria-labelledby", "title"].iter().any(|a| t.attr(a).is_some_and(|v| !v.trim().is_empty())) || t.has("{...}");
    walk(source, |step, stack| {
            let node = match step {
                Step::At(node) => node,
                Step::Done(done) => {
                    let t = done.tag;
                    let control = t.name == "button" || (t.name == "a" && t.has("href")) || t.attr("role").is_some_and(|r| r == "button" || r == "link");
                    if control && !named_by_attr(t) && done.words.trim().is_empty() && !done.expr && !done.named {
                        out.push(Finding::new("control-name", &source.path, Some(t.line), format!("<{}> has no text and no aria-label", t.name)));
                    }
                    return;
                }
            };
            let Node::Open(tag) = node else { return };
            if named_by_attr(tag) {
                return;
            }
            match tag.name.as_str() {
                "img" if !tag.has("alt") => out.push(Finding::new("control-name", &source.path, Some(tag.line), format!("<img src=\"{}\"> has no alt", clip(tag.attr("src").unwrap_or(""), 40)))),
                "input" | "select" | "textarea" => {
                    let kind = tag.attr("type").unwrap_or("text").to_ascii_lowercase();
                    if matches!(kind.as_str(), "hidden" | "submit" | "button" | "reset" | "image") {
                        return;
                    }
                    let in_label = stack.iter().any(|o| o.tag.name == "label");
                    let by_id = tag.attr("id").is_some_and(|id| labelled.iter().any(|l| l == id));
                    if !in_label && !by_id {
                        let hint = tag.attr("placeholder").map(|p| format!(" (a placeholder, “{}”, isn't a label)", clip(p, 30))).unwrap_or_default();
                        out.push(Finding::new("control-name", &source.path, Some(tag.line), format!("<{} name=\"{}\"> has no label{hint}", tag.name, tag.attr("name").unwrap_or(""))));
                    }
                }
                _ => {}
            }
    });
}

// ── habits ──

const LABEL_HINTS: [&str; 14] = ["eyebrow", "kicker", "overline", "pretitle", "pre-title", "tagline", "label", "badge", "pill", "chip", "uppercase", "tracking-", "caps", "subheading"];

fn label_like(tag: &Tag, words: &str) -> bool {
    if !matches!(tag.name.as_str(), "p" | "span" | "div" | "small" | "strong" | "em" | "b" | "label") {
        return false;
    }
    let n = words_of(words);
    if n == 0 || n > 5 {
        return false;
    }
    let hinted = tag.classes().iter().any(|c| LABEL_HINTS.iter().any(|h| base_class(c).contains(h))) || tag.attr("style").is_some_and(|s| s.contains("uppercase"));
    let letters: Vec<char> = words.chars().filter(|c| c.is_alphabetic()).collect();
    let shouted = letters.len() >= 3 && letters.iter().all(|c| c.is_uppercase());
    hinted || shouted
}

fn eyebrows(source: &Source, out: &mut Vec<Finding>) {
    // The element just closed at each depth, with its words: a heading's previous sibling.
    let mut last: Vec<Option<(Tag, String)>> = vec![None];
    let mut headings = 0;
    let mut over: Vec<(u32, String)> = Vec::new();
    let mut depth = 0usize;
    walk(source, |step, stack| {
            let node = match step {
                Step::At(node) => node,
                Step::Done(done) => {
                    let at = stack.len();
                    if last.len() <= at + 1 {
                        last.resize(at + 2, None);
                    }
                    last[at] = Some((done.tag.clone(), done.words.clone()));
                    last[at + 1] = None;
                    return;
                }
            };
            depth = stack.len();
            while last.len() <= depth + 1 {
                last.push(None);
            }
            match node {
                Node::Open(tag) if tag.is_heading().is_some_and(|l| l <= 3) => {
                    headings += 1;
                    if let Some((prev, words)) = &last[depth]
                        && label_like(prev, words)
                    {
                        over.push((tag.line, clip(words, 30)));
                    }
                    last[depth] = None;
                }
                Node::Open(_) => {}
                Node::Text { text, .. } if !text.trim().is_empty() => last[depth] = None,
                _ => {}
            }
    });
    if !over.is_empty() && (over.len() >= 2 || over.len() == headings) {
        let said: Vec<String> = over.iter().take(3).map(|(_, w)| format!("“{w}”")).collect();
        out.push(Finding::new("eyebrow", &source.path, Some(over[0].0), format!("{} of {headings} heading(s) have a small label over them: {}", over.len(), said.join(", "))));
    }
}

fn tracked_caps(source: &Source, out: &mut Vec<Finding>) {
    let wide = |v: &str| {
        let v = v.trim();
        if let Some(em) = v.strip_suffix("em") {
            return em.parse::<f64>().is_ok_and(|e| e >= 0.05);
        }
        px(v).is_some_and(|p| p >= 1.)
    };
    // A rule's declarations: those in a row with its selector.
    let mut i = 0;
    let decls = &source.decls;
    while i < decls.len() {
        let mut j = i;
        while j < decls.len() && decls[j].selector == decls[i].selector && decls[j].line.abs_diff(decls[i].line) < 40 {
            j += 1;
        }
        let group = &decls[i..j.max(i + 1)];
        let caps = group.iter().find(|d| d.prop == "text-transform" && d.value.trim() == "uppercase");
        let spaced = group.iter().any(|d| d.prop == "letter-spacing" && wide(&d.value));
        if let Some(c) = caps
            && spaced
        {
            out.push(Finding::new("tracked-caps", &source.path, Some(c.line), format!("{} sets spaced capitals", c.selector.as_deref().unwrap_or("a style"))));
        }
        i = j.max(i + 1);
    }
    for tag in tags(source) {
        let classes: Vec<&str> = tag.classes().into_iter().map(base_class).collect();
        if classes.contains(&"uppercase") && classes.iter().any(|c| matches!(*c, "tracking-wide" | "tracking-wider" | "tracking-widest") || c.starts_with("tracking-[0.") || c.starts_with("tracking-[1") || c.starts_with("tracking-[2")) {
            out.push(Finding::new("tracked-caps", &source.path, Some(tag.line), format!("<{} class=\"{}\">", tag.name, clip(&tag.classes().join(" "), 50))));
        }
    }
}

fn colour_class(c: &str) -> bool {
    let c = base_class(c);
    c == "bg-clip-text" || c == "text-transparent" || c.contains("gradient") || c.contains("highlight") || c.starts_with("text-[#") || ["text-primary", "text-accent", "text-brand", "text-secondary"].iter().any(|p| c.starts_with(p)) || {
        let parts: Vec<&str> = c.split('-').collect();
        parts.len() >= 3 && parts[0] == "text" && TAILWIND_PALETTE.contains(&parts[1])
    }
}

fn coloured_word(source: &Source, out: &mut Vec<Finding>) {
    let mut marked: Vec<(u32, String)> = Vec::new();
    walk(source, |step, rest| {
            let Step::Done(done) = step else { return };
            let in_heading = rest.iter().any(|o| o.tag.is_heading().is_some_and(|l| l <= 2));
            let t = done.tag;
            if !in_heading || !matches!(t.name.as_str(), "span" | "em" | "strong" | "mark" | "b" | "i") {
                return;
            }
            let styled = t.classes().iter().any(|c| colour_class(c)) || t.attr("style").is_some_and(|s| s.contains("color") || s.contains("background-clip") || s.contains("gradient"));
            if styled && words_of(&done.words) <= 3 && words_of(&done.words) >= 1 {
                marked.push((t.line, clip(&done.words, 30)));
            }
    });
    for d in &source.decls {
        let sel = d.selector.as_deref().unwrap_or("");
        let heading_child = (sel.contains("h1 ") || sel.contains("h2 ") || sel.contains("title ") || sel.contains("headline ")) && (sel.ends_with("span") || sel.ends_with("em") || sel.ends_with("strong") || sel.ends_with("mark"));
        if heading_child && (d.prop == "color" || d.prop.contains("background-clip") || d.value.contains("gradient")) {
            marked.push((d.line, sel.to_owned()));
        }
    }
    marked.dedup_by(|a, b| a.0 == b.0);
    for (line, words) in marked {
        out.push(Finding::new("coloured-word", &source.path, Some(line), format!("“{words}” in another colour from the rest of its headline")));
    }
}

fn gradient_hero(source: &Source, out: &mut Vec<Finding>) {
    const HERO: [&str; 9] = ["hero", "banner", "masthead", "jumbotron", "splash", "intro", "header", "landing", "cover"];
    let heroish = |s: &str| {
        let s = s.to_ascii_lowercase();
        HERO.iter().any(|h| s.contains(h)) || s.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == "body" || w == "html" || w == "main")
    };
    for d in &source.decls {
        if (d.prop == "background" || d.prop == "background-image") && d.value.contains("gradient(") && d.selector.as_deref().is_some_and(heroish) {
            out.push(Finding::new("gradient-hero", &source.path, Some(d.line), format!("{} has a {} behind it", d.selector.as_deref().unwrap_or(""), d.value.split('(').next().unwrap_or("gradient"))));
        }
    }
    let mut first_block = true;
    for (n, tag) in tags(source).enumerate() {
        // A component's outermost element is its first screen.
        let block = matches!(tag.name.as_str(), "section" | "header" | "main" | "body") || (n == 0 && source.kind == Kind::Script);
        let classes = tag.classes();
        let gradient = classes.iter().any(|c| {
            let b = base_class(c);
            b.starts_with("bg-gradient-") || b.starts_with("bg-linear-") || b.starts_with("bg-radial") || b.starts_with("from-")
        }) || tag.attr("style").is_some_and(|s| s.contains("gradient("));
        let named = classes.iter().any(|c| heroish(c)) || tag.attr("id").is_some_and(heroish);
        if gradient && (named || (block && first_block)) {
            out.push(Finding::new("gradient-hero", &source.path, Some(tag.line), format!("<{}> opens on a gradient", tag.name)));
        }
        if block && tag.name != "body" && tag.name != "main" {
            first_block = false;
        }
    }
}

/// A card: by its class (`card`, `panel`, `tile`), its component's name, or
/// a rounded, padded box with a shadow or border.
fn card_like(tag: &Tag) -> bool {
    if tag.name.chars().next().is_some_and(char::is_uppercase) {
        return tag.name.ends_with("Card") || tag.name == "Card" || tag.name.ends_with("Panel");
    }
    let classes: Vec<&str> = tag.classes().into_iter().map(base_class).collect();
    let named = classes.iter().any(|c| {
        let c = c.to_ascii_lowercase();
        c == "card" || c == "panel" || c == "tile" || c.ends_with("-card") || c.starts_with("card-") && !c.starts_with("card-title") && !c.starts_with("card-body") && !c.starts_with("card-header") && !c.starts_with("card-footer") && !c.starts_with("card-text") || c.ends_with("__card")
    });
    let rounded = classes.iter().any(|c| c.starts_with("rounded") && !matches!(*c, "rounded-none" | "rounded-full"));
    let lifted = classes.iter().any(|c| (c.starts_with("shadow") && *c != "shadow-none") || *c == "border" || c.starts_with("border-2"));
    let padded = classes.iter().any(|c| c.starts_with("p-") || c.starts_with("px-") || c.starts_with("py-"));
    named || (rounded && lifted && padded)
}

fn nested_cards(source: &Source, out: &mut Vec<Finding>) {
    walk(source, |step, stack| {
            if let Step::At(Node::Open(tag)) = step
                && card_like(tag)
                && let Some(outer) = stack.iter().rev().find(|o| card_like(o.tag))
            {
                out.push(Finding::new("nested-cards", &source.path, Some(tag.line), format!("a card (line {}) inside a card (line {})", tag.line, outer.tag.line)));
            }
    });
}

/// An emoji, as a picture: not the marks typography has (✓ ✕ ★ ☐).
pub fn is_emoji(c: char) -> bool {
    let u = c as u32;
    let typographic = matches!(u, 0x2605 | 0x2606 | 0x2610..=0x2612 | 0x2713..=0x2718 | 0x2731..=0x2733 | 0x2756);
    !typographic && matches!(u, 0x1F300..=0x1FAFF | 0x1F000..=0x1F2FF | 0x2600..=0x27BF | 0x2B50 | 0x2B55 | 0x23F0..=0x23F3 | 0x231A | 0x231B)
}

fn emoji(source: &Source, out: &mut Vec<Finding>) {
    let mut found: Vec<(u32, String)> = Vec::new();
    walk(source, |step, stack| {
            let Step::At(Node::Text { text, line, .. }) = step else { return };
            let in_control = stack.iter().any(|o| matches!(o.tag.name.as_str(), "button" | "a" | "li" | "nav" | "label" | "th" | "h1" | "h2" | "h3" | "h4" | "dt"));
            let lead = text.trim_start().chars().next().is_some_and(is_emoji);
            if let Some(e) = text.chars().find(|c| is_emoji(*c))
                && (lead || in_control)
            {
                found.push((*line, format!("{e} {}", clip(text.trim().trim_start_matches(e), 24))));
            }
    });
    if let Some((line, _)) = found.first() {
        let said: Vec<String> = found.iter().take(4).map(|(_, w)| format!("“{w}”")).collect();
        out.push(Finding::new("emoji-icons", &source.path, Some(*line), format!("{} emoji used as icons: {}", found.len(), said.join(", "))));
    }
}

/// The classes a stylesheet centres text with (`.hero { text-align: center }`).
fn centring_classes(source: &Source) -> Vec<String> {
    let mut out = Vec::new();
    for d in source.decls.iter().filter(|d| d.prop == "text-align" && d.value.trim() == "center") {
        for compound in d.selector.as_deref().unwrap_or("").split(',') {
            if let Some(last) = compound.split([' ', '>', '+', '~']).rfind(|p| !p.is_empty()) {
                out.extend(last.split('.').skip(1).map(|c| c.split(':').next().unwrap_or(c).to_owned()));
            }
        }
    }
    out
}

fn centred(source: &Source, out: &mut Vec<Finding>) {
    let classes = centring_classes(source);
    let centres = |t: &Tag| {
        let cs: Vec<&str> = t.classes().into_iter().map(base_class).collect();
        if cs.iter().any(|c| matches!(*c, "text-left" | "text-start")) {
            return Some(false);
        }
        (cs.contains(&"text-center") || cs.iter().any(|c| classes.iter().any(|k| k == c)) || t.attr("style").is_some_and(|s| s.replace(' ', "").contains("text-align:center")) || t.attr("align").is_some_and(|a| a == "center") || t.name == "center").then_some(true)
    };
    let (mut total, mut centred) = (0usize, 0usize);
    let mut first = None;
    walk(source, |step, stack| {
            let Step::At(Node::Open(tag)) = step else { return };
            if !(tag.is_heading().is_some() || tag.name == "p") {
                return;
            }
            total += 1;
            let mine = centres(tag).or_else(|| stack.iter().rev().find_map(|o| centres(o.tag))).unwrap_or(false);
            if mine {
                centred += 1;
                first.get_or_insert(tag.line);
            }
    });
    if total >= 4 && centred * 5 >= total * 4 {
        out.push(Finding::new("all-centred", &source.path, first, format!("{centred} of {total} headings and paragraphs are centred")));
    }
}

fn uniform(source: &Source, out: &mut Vec<Finding>) {
    let mut looks: HashMap<String, usize> = HashMap::new();
    let mut cards = 0;
    let mut first = None;
    for tag in tags(source).filter(|t| card_like(t)) {
        cards += 1;
        first.get_or_insert(tag.line);
        let classes: Vec<&str> = tag.classes().into_iter().map(base_class).collect();
        let mut look: Vec<&str> = classes.iter().copied().filter(|c| c.starts_with("rounded") || c.starts_with("shadow")).collect();
        look.sort_unstable();
        if look.iter().any(|c| c.starts_with("shadow")) {
            *looks.entry(look.join(" ")).or_default() += 1;
        }
    }
    if cards >= 4 && looks.len() == 1 && looks.values().next() == Some(&cards) {
        out.push(Finding::new("uniform-blocks", &source.path, first, format!("all {cards} blocks are `{}`", looks.keys().next().cloned().unwrap_or_default())));
    }
    // A stylesheet giving many selectors one radius and shadow.
    let mut pairs: HashMap<(String, String), Vec<u32>> = HashMap::new();
    let mut by_selector: HashMap<String, (Option<String>, Option<String>, u32)> = HashMap::new();
    for d in &source.decls {
        let Some(sel) = &d.selector else { continue };
        let e = by_selector.entry(sel.clone()).or_insert((None, None, d.line));
        if d.prop == "border-radius" {
            e.0 = Some(d.value.clone());
        }
        if d.prop == "box-shadow" && d.value.trim() != "none" {
            e.1 = Some(d.value.clone());
        }
    }
    for (_, (radius, shadow, line)) in by_selector {
        if let (Some(r), Some(s)) = (radius, shadow) {
            pairs.entry((r, s)).or_default().push(line);
        }
    }
    if let Some(((r, s), lines)) = pairs.into_iter().find(|(_, l)| l.len() >= 3) {
        out.push(Finding::new("uniform-blocks", &source.path, lines.iter().min().copied(), format!("{} styles share radius {r} and shadow {}", lines.len(), clip(&s, 30))));
    }
}

fn dot_like(tag: &Tag) -> bool {
    let classes: Vec<&str> = tag.classes().into_iter().map(base_class).collect();
    let named = classes.iter().any(|c| *c == "dot" || c.ends_with("-dot") || c.starts_with("dot-") || c.contains("pulse-dot") || c.contains("status-dot"));
    let round = classes.contains(&"rounded-full") && classes.iter().any(|c| matches!(*c, "w-1" | "w-1.5" | "w-2" | "w-2.5" | "w-3" | "size-1" | "size-1.5" | "size-2" | "size-2.5" | "size-3" | "h-2" | "h-1.5"));
    named || round
}

fn dots(source: &Source, out: &mut Vec<Finding>) {
    let words = crate::i18n::every("match.dot_words");
    // A dot just closed, waiting to see the words after it.
    let mut pending: Option<(u32, bool)> = None;
    walk(source, |step, _| match step {
            Step::At(Node::Open(tag)) if dot_like(tag) => {
                let status = tag.attr("role").is_some_and(|r| r == "status") || tag.has("aria-label");
                let animated = tag.classes().iter().any(|c| {
                    let b = base_class(c);
                    b == "animate-pulse" || b == "animate-ping" || b.contains("pulse") || b.contains("blink")
                });
                if status {
                    return;
                }
                if animated {
                    out.push(Finding::new("decorative-dots", &source.path, Some(tag.line), format!("<{}> pulses, reporting nothing", tag.name)));
                } else {
                    pending = Some((tag.line, true));
                }
            }
            Step::At(Node::Text { text, .. }) if !text.trim().is_empty() => {
                if let Some((line, _)) = pending.take() {
                    let first = text.trim().to_lowercase();
                    if words.iter().any(|w| first.starts_with(w)) {
                        out.push(Finding::new("decorative-dots", &source.path, Some(line), format!("a dot before “{}”, a status that isn't one", clip(text, 24))));
                    }
                }
            }
            _ => {}
    });
}

fn forms(source: &Source, out: &mut Vec<Finding>) {
    for tag in tags(source).filter(|t| t.name == "form") {
        let handled = ["action", "onSubmit", "onsubmit", "@submit", "@submit.prevent", "on:submit", "on:submit|preventDefault", "v-on:submit", "(submit)", "(ngSubmit)", "hx-post", "hx-get", "hx-put", "method", "use:enhance"].iter().any(|a| tag.has(a)) || tag.has_prefix("@submit") || tag.has_prefix("on:submit") || tag.has("{...}");
        if !handled {
            out.push(Finding::new("pretend-submit", &source.path, Some(tag.line), "a <form> that submits nowhere (no action, no submit handler)"));
        }
    }
    // A handler that stops the form, then only says it worked.
    let text = &source.text;
    let mut at = 0;
    while let Some(found) = text[at..].find("preventDefault()") {
        let start = at + found;
        let end = text[start..].find("\n}").map_or(text.len(), |e| start + e).min(start + 700);
        let body = &text[start..end];
        at = start + 16;
        let says = body.contains("alert(") || body.contains("toast") || ["Success(true)", "Submitted(true)", "Sent(true)", "Done(true)", "Saved(true)", "Thanks(true)", "success = true", "submitted = true", "sent = true"].iter().any(|s| body.contains(s));
        let does = ["fetch(", "axios", "mutate", ".post(", ".put(", "api.", "api/", "action(", "send(", "submit(", "invoke(", "request(", "await ", "supabase", "firebase", "trpc", "graphql", "emit("].iter().any(|w| body.contains(w));
        if says && !does {
            out.push(Finding::new("pretend-submit", &source.path, Some(source.line_at(start)), "a submit handler that says it worked without sending anything"));
        }
    }
}

// ── finish ──

fn headings(source: &Source, out: &mut Vec<Finding>) {
    let mut last: Option<u8> = None;
    for tag in tags(source) {
        let Some(level) = tag.is_heading() else { continue };
        if let Some(prev) = last
            && level > prev + 1
        {
            out.push(Finding::new("heading-order", &source.path, Some(tag.line), format!("h{prev} then h{level}")));
        }
        last = Some(level);
    }
}

fn images(source: &Source, out: &mut Vec<Finding>) {
    for tag in tags(source).filter(|t| t.name == "img") {
        let sized = (tag.has("width") && tag.has("height")) || tag.has("fill") || tag.classes().iter().any(|c| {
            let b = base_class(c);
            b.starts_with("aspect-") || b.starts_with("size-") || (b.starts_with("h-") && tag.classes().iter().any(|c| base_class(c).starts_with("w-")))
        }) || tag.attr("style").is_some_and(|s| s.contains("aspect-ratio") || (s.contains("width") && s.contains("height")));
        if !sized {
            out.push(Finding::new("image-size", &source.path, Some(tag.line), format!("<img src=\"{}\"> has no width and height", clip(tag.attr("src").unwrap_or(""), 40))));
        }
    }
}

fn figures(source: &Source, out: &mut Vec<Finding>) {
    let tabular = source.text.contains("tabular-nums") || source.text.contains("font-variant-numeric") || source.text.contains("fontVariantNumeric");
    if tabular {
        return;
    }
    if let Some(table) = tags(source).find(|t| t.name == "table" || t.name == "td") {
        let has_figures = source.words().split_whitespace().filter(|w| w.chars().any(|c| c.is_ascii_digit())).count() >= 4;
        if has_figures {
            out.push(Finding::new("tabular-figures", &source.path, Some(table.line), "a table of figures with proportional digits"));
        }
    }
}

/// A colour literal's value: what [`values::color`] reads, and the plain names.
fn colour_of(text: &str) -> Option<Color> {
    let hex = match text.to_ascii_lowercase().as_str() {
        "white" => Some(0xFFFFFF),
        "black" => Some(0x000000),
        "red" => Some(0xFF0000),
        "blue" => Some(0x0000FF),
        "green" => Some(0x008000),
        "gray" | "grey" => Some(0x808080),
        _ => None,
    };
    hex.map(Color::rgb).or_else(|| values::color(&serde_json::Value::String(text.to_owned())))
}
