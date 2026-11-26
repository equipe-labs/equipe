//! Design QA's precheck: what can be told about a hand-in's screens without
//! a model, before any designer turn (docs/DESIGN-QA.md).
//!
//! `equipe design precheck` renders the screens a change touches — the UI
//! map's states whose `source` lists a changed file — from the ticket's
//! worktree, at each width, light and dark, at normal size, with text at
//! 200% and with the page zoomed to 200% (18 conditions by default). In each
//! rendered page `precheck.mjs` measures what is deterministic: text cut off
//! by its box, text over text, what reaches past the window, the page
//! scrolling sideways, axe-core's WCAG A/AA rules, and the countable part of
//! the designer's taste checklist ([`Taste`], polish only). Each normal picture is
//! compared, pixel by pixel, with the map's picture committed at the base.
//!
//! The judging is here, so it is tested like the rest: [`assemble`] turns the
//! measures and pictures into a [`Report`] — the findings classed as content
//! lost or cosmetic, blocker (S1/S2) or polish (S3/S4), new on a changed
//! screen or there before — with crops of what it flags, and a short
//! Markdown summary for the person and the designer. What the report decides
//! for the ticket is `crate::design_qa::gate`.
//!
//! The map may say how the precheck runs its pages, in `flows.json`:
//!
//! ```json
//! { "precheck": { "preview": "npm run dev -- --port {port}", "origin": "http://localhost:3000",
//!                 "ready": "/", "modes": ["light-large"], "widths": [390, 768, 1280], "maxStates": 12 },
//!   "flows": [ … ] }
//! ```
//!
//! - `preview`: a shell command, run at the root of the ticket's worktree,
//!   that serves the app on `{port}`; the map's URLs written against `origin`
//!   are opened on it instead. Without one (and without `--url`), a state
//!   shot by its URL can't be rendered from the ticket's own code and is
//!   reported as not rendered.
//! - `modes`: more modes a state's `command` is run in, beside light and dark.
//! - `widths`, `maxStates`: the widths, and how many states at most are
//!   rendered; more are reported as left out. The states are taken in order
//!   of relevance ([`prioritize`]): those of screens whose `source` lists a
//!   changed file first, each screen's default state before its others.
//!   Without `maxStates`, [`MAX_STATES`] is the ceiling and the time budget
//!   (`--budget-secs`) the real limit: no state is started past it.
//! - `unmappedLarge`: how many changed UI files no screen lists make that
//!   a large change a designer's top tier looks at ([`UNMAPPED_LARGE`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The report's files, in the ticket's design QA folder.
pub const REPORT_JSON: &str = "report.json";
pub const REPORT_MD: &str = "report.md";
/// What a report on disk is written as.
pub const SCHEMA: u32 = 1;
/// The widths rendered unless the map says others.
pub const WIDTHS: [u32; 3] = [390, 768, 1280];
/// Light and dark.
pub const SCHEMES: [&str; 2] = ["light", "dark"];
/// The narrowest viewport, in CSS px, WCAG 2.2's reflow (1.4.10) asks
/// content to fit without scrolling sideways: narrower, a sideways scroll is
/// noted, never a blocker.
pub const REFLOW_MIN: u32 = 320;
/// The share of a picture's pixels that may differ before it counts as changed.
pub const CHANGED_SHARE: f64 = 0.001;
/// A colour channel differing by more than this makes a pixel differ.
const PIXEL_TOLERANCE: u8 = 24;
/// How many states are rendered at most unless the map says. High on
/// purpose: the render time budget is what stops a big change, and what it
/// leaves out is reported, never passed.
pub const MAX_STATES: usize = 40;
/// So many changed UI files no screen lists make an unmapped change large.
pub const UNMAPPED_LARGE: usize = 10;
/// How many left-out states a report names.
const MAX_LEFT_OUT_LISTED: usize = 200;
/// How many crops a report cuts at most.
const MAX_CROPS: usize = 24;
/// How many flagged pictures the designer is pointed at.
pub const MAX_FLAGGED: usize = 16;
/// Two texts overlap when this share of the smaller is covered.
const OVERLAP_SHARE: f64 = 0.3;
/// Room left around what a crop shows, in CSS px.
const CROP_PAD: f64 = 24.;

// ── conditions ──

/// How a page is drawn beside its width and scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Variant {
    /// As it is.
    Normal,
    /// Every text at twice its size (WCAG 1.4.4, resize text).
    Text200,
    /// The page zoomed to 200%: half as many CSS px across (WCAG 1.4.10, reflow).
    Zoom200,
}

impl Variant {
    pub const ALL: [Variant; 3] = [Variant::Normal, Variant::Text200, Variant::Zoom200];

    pub fn code(self) -> &'static str {
        match self {
            Variant::Normal => "normal",
            Variant::Text200 => "text-200",
            Variant::Zoom200 => "zoom-200",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Variant::ALL.into_iter().find(|v| v.code() == text)
    }

    /// The CSS width a window `width` px across has in this variant.
    pub fn css_width(self, width: u32) -> u32 {
        match self {
            Variant::Zoom200 => width / 2,
            _ => width,
        }
    }
}

/// One way a screen is looked at: `390.dark.text-200`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Condition {
    pub width: u32,
    /// `light`, `dark`, or a mode the map's commands know (`light-large`).
    pub scheme: String,
    pub variant: Variant,
}

impl Condition {
    pub fn new(width: u32, scheme: &str, variant: Variant) -> Self {
        Condition { width, scheme: scheme.to_owned(), variant }
    }

    pub fn code(&self) -> String {
        format!("{}.{}.{}", self.width, self.scheme, self.variant.code())
    }

    pub fn parse(code: &str) -> Option<Self> {
        let mut parts = code.splitn(3, '.');
        let width = parts.next()?.parse().ok()?;
        let scheme = parts.next().filter(|s| !s.is_empty())?;
        let variant = Variant::parse(parts.next()?)?;
        Some(Condition::new(width, scheme, variant))
    }

    /// The same width and scheme at normal size: whose picture says whether
    /// the screen changed.
    pub fn normal(&self) -> Condition {
        Condition { variant: Variant::Normal, ..self.clone() }
    }

    /// Whether WCAG's reflow holds here: the CSS viewport is at least [`REFLOW_MIN`].
    pub fn in_wcag_scope(&self) -> bool {
        self.variant.css_width(self.width) >= REFLOW_MIN
    }
}

/// Every condition at `widths` in `schemes`: each width, scheme and variant.
pub fn conditions(widths: &[u32], schemes: &[&str]) -> Vec<Condition> {
    widths.iter().flat_map(|&w| schemes.iter().flat_map(move |s| Variant::ALL.into_iter().map(move |v| Condition::new(w, s, v)))).collect()
}

// ── the map's say ──

/// The preview a worktree serves itself with when its map names none: a Vite
/// project with its dependencies installed runs Vite's dev server on the
/// precheck's port. Without one the precheck of a page shot by URL renders
/// nothing, and what it can't see waits for a designer
/// (`crate::design_qa::Risk::Unverified`): a picture it can take saves that turn.
pub fn default_preview(root: &Path) -> Option<String> {
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(root.join("package.json")).ok()?).ok()?;
    let uses_vite = ["dependencies", "devDependencies"].iter().any(|k| manifest.get(k).and_then(|d| d.get("vite")).is_some());
    (uses_vite && root.join("node_modules/.bin/vite").exists()).then(|| "node_modules/.bin/vite --port {port} --strictPort --host 127.0.0.1".to_owned())
}


/// How the map asks to be prechecked (`flows.json`'s `precheck`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub preview: Option<String>,
    pub origin: Option<String>,
    pub ready: Option<String>,
    pub modes: Vec<String>,
    pub widths: Vec<u32>,
    pub max_states: Option<usize>,
    pub unmapped_large: Option<usize>,
}

impl Config {
    /// The map's own, or none when it says nothing (or nothing readable).
    pub fn of(map: &Value) -> Config {
        map.get("precheck").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }

    pub fn widths(&self) -> Vec<u32> {
        if self.widths.is_empty() { WIDTHS.to_vec() } else { self.widths.clone() }
    }

    pub fn max_states(&self) -> usize {
        self.max_states.unwrap_or(MAX_STATES).max(1)
    }

    pub fn unmapped_large(&self) -> usize {
        self.unmapped_large.unwrap_or(UNMAPPED_LARGE).max(1)
    }

    /// `url` written against the map's origin, opened on `preview` instead.
    pub fn rebase(&self, url: &str, preview: &str) -> String {
        let preview = preview.trim_end_matches('/');
        match self.origin.as_deref().map(|o| o.trim_end_matches('/')) {
            Some(origin) if !origin.is_empty() && url.starts_with(origin) => format!("{preview}{}", &url[origin.len()..]),
            // No origin said: whatever scheme, host and port the URL has.
            None => match url.find("://").map(|at| at + 3) {
                Some(host) => {
                    let path = url[host..].find('/').map_or(url.len(), |p| host + p);
                    format!("{preview}{}", &url[path..])
                }
                None => url.to_owned(),
            },
            _ => url.to_owned(),
        }
    }
}

/// A screen of the map: `flow/screen`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ScreenRef {
    pub flow: String,
    pub screen: String,
}

impl ScreenRef {
    pub fn key(&self) -> String {
        format!("{}/{}", self.flow, self.screen)
    }
}

/// What a change touches of the map.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Touched {
    /// The screens one of whose `source` files changed, or whose pictures did.
    pub screens: Vec<ScreenRef>,
    /// Changed files that can change what users see and that no screen lists.
    pub unmapped: Vec<String>,
    /// Changed files no screen lists that look like code or config users
    /// never see (told apart by what is in them: [`split_unmapped`]).
    pub quiet: Vec<String>,
    /// Per screen (`flow/screen`), how many changed files its `source` lists:
    /// the screens whose own code changed, as against those only a picture of
    /// which changed.
    pub source_hits: BTreeMap<String, usize>,
}

/// What of `map` (`flows.json`, its flows folder at `flows_dir` from the
/// repository's root) the `changed` files touch.
pub fn touched(map: &Value, flows_dir: &str, changed: &[String]) -> Touched {
    let flows_dir = flows_dir.trim_end_matches('/');
    let mut screens = BTreeSet::new();
    let mut mapped = BTreeSet::new();
    let mut source_hits: BTreeMap<String, usize> = BTreeMap::new();
    for flow in map.get("flows").and_then(Value::as_array).into_iter().flatten() {
        let Some(flow_id) = flow.get("id").and_then(Value::as_str) else { continue };
        for screen in flow.get("screens").and_then(Value::as_array).into_iter().flatten() {
            let Some(screen_id) = screen.get("id").and_then(Value::as_str) else { continue };
            let sources: Vec<&str> = screen.get("source").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
            let pictures = format!("{flows_dir}/{flow_id}/{screen_id}/");
            for file in changed {
                if sources.iter().any(|s| source_matches(s, file)) {
                    *source_hits.entry(format!("{flow_id}/{screen_id}")).or_default() += 1;
                }
                if file.starts_with(&pictures) || sources.iter().any(|s| source_matches(s, file)) {
                    screens.insert(ScreenRef { flow: flow_id.to_owned(), screen: screen_id.to_owned() });
                    mapped.insert(file.clone());
                }
            }
        }
    }
    let unmapped = changed.iter().filter(|f| !mapped.contains(*f) && !f.starts_with(&format!("{flows_dir}/")) && ui_file(f)).cloned().collect();
    Touched { screens: screens.into_iter().collect(), unmapped, quiet: Vec::new(), source_hits }
}

