//! Brands a project inherits its design system from (`docs/BRANDS.md`).
//!
//! A brand is a machine-level folder, `~/.equipe/brands/<slug>/`, several
//! squads may share: `brand.json` (`{slug, name, version}`), its
//! `design/brand.md`, `design/tokens/*.json` (DTCG), `design/components/`,
//! `taste.md`, `refs.md`. Each released version is kept whole under
//! `versions/<version>/`, never changed again. A squad lists the brands it
//! uses (`Journal::brands`).
//!
//! A project opts in with `design/brand.json` (`{brand, pin}`); its
//! `design/tokens/*.json` then holds only overrides. Its system is the
//! brand's tokens with the overrides laid over them, path by path, the
//! project winning — what `tokens.json` is read as wherever equipe reads it
//! (the design screen, `check`, the lint, the system found in code), and
//! what `equipe design emit` writes. Without `design/brand.json` nothing
//! here runs: `tokens.json` is read as it is.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::folder::TOKENS;
use super::tokens;

/// The brands' folder in equipe's home.
pub const DIR: &str = "brands";
/// A brand's own file, and a project's link to one (in its `design/`).
pub const FILE: &str = "brand.json";
/// Where a brand keeps its released versions.
pub const VERSIONS: &str = "versions";
/// The biggest token file read from a brand or a project's overrides.
const FILE_MAX: u64 = 1 << 20;

/// `~/.equipe/brands/<slug>/brand.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Brand {
    pub slug: String,
    pub name: String,
    pub version: String,
}

/// `<project>/design/brand.json`: the brand, and the version the project
/// stays on; no pin follows the brand as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub brand: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
}

/// A project's system, resolved.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Resolved {
    pub link: Link,
    pub brand: Brand,
    /// The brand's folder.
    pub dir: PathBuf,
    /// The version read: the pin, else the brand's current one.
    pub version: String,
    /// The brand's current version, when the pin is behind it.
    pub newer: Option<String>,
    /// The resolved `tokens.json`.
    #[serde(skip)]
    pub text: String,
    /// Tokens as the brand has them.
    pub inherited: Vec<String>,
    /// The brand's tokens the project changes.
    pub overridden: Vec<String>,
    /// The project's tokens the brand doesn't have — new to it, or gone from the brand.
    pub added: Vec<String>,
    /// The override files, in the order laid.
    pub overrides: Vec<PathBuf>,
}

/// `~/.equipe/brands`.
pub fn root() -> PathBuf {
    equipe_providers::credentials::equipe_home().join(DIR)
}

/// A brand's name on disk: lower-case letters, digits and dashes.
pub fn valid_slug(slug: &str) -> bool {
    (1..=48).contains(&slug.len()) && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !slug.starts_with('-')
}

/// A version's name on disk.
fn valid_version(v: &str) -> bool {
    (1..=32).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') && !v.starts_with('.')
}

pub fn brand_dir(brands: &Path, slug: &str) -> PathBuf {
    brands.join(slug)
}

pub fn read_brand(brands: &Path, slug: &str) -> Result<Brand, String> {
    if !valid_slug(slug) {
        return Err(format!("`{slug}` isn't a brand name: lower-case letters, digits and dashes."));
    }
    let path = brand_dir(brands, slug).join(FILE);
    let text = std::fs::read_to_string(&path).map_err(|_| format!("No brand `{slug}` on this Mac ({} is missing).", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} can't be read: {e}", path.display()))
}

/// Every brand on this Mac, by slug.
pub fn list(brands: &Path) -> Vec<Brand> {
    let Ok(entries) = std::fs::read_dir(brands) else { return Vec::new() };
    let mut out: Vec<Brand> = entries.flatten().filter_map(|e| read_brand(brands, &e.file_name().to_string_lossy()).ok()).collect();
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

/// The versions a brand has released, oldest first as far as numbers go.
pub fn versions(brands: &Path, slug: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(brand_dir(brands, slug).join(VERSIONS)) else { return Vec::new() };
    let mut out: Vec<String> = entries.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|v| valid_version(v)).collect();
    out.sort_by_key(|v| version_key(v));
    out
}

fn version_key(v: &str) -> Vec<(u64, String)> {
    v.split('.').map(|p| (p.parse().unwrap_or(u64::MAX), p.to_owned())).collect()
}

/// The project's link, when `design` has a `brand.json`: `None` without one.
pub fn link_of(design: &Path) -> Option<Result<Link, String>> {
    let path = design.join(FILE);
    if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
        return None;
    }
    Some(std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Link>(&t).map_err(|e| format!("{} can't be read: {e}", path.display()))))
}

