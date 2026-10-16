//! `tokens.json`, read: the W3C Design Tokens Community Group format. A node
//! with `$value` is a token; any other object is a group, whose `$type` its
//! tokens inherit. `"{color.blue.600}"` (a whole value, or a part of a
//! composite one) refers to another token, resolved here; a cycle or a name
//! that isn't there is a problem, not a failure — the rest still shows.
//!
//! Modes: a token's dark value is in `$extensions.mode.dark` (its `$value`
//! is its light one), or the file keeps groups named `light` and `dark`
//! side by side (`color.light.bg`, `color.dark.bg` — one token, `color.bg`).
//! A token that refers to one with modes has those modes too.

use std::collections::{BTreeMap, HashMap};

use equipe_protocol::design::Token;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

/// Group names that are modes, in the order they are listed.
pub const MODE_GROUPS: [&str; 2] = ["light", "dark"];
/// The most tokens read, and how deep groups may go.
pub const MAX_TOKENS: usize = 4000;
const MAX_DEPTH: usize = 32;
/// How many aliases deep a value may refer.
const MAX_HOPS: usize = 16;

/// What couldn't be read, said in the person's language by the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The file isn't JSON: where it stops being.
    NotJson { line: usize, column: usize },
    /// The file is JSON, but not an object of groups.
    NotGroups,
    /// `path` is neither a token (no `$value`) nor a group (not an object).
    Stray { path: String },
    /// `token` refers to `reference`, which isn't a token.
    Unresolved { token: String, reference: String },
    /// `token`'s aliases go round in a circle.
    Cycle { token: String },
    /// `token`'s value isn't a `kind`: `value` as written.
    Invalid { token: String, kind: String, value: String },
    /// More than [`MAX_TOKENS`]: the rest left out.
    TooMany,
}

/// What `tokens.json` holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    /// The modes found, the default first.
    pub modes: Vec<String>,
    pub tokens: Vec<Token>,
    pub problems: Vec<Problem>,
}

/// JSON with its objects' keys in the file's order, whatever `serde_json` was built with.
#[derive(Debug, Clone, PartialEq)]
enum Node {
    Null,
    Bool(bool),
    Num(serde_json::Number),
    Str(String),
    Arr(Vec<Node>),
    Obj(Vec<(String, Node)>),
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Node;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON")
            }
            fn visit_unit<E>(self) -> Result<Node, E> {
                Ok(Node::Null)
            }
            fn visit_none<E>(self) -> Result<Node, E> {
                Ok(Node::Null)
            }
            fn visit_bool<E>(self, b: bool) -> Result<Node, E> {
                Ok(Node::Bool(b))
            }
            fn visit_i64<E>(self, n: i64) -> Result<Node, E> {
                Ok(Node::Num(n.into()))
            }
            fn visit_u64<E>(self, n: u64) -> Result<Node, E> {
                Ok(Node::Num(n.into()))
            }
            fn visit_f64<E>(self, n: f64) -> Result<Node, E> {
                Ok(serde_json::Number::from_f64(n).map_or(Node::Null, Node::Num))
            }
            fn visit_str<E>(self, s: &str) -> Result<Node, E> {
                Ok(Node::Str(s.to_owned()))
            }
            fn visit_string<E>(self, s: String) -> Result<Node, E> {
                Ok(Node::Str(s))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Node, A::Error> {
                let mut out = Vec::new();
                while let Some(item) = seq.next_element()? {
                    out.push(item);
                }
                Ok(Node::Arr(out))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
                let mut out: Vec<(String, Node)> = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, Node>()? {
                    // A key said twice: the last one, as JSON parsers do.
                    out.retain(|(k, _)| *k != key);
                    out.push((key, value));
                }
                Ok(Node::Obj(out))
            }
        }
        d.deserialize_any(V)
    }
}

impl Node {
    fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Node::Str(s) => Some(s),
            _ => None,
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Node::Null => Value::Null,
            Node::Bool(b) => Value::Bool(*b),
            Node::Num(n) => Value::Number(n.clone()),
            Node::Str(s) => Value::String(s.clone()),
            Node::Arr(items) => Value::Array(items.iter().map(Node::to_value).collect()),
            Node::Obj(entries) => Value::Object(entries.iter().map(|(k, v)| (k.clone(), v.to_value())).collect()),
        }
    }
}

/// The name a whole-value alias refers to: `{color.bg}` → `color.bg`.
pub fn reference(text: &str) -> Option<&str> {
    let inner = text.trim().strip_prefix('{')?.strip_suffix('}')?;
    (!inner.is_empty() && !inner.contains(['{', '}'])).then_some(inner)
}