/// What of the UI map of the app at `app` (from the repository's root; `""`
/// for the root's own map, exactly [`touched`]) the `changed` files touch.
/// An app's map names its files from the app's folder (`src/App.tsx`) and
/// keeps its pictures in the app's own `design/flows/`; changed files outside
/// the app that can change what users see are unmapped.
pub fn touched_in_app(map: &Value, app: &str, changed: &[String]) -> Touched {
    let flows_dir = format!("{}/{}", crate::design::DIR, crate::design::flows::DIR);
    if app.is_empty() {
        return touched(map, &flows_dir, changed);
    }
    let prefix = format!("{}/", app.trim_end_matches('/'));
    let inside: Vec<String> = changed.iter().filter_map(|f| f.strip_prefix(&prefix).map(str::to_owned)).collect();
    let mut found = touched(map, &flows_dir, &inside);
    found.unmapped = found.unmapped.into_iter().map(|f| format!("{prefix}{f}")).collect();
    found.unmapped.extend(changed.iter().filter(|f| !f.starts_with(&prefix) && ui_file(f)).cloned());
    found
}

/// Order `items` by how relevant they are to the change, most first: the
/// states of screens whose own `source` changed (most changed files first),
/// each screen's default state before its others, the rest in their order.
/// `key` gives an item's screen (`flow/screen`) and whether it is a default state.
pub fn prioritize<T>(items: &mut [T], hits: &BTreeMap<String, usize>, key: impl Fn(&T) -> (String, bool)) {
    items.sort_by_cached_key(|item| {
        let (screen, default) = key(item);
        let hit = hits.get(&screen).copied().unwrap_or(0);
        (hit == 0, !default, std::cmp::Reverse(hit))
    });
}

/// Keep the first `max` of the already prioritized `items`: what is kept,
/// and what was left out.
pub fn take_most_relevant<T>(mut items: Vec<T>, max: usize) -> (Vec<T>, Vec<T>) {
    let rest = items.split_off(max.min(items.len()));
    (items, rest)
}

/// Whether `file` is what `source` names: the file itself, a folder it is
/// in (`src/components/`), or a pattern with one `*` (`app/styles/*.css`).
pub fn source_matches(source: &str, file: &str) -> bool {
    let source = source.trim().trim_start_matches("./");
    if source.is_empty() {
        return false;
    }
    if let Some((before, after)) = source.split_once('*') {
        let after = after.trim_start_matches('*');
        return file.len() >= before.len() + after.len() && file.starts_with(before) && file.ends_with(after);
    }
    file == source || (source.ends_with('/') && file.starts_with(source)) || file.starts_with(&format!("{source}/"))
}

/// What a changed file's path says of whether users see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visual {
    /// Styles, markup, templates, the words shown: users see it.
    Yes,
    /// Tests, docs, config, server code, type declarations: they don't.
    No,
    /// Code that may or may not draw anything (a `.tsx` without JSX is a
    /// helper, a `.ts` of types only is declarations): [`shows_ui`] reads it.
    Maybe,
}

/// Files that are styles or markup whatever their folder: users see them.
const MARKUP: [&str; 38] = [
    "css", "scss", "sass", "less", "styl", "vue", "svelte", "astro", "html", "htm", "erb", "haml", "slim", "hbs", "ejs", "twig", "liquid", "swift", "kt", "dart", "xaml", "xib", "pug", "jade", "njk", "jinja", "jinja2", "j2",
    "mustache", "phtml", "cshtml", "razor", "jsp", "xhtml", "leaf", "storyboard", "mdx", "svg",
];
/// Script files: whether one is UI depends on where it is and what is in it.
const SCRIPT: [&str; 8] = ["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];
/// Folders whose code is the server's, a script or a migration, even inside an `app/` folder.
const SERVER_DIRS: [&str; 14] = ["api/", "server/", "functions/", "middleware/", "migrations/", "db/", "prisma/", "scripts/", "workers/", "jobs/", "cron/", "e2e/", "mocks/", "fixtures/"];
/// Folders of shared logic: not UI unless a UI folder is also in the path.
const LOGIC_DIRS: [&str; 10] = ["lib/", "utils/", "util/", "helpers/", "services/", "types/", "config/", "constants/", "schemas/", "models/"];
/// Folders of what users see.
const UI_DIRS: [&str; 8] = ["components/", "app/", "pages/", "views/", "ui/", "screens/", "layouts/", "src/routes/"];
/// Folders of the words shown.
const WORD_DIRS: [&str; 5] = ["locales/", "i18n/", "messages/", "lang/", "translations/"];

fn in_dir(lower: &str, dir: &str) -> bool {
    lower.starts_with(dir) || lower.contains(&format!("/{dir}"))
}

/// Whether the path of a changed file says users see it, by its name and
/// folders alone, for any language or framework: styles, markup and
/// templates always; script code only outside server and shared-logic
/// folders (and then [`shows_ui`] has the last word); locale files; design
/// tokens; never tests, docs, config or type declarations.
pub fn visual_path(path: &str) -> Visual {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    if test_or_docs(&lower, name) {
        return Visual::No;
    }
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    if MARKUP.contains(&ext) || name.starts_with("tailwind.config") || name == "tokens.json" || name.ends_with(".tokens.json") {
        return Visual::Yes;
    }
    if matches!(ext, "json" | "yaml" | "yml" | "po" | "strings" | "xliff" | "arb" | "resx") {
        return if WORD_DIRS.iter().any(|d| in_dir(&lower, d)) { Visual::Yes } else { Visual::No };
    }
    if !SCRIPT.contains(&ext) {
        return Visual::No;
    }
    // Type declarations and tool config are never what users see.
    let config = name.ends_with(".d.ts") || name.contains(".config.") || name.contains(".setup.") || name.contains(".conf.") || name.starts_with("jest.") || name.starts_with("vitest.") || name.starts_with("vite.") || name.starts_with("next.config");
    if config || SERVER_DIRS.iter().any(|d| in_dir(&lower, d)) {
        return Visual::No;
    }
    if matches!(ext, "tsx" | "jsx") {
        return Visual::Maybe;
    }
    // Other scripts are UI code only when a UI folder is nearer the file than any shared-logic folder.
    match (last_dir(&lower, &UI_DIRS), last_dir(&lower, &LOGIC_DIRS)) {
        (Some(ui), Some(logic)) if logic > ui => Visual::No,
        (Some(_), _) => Visual::Maybe,
        _ => Visual::No,
    }
}

/// Whether a lowercased path (`name` its last part) is a test or a document.
fn test_or_docs(lower: &str, name: &str) -> bool {
    let test = name.contains(".test.") || name.contains(".spec.") || name.contains(".stories.") || lower.contains("__tests__/") || lower.contains("__mocks__/") || in_dir(lower, "tests/") || in_dir(lower, "test/");
    let docs = name.ends_with(".md") || name.ends_with(".txt") || name.ends_with(".rst") || in_dir(lower, "docs/") || name.starts_with("readme") || name.starts_with("changelog") || name.starts_with("license");
    test || docs
}

/// Where the deepest of `dirs` that `lower` is in starts, if it is in one.
fn last_dir(lower: &str, dirs: &[&str]) -> Option<usize> {
    dirs.iter().filter_map(|d| if lower.starts_with(d) { Some(0) } else { lower.rfind(&format!("/{d}")).map(|i| i + 1) }).max()
}

/// Whether a changed file's path can change what users see — [`visual_path`]
/// without reading the file: a `.tsx` or a script in a UI folder counts.
pub fn ui_file(path: &str) -> bool {
    visual_path(path) != Visual::No
}

/// Whether the contents of a script file `path` (one [`visual_path`] said
/// `Maybe` of) draw something: JSX/markup in a `.tsx`/`.jsx`, and in other
/// scripts anything but type declarations.
pub fn shows_ui(path: &str, text: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".tsx") || lower.ends_with(".jsx") {
        return text.contains("/>") || text.contains("</") || text.contains("<>");
    }
    !type_only(text)
}

/// Whether script `text` declares types and nothing else: every top-level
/// (unindented) statement is an import, a type, an interface or a re-export.
fn type_only(text: &str) -> bool {
    let mut in_block = false;
    let mut any = false;
    for line in text.lines() {
        let head = line.trim_start();
        if in_block {
            in_block = !head.contains("*/");
            continue;
        }
        if head.is_empty() || head.starts_with("//") {
            continue;
        }
        if head.starts_with("/*") {
            in_block = !head.contains("*/");
            continue;
        }
        if line.starts_with(char::is_whitespace) || line.starts_with(['}', ')', ']', '|', '&']) {
            continue;
        }
        any = true;
        let rest = head.strip_prefix("export ").map_or(head, |r| r.strip_prefix("default ").unwrap_or(r));
        let rest = rest.strip_prefix("declare ").unwrap_or(rest);
        if !["import ", "import{", "type ", "interface ", "namespace ", "{", "*"].iter().any(|k| rest.starts_with(k)) {
            return false;
        }
    }
    any
}

/// Split the changed UI-looking files no screen lists into those users do
/// see and those they don't, by reading each (`read` gives a file's text,
/// `None` when it can't be read — a deleted file stays visual, to be safe):
/// `(visual, quiet)`.
pub fn split_unmapped(unmapped: Vec<String>, read: impl Fn(&str) -> Option<String>) -> (Vec<String>, Vec<String>) {
    let mut visual = Vec::new();
    let mut quiet = Vec::new();
    for file in unmapped {
        let shows = match visual_path(&file) {
            Visual::Yes => true,
            Visual::No => false,
            Visual::Maybe => read(&file).is_none_or(|text| shows_ui(&file, &text)),
        };
        if shows { visual.push(file) } else { quiet.push(file) }
    }
    (visual, quiet)
}

// ── the app a change is in, and its pages without a map ──
//
// A repository may hold several apps (`cafe/`, `habits/`, `o1path/`), each
// with its own `package.json` or `index.html` and its own `design/` folder,
// and most small products never get a UI map at all. The precheck looks for
// the map at the repository's root first (as it always has), then in each
// changed app's own design folder ([`map_dirs`]). With none, it still renders
// the changed app's own pages ([`serve_plan`], [`fallback_routes`]) at every
// condition and measures them, so the gate gets pictures and findings instead
// of "nothing to render".

/// How many pages of an app without a UI map are rendered at most: its home
/// and a few language versions of it.
pub const MAX_FALLBACK_ROUTES: usize = 4;

/// Image files: not code, but a changed one (a logo) is what users see.
const IMAGES: [&str; 8] = ["png", "jpg", "jpeg", "webp", "gif", "avif", "ico", "bmp"];

/// Folders whose `index.html` is an app's assets or output, never the app.
const NOT_APP_DIRS: [&str; 8] = ["public", "static", "assets", "design", "dist", "build", "out", "node_modules"];

/// Whether a changed file can change what an app's pages show, to find the
/// app it belongs to: a UI file ([`ui_file`]), an image, or what an app's
/// `design/` folder holds (tokens, brand, assets) — never a test or a document.
pub fn shown_by_app(path: &str) -> bool {
    if ui_file(path) {
        return true;
    }
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    if test_or_docs(&lower, name) {
        return false;
    }
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    IMAGES.contains(&ext) || (ext == "json" && in_dir(&lower, "design/"))
}

