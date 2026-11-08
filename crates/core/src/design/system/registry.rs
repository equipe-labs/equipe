//! A component registry as shadcn's CLI reads it: `registry.json` (its
//! items, each with its files, dependencies and the items it needs) and
//! what `shadcn build` makes of it (an item JSON a component, its files'
//! contents inside). Checked before a release: every file there, every
//! import declared, where each item came from and under what licence, no
//! other brand's pictures; and a sample install into a scratch project —
//! offline, from the built files — that resolves.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The item types shadcn's CLI installs.
const TYPES: [&str; 11] = ["registry:ui", "registry:component", "registry:block", "registry:lib", "registry:hook", "registry:page", "registry:file", "registry:style", "registry:theme", "registry:item", "registry:example"];

/// Packages a component may import without declaring: the framework the
/// project has already.
const PROVIDED: [&str; 8] = ["react", "react-dom", "next", "vue", "svelte", "nuxt", "@sveltejs/kit", "solid-js"];

/// Hosts whose pictures belong to another brand, as shadcn's own examples
/// use them (avatars, logos), never to ship in a system of one's own.
const OTHER_BRANDS_HOSTS: [&str; 9] = ["github.com/", "avatars.githubusercontent.com", "ui.shadcn.com", "vercel.com", "nextjs.org", "assets.vercel.com", "images.unsplash.com", "pbs.twimg.com", "cdn.discordapp.com"];

/// Brands whose name on an asset file says it is theirs.
const BRANDS: [&str; 22] = [
    "vercel", "nextjs", "next-js", "shadcn", "github", "google", "apple", "microsoft", "stripe", "radix", "tailwind", "figma", "slack", "discord", "twitter", "facebook", "instagram", "linkedin", "openai", "anthropic", "notion", "spotify",
];

const ASSET_EXTENSIONS: [&str; 11] = ["svg", "png", "jpg", "jpeg", "webp", "gif", "ico", "woff", "woff2", "ttf", "otf"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct File {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// In a built item: the file itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Item {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub dependencies: Vec<String>,
    pub dev_dependencies: Vec<String>,
    pub registry_dependencies: Vec<String>,
    pub files: Vec<File>,
    pub docs: Option<String>,
    pub meta: serde_json::Map<String, serde_json::Value>,
}

impl Item {
    fn meta(&self, keys: &[&str]) -> Option<String> {
        keys.iter().find_map(|k| self.meta.get(*k)).map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)).filter(|s| !s.trim().is_empty())
    }

    /// Where it came from: `meta.source` (a URL, or `original`).
    pub fn source(&self) -> Option<String> {
        self.meta(&["source", "origin", "basedOn"])
    }

    pub fn license(&self) -> Option<String> {
        self.meta(&["license", "licence"])
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Registry {
    pub name: String,
    pub homepage: Option<String>,
    pub items: Vec<Item>,
    /// Where it was read from.
    #[serde(skip)]
    pub path: PathBuf,
}

impl Registry {
    pub fn read(path: &Path) -> Result<Registry, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut r: Registry = serde_json::from_str(&text).map_err(|e| format!("{}: not a registry ({e})", path.display()))?;
        r.path = path.to_path_buf();
        Ok(r)
    }

    pub fn item(&self, name: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.name == name)
    }

    fn root(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Error,
    Warn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub level: Level,
    /// The item, when it is about one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub words: String,
}

impl Problem {
    fn error(item: Option<&str>, words: impl Into<String>) -> Self {
        Problem { level: Level::Error, item: item.map(str::to_owned), words: words.into() }
    }

    fn warn(item: Option<&str>, words: impl Into<String>) -> Self {
        Problem { level: Level::Warn, item: item.map(str::to_owned), words: words.into() }
    }

    pub fn line(&self) -> String {
        format!("{} {}{}", if self.level == Level::Error { "✗" } else { "!" }, self.item.as_ref().map(|i| format!("{i}: ")).unwrap_or_default(), self.words)
    }
}

