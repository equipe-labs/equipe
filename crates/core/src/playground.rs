//! "Try it" on a reference card: a live, sandboxed playground of a registry
//! component (21st.dev, the shadcn registry) the person clicks, hovers and
//! types in before deciding — opened as a tab of their own in equipe's shared
//! browser, so a screen attached from another machine sees it through the
//! browser panel as any page.
//!
//! - **What runs.** The daemon reads the component's registry item (shadcn's
//!   format: its files' code, its declared `dependencies` and the registry
//!   components it uses, a level or two deep) with [`crate::references::fetch_public`]'s
//!   rules, and writes one page: the code transpiled in the page (Babel),
//!   React and each declared dependency from esm.sh at a pinned version
//!   ([`pinned`]), Tailwind's browser build at a pinned version, the
//!   project's design tokens as CSS variables when it has some (else the
//!   registry's default theme), and a bar to switch the component's exports
//!   (its variants), light and dark, and the width.
//! - **The sandbox.** The page is served by the daemon on `127.0.0.1` only, at
//!   a path with a random token ([`Server`]), and carries a strict CSP: scripts
//!   from itself, `blob:` and the three pinned CDNs only, no fetch or
//!   websocket anywhere (`connect-src 'none'`), no frames, forms or outside
//!   pictures. Nothing in it knows a secret; nothing is written to a project.
//!   Third-party code goes into a project only when the person adds it there
//!   themselves.
//! - **Not a registry item** (a Mobbin screen, a page with no registry
//!   behind it, a dependency with no pinned version): the source page opens
//!   instead, to look at.

use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::protocol::references::RefCard;

/// The biggest registry item read.
pub const ITEM_MAX: usize = 1 << 20;
/// Registry items read for one page, at most (the component and what it uses).
pub const ITEMS: usize = 10;
/// Pages kept, at most: the oldest go.
pub const PAGES: usize = 64;

/// The pinned scripts every page loads.
pub const REACT: &str = "18.3.1";
pub const BABEL: &str = "https://unpkg.com/@babel/standalone@7.26.4/babel.min.js";
pub const TAILWIND: &str = "https://cdn.jsdelivr.net/npm/@tailwindcss/browser@4.1.11/dist/index.global.js";

/// The page's content security policy: only the pinned CDNs, nothing fetched.
pub const CSP: &str = "default-src 'none'; script-src 'self' 'unsafe-inline' 'unsafe-eval' blob: https://esm.sh https://unpkg.com https://cdn.jsdelivr.net; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'";

/// The version each known package is loaded at; one not here (and not
/// written `name@x.y.z` by its registry) isn't loaded, and the page says so.
const PINNED: [(&str, &str); 44] = [
    ("clsx", "2.1.1"),
    ("tailwind-merge", "3.0.2"),
    ("class-variance-authority", "0.7.1"),
    ("lucide-react", "0.468.0"),
    ("radix-ui", "1.1.2"),
    ("@radix-ui/react-slot", "1.1.1"),
    ("@radix-ui/react-dialog", "1.1.4"),
    ("@radix-ui/react-dropdown-menu", "2.1.4"),
    ("@radix-ui/react-popover", "1.1.4"),
    ("@radix-ui/react-tabs", "1.1.2"),
    ("@radix-ui/react-tooltip", "1.1.6"),
    ("@radix-ui/react-switch", "1.1.2"),
    ("@radix-ui/react-checkbox", "1.1.3"),
    ("@radix-ui/react-label", "2.1.1"),
    ("@radix-ui/react-select", "2.1.4"),
    ("@radix-ui/react-separator", "1.1.1"),
    ("@radix-ui/react-avatar", "1.1.2"),
    ("@radix-ui/react-accordion", "1.2.2"),
    ("@radix-ui/react-toggle", "1.1.1"),
    ("@radix-ui/react-toggle-group", "1.1.1"),
    ("@radix-ui/react-radio-group", "1.2.2"),
    ("@radix-ui/react-slider", "1.2.2"),
    ("@radix-ui/react-scroll-area", "1.2.2"),
    ("@radix-ui/react-progress", "1.1.1"),
    ("@radix-ui/react-hover-card", "1.1.4"),
    ("@radix-ui/react-collapsible", "1.1.2"),
    ("@radix-ui/react-navigation-menu", "1.2.3"),
    ("@radix-ui/react-menubar", "1.1.4"),
    ("@radix-ui/react-context-menu", "2.2.4"),
    ("@radix-ui/react-alert-dialog", "1.1.4"),
    ("@radix-ui/react-aspect-ratio", "1.1.1"),
    ("@radix-ui/react-icons", "1.3.2"),
    ("framer-motion", "11.15.0"),
    ("motion", "11.15.0"),
    ("cmdk", "1.0.4"),
    ("sonner", "1.7.1"),
    ("vaul", "1.1.2"),
    ("embla-carousel-react", "8.5.1"),
    ("date-fns", "4.1.0"),
    ("react-day-picker", "9.4.4"),
    ("recharts", "2.15.0"),
    ("input-otp", "1.4.1"),
    ("next-themes", "0.4.4"),
    ("react-resizable-panels", "2.1.7"),
];