/// A token as the file has it, before modes are merged and aliases resolved.
struct Raw {
    path: Vec<String>,
    kind: Option<String>,
    value: Node,
    description: Option<String>,
    /// `$extensions.mode` (or `modes`): other modes' values.
    modes: Vec<(String, Node)>,
}

/// One token, its modes merged.
#[derive(Debug)]
struct Entry {
    kind: Option<String>,
    description: Option<String>,
    /// Its value with no mode said, and its value per mode.
    plain: Option<Node>,
    moded: Vec<(String, Node)>,
}

fn walk(node: &[(String, Node)], path: &mut Vec<String>, inherited: Option<&str>, out: &mut Vec<Raw>, problems: &mut Vec<Problem>) {
    if path.len() > MAX_DEPTH {
        return;
    }
    let kind = node.iter().find(|(k, _)| k == "$type").and_then(|(_, v)| v.as_str()).or(inherited);
    for (key, child) in node {
        if key.starts_with('$') {
            continue;
        }
        path.push(key.clone());
        match child {
            Node::Obj(entries) if child.get("$value").is_some() => {
                if out.len() >= MAX_TOKENS {
                    if !problems.contains(&Problem::TooMany) {
                        problems.push(Problem::TooMany);
                    }
                } else {
                    let own = entries.iter().find(|(k, _)| k == "$type").and_then(|(_, v)| v.as_str());
                    let modes = child
                        .get("$extensions")
                        .and_then(|e| e.get("mode").or_else(|| e.get("modes")))
                        .map(|m| match m {
                            Node::Obj(modes) => modes.clone(),
                            _ => Vec::new(),
                        })
                        .unwrap_or_default();
                    out.push(Raw {
                        path: path.clone(),
                        kind: own.or(kind).map(str::to_owned),
                        value: child.get("$value").cloned().unwrap_or(Node::Null),
                        description: child.get("$description").and_then(Node::as_str).map(str::to_owned),
                        modes,
                    });
                }
            }
            Node::Obj(entries) => walk(entries, path, kind, out, problems),
            _ => problems.push(Problem::Stray { path: path.join(".") }),
        }
        path.pop();
    }
}