/// The app folder `file` (from the repository's root) belongs to: the
/// nearest folder above it with a `package.json` or an `index.html` — an
/// `index.html` inside a `public/`, `design/` or build folder being the
/// app's assets, not the app — else the repository's root (`""`). `has`
/// says whether a file, from the repository's root, exists.
pub fn app_root(file: &str, has: impl Fn(&str) -> bool) -> String {
    let mut dir = file.rsplit_once('/').map_or("", |(d, _)| d);
    while !dir.is_empty() {
        let assets = dir.split('/').any(|part| NOT_APP_DIRS.contains(&part.to_ascii_lowercase().as_str()));
        if has(&format!("{dir}/package.json")) || (!assets && has(&format!("{dir}/index.html"))) {
            return dir.to_owned();
        }
        dir = dir.rsplit_once('/').map_or("", |(d, _)| d);
    }
    String::new()
}

/// The app folders the `changed` files that can change what users see
/// belong to ([`app_root`]), the one with the most such files first; `""` is
/// the repository's root.
pub fn app_roots(changed: &[String], has: impl Fn(&str) -> bool) -> Vec<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for file in changed.iter().filter(|f| shown_by_app(f)) {
        *counts.entry(app_root(file, &has)).or_default() += 1;
    }
    let mut roots: Vec<(String, usize)> = counts.into_iter().collect();
    roots.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    roots.into_iter().map(|(root, _)| root).collect()
}

/// Where a change's UI map is looked for, the first found wins: the
/// repository's own `design/` folder (as always), then each changed app's
/// (`o1path/design`), as design folders from the repository's root.
pub fn map_dirs(apps: &[String]) -> Vec<String> {
    let mut out = vec![crate::design::DIR.to_owned()];
    for app in apps.iter().filter(|a| !a.is_empty()) {
        let dir = format!("{app}/{}", crate::design::DIR);
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

/// How the pages of an app without a UI map are served for the precheck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Serve {
    /// Built by its own Vite (`vite build --base /`) into a folder of the
    /// precheck's — never into the worktree — and that folder served as files.
    Vite,
    /// Its folder served as it is: plain HTML, CSS and scripts.
    Files,
}

impl Serve {
    /// How it was served, for the report.
    pub fn says(self) -> &'static str {
        match self {
            Serve::Vite => "built with its own Vite",
            Serve::Files => "its files served as they are",
        }
    }
}

/// How the pages of the app at `app` are served without a UI map, or why
/// they can't be: a Vite app with its dependencies installed is built; an
/// app with an `index.html` and no build step is served as it is. Any other
/// build isn't guessed at (its pictures would be of unbuilt code).
pub fn serve_plan(app: &Path) -> Result<Serve, String> {
    let manifest: Option<Value> = std::fs::read_to_string(app.join("package.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
    let has_dep = |name: &str| manifest.as_ref().is_some_and(|m| ["dependencies", "devDependencies"].iter().any(|k| m.get(k).and_then(|d| d.get(name)).is_some()));
    if has_dep("vite") {
        return if app.join("node_modules/.bin/vite").exists() { Ok(Serve::Vite) } else { Err("it builds with Vite and its dependencies aren't installed here (no node_modules/.bin/vite)".into()) };
    }
    if let Some(script) = manifest.as_ref().and_then(|m| m.get("scripts")?.get("build")?.as_str()) {
        return Err(format!("it builds with `{script}`, which the precheck runs only for Vite apps: give it a UI map with `precheck.preview`"));
    }
    if app.join("index.html").is_file() { Ok(Serve::Files) } else { Err("it has no index.html to serve and no Vite build".into()) }
}

/// Whether a folder's name reads as a language: `en`, `ko`, `pt-br`, `zh_Hant`.
fn locale_like(name: &str) -> bool {
    let (lang, region) = name.split_once(['-', '_']).unwrap_or((name, ""));
    lang.len() == 2 && lang.bytes().all(|b| b.is_ascii_lowercase()) && (region.is_empty() || ((2..=4).contains(&region.len()) && region.bytes().all(|b| b.is_ascii_alphabetic())))
}

/// The pages of the served app at `site` to render without a UI map: its
/// home (`/`), and the language versions its build or folder has
/// (`en/index.html` → `/en/`), at most [`MAX_FALLBACK_ROUTES`] in all.
pub fn fallback_routes(site: &Path) -> Vec<String> {
    let mut locales: Vec<String> = std::fs::read_dir(site)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|name| locale_like(name) && site.join(name).join("index.html").is_file())
        .collect();
    locales.sort();
    let home = site.join("index.html").is_file() || locales.is_empty();
    let mut out: Vec<String> = if home { vec!["/".to_owned()] } else { Vec::new() };
    out.extend(locales.into_iter().map(|l| format!("/{l}/")));
    out.truncate(MAX_FALLBACK_ROUTES);
    out
}

/// A page of an app without a map, as a screen's name: `/` is `home`, `/en/` is `en`.
pub fn route_screen(route: &str) -> String {
    let trimmed = route.trim_matches('/');
    if trimmed.is_empty() { "home".to_owned() } else { trimmed.replace('/', "-") }
}

/// An app folder without a map, as a flow's name: the root is `app`, `apps/web` is `apps-web`.
pub fn app_flow(app: &str) -> String {
    if app.is_empty() { "app".to_owned() } else { app.trim_matches('/').replace('/', "-") }
}

/// The file the precheck's static server answers request `target` with,
/// from the served folder `site`: the file itself, a folder's `index.html`,
/// and for a client-side route (no extension in its last part) the app's
/// own `index.html`. Never anything outside `site` or hidden (`.env`, `.git`).
pub fn static_file(site: &Path, target: &str) -> Option<PathBuf> {
    let path = percent_decode(target.split(['?', '#']).next().unwrap_or("/"));
    let mut at = site.to_path_buf();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part.starts_with('.') || part.contains('\\') {
            return None;
        }
        at.push(part);
    }
    if at.is_dir() {
        at.push("index.html");
    }
    if at.is_file() {
        return Some(at);
    }
    let last = path.rsplit('/').next().unwrap_or_default();
    let index = site.join("index.html");
    (!last.contains('.') && index.is_file()).then_some(index)
}

/// `%XX` escapes in a URL's path, decoded.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%').then(|| text.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())).flatten();
        match hex {
            Some(b) => {
                out.push(b);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The `Content-Type` the precheck's static server sends `path` with.
pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs" | "cjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml",
        Some("wasm") => "application/wasm",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

// ── what precheck.mjs measures ──

/// A box in the page, in CSS px from the document's top left.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    fn scaled(&self, by: f64) -> Rect {
        Rect { x: self.x * by, y: self.y * by, w: self.w * by, h: self.h * by }
    }
}

/// One picture taken, and — rendered by its URL — what was measured in it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Taken {
    /// `flow/screen/state`.
    pub key: String,
    /// [`Condition::code`].
    pub condition: String,
    pub path: Option<String>,
    /// Device pixels per CSS px in the picture.
    pub dpr: f64,
    /// Why it couldn't be taken.
    pub error: Option<String>,
    /// `None`: shot by a command, nothing measured.
    pub dom: Option<Dom>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Dom {
    pub scroll_width: f64,
    pub client_width: f64,
    /// What reaches past the window's right edge, outside anything that scrolls.
    pub wide: Vec<Placed>,
    /// Text that starts left of the window, where nothing can scroll to it.
    pub offscreen: Vec<Placed>,
    pub clipped: Vec<Clip>,
    pub overlaps: Vec<Overlap>,
    pub axe: Option<Axe>,
    /// The taste checklist's countable part; never a blocker.
    pub taste: Option<Taste>,
}