/// A dependency as a registry declares it (`name`, `name@1.2.3`), as an
/// esm.sh address at a pinned version — React kept outside, so the page's one
/// copy serves it. `None`: no version to pin it at.
pub fn pinned(dep: &str) -> Option<(String, String)> {
    let dep = dep.trim();
    let (name, version) = match dep.rfind('@').filter(|&at| at > 0) {
        Some(at) => (&dep[..at], Some(&dep[at + 1..])),
        None => (dep, None),
    };
    let exact = |v: &str| !v.is_empty() && v.split('.').count() == 3 && v.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    let version = version.filter(|v| exact(v)).map(str::to_owned).or_else(|| PINNED.iter().find(|(n, _)| *n == name).map(|(_, v)| (*v).to_owned()))?;
    let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '/' | '-' | '.' | '_'));
    plain.then(|| (name.to_owned(), format!("https://esm.sh/{name}@{version}?external=react,react-dom")))
}

/// The registry item addresses a card's link may stand for, the demo first:
/// a shadcn component's page (`ui.shadcn.com/docs/components/<name>`) and a
/// 21st.dev component's (`21st.dev/<author>/<slug>…`). Empty: no registry.
pub fn registry_urls(card: &RefCard) -> Vec<String> {
    if card.study_only() {
        return Vec::new();
    }
    let host = card.host();
    let path: Vec<&str> = card.link.trim().split("://").nth(1).unwrap_or_default().split(['?', '#']).next().unwrap_or_default().split('/').skip(1).filter(|p| !p.is_empty()).collect();
    let clean = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    match host.as_str() {
        "ui.shadcn.com" => {
            let name = match path.as_slice() {
                ["docs", "components", name, ..] => name.trim_end_matches(".json"),
                [.., last] if last.ends_with(".json") => last.trim_end_matches(".json"),
                _ => return Vec::new(),
            };
            if !clean(name) {
                return Vec::new();
            }
            let base = "https://ui.shadcn.com/r/styles/new-york-v4";
            vec![format!("{base}/{name}-demo.json"), format!("{base}/{name}.json")]
        }
        "21st.dev" => match path.as_slice() {
            ["r", author, slug, ..] | [author, slug, ..] if clean(author) && clean(slug) && !matches!(*author, "r" | "s" | "docs" | "community") => vec![format!("https://21st.dev/r/{author}/{slug}")],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// One file of a registry item, as the page loads it: by the stem other
/// files import it by (`button` for `@/components/ui/button`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct File {
    pub stem: String,
    pub code: String,
    /// What it shows (the demo, the component): the page renders it.
    pub entry: bool,
}

/// What a registry item holds: its files, the packages it declares and the
/// registry components it uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    pub files: Vec<File>,
    pub dependencies: Vec<String>,
    pub uses: Vec<String>,
}

/// A registry item (shadcn's JSON format), or `None`.
pub fn parse_item(json: &str) -> Option<Item> {
    let v: Value = serde_json::from_str(json).ok()?;
    let strs = |key: &str| v[key].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default();
    let files: Vec<File> = v["files"]
        .as_array()?
        .iter()
        .filter_map(|f| {
            let path = f["path"].as_str().or_else(|| f["target"].as_str())?;
            let code = f["content"].as_str()?.to_owned();
            let name = path.rsplit('/').next()?;
            let stem = name.split('.').next()?.to_owned();
            let kind = f["type"].as_str().unwrap_or_default();
            (name.ends_with(".tsx") || name.ends_with(".ts") || name.ends_with(".jsx") || name.ends_with(".js")).then(|| File { stem, code, entry: kind == "registry:example" || kind == "registry:block" || name.contains("demo") })
        })
        .collect();
    (!files.is_empty()).then(|| Item { files, dependencies: strs("dependencies"), uses: strs("registryDependencies") })
}

/// Where a registry component another item uses is read from: an address as
/// it is, a name from the shadcn registry.
fn used_url(used: &str) -> Option<String> {
    let used = used.trim();
    if used.starts_with("https://") {
        return crate::references::check_link(used).ok();
    }
    used.chars().all(|c| c.is_ascii_alphanumeric() || c == '-').then(|| format!("https://ui.shadcn.com/r/styles/new-york-v4/{used}.json"))
}

/// The project's design tokens (`design/tokens.json`, plain values or
/// `value`/`$value` leaves) as CSS variables over the registry's theme: each
/// token whose last name is one of the theme's (`primary`, `background`,
/// `radius`…) sets it.
pub fn tokens_css(tokens: &str) -> String {
    const NAMES: [&str; 19] = ["background", "foreground", "card", "card-foreground", "popover", "popover-foreground", "primary", "primary-foreground", "secondary", "secondary-foreground", "muted", "muted-foreground", "accent", "accent-foreground", "destructive", "border", "input", "ring", "radius"];
    fn walk(v: &Value, path: &mut Vec<String>, out: &mut Vec<(String, String)>) {
        match v {
            Value::String(s) => out.push((path.join("-"), s.clone())),
            Value::Number(n) => out.push((path.join("-"), n.to_string())),
            Value::Object(map) => {
                if let Some(leaf) = map.get("$value").or_else(|| map.get("value")).filter(|l| l.is_string() || l.is_number()) {
                    return walk(leaf, path, out);
                }
                for (k, child) in map {
                    path.push(k.to_ascii_lowercase());
                    walk(child, path, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let Ok(v) = serde_json::from_str::<Value>(tokens) else { return String::new() };
    let mut flat = Vec::new();
    walk(&v, &mut Vec::new(), &mut flat);
    let safe = |s: &str| s.len() <= 120 && !s.contains([';', '{', '}', '<', '>', '\\']);
    let set: Vec<String> = flat.into_iter().filter_map(|(path, value)| NAMES.iter().find(|n| path == **n || path.ends_with(&format!("-{n}"))).filter(|_| safe(&value)).map(|n| format!("--{n}: {value};"))).collect();
    if set.is_empty() { String::new() } else { format!(":root {{ {} }}", set.join(" ")) }
}

/// The registry's default theme (shadcn's neutral), light and dark, and the
/// Tailwind theme that reads it.
const THEME: &str = r#"@custom-variant dark (&:is(.dark *));
@theme inline {
  --color-background: var(--background); --color-foreground: var(--foreground);
  --color-card: var(--card); --color-card-foreground: var(--card-foreground);
  --color-popover: var(--popover); --color-popover-foreground: var(--popover-foreground);
  --color-primary: var(--primary); --color-primary-foreground: var(--primary-foreground);
  --color-secondary: var(--secondary); --color-secondary-foreground: var(--secondary-foreground);
  --color-muted: var(--muted); --color-muted-foreground: var(--muted-foreground);
  --color-accent: var(--accent); --color-accent-foreground: var(--accent-foreground);
  --color-destructive: var(--destructive); --color-border: var(--border); --color-input: var(--input); --color-ring: var(--ring);
  --radius-sm: calc(var(--radius) - 4px); --radius-md: calc(var(--radius) - 2px); --radius-lg: var(--radius); --radius-xl: calc(var(--radius) + 4px);
}
:root { --radius: 0.625rem; --background: oklch(1 0 0); --foreground: oklch(0.145 0 0); --card: oklch(1 0 0); --card-foreground: oklch(0.145 0 0); --popover: oklch(1 0 0); --popover-foreground: oklch(0.145 0 0); --primary: oklch(0.205 0 0); --primary-foreground: oklch(0.985 0 0); --secondary: oklch(0.97 0 0); --secondary-foreground: oklch(0.205 0 0); --muted: oklch(0.97 0 0); --muted-foreground: oklch(0.556 0 0); --accent: oklch(0.97 0 0); --accent-foreground: oklch(0.205 0 0); --destructive: oklch(0.577 0.245 27.325); --border: oklch(0.922 0 0); --input: oklch(0.922 0 0); --ring: oklch(0.708 0 0); }
.dark { --background: oklch(0.145 0 0); --foreground: oklch(0.985 0 0); --card: oklch(0.205 0 0); --card-foreground: oklch(0.985 0 0); --popover: oklch(0.205 0 0); --popover-foreground: oklch(0.985 0 0); --primary: oklch(0.922 0 0); --primary-foreground: oklch(0.205 0 0); --secondary: oklch(0.269 0 0); --secondary-foreground: oklch(0.985 0 0); --muted: oklch(0.269 0 0); --muted-foreground: oklch(0.708 0 0); --accent: oklch(0.269 0 0); --accent-foreground: oklch(0.985 0 0); --destructive: oklch(0.704 0.191 22.216); --border: oklch(1 0 0 / 10%); --input: oklch(1 0 0 / 15%); --ring: oklch(0.556 0 0); }
* { border-color: var(--border); }
body { background: var(--background); color: var(--foreground); margin: 0; font-family: ui-sans-serif, system-ui, sans-serif; }"#;

/// What runs in the page: each file transpiled (TypeScript and JSX), its
/// `@/…` imports pointed at the others by stem (`@/lib/utils` is `cn`), the
/// entry's exported components offered as variants.
const RUNNER: &str = r#"
const files = JSON.parse(document.getElementById('eq-files').textContent);
const out = document.getElementById('eq-root');
const bar = document.getElementById('eq-variants');
const fail = (why) => { document.getElementById('eq-error').textContent = why; };
const utils = `import { clsx } from "clsx"; import { twMerge } from "tailwind-merge"; export function cn(...a) { return twMerge(clsx(a)); }`;
const made = {};
function load(stem, seen = []) {
  if (made[stem]) return made[stem];
  const file = stem === 'utils' && !files.some(f => f.stem === 'utils') ? { stem, code: utils } : files.find(f => f.stem === stem);
  if (!file) throw new Error(`the code for "${stem}" isn't in the registry item`);
  if (seen.includes(stem)) throw new Error(`"${stem}" imports itself`);
  let code = Babel.transform(file.code, { filename: stem + '.tsx', presets: [['typescript', { isTSX: true, allExtensions: true }], ['react', { runtime: 'automatic' }]] }).code;
  code = code.replace(/(from\s*|import\s*\(\s*|import\s+)(["'])@\/([^"']+)\2/g, (_, head, q, path) => head + q + load(path.split('/').pop().replace(/\.(t|j)sx?$/, ''), [...seen, stem]) + q);
  made[stem] = URL.createObjectURL(new Blob([code], { type: 'text/javascript' }));
  return made[stem];
}
(async () => {
  try {
    const entry = files.find(f => f.entry) || files[files.length - 1];
    const mod = await import(load(entry.stem));
    const React = await import('react');
    const { createRoot } = await import('react-dom/client');
    const shown = Object.entries(mod).filter(([, v]) => typeof v === 'function' || (v && typeof v === 'object' && v.$$typeof));
    if (!shown.length) throw new Error('the component exports nothing to show');
    shown.sort(([a], [b]) => (a === 'default' ? -1 : b === 'default' ? 1 : 0));
    const root = createRoot(out);
    const show = (i) => {
      bar.querySelectorAll('button').forEach((b, k) => b.setAttribute('aria-pressed', String(k === i)));
      root.render(React.createElement(shown[i][1]));
    };
    shown.forEach(([name], i) => { const b = document.createElement('button'); b.textContent = name === 'default' ? entry.stem : name; b.onclick = () => show(i); bar.appendChild(b); });
    show(0);
  } catch (e) { fail(String(e && e.message || e)); }
})();
document.getElementById('eq-dark').onclick = () => document.documentElement.classList.toggle('dark');
document.getElementById('eq-narrow').onclick = () => out.classList.toggle('eq-narrow');
"#;

/// Text as it may sit inside an HTML element or a `<script type="application/json">`.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The playground page of `card`: its files, its pinned packages, the
/// project's tokens (CSS from [`tokens_css`], or empty) over the default
/// theme. What can't be loaded (a package with no pinned version) is said on
/// the page.
pub fn page(card: &RefCard, files: &[File], dependencies: &[String], tokens: &str) -> String {
    let mut imports: Vec<(String, String)> = vec![
        ("react".into(), format!("https://esm.sh/react@{REACT}")),
        ("react/".into(), format!("https://esm.sh/react@{REACT}/")),
        ("react-dom".into(), format!("https://esm.sh/react-dom@{REACT}")),
        ("react-dom/".into(), format!("https://esm.sh/react-dom@{REACT}/")),
    ];
    let mut missing: Vec<String> = Vec::new();
    for dep in ["clsx", "tailwind-merge"].iter().map(|d| (*d).to_owned()).chain(dependencies.iter().cloned()) {
        match pinned(&dep) {
            Some((name, url)) if !imports.iter().any(|(n, _)| *n == name) => {
                imports.push((name, url));
            }
            Some(_) => {}
            None if !matches!(dep.as_str(), "react" | "react-dom") => missing.push(dep),
            None => {}
        }
    }
    let map = serde_json::json!({ "imports": imports.into_iter().collect::<serde_json::Map<String, Value>>() });
    let files_json = serde_json::to_string(files).unwrap_or_else(|_| "[]".into());
    let missing = if missing.is_empty() { String::new() } else { format!("<p class=\"eq-note\">Not loaded (no pinned version): {}</p>", escape(&missing.join(", "))) };
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="{CSP}">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — try it</title>
<script type="importmap">{map}</script>
<script src="{BABEL}"></script>
<script src="{TAILWIND}"></script>
<style type="text/tailwindcss">{THEME}
{tokens}</style>
<style>
#eq-bar {{ position: sticky; top: 0; display: flex; flex-wrap: wrap; gap: 6px; align-items: center; padding: 8px 12px; border-bottom: 1px solid var(--border); background: var(--background); font: 12px ui-sans-serif, system-ui; z-index: 10; }}
#eq-bar button {{ font: inherit; padding: 3px 9px; border-radius: 999px; border: 1px solid var(--border); background: transparent; color: var(--foreground); cursor: pointer; }}
#eq-bar button[aria-pressed="true"] {{ background: var(--foreground); color: var(--background); }}
#eq-bar .eq-title {{ font-weight: 600; margin-right: 6px; }}
#eq-root {{ padding: 40px 24px; display: flex; justify-content: center; }}
#eq-root.eq-narrow {{ max-width: 390px; margin: 0 auto; }}
#eq-error, .eq-note {{ color: var(--destructive); font: 12px ui-monospace, monospace; padding: 0 12px; white-space: pre-wrap; }}
</style></head>
<body>
<div id="eq-bar"><span class="eq-title">{title}</span><span id="eq-variants"></span><span style="flex:1"></span><button id="eq-dark">Light / dark</button><button id="eq-narrow">Narrow</button></div>
{missing}<pre id="eq-error"></pre>
<div id="eq-root"></div>
<script type="application/json" id="eq-files">{files}</script>
<script type="module">{RUNNER}</script>
</body></html>"#,
        title = escape(&card.title),
        map = map.to_string().replace("</", "<\\/"),
        files = files_json.replace('<', "\\u003c"),
    )
}

/// The playground page of `card`, its registry items read (the component,
/// then those it uses), or why there is none — then its source page opens.
pub async fn build(card: &RefCard, tokens: &str) -> Result<String, String> {
    let urls = registry_urls(card);
    if urls.is_empty() {
        return Err("it has no registry item to run".into());
    }
    let mut first = None;
    for url in &urls {
        if let Ok(bytes) = crate::references::fetch_public(url).await
            && bytes.len() <= ITEM_MAX
            && let Some(item) = parse_item(&String::from_utf8_lossy(&bytes))
        {
            first = Some(item);
            break;
        }
    }
    let first = first.ok_or("its registry item couldn't be read")?;
    let (mut files, mut deps) = (first.files.clone(), first.dependencies.clone());
    let mut queue: Vec<String> = first.uses.clone();
    let mut read = 1;
    while let Some(used) = queue.pop() {
        if read >= ITEMS {
            break;
        }
        let Some(url) = used_url(&used) else { continue };
        read += 1;
        let Ok(bytes) = crate::references::fetch_public(&url).await else { continue };
        let Some(item) = parse_item(&String::from_utf8_lossy(&bytes[..bytes.len().min(ITEM_MAX)])) else { continue };
        for mut f in item.files {
            if !files.iter().any(|g| g.stem == f.stem) {
                f.entry = false;
                files.push(f);
            }
        }
        deps.extend(item.dependencies);
        queue.extend(item.uses);
    }
    if !files.iter().any(|f| f.entry) {
        if let Some(f) = files.first_mut() {
            f.entry = true;
        }
    }
    Ok(page(card, &files, &deps, tokens))
}

/// The daemon's playground server: `127.0.0.1` only, each page at a path
/// with a token no other page knows.
#[derive(Clone)]
pub struct Server {
    port: u16,
    token: String,
    pages: Arc<Mutex<Vec<(String, String)>>>,
}

static SERVER: tokio::sync::OnceCell<Option<Server>> = tokio::sync::OnceCell::const_new();

/// A random token, from the system's randomness.
fn token() -> String {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0u8; 16];
    let _ = ring::rand::SystemRandom::new().fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Server {
    /// The server, started on first use; `None` when it couldn't listen.
    pub async fn get() -> Option<Server> {
        SERVER.get_or_init(|| async { Server::start().await.ok() }).await.clone()
    }

    async fn start() -> std::io::Result<Server> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let server = Server { port: listener.local_addr()?.port(), token: token(), pages: Arc::default() };
        let serving = server.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(serving.clone().answer(socket));
            }
        });
        Ok(server)
    }

    /// Keep `html` as a page; its address.
    pub fn put(&self, html: String) -> String {
        let key = token();
        if let Ok(mut pages) = self.pages.lock() {
            pages.push((key.clone(), html));
            let over = pages.len().saturating_sub(PAGES);
            pages.drain(..over);
        }
        format!("http://127.0.0.1:{}/{}/{key}", self.port, self.token)
    }

    /// The page asked for, by its path: `GET /<token>/<key>`, nothing else.
    fn page_at(&self, path: &str) -> Option<String> {
        let rest = path.strip_prefix('/')?.strip_prefix(self.token.as_str())?.strip_prefix('/')?;
        self.pages.lock().ok()?.iter().find(|(k, _)| k == rest).map(|(_, html)| html.clone())
    }

    async fn answer(self, mut socket: tokio::net::TcpStream) {
        let mut buf = vec![0u8; 8192];
        let Ok(Ok(n)) = tokio::time::timeout(std::time::Duration::from_secs(10), socket.read(&mut buf)).await else { return };
        let head = String::from_utf8_lossy(&buf[..n]);
        let mut words = head.lines().next().unwrap_or_default().split(' ');
        let found = match (words.next(), words.next()) {
            (Some("GET"), Some(path)) => self.page_at(path),
            _ => None,
        };
        let (status, body) = match found {
            Some(html) => ("200 OK", html),
            None => ("404 Not Found", "Not here.".to_owned()),
        };
        let reply = format!(
            "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-security-policy: {CSP}\r\ncache-control: no-store\r\nx-content-type-options: nosniff\r\nreferrer-policy: no-referrer\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(reply.as_bytes()).await;
        let _ = socket.shutdown().await;
    }
}

/// Pages by key, for tests.
#[cfg(test)]
fn keys(server: &Server) -> std::collections::HashMap<String, usize> {
    server.pages.lock().unwrap().iter().map(|(k, h)| (k.clone(), h.len())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::references::RefKind;

    fn card(link: &str) -> RefCard {
        RefCard { title: "Button <b>".into(), link: link.into(), site: "x".into(), kind: RefKind::Component, ..RefCard::default() }
    }

    #[test]
    fn a_registry_components_page_stands_for_its_registry_item_and_a_study_screen_for_none() {
        assert_eq!(registry_urls(&card("https://ui.shadcn.com/docs/components/button")), ["https://ui.shadcn.com/r/styles/new-york-v4/button-demo.json", "https://ui.shadcn.com/r/styles/new-york-v4/button.json"]);
        assert_eq!(registry_urls(&card("https://21st.dev/aceternity/hero-parallax/default")), ["https://21st.dev/r/aceternity/hero-parallax"]);
        assert_eq!(registry_urls(&card("https://21st.dev/r/shadcn/accordion")), ["https://21st.dev/r/shadcn/accordion"]);
        assert!(registry_urls(&card("https://mobbin.com/screens/1")).is_empty(), "study material: its page, to look at");
        assert!(registry_urls(&card("https://21st.dev/s/hero")).is_empty());
        assert!(registry_urls(&card("https://example.com/c/button")).is_empty());
        assert!(registry_urls(&card("https://ui.shadcn.com/docs/components/a%2f..")).is_empty());
    }

    #[test]
    fn only_pinned_packages_load_and_react_stays_one_copy() {
        assert_eq!(pinned("@radix-ui/react-slot"), Some(("@radix-ui/react-slot".into(), "https://esm.sh/@radix-ui/react-slot@1.1.1?external=react,react-dom".into())));
        assert_eq!(pinned("left-pad@1.3.0").unwrap().1, "https://esm.sh/left-pad@1.3.0?external=react,react-dom");
        assert_eq!(pinned("left-pad"), None, "no version to pin it at");
        assert_eq!(pinned("left-pad@^1.3.0"), None, "a range isn't a pin");
        assert_eq!(pinned("evil\"pkg@1.0.0"), None);
    }

    #[test]
    fn a_registry_item_gives_its_files_by_stem_and_what_it_uses() {
        let item = parse_item(r#"{"name":"button-demo","dependencies":["@radix-ui/react-slot"],"registryDependencies":["button"],"files":[{"path":"registry/new-york-v4/examples/button-demo.tsx","type":"registry:example","content":"export default function ButtonDemo(){return null}"},{"path":"x/readme.md","content":"# no"}]}"#).unwrap();
        assert_eq!(item.files, vec![File { stem: "button-demo".into(), code: "export default function ButtonDemo(){return null}".into(), entry: true }]);
        assert_eq!((item.dependencies, item.uses), (vec!["@radix-ui/react-slot".to_owned()], vec!["button".to_owned()]));
        assert_eq!(parse_item(r#"{"files":[]}"#), None);
        assert_eq!(used_url("button").as_deref(), Some("https://ui.shadcn.com/r/styles/new-york-v4/button.json"));
        assert_eq!(used_url("http://evil.example.com/x.json"), None);
    }

    #[test]
    fn the_projects_tokens_set_the_theme_and_nothing_breaks_out() {
        let css = tokens_css(r##"{"color":{"primary":{"$value":"#5b5bd6"},"background":"#fff","brand":"red"},"radius":{"value":"8px"},"evil":{"ring":"red; } body { x"}}"##);
        assert!(css.contains("--primary: #5b5bd6;") && css.contains("--background: #fff;") && css.contains("--radius: 8px;"), "{css}");
        assert!(!css.contains("brand") && !css.contains("body"), "{css}");
        assert_eq!(tokens_css("not json"), "");
    }

    #[test]
    fn the_page_is_sandboxed_and_its_code_and_title_cant_close_its_tags() {
        let files = vec![File { stem: "demo".into(), code: "const s = \"</script><script>alert(1)</script>\"; export default () => null".into(), entry: true }];
        let html = page(&card("https://ui.shadcn.com/docs/components/button"), &files, &["lucide-react".into(), "left-pad".into()], ":root { --primary: red; }");
        assert!(html.contains(CSP) && CSP.contains("connect-src 'none'") && CSP.contains("img-src data: blob:"));
        assert!(!html.contains("</script><script>alert(1)"), "the code stays data");
        let start = html.find(r#"id="eq-files">"#).unwrap() + r#"id="eq-files">"#.len();
        let json = &html[start..start + html[start..].find("</script>").unwrap()];
        let back: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert_eq!(back[0]["code"], files[0].code, "read back whole");
        assert!(html.contains("Button &lt;b&gt;"));
        assert!(html.contains("https://esm.sh/lucide-react@0.468.0?external=react,react-dom") && html.contains(&format!("https://esm.sh/react@{REACT}")));
        assert!(html.contains("Not loaded (no pinned version): left-pad"));
        assert!(html.contains(":root { --primary: red; }"));
    }

    #[tokio::test]
    async fn the_server_answers_only_its_pages_on_loopback() {
        let server = Server::start().await.unwrap();
        let url = server.put("<p>hi</p>".into());
        assert!(url.starts_with("http://127.0.0.1:"));
        let client = reqwest::Client::new();
        let got = client.get(&url).send().await.unwrap();
        assert_eq!(got.status().as_u16(), 200);
        assert!(got.headers()["content-security-policy"].to_str().unwrap().contains("connect-src 'none'"));
        assert_eq!(got.text().await.unwrap(), "<p>hi</p>");
        let wrong = url.replace(&server.token, "0000");
        assert_eq!(client.get(&wrong).send().await.unwrap().status().as_u16(), 404);
        for _ in 0..PAGES + 3 {
            server.put("x".into());
        }
        assert_eq!(keys(&server).len(), PAGES, "the oldest go");
        assert_eq!(client.get(&url).send().await.unwrap().status().as_u16(), 404);
    }
}