/// What a source file imports: each module named in `from "…"`,
/// `import "…"`, `require("…")` and `import("…")`.
pub fn imports(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("//") || t.starts_with('*') || t.starts_with("/*") {
            continue;
        }
        for key in ["from ", "import ", "require(", "import("] {
            let mut rest = t;
            while let Some(at) = rest.find(key) {
                let after = rest[at + key.len()..].trim_start();
                let Some(q) = after.chars().next().filter(|c| matches!(c, '"' | '\'' | '`')) else {
                    rest = &rest[at + key.len()..];
                    continue;
                };
                if let Some(end) = after[1..].find(q) {
                    let spec = after[1..1 + end].to_owned();
                    if !spec.is_empty() && !out.contains(&spec) {
                        out.push(spec);
                    }
                }
                rest = &rest[at + key.len()..];
            }
        }
    }
    out
}

/// The package a module belongs to: `@radix-ui/react-dialog/x` → `@radix-ui/react-dialog`; none for a path or an alias.
pub fn package(spec: &str) -> Option<String> {
    if spec.starts_with(['.', '/', '~', '#']) || spec.starts_with("@/") || spec.starts_with("node:") || spec.starts_with("$lib") || spec.starts_with("virtual:") {
        return None;
    }
    let mut parts = spec.split('/');
    let first = parts.next()?;
    if first.starts_with('@') { parts.next().map(|second| format!("{first}/{second}")) } else { Some(first.to_owned()) }
}

/// A declared dependency without its version: `lucide-react@^0.4` → `lucide-react`.
fn bare(dep: &str) -> &str {
    match dep.strip_prefix('@') {
        Some(rest) => rest.find('@').map_or(dep, |at| &dep[..at + 1]),
        None => dep.split('@').next().unwrap_or(dep),
    }
}

/// The registry item an alias import names: `@/components/ui/button` →
/// `button`, `@/lib/utils` → `utils`, `@/registry/new-york/ui/dialog` → `dialog`.
fn item_named_by(spec: &str) -> Option<String> {
    if !(spec.starts_with("@/") || spec.starts_with("~/") || spec.starts_with("$lib")) {
        return None;
    }
    let last = spec.trim_end_matches('/').rsplit('/').next()?;
    let last = last.split('.').next().unwrap_or(last);
    (!last.is_empty()).then(|| last.to_owned())
}

/// The item a `registryDependencies` entry names, if it is one of `reg`'s
/// own: by its URL under the homepage, a namespaced `@ns/name`, or a path.
fn own_dependency<'a>(reg: &'a Registry, dep: &str) -> Option<&'a Item> {
    let last = dep.trim_end_matches(".json").rsplit('/').next().unwrap_or(dep);
    let ours = dep.starts_with("http") && reg.homepage.as_deref().is_some_and(|h| dep.starts_with(h.trim_end_matches('/'))) || dep.starts_with(&format!("@{}/", reg.name)) || dep.starts_with('.') || dep.ends_with(".json") && !dep.starts_with("http");
    if ours { reg.item(last) } else { None }
}

fn is_bare_name(dep: &str) -> bool {
    !dep.contains('/') && !dep.contains(':')
}

