//! agent-desktop's JSON, written for a model to read in as few tokens as it
//! can: one line per element, refs short, empty wrappers gone.
//!
//! ```text
//! TextEdit · "Untitled" · snapshot s8f3k2p9 · 4 refs
//! window "Untitled"
//!  e1 button "Bold"
//!  e2 checkbox "Wrap" checked
//!  e3 textfield "Body" ="Dear Kim," focused
//!  e4 group "Sidebar" +26
//! ```
//!
//! A ref prints as `e3`; the member passes it back as is (its last snapshot)
//! or as `s8f3k2p9:e3`. `+26` is a cut branch with 26 children: drill into it
//! with `root`.

use serde_json::Value;

/// At most this many lines of tree; past it, the rest is counted.
const MAX_LINES: usize = 400;
/// Names and values are cut to this many characters.
const MAX_TEXT: usize = 80;

/// States worth a model's attention. Others (`enabled`, `visible`…) are noise.
const STATES: [&str; 8] = ["focused", "disabled", "checked", "selected", "expanded", "secure", "indeterminate", "required"];

/// `@s8f3k2p9:e12` → `e12` (when it belongs to `snapshot`), else `s8f3k2p9:e12`.
pub fn short_ref(qualified: &str, snapshot: Option<&str>) -> String {
    let bare = qualified.trim_start_matches('@');
    match bare.split_once(':') {
        Some((sid, e)) if Some(sid) == snapshot => e.to_owned(),
        _ => bare.to_owned(),
    }
}