/// What `precheck.mjs` counts of the taste checklist (the `design-references`
/// skill): what the designer's judgment weighs, said as polish.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Taste {
    /// Framed boxes (a border on three sides or more, or a shadow) inside
    /// framed boxes: a card in a card, or three levels or more.
    pub frames: Vec<Framed>,
    /// Blocks (framed boxes with none inside) setting more than three type sizes.
    pub blocks: Vec<TypeBlock>,
    /// A label or heading said two or three times on one screen.
    pub repeated: Vec<Repeated>,
    /// Buttons that look primary in the first screenful, when there are two or more.
    pub primaries: Vec<Placed>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Framed {
    pub selector: String,
    /// How many framed boxes deep, itself counted.
    pub depth: u32,
    /// Two levels, both with rounded corners: a card in a card.
    pub card_in_card: bool,
    pub rect: Rect,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TypeBlock {
    pub selector: String,
    /// Its distinct type sizes, in px.
    pub sizes: Vec<u32>,
    pub rect: Rect,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Repeated {
    pub text: String,
    pub count: u32,
    /// The second time it is said.
    pub selector: String,
    pub rect: Rect,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Placed {
    pub selector: String,
    pub text: String,
    pub rect: Rect,
}

/// Text its box cuts off.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clip {
    pub selector: String,
    pub text: String,
    /// `x` or `y`.
    pub axis: String,
    /// Cut with an ellipsis or a line clamp, on purpose.
    pub ellipsis: bool,
    /// In a link, a button, a tab, a label…
    pub interactive: bool,
    /// A heading, a table header, a legend.
    pub heading: bool,
    /// It holds a number.
    pub numeric: bool,
    /// Its whole text is there to read anyway: a `title` or `aria-label`.
    pub labelled: bool,
    /// Hidden from assistive technology: decoration.
    pub decorative: bool,
    pub rect: Rect,
}

/// Two things drawn over each other.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Overlap {
    pub a: String,
    pub b: String,
    pub text_a: String,
    pub text_b: String,
    /// Both hold text of their own.
    pub both_text: bool,
    /// How much of the smaller one is covered.
    pub share: f64,
    pub rect: Rect,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Axe {
    pub ran: bool,
    pub why: Option<String>,
    pub violations: Vec<AxeViolation>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AxeViolation {
    pub id: String,
    /// `minor`, `moderate`, `serious`, `critical`.
    pub impact: String,
    pub tags: Vec<String>,
    pub help: String,
    pub nodes: Vec<AxeNode>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AxeNode {
    pub target: String,
    pub rect: Option<Rect>,
}

// ── the report ──

/// How bad a finding is, as design QA's policy reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// S1/S2: content or function lost, layout broken at a major width, a
    /// WCAG 2.2 A/AA failure. Sends the work back.
    Blocker,
    /// S3/S4: polish. Never sends it back.
    Polish,
}

/// What a finding costs the person using the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// Something they need is lost: a label, a heading, a number, a control.
    Content,
    /// It looks off, nothing is lost.
    Cosmetic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// `text-clipped`, `text-overlap`, `overlap`, `reflow`, `offscreen`, `axe/<rule>`.
    pub rule: String,
    pub severity: Severity,
    pub class: Class,
    /// `flow/screen/state`.
    pub key: String,
    /// Where it shows: [`Condition::code`]s.
    pub conditions: Vec<String>,
    pub selector: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    pub evidence: String,
    /// On a screen this change changed (or one with nothing to compare
    /// with): the change's. `false`: it was there before.
    pub new: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<String>,
    /// Where it is, and in which picture: what its crop is cut from.
    #[serde(skip)]
    at: Option<(Rect, String, f64)>,
}

impl Finding {
    /// A finding with no place in a picture: for tests elsewhere in the crate.
    #[cfg(test)]
    pub(crate) fn bare(rule: &str, severity: Severity, key: &str, condition: &str, new: bool) -> Finding {
        let class = if severity == Severity::Blocker { Class::Content } else { Class::Cosmetic };
        Finding { rule: rule.into(), severity, class, key: key.into(), conditions: vec![condition.into()], selector: "x".into(), text: String::new(), evidence: String::new(), new, crop: None, at: None }
    }
}

/// A picture compared with the map's at the base.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Diff {
    /// The share of pixels that differ.
    pub share: f64,
    pub changed: bool,
    /// The two aren't the same size: the layout moved.
    pub resized: bool,
    /// Around what differs, in the picture's pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Rect>,
    /// The base beside the new, cut around what differs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    pub key: String,
    pub condition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// The map's picture of it at the base.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<Diff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The map's picture on the branch isn't what the branch draws.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub map_outdated: bool,
}

impl Shot {
    /// Whether this picture says its screen changed: it differs from the
    /// base's, or there is no base picture to tell.
    pub fn changed(&self) -> bool {
        self.error.is_none() && self.diff.as_ref().is_none_or(|d| d.changed)
    }
}

/// Whether axe-core ran.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxeRun {
    /// Pages it ran on.
    pub ran: usize,
    /// Why it didn't, where it didn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

/// Everything a precheck found, as kept beside the ticket.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Report {
    pub schema: u32,
    /// The ticket checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub changed_files: usize,
    /// `flow/screen`, each the change touches.
    pub touched: Vec<String>,
    /// Changed UI files (components, pages, styles, markup) no screen lists.
    pub unmapped: Vec<String>,
    /// Changed code or config no screen lists that users never see, so it
    /// is not held against the change (`.ts` helpers, type files, tests…).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unmapped_quiet: Vec<String>,
    /// How many unmapped UI files make the change large (0: [`UNMAPPED_LARGE`]).
    #[serde(skip_serializing_if = "is_zero")]
    pub unmapped_large: usize,
    /// States the change touches that weren't rendered: past the cap or
    /// the time budget. The report says nothing of them.
    pub left_out: usize,
    /// Which (`flow/screen/state`), most relevant first, up to a limit.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub left_out_states: Vec<String>,
    pub conditions: Vec<String>,
    pub shots: Vec<Shot>,
    pub findings: Vec<Finding>,
    pub axe: AxeRun,
    /// What stopped the whole run, or part of it: no map, no preview.
    pub errors: Vec<String>,
    /// With no UI map: which app's own pages were rendered instead, how they
    /// were served, and what they were compared with (or why they weren't).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    pub secs: u64,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Report {
    /// How many unmapped UI files make the change large.
    pub fn unmapped_large_at(&self) -> usize {
        if self.unmapped_large == 0 { UNMAPPED_LARGE } else { self.unmapped_large }
    }

    /// The states (`flow/screen/state`) this change changed: any picture at
    /// normal size differs from the base's, or has none to compare with.
    pub fn changed(&self) -> Vec<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        for s in self.shots.iter().filter(|s| is_normal(&s.condition) && s.changed()) {
            out.insert(s.key.clone());
        }
        out.into_iter().collect()
    }

    /// Why pictures failed, the most common cause first, each with how many
    /// it took: the error's lines joined and cut short, so a squad can fix its setup.
    pub fn failure_causes(&self, max: usize) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for e in self.shots.iter().filter_map(|s| s.error.as_deref()) {
            let flat = e.split(['\n', '|']).map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
            *counts.entry(clip(&flat, 200)).or_default() += 1;
        }
        let mut out: Vec<(String, usize)> = counts.into_iter().collect();
        out.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        out.truncate(max);
        out
    }

    /// The screens (`flow/screen`) with a changed state.
    pub fn changed_screens(&self) -> Vec<String> {
        let screens: BTreeSet<String> = self.changed().iter().filter_map(|k| k.rsplit_once('/').map(|(screen, _)| screen.to_owned())).collect();
        screens.into_iter().collect()
    }

    /// Pictures that couldn't be taken.
    pub fn failed(&self) -> usize {
        self.shots.iter().filter(|s| s.error.is_some()).count()
    }

    /// Pictures that were taken.
    pub fn taken(&self) -> usize {
        self.shots.len() - self.failed()
    }

    /// The states (`flow/screen/state`) this change adds to what users see:
    /// rendered at normal size, and the map had no picture of them at the
    /// base in any condition — a new screen, flow or state. Read from the
    /// map's committed pictures, not from words.
    pub fn new_states(&self) -> Vec<String> {
        let mut seen: BTreeMap<&str, bool> = BTreeMap::new();
        for s in self.shots.iter().filter(|s| is_normal(&s.condition) && s.error.is_none()) {
            let new = s.baseline.is_none() && s.diff.is_none();
            seen.entry(s.key.as_str()).and_modify(|all| *all &= new).or_insert(new);
        }
        seen.into_iter().filter(|(_, new)| *new).map(|(k, _)| k.to_owned()).collect()
    }

    /// The findings that send work back: blockers this change brought.
    pub fn new_blockers(&self) -> Vec<&Finding> {
        self.findings.iter().filter(|f| f.severity == Severity::Blocker && f.new).collect()
    }

    /// The pictures to look at, first what matters most: crops of new
    /// blockers, then of the changes, then the changed pictures themselves.
    pub fn flagged(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut push = |p: &str| {
            if out.len() < MAX_FLAGGED && !out.iter().any(|o| o == p) {
                out.push(p.to_owned());
            }
        };
        for f in self.findings.iter().filter(|f| f.new) {
            if let Some(c) = &f.crop {
                push(c);
            }
        }
        for s in self.shots.iter().filter(|s| s.changed()) {
            if let Some(c) = s.diff.as_ref().and_then(|d| d.crop.as_deref()) {
                push(c);
            }
        }
        for s in self.shots.iter().filter(|s| is_normal(&s.condition) && s.changed()) {
            if let Some(p) = &s.picture {
                push(p);
            }
        }
        out
    }

    /// The report in short, in English Markdown, under `headline` (what the
    /// gate made of it): for the designer's brief, the ticket's record and the person.
    pub fn summary(&self, headline: &str) -> String {
        let mut out = vec![format!("# Design QA precheck\n\n{headline}")];
        let changed = self.changed();
        let mut facts = vec![format!("- Touched screens: {}", if self.touched.is_empty() { "none on the UI map".to_owned() } else { self.touched.join(", ") })];
        if let Some(fallback) = &self.fallback {
            facts.push(format!("- Fallback: {fallback}"));
        }
        facts.push(format!("- Changed states (pixels against the base): {}", if changed.is_empty() { "none".to_owned() } else { changed.join(", ") }));
        let taken = self.shots.len() - self.failed();
        facts.push(format!("- Pictures: {taken} taken across {} conditions, {} failed", self.conditions.len(), self.failed()));
        let causes = self.failure_causes(3);
        if !causes.is_empty() {
            facts.push(format!("- Why pictures failed: {}", causes.iter().map(|(why, n)| format!("{why} (x{n})")).collect::<Vec<_>>().join("; ")));
        }
        if !self.unmapped.is_empty() {
            facts.push(format!("- Changed UI files no screen lists: {}", clip_list(&self.unmapped, 8)));
        }
        if !self.unmapped_quiet.is_empty() {
            facts.push(format!("- Changed code no screen lists and users don't see (not held against the change): {}", clip_list(&self.unmapped_quiet, 4)));
        }
        if self.left_out > 0 {
            let which = if self.left_out_states.is_empty() { String::new() } else { format!(": {}", clip_list(&self.left_out_states, 12)) };
            facts.push(format!("- States left out (over the cap or the time budget; this report says nothing of them, the designer checks them): {}{which}", self.left_out));
        }
        match &self.axe.skipped {
            Some(why) => facts.push(format!("- axe-core: not run ({why})")),
            None if self.axe.ran > 0 => facts.push(format!("- axe-core: ran on {} pages", self.axe.ran)),
            None => {}
        }
        let outdated: BTreeSet<&str> = self.shots.iter().filter(|s| s.map_outdated).map(|s| s.key.as_str()).collect();
        if !outdated.is_empty() {
            facts.push(format!("- The map's pictures on the branch aren't what it draws: {}", outdated.into_iter().collect::<Vec<_>>().join(", ")));
        }
        out.push(facts.join("\n"));
        if !self.errors.is_empty() {
            out.push(format!("## Problems\n{}", self.errors.iter().map(|e| format!("- {e}")).collect::<Vec<_>>().join("\n")));
        }
        let line = |f: &Finding| {
            let crop = f.crop.as_deref().map(|c| format!(" — {c}")).unwrap_or_default();
            let text = if f.text.is_empty() { String::new() } else { format!(" “{}”", clip(&f.text, 40)) };
            format!("- **{}** {} `{}`{text}: {} [{}]{crop}", f.rule, f.key, clip(&f.selector, 60), f.evidence, clip_list(&f.conditions, 4))
        };
        for (head, severity, new) in [("Blockers (S1/S2) this change brought", Severity::Blocker, true), ("Polish (S3/S4) on changed screens", Severity::Polish, true), ("Not this change's: there before, or page-wide", Severity::Blocker, false)] {
            let these: Vec<String> = self.findings.iter().filter(|f| f.new == new && (f.severity == severity || !new)).take(12).map(line).collect();
            if !these.is_empty() {
                out.push(format!("## {head}\n{}", these.join("\n")));
            }
        }
        let flagged = self.flagged();
        if !flagged.is_empty() {
            out.push(format!("## Flagged pictures\n{}", flagged.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n")));
        }
        out.join("\n\n") + "\n"
    }
}

fn is_normal(condition: &str) -> bool {
    condition.ends_with(".normal")
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max { text.to_owned() } else { format!("{}…", text.chars().take(max).collect::<String>()) }
}

/// The first lines of what a log says, to put beside an error so a squad
/// can fix its setup: ` — line | line`, or nothing when the log is empty.
pub fn log_head(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).take(4).collect();
    if lines.is_empty() {
        return String::new();
    }
    format!(" — {}", clip(&lines.join(" | "), 400))
}

fn clip_list(items: &[String], max: usize) -> String {
    let mut shown = items.iter().take(max).cloned().collect::<Vec<_>>().join(", ");
    if items.len() > max {
        shown.push_str(&format!(" and {} more", items.len() - max));
    }
    shown
}

// ── comparing pictures ──

