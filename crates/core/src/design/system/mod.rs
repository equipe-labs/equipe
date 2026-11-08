//! A project's design system as code: its tokens (a DTCG `tokens.json`, the
//! CSS variables a theme sets, Tailwind v4's `@theme`), its component
//! registry (`components.json`, `registry.json`, the folder the components
//! are kept in, what `shadcn build` wrote) and a docs page per component —
//! found wherever the project keeps them, never at fixed paths, and said in
//! plain words: what is there and what is missing.
//!
//! - `catalog`: what equipe knows of each component, as data.
//! - `css`: tokens written as CSS custom properties.
//! - `registry`: `registry.json` checked, built and installed into a scratch project.
//! - `matrix`: a component drawn in every variant, state and theme, and judged.

pub mod catalog;
pub mod css;
pub mod matrix;
pub mod registry;

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::design::tokens;
use crate::protocol::design::Token;

/// Folders never searched: dependencies, builds, caches.
const SKIP: [&str; 14] = ["node_modules", "dist", "build", "target", "out", ".next", ".nuxt", ".svelte-kit", "vendor", "coverage", ".turbo", ".vercel", "checks", "Pods"];
/// The most files looked at, and how deep.
const MAX_FILES: usize = 4000;
const MAX_DEPTH: usize = 7;

/// How a tokens file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Shape {
    /// W3C Design Tokens JSON (`$value`, `$type`).
    Dtcg,
    /// Custom properties on `:root` (and `.dark`).
    CssVariables,
    /// Tailwind v4's `@theme`.
    TailwindTheme,
}

impl Shape {
    pub fn words(self) -> &'static str {
        match self {
            Shape::Dtcg => "DTCG tokens",
            Shape::CssVariables => "CSS variables",
            Shape::TailwindTheme => "Tailwind @theme",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TokenFile {
    pub path: PathBuf,
    pub shapes: Vec<Shape>,
    pub tokens: usize,
    pub colours: usize,
    /// It gives colours a dark value.
    pub dark: bool,
}

/// `components.json`, as `shadcn init` writes it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ComponentsJson {
    pub path: PathBuf,
    pub style: Option<String>,
    /// The stylesheet its theme is in (`tailwind.css`).
    pub css: Option<String>,
    pub css_variables: bool,
    /// `components`, `ui`, `lib`, `utils`, `hooks`, as written (`@/components/ui`).
    pub aliases: Vec<(String, String)>,
    /// The registries it installs from by namespace (`@acme`).
    pub registries: Vec<(String, String)>,
}

impl ComponentsJson {
    pub fn read(path: &Path) -> Option<ComponentsJson> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        let s = |p: &str| v.pointer(p).and_then(|x| x.as_str()).map(str::to_owned);
        let pairs = |p: &str| -> Vec<(String, String)> {
            v.pointer(p).and_then(|x| x.as_object()).map(|o| o.iter().map(|(k, x)| (k.clone(), x.as_str().map_or_else(|| x.to_string(), str::to_owned))).collect()).unwrap_or_default()
        };
        Some(ComponentsJson {
            path: path.to_path_buf(),
            style: s("/style"),
            css: s("/tailwind/css"),
            css_variables: v.pointer("/tailwind/cssVariables").and_then(|x| x.as_bool()).unwrap_or(true),
            aliases: pairs("/aliases"),
            registries: pairs("/registries"),
        })
    }

    pub fn alias(&self, name: &str) -> Option<&str> {
        self.aliases.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// An import alias as a folder: `@/components/ui` → `<root>/src/components/ui`
/// when the project keeps its code in `src/`, else `<root>/components/ui`.
pub fn alias_dir(root: &Path, alias: &str) -> PathBuf {
    let rest = alias.trim_start_matches("@/").trim_start_matches("~/").trim_start_matches('@').trim_start_matches('/');
    let src = root.join("src").join(rest);
    if root.join("src").is_dir() && (src.exists() || !root.join(rest).exists()) { src } else { root.join(rest) }
}

/// A docs page found for a component.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Doc {
    pub component: String,
    pub path: PathBuf,
}

/// A project's design system, as found.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Detected {
    pub root: PathBuf,
    pub tokens: Vec<TokenFile>,
    pub components_json: Option<ComponentsJson>,
    pub registry: Option<registry::Registry>,
    /// Where the components are kept.
    pub components_dir: Option<PathBuf>,
    /// The components it has, by catalog name where the catalog knows them.
    pub components: Vec<String>,
    /// What `shadcn build` wrote: the folder of item JSON files.
    pub built: Option<PathBuf>,
    pub docs: Vec<Doc>,
}