pub fn clip(text: &str, max: usize) -> String {
    let one_line: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let one_line = one_line.trim();
    if one_line.chars().count() <= max {
        return one_line.to_owned();
    }
    format!("{}…", one_line.chars().take(max).collect::<String>())
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

/// One element, in one line (no indent).
pub fn node_line(node: &Value, snapshot: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(r) = str_of(node, "ref_id") {
        parts.push(short_ref(r, snapshot));
    }
    parts.push(str_of(node, "role").unwrap_or("element").to_owned());
    if let Some(name) = str_of(node, "name") {
        parts.push(format!("{:?}", clip(name, MAX_TEXT)));
    } else if let Some(desc) = str_of(node, "description") {
        parts.push(format!("({})", clip(desc, MAX_TEXT)));
    }
    let secure = node.get("states").and_then(Value::as_array).is_some_and(|s| s.iter().any(|v| v == "secure"));
    if let Some(value) = str_of(node, "value").filter(|_| !secure) {
        parts.push(format!("={:?}", clip(value, MAX_TEXT)));
    }
    if let Some(states) = node.get("states").and_then(Value::as_array) {
        parts.extend(states.iter().filter_map(Value::as_str).filter(|s| STATES.contains(s)).map(str::to_owned));
    }
    if let Some(n) = node.get("children_count").and_then(Value::as_u64) {
        parts.push(format!("+{n}"));
    } else if node.get("subtree_truncated").and_then(Value::as_bool) == Some(true) {
        parts.push("+…".into());
    }
    if let Some(b) = node.get("bounds").filter(|b| b.is_object()) {
        let n = |k: &str| b.get(k).and_then(Value::as_f64).unwrap_or(0.0).round() as i64;
        parts.push(format!("@{},{} {}x{}", n("x"), n("y"), n("width"), n("height")));
    }
    parts.join(" ")
}

/// Whether a node says nothing of its own: no ref, no name, no value.
fn hollow(node: &Value) -> bool {
    ["ref_id", "name", "value", "description"].iter().all(|k| str_of(node, k).is_none()) && node.get("children_count").is_none()
}

fn walk(node: &Value, depth: usize, snapshot: Option<&str>, out: &mut Vec<String>, hidden: &mut usize) {
    let children = node.get("children").and_then(Value::as_array);
    // An empty wrapper adds a line and a level and nothing else: its children
    // take its place.
    let keep = !hollow(node) || children.is_none_or(|c| c.is_empty());
    let next = if keep {
        if out.len() < MAX_LINES {
            out.push(format!("{}{}", " ".repeat(depth), node_line(node, snapshot)));
        } else {
            *hidden += 1;
        }
        depth + 1
    } else {
        depth
    };
    for child in children.into_iter().flatten() {
        walk(child, next, snapshot, out, hidden);
    }
}

/// A snapshot's `data`, as lines.
pub fn snapshot(data: &Value) -> String {
    let sid = str_of(data, "snapshot_id");
    let mut head = Vec::new();
    if let Some(app) = str_of(data, "app") {
        head.push(app.to_owned());
    }
    if let Some(title) = data.get("window").and_then(|w| str_of(w, "title")) {
        head.push(format!("{:?}", clip(title, MAX_TEXT)));
    }
    if let Some(w) = data.get("window").and_then(|w| str_of(w, "id")) {
        head.push(w.to_owned());
    }
    if let Some(sid) = sid {
        head.push(format!("snapshot {sid}"));
    }
    if let Some(n) = data.get("ref_count").and_then(Value::as_u64) {
        head.push(format!("{n} refs"));
    }
    let mut lines = vec![head.join(" · ")];
    let mut hidden = 0;
    if let Some(tree) = data.get("tree") {
        walk(tree, 0, sid, &mut lines, &mut hidden);
    }
    if hidden > 0 {
        lines.push(format!("… {hidden} more: drill into a branch with `root`, or ask for a smaller `depth`."));
    }
    if data.get("complete").and_then(Value::as_bool) == Some(false) {
        lines.push("partial: the app took too long to read whole. Drill into a branch with `root`, or snapshot again with a longer `timeout_ms`.".into());
    }
    if data.get("identity_unproven").and_then(Value::as_bool) == Some(true) {
        lines.push("refs unproven: snapshot again with a longer `timeout_ms` before acting.".into());
    }
    if let Some(surfaces) = surfaces(data) {
        lines.push(surfaces);
    }
    lines.join("\n")
}

/// `find`'s matches, as lines.
pub fn matches(data: &Value) -> String {
    let sid = str_of(data, "snapshot_id");
    if let Some(n) = data.get("count").and_then(Value::as_u64) {
        return format!("{n} match{}", if n == 1 { "" } else { "es" });
    }
    let found: Vec<String> = data.get("matches").and_then(Value::as_array).into_iter().flatten().map(|m| node_line(m, sid)).collect();
    if found.is_empty() {
        let roles = data.get("roles_present").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "));
        return match roles {
            Some(roles) if !roles.is_empty() => format!("no match. Roles there: {roles}"),
            _ => "no match".into(),
        };
    }
    let mut head = String::from("matches");
    if let Some(sid) = sid {
        head.push_str(&format!(" · snapshot {sid}"));
    }
    std::iter::once(head).chain(found).collect::<Vec<_>>().join("\n")
}

fn surfaces(data: &Value) -> Option<String> {
    let list: Vec<String> = data
        .get("surfaces")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|s| {
            // agent-desktop's `SurfaceInfo`: `{ "id", "type": "menu", "title"? }`.
            let kind = s.as_str().or_else(|| str_of(s, "type")).or_else(|| str_of(s, "kind"))?;
            Some(match str_of(s, "title") {
                Some(title) => format!("{kind} {:?}", clip(title, MAX_TEXT)),
                None => kind.to_owned(),
            })
        })
        .collect();
    (!list.is_empty()).then(|| format!("open now: {} — snapshot it with `surface`.", list.join(", ")))
}

fn disposition(v: &Value) -> Option<(String, String)> {
    let d = v.get("disposition")?;
    Some((str_of(d, "delivery").unwrap_or("unknown").to_owned(), str_of(d, "retry").unwrap_or("unknown").to_owned()))
}

