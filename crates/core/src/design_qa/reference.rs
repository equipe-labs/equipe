//! Reference-first design QA. A product built from the person's own design
//! (a Claude Design export or other prototype in `prototype/`, a site they
//! own in `reference/`, a saved design page) is judged against that design,
//! side by side, before anything else. A build once passed on render checks
//! alone while it had dropped every signature element of the demo it came
//! from (the canvas, the orb, the lock moment, the meeting space): a clean
//! precheck says nothing about whether it still feels like their design.
//!
//! `design/reference/signature.json` lists what makes the reference itself
//! (written by the platform's `reference_signature` tool from the design's
//! own markup, then named by the PM): the screens to compare and the
//! signature items. A pass needs a "same feel?" verdict per screen, each one
//! yes, and every signature item seen in the build. A missing item is S1.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

/// Where the reference's checklist lives in a product's repository.
pub const SIGNATURE: &str = "design/reference/signature.json";

/// One thing that makes the reference itself: a tab bar, a canvas field, a
/// full-screen moment, a special state.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Item {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// The person's own design a build is judged against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reference {
    /// The design's entry, relative to the repository (`prototype/index.html`).
    pub source: String,
    /// The screens or states to put side by side (`lock`, `meeting-space`...).
    pub screens: Vec<String>,
    /// The signature items the build must keep.
    pub items: Vec<Item>,
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    source: String,
    #[serde(default)]
    screens: Vec<Value>,
    #[serde(default)]
    items: Vec<Value>,
}

fn label(v: &Value, keys: &[&str]) -> Option<String> {
    let s = match v {
        Value::String(s) => Some(s.as_str()),
        Value::Object(m) => keys.iter().find_map(|k| m.get(*k).and_then(Value::as_str)),
        _ => None,
    }?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

fn is_dir(p: &Path) -> bool {
    p.is_dir()
}

/// An `.html` page in `dir` (one level), if there is one.
fn page_in(dir: &Path) -> Option<PathBuf> {
    let mut pages: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("html") || x.eq_ignore_ascii_case("htm"))).collect();
    pages.sort();
    pages.iter().find(|p| p.file_name().is_some_and(|n| n == "index.html")).cloned().or_else(|| pages.into_iter().next())
}

impl Reference {
    /// The reference of the product checked out at `root`: its checklist
    /// when written, else the bare design it came with (`prototype/`, or an
    /// imported site in `reference/pages/` or `reference/`), else none.
    pub fn find(root: &Path) -> Option<Reference> {
        if let Ok(text) = std::fs::read_to_string(root.join(SIGNATURE))
            && let Ok(f) = serde_json::from_str::<File>(&text)
        {
            let screens = f.screens.iter().filter_map(|v| label(v, &["id", "name", "screen"])).collect();
            let items = f
                .items
                .iter()
                .filter_map(|v| {
                    let id = label(v, &["id", "name"])?;
                    let name = label(v, &["name", "title", "id"]).unwrap_or_else(|| id.clone());
                    Some(Item { id, name })
                })
                .collect();
            return Some(Reference { source: f.source.trim().to_owned(), screens, items });
        }
        for dir in ["prototype", "reference/pages", "reference"] {
            let d = root.join(dir);
            if is_dir(&d)
                && let Some(p) = page_in(&d)
            {
                let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                return Some(Reference { source: rel, screens: Vec::new(), items: Vec::new() });
            }
        }
        None
    }