impl Detected {
    /// Whether there is a system for product code to be built from: a
    /// registry, or a `components.json`.
    pub fn has_system(&self) -> bool {
        self.registry.is_some() || self.components_json.is_some()
    }

    /// Every token, from every tokens file found.
    pub fn all_tokens(&self) -> Vec<Token> {
        self.tokens.iter().flat_map(|f| read_tokens(&f.path)).collect()
    }

    pub fn doc(&self, component: &str) -> Option<&Doc> {
        self.docs.iter().find(|d| d.component == component)
    }

    /// The folders a component's own code lives in: not product code.
    pub fn sources(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = self.components_dir.iter().cloned().collect();
        if let Some(reg) = &self.registry {
            for item in &reg.items {
                for f in &item.files {
                    if let Some(dir) = self.root.join(&f.path).parent()
                        && !out.iter().any(|d| d == dir)
                    {
                        out.push(dir.to_path_buf());
                    }
                }
            }
        }
        out
    }
}

/// The tokens a file holds, whatever its shape.
pub fn read_tokens(path: &Path) -> Vec<Token> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    if path.extension().is_some_and(|e| e == "json") { tokens::parse(&text).tokens } else { css::tokens(&css::read(&text)) }
}

fn walk(root: &Path, each: &mut dyn FnMut(&Path)) {
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut seen = 0;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if p.is_dir() {
                if depth < MAX_DEPTH && !name.starts_with('.') && !SKIP.contains(&name.as_str()) {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            seen += 1;
            if seen > MAX_FILES {
                return;
            }
            each(&p);
        }
    }
}

fn token_file(path: &Path) -> Option<TokenFile> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    let text = std::fs::read_to_string(path).ok()?;
    let (shapes, tokens) = match ext.as_str() {
        "json" if (name == "tokens.json" || name.ends_with(".tokens.json") || path.components().any(|c| c.as_os_str() == "tokens")) && text.contains("\"$value\"") => {
            (vec![Shape::Dtcg], tokens::parse(&text).tokens)
        }
        "css" | "scss" | "pcss" => {
            let sheet = css::read(&text);
            let mut shapes = Vec::new();
            if sheet.properties.iter().any(|p| p.place != css::Place::Theme) {
                shapes.push(Shape::CssVariables);
            }
            if sheet.theme {
                shapes.push(Shape::TailwindTheme);
            }
            let tokens = css::tokens(&sheet);
            // A sheet that sets a few properties of its own isn't a theme.
            if tokens.iter().filter(|t| t.kind == "color").count() < 3 {
                return None;
            }
            (shapes, tokens)
        }
        _ => return None,
    };
    if tokens.is_empty() {
        return None;
    }
    let colours = tokens.iter().filter(|t| t.kind == "color").count();
    let dark = tokens.iter().any(|t| t.kind == "color" && t.modes.contains_key("dark"));
    Some(TokenFile { path: path.to_path_buf(), shapes, tokens: tokens.len(), colours, dark })
}

/// Where a docs page for a component is, by how docs sites lay them out:
/// `docs/components/button.mdx`, `app/docs/components/button/page.tsx`,
/// `content/docs/button.md`, `stories/Button.stories.tsx`.
fn doc_of(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    let name = path.file_name()?.to_string_lossy().into_owned();
    let lower = name.to_ascii_lowercase();
    if let Some(stem) = lower.strip_suffix(".stories.tsx").or_else(|| lower.strip_suffix(".stories.jsx")).or_else(|| lower.strip_suffix(".stories.ts")).or_else(|| lower.strip_suffix(".stories.mdx")) {
        return Some(catalog::normal(&name[..stem.len()]));
    }
    if !rel.contains("docs/") && !rel.contains("/docs") {
        return None;
    }
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if !["md", "mdx", "tsx", "jsx", "html", "astro", "vue", "svelte"].contains(&ext.as_str()) {
        return None;
    }
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    // A route's page: its folder is the name.
    if ["page", "index", "+page", "route"].contains(&stem.to_ascii_lowercase().as_str()) {
        return path.parent()?.file_name().map(|n| catalog::normal(&n.to_string_lossy()));
    }
    Some(catalog::normal(&stem))
}