/// An action's `data`, in a line or two: what landed, what's open now.
pub fn action(what: &str, data: &Value) -> String {
    let mut line = what.to_owned();
    if let Some((delivery, _)) = disposition(data) {
        line.push_str(&format!(": {}", delivery.replace('_', " ")));
    } else {
        line.push_str(": done");
    }
    if let Some(post) = data.get("post_state") {
        if let Some(value) = str_of(post, "value") {
            line.push_str(&format!(" · now ={:?}", clip(value, MAX_TEXT)));
        }
        if let Some(states) = post.get("states").and_then(Value::as_array) {
            let shown: Vec<&str> = states.iter().filter_map(Value::as_str).filter(|s| STATES.contains(s)).collect();
            if !shown.is_empty() {
                line.push_str(&format!(" · {}", shown.join(" ")));
            }
        }
    }
    if let Some(open) = surfaces(data) {
        line.push('\n');
        line.push_str(&open);
    }
    line
}

/// An error envelope's `error`, in a line, with what to do next — retrying
/// only when agent-desktop says nothing was delivered.
pub fn error(err: &Value) -> String {
    let code = str_of(err, "code").unwrap_or("ERROR");
    let message = str_of(err, "message").map(|m| clip(m, 240)).unwrap_or_default();
    let mut line = format!("{code}: {message}");
    match disposition(err) {
        Some((delivery, retry)) if retry == "safe" => line.push_str(&format!(" ({}: safe to retry)", delivery.replace('_', " "))),
        Some((delivery, _)) => line.push_str(&format!(
            " ({}: it may have landed — read the state (snapshot or find) before you try again)",
            delivery.replace('_', " ")
        )),
        None => {}
    }
    let next = match code {
        "STALE_REF" | "SNAPSHOT_NOT_FOUND" => Some("Take a fresh snapshot and use its refs."),
        "AMBIGUOUS_TARGET" => Some("Act on a control that is unique in the window, or snapshot again with a longer timeout_ms."),
        "PERM_DENIED" => Some("The helper lacks a permission: ask the person to run `equipe desktop setup`."),
        "POLICY_DENIED" => Some("That needs physical input: call again with `headed: true` (it waits for the screen)."),
        "APP_NOT_FOUND" => Some("Launch it with `apps` (action launch)."),
        _ => None,
    };
    if let Some(next) = next {
        line.push(' ');
        line.push_str(next);
    } else if let Some(s) = str_of(err, "suggestion") {
        line.push(' ');
        line.push_str(&clip(s, 200));
    }
    line
}

/// `list-apps` data.
pub fn apps(data: &Value) -> String {
    let list = data.get("apps").or(Some(data)).and_then(Value::as_array);
    let lines: Vec<String> = list
        .into_iter()
        .flatten()
        .map(|a| {
            let mut parts = vec![str_of(a, "name").unwrap_or("?").to_owned()];
            if let Some(b) = str_of(a, "bundle_id") {
                parts.push(format!("({b})"));
            }
            if let Some(p) = str_of(a, "presentation") {
                parts.push(p.to_owned());
            }
            parts.join(" ")
        })
        .collect();
    if lines.is_empty() { "no apps".into() } else { lines.join("\n") }
}

/// `list-windows` data.
pub fn windows(data: &Value) -> String {
    let list = data.get("windows").or(Some(data)).and_then(Value::as_array);
    let lines: Vec<String> = list
        .into_iter()
        .flatten()
        .map(|w| {
            let mut parts = vec![str_of(w, "id").unwrap_or("?").to_owned(), str_of(w, "app_name").unwrap_or("?").to_owned()];
            if let Some(t) = str_of(w, "title") {
                parts.push(format!("{:?}", clip(t, MAX_TEXT)));
            }
            if w.get("is_focused").and_then(Value::as_bool) == Some(true) {
                parts.push("focused".into());
            }
            if w.get("accessible").and_then(Value::as_bool) == Some(false) {
                parts.push("no-accessibility".into());
            }
            parts.join(" ")
        })
        .collect();
    if lines.is_empty() { "no windows".into() } else { lines.join("\n") }
}