/// Read `tokens.json`'s text.
pub fn parse(text: &str) -> Parsed {
    let root: Node = match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
        Ok(root) => root,
        Err(e) => return Parsed { problems: vec![Problem::NotJson { line: e.line(), column: e.column() }], ..Parsed::default() },
    };
    let Node::Obj(entries) = &root else {
        return Parsed { problems: vec![Problem::NotGroups], ..Parsed::default() };
    };
    let mut problems = Vec::new();
    let mut raws = Vec::new();
    walk(entries, &mut Vec::new(), None, &mut raws, &mut problems);

    // Modes merged: `color.dark.bg` is `color.bg` in dark. The paths as
    // written still name them, for aliases that say `{color.dark.bg}`.
    let mut order: Vec<String> = Vec::new();
    let mut entries: HashMap<String, Entry> = HashMap::new();
    let mut written: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut seen_modes: Vec<String> = Vec::new();
    for raw in raws {
        let at = raw.path[..raw.path.len() - 1].iter().position(|seg| MODE_GROUPS.contains(&seg.as_str()));
        let (name, mode) = match at {
            Some(at) => {
                let mut rest = raw.path.clone();
                let mode = rest.remove(at);
                (rest.join("."), Some(mode))
            }
            None => (raw.path.join("."), None),
        };
        written.insert(raw.path.join("."), (name.clone(), mode.clone()));
        let entry = entries.entry(name.clone()).or_insert_with(|| {
            order.push(name.clone());
            Entry { kind: None, description: None, plain: None, moded: Vec::new() }
        });
        entry.kind = entry.kind.take().or(raw.kind);
        entry.description = entry.description.take().or(raw.description);
        match &mode {
            Some(mode) => entry.moded.push((mode.clone(), raw.value)),
            None => entry.plain = Some(raw.value),
        }
        // `$extensions.mode` names the other modes: its `$value` is light.
        let light = (mode.is_none() && !raw.modes.is_empty()).then(|| MODE_GROUPS[0].to_owned());
        entry.moded.extend(raw.modes);
        for mode in light.into_iter().chain(mode).chain(entry.moded.iter().map(|(m, _)| m.clone())) {
            if !seen_modes.contains(&mode) {
                seen_modes.push(mode);
            }
        }
    }
    // Light first, then dark, then the others as they came.
    let mut modes: Vec<String> = MODE_GROUPS.iter().filter(|m| seen_modes.iter().any(|s| s == *m)).map(|m| (*m).to_owned()).collect();
    modes.extend(seen_modes.into_iter().filter(|m| !MODE_GROUPS.contains(&m.as_str())));
    // With modes, a token with only its plain value still counts as light.
    let default_mode = modes.first().cloned();

    let resolver = Resolver { entries: &entries, written: &written, modes: &modes };
    let mut tokens = Vec::new();
    for name in &order {
        let entry = &entries[name];
        let alias = entry.plain.as_ref().or(entry.moded.first().map(|(_, v)| v)).and_then(Node::as_str).filter(|s| reference(s).is_some()).map(str::to_owned);
        let mut per_mode: Vec<(String, Value)> = Vec::new();
        let mut failed = None;
        let wanted: Vec<Option<&str>> = if modes.is_empty() { vec![None] } else { modes.iter().map(|m| Some(m.as_str())).collect() };
        for mode in &wanted {
            let Some(node) = resolver.pick(entry, *mode) else { continue };
            match resolver.resolve(node, *mode, &mut vec![name.clone()]) {
                Ok(value) => per_mode.push((mode.unwrap_or_default().to_owned(), value)),
                Err(problem) => {
                    failed = Some(match problem {
                        Problem::Unresolved { reference, .. } => Problem::Unresolved { token: name.clone(), reference },
                        _ => Problem::Cycle { token: name.clone() },
                    });
                    break;
                }
            }
        }
        if let Some(problem) = failed {
            problems.push(problem);
            continue;
        }
        let Some((_, value)) = per_mode.first().cloned() else { continue };
        let kind = entry.kind.clone().or_else(|| resolver.kind_through(entry, name)).unwrap_or_else(|| infer(&value).to_owned());
        let differs = per_mode.iter().any(|(_, v)| *v != value);
        let modes: BTreeMap<String, Value> = if differs { per_mode.into_iter().collect() } else { BTreeMap::new() };
        // The default mode's value comes first, whatever order the file had.
        let value = default_mode.as_ref().and_then(|m| modes.get(m)).cloned().unwrap_or(value);
        let token = Token { name: name.clone(), kind, value, modes, description: entry.description.clone(), alias };
        for (_, v) in std::iter::once((&String::new(), &token.value)).chain(token.modes.iter()) {
            if !super::values::fits(&token.kind, v) {
                problems.push(Problem::Invalid { token: token.name.clone(), kind: token.kind.clone(), value: short(v) });
                break;
            }
        }
        tokens.push(token);
    }
    Parsed { modes, tokens, problems }
}

/// A value as written, short enough for a line.
fn short(v: &Value) -> String {
    let text = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if text.chars().count() > 60 { format!("{}…", text.chars().take(60).collect::<String>()) } else { text }
}

/// A type for a token that says none: what its value looks like.
fn infer(value: &Value) -> &'static str {
    use super::values;
    match value {
        Value::String(s) if values::color(value).is_some() && (s.starts_with('#') || s.starts_with("rgb")) => "color",
        Value::String(s) if s.ends_with("ms") || (s.ends_with('s') && s.trim_end_matches('s').parse::<f64>().is_ok()) => "duration",
        Value::String(_) if values::dimension(value).is_some() => "dimension",
        Value::Number(_) => "number",
        Value::Array(items) if items.len() == 4 && items.iter().all(Value::is_number) => "cubicBezier",
        Value::Object(o) if o.contains_key("colorSpace") => "color",
        Value::Object(o) if o.contains_key("fontFamily") || o.contains_key("fontSize") => "typography",
        Value::Object(o) if o.contains_key("offsetX") || o.contains_key("blur") => "shadow",
        _ => "other",
    }
}

struct Resolver<'a> {
    entries: &'a HashMap<String, Entry>,
    /// A path as written, to the token it is and the mode it fixes.
    written: &'a HashMap<String, (String, Option<String>)>,
    modes: &'a [String],
}