/// `head` against `base`, pixel by pixel where they overlap; what the other
/// has beyond counts as differing. With `crop`, the two side by side cut
/// around what differs are written there.
pub fn pixel_diff(base: &Path, head: &Path, crop: Option<&Path>) -> Result<Diff, String> {
    let open = |p: &Path| image::open(p).map(|i| i.to_rgba8()).map_err(|e| format!("{}: {e}", p.display()));
    let (a, b) = (open(base)?, open(head)?);
    let (w, h) = (a.width().min(b.width()), a.height().min(b.height()));
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut differ: u64 = 0;
    for y in 0..h {
        for x in 0..w {
            let (p, q) = (a.get_pixel(x, y).0, b.get_pixel(x, y).0);
            if p.iter().zip(q.iter()).any(|(c, d)| c.abs_diff(*d) > PIXEL_TOLERANCE) {
                differ += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    let resized = a.dimensions() != b.dimensions();
    let whole = u64::from(a.width().max(b.width())) * u64::from(a.height().max(b.height()));
    let beyond = whole - u64::from(w) * u64::from(h);
    let share = if whole == 0 { 0. } else { (differ + beyond) as f64 / whole as f64 };
    let mut bbox = (differ > 0).then(|| Rect { x: f64::from(x0), y: f64::from(y0), w: f64::from(x1 - x0 + 1), h: f64::from(y1 - y0 + 1) });
    if resized && bbox.is_none() {
        // Only the length changed: what differs starts where the shorter ends.
        bbox = Some(Rect { x: 0., y: f64::from(h.saturating_sub(1)), w: f64::from(w), h: 1. });
    }
    let changed = resized || share > CHANGED_SHARE;
    let mut diff = Diff { share, changed, resized, bbox, crop: None };
    if let (true, Some(out), Some(bbox)) = (changed, crop, bbox) {
        diff.crop = side_by_side(&a, &b, &bbox, out).ok().map(|()| out.to_string_lossy().into_owned());
    }
    Ok(diff)
}

/// Pixels of `bbox` padded and kept to `img`, at most `max_h` tall.
fn region(img: &image::RgbaImage, bbox: &Rect, pad: f64, max_h: u32) -> (u32, u32, u32, u32) {
    let x = (bbox.x - pad).max(0.) as u32;
    let y = (bbox.y - pad).max(0.) as u32;
    let x = x.min(img.width().saturating_sub(1));
    let y = y.min(img.height().saturating_sub(1));
    let w = ((bbox.w + 2. * pad) as u32).clamp(1, img.width() - x);
    let h = ((bbox.h + 2. * pad) as u32).clamp(1, img.height() - y).min(max_h);
    (x, y, w, h)
}

const CROP_MAX_H: u32 = 1600;

fn side_by_side(a: &image::RgbaImage, b: &image::RgbaImage, bbox: &Rect, out: &Path) -> Result<(), String> {
    let pad = CROP_PAD * 2.;
    let (ax, ay, aw, ah) = region(a, bbox, pad, CROP_MAX_H);
    let (bx, by, bw, bh) = region(b, bbox, pad, CROP_MAX_H);
    let gap = 12;
    let mut sheet = image::RgbaImage::from_pixel(aw + gap + bw, ah.max(bh), image::Rgba([255, 0, 255, 255]));
    let left = image::imageops::crop_imm(a, ax, ay, aw, ah).to_image();
    let right = image::imageops::crop_imm(b, bx, by, bw, bh).to_image();
    image::imageops::overlay(&mut sheet, &left, 0, 0);
    image::imageops::overlay(&mut sheet, &right, i64::from(aw + gap), 0);
    save(&sheet, out)
}

fn save(img: &image::RgbaImage, out: &Path) -> Result<(), String> {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    img.save(out).map_err(|e| format!("{}: {e}", out.display()))
}

/// The part of the picture at `picture` around `rect` (CSS px, `dpr` device
/// px each), written to `out`.
fn crop_at(picture: &Path, rect: &Rect, dpr: f64, out: &Path) -> Result<(), String> {
    let img = image::open(picture).map(|i| i.to_rgba8()).map_err(|e| format!("{}: {e}", picture.display()))?;
    let dpr = if dpr > 0. { dpr } else { 1. };
    let r = rect.scaled(dpr);
    if r.x >= f64::from(img.width()) || r.y >= f64::from(img.height()) {
        return Err("outside the picture".into());
    }
    let (x, y, w, h) = region(&img, &r, CROP_PAD * dpr, CROP_MAX_H);
    save(&image::imageops::crop_imm(&img, x, y, w, h).to_image(), out)
}

// ── judging ──

/// What one rendered page shows, before it is known whether it is new.
fn judge(t: &Taken, condition: &Condition) -> Vec<Finding> {
    let Some(dom) = &t.dom else { return Vec::new() };
    let at = |rect: Rect| t.path.clone().map(|p| (rect, p, t.dpr));
    let scope = condition.in_wcag_scope();
    let mut out = Vec::new();
    let mut push = |rule: &str, blocker: bool, class: Class, selector: &str, text: &str, evidence: String, rect: Option<Rect>| {
        out.push(Finding {
            rule: rule.to_owned(),
            severity: if blocker { Severity::Blocker } else { Severity::Polish },
            class,
            key: t.key.clone(),
            conditions: vec![condition.code()],
            selector: selector.to_owned(),
            text: text.to_owned(),
            evidence,
            new: false,
            crop: None,
            at: rect.and_then(at),
        });
    };
    let css = condition.variant.css_width(condition.width);
    if dom.scroll_width > dom.client_width + 1. {
        let first = dom.wide.first();
        let what = first.map(|w| format!(": {} reaches {:.0}px", w.selector, w.rect.x + w.rect.w)).unwrap_or_default();
        let evidence = format!("the page scrolls sideways — {:.0}px wide in a {css}px viewport{what}", dom.scroll_width);
        push("reflow", scope, Class::Content, first.map_or("html", |w| w.selector.as_str()), "", evidence, first.map(|w| w.rect));
    }
    for o in &dom.offscreen {
        push("offscreen", scope, Class::Content, &o.selector, &o.text, "text starts left of the window, where nothing scrolls to it".into(), Some(o.rect));
    }
    for c in &dom.clipped {
        let content = !c.labelled && !c.decorative && (c.interactive || c.heading || c.numeric || !c.ellipsis);
        let what = match (c.ellipsis, c.axis.as_str()) {
            (true, _) => "cut short with an ellipsis",
            (false, "y") => "cut off at the bottom by its box",
            _ => "cut off at the side by its box",
        };
        let role = if c.interactive {
            " (a control's label)"
        } else if c.heading {
            " (a heading)"
        } else if c.numeric {
            " (a number)"
        } else {
            ""
        };
        let class = if content { Class::Content } else { Class::Cosmetic };
        push("text-clipped", content && scope, class, &c.selector, &c.text, format!("{what}{role}"), Some(c.rect));
    }
    for o in &dom.overlaps {
        let text = o.both_text && o.share >= OVERLAP_SHARE;
        let evidence = format!("{} is drawn over {} ({:.0}% of the smaller covered)", o.a, o.b, o.share * 100.);
        let shown = if o.text_b.is_empty() { o.text_a.clone() } else { format!("{} / {}", o.text_a, o.text_b) };
        push(if text { "text-overlap" } else { "overlap" }, text && scope, if text { Class::Content } else { Class::Cosmetic }, &o.a, &shown, evidence, Some(o.rect));
    }
    if let Some(axe) = dom.axe.as_ref().filter(|a| a.ran) {
        for v in &axe.violations {
            let wcag = v.tags.iter().any(|t| matches!(t.as_str(), "wcag2a" | "wcag2aa" | "wcag21a" | "wcag21aa" | "wcag22a" | "wcag22aa"));
            let serious = matches!(v.impact.as_str(), "serious" | "critical");
            let node = v.nodes.first();
            let more = if v.nodes.len() > 1 { format!(" ({} elements)", v.nodes.len()) } else { String::new() };
            push(&format!("axe/{}", v.id), wcag && serious, if wcag && serious { Class::Content } else { Class::Cosmetic }, node.map_or("", |n| n.target.as_str()), "", format!("{} — {}{more}", v.help, v.impact), node.and_then(|n| n.rect));
        }
    }
    // The taste checklist's countable part: polish, read at normal size only.
    if let Some(taste) = dom.taste.as_ref().filter(|_| condition.variant == Variant::Normal) {
        for f in &taste.frames {
            let (rule, evidence) = if f.card_in_card && f.depth == 2 { ("nested-cards", "a card inside a card".to_owned()) } else { ("border-levels", format!("{} framed boxes deep (two at most)", f.depth)) };
            push(rule, false, Class::Cosmetic, &f.selector, "", evidence, Some(f.rect));
        }
        for b in &taste.blocks {
            let sizes = b.sizes.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
            push("type-sizes", false, Class::Cosmetic, &b.selector, "", format!("{} type sizes in one block ({sizes}px; two or three is enough)", b.sizes.len()), Some(b.rect));
        }
        for r in &taste.repeated {
            push("duplicate-text", false, Class::Cosmetic, &r.selector, &r.text, format!("said {} times on one screen", r.count), Some(r.rect));
        }
        if let Some(first) = taste.primaries.first().filter(|_| taste.primaries.len() >= 2) {
            let all = taste.primaries.iter().map(|p| format!("“{}”", clip(&p.text, 20))).collect::<Vec<_>>().join(", ");
            push("primary-actions", false, Class::Cosmetic, &first.selector, &first.text, format!("{} buttons look primary in the first screenful: {all}", taste.primaries.len()), Some(first.rect));
        }
    }
    out
}

/// What [`assemble`] is given: what the change touched and every picture
/// taken, with the map's pictures to compare them with.
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub ticket: Option<u64>,
    pub head: Option<String>,
    pub base: Option<String>,
    pub changed_files: usize,
    pub touched: Touched,
    pub left_out: usize,
    pub left_out_states: Vec<String>,
    pub unmapped_large: usize,
    pub conditions: Vec<Condition>,
    pub taken: Vec<Taken>,
    /// The map's picture at the base, by `(key, condition)` (normal size).
    pub baselines: BTreeMap<(String, String), PathBuf>,
    /// The map's picture on the branch, by `(key, condition)`.
    pub committed: BTreeMap<(String, String), PathBuf>,
    pub axe_skipped: Option<String>,
    pub errors: Vec<String>,
    /// With no UI map, what was rendered instead ([`Report::fallback`]).
    pub fallback: Option<String>,
    /// Nothing to compare the pictures with — no map's pictures and no base
    /// rendered: whether a finding predates the change can't be told, so none
    /// is held against it as new (a designer reads them all).
    pub no_base: bool,
    pub secs: u64,
}

/// The report from `input`; diff and finding crops go under `out/crops/`.
pub fn assemble(input: Input, out: &Path) -> Report {
    let crops = out.join("crops");
    let mut shots = Vec::new();
    let mut found: Vec<Finding> = Vec::new();
    let mut axe_ran = 0;
    for (n, t) in input.taken.iter().enumerate() {
        let Some(condition) = Condition::parse(&t.condition) else { continue };
        let at = (t.key.clone(), t.condition.clone());
        let baseline = input.baselines.get(&at);
        let mut shot = Shot { key: t.key.clone(), condition: t.condition.clone(), picture: t.path.clone(), baseline: baseline.map(|b| b.to_string_lossy().into_owned()), diff: None, error: t.error.clone(), map_outdated: false };
        if let (None, Some(picture)) = (&t.error, &t.path) {
            if let Some(base) = baseline {
                match pixel_diff(base, Path::new(picture), Some(&crops.join(format!("change-{n}.png")))) {
                    Ok(d) => shot.diff = Some(d),
                    Err(why) => shot.error = Some(format!("couldn't compare with the base: {why}")),
                }
            }
            if let Some(committed) = input.committed.get(&at) {
                shot.map_outdated = pixel_diff(committed, Path::new(picture), None).is_ok_and(|d| d.changed);
            }
        }
        if t.dom.as_ref().and_then(|d| d.axe.as_ref()).is_some_and(|a| a.ran) {
            axe_ran += 1;
        }
        found.extend(judge(t, &condition));
        shots.push(shot);
    }
    // New: on a changed picture — at normal size, where what changed is
    // known, only if it is in the part that changed.
    for f in &mut found {
        if input.no_base {
            f.evidence.push_str(" (no base to compare with: it may predate this change)");
            f.new = false;
            continue;
        }
        let Some(c) = f.conditions.first().and_then(|c| Condition::parse(c)) else { continue };
        let normal = c.normal().code();
        f.new = match shots.iter().find(|s| s.key == f.key && s.condition == normal) {
            // No picture at normal size here (a command's own mode): new if
            // the state changed at any, or has none at all.
            None => {
                let mut normals = shots.iter().filter(|s| s.key == f.key && is_normal(&s.condition)).peekable();
                normals.peek().is_none() || normals.any(Shot::changed)
            }
            // It couldn't be taken: whether it changed is unknown.
            Some(s) if s.error.is_some() => true,
            Some(s) if !s.changed() => false,
            // The whole page (`html`, `body`): where this change is in it can't
            // be told, so it never sends work back on its own.
            Some(_) if page_wide(&f.selector) => {
                f.evidence.push_str(" (page-wide: it may predate this change)");
                false
            }
            Some(s) if c.variant == Variant::Normal => match (s.diff.as_ref().and_then(|d| d.bbox), &f.at) {
                (Some(bbox), Some((rect, _, dpr))) => rect.scaled(if *dpr > 0. { *dpr } else { 1. }).intersects(&bbox),
                _ => true,
            },
            Some(_) => true,
        };
    }
    let mut findings = collapse(found);
    findings.sort_by_key(|f| (!f.new, f.severity, f.class, f.key.clone()));
    let mut cut = 0;
    for (n, f) in findings.iter_mut().enumerate() {
        if cut >= MAX_CROPS || !(f.new || f.severity == Severity::Blocker) {
            continue;
        }
        if let Some((rect, picture, dpr)) = &f.at {
            let path = crops.join(format!("finding-{n}.png"));
            if crop_at(Path::new(picture), rect, *dpr, &path).is_ok() {
                f.crop = Some(path.to_string_lossy().into_owned());
                cut += 1;
            }
        }
    }
    Report {
        schema: SCHEMA,
        ticket: input.ticket,
        head: input.head,
        base: input.base,
        changed_files: input.changed_files,
        touched: input.touched.screens.iter().map(ScreenRef::key).collect(),
        unmapped: input.touched.unmapped,
        unmapped_quiet: input.touched.quiet,
        unmapped_large: input.unmapped_large,
        left_out: input.left_out,
        left_out_states: input.left_out_states.into_iter().take(MAX_LEFT_OUT_LISTED).collect(),
        conditions: input.conditions.iter().map(Condition::code).collect(),
        shots,
        findings,
        axe: AxeRun { ran: axe_ran, skipped: input.axe_skipped },
        errors: input.errors,
        fallback: input.fallback,
        secs: input.secs,
    }
}

/// Whether `selector` names the whole page rather than a part of it.
fn page_wide(selector: &str) -> bool {
    let first = selector.split([' ', '>', '.', '#', '[', ':']).next().unwrap_or_default();
    matches!(first, "html" | "body")
}

/// The same finding at several conditions, said once with them all; the
/// first one's place kept for its crop. New anywhere is new.
fn collapse(found: Vec<Finding>) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    for f in found {
        match out.iter_mut().find(|o| o.rule == f.rule && o.key == f.key && o.selector == f.selector && o.text == f.text && o.severity == f.severity) {
            Some(o) => {
                o.conditions.extend(f.conditions);
                o.new |= f.new;
                if o.at.is_none() {
                    o.at = f.at;
                }
            }
            None => out.push(f),
        }
    }
    out
}

/// Remove the folders under `dir` (a squad's precheck reports) untouched
/// for more than `days` days.
pub fn prune(dir: &Path, days: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let limit = std::time::Duration::from_secs(days * 86_400);
    for entry in entries.flatten() {
        let old = entry.metadata().ok().filter(std::fs::Metadata::is_dir).and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age > limit);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// A report kept on disk, read back.
pub fn read(dir: &Path) -> Option<Report> {
    let text = std::fs::read_to_string(dir.join(REPORT_JSON)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Keep `report` and its `summary` in `dir`.
pub fn write(dir: &Path, report: &Report, summary: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(REPORT_JSON), serde_json::to_string_pretty(report).unwrap_or_default())?;
    std::fs::write(dir.join(REPORT_MD), summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-precheck-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A flat picture `w`×`h`, with a block of another colour at `block`.
    fn png(path: &Path, w: u32, h: u32, block: Option<(u32, u32, u32, u32)>) -> PathBuf {
        let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([250, 250, 250, 255]));
        if let Some((x, y, bw, bh)) = block {
            for yy in y..(y + bh).min(h) {
                for xx in x..(x + bw).min(w) {
                    img.put_pixel(xx, yy, image::Rgba([20, 20, 120, 255]));
                }
            }
        }
        img.save(path).unwrap();
        path.to_path_buf()
    }

    #[test]
    fn the_default_conditions_are_three_widths_light_and_dark_at_three_sizes() {
        let all = conditions(&WIDTHS, &SCHEMES);
        assert_eq!(all.len(), 18);
        assert!(all.iter().all(|c| Condition::parse(&c.code()).as_ref() == Some(c)));
        assert_eq!(Condition::parse("390.light-large.normal").unwrap().scheme, "light-large");
        // Zoomed 200% at 390, the viewport is 195 CSS px: under reflow's 320.
        assert!(!Condition::new(390, "light", Variant::Zoom200).in_wcag_scope());
        assert!(Condition::new(768, "dark", Variant::Zoom200).in_wcag_scope());
        assert_eq!(Condition::new(768, "dark", Variant::Text200).normal().code(), "768.dark.normal");
    }

    #[test]
    fn a_change_touches_the_screens_listing_its_files_and_the_rest_is_unmapped() {
        let map = json!({ "flows": [
            { "id": "auth", "screens": [
                { "id": "login", "source": ["src/app/login/page.tsx", "src/styles/forms/"] },
                { "id": "reset", "source": ["src/app/reset/*.tsx"] } ] },
            { "id": "board", "screens": [ { "id": "list", "source": ["src/board/"] } ] } ] });
        let changed: Vec<String> = ["src/styles/forms/input.css", "src/app/reset/page.tsx", "src/lib/api.ts", "src/components/Nav.tsx", "src/app/login/page.test.tsx", "design/flows/board/list/default.light.390.png", "README.md"]
            .map(str::to_owned)
            .to_vec();
        let t = touched(&map, "design/flows", &changed);
        assert_eq!(t.screens.iter().map(ScreenRef::key).collect::<Vec<_>>(), ["auth/login", "auth/reset", "board/list"]);
        assert_eq!(t.unmapped, ["src/components/Nav.tsx"], "API code, tests and docs change nothing users see");
        assert!(source_matches("src/board", "src/board/x.tsx") && !source_matches("src/board", "src/boards.tsx"));
        assert!(ui_file("locales/de.json") && !ui_file("package.json") && !ui_file("src/server/db.ts"));
    }

    #[test]
    fn a_path_says_whether_users_see_it_in_any_language_or_framework() {
        use Visual::{Maybe, No, Yes};
        for (path, want) in [
            ("src/styles/app.css", Yes),
            ("src/lib/Card.svelte", Yes),
            ("templates/index.html.erb", Yes),
            ("app/views/users/show.html.slim", Yes),
            ("locales/de.json", Yes),
            ("tokens.json", Yes),
            ("src/components/Nav.tsx", Maybe),
            ("src/lib/icons.tsx", Maybe),
            ("src/components/format.ts", Maybe),
            ("src/lib/format.ts", No),
            ("src/utils/dates.js", No),
            ("app/lib/session.ts", No),
            ("lib/components/format.ts", Maybe),
            ("src/types/user.ts", No),
            ("src/components/Button.d.ts", No),
            ("app/api/users/route.ts", No),
            ("app/api/users/route.tsx", No),
            ("server/index.ts", No),
            ("tests/nav.test.tsx", No),
            ("src/components/Nav.stories.tsx", No),
            ("docs/DESIGN.md", No),
            ("README.md", No),
            ("package.json", No),
            ("next.config.js", No),
            ("tailwind.config.ts", Yes),
            ("main.go", No),
            ("app/models/user.rb", No),
            ("config/locales/en.yml", Yes),
            ("db/migrate/001_init.sql", No),
        ] {
            assert_eq!(visual_path(path), want, "{path}");
        }
    }

    #[test]
    fn a_script_that_draws_nothing_is_not_ui_by_what_is_in_it() {
        let jsx = "export function Nav() {\n  return <nav><a href=\"/\">Home</a></nav>;\n}\n";
        let helper = "import { x } from './x';\nexport function nav() {\n  return [1, 2].map((n) => n * 2);\n}\n";
        assert!(shows_ui("src/components/Nav.tsx", jsx) && !shows_ui("src/components/nav.tsx", helper));
        assert!(shows_ui("src/components/Frag.jsx", "export default () => <>hi</>;"));
        let types = "// Props of the card.\n/* a block\n comment */\nimport type { Tone } from './tone';\nexport interface CardProps {\n  tone: Tone;\n  onClick: () => void;\n}\nexport type Size = 'sm' | 'lg';\nexport type Long =\n  | 'a'\n  | 'b';\n";
        assert!(!shows_ui("src/components/types.ts", types), "type declarations draw nothing");
        assert!(shows_ui("src/components/theme.ts", "export const theme = { radius: 8 };\n"), "a value a component reads may change what is drawn");
        assert!(!type_only("") && !type_only("// nothing but a comment\n"));
        let read = |f: &str| match f {
            "src/components/Nav.tsx" => Some(jsx.to_owned()),
            "src/components/useNav.tsx" => Some(helper.to_owned()),
            "src/components/types.ts" => Some(types.to_owned()),
            _ => None,
        };
        let all: Vec<String> = ["src/components/Nav.tsx", "src/components/useNav.tsx", "src/components/types.ts", "src/styles/a.css", "src/components/Gone.tsx"].map(str::to_owned).to_vec();
        let (visual, quiet) = split_unmapped(all, read);
        assert_eq!(visual, ["src/components/Nav.tsx", "src/styles/a.css", "src/components/Gone.tsx"], "styles are visual and a file that can't be read is held against the change");
        assert_eq!(quiet, ["src/components/useNav.tsx", "src/components/types.ts"]);
    }

    #[test]
    fn states_are_taken_by_relevance_and_what_is_cut_is_named() {
        // (screen, state): `a/own` has a changed source file, `b/shared` only a picture that changed.
        let mut states: Vec<(&str, &str)> = vec![("b/shared", "default"), ("b/shared", "error"), ("a/own", "error"), ("a/own", "default"), ("c/many", "default"), ("c/many", "empty")];
        let hits: BTreeMap<String, usize> = [("a/own".to_owned(), 1), ("c/many".to_owned(), 3)].into();
        prioritize(&mut states, &hits, |(screen, state)| ((*screen).to_owned(), *state == "default"));
        assert_eq!(states, [("c/many", "default"), ("a/own", "default"), ("c/many", "empty"), ("a/own", "error"), ("b/shared", "default"), ("b/shared", "error")]);
        let (kept, cut) = take_most_relevant(states, 4);
        assert_eq!((kept.len(), cut), (4, vec![("b/shared", "default"), ("b/shared", "error")]));
        assert_eq!(take_most_relevant(vec![1, 2], 9), (vec![1, 2], vec![]));
        // The change's own source files are counted per screen.
        let map = json!({ "flows": [ { "id": "a", "screens": [ { "id": "own", "source": ["src/a/"] }, { "id": "other", "source": ["src/b/"] } ] } ] });
        let t = touched(&map, "design/flows", &["src/a/x.tsx".to_owned(), "src/a/y.css".to_owned(), "design/flows/a/other/default.light.390.png".to_owned()]);
        assert_eq!(t.source_hits.get("a/own"), Some(&2));
        assert_eq!(t.source_hits.get("a/other"), None);
        assert_eq!(t.screens.len(), 2, "a changed picture still touches its screen");
    }

    #[test]
    fn the_cap_is_high_by_default_and_the_map_can_move_it_and_the_unmapped_threshold() {
        assert_eq!((Config::default().max_states(), Config::default().unmapped_large()), (MAX_STATES, UNMAPPED_LARGE));
        let c = Config::of(&json!({ "precheck": { "maxStates": 5, "unmappedLarge": 3 } }));
        assert_eq!((c.max_states(), c.unmapped_large()), (5, 3));
        assert_eq!(Report::default().unmapped_large_at(), UNMAPPED_LARGE);
        assert_eq!(Report { unmapped_large: 3, ..Report::default() }.unmapped_large_at(), 3);
    }

    #[test]
    fn the_summary_names_what_was_left_out_and_what_was_not_held_against_the_change() {
        let r = Report { left_out: 3, left_out_states: vec!["a/b/default".into(), "a/c/default".into(), "a/c/error".into()], unmapped_quiet: vec!["src/lib/api.ts".into()], ..Report::default() };
        let md = r.summary("x");
        assert!(md.contains("States left out") && md.contains("a/c/error") && md.contains("the designer checks them"), "{md}");
        assert!(md.contains("src/lib/api.ts") && md.contains("not held against"), "{md}");
        let dir = temp("log-head");
        std::fs::write(dir.join("p.log"), "\n  first line\nsecond\n\nthird\nfourth\nfifth\n").unwrap();
        assert_eq!(log_head(&dir.join("p.log")), " — first line | second | third | fourth");
        assert_eq!(log_head(&dir.join("none.log")), "");
    }

    #[test]
    fn a_vite_project_with_its_dependencies_serves_itself_when_the_map_names_no_preview() {
        let root = std::env::temp_dir().join(format!("eq-precheck-vite-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
        std::fs::write(root.join("package.json"), r#"{"devDependencies":{"vite":"^5.4.0"}}"#).unwrap();
        assert_eq!(default_preview(&root), None, "not installed yet: nothing to run");
        std::fs::write(root.join("node_modules/.bin/vite"), "").unwrap();
        assert_eq!(default_preview(&root).as_deref(), Some("node_modules/.bin/vite --port {port} --strictPort --host 127.0.0.1"));
        std::fs::write(root.join("package.json"), r#"{"dependencies":{"next":"15.0.0"}}"#).unwrap();
        assert_eq!(default_preview(&root), None, "only Vite is known");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_maps_urls_are_opened_on_the_preview() {
        let c = Config { origin: Some("http://localhost:3780".into()), ..Config::default() };
        assert_eq!(c.rebase("http://localhost:3780/guide?x=1", "http://127.0.0.1:41234/"), "http://127.0.0.1:41234/guide?x=1");
        assert_eq!(c.rebase("https://example.com/a", "http://127.0.0.1:9"), "https://example.com/a", "another origin is left alone");
        assert_eq!(Config::default().rebase("http://localhost:3000/login", "http://127.0.0.1:9"), "http://127.0.0.1:9/login");
        let read = Config::of(&json!({ "precheck": { "preview": "npm run dev -- --port {port}", "maxStates": 3, "widths": [390] } }));
        assert_eq!((read.max_states(), read.widths()), (3, vec![390]));
        assert_eq!(Config::of(&json!({})).widths(), WIDTHS);
    }

    #[test]
    fn pixels_say_what_changed_and_where() {
        let dir = temp("diff");
        let base = png(&dir.join("base.png"), 200, 100, None);
        let same = png(&dir.join("same.png"), 200, 100, None);
        let moved = png(&dir.join("moved.png"), 200, 100, Some((40, 20, 30, 30)));
        let longer = png(&dir.join("longer.png"), 200, 140, None);
        let d = pixel_diff(&base, &same, None).unwrap();
        assert!(!d.changed && d.share == 0. && d.bbox.is_none());
        let crop = dir.join("crop.png");
        let d = pixel_diff(&base, &moved, Some(&crop)).unwrap();
        assert!(d.changed && !d.resized);
        assert_eq!(d.bbox, Some(Rect { x: 40., y: 20., w: 30., h: 30. }));
        assert!(crop.is_file() && d.crop.is_some(), "the base beside the new, around the change");
        let d = pixel_diff(&base, &longer, None).unwrap();
        assert!(d.changed && d.resized, "a longer page is a changed layout");
        assert!(pixel_diff(&base, &dir.join("missing.png"), None).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn dom(clipped: Vec<Clip>) -> Dom {
        Dom { scroll_width: 390., client_width: 390., clipped, ..Dom::default() }
    }

    fn clip(text: &str, interactive: bool, ellipsis: bool) -> Clip {
        Clip { selector: format!("span “{text}”"), text: text.into(), axis: "x".into(), ellipsis, interactive, rect: Rect { x: 10., y: 10., w: 40., h: 10. }, ..Clip::default() }
    }

    #[test]
    fn clipped_text_is_content_lost_where_people_need_it_and_cosmetic_elsewhere() {
        let c = Condition::new(390, "light", Variant::Text200);
        let t = Taken { key: "a/b/default".into(), condition: c.code(), dom: Some(dom(vec![clip("Save", true, true), clip("A long description of the item", false, true), Clip { numeric: true, ..clip("Total 1,204", false, true) }, Clip { labelled: true, ..clip("Pay", true, true) }])), ..Taken::default() };
        let found = judge(&t, &c);
        let classes: Vec<(&str, Severity, Class)> = found.iter().map(|f| (f.text.as_str(), f.severity, f.class)).collect();
        assert_eq!(
            classes,
            [
                ("Save", Severity::Blocker, Class::Content),
                ("A long description of the item", Severity::Polish, Class::Cosmetic),
                ("Total 1,204", Severity::Blocker, Class::Content),
                ("Pay", Severity::Polish, Class::Cosmetic),
            ],
            "a control's label, a number: lost; a description with an ellipsis, or a label whose whole text is in its title: cosmetic"
        );
        // Zoomed into a 195px viewport, nothing blocks: under WCAG's 320.
        let small = Condition::new(390, "light", Variant::Zoom200);
        let t = Taken { condition: small.code(), dom: Some(Dom { scroll_width: 300., client_width: 195., ..dom(vec![clip("Save", true, true)]) }), ..t };
        assert!(judge(&t, &small).iter().all(|f| f.severity == Severity::Polish));
    }

    #[test]
    fn sideways_scroll_text_over_text_and_serious_axe_rules_block() {
        let c = Condition::new(768, "dark", Variant::Zoom200);
        let axe = Axe {
            ran: true,
            why: None,
            violations: vec![
                AxeViolation { id: "color-contrast".into(), impact: "serious".into(), tags: vec!["wcag2aa".into()], help: "Elements must meet minimum color contrast ratio thresholds".into(), nodes: vec![AxeNode { target: ".muted".into(), rect: None }] },
                AxeViolation { id: "region".into(), impact: "moderate".into(), tags: vec!["best-practice".into()], help: "All page content should be contained by landmarks".into(), nodes: vec![] },
            ],
        };
        let overlaps = vec![
            Overlap { a: "h2".into(), b: "p".into(), text_a: "Title".into(), text_b: "Body".into(), both_text: true, share: 0.6, rect: Rect::default() },
            Overlap { a: "span.badge".into(), b: "svg".into(), both_text: false, share: 0.9, ..Overlap::default() },
        ];
        let t = Taken { key: "a/b/default".into(), condition: c.code(), dom: Some(Dom { scroll_width: 520., client_width: 384., overlaps, axe: Some(axe), ..Dom::default() }), ..Taken::default() };
        let rules: Vec<(String, Severity)> = judge(&t, &c).into_iter().map(|f| (f.rule, f.severity)).collect();
        assert_eq!(
            rules,
            [
                ("reflow".to_owned(), Severity::Blocker),
                ("text-overlap".to_owned(), Severity::Blocker),
                ("overlap".to_owned(), Severity::Polish),
                ("axe/color-contrast".to_owned(), Severity::Blocker),
                ("axe/region".to_owned(), Severity::Polish),
            ]
        );
    }

    #[test]
    fn the_taste_checklists_countable_part_is_polish_at_normal_size_only() {
        let taste = Taste {
            frames: vec![Framed { selector: "div.kpi".into(), depth: 2, card_in_card: true, rect: Rect::default() }, Framed { selector: "div.badge".into(), depth: 3, card_in_card: false, rect: Rect::default() }],
            blocks: vec![TypeBlock { selector: "div.kpi".into(), sizes: vec![11, 12, 13, 28], rect: Rect::default() }],
            repeated: vec![Repeated { text: "Net asset value".into(), count: 2, selector: "h3".into(), rect: Rect::default() }],
            primaries: vec![Placed { selector: "button".into(), text: "Export".into(), rect: Rect::default() }, Placed { selector: "button".into(), text: "Add fund".into(), rect: Rect::default() }],
        };
        let normal = Condition::new(1280, "light", Variant::Normal);
        let t = Taken { key: "funds/overview/default".into(), condition: normal.code(), dom: Some(Dom { scroll_width: 1280., client_width: 1280., taste: Some(taste), ..Dom::default() }), ..Taken::default() };
        let found = judge(&t, &normal);
        let rules: Vec<&str> = found.iter().map(|f| f.rule.as_str()).collect();
        assert_eq!(rules, ["nested-cards", "border-levels", "type-sizes", "duplicate-text", "primary-actions"]);
        assert!(found.iter().all(|f| f.severity == Severity::Polish && f.class == Class::Cosmetic), "never a blocker");
        let said: Vec<&str> = found.iter().map(|f| f.evidence.as_str()).collect();
        assert!(said.contains(&"4 type sizes in one block (11, 12, 13, 28px; two or three is enough)") && said.contains(&"2 buttons look primary in the first screenful: “Export”, “Add fund”"), "{said:?}");
        let large = Condition::new(1280, "light", Variant::Text200);
        assert!(judge(&Taken { condition: large.code(), ..t }, &large).is_empty(), "text at 200% isn't a taste call");
    }

    #[test]
    fn a_report_tells_new_findings_on_changed_screens_from_those_there_before() {
        let dir = temp("assemble");
        let base = png(&dir.join("base.png"), 400, 200, None);
        // The new picture differs only in its top left corner.
        let head = png(&dir.join("head.png"), 400, 200, Some((0, 0, 60, 40)));
        let same = png(&dir.join("same.png"), 400, 200, None);
        let normal = Condition::new(390, "light", Variant::Normal);
        let large = Condition::new(390, "light", Variant::Text200);
        let lost = |text: &str, x: f64| Clip { rect: Rect { x, y: 2., w: 20., h: 10. }, ..clip(text, true, false) };
        let input = Input {
            ticket: Some(7),
            touched: Touched { screens: vec![ScreenRef { flow: "a".into(), screen: "b".into() }, ScreenRef { flow: "a".into(), screen: "c".into() }], unmapped: vec![], ..Touched::default() },
            conditions: vec![normal.clone(), large.clone()],
            taken: vec![
                // In the part that changed, and out of it.
                Taken { key: "a/b/default".into(), condition: normal.code(), path: Some(head.to_string_lossy().into()), dpr: 1., dom: Some(dom(vec![lost("Save", 5.), lost("Help", 300.)])), error: None },
                Taken { key: "a/b/default".into(), condition: large.code(), path: Some(head.to_string_lossy().into()), dpr: 1., dom: Some(dom(vec![lost("Cancel", 300.)])), error: None },
                // An unchanged screen with the same kind of trouble.
                Taken { key: "a/c/default".into(), condition: normal.code(), path: Some(same.to_string_lossy().into()), dpr: 1., dom: Some(dom(vec![lost("Old", 5.)])), error: None },
                Taken { key: "a/c/empty".into(), condition: normal.code(), path: None, dpr: 1., dom: None, error: Some("timed out".into()) },
            ],
            baselines: [(("a/b/default".to_owned(), normal.code()), base.clone()), (("a/c/default".to_owned(), normal.code()), base.clone())].into_iter().collect(),
            ..Input::default()
        };
        let report = assemble(input, &dir);
        assert_eq!(report.changed(), ["a/b/default"]);
        assert_eq!(report.failed(), 1);
        let new: Vec<(&str, bool)> = report.findings.iter().map(|f| (f.text.as_str(), f.new)).collect();
        assert!(new.contains(&("Save", true)), "in the changed part: {new:?}");
        assert!(new.contains(&("Help", false)), "beside the change, untouched: {new:?}");
        assert!(new.contains(&("Cancel", true)), "at 200% text on a changed screen: {new:?}");
        assert!(new.contains(&("Old", false)), "on an unchanged screen: {new:?}");
        assert_eq!(report.new_blockers().len(), 2);
        assert!(page_wide("html") && page_wide("body > main") && !page_wide("button.buy") && !page_wide("htmlish"));
        assert!(report.findings.iter().filter(|f| f.new).all(|f| f.crop.as_deref().is_some_and(|c| Path::new(c).is_file())), "each new finding cropped");
        let flagged = report.flagged();
        assert!(!flagged.is_empty() && flagged.len() <= MAX_FLAGGED);
        let md = report.summary("Would send it back.");
        assert!(md.contains("Would send it back.") && md.contains("Blockers (S1/S2)") && md.contains("Not this change"), "{md}");
        // Kept and read back the same.
        write(&dir, &report, &md).unwrap();
        let back = read(&dir).unwrap();
        assert_eq!((back.changed(), back.findings.len(), back.ticket), (report.changed(), report.findings.len(), Some(7)));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Files of a repository, from its root: `has` for [`app_root`].
    fn files(list: &[&str]) -> impl Fn(&str) -> bool {
        let list: Vec<String> = list.iter().map(|f| (*f).to_owned()).collect();
        move |f: &str| list.iter().any(|l| l == f)
    }

    fn changed(list: &[&str]) -> Vec<String> {
        list.iter().map(|f| (*f).to_owned()).collect()
    }

    #[test]
    fn a_change_in_a_multi_app_repository_is_in_the_app_folder_nearest_its_files() {
        // As a real team's repository: three apps side by side, the changed one a Vite app with no UI map.
        let repo = files(&["cafe/index.html", "habits/package.json", "habits/index.html", "o1path/package.json", "o1path/index.html", "o1path/public/index.html"]);
        let change = changed(&["o1path/src/App.tsx", "o1path/public/brand/logo.svg", "o1path/public/brand/mark.png", "o1path/design/assets/logo/logo.svg", "cafe/style.css", "README.md", "o1path/src/App.test.tsx"]);
        assert_eq!(app_roots(&change, &repo), ["o1path", "cafe"], "the app with the most changed files first; a README and a test are in none");
        // An index.html inside public/ is the app's assets, not an app of its own.
        assert_eq!(app_root("o1path/public/brand/logo.svg", &repo), "o1path");
        assert_eq!(app_root("cafe/css/site.css", &repo), "cafe");
        assert_eq!(app_root("tools/script.ts", &repo), "", "in no app folder: the root");
        assert_eq!(map_dirs(&app_roots(&change, &repo)), ["design", "o1path/design", "cafe/design"], "the root's map first, as always, then each changed app's");
    }

    #[test]
    fn a_single_app_at_the_root_is_the_root() {
        let repo = files(&["package.json", "index.html", "public/index.html", "packages/ui/package.json"]);
        assert_eq!(app_roots(&changed(&["src/App.tsx", "public/favicon.png", "src/styles.css"]), &repo), [""]);
        assert_eq!(app_roots(&changed(&["packages/ui/src/Button.tsx", "src/App.tsx", "src/Nav.tsx"]), &repo), ["", "packages/ui"]);
        assert_eq!(map_dirs(&[String::new()]), ["design"]);
        assert!(app_roots(&changed(&["server/api.ts", "docs/guide.md", "Cargo.toml"]), &repo).is_empty(), "nothing users see: no app to render");
    }

    #[test]
    fn a_change_only_in_design_folders_is_in_the_app_whose_design_it_is() {
        let root_app = files(&["package.json", "index.html"]);
        assert_eq!(app_roots(&changed(&["design/tokens.json", "design/assets/logo.svg", "design/brand.md"]), &root_app), [""]);
        // An app's own design folder, even with a page of its own in it (a style guide), is that app's.
        let multi = files(&["o1path/package.json", "o1path/design/index.html", "habits/package.json"]);
        assert_eq!(app_roots(&changed(&["o1path/design/assets/logo/mark.svg", "o1path/design/brand.md", "o1path/design/index.html"]), &multi), ["o1path"]);
        assert!(shown_by_app("o1path/design/tokens/color.json") && shown_by_app("public/brand/hero.webp"));
        assert!(!shown_by_app("design/brand.md") && !shown_by_app("tests/fixtures/shot.png") && !shown_by_app("package.json"));
    }

    #[test]
    fn an_apps_own_map_names_its_files_from_the_app_folder() {
        let map = json!({ "flows": [{ "id": "home", "screens": [{ "id": "landing", "source": ["src/App.tsx"], "states": [{ "id": "default" }] }] }] });
        let change = changed(&["o1path/src/App.tsx", "o1path/src/Footer.tsx", "cafe/style.css", "o1path/design/flows/home/landing/default.light.390.png"]);
        let t = touched_in_app(&map, "o1path", &change);
        assert_eq!(t.screens, [ScreenRef { flow: "home".into(), screen: "landing".into() }]);
        assert_eq!(t.unmapped, ["o1path/src/Footer.tsx", "cafe/style.css"], "named from the repository's root again; the other app's file is unmapped");
        // The root's own map: exactly as before.
        let root_map = json!({ "flows": [{ "id": "home", "screens": [{ "id": "landing", "source": ["o1path/src/App.tsx"], "states": [{ "id": "default" }] }] }] });
        assert_eq!(touched_in_app(&root_map, "", &change), touched(&root_map, "design/flows", &change));
    }

    #[test]
    fn an_app_without_a_map_is_built_by_its_vite_served_as_files_or_said_why_not() {
        let dir = temp("serve");
        let app = |name: &str, files: &[(&str, &str)]| {
            let at = dir.join(name);
            for (file, text) in files {
                let path = at.join(file);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            at
        };
        let vite = r#"{ "scripts": { "build": "tsc -b && vite build" }, "devDependencies": { "vite": "^5" } }"#;
        assert_eq!(serve_plan(&app("vite", &[("package.json", vite), ("index.html", "<div id=root>"), ("node_modules/.bin/vite", "")])), Ok(Serve::Vite));
        let missing = serve_plan(&app("vite-bare", &[("package.json", vite), ("index.html", "<div id=root>")])).unwrap_err();
        assert!(missing.contains("dependencies aren't installed"), "{missing}");
        let other = serve_plan(&app("webpack", &[("package.json", r#"{ "scripts": { "build": "webpack" } }"#), ("index.html", "")])).unwrap_err();
        assert!(other.contains("`webpack`") && other.contains("precheck.preview"), "{other}");
        assert_eq!(serve_plan(&app("static", &[("index.html", "<h1>Cafe</h1>")])), Ok(Serve::Files));
        assert_eq!(serve_plan(&app("tooling", &[("package.json", r#"{ "scripts": { "lint": "eslint ." } }"#), ("index.html", "")])), Ok(Serve::Files), "no build step: served as it is");
        assert!(serve_plan(&app("nothing", &[("main.py", "")])).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_pages_of_an_app_without_a_map_are_its_home_and_its_language_versions() {
        let dir = temp("routes");
        for page in ["index.html", "en/index.html", "ko/index.html", "pt-br/index.html", "zh_Hant/index.html", "assets/index.html", "about/index.html", "fr/notes.txt"] {
            let path = dir.join(page);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "<p>").unwrap();
        }
        assert_eq!(fallback_routes(&dir), ["/", "/en/", "/ko/", "/pt-br/"], "a handful at most, languages only");
        assert_eq!(fallback_routes(&dir.join("about")), ["/"]);
        assert_eq!(fallback_routes(&dir.join("nowhere")), ["/"], "the home page is always asked for");
        assert_eq!((route_screen("/"), route_screen("/pt-br/"), app_flow(""), app_flow("apps/web")), ("home".into(), "pt-br".into(), "app".into(), "apps-web".into()));
        // The static server: files, a folder's page, a client-side route, nothing hidden or outside.
        assert_eq!(static_file(&dir, "/"), Some(dir.join("index.html")));
        assert_eq!(static_file(&dir, "/ko/?utm=x"), Some(dir.join("ko/index.html")));
        assert_eq!(static_file(&dir, "/zh%5FHant"), Some(dir.join("zh_Hant/index.html")));
        assert_eq!(static_file(&dir, "/settings/profile"), Some(dir.join("index.html")), "a client-side route gets the app's page");
        assert_eq!(static_file(&dir, "/brand/logo.svg"), None, "a missing file is a 404, not the page");
        std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
        assert_eq!(static_file(&dir, "/.env"), None);
        assert_eq!(static_file(&dir, "/../etc/passwd"), None);
        assert_eq!(content_type(Path::new("a/logo.svg")), "image/svg+xml");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn with_nothing_to_compare_with_findings_are_told_but_not_held_against_the_change() {
        let dir = temp("nobase");
        let picture = png(&dir.join("home.png"), 400, 200, None);
        let normal = Condition::new(390, "light", Variant::Normal);
        let lost = Clip { rect: Rect { x: 5., y: 2., w: 20., h: 10. }, ..clip("Sign up", true, false) };
        let taken = vec![Taken { key: "o1path/home/default".into(), condition: normal.code(), path: Some(picture.to_string_lossy().into()), dpr: 1., dom: Some(dom(vec![lost])), error: None }];
        let input = |no_base: bool| Input {
            touched: Touched { unmapped: vec!["o1path/src/App.tsx".into()], ..Touched::default() },
            conditions: vec![normal.clone()],
            taken: taken.clone(),
            fallback: Some("no UI map; rendered the changed app at `o1path/` (built with its own Vite) at /; no base comparison (the app is new since the base)".into()),
            no_base,
            ..Input::default()
        };
        let report = assemble(input(true), &dir);
        assert_eq!(report.taken(), 1);
        assert!(!report.findings.is_empty() && report.new_blockers().is_empty(), "{:?}", report.findings);
        assert!(report.findings.iter().all(|f| f.evidence.contains("may predate this change")));
        let md = report.summary("A designer checks what this report flags.");
        assert!(md.contains("- Fallback: no UI map; rendered the changed app at `o1path/`") && md.contains("no base comparison"), "{md}");
        // Kept and read back with it; a report without one says nothing of it.
        write(&dir, &report, &md).unwrap();
        assert_eq!(read(&dir).unwrap().fallback, report.fallback);
        assert!(!serde_json::to_string(&Report::default()).unwrap().contains("fallback"));
        // Without the flag, as for a new state on a map, the finding is the change's.
        assert_eq!(assemble(input(false), &dir).new_blockers().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