fn read_capped(path: &Path) -> Option<String> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(FILE_MAX).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The token files in a `design/` folder's `tokens/`, sorted by name.
fn layer_files(design: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(design.join("tokens")) else { return Vec::new() };
    let mut out: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json") && p.is_file()).collect();
    out.sort();
    out
}

/// A brand's tokens at `root` (its folder, or a version's): `design/tokens/*.json`, else `design/tokens.json`.
fn brand_layers(root: &Path) -> Vec<PathBuf> {
    let design = root.join(super::DIR);
    let files = layer_files(&design);
    if files.is_empty() && design.join(TOKENS).is_file() { vec![design.join(TOKENS)] } else { files }
}

/// The folder a link reads: the pinned version's, else the brand as it is.
fn source(brands: &Path, brand: &Brand, pin: Option<&str>) -> Result<PathBuf, String> {
    let dir = brand_dir(brands, &brand.slug);
    let Some(pin) = pin else { return Ok(dir) };
    if !valid_version(pin) {
        return Err(format!("`{pin}` isn't a version."));
    }
    let kept = dir.join(VERSIONS).join(pin);
    if kept.is_dir() {
        Ok(kept)
    } else if pin == brand.version {
        Ok(dir)
    } else {
        Err(format!("Brand `{}` has no version {pin} (it has {}).", brand.slug, versions(brands, &brand.slug).join(", ")))
    }
}

fn names(text: &str) -> Vec<String> {
    tokens::parse(text).tokens.into_iter().map(|t| t.name).collect()
}

/// The system of the project whose design folder is `design`, its brand
/// read from `brands`: `None` when it links none.
pub fn resolve(brands: &Path, design: &Path) -> Option<Result<Resolved, String>> {
    let link = match link_of(design)? {
        Ok(link) => link,
        Err(e) => return Some(Err(e)),
    };
    Some(resolve_link(brands, design, link))
}

fn resolve_link(brands: &Path, design: &Path, link: Link) -> Result<Resolved, String> {
    let brand = read_brand(brands, &link.brand)?;
    let from = source(brands, &brand, link.pin.as_deref())?;
    let read = |files: &[PathBuf]| -> Result<Vec<String>, String> { files.iter().map(|p| read_capped(p).ok_or_else(|| format!("{} can't be read.", p.display()))).collect() };
    let base = read(&brand_layers(&from))?;
    let overrides = layer_files(design);
    let over = read(&overrides)?;
    let all: Vec<&str> = base.iter().chain(&over).map(String::as_str).collect();
    let text = tokens::layer(&all).map_err(|(i, p)| {
        let file = brand_layers(&from).into_iter().chain(overrides.iter().cloned()).nth(i).unwrap_or_default();
        format!("{} can't be laid: {p:?}", file.display())
    })?;
    let brand_text = tokens::layer(&base.iter().map(String::as_str).collect::<Vec<_>>()).unwrap_or_default();
    let brand_names = names(&brand_text);
    let project_names: Vec<String> = over.iter().flat_map(|t| names(t)).collect();
    let mut overridden: Vec<String> = Vec::new();
    let mut added: Vec<String> = Vec::new();
    for n in project_names {
        let into = if brand_names.contains(&n) { &mut overridden } else { &mut added };
        if !into.contains(&n) {
            into.push(n);
        }
    }
    let inherited = brand_names.into_iter().filter(|n| !overridden.contains(n)).collect();
    let version = link.pin.clone().unwrap_or_else(|| brand.version.clone());
    let newer = (link.pin.is_some() && version != brand.version).then(|| brand.version.clone());
    Ok(Resolved { dir: brand_dir(brands, &brand.slug), link, brand, version, newer, text, inherited, overridden, added, overrides })
}