/// `list-notifications` data.
pub fn notifications(data: &Value) -> String {
    let list = data.get("notifications").or(Some(data)).and_then(Value::as_array);
    let lines: Vec<String> = list
        .into_iter()
        .flatten()
        .map(|n| {
            let mut line = format!("{} {}", n.get("index").and_then(Value::as_u64).unwrap_or(0), str_of(n, "app_name").unwrap_or("?"));
            if let Some(t) = str_of(n, "title") {
                line.push_str(&format!(" {:?}", clip(t, MAX_TEXT)));
            }
            if let Some(b) = str_of(n, "body") {
                line.push_str(&format!(": {}", clip(b, 160)));
            }
            let actions: Vec<&str> = n.get("actions").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
            if !actions.is_empty() {
                line.push_str(&format!(" [{}]", actions.join(", ")));
            }
            line
        })
        .collect();
    if lines.is_empty() { "no notifications".into() } else { lines.join("\n") }
}

/// Anything else: the data as compact JSON, cut short.
pub fn generic(data: &Value) -> String {
    clip(&serde_json::to_string(data).unwrap_or_default(), 2000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_snapshot_reads_one_line_per_element_with_short_refs() {
        let data = json!({
            "app": "TextEdit", "snapshot_id": "s8f3k2p9", "ref_count": 3, "complete": true,
            "window": { "id": "w-1", "title": "Untitled" },
            "tree": { "role": "window", "name": "Untitled", "children": [
                { "role": "group", "children": [
                    { "ref_id": "@s8f3k2p9:e1", "role": "button", "name": "Bold", "available_actions": ["Click"] },
                    { "ref_id": "@s8f3k2p9:e2", "role": "textfield", "name": "Body", "value": "Dear Kim,\nsee you", "states": ["focused", "enabled"] }
                ]},
                { "ref_id": "@s8f3k2p9:e3", "role": "group", "name": "Sidebar", "children_count": 26 }
            ]}
        });
        assert_eq!(
            snapshot(&data),
            "TextEdit · \"Untitled\" · w-1 · snapshot s8f3k2p9 · 3 refs\n\
             window \"Untitled\"\n \
             e1 button \"Bold\"\n \
             e2 textfield \"Body\" =\"Dear Kim, see you\" focused\n \
             e3 group \"Sidebar\" +26"
        );
    }

    #[test]
    fn a_password_fields_value_is_never_shown() {
        let node = json!({ "ref_id": "@s1:e4", "role": "textfield", "name": "Password", "value": "••••", "states": ["secure"] });
        assert_eq!(node_line(&node, Some("s1")), "e4 textfield \"Password\" secure");
    }

    #[test]
    fn errors_say_whether_a_retry_is_safe() {
        let stale = json!({ "code": "STALE_REF", "message": "@s1:e7 not found", "disposition": { "delivery": "not_delivered", "retry": "safe" } });
        assert_eq!(error(&stale), "STALE_REF: @s1:e7 not found (not delivered: safe to retry) Take a fresh snapshot and use its refs.");
        let maybe = json!({ "code": "TIMEOUT", "message": "late", "disposition": { "delivery": "delivery_uncertain", "retry": "unsafe" } });
        assert!(error(&maybe).contains("it may have landed"), "{}", error(&maybe));
    }

    #[test]
    fn an_action_says_what_landed_and_what_opened() {
        let data = json!({ "action": "click", "disposition": { "delivery": "delivered_verified", "retry": "unsafe" }, "surfaces": [{ "id": "m1", "type": "menu" }] });
        assert_eq!(action("click e5", &data), "click e5: delivered verified\nopen now: menu — snapshot it with `surface`.");
    }
}