/// Where a project usually keeps its theme's stylesheet and its tokens,
/// looked at first (and alone, when the edit-time check asks).
const USUAL: [&str; 9] = ["design/tokens.json", "tokens.json", "app/globals.css", "src/app/globals.css", "src/index.css", "src/styles/globals.css", "styles/globals.css", "src/app.css", "app/app.css"];

/// The design system of the project at `root`, read from its usual places
/// only: `components.json` (and the stylesheet it names), `registry.json`,
/// the components' folder. Quick enough for an edit-time check.
pub fn detect_quick(root: &Path) -> Detected {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut found = Detected { root: root.clone(), ..Detected::default() };
    found.components_json = ComponentsJson::read(&root.join("components.json"));
    found.registry = registry::Registry::read(&root.join("registry.json")).ok();
    let named = found.components_json.as_ref().and_then(|c| c.css.clone()).filter(|c| !c.is_empty());
    for rel in named.iter().map(String::as_str).chain(USUAL) {
        let path = root.join(rel);
        if !found.tokens.iter().any(|t| t.path == path)
            && let Some(t) = token_file(&path)
        {
            found.tokens.push(t);
        }
    }
    let ui = found.components_json.as_ref().and_then(|c| c.alias("ui").map(str::to_owned).or_else(|| c.alias("components").map(|a| format!("{a}/ui"))));
    let candidates: Vec<PathBuf> = ui.iter().map(|a| alias_dir(&root, a)).chain(["components/ui", "src/components/ui", "registry/ui", "src/registry/ui"].iter().map(|p| root.join(p))).collect();
    found.components_dir = candidates.into_iter().find(|d| d.is_dir());
    let mut names: Vec<String> = Vec::new();
    let mut add = |n: String| {
        let n = catalog::catalog().get(&n).map_or(n, |c| c.name.clone());
        if !names.contains(&n) {
            names.push(n);
        }
    };
    if let Some(reg) = &found.registry {
        for item in reg.items.iter().filter(|i| i.kind == "registry:ui" || i.kind == "registry:component") {
            add(catalog::normal(&item.name));
        }
    }
    if let Some(dir) = &found.components_dir
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        let mut files: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        files.sort();
        for p in files {
            let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if ["tsx", "jsx", "vue", "svelte", "ts", "js"].contains(&ext.as_str()) {
                add(catalog::normal(&p.file_stem().unwrap_or_default().to_string_lossy()));
            } else if p.is_dir() {
                // A component in a folder of its own (`button/index.ts`, Vue's and Svelte's way).
                add(catalog::normal(&p.file_name().unwrap_or_default().to_string_lossy()));
            }
        }
    }
    found.components = names;
    found
}

/// The design system of the project at `root`: its usual places, then the
/// whole project for tokens files, docs pages and what `shadcn build` wrote.
pub fn detect(root: &Path) -> Detected {
    let mut found = detect_quick(root);
    let root = found.root.clone();
    let mut docs: Vec<Doc> = Vec::new();
    let mut built: Option<PathBuf> = None;
    let mut more: Vec<TokenFile> = Vec::new();
    walk(&root, &mut |p| {
        if !found.tokens.iter().any(|t| t.path == p)
            && let Some(t) = token_file(p)
        {
            more.push(t);
        }
        if let Some(component) = doc_of(&root, p)
            && catalog::catalog().get(&component).is_some()
            && !docs.iter().any(|d| d.component == component)
        {
            docs.push(Doc { component: catalog::catalog().get(&component).map_or(component, |c| c.name.clone()), path: p.to_path_buf() });
        }
        // `shadcn build`'s output: item JSON files with their files' contents.
        if built.is_none()
            && p.extension().is_some_and(|e| e == "json")
            && p.parent().is_some_and(|d| d.file_name().is_some_and(|n| n == "r"))
            && std::fs::read_to_string(p).is_ok_and(|t| t.contains("\"files\"") && t.contains("\"content\""))
        {
            built = p.parent().map(Path::to_path_buf);
        }
    });
    found.tokens.extend(more);
    found.docs = docs;
    found.built = built;
    found
}