/// `tokens.json` in `design` as equipe reads it, brands at `brands`: the
/// resolved system when the project links a brand that resolves, else the
/// file as it is (`None` without one).
pub fn tokens_text_in(brands: &Path, design: &Path) -> Option<String> {
    match resolve(brands, design) {
        Some(Ok(r)) => Some(r.text),
        _ => std::fs::read_to_string(design.join(TOKENS)).ok(),
    }
}

/// [`tokens_text_in`] with this Mac's brands.
pub fn tokens_text(design: &Path) -> Option<String> {
    tokens_text_in(&root(), design)
}

/// Whether `path` is a design folder's `tokens.json` under a brand link —
/// read as the resolved system, not as the file.
pub fn linked_tokens_file(path: &Path) -> Option<&Path> {
    let dir = path.parent()?;
    (path.file_name().is_some_and(|n| n == TOKENS) && dir.join(FILE).is_file()).then_some(dir)
}

/// Whether `path` is one of a brand-linked project's override files: only a
/// part of its system, never read on its own.
pub fn is_override_file(path: &Path) -> bool {
    let Some(tokens_dir) = path.parent() else { return false };
    tokens_dir.file_name().is_some_and(|n| n == "tokens") && tokens_dir.parent().is_some_and(|d| d.join(FILE).is_file())
}

/// Make brand `slug`: its folder with brand.json, a brief, tokens (the ones
/// at `from` — a design folder or a tokens file — else the starter set),
/// components, taste and refs; version 1 released. Never over one that's there.
pub fn create(brands: &Path, slug: &str, name: &str, from: Option<&Path>) -> Result<Brand, String> {
    if !valid_slug(slug) {
        return Err(format!("`{slug}` isn't a brand name: lower-case letters, digits and dashes."));
    }
    let dir = brand_dir(brands, slug);
    if dir.exists() {
        return Err(format!("Brand `{slug}` is there already ({}).", dir.display()));
    }
    let design = dir.join(super::DIR);
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(design.join("tokens")).map_err(io)?;
    std::fs::create_dir_all(design.join("components")).map_err(io)?;
    let starter: Vec<(String, String)> = match from {
        None => vec![("tokens.json".into(), super::kit::STARTER_TOKENS.into())],
        Some(f) if f.is_file() => vec![("tokens.json".into(), std::fs::read_to_string(f).map_err(io)?)],
        Some(d) => {
            let mut out: Vec<(String, String)> = layer_files(d).iter().filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), std::fs::read_to_string(p).ok()?))).collect();
            if out.is_empty() {
                out.push(("tokens.json".into(), std::fs::read_to_string(d.join(TOKENS)).map_err(|_| format!("No tokens in {}.", d.display()))?));
            }
            out
        }
    };
    for (file, body) in starter {
        std::fs::write(design.join("tokens").join(file), body).map_err(io)?;
    }
    let brief = from.filter(|d| d.is_dir()).and_then(|d| std::fs::read_to_string(d.join(super::folder::BRIEF)).ok()).unwrap_or_else(|| format!("# {name}\n\nWho the brand is for, what it promises, how it sounds.\n"));
    std::fs::write(design.join(super::folder::BRIEF), brief).map_err(io)?;
    std::fs::write(dir.join("taste.md"), crate::taste::store::TEMPLATE).map_err(io)?;
    std::fs::write(dir.join("refs.md"), "# References\n\nLinks and what to learn from each — never another company's screenshots here.\n").map_err(io)?;
    let brand = Brand { slug: slug.into(), name: name.into(), version: "1".into() };
    write_brand(&dir, &brand)?;
    snapshot(&dir, "1")?;
    Ok(brand)
}