    /// What the designer is told to do with it, in the design QA brief.
    pub fn brief(&self) -> String {
        let source = if self.source.is_empty() { "the person's own design".to_owned() } else { format!("`{}`", self.source) };
        let screens = if self.screens.is_empty() { "each screen and key state the reference shows".to_owned() } else { self.screens.join(", ") };
        let items = if self.items.is_empty() {
            String::new()
        } else {
            format!(
                "\nIts signature items (each must be in the build; a missing one is an S1 blocker; list the ids you saw in `signature`):\n{}\n",
                self.items.iter().map(|i| if i.name.is_empty() || i.name == i.id { format!("- {}", i.id) } else { format!("- {}: {}", i.id, i.name) }).collect::<Vec<_>>().join("\n")
            )
        };
        format!(
            "\n\nThis product is built from the person's own design, {source}: it is the spec for look and feel, not a mood board. \
Render the reference at the same widths and in the same states as the build (`equipe design render {src} --widths 390,1280`, or \
open it in your browser for states that need a click), put each pair side by side (`equipe design diff <reference.png> <built.png> --out \
handin-shots/<screen>-side.png`), and judge with both pictures, not the diff number. Screens to compare: {screens}. For each, give \
`side_by_side` an entry {{\"screen\": \"…\", \"same_feel\": true|false, \"why\": \"…\", \"pictures\": [\"handin-shots/<screen>-side.png\"]}}, \
its side-by-side picture also in `images`; a pass needs every one true, each judged from its own picture, never from the engineer's \
hand-in alone, and nothing written as \"not confirmed\" or \"didn't render\": what you couldn't check is not a pass. A screen that no \
longer feels like the reference (its structure, motion, mood or signature element replaced by a generic look or a kit component) is a blocker. \
Check the other half too: every equipe ui piece used for what the reference does not have (new screens, menus, selects, inputs, dialogs, \
toasts) restyled to the reference's tokens and at the quality bar: accessible, its states (hover, focus, empty, error, loading), no native \
select or date input.{items}",
            src = if self.source.is_empty() { "prototype/index.html" } else { self.source.as_str() },
        )
    }
}

/// One screen's side-by-side verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feel {
    pub screen: String,
    pub same: bool,
    pub why: String,
    /// The side-by-side picture(s) it was judged from: the reference and the
    /// build at the same width and state (`handin-shots/<screen>-side.png`).
    pub pictures: Vec<String>,
}

fn truthy(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "same" | "y" => Some(true),
            "false" | "no" | "different" | "n" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// `side_by_side` as the designer wrote it: objects, or lines like
/// `"lock: yes"` / `"meeting-space: no, the graph is gone"`.
pub fn read_feel(args: &Value) -> Vec<Feel> {
    let items: Vec<Value> = match args.get("side_by_side") {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(Value::Array(a)) => a,
            _ => s.split(['\n', ';']).map(|l| Value::String(l.to_owned())).collect(),
        },
        Some(Value::Object(m)) => m.iter().map(|(k, v)| serde_json::json!({ "screen": k, "same_feel": v })).collect(),
        _ => Vec::new(),
    };
    items
        .iter()
        .filter_map(|v| match v {
            Value::Object(m) => {
                let screen = label(v, &["screen", "name", "id"])?;
                let same = m.get("same_feel").or_else(|| m.get("same")).and_then(truthy).unwrap_or(false);
                let why = m.get("why").or_else(|| m.get("note")).and_then(Value::as_str).unwrap_or("").trim().to_owned();
                let pictures = match m.get("pictures").or_else(|| m.get("picture")).or_else(|| m.get("images")).or_else(|| m.get("image")) {
                    Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect(),
                    Some(Value::String(s)) => s.split([',', '\n']).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect(),
                    _ => Vec::new(),
                };
                Some(Feel { screen, same, why, pictures })
            }
            Value::String(line) => {
                let (screen, rest) = line.split_once(':')?;
                let rest = rest.trim();
                let word = rest.split(|c: char| !c.is_alphanumeric()).next().unwrap_or("");
                let same = truthy(&Value::String(word.to_owned())).unwrap_or(false);
                let screen = screen.trim();
                (!screen.is_empty()).then(|| Feel { screen: screen.to_owned(), same, why: rest.to_owned(), pictures: Vec::new() })
            }
            _ => None,
        })
        .collect()
}

/// What a designer writes when it judged without looking: a pass on a build
/// from the person's own design can't stand on any of these.
pub const HEDGES: [&str; 14] = [
    "not confirmed",
    "unconfirmed",
    "not looked at",
    "didn't look at",
    "did not look at",
    "didn't render",
    "did not render",
    "couldn't open",
    "could not open",
    "didn't compare",
    "did not compare",
    "not compared",
    "not verified",
    "unverified",
];

/// The hedges in a `design_qa` call's own words (its findings, summary,
/// polish and checked), lowercased.
pub fn hedges(args: &Value) -> Vec<String> {
    let mut text = String::new();
    for key in ["findings", "summary", "checked", "polish", "side_by_side"] {
        if let Some(v) = args.get(key) {
            text.push_str(&v.to_string().to_lowercase().replace('\u{2019}', "'"));
            text.push('\n');
        }
    }
    HEDGES.iter().filter(|h| text.contains(*h)).map(|h| (*h).to_owned()).collect()
}