/// Every problem with `reg`: its items, their files, imports, origins and
/// assets; with `built`, the output of `shadcn build` too.
pub fn check(reg: &Registry, built: Option<&Path>) -> Vec<Problem> {
    let mut out = Vec::new();
    if reg.name.trim().is_empty() {
        out.push(Problem::error(None, "registry.json has no `name`: it is how a project names this registry (`@name/button`)."));
    }
    if reg.homepage.as_deref().is_none_or(|h| h.trim().is_empty()) {
        out.push(Problem::warn(None, "registry.json has no `homepage`: the items' URLs (`<homepage>/r/<item>.json`) start from it."));
    }
    if reg.items.is_empty() {
        out.push(Problem::error(None, "registry.json lists no items."));
    }
    let root = reg.root();
    let mut names = BTreeSet::new();
    for item in &reg.items {
        let n = Some(item.name.as_str());
        if item.name.trim().is_empty() {
            out.push(Problem::error(None, "an item has no `name`."));
            continue;
        }
        if !names.insert(item.name.clone()) {
            out.push(Problem::error(n, "listed twice."));
        }
        if !TYPES.contains(&item.kind.as_str()) {
            out.push(Problem::error(n, format!("`type` is `{}`; shadcn installs {}.", item.kind, TYPES.join(", "))));
        }
        if item.files.is_empty() && !matches!(item.kind.as_str(), "registry:style" | "registry:theme") {
            out.push(Problem::error(n, "lists no files."));
        }
        if item.source().is_none() {
            out.push(Problem::error(n, "doesn't say where it came from: set `meta.source` (the shadcn component's URL it was built on, or `original`)."));
        }
        if item.license().is_none() && item.source().is_some_and(|s| s != "original") {
            out.push(Problem::error(n, "doesn't say its source's licence: set `meta.license` (shadcn/ui's is MIT) and keep the licence text in the repository."));
        }
        if matches!(item.kind.as_str(), "registry:ui" | "registry:component" | "registry:block") && item.title.is_none() && item.description.is_none() {
            out.push(Problem::warn(n, "has no `title` or `description` for the docs and the CLI's list."));
        }
        let declared: BTreeSet<&str> = item.dependencies.iter().chain(&item.dev_dependencies).map(|d| bare(d)).collect();
        let mut used: BTreeSet<String> = BTreeSet::new();
        let own: BTreeSet<String> = item.files.iter().filter_map(|f| Path::new(&f.path).file_stem().map(|s| s.to_string_lossy().into_owned())).collect();
        for f in &item.files {
            if matches!(f.kind.as_str(), "registry:file" | "registry:page") && f.target.as_deref().is_none_or(str::is_empty) {
                out.push(Problem::error(n, format!("{} is a {} with no `target`: say where it goes in a project.", f.path, f.kind)));
            }
            let path = root.join(&f.path);
            let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if ASSET_EXTENSIONS.contains(&ext.as_str()) {
                let stem = path.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                if let Some(brand) = BRANDS.iter().find(|b| stem.contains(*b) && !reg.name.to_ascii_lowercase().contains(*b)) {
                    out.push(Problem::error(n, format!("{} is named for {brand}: another brand's asset doesn't ship in this system — draw or license your own.", f.path)));
                } else if item.license().is_none() {
                    out.push(Problem::error(n, format!("{} is a picture or font with no licence noted: set `meta.license` for what it ships.", f.path)));
                }
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                if !path.exists() {
                    out.push(Problem::error(n, format!("{} isn't there.", f.path)));
                }
                continue;
            };
            for host in OTHER_BRANDS_HOSTS {
                if let Some(at) = text.find(host) {
                    let at = text[..at].rfind(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(')).map_or(0, |i| i + 1);
                    let url: String = text[at..].chars().take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '`' | ')')).collect();
                    let picture = [".png", ".jpg", ".jpeg", ".svg", ".webp", "avatars", "images."].iter().any(|x| url.contains(x));
                    if picture {
                        out.push(Problem::error(n, format!("{} shows {url}: another brand's picture (shadcn's own examples use it) — use your own placeholder.", f.path)));
                    }
                }
            }
            for spec in imports(&text) {
                if let Some(pkg) = package(&spec) {
                    used.insert(pkg);
                } else if let Some(dep) = item_named_by(&spec)
                    && !own.contains(&dep)
                    && dep != item.name
                {
                    let declared_dep = item.registry_dependencies.iter().any(|d| d.trim_end_matches(".json").rsplit('/').next() == Some(dep.as_str()));
                    if !declared_dep {
                        out.push(Problem::error(n, format!("{} imports `{spec}` but `registryDependencies` doesn't name `{dep}`: a project installing it gets a broken import.", f.path)));
                    }
                }
            }
        }
        for pkg in &used {
            if !declared.contains(pkg.as_str()) && !PROVIDED.contains(&pkg.as_str()) {
                out.push(Problem::error(n, format!("imports `{pkg}` without declaring it in `dependencies`: a project installing it won't get it.")));
            }
        }
        for dep in &declared {
            if !used.contains(*dep) && !item.files.is_empty() && item.files.iter().all(|f| root.join(&f.path).is_file()) {
                out.push(Problem::warn(n, format!("declares `{dep}`, which none of its files imports.")));
            }
        }
        for dep in &item.registry_dependencies {
            if is_bare_name(dep) && reg.item(dep).is_some() {
                out.push(Problem::error(n, format!(
                    "needs `{dep}` by its bare name, which installs shadcn's own `{dep}`, not this registry's: write {}",
                    reg.homepage.as_deref().map_or_else(|| format!("`@{}/{dep}`", reg.name), |h| format!("`{}/r/{dep}.json` or `@{}/{dep}`", h.trim_end_matches('/'), reg.name))
                )));
            } else if is_bare_name(dep) {
                out.push(Problem::warn(n, format!("needs `{dep}` from shadcn's own registry: it comes unstyled by your tokens — add your own `{dep}` and point to it.")));
            } else if (dep.starts_with(&format!("@{}/", reg.name)) || dep.starts_with("http") && reg.homepage.as_deref().is_some_and(|h| dep.starts_with(h.trim_end_matches('/')))) && own_dependency(reg, dep).is_none() {
                out.push(Problem::error(n, format!("needs `{dep}`, which this registry doesn't have.")));
            }
        }
    }
    if let Some(built) = built {
        out.extend(check_built(reg, built));
    }
    out
}

