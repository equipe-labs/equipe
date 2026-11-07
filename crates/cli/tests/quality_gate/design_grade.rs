//! Graders for the design tasks, kept apart from equipe's own design gates
//! on purpose: a page is measured by `page_check.mjs` (its own browser
//! driver and measuring code, not `equipe design lint`), and judged — where
//! only judgement will do — by a reviewer with no context but a fixed rubric
//! and the pictures. So a gate is never graded by itself, and A and B are
//! graded by the same yardstick.

use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};

use serde_json::Value;

use crate::tasks::Check;

/// Chrome for the measurements: `EQUIPE_EVAL_CHROME`, else the usual places.
pub fn chrome() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("EQUIPE_EVAL_CHROME").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let known = ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/Applications/Chromium.app/Contents/MacOS/Chromium"];
    if let Some(p) = known.iter().map(PathBuf::from).find(|p| p.is_file()) {
        return Some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).flat_map(|d| ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"].map(|n| d.join(n))).find(|p| p.is_file())
}

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/quality_gate/page_check.mjs")
}

/// Whether pages can be measured here: Node and Chrome.
pub fn can_measure() -> bool {
    chrome().is_some() && Process::new("node").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn node(args: &[&str]) -> Result<Value, String> {
    let chrome = chrome().ok_or("no Chrome to measure the page in (set EQUIPE_EVAL_CHROME)")?;
    let mut c = Process::new("node");
    c.arg(script()).arg("--chrome").arg(&chrome).args(args).stdin(Stdio::null());
    let o = c.output().map_err(|e| format!("node: {e}"))?;
    if !o.status.success() {
        return Err(format!("page_check.mjs: {}", String::from_utf8_lossy(&o.stderr).trim().chars().take(300).collect::<String>()));
    }
    serde_json::from_slice(&o.stdout).map_err(|e| format!("page_check.mjs printed no JSON: {e}"))
}

/// `page` measured at 360, 768 and 1440 px, its pictures in `out`.
pub fn measure(page: &Path, out: &Path) -> Result<Value, String> {
    node(&["--page", &page.to_string_lossy(), "--out", &out.to_string_lossy()])
}

/// Pictures of `file` (an SVG or HTML mockup) as PNGs in `out`.
pub fn shoot(file: &Path, out: &Path) -> Result<Vec<PathBuf>, String> {
    let v = node(&["--shot", &file.to_string_lossy(), "--out", &out.to_string_lossy()])?;
    Ok(v["shots"].as_array().into_iter().flatten().filter_map(Value::as_str).map(PathBuf::from).collect())
}

fn must(name: &'static str, ok: bool, detail: impl Into<String>) -> Check {
    Check { name, ok, required: true, detail: detail.into() }
}

fn should(name: &'static str, ok: bool, detail: impl Into<String>) -> Check {
    Check { name, ok, required: false, detail: detail.into() }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().map(|x| x.as_str().map(str::to_owned).unwrap_or_else(|| x.to_string())).collect()
}

/// `text` with every kind of space (a no-break space between a number and
/// its unit, say) as a plain one; line breaks kept.
pub fn spaced(text: &str) -> String {
    text.chars().map(|c| if c.is_whitespace() && c != '\n' { ' ' } else { c }).collect()
}

/// The landing page's checks, from what `page_check.mjs` measured, against the brief it was made from.
pub fn landing_checks(m: &Value, brief: &str, html: &str) -> Vec<Check> {
    let mut checks = Vec::new();
    // Fits: no sideways scroll, nothing cut at the edge, at each width.
    let mut wide = Vec::new();
    for w in ["360", "768", "1440"] {
        let at = &m["widths"][w];
        if at["scrolls"].as_bool() != Some(false) || at["cutCount"].as_u64() != Some(0) {
            wide.push(format!("{w}: {} wide in {}; cut: {}", at["scrollWidth"], at["viewport"], strings(&at["cut"]).join(", ")));
        }
    }
    checks.push(must("fits at 360, 768 and 1440", wide.is_empty(), wide.join(" · ")));
    // Contrast: WCAG AA for each text the styles say the colours of.
    let mut low = Vec::new();
    let (mut checked, mut unknown) = (0, 0);
    for w in ["360", "1440"] {
        let c = &m["contrast"][w];
        checked += c["checked"].as_u64().unwrap_or(0);
        unknown += c["unknown"].as_u64().unwrap_or(0);
        for f in c["failures"].as_array().into_iter().flatten() {
            low.push(format!("{w}: {} {}:1 (needs {})", f["el"].as_str().unwrap_or(""), f["ratio"], f["need"]));
        }
    }
    checks.push(must("text contrast meets WCAG AA", checked > 0 && low.is_empty(), if low.is_empty() { format!("{checked} texts, {unknown} on pictures unmeasured") } else { low.into_iter().take(6).collect::<Vec<_>>().join(" · ") }));
    // Focus: Tab reaches the controls, and each shows it has focus.
    let f = &m["focus"];
    let (focusable, reached, unseen) = (f["focusable"].as_u64().unwrap_or(0), f["reached"].as_u64().unwrap_or(0), f["noIndicatorCount"].as_u64().unwrap_or(u64::MAX));
    checks.push(must("focus shows on every control", focusable > 0 && reached > 0 && unseen == 0, format!("{reached} of {focusable} reached by Tab; no indicator: {}", strings(&f["noIndicator"]).join(", "))));
    // Structure: what HTML and WCAG ask of any page, read from the DOM.
    let s = &m["structure"];
    let mut wrong = Vec::new();
    if s["lang"].as_str().unwrap_or("").is_empty() {
        wrong.push("no lang on <html>".to_owned());
    }
    if s["viewport"].as_bool() != Some(true) {
        wrong.push("no width=device-width viewport".to_owned());
    }
    if s["title"].as_str().unwrap_or("").is_empty() {
        wrong.push("no title".to_owned());
    }
    if s["main"].as_u64() != Some(1) {
        wrong.push(format!("{} main landmarks", s["main"]));
    }
    if s["h1"].as_u64() != Some(1) {
        wrong.push(format!("{} h1 headings", s["h1"]));
    }
    for (key, what) in [("skips", "heading levels skipped"), ("imgsNoAlt", "images without alt"), ("unnamedControls", "fields without a name"), ("unnamedActions", "links or buttons without a name"), ("dupIds", "ids used twice")] {
        let list = strings(&s[key]);
        if !list.is_empty() {
            wrong.push(format!("{what}: {}", list.into_iter().take(4).collect::<Vec<_>>().join(", ")));
        }
    }
    checks.push(must("sound structure", wrong.is_empty(), wrong.join(" · ")));
    // The sections the ticket asked for.
    let plain = spaced(s["text"].as_str().unwrap_or(""));
    let text = plain.to_lowercase();
    let actions: Vec<String> = s["actions"].as_array().into_iter().flatten().filter_map(|a| a["text"].as_str()).map(|a| spaced(a).to_lowercase()).collect();
    // A page sets a figure apart from its words (`$` `8`, `3` `meetings a month` in boxes of
    // their own): the words read across line breaks, and with no spaces at all.
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let has = |any: &[&str]| any.iter().any(|w| flat.contains(w) || compact.contains(&w.replace(" ", "")));
    let mut missing = Vec::new();
    for (what, ok) in [
        ("the product's name", has(&["quillo"])),
        ("a \"Join the beta\" button", actions.iter().any(|a| a.contains("join the beta"))),
        ("recordings up to 2 hours", has(&["2 hours", "2-hour", "two hours", "2 hrs", "2h "])),
        ("decisions and to-dos", has(&["decision"]) && has(&["to-do", "todo", "to do", "action item"])),
        ("sharing as a link or a PDF", has(&["pdf"])),
        ("the free plan", has(&["free"]) && has(&["3 meetings", "three meetings"])),
        ("the team plan", has(&["$8"])),
        ("a sign-up form with a named email field", s["emailInForm"].as_bool() == Some(true) && s["emailFieldNamed"].as_bool() == Some(true)),
        ("a footer", s["footer"].as_u64().unwrap_or(0) >= 1),
    ] {
        if !ok {
            missing.push(what);
        }
    }
    checks.push(must("the sections asked for", missing.is_empty(), if missing.is_empty() { String::new() } else { format!("missing: {}", missing.join(", ")) }));
    let invented = made_up(&plain, html, brief);
    checks.push(must("only the brief's facts", invented.is_empty(), invented.join(" · ")));
    checks.push(should("the beta's date", has(&["3 march", "march 3", "march 3rd", "3rd march"]), ""));
    checks
}

/// Words that claim people use or love a product: a product nobody has used yet can't say them.
const PROOF: &[&str] = &[
    "trusted by", "loved by", "used by", "join over", "joined by", "as seen in", "as featured", "featured in", "testimonial", "what our", "what people say", "what teams say", "customers say", "users say", "rated", "out of 5", "★", "⭐", "five stars", "5 stars", "our customers", "happy customers",
];

/// What a number is a count of when it boasts: "10,000 teams", "5 hours saved".
const COUNTED: &[&str] = &[
    "users", "teams", "customers", "companies", "people", "businesses", "organizations", "organisations", "meetings", "hours", "minutes", "seconds", "countries", "languages", "downloads", "reviews", "stars", "notes", "summaries", "times", "percent",
];

fn at_word(hay: &str, at: usize, len: usize) -> bool {
    let before = hay[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
    let after = hay[at + len..].chars().next().is_none_or(|c| !c.is_alphanumeric());
    before && after
}

fn find_words(hay: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(i) = hay[from..].find(needle) {
        let at = from + i;
        if needle.chars().all(|c| !c.is_alphanumeric()) || at_word(hay, at, needle.len()) {
            return true;
        }
        from = at + needle.len().max(1);
    }
    false
}

/// The numbers in `text`: each with the text right before and after it.
fn numbers(text: &str) -> Vec<(f64, String, String, String)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        let glued = i > 0 && chars[i - 1].1.is_alphanumeric();
        if c.is_ascii_digit() && !glued {
            let mut j = i;
            while j < chars.len() && (chars[j].1.is_ascii_digit() || ((chars[j].1 == ',' || chars[j].1 == '.') && chars.get(j + 1).is_some_and(|n| n.1.is_ascii_digit()))) {
                j += 1;
            }
            let end = chars.get(j).map(|c| c.0).unwrap_or(text.len());
            let raw = &text[start..end];
            if let Ok(v) = raw.replace(',', "").parse::<f64>() {
                let before: String = text[..start].chars().rev().take(2).collect::<Vec<_>>().into_iter().rev().collect();
                let after: String = text[end..].chars().take(24).collect();
                out.push((v, raw.to_owned(), before, after));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// What a page says that its brief doesn't: social proof, testimonials and
/// numbers it made up. Numbers the brief has are fine anywhere, and so are a
/// copyright year, 0 and 1 (a free plan, a first step); a number counting
/// people, time or things, a percentage, a multiple, a "10,000+" or a price
/// the brief doesn't have is invented.
pub fn made_up(text: &str, html: &str, brief: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let brief_lower = brief.to_lowercase();
    let mut found = Vec::new();
    for p in PROOF {
        if find_words(&lower, p) && !find_words(&brief_lower, p) {
            found.push(format!("says \"{p}\""));
        }
    }
    let tags = html.to_lowercase();
    if tags.contains("<blockquote") {
        found.push("a quote (<blockquote>)".into());
    }
    let mut allowed: Vec<f64> = numbers(brief).into_iter().map(|n| n.0).collect();
    allowed.extend([0.0, 1.0, 2025.0, 2026.0, 2027.0]);
    let known = |v: f64| allowed.iter().any(|a| (a - v).abs() < 1e-9);
    for (v, raw, before, after) in numbers(text) {
        let next = after.trim_start();
        let word: String = next.chars().take_while(|c| c.is_alphabetic()).collect::<String>().to_lowercase();
        let unit = after.chars().next();
        let claim = if matches!(unit, Some('%')) || next.starts_with("percent") {
            Some("a percentage")
        } else if matches!(unit, Some('x' | '×')) && !after.chars().nth(1).is_some_and(char::is_alphanumeric) {
            Some("a multiple")
        } else if matches!(unit, Some('+')) {
            Some("a count with +")
        } else if matches!(unit, Some('k' | 'K' | 'M')) && !after.chars().nth(1).is_some_and(char::is_alphanumeric) {
            Some("a count in thousands or millions")
        } else if unit == Some('/') && after.chars().nth(1).is_some_and(|c| c.is_ascii_digit()) {
            Some("a rating or ratio")
        } else if before.ends_with(['$', '€', '£']) && !known(v) {
            Some("a price")
        } else if !known(v) && COUNTED.contains(&word.as_str()) {
            Some("a count")
        } else {
            None
        };
        if let Some(what) = claim {
            let shown = format!("{raw}{}", after.chars().take(12).collect::<String>());
            if !brief.contains(shown.trim()) {
                found.push(format!("{what}: \"{}{}\"", before.trim_start(), shown.trim_end()));
            }
        }
    }
    found.dedup();
    found
}

// ── The reviewer: judgement, with no context but the rubric and the pictures ──

/// The rubric for the landing page, fixed: the same for every run of A and B.
pub const LANDING_RUBRIC: &str = "You are reviewing screenshots of a product landing page, at a desktop width (1440 px, files named desktop-*) and a phone width (360 px, files named phone-*), each cut top to bottom into tiles in order. You know nothing else about who made it or how. The brief it was made from: Quillo turns a recorded meeting into a one-page summary (decisions, owners, to-dos); recordings up to 2 hours; share as a link or PDF; free for 3 meetings a month, team plan $8 per person per month; beta opens 3 March; nobody has used it yet.

Score each criterion from 1 (poor) to 5 (excellent), judging only what you see:
1. hierarchy — one clear focal point on the first screen; headline, supporting line and the main action read in that order; sections are easy to tell apart.
2. typography — a deliberate type scale and weights, comfortable line lengths, nothing too small to read.
3. layout — consistent spacing rhythm and alignment; nothing cramped, orphaned or awkwardly empty.
4. specificity — the page is about this product, not a template: penalize generic AI-landing-page habits (purple-blue gradients, glassy cards, emoji or stock icons in three identical cards, vague hype like 'unlock' or 'supercharge', gradient text, decoration that says nothing).
5. phone — at 360 px it holds up: readable, sensible stacking, the main action easy to reach.
6. finish — the details are done: consistent radii, borders and shadows, aligned icons, no broken, placeholder or clipped bits.

Open every image with the Read tool first. Then answer with only one JSON object and nothing else: {\"hierarchy\": n, \"typography\": n, \"layout\": n, \"specificity\": n, \"phone\": n, \"finish\": n, \"worst\": \"the single biggest problem, in one sentence\"}";

/// The rubric for the empty-state mockup, fixed likewise.
pub const EMPTY_STATE_RUBRIC: &str = "You are reviewing mockups of an empty state: what the 'Saved' page of a reading app (people save articles to read later) shows before the person has saved anything. The page's existing styles: a warm off-white paper background (#fbfaf7), dark ink text (#1d1d1f), muted grey (#6e6e73), a rust accent (#b4532a), Georgia serif type, 10 px radius, 8 px spacing unit. You know nothing else about who made it or how. If there are several images, they are options or views of one design: judge the strongest.

Score each criterion from 1 (poor) to 5 (excellent), judging only what you see:
1. clarity — it is obvious at a glance that nothing is saved yet and what to do about it.
2. guidance — it tells the person how to save their first article, in plain words, with at most one main action.
3. fit — it uses the page's own styles above (colours, serif type, spacing) rather than a different look.
4. specificity — it is about saving articles to read, not a generic 'nothing here' with a stock illustration or emoji.
5. finish — hierarchy, spacing and alignment are done; nothing clipped, placeholder or awkward.

Open every image with the Read tool first. Then answer with only one JSON object and nothing else: {\"clarity\": n, \"guidance\": n, \"fit\": n, \"specificity\": n, \"finish\": n, \"worst\": \"the single biggest problem, in one sentence\"}";

/// What the reviewer said.
#[derive(Debug, Clone)]
pub struct Review {
    pub score: f64,
    pub max: f64,
    pub scores: Value,
    pub tokens: u64,
    pub cost_usd: f64,
}

/// The first JSON object in `text`.
pub fn first_object(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let mut depth = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in text[start..].char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '{' if !quoted => depth += 1,
            '}' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&text[start..=start + i]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

/// The sum of a review's 1–5 scores, and the most it could be.
pub fn total(scores: &Value) -> (f64, f64) {
    let nums: Vec<f64> = scores.as_object().into_iter().flatten().filter_map(|(_, v)| v.as_f64()).filter(|v| (1.0..=5.0).contains(v)).collect();
    (nums.iter().sum(), nums.len() as f64 * 5.0)
}

/// `images` judged against `rubric` by a reviewer in a folder of its own: the
/// person's own `claude` with its normal login, one turn, the Read tool only,
/// no MCP servers, no skills, no saved session. The pictures are copied in
/// under neutral names (`names(i)`), so nothing tells it which variant made them.
pub fn review(images: &[(String, PathBuf)], rubric: &str, dir: &Path) -> Result<Review, String> {
    if images.is_empty() {
        return Err("no pictures to review".into());
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut listed = Vec::new();
    for (name, from) in images {
        let to = dir.join(name);
        std::fs::copy(from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        listed.push(to.display().to_string());
    }
    std::fs::write(dir.join("rubric.txt"), rubric).map_err(|e| e.to_string())?;
    if review_engine() == "codex" {
        return review_on_codex(images, rubric, dir, &listed);
    }
    let prompt = format!("{rubric}\n\nThe images:\n{}", listed.join("\n"));
    let model = std::env::var("EQUIPE_EVAL_REVIEW_MODEL").ok().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| "opus".into());
    let o = Process::new("claude")
        .args(["-p", &prompt, "--model", &model, "--output-format", "json", "--allowedTools", "Read", "--strict-mcp-config", "--disable-slash-commands", "--no-session-persistence"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("claude: {e}"))?;
    let out: Value = serde_json::from_slice(&o.stdout).map_err(|_| format!("the reviewer printed no JSON: {}", String::from_utf8_lossy(&o.stderr).chars().take(200).collect::<String>()))?;
    let _ = std::fs::write(dir.join("review.json"), serde_json::to_string_pretty(&out).unwrap_or_default());
    let said = out["result"].as_str().unwrap_or("");
    let scores = first_object(said).ok_or_else(|| format!("no scores in the review: {}", said.chars().take(200).collect::<String>()))?;
    let (score, max) = total(&scores);
    if max == 0.0 {
        return Err(format!("no 1–5 scores in {scores}"));
    }
    let u = &out["usage"];
    let tokens = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens", "output_tokens"].iter().map(|k| u[*k].as_u64().unwrap_or(0)).sum();
    Ok(Review { score, max, scores, tokens, cost_usd: out["total_cost_usd"].as_f64().unwrap_or(0.0) })
}

/// The reviewer's engine: `EQUIPE_EVAL_REVIEW_ENGINE` (`claude` or `codex`),
/// else the members' when the run puts them all on Codex, else Claude.
pub fn review_engine() -> String {
    let var = |n: &str| std::env::var(n).ok().map(|v| v.trim().to_lowercase()).filter(|v| !v.is_empty());
    var("EQUIPE_EVAL_REVIEW_ENGINE").unwrap_or_else(|| if var("EQUIPE_EVAL_ENGINE").as_deref() == Some("codex") { "codex".into() } else { "claude".into() })
}

/// The rubric as Codex reads it: the pictures come attached, not to be opened.
pub fn for_codex(rubric: &str) -> String {
    rubric.replace("Open every image with the Read tool first.", "The images are attached, in the order listed.")
}

/// The same review on Codex: `codex exec`, read-only, not kept (`--ephemeral`),
/// the pictures attached; its usage read from its JSON events.
fn review_on_codex(images: &[(String, PathBuf)], rubric: &str, dir: &Path, listed: &[String]) -> Result<Review, String> {
    let names: Vec<&str> = images.iter().map(|(n, _)| n.as_str()).collect();
    let prompt = format!("{}\n\nThe images: {}", for_codex(rubric), names.join(", "));
    let answer = dir.join("answer.md");
    let mut args: Vec<String> = ["exec", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral", "--color", "never", "--json"].map(String::from).into();
    if let Some(model) = std::env::var("EQUIPE_EVAL_REVIEW_MODEL").ok().filter(|m| !m.trim().is_empty()) {
        args.extend(["--model".into(), model]);
    }
    for image in listed {
        args.extend(["--image".into(), image.clone()]);
    }
    args.extend(["--output-last-message".into(), answer.display().to_string(), prompt]);
    let o = Process::new("codex").args(&args).current_dir(dir).stdin(Stdio::null()).output().map_err(|e| format!("codex: {e}"))?;
    let events = String::from_utf8_lossy(&o.stdout).into_owned();
    let _ = std::fs::write(dir.join("review.jsonl"), &events);
    let said = std::fs::read_to_string(&answer).map_err(|_| format!("the reviewer wrote no answer: {}", String::from_utf8_lossy(&o.stderr).chars().take(200).collect::<String>()))?;
    let scores = first_object(&said).ok_or_else(|| format!("no scores in the review: {}", said.chars().take(200).collect::<String>()))?;
    let (score, max) = total(&scores);
    if max == 0.0 {
        return Err(format!("no 1–5 scores in {scores}"));
    }
    Ok(Review { score, max, scores, tokens: codex_tokens(&events), cost_usd: 0.0 })
}

/// The tokens a `codex exec --json` run reports: each turn's input (cached
/// ones included) and output.
pub fn codex_tokens(events: &str) -> u64 {
    events
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["type"] == "turn.completed")
        .map(|e| ["input_tokens", "output_tokens"].iter().map(|k| e["usage"][*k].as_u64().unwrap_or(0)).sum::<u64>())
        .sum()
}

/// A review as a check of its own: never required — it informs, the
/// automatic checks decide a run's pass.
pub fn review_check(r: &Result<Review, String>) -> Check {
    match r {
        Ok(r) => should("the reviewer's score", r.score >= 0.6 * r.max, format!("{}/{} {}", r.score, r.max, r.scores)),
        Err(e) => should("the reviewer's score", false, e.clone()),
    }
}
