//! Design registries merged entry by entry (docs/DESIGN-CAPTURES.md).
//!
//! A few JSON files under a design folder are registries every design
//! ticket adds to: the UI map (`flows/flows.json`), the reference board
//! (`references/board.json`), the directions' log (`directions/log.json`)
//! and the captures' manifest (`captures.json`). The UI map is kept on one
//! line, so two tickets that each add a flow conflict for git every time,
//! and a member had to merge it by hand on almost every landing.
//!
//! Merged as JSON instead, three-way ([`merge3`]): an array whose entries
//! each carry an `id` (or a `path`, or a `name`) is merged entry by entry —
//! both sides' additions kept, an entry one side changed taken from it —
//! objects key by key, and only a value both sides changed differently is a
//! conflict. The result keeps the file's style: one line stays one line,
//! an indented file keeps its indent ([`Style`]). The captures' manifest
//! merges as its own module says ([`super::captures::merge3`]).
//!
//! Two places use it:
//!
//! - **equipe's own landing**: where `git merge-tree` reports a registry
//!   conflicting, [`settle_tree`] merges it as JSON, and the PM is told of a
//!   conflict only when one is left ([`real_conflicts`]).
//! - **git itself**, in every checkout of the repository: [`ensure`] adds a
//!   merge driver to the repository's config (`merge.equipe-registry`) and
//!   lines naming the registries to its `info/attributes`, both local to the
//!   machine and shared by all its worktrees, so a member's `git merge main`
//!   runs `equipe design merge-registry` ([`run_driver`]) for these files.
//!   Nothing is committed for it; without equipe the files merge as text.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

use crate::worktree::{self, Result};

/// The registries, by their path in a design folder.
pub const FILES: [&str; 4] = ["flows/flows.json", "references/board.json", "directions/log.json", super::captures::MANIFEST];
/// What an entry of a registry's array is known by, in this order.
pub const KEYS: [&str; 3] = ["id", "path", "name"];
/// The merge driver's name in git's config and attributes.
pub const DRIVER: &str = "equipe-registry";
const BLOCK_START: &str = "# >>> equipe: design registries merge entry by entry";
const BLOCK_END: &str = "# <<< equipe: design registries";

/// Whether `path` (from the top of a repository) is a registry, and which:
/// its path inside the design folder.
pub fn registry_of(path: &str) -> Option<&'static str> {
    let inner = path.strip_prefix(&format!("{}/", super::DIR)).or_else(|| path.split_once(&format!("/{}/", super::DIR)).map(|(_, rest)| rest))?;
    FILES.into_iter().find(|f| *f == inner)
}

// ── JSON, its keys kept in order ──