/// What `shadcn build` wrote, against the registry: an item JSON for each
/// item, every file with its content, none older than its source.
pub fn check_built(reg: &Registry, built: &Path) -> Vec<Problem> {
    let mut out = Vec::new();
    let root = reg.root();
    for item in &reg.items {
        let n = Some(item.name.as_str());
        let file = built.join(format!("{}.json", item.name));
        let Some(built_item) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Item>(&t).ok()) else {
            out.push(Problem::error(n, format!("isn't built: no {} — run `shadcn build`.", file.display())));
            continue;
        };
        for f in &item.files {
            let Some(b) = built_item.files.iter().find(|b| b.path == f.path) else {
                out.push(Problem::error(n, format!("{} isn't in its build — run `shadcn build` again.", f.path)));
                continue;
            };
            if b.content.as_deref().is_none_or(|c| c.trim().is_empty()) {
                out.push(Problem::error(n, format!("{} was built empty.", f.path)));
                continue;
            }
            let src = std::fs::read_to_string(root.join(&f.path)).unwrap_or_default();
            if !src.is_empty() && b.content.as_deref().map(str::trim) != Some(src.trim()) {
                out.push(Problem::error(n, format!("{} changed after it was built — run `shadcn build` again.", f.path)));
            }
        }
    }
    out
}

/// What a sample install wrote and what it couldn't resolve.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Installed {
    /// The items installed, the one asked for first.
    pub items: Vec<String>,
    /// The files written, relative to the scratch project.
    pub files: Vec<String>,
    /// The packages a project would install with them.
    pub packages: Vec<String>,
    /// What it couldn't install, and why.
    pub unresolved: Vec<String>,
    /// Imports in what it wrote that point at nothing there.
    pub broken: Vec<String>,
}

impl Installed {
    pub fn resolves(&self) -> bool {
        self.unresolved.is_empty() && self.broken.is_empty()
    }
}