impl Resolver<'_> {
    /// `entry`'s value in `mode`: its own for it, else its plain one, else
    /// the first mode's it has.
    fn pick<'e>(&self, entry: &'e Entry, mode: Option<&str>) -> Option<&'e Node> {
        let own = mode.and_then(|m| entry.moded.iter().find(|(k, _)| k == m).map(|(_, v)| v));
        own.or(entry.plain.as_ref()).or_else(|| self.modes.iter().find_map(|m| entry.moded.iter().find(|(k, _)| k == m).map(|(_, v)| v))).or(entry.moded.first().map(|(_, v)| v))
    }

    /// Which token `reference` names, and the mode it fixes.
    fn target(&self, reference: &str) -> Option<(&str, Option<&str>)> {
        if let Some((name, _)) = self.entries.get_key_value(reference) {
            return Some((name.as_str(), None));
        }
        self.written.get(reference).map(|(name, mode)| (name.as_str(), mode.as_deref()))
    }

    fn resolve(&self, node: &Node, mode: Option<&str>, stack: &mut Vec<String>) -> Result<Value, Problem> {
        match node {
            Node::Str(s) => match reference(s) {
                Some(reference) => {
                    let (name, fixed) = self.target(reference).ok_or_else(|| Problem::Unresolved { token: String::new(), reference: s.clone() })?;
                    if stack.iter().any(|n| n == name) || stack.len() > MAX_HOPS {
                        return Err(Problem::Cycle { token: String::new() });
                    }
                    let mode = fixed.or(mode);
                    let next = self.pick(&self.entries[name], mode).ok_or_else(|| Problem::Unresolved { token: String::new(), reference: s.clone() })?;
                    stack.push(name.to_owned());
                    let value = self.resolve(next, mode, stack);
                    stack.pop();
                    value
                }
                None => Ok(Value::String(s.clone())),
            },
            Node::Arr(items) => items.iter().map(|i| self.resolve(i, mode, stack)).collect::<Result<Vec<_>, _>>().map(Value::Array),
            Node::Obj(entries) => {
                let mut out = serde_json::Map::new();
                for (k, v) in entries {
                    out.insert(k.clone(), self.resolve(v, mode, stack)?);
                }
                Ok(Value::Object(out))
            }
            other => Ok(other.to_value()),
        }
    }

    /// The type of the token a whole-value alias leads to.
    fn kind_through(&self, entry: &Entry, name: &str) -> Option<String> {
        let mut at = entry;
        let mut seen = vec![name.to_owned()];
        loop {
            let reference = self.pick(at, None).and_then(Node::as_str).and_then(reference)?;
            let (next, _) = self.target(reference)?;
            if seen.iter().any(|s| s == next) || seen.len() > MAX_HOPS {
                return None;
            }
            seen.push(next.to_owned());
            at = &self.entries[next];
            if let Some(kind) = &at.kind {
                return Some(kind.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn token<'a>(parsed: &'a Parsed, name: &str) -> &'a Token {
        parsed.tokens.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{name}: {:?}", parsed.tokens.iter().map(|t| &t.name).collect::<Vec<_>>()))
    }

    #[test]
    fn groups_pass_their_type_down_and_order_is_the_files() {
        let parsed = parse(
            r##"{
              "spacing": { "$type": "dimension", "xs": { "$value": "4px" }, "sm": { "$value": "8px" }, "lg": { "$value": "24px", "$description": "Between sections" } },
              "color": { "zebra": { "$type": "color", "$value": "#000" }, "apple": { "$type": "color", "$value": "#fff" } }
            }"##,
        );
        assert!(parsed.problems.is_empty(), "{:?}", parsed.problems);
        let names: Vec<&str> = parsed.tokens.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["spacing.xs", "spacing.sm", "spacing.lg", "color.zebra", "color.apple"], "as written, not sorted");
        assert_eq!(token(&parsed, "spacing.sm").kind, "dimension");
        assert_eq!(token(&parsed, "spacing.lg").description.as_deref(), Some("Between sections"));
        assert!(parsed.modes.is_empty());
    }

    #[test]
    fn an_alias_takes_the_value_and_type_of_what_it_names_through_chains_and_composites() {
        let parsed = parse(
            r##"{
              "blue": { "600": { "$type": "color", "$value": "#1D4ED8" } },
              "color": { "accent": { "$value": "{blue.600}" }, "link": { "$value": "{color.accent}" } },
              "font": { "sans": { "$type": "fontFamily", "$value": ["Inter", "sans-serif"] } },
              "type": { "body": { "$type": "typography", "$value": { "fontFamily": "{font.sans}", "fontSize": "16px", "fontWeight": 400, "lineHeight": 1.5 } } }
            }"##,
        );
        assert!(parsed.problems.is_empty(), "{:?}", parsed.problems);
        let link = token(&parsed, "color.link");
        assert_eq!((link.kind.as_str(), &link.value, link.alias.as_deref()), ("color", &json!("#1D4ED8"), Some("{color.accent}")));
        assert_eq!(token(&parsed, "type.body").value["fontFamily"], json!(["Inter", "sans-serif"]));
    }

    #[test]
    fn light_and_dark_groups_are_one_token_with_two_modes_and_so_is_what_refers_to_it() {
        // As the designer wrote T-144's draft.
        let parsed = parse(
            r##"{
              "color": {
                "light": { "bg": { "$value": "#F6F5F1", "$type": "color" }, "accent": { "$value": "#1D4ED8", "$type": "color" } },
                "dark": { "bg": { "$value": "#0A0A0C", "$type": "color" }, "accent": { "$value": "#8FB0FF", "$type": "color" } },
                "link": { "$value": "{color.accent}" },
                "focus": { "$value": "{color.dark.accent}" }
              },
              "brand": { "dot": { "$value": "#3F6FF5", "$type": "color" } }
            }"##,
        );
        assert!(parsed.problems.is_empty(), "{:?}", parsed.problems);
        assert_eq!(parsed.modes, ["light", "dark"]);
        let bg = token(&parsed, "color.bg");
        assert_eq!((bg.value_in(Some("light")), bg.value_in(Some("dark"))), (&json!("#F6F5F1"), &json!("#0A0A0C")));
        assert_eq!(bg.value, json!("#F6F5F1"), "light is the default");
        let link = token(&parsed, "color.link");
        assert_eq!(link.value_in(Some("dark")), &json!("#8FB0FF"), "an alias of a token with modes has them too");
        assert_eq!(token(&parsed, "color.focus").value, json!("#8FB0FF"), "a path that names the mode fixes it");
        assert!(token(&parsed, "color.focus").modes.is_empty());
        assert!(token(&parsed, "brand.dot").modes.is_empty(), "one value: no modes");
        assert!(!parsed.tokens.iter().any(|t| t.name.contains("light")));
    }

    #[test]
    fn a_dark_value_in_extensions_is_a_mode() {
        let parsed = parse(r##"{ "color": { "$type": "color", "text": { "$value": "#15161A", "$extensions": { "mode": { "dark": "#EFEDE7" } } } } }"##);
        assert_eq!(parsed.modes, ["light", "dark"], "its `$value` is light");
        let text = token(&parsed, "color.text");
        assert_eq!(text.value, json!("#15161A"));
        assert_eq!(text.value_in(Some("dark")), &json!("#EFEDE7"));
    }

    #[test]
    fn what_cant_be_read_is_said_and_the_rest_still_shows() {
        assert_eq!(parse("{ \"color\": ").problems.len(), 1);
        assert!(matches!(parse("{ \"a\": 1,, }").problems[0], Problem::NotJson { line: 1, .. }));
        assert_eq!(parse("[1, 2]").problems, [Problem::NotGroups]);
        let parsed = parse(
            r##"{
              "color": {
                "$type": "color",
                "ok": { "$value": "#123456" },
                "gone": { "$value": "{color.nowhere}" },
                "a": { "$value": "{color.b}" },
                "b": { "$value": "{color.a}" },
                "bad": { "$value": "#GG0000" },
                "note": "not a token"
              },
              "space": { "$type": "dimension", "odd": { "$value": "twelve" } }
            }"##,
        );
        assert_eq!(parsed.tokens.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["color.ok", "color.bad", "space.odd"]);
        let has = |p: &Problem| parsed.problems.contains(p);
        assert!(has(&Problem::Unresolved { token: "color.gone".into(), reference: "{color.nowhere}".into() }), "{:?}", parsed.problems);
        assert!(has(&Problem::Cycle { token: "color.a".into() }) && has(&Problem::Cycle { token: "color.b".into() }));
        assert!(has(&Problem::Invalid { token: "color.bad".into(), kind: "color".into(), value: "#GG0000".into() }));
        assert!(has(&Problem::Invalid { token: "space.odd".into(), kind: "dimension".into(), value: "twelve".into() }));
        assert!(has(&Problem::Stray { path: "color.note".into() }));
    }

    #[test]
    fn a_huge_file_is_cut_with_a_word() {
        let many: Vec<String> = (0..MAX_TOKENS + 5).map(|n| format!("\"t{n}\": {{ \"$value\": {n} }}")).collect();
        let parsed = parse(&format!("{{ \"n\": {{ \"$type\": \"number\", {} }} }}", many.join(",")));
        assert_eq!(parsed.tokens.len(), MAX_TOKENS);
        assert_eq!(parsed.problems, [Problem::TooMany]);
    }

    #[test]
    fn untyped_tokens_are_typed_by_what_they_look_like() {
        let parsed = parse(r##"{ "c": { "$value": "#fff" }, "d": { "$value": "12px" }, "t": { "$value": "200ms" }, "e": { "$value": [0.2, 0, 0, 1] } }"##);
        let kinds: Vec<&str> = parsed.tokens.iter().map(|t| t.kind.as_str()).collect();
        assert_eq!(kinds, ["color", "dimension", "duration", "cubicBezier"]);
    }
}