fn base_name(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or(p).trim().to_ascii_lowercase()
}

fn norm(s: &str) -> String {
    s.trim().to_ascii_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect::<String>().split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

/// Why a pass can't stand against `reference`, if it can't: a side-by-side
/// verdict per screen, each the same feel, and every signature item seen.
///
/// The verdicts are only as good as what they were judged from, so each needs
/// a side-by-side picture of the designer's own (`own`: the file names of the
/// pictures it gave in `images`, not the engineer's), and a pass can't leave
/// anything "not confirmed" (`hedged`): an item it didn't see is not seen.
pub fn refuses(reference: &Reference, feel: &[Feel], seen: &[String], own: &[String], hedged: &[String]) -> Vec<String> {
    let mut missing = Vec::new();
    if feel.is_empty() {
        missing.push(format!(
            "the build put side by side with the person's own design{} at the same widths and states, with a \"same feel?\" verdict per screen in `side_by_side`",
            if reference.source.is_empty() { String::new() } else { format!(" ({})", reference.source) }
        ));
    } else {
        let have: Vec<String> = feel.iter().map(|f| norm(&f.screen)).collect();
        let unseen: Vec<&str> = reference.screens.iter().filter(|s| !have.contains(&norm(s))).map(String::as_str).collect();
        if !unseen.is_empty() {
            missing.push(format!("a side-by-side verdict for {} too", unseen.join(", ")));
        }
        let off: Vec<String> = feel.iter().filter(|f| !f.same).map(|f| if f.why.is_empty() { f.screen.clone() } else { format!("{} ({})", f.screen, f.why) }).collect();
        if !off.is_empty() {
            missing.push(format!("every screen to feel like the reference — not yet: {}; that is a blocker, so the verdict is changes", off.join(", ")));
        }
        let own: Vec<String> = own.iter().map(|p| base_name(p)).collect();
        let blind: Vec<&str> = feel.iter().filter(|f| !f.pictures.iter().any(|p| own.contains(&base_name(p)))).map(|f| f.screen.as_str()).collect();
        if !blind.is_empty() {
            missing.push(format!(
                "a side-by-side picture of your own for each verdict — the reference and the build at the same width and state, made with `equipe design diff`, named in its entry's `pictures` and given in `images` (the engineer's pictures don't count) — not yet for: {}",
                blind.join(", ")
            ));
        }
    }
    if !hedged.is_empty() {
        missing.push(format!(
            "nothing left unconfirmed on a build from the person's own design — you wrote \"{}\": open the reference and the build and look, or send it back for what you couldn't check",
            hedged.join("\", \"")
        ));
    }
    let seen: Vec<String> = seen.iter().map(|s| norm(s)).collect();
    let lost: Vec<&Item> = reference.items.iter().filter(|i| !seen.contains(&norm(&i.id)) && !seen.contains(&norm(&i.name))).collect();
    if !lost.is_empty() {
        missing.push(format!(
            "every signature item of the reference seen in the build (`signature`): not listed yet: {}. A missing one is an S1 blocker",
            lost.iter().map(|i| i.id.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("eq-ref-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_product_with_a_prototype_or_a_checklist_has_a_reference_and_one_without_has_none() {
        let none = tmp("none");
        assert_eq!(Reference::find(&none), None);
        let proto = tmp("proto");
        std::fs::create_dir_all(proto.join("prototype")).unwrap();
        std::fs::write(proto.join("prototype/index.html"), "<canvas></canvas>").unwrap();
        assert_eq!(Reference::find(&proto).unwrap().source, "prototype/index.html");
        std::fs::create_dir_all(proto.join("design/reference")).unwrap();
        std::fs::write(proto.join(SIGNATURE), json!({ "source": "prototype/index.html", "screens": ["grid", {"id": "lock"}], "items": [{"id": "orb", "name": "The orb with its confidence"}, "tabs"] }).to_string()).unwrap();
        let r = Reference::find(&proto).unwrap();
        assert_eq!(r.screens, vec!["grid", "lock"]);
        assert_eq!(r.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["orb", "tabs"]);
        assert!(r.brief().contains("orb: The orb with its confidence") && r.brief().contains("side_by_side"));
    }

    /// Each verdict given its own side-by-side picture, `<screen>-side.png`.
    fn shot(mut feel: Vec<Feel>) -> (Vec<Feel>, Vec<String>) {
        let mut own = Vec::new();
        for f in &mut feel {
            let p = format!("handin-shots/{}-side.png", norm(&f.screen));
            own.push(format!("/work/{p}"));
            f.pictures = vec![p];
        }
        (feel, own)
    }

    #[test]
    fn a_pass_needs_every_screen_side_by_side_feeling_the_same_and_every_signature_item() {
        let r = Reference { source: "prototype/index.html".into(), screens: vec!["grid".into(), "Meeting space".into()], items: vec![Item { id: "orb".into(), name: "Orb".into() }, Item { id: "canvas".into(), name: String::new() }] };
        // Render checks alone: refused.
        let why = refuses(&r, &[], &[], &[], &[]);
        assert!(why[0].contains("side by side") && why[1].contains("orb, canvas"), "{why:?}");
        // Objects and lines both read; a screen missing, one that feels different.
        let (feel, own) = shot(read_feel(&json!({ "side_by_side": [{"screen": "grid", "same_feel": true}, "meeting-space: no, the constellation is a list"] })));
        assert_eq!(feel.len(), 2);
        let why = refuses(&r, &feel, &["orb".into(), "Canvas".into()], &own, &[]);
        assert_eq!(why.len(), 1);
        assert!(why[0].contains("constellation is a list"), "{why:?}");
        // All the same, all seen, each from its own picture: no objection.
        let (feel, own) = shot(read_feel(&json!({ "side_by_side": { "grid": "yes", "meeting space": true } })));
        assert!(refuses(&r, &feel, &["orb".into(), "canvas".into()], &own, &[]).is_empty());
        // A screen the reference lists but nobody compared.
        let (feel, own) = shot(read_feel(&json!({ "side_by_side": [{"screen": "grid", "same_feel": "yes"}] })));
        assert!(refuses(&r, &feel, &["orb".into(), "canvas".into()], &own, &[])[0].contains("Meeting space"));
    }

    #[test]
    fn a_same_feel_verdict_judged_from_the_engineers_pictures_or_left_unconfirmed_is_no_pass() {
        // T-69 and T-70 on asap: "same feel" from the hand-in pictures, "not confirmed" for the rest, and passed.
        let r = Reference { source: "prototype/index.html".into(), screens: vec!["lock".into()], items: vec![Item { id: "orb".into(), name: "Orb".into() }] };
        let args = json!({
            "findings": "Passes. I did not render the built preview or the demo side by side myself. Not confirmed: the amber reschedule state.",
            "side_by_side": [{"screen": "lock", "same_feel": true, "pictures": ["handin-shots/t68/lock-390-2.png"]}],
            "signature": ["orb"]
        });
        let feel = read_feel(&args);
        assert_eq!(feel[0].pictures, vec!["handin-shots/t68/lock-390-2.png"]);
        // The engineer's picture isn't the designer's own.
        let why = refuses(&r, &feel, &["orb".into()], &["/w/design-qa/T-70/shots/create.png".into()], &[]);
        assert!(why.len() == 1 && why[0].contains("side-by-side picture of your own") && why[0].contains("lock"), "{why:?}");
        // Its own picture, but it wrote what it didn't check.
        let hedged = hedges(&args);
        assert_eq!(hedged, vec!["not confirmed", "did not render"]);
        let why = refuses(&r, &feel, &["orb".into()], &["/w/handin-shots/t68/lock-390-2.png".into()], &hedged);
        assert!(why.len() == 1 && why[0].contains("did not render"), "{why:?}");
        // Looked at everything itself: a pass stands.
        let clean = json!({ "findings": "Same feel on every screen.", "side_by_side": args["side_by_side"].clone() });
        assert!(hedges(&clean).is_empty() && hedges(&json!({ "findings": "I didn\u{2019}t render the demo" })) == vec!["didn't render"]);
        assert!(refuses(&r, &read_feel(&clean), &["orb".into()], &["/w/handin-shots/t68/lock-390-2.png".into()], &[]).is_empty());
    }
}