fn write_brand(dir: &Path, brand: &Brand) -> Result<(), String> {
    let text = serde_json::to_string_pretty(brand).map_err(|e| e.to_string())? + "\n";
    std::fs::write(dir.join(FILE), text).map_err(|e| e.to_string())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let (path, kind) = (entry.path(), entry.file_type()?);
        if kind.is_dir() {
            copy_dir(&path, &to.join(entry.file_name()))?;
        } else if kind.is_file() {
            std::fs::copy(&path, to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Keep the brand as it is now as `version`: its brand.json, design/, taste and refs.
fn snapshot(dir: &Path, version: &str) -> Result<(), String> {
    let kept = dir.join(VERSIONS).join(version);
    if kept.exists() {
        return Err(format!("Version {version} is kept already."));
    }
    let io = |e: std::io::Error| e.to_string();
    copy_dir(&dir.join(super::DIR), &kept.join(super::DIR)).map_err(io)?;
    for file in [FILE, "taste.md", "refs.md"] {
        if dir.join(file).is_file() {
            std::fs::copy(dir.join(file), kept.join(file)).map_err(io)?;
        }
    }
    Ok(())
}

/// Release the brand as it is now: kept as `version` (default: the next
/// whole number), which becomes its current one. Projects pinned to an
/// earlier version keep it.
pub fn release(brands: &Path, slug: &str, version: Option<&str>) -> Result<Brand, String> {
    let mut brand = read_brand(brands, slug)?;
    let next = match version {
        Some(v) if valid_version(v) => v.to_owned(),
        Some(v) => return Err(format!("`{v}` isn't a version.")),
        None => brand.version.split('.').next().and_then(|n| n.parse::<u64>().ok()).map(|n| (n + 1).to_string()).ok_or_else(|| format!("Say the version: `{}` isn't a whole number to count on from.", brand.version))?,
    };
    let dir = brand_dir(brands, slug);
    // The released brand.json says the version it is.
    let before = brand.clone();
    brand.version = next.clone();
    write_brand(&dir, &brand)?;
    if let Err(e) = snapshot(&dir, &next) {
        write_brand(&dir, &before)?;
        return Err(e);
    }
    Ok(brand)
}

/// Link the project whose design folder is `design` to brand `slug`, pinned
/// to `pin` (default: the brand's current version; `None` inside follows it).
pub fn link(brands: &Path, design: &Path, slug: &str, pin: Option<Option<&str>>) -> Result<Link, String> {
    let brand = read_brand(brands, slug)?;
    let pin = match pin {
        None => Some(brand.version.clone()),
        Some(p) => p.map(str::to_owned),
    };
    let link = Link { brand: slug.into(), pin };
    resolve_link(brands, design, link.clone())?;
    std::fs::create_dir_all(design).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(&link).map_err(|e| e.to_string())? + "\n";
    std::fs::write(design.join(FILE), text).map_err(|e| e.to_string())?;
    Ok(link)
}

/// Unlink the project: its `brand.json` goes; its overrides stay. Whether there was one.
pub fn unlink(design: &Path) -> Result<bool, String> {
    match link_of(design) {
        None => Ok(false),
        Some(_) => std::fs::remove_file(design.join(FILE)).map(|_| true).map_err(|e| e.to_string()),
    }
}

/// The squads on this Mac that list brand `slug`.
pub fn squads_using(home: &Path, slug: &str) -> Vec<String> {
    crate::squads::list(home)
        .into_iter()
        .filter(|s| crate::journal::Journal::open(&crate::journal::squad_dir(home, &s.name).join("squad.db")).is_ok_and(|j| j.brands().iter().any(|b| b == slug)))
        .map(|s| s.name)
        .collect()
}

/// What a member working on a brand-linked project is told: the system is
/// the brand's, changed only on a brand ticket — and on one, how.
pub fn prompt_note(brands: &Path, design: &Path, brand_ticket: bool) -> Option<String> {
    let r = match resolve(brands, design)? {
        Ok(r) => r,
        Err(e) => return Some(format!("\n\nThis project's `design/brand.json` links a brand that doesn't resolve: {e} Its own `design/tokens.json` is read meanwhile; say so on the ticket.")),
    };
    let pinned = match (&r.link.pin, &r.newer) {
        (Some(p), Some(n)) => format!("pinned to v{p}; a newer v{n} exists — move the pin only when a ticket asks"),
        (Some(p), None) => format!("pinned to v{p}, its current version"),
        (None, _) => format!("following its current version, v{}", r.version),
    };
    let mut note = format!(
        "\n\nThis project's design system is brand-owned: it inherits the brand **{}** (`{}`, {pinned}), kept at `{}` and shared by every project and squad that links it. \
`design/tokens/` here holds only this project's overrides ({} of the brand's {} tokens{}); `equipe design emit` gives the resolved tokens, and check, lint, contrast and the precheck run on them. \
`equipe design brand show` says which are inherited and which overridden.",
        r.brand.name,
        r.brand.slug,
        r.dir.display(),
        r.overridden.len(),
        r.inherited.len() + r.overridden.len(),
        if r.added.is_empty() { String::new() } else { format!(", plus {} of its own", r.added.len()) },
    );
    if brand_ticket {
        note.push_str(&format!(
            " This is a brand ticket: change the brand itself there (its `design/`, `taste.md`, `refs.md`), then `equipe design brand release {}` so projects can move to the new version; projects pinned to an older one keep it.",
            r.brand.slug
        ));
    } else {
        note.push_str(" Change the brand only on a brand ticket; on this one, change only this project's overrides — and only where this project must differ.");
    }
    Some(note)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::{check, lint};

    const BRAND_TOKENS: &str = r##"{
  "color": {
    "$type": "color",
    "background": { "$value": "#FFFFFF", "$extensions": { "mode": { "dark": "#0A0A0C" } } },
    "foreground": { "$value": "#111111", "$extensions": { "mode": { "dark": "#F5F5F5" } } },
    "primary": { "$value": "#1D4ED8" },
    "on-primary": { "$value": "#FFFFFF" }
  },
  "space": { "$type": "dimension", "2": { "$value": "8px" }, "4": { "$value": "16px" } }
}
"##;

    struct Mac {
        home: PathBuf,
        brands: PathBuf,
    }

    impl Drop for Mac {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    fn mac() -> Mac {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let home = std::env::temp_dir().join(format!("equipe-brands-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let brands = home.join(DIR);
        std::fs::create_dir_all(&brands).unwrap();
        Mac { home, brands }
    }

    fn brand(m: &Mac, slug: &str) -> PathBuf {
        let tokens = m.home.join("starter.tokens.json");
        std::fs::write(&tokens, BRAND_TOKENS).unwrap();
        create(&m.brands, slug, "Kettle", Some(&tokens)).unwrap();
        brand_dir(&m.brands, slug)
    }

    fn project(m: &Mac, name: &str) -> PathBuf {
        let design = m.home.join("projects").join(name).join("design");
        std::fs::create_dir_all(design.join("tokens")).unwrap();
        design
    }

    fn token(text: &str, name: &str) -> crate::protocol::design::Token {
        tokens::parse(text).tokens.into_iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{name}"))
    }

    #[test]
    fn tp6_a_linked_project_is_the_brand_with_its_one_override() {
        let m = mac();
        brand(&m, "kettle");
        let design = project(&m, "site");
        std::fs::write(design.join("tokens/overrides.json"), r##"{ "color": { "primary": { "$value": "#1E40AF" } } }"##).unwrap();
        link(&m.brands, &design, "kettle", None).unwrap();
        let emitted = tokens_text_in(&m.brands, &design).unwrap();
        let resolved = tokens::parse(&emitted);
        let brand_only = tokens::parse(BRAND_TOKENS);
        assert_eq!(resolved.problems, []);
        assert_eq!(token(&emitted, "color.primary").value, serde_json::json!("#1E40AF"), "the override shows");
        let rest = |p: &tokens::Parsed| p.tokens.iter().filter(|t| t.name != "color.primary").cloned().collect::<Vec<_>>();
        assert_eq!(rest(&resolved), rest(&brand_only), "everything else is the brand's");
        let r = resolve(&m.brands, &design).unwrap().unwrap();
        assert_eq!(r.overridden, ["color.primary"]);
        assert_eq!(r.inherited.len(), brand_only.tokens.len() - 1);
        assert!(r.added.is_empty() && r.newer.is_none() && r.version == "1");
        // The screen's reader, the lint's and the system found in code read it too.
        assert!(lint::System::read_in(&design, &m.brands).colours.iter().any(|(n, c)| n == "color.primary" && c.hex() == "#1E40AF"));
    }

    #[test]
    fn tp7_without_a_link_tokens_read_byte_for_byte_as_today() {
        let m = mac();
        brand(&m, "kettle");
        let design = project(&m, "app");
        let today = "{\n  \"color\": { \"bg\": { \"$value\": \"#fff\", \"$type\": \"color\" } }\n}\n";
        std::fs::write(design.join(TOKENS), today).unwrap();
        // An override-looking file means nothing without brand.json.
        std::fs::write(design.join("tokens/primitive.json"), r##"{ "color": { "bg": { "$value": "#000" } } }"##).unwrap();
        assert!(resolve(&m.brands, &design).is_none());
        assert_eq!(tokens_text_in(&m.brands, &design).as_deref(), Some(today));
        assert!(!is_override_file(&design.join("tokens/primitive.json")) && linked_tokens_file(&design.join(TOKENS)).is_none());
        assert_eq!(check::check_in(&design, &m.brands), check::check_in(&design, &m.home.join("no-brands")), "check is today's");
        assert!(prompt_note(&m.brands, &design, false).is_none(), "no prompt note");
    }

    #[test]
    fn tp8_squads_share_a_brand_and_a_pin_keeps_the_old_version() {
        let m = mac();
        let dir = brand(&m, "kettle");
        for squad in ["acme", "labs"] {
            crate::squads::create_without_project(&m.home, squad).unwrap();
            let j = crate::journal::Journal::open(&crate::journal::squad_dir(&m.home, squad).join("squad.db")).unwrap();
            j.set_brands(&["kettle".to_owned()]).unwrap();
        }
        assert_eq!(squads_using(&m.home, "kettle"), ["acme", "labs"]);
        let (a, b, old) = (project(&m, "acme-site"), project(&m, "labs-app"), project(&m, "legacy"));
        link(&m.brands, &a, "kettle", Some(None)).unwrap();
        link(&m.brands, &b, "kettle", Some(None)).unwrap();
        link(&m.brands, &old, "kettle", None).unwrap();
        assert_eq!(link_of(&old).unwrap().unwrap().pin.as_deref(), Some("1"), "a link pins the current version by default");
        // A brand ticket changes the brand and releases v2.
        let live = dir.join("design/tokens/tokens.json");
        std::fs::write(&live, BRAND_TOKENS.replace("#1D4ED8", "#7C3AED")).unwrap();
        release(&m.brands, "kettle", None).unwrap();
        assert_eq!(read_brand(&m.brands, "kettle").unwrap().version, "2");
        assert_eq!(versions(&m.brands, "kettle"), ["1", "2"]);
        for design in [&a, &b] {
            assert_eq!(token(&tokens_text_in(&m.brands, design).unwrap(), "color.primary").value, serde_json::json!("#7C3AED"), "both squads see the change");
        }
        let kept = resolve(&m.brands, &old).unwrap().unwrap();
        assert_eq!(token(&kept.text, "color.primary").value, serde_json::json!("#1D4ED8"), "pinned to v1, it keeps v1");
        assert_eq!(kept.newer.as_deref(), Some("2"));
        let note = prompt_note(&m.brands, &old, false).unwrap();
        assert!(note.contains("a newer v2 exists") && note.contains("brand-owned") && note.contains("only on a brand ticket"), "{note}");
        assert!(prompt_note(&m.brands, &a, true).unwrap().contains("equipe design brand release kettle"));
        // Later edits to the brand never reach a released version.
        std::fs::write(&live, BRAND_TOKENS.replace("#1D4ED8", "#000000")).unwrap();
        assert_eq!(token(&tokens_text_in(&m.brands, &old).unwrap(), "color.primary").value, serde_json::json!("#1D4ED8"));
        assert!(link(&m.brands, &old, "kettle", Some(Some("9"))).is_err(), "no such version");
    }

    #[test]
    fn tp9_an_override_that_breaks_aa_fails_the_checks_on_the_resolved_system() {
        let m = mac();
        brand(&m, "kettle");
        let design = project(&m, "site");
        link(&m.brands, &design, "kettle", None).unwrap();
        let aa = |design: &Path| check::check_in(design, &m.brands).into_iter().filter(|f| f.area == "contrast" && f.level == check::Level::Error).count();
        assert_eq!(aa(&design), 0, "the brand passes: {:#?}", check::check_in(&design, &m.brands));
        std::fs::write(design.join("tokens/overrides.json"), r##"{ "color": { "primary": { "$value": "#93C5FD" } } }"##).unwrap();
        assert!(aa(&design) > 0, "a light primary under white text fails: {:#?}", check::check_in(&design, &m.brands));
        let pairs = crate::design::system::token_contrast(&tokens::parse(&tokens_text_in(&m.brands, &design).unwrap()).tokens);
        assert!(pairs.iter().any(|p| p.contains("on-primary") && p.contains("4.5:1")), "{pairs:?}");
    }

    #[test]
    fn a_brand_link_that_does_not_resolve_is_said_and_the_file_is_read() {
        let m = mac();
        let design = project(&m, "site");
        std::fs::write(design.join(TOKENS), "{}").unwrap();
        std::fs::write(design.join(FILE), r#"{ "brand": "gone" }"#).unwrap();
        assert!(resolve(&m.brands, &design).unwrap().unwrap_err().contains("No brand `gone`"));
        assert_eq!(tokens_text_in(&m.brands, &design).as_deref(), Some("{}"));
        assert!(check::check_in(&design, &m.brands).iter().any(|f| f.area == "brand" && f.level == check::Level::Error));
        assert!(!valid_slug("../x") && !valid_slug("Kettle") && valid_slug("kettle-2"));
    }

    #[test]
    fn an_override_the_brand_does_not_have_is_kept_and_warned() {
        let m = mac();
        brand(&m, "kettle");
        let design = project(&m, "site");
        std::fs::write(design.join("tokens/overrides.json"), r##"{ "color": { "$type": "color", "brand-gone": { "$value": "#222222" } } }"##).unwrap();
        link(&m.brands, &design, "kettle", None).unwrap();
        let r = resolve(&m.brands, &design).unwrap().unwrap();
        assert_eq!(r.added, ["color.brand-gone"]);
        assert!(tokens::parse(&r.text).tokens.iter().any(|t| t.name == "color.brand-gone"), "kept");
        assert!(check::check_in(&design, &m.brands).iter().any(|f| f.area == "brand" && f.level == check::Level::Warn && f.message.contains("color.brand-gone")));
        assert!(unlink(&design).unwrap() && resolve(&m.brands, &design).is_none());
    }
}