/// Where a built file lands in a project with `aliases` (from its
/// `components.json`): `registry:ui` in `ui`, `registry:lib` in `lib`, a
/// file or page at its `target`.
fn landing(f: &File, aliases: &[(String, String)]) -> String {
    let alias = |k: &str, fallback: &str| aliases.iter().find(|(a, _)| a == k).map_or(fallback, |(_, v)| v.as_str()).trim_start_matches("@/").trim_start_matches("~/").to_owned();
    let base = Path::new(&f.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(t) = f.target.as_deref().filter(|t| !t.is_empty()) {
        return t.trim_start_matches("~/").to_owned();
    }
    let dir = match f.kind.as_str() {
        "registry:ui" => alias("ui", "components/ui"),
        "registry:lib" => alias("lib", "lib"),
        "registry:hook" => alias("hooks", "hooks"),
        _ => alias("components", "components"),
    };
    format!("{dir}/{base}")
}

/// Install `name` from the built items in `built` into `project` (a scratch
/// folder), the way `npx shadcn add` would — offline: each item it needs
/// from this registry too, every file where the aliases put it; then every
/// alias import in what it wrote checked against what is there.
pub fn install(reg: &Registry, built: &Path, name: &str, project: &Path, aliases: &[(String, String)]) -> Installed {
    let mut done = Installed::default();
    let mut queue = vec![name.to_owned()];
    while let Some(next) = queue.pop() {
        if done.items.contains(&next) {
            continue;
        }
        let Some(item) = std::fs::read_to_string(built.join(format!("{next}.json"))).ok().and_then(|t| serde_json::from_str::<Item>(&t).ok()) else {
            done.unresolved.push(format!("`{next}`: no built item {next}.json"));
            continue;
        };
        done.items.push(next.clone());
        for f in &item.files {
            let Some(content) = &f.content else {
                done.unresolved.push(format!("`{next}`: {} has no content in its build", f.path));
                continue;
            };
            let to = landing(f, aliases);
            let path = project.join(&to);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if std::fs::write(&path, content).is_ok() {
                done.files.push(to);
            }
        }
        for pkg in item.dependencies.iter().map(|d| bare(d).to_owned()) {
            if !done.packages.contains(&pkg) {
                done.packages.push(pkg);
            }
        }
        for dep in &item.registry_dependencies {
            match own_dependency(reg, dep).or_else(|| is_bare_name(dep).then(|| reg.item(dep)).flatten()) {
                Some(i) if is_bare_name(dep) => done.unresolved.push(format!("`{next}` needs `{}` by its bare name: `npx shadcn add` would fetch shadcn's own, not this registry's", i.name)),
                Some(i) => queue.push(i.name.clone()),
                None if is_bare_name(dep) => done.unresolved.push(format!("`{next}` needs `{dep}` from shadcn's registry (fetched over the network, unstyled by your tokens)")),
                None => done.unresolved.push(format!("`{next}` needs `{dep}`, which isn't one of this registry's built items (another registry's, fetched over the network)")),
            }
        }
    }
    // Each alias import in what was written, against the files there.
    for file in &done.files {
        let text = std::fs::read_to_string(project.join(file)).unwrap_or_default();
        for spec in imports(&text) {
            let Some(rest) = spec.strip_prefix("@/").or_else(|| spec.strip_prefix("~/")) else { continue };
            let base = project.join(rest);
            let found = ["", ".ts", ".tsx", ".js", ".jsx", ".vue", ".svelte", "/index.ts", "/index.tsx", "/index.js"].iter().any(|ext| PathBuf::from(format!("{}{ext}", base.display())).is_file());
            if !found {
                done.broken.push(format!("{file} imports `{spec}`, which the install didn't bring"));
            }
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_and_packages_read_as_a_bundler_would() {
        let text = "import * as React from \"react\"\nimport { Slot } from '@radix-ui/react-slot'\nimport { cn } from \"@/lib/utils\"\n// import x from 'commented'\nconst y = require(\"clsx\")\nexport { z } from './z'\n";
        assert_eq!(imports(text), ["react", "@radix-ui/react-slot", "@/lib/utils", "clsx", "./z"]);
        assert_eq!(package("@radix-ui/react-dialog/dist/x").as_deref(), Some("@radix-ui/react-dialog"));
        assert_eq!(package("lucide-react").as_deref(), Some("lucide-react"));
        assert!(package("@/components/ui/button").is_none() && package("./x").is_none());
        assert_eq!(bare("@radix-ui/react-slot@^1.2"), "@radix-ui/react-slot");
        assert_eq!(bare("lucide-react@0.4"), "lucide-react");
        assert_eq!(item_named_by("@/registry/new-york/ui/dialog").as_deref(), Some("dialog"));
    }
}