/// The project a file belongs to, as far as its design system goes: the
/// nearest folder at or above it with a `components.json` or `registry.json`.
pub fn project_of(file: &Path) -> Option<PathBuf> {
    let start = if file.is_dir() { file.to_path_buf() } else { file.parent()?.to_path_buf() };
    let start = std::fs::canonicalize(&start).unwrap_or(start);
    for dir in start.ancestors() {
        if dir.join("components.json").is_file() || dir.join("registry.json").is_file() {
            return Some(dir.to_path_buf());
        }
        // A repository's root is as far as its project goes.
        if dir.join(".git").exists() {
            return None;
        }
    }
    None
}

/// How much of the catalog's core set a system has.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Coverage {
    pub total: usize,
    pub present: Vec<String>,
    pub missing: Vec<String>,
    /// Newer ones it has beyond the core set.
    pub extras: Vec<String>,
    /// Its own, which the catalog doesn't know.
    pub own: Vec<String>,
    /// Present without a docs page.
    pub undocumented: Vec<String>,
}

impl Coverage {
    pub fn line(&self) -> String {
        let mut out = format!("{}/{} components", self.present.len(), self.total);
        if !self.missing.is_empty() {
            out.push_str(&format!(", missing: {}", self.missing.join(", ")));
        }
        out
    }
}

pub fn coverage(found: &Detected) -> Coverage {
    let cat = catalog::catalog();
    let mut c = Coverage { total: cat.core().count(), ..Coverage::default() };
    for comp in cat.core() {
        if found.components.contains(&comp.name) {
            c.present.push(comp.name.clone());
        } else {
            c.missing.push(comp.name.clone());
        }
    }
    for name in &found.components {
        match cat.get(name) {
            Some(comp) if !comp.core() => c.extras.push(name.clone()),
            Some(_) => {}
            None => c.own.push(name.clone()),
        }
        if cat.get(name).is_some() && found.doc(name).is_none() {
            c.undocumented.push(name.clone());
        }
    }
    c
}

/// A line each on what a system lacks, as what to do.
pub fn missing(found: &Detected) -> Vec<String> {
    let mut out = Vec::new();
    if found.tokens.is_empty() {
        out.push("No design tokens found: keep them as a DTCG tokens.json (exported to CSS variables for the code), or at least as the CSS variables a theme sets on :root and .dark.".to_owned());
    } else if !found.tokens.iter().any(|t| t.dark) {
        out.push("The tokens have no dark values: give each colour one (`.dark { … }`, or `$extensions.mode.dark` in DTCG).".to_owned());
    }
    if found.components_json.is_none() {
        out.push("No components.json: write one (`npx shadcn init`) so components install where this project keeps them, with its aliases and its theme's stylesheet.".to_owned());
    }
    match &found.registry {
        None => out.push("No registry.json: the components aren't a registry yet — list each one as an item (its files, dependencies, source and licence) so `npx shadcn add` installs them from it.".to_owned()),
        Some(_) if found.built.is_none() => out.push("registry.json isn't built: `shadcn build` writes each item, with its files' contents, to public/r/.".to_owned()),
        Some(_) => {}
    }
    if found.components_dir.is_none() && found.registry.is_none() {
        out.push("No folder of components found (components.json's `ui` alias, components/ui).".to_owned());
    }
    let cov = coverage(found);
    if !cov.missing.is_empty() && found.has_system() {
        out.push(format!("{} — build the missing ones on the shadcn base, restyled to the tokens.", cov.line()));
    }
    if !cov.undocumented.is_empty() {
        out.push(format!("No docs page for: {} — each component gets one (examples, every variant and state, light and dark, accessibility notes).", cov.undocumented.join(", ")));
    }
    out
}