/// A JSON value whose objects keep their keys in the order the file has them.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// What an entry is known by under `key`, when it is an object with a string there.
    fn identity(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Json::String(s)) => Some(s),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Json, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Json;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON")
            }
            fn visit_unit<E>(self) -> std::result::Result<Json, E> {
                Ok(Json::Null)
            }
            fn visit_none<E>(self) -> std::result::Result<Json, E> {
                Ok(Json::Null)
            }
            fn visit_bool<E>(self, v: bool) -> std::result::Result<Json, E> {
                Ok(Json::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> std::result::Result<Json, E> {
                Ok(Json::Number(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> std::result::Result<Json, E> {
                Ok(Json::Number(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Json, E> {
                serde_json::Number::from_f64(v).map(Json::Number).ok_or_else(|| E::custom("a number JSON can't hold"))
            }
            fn visit_str<E>(self, v: &str) -> std::result::Result<Json, E> {
                Ok(Json::String(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> std::result::Result<Json, E> {
                Ok(Json::String(v))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Json, A::Error> {
                let mut out = Vec::new();
                while let Some(v) = seq.next_element()? {
                    out.push(v);
                }
                Ok(Json::Array(out))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Json, A::Error> {
                let mut out: Vec<(String, Json)> = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, Json>()? {
                    // A key twice: the last wins, as JSON parsers take it.
                    match out.iter_mut().find(|(key, _)| *key == k) {
                        Some(slot) => slot.1 = v,
                        None => out.push((k, v)),
                    }
                }
                Ok(Json::Object(out))
            }
        }
        deserializer.deserialize_any(V)
    }
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Json::Null => serializer.serialize_unit(),
            Json::Bool(b) => serializer.serialize_bool(*b),
            Json::Number(n) => n.serialize(serializer),
            Json::String(s) => serializer.serialize_str(s),
            Json::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Json::Object(fields) => {
                let mut map = serializer.serialize_map(Some(fields.len()))?;
                for (k, v) in fields {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

/// How a registry is written: on one line, or indented by `indent` — and
/// whether it ends with a newline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Style {
    pub indent: Option<String>,
    pub newline: bool,
}

impl Style {
    /// The style `text` is written in.
    pub fn of(text: &str) -> Style {
        let newline = text.ends_with('\n');
        let body = text.trim();
        let indent = body.contains('\n').then(|| {
            let lead: String = body.lines().skip(1).find(|l| !l.trim().is_empty()).map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect()).unwrap_or_default();
            if lead.is_empty() { "  ".to_owned() } else { lead }
        });
        Style { indent, newline }
    }

    /// `value` written in this style.
    pub fn write(&self, value: &Json) -> String {
        let mut out = match &self.indent {
            None => serde_json::to_string(value).unwrap_or_default(),
            Some(indent) => {
                let mut bytes = Vec::new();
                let mut ser = serde_json::Serializer::with_formatter(&mut bytes, serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes()));
                let _ = value.serialize(&mut ser);
                String::from_utf8(bytes).unwrap_or_default()
            }
        };
        if self.newline {
            out.push('\n');
        }
        out
    }
}

// ── the merge ──

/// Three versions of a JSON value merged: `None` where a version doesn't
/// have it. What one side changed since `base` and the other didn't is
/// taken; where both changed it, objects merge key by key and arrays of
/// entries entry by entry ([`KEYS`]), other arrays keep both sides'
/// additions at their end; anything else both changed differently is a
/// conflict, named by its place (`flows[id=erp].screens[id=list].title`).
/// `Ok(None)`: it is removed.
pub fn merge3(base: Option<&Json>, ours: Option<&Json>, theirs: Option<&Json>) -> std::result::Result<Option<Json>, Vec<String>> {
    let mut conflicts = Vec::new();
    let merged = merge_at("", base, ours, theirs, &mut conflicts);
    if conflicts.is_empty() { Ok(merged) } else { Err(conflicts) }
}

fn merge_at(at: &str, base: Option<&Json>, ours: Option<&Json>, theirs: Option<&Json>, conflicts: &mut Vec<String>) -> Option<Json> {
    if ours == theirs {
        return ours.cloned();
    }
    if base == ours {
        return theirs.cloned();
    }
    if base == theirs {
        return ours.cloned();
    }
    match (ours, theirs) {
        (Some(Json::Object(o)), Some(Json::Object(t))) => {
            let empty = Vec::new();
            let b = match base {
                Some(Json::Object(b)) => b,
                _ => &empty,
            };
            Some(Json::Object(merge_objects(at, b, o, t, conflicts)))
        }
        (Some(Json::Array(o)), Some(Json::Array(t))) => {
            let empty = Vec::new();
            let b = match base {
                Some(Json::Array(b)) => b,
                _ => &empty,
            };
            Some(Json::Array(merge_arrays(at, b, o, t, conflicts)))
        }
        _ => {
            conflicts.push(if at.is_empty() { "the whole file".to_owned() } else { at.to_owned() });
            ours.cloned()
        }
    }
}

fn merge_objects(at: &str, base: &[(String, Json)], ours: &[(String, Json)], theirs: &[(String, Json)], conflicts: &mut Vec<String>) -> Vec<(String, Json)> {
    let find = |fields: &'_ [(String, Json)], key: &str| -> Option<Json> { fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()) };
    let mut keys: Vec<&str> = ours.iter().map(|(k, _)| k.as_str()).collect();
    keys.extend(theirs.iter().map(|(k, _)| k.as_str()).filter(|k| !ours.iter().any(|(o, _)| o == k)));
    keys.extend(base.iter().map(|(k, _)| k.as_str()).filter(|k| !ours.iter().any(|(o, _)| o == k) && !theirs.iter().any(|(t, _)| t == k)));
    let mut out = Vec::new();
    for key in keys {
        let place = if at.is_empty() { key.to_owned() } else { format!("{at}.{key}") };
        let (b, o, t) = (find(base, key), find(ours, key), find(theirs, key));
        if let Some(v) = merge_at(&place, b.as_ref(), o.as_ref(), t.as_ref(), conflicts) {
            out.push((key.to_owned(), v));
        }
    }
    out
}

/// The key every entry of all three arrays is known by, each once: the
/// first of [`KEYS`] that fits. `None`: not an array of entries.
fn entry_key(arrays: [&[Json]; 3]) -> Option<&'static str> {
    if arrays.iter().all(|a| a.is_empty()) {
        return None;
    }
    KEYS.into_iter().find(|key| {
        arrays.iter().all(|a| {
            let mut seen = HashSet::new();
            a.iter().all(|e| e.identity(key).is_some_and(|id| seen.insert(id)))
        })
    })
}

fn merge_arrays(at: &str, base: &[Json], ours: &[Json], theirs: &[Json], conflicts: &mut Vec<String>) -> Vec<Json> {
    let Some(key) = entry_key([base, ours, theirs]) else {
        // Not entries: both sides' additions to what both kept, else a conflict.
        if ours.starts_with(base) && theirs.starts_with(base) {
            let mut out = ours.to_vec();
            out.extend(theirs[base.len()..].iter().filter(|t| !ours[base.len()..].contains(t)).cloned());
            return out;
        }
        conflicts.push(if at.is_empty() { "the whole file".to_owned() } else { at.to_owned() });
        return ours.to_vec();
    };
    let by_id = |a: &'_ [Json]| -> HashMap<String, Json> { a.iter().filter_map(|e| Some((e.identity(key)?.to_owned(), e.clone()))).collect() };
    let (b, o, t) = (by_id(base), by_id(ours), by_id(theirs));
    // Ours in its order; each entry only theirs added goes after the one
    // before it there — and after what only ours added at that place.
    let mut order: Vec<String> = ours.iter().filter_map(|e| e.identity(key).map(str::to_owned)).collect();
    let ours_only = |id: &str| !b.contains_key(id) && !t.contains_key(id);
    let mut after: Option<String> = None;
    for e in theirs {
        let Some(id) = e.identity(key) else { continue };
        if order.iter().any(|x| x == id) {
            after = Some(id.to_owned());
            continue;
        }
        if b.contains_key(id) {
            // Ours dropped it: merged below with what the base had.
            continue;
        }
        let mut at = after.as_ref().and_then(|a| order.iter().position(|x| x == a)).map_or(0, |p| p + 1);
        while at < order.len() && ours_only(&order[at]) {
            at += 1;
        }
        order.insert(at, id.to_owned());
        after = Some(id.to_owned());
    }
    // Then what the base had that ours dropped: gone, unless theirs changed it.
    let dropped: Vec<String> = base.iter().filter_map(|e| e.identity(key).map(str::to_owned)).collect();
    let mut out = Vec::new();
    let mut done: BTreeSet<String> = BTreeSet::new();
    for id in order.iter().chain(&dropped) {
        if !done.insert(id.clone()) {
            continue;
        }
        let place = format!("{at}[{key}={id}]");
        if let Some(v) = merge_at(&place, b.get(id), o.get(id), t.get(id), conflicts) {
            out.push(v);
        }
    }
    out
}

/// Three versions of a registry's text merged: the merged text, in `ours`'
/// style — or, when one side's text is the result, that text as it is.
/// The places both sides changed differently, or what couldn't be read, as
/// the error. `file` is the registry's path in the design folder.
pub fn merge_text(file: &str, base: Option<&str>, ours: &str, theirs: &str) -> std::result::Result<String, Vec<String>> {
    if file == super::captures::MANIFEST {
        let parse = |text: &str, side: &str| super::captures::Manifest::parse(text).ok_or_else(|| vec![format!("{side} isn't a captures manifest")]);
        let (o, t) = (parse(ours, "ours")?, parse(theirs, "theirs")?);
        let b = match base.filter(|b| !b.trim().is_empty()) {
            Some(text) => parse(text, "the base")?,
            None => super::captures::Manifest::default(),
        };
        let merged = super::captures::merge3(&b, &o, &t);
        return Ok(if merged == o { ours.to_owned() } else if merged == t { theirs.to_owned() } else { merged.render() });
    }
    let parse = |text: &str, side: &str| serde_json::from_str::<Json>(text).map_err(|e| vec![format!("{side} isn't JSON: {e}")]);
    let (o, t) = (parse(ours, "ours")?, parse(theirs, "theirs")?);
    let b = base.filter(|b| !b.trim().is_empty()).map(|text| parse(text, "the base")).transpose()?;
    let merged = merge3(b.as_ref(), Some(&o), Some(&t))?;
    Ok(match merged {
        Some(m) if m == o => ours.to_owned(),
        Some(m) if m == t => theirs.to_owned(),
        Some(m) => Style::of(ours).write(&m),
        None => String::new(),
    })
}

// ── equipe's landing ──

/// The blob at `rev:path`, as text; `None` when it isn't there.
fn text_at(repo: &Path, rev: &str, path: &str) -> Option<String> {
    worktree::git_bytes(repo, &["show", &format!("{rev}:{path}")], None, None).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// A registry that merging `head` into `tip` reports conflicting, merged as
/// JSON: its merged text, or `None` when a conflict is left.
pub fn merged_at(repo: &Path, base: Option<&str>, tip: &str, head: &str, path: &str) -> Option<String> {
    let file = registry_of(path)?;
    let ours = text_at(repo, tip, path)?;
    let theirs = text_at(repo, head, path)?;
    let base = base.and_then(|b| text_at(repo, b, path));
    merge_text(file, base.as_deref(), &ours, &theirs).ok()
}

/// Of the files merging `head` into `tip` reports conflicting, those still
/// conflicting once the registries among them are merged as JSON.
pub fn real_conflicts(repo: &Path, tip: &str, head: &str, conflicted: Vec<String>) -> Vec<String> {
    if !conflicted.iter().any(|f| registry_of(f).is_some()) {
        return conflicted;
    }
    let base = worktree::git(repo, &["merge-base", tip, head]).ok();
    conflicted.into_iter().filter(|f| registry_of(f).is_none() || merged_at(repo, base.as_deref(), tip, head, f).is_none()).collect()
}

/// The tree git's merge made (`merged`, its conflicted files marked) with
/// each conflicting registry merged as JSON, and the conflicted files left.
/// `None` when a registry's conflict is left.
pub fn settle_tree<'a>(repo: &Path, merged: &str, tip: &str, head: &str, conflicted: &[&'a str]) -> Option<(String, Vec<&'a str>)> {
    let (registries, rest): (Vec<&str>, Vec<&str>) = conflicted.iter().partition(|f| registry_of(f).is_some());
    if registries.is_empty() {
        return Some((merged.to_owned(), rest));
    }
    let base = worktree::git(repo, &["merge-base", tip, head]).ok();
    let mut texts = Vec::new();
    for path in registries {
        texts.push((path, merged_at(repo, base.as_deref(), tip, head, path)?));
    }
    let tree = edit_tree(repo, merged, &texts).ok()?;
    Some((tree, rest))
}

/// A scratch index read from `tree`, each of `files` (path, text) put in
/// it, written back: the new tree.
fn edit_tree(repo: &Path, tree: &str, files: &[(&str, String)]) -> Result<String> {
    let scratch = worktree::scratch_path("equipe-registry", "");
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let done = (|| {
        let index = scratch.join("index");
        worktree::git_bytes(repo, &["read-tree", tree], Some(&index), None)?;
        for (n, (path, text)) in files.iter().enumerate() {
            let file = scratch.join(n.to_string());
            std::fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
            let blob = worktree::git(repo, &["hash-object", "-w", "--", &file.to_string_lossy()])?;
            worktree::git_bytes(repo, &["update-index", "--add", "--cacheinfo", &format!("100644,{blob},{path}")], Some(&index), None)?;
        }
        let out = worktree::git_bytes(repo, &["write-tree"], Some(&index), None)?;
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    done
}

// ── git's merge driver ──

static BINARY: OnceLock<PathBuf> = OnceLock::new();
static ENSURED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

/// The `equipe` program git's merge driver runs: set once by the daemon.
/// Anything but an `equipe` program that is there is never set — a test's
/// own binary must not be run as one.
pub fn use_binary(path: &Path) {
    let named = path.file_stem().is_some_and(|s| s == "equipe");
    if named && path.is_file() {
        let _ = BINARY.set(path.to_owned());
    }
}

/// The driver's command line for `binary`: equipe merges the registry as
/// JSON; where it can't (a conflict left, or no equipe there), git's own
/// text merge runs, as it would without a driver.
pub fn driver_command(binary: &Path) -> String {
    let quoted = format!("'{}'", binary.to_string_lossy().replace('\'', "'\\''"));
    format!("{quoted} design merge-registry %O %A %B %P || git merge-file -q --marker-size=%L -L ours -L base -L theirs %A %O %B")
}

/// The lines equipe keeps in a repository's `info/attributes`.
pub fn attributes_block() -> String {
    let mut out = vec![BLOCK_START.to_owned(), "# equipe rewrites this block: edit outside it.".to_owned()];
    out.extend(FILES.iter().map(|f| format!("**/{}/{f} merge={DRIVER}", super::DIR)));
    out.push(BLOCK_END.to_owned());
    out.join("\n") + "\n"
}

/// `existing` with equipe's attributes block in it, once, as it should be.
pub fn with_attributes(existing: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in existing.lines() {
        if line.trim() == BLOCK_START {
            inside = true;
            continue;
        }
        if inside {
            if line.trim() == BLOCK_END {
                inside = false;
            }
            continue;
        }
        kept.push(line);
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out + &attributes_block()
}

/// Make git in `repo` merge the registries as JSON: the driver in the
/// repository's config and the lines naming them in its `info/attributes`
/// — local to this machine, shared by every worktree, never committed. Done
/// once per repository a run; nothing without an `equipe` program
/// ([`use_binary`]).
pub fn ensure(repo: &Path) {
    let Some(binary) = BINARY.get() else { return };
    let Ok(common) = worktree::git(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"]) else { return };
    let common = PathBuf::from(common);
    let ensured = ENSURED.get_or_init(Default::default);
    if ensured.lock().is_ok_and(|e| e.contains(&common)) {
        return;
    }
    let key = format!("merge.{DRIVER}.driver");
    let command = driver_command(binary);
    let mut ok = true;
    if worktree::git(repo, &["config", "--local", "--get", &key]).ok().as_deref() != Some(command.as_str()) {
        ok &= worktree::git(repo, &["config", "--local", &format!("merge.{DRIVER}.name"), "equipe: design registries merged entry by entry"]).is_ok();
        ok &= worktree::git(repo, &["config", "--local", &key, &command]).is_ok();
    }
    let file = common.join("info").join("attributes");
    let existing = std::fs::read_to_string(&file).unwrap_or_default();
    let wanted = with_attributes(&existing);
    if wanted != existing {
        ok &= std::fs::create_dir_all(common.join("info")).is_ok() && std::fs::write(&file, wanted).is_ok();
    }
    if ok && let Ok(mut e) = ensured.lock() {
        e.insert(common);
    }
}

/// The driver itself (`equipe design merge-registry %O %A %B %P`): the
/// registry at `path` merged from `base`, `ours` and `theirs`, written over
/// `ours`. `Err` with the places in conflict, `ours` untouched: git's text
/// merge then marks the conflict.
pub fn run_driver(base: &Path, ours: &Path, theirs: &Path, path: &str) -> std::result::Result<(), String> {
    let file = registry_of(path).ok_or_else(|| format!("{path} isn't a design registry equipe merges"))?;
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
    let (b, o, t) = (std::fs::read_to_string(base).ok(), read(ours)?, read(theirs)?);
    let merged = merge_text(file, b.as_deref(), &o, &t).map_err(|places| format!("{path}: both sides changed {}", places.join(", ")))?;
    if merged != o {
        std::fs::write(ours, merged).map_err(|e| format!("{}: {e}", ours.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