/// What was found and what is missing, as a person reads it.
pub fn report(found: &Detected) -> String {
    let rel = |p: &Path| p.strip_prefix(&found.root).map_or_else(|_| p.display().to_string(), |r| r.display().to_string());
    let mut out = String::new();
    for t in &found.tokens {
        out.push_str(&format!(
            "✓ tokens: {} ({}) — {} tokens, {} colours{}\n",
            rel(&t.path),
            t.shapes.iter().map(|s| s.words()).collect::<Vec<_>>().join(", "),
            t.tokens,
            t.colours,
            if t.dark { ", light and dark" } else { ", no dark values" }
        ));
    }
    if let Some(c) = &found.components_json {
        out.push_str(&format!("✓ components.json: style {}, theme in {}{}\n", c.style.as_deref().unwrap_or("—"), c.css.as_deref().unwrap_or("—"), if c.registries.is_empty() { String::new() } else { format!(", registries {}", c.registries.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(", ")) }));
    }
    if let Some(r) = &found.registry {
        out.push_str(&format!("✓ registry.json: {} — {} items\n", r.name, r.items.len()));
    }
    if let Some(b) = &found.built {
        out.push_str(&format!("✓ built: {}\n", rel(b)));
    }
    if let Some(d) = &found.components_dir {
        out.push_str(&format!("✓ components in {}\n", rel(d)));
    }
    let cov = coverage(found);
    if found.has_system() || !found.components.is_empty() {
        out.push_str(&format!("{} {}\n", if cov.missing.is_empty() { "✓" } else { "·" }, cov.line()));
        if !cov.extras.is_empty() {
            out.push_str(&format!("  also: {}\n", cov.extras.join(", ")));
        }
        if !cov.own.is_empty() {
            out.push_str(&format!("  its own: {}\n", cov.own.join(", ")));
        }
        out.push_str(&format!("  docs pages: {}/{}\n", found.components.iter().filter(|c| found.doc(c).is_some()).count(), found.components.len()));
    }
    let tokens = found.all_tokens();
    for p in token_contrast(&tokens) {
        out.push_str(&format!("✗ {p}\n"));
    }
    for m in missing(found) {
        out.push_str(&format!("- {m}\n"));
    }
    out
}

/// The surface a text colour is named for: `primary-foreground` → `primary`,
/// `foreground` → `background`, `color.on-primary` → `color.primary`.
fn surface_of(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    if lower == "foreground" || lower.ends_with("-foreground") && lower.trim_end_matches("-foreground").rsplit(['.', '-']).next() == Some("color") {
        return Some(lower.replace("foreground", "background"));
    }
    if let Some(surface) = lower.strip_suffix("-foreground").or_else(|| lower.strip_suffix(".foreground")) {
        return Some(surface.to_owned());
    }
    let (group, last) = lower.rsplit_once('.').unwrap_or(("", &lower));
    let rest = last.strip_prefix("on-")?;
    Some(if group.is_empty() { rest.to_owned() } else { format!("{group}.{rest}") })
}

/// Text on its surface, as a system's tokens pair them — shadcn's
/// `primary` / `primary-foreground`, or `on-primary` — and the focus ring on
/// the page: each pair below WCAG AA in a mode, said.
pub fn token_contrast(tokens: &[Token]) -> Vec<String> {
    use crate::design::{contrast, values};
    let colour = |t: &Token, mode: Option<&str>| values::color(t.value_in(mode));
    let mut out = Vec::new();
    let modes: Vec<Option<&str>> = if tokens.iter().any(|t| t.modes.contains_key("dark")) { vec![Some("light"), Some("dark")] } else { vec![None] };
    let find = |name: &str| tokens.iter().find(|t| t.kind == "color" && t.name.eq_ignore_ascii_case(name));
    for fg in tokens.iter().filter(|t| t.kind == "color") {
        let Some(bg) = surface_of(&fg.name).and_then(|s| find(&s)) else { continue };
        for mode in &modes {
            let (Some(f), Some(b)) = (colour(fg, *mode), colour(bg, *mode)) else { continue };
            let r = contrast::ratio(f, contrast::over(b, values::Color::WHITE));
            if r < 4.5 {
                out.push(format!("`{}` on `{}` is {r:.1}:1{} — text needs 4.5:1", fg.name, bg.name, mode.map(|m| format!(" in {m}")).unwrap_or_default()));
            }
        }
    }
    if let (Some(ring), Some(bg)) = (find("ring").or_else(|| find("color-ring")), find("background").or_else(|| find("color-background"))) {
        for mode in &modes {
            let (Some(f), Some(b)) = (colour(ring, *mode), colour(bg, *mode)) else { continue };
            let r = contrast::ratio(f, contrast::over(b, values::Color::WHITE));
            if r < 3. {
                out.push(format!("the focus ring `{}` on `{}` is {r:.1}:1{} — a focus indicator needs 3:1", ring.name, bg.name, mode.map(|m| format!(" in {m}")).unwrap_or_default()));
            }
        }
    }
    out.dedup();
    out
}

#[cfg(test)]
mod tests;
