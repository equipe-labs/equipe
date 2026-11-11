//! Members working desktop apps through the accessibility tree, with
//! [agent-desktop](https://github.com/vercel-labs/agent-desktop) (Apache-2.0)
//! as the engine.
//!
//! A member reads an app as a compact tree with a ref on everything it can act
//! on (`snapshot`, `find`), acts on a ref (`act`: click, type, set a value,
//! select, toggle, scroll…), and reads again to check. Ref actions are
//! *headless*: Accessibility calls, no pointer, no keystrokes, no focus taken —
//! so several members can work different apps at once, beside the person, and
//! the daemon keeps its one-at-a-time screen lease only for physical input
//! (`headed`, and the screenshot tools in [`crate::controller`]).
//!
//! The engine runs as a child of the desktop helper ([`cli`]), under the
//! helper's one Accessibility grant. For the send gate, the helper remembers
//! what each ref was when the member's snapshot showed it — app, window, role,
//! name — and answers the daemon's `_scene` from that: the gate judges a ref
//! action by code, before it happens. agent-desktop itself re-identifies the
//! element before acting and refuses (`STALE_REF`) if it is no longer that
//! element, so what the gate judged is what gets acted on.
//!
//! Nothing a snapshot, the clipboard or a notification shows is logged here:
//! it goes back to the member as the tool's answer, and that is all.

pub mod cli;
pub mod compact;
pub mod pin;

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};

pub use cli::{Cli, Runner};

/// The engine's member tools (`mcp__desktop__…`).
pub const TOOLS: [&str; 8] = ["snapshot", "find", "act", "apps", "windows", "wait", "notifications", "clipboard"];

/// Whether `tool` is one of the engine's.
pub fn handles(tool: &str) -> bool {
    TOOLS.contains(&tool)
}

/// What `act` does, by `action`.
pub const ACTIONS: [&str; 15] = [
    "click",
    "right_click",
    "double_click",
    "type",
    "set_value",
    "clear",
    "focus",
    "select",
    "check",
    "uncheck",
    "toggle",
    "expand",
    "collapse",
    "scroll",
    "scroll_to",
];

/// Snapshots whose refs the helper keeps per member, newest last.
const KEPT_SNAPSHOTS: usize = 12;
/// The longest a member may make `wait` wait.
const MAX_WAIT_MS: u64 = 60_000;

/// One call's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub text: String,
    pub is_error: bool,
}

impl Answer {
    fn ok(text: impl Into<String>) -> Self {
        Self { text: text.into(), is_error: false }
    }

    fn refuse(text: impl Into<String>) -> Self {
        Self { text: text.into(), is_error: true }
    }
}

/// What one ref was when a snapshot showed it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RefInfo {
    role: String,
    name: Option<String>,
    description: Option<String>,
    value: Option<String>,
    secure: bool,
}

/// What a snapshot was of.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Snap {
    app: Option<String>,
    window: Option<String>,
    window_id: Option<String>,
    /// The text field most likely holding a draft: focused, or the last one with text.
    draft: Option<String>,
}

#[derive(Default)]
struct Member {
    last: Option<String>,
    order: VecDeque<String>,
    snaps: HashMap<String, Snap>,
    refs: HashMap<String, RefInfo>,
    /// The last notification listing: index → (app, title).
    notifications: Vec<(u64, String, Option<String>)>,
}

impl Member {
    fn forget_old(&mut self) {
        while self.order.len() > KEPT_SNAPSHOTS {
            if let Some(old) = self.order.pop_front() {
                self.snaps.remove(&old);
                let prefix = format!("@{old}:");
                self.refs.retain(|r, _| !r.starts_with(&prefix));
            }
        }
    }
}

/// The engine as the helper uses it. Safe to share across threads: calls of
/// different members run at the same time.
pub struct Engine {
    runner: Box<dyn Runner>,
    members: Mutex<HashMap<String, Member>>,
    /// App name → bundle id, from `list-apps`: the send gate knows messaging
    /// apps by bundle id when their shown name is localised.
    bundles: Mutex<HashMap<String, Option<String>>>,
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// `--flag=value`: one argument, so a value starting with `-` can't read as a flag.
fn kv(flag: &str, value: impl std::fmt::Display) -> String {
    format!("{flag}={value}")
}

fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true"))).unwrap_or(false)
}

fn number(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64)).or_else(|| v.as_str()?.trim().parse().ok()))
}

/// agent-desktop's session for `member`: its own snapshot namespace, named
/// by a hash so no squad or member name lands on disk.
pub fn session_of(member: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in member.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("eq-{hash:016x}")
}

/// The role the send gate (`desktop_gate`, written for macOS AX roles) reads.
pub fn gate_role(role: &str, secure: bool) -> String {
    let ax = match role {
        "textfield" | "textbox" | "searchfield" | "textarea" if secure => "AXSecureTextField",
        "textfield" | "textbox" | "searchfield" => "AXTextField",
        "textarea" => "AXTextArea",
        "button" => "AXButton",
        "menubutton" => "AXMenuButton",
        "combobox" => "AXPopUpButton",
        "checkbox" | "switch" => "AXCheckBox",
        "radiobutton" => "AXRadioButton",
        "menuitem" => "AXMenuItem",
        "link" => "AXLink",
        "tab" => "AXRadioButton",
        other => {
            let mut chars = other.chars();
            return match chars.next() {
                Some(first) => format!("AX{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => "AXUnknown".into(),
            };
        }
    };
    ax.to_owned()
}

/// `@s8f3k2p9:e12` from what a member wrote (`e12`, `@e12`, `s8f3k2p9:e12`),
/// a bare one meaning its last snapshot.
fn qualify(raw: &str, last: Option<&str>) -> Result<String, String> {
    let bare = raw.trim().trim_start_matches('@');
    let (sid, e) = match bare.split_once(':') {
        Some((sid, e)) => (sid.to_owned(), e),
        None => (last.ok_or("Take a snapshot first: refs come from your snapshots.")?.to_owned(), bare),
    };
    let ok_sid = !sid.is_empty() && sid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let ok_e = e.len() > 1 && e.starts_with('e') && e[1..].chars().all(|c| c.is_ascii_digit());
    if !ok_sid || !ok_e {
        return Err(format!("{raw:?} isn't a ref: use one a snapshot printed, like e12 or s8f3k2p9:e12."));
    }
    Ok(format!("@{sid}:{e}"))
}

impl Engine {
    pub fn new(runner: Box<dyn Runner>) -> Self {
        Self { runner, members: Mutex::new(HashMap::new()), bundles: Mutex::new(HashMap::new()) }
    }

    fn members(&self) -> std::sync::MutexGuard<'_, HashMap<String, Member>> {
        self.members.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The member's last snapshot id: what a bare `e12` means.
    fn last(&self, member: &str) -> Option<String> {
        self.members().get(member).and_then(|m| m.last.clone())
    }

    /// Run agent-desktop for `member`; the envelope's `data`, or the refusal in words.
    fn run(&self, member: &str, mut args: Vec<String>, stdin: Option<&str>, timeout: Duration) -> Result<Value, String> {
        args.insert(0, kv("--session", session_of(member)));
        let envelope = self.runner.run(&args, stdin, timeout)?;
        if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
            return Ok(envelope.get("data").cloned().unwrap_or(Value::Null));
        }
        Err(envelope.get("error").map(compact::error).unwrap_or_else(|| "agent-desktop failed without saying why".into()))
    }

    /// The engine's state for `_status`: installed, and what macOS lets it do
    /// (as a child of the helper, under the helper's grant).
    pub fn status(&self) -> Value {
        let mut status = json!({ "name": "agent-desktop", "version": pin::VERSION });
        match self.runner.run(&["permissions".into()], None, Duration::from_secs(10)) {
            Ok(envelope) if envelope.get("ok").and_then(Value::as_bool) == Some(true) => {
                status["installed"] = json!(true);
                let data = &envelope["data"];
                for key in ["accessibility", "screen_recording"] {
                    if let Some(state) = data.get(key).and_then(|p| p.get("state")).and_then(Value::as_str) {
                        status[key] = json!(state);
                    }
                }
            }
            Ok(_) => status["installed"] = json!(true),
            Err(err) => {
                status["installed"] = json!(err != cli::NOT_INSTALLED);
                status["problem"] = json!(err);
            }
        }
        status
    }

    /// Run `tool` for `member`. `_expect` in `args` (the scene the daemon's
    /// gate judged) makes an input go ahead only while it still holds.
    pub fn call(&self, member: &str, tool: &str, args: &Value) -> Answer {
        if let Some(expected) = args.get(crate::controller::EXPECT)
            && let Err(refused) = self.still(member, tool, args, expected)
        {
            return Answer::refuse(refused);
        }
        let result = match tool {
            "snapshot" => self.snapshot(member, args),
            "find" => self.find(member, args),
            "act" => self.act(member, args),
            "apps" => self.apps(member, args),
            "windows" => self.windows(member, args),
            "wait" => self.wait(member, args),
            "notifications" => self.notifications(member, args),
            "clipboard" => self.clipboard(member, args),
            other => Err(format!("There is no desktop tool called {other}.")),
        };
        match result {
            Ok(text) => Answer::ok(text),
            Err(text) => Answer::refuse(text),
        }
    }

    fn snapshot(&self, member: &str, args: &Value) -> Result<String, String> {
        let mut cli = vec!["snapshot".to_owned()];
        let root = match text(args, "root") {
            Some(raw) => Some(qualify(raw, self.last(member).as_deref())?),
            None => None,
        };
        match &root {
            Some(root) => cli.push(kv("--root", root)),
            None => {
                if let Some(app) = text(args, "app") {
                    cli.push(kv("--app", app.to_owned()));
                }
                if let Some(window) = text(args, "window") {
                    cli.push(kv("--window-id", window.to_owned()));
                }
                if let Some(surface) = text(args, "surface") {
                    cli.push(kv("--surface", surface.to_owned()));
                }
                if flag(args, "skeleton") {
                    cli.push("--skeleton".into());
                }
            }
        }
        if !flag(args, "all") {
            cli.push("-i".into());
        }
        cli.push("--compact".into());
        if let Some(depth) = number(args, "depth") {
            cli.push(kv("--max-depth", depth.clamp(1, 30).to_string()));
        }
        if flag(args, "bounds") {
            cli.push("--include-bounds".into());
        }
        let timeout = number(args, "timeout_ms").map(|t| t.clamp(500, 30_000));
        if let Some(t) = timeout {
            cli.push(kv("--timeout-ms", t.to_string()));
        }
        let data = self.run(member, cli, None, Duration::from_millis(timeout.unwrap_or(3000) + 20_000))?;
        self.remember(member, &data, text(args, "app"), root.as_deref());
        Ok(compact::snapshot(&data))
    }

    fn find(&self, member: &str, args: &Value) -> Result<String, String> {
        let mut cli = vec!["find".to_owned()];
        let root = match text(args, "root") {
            Some(raw) => Some(qualify(raw, self.last(member).as_deref())?),
            None => None,
        };
        match (&root, text(args, "app")) {
            (Some(root), _) => cli.push(kv("--root", root)),
            (None, Some(app)) => {
                cli.push(kv("--app", app.to_owned()));
                if let Some(window) = text(args, "window") {
                    cli.push(kv("--window-id", window.to_owned()));
                }
                if let Some(surface) = text(args, "surface") {
                    cli.push(kv("--surface", surface.to_owned()));
                }
            }
            (None, None) => return Err("Say where to look: `app`, or `root` (a ref from a snapshot).".into()),
        }
        for key in ["role", "name", "text", "value", "description"] {
            if let Some(v) = text(args, key) {
                cli.push(format!("--{key}={v}"));
            }
        }
        if flag(args, "exact") {
            cli.push("--exact".into());
        }
        cli.push(kv("--limit", number(args, "limit").unwrap_or(10).clamp(1, 50).to_string()));
        let data = self.run(member, cli, None, Duration::from_secs(30))?;
        self.remember(member, &data, text(args, "app"), root.as_deref());
        Ok(compact::matches(&data))
    }

    /// Keep what `data` (a snapshot or a find) shows: each ref's role and
    /// name, and which app and window its snapshot was of.
    fn remember(&self, member: &str, data: &Value, app: Option<&str>, root: Option<&str>) {
        let Some(sid) = data.get("snapshot_id").and_then(Value::as_str).map(str::to_owned) else { return };
        let mut members = self.members();
        let state = members.entry(member.to_owned()).or_default();
        // A drill keeps its snapshot's app and window.
        let inherited = root.and_then(|r| r.trim_start_matches('@').split_once(':')).and_then(|(s, _)| state.snaps.get(s).cloned());
        let mut snap = state.snaps.get(&sid).cloned().or(inherited).unwrap_or_default();
        if let Some(name) = data.get("app").and_then(Value::as_str).filter(|s| !s.is_empty()).or(app) {
            snap.app = Some(name.to_owned());
        }
        if let Some(window) = data.get("window") {
            snap.window = window.get("title").and_then(Value::as_str).map(str::to_owned).or(snap.window);
            snap.window_id = window.get("id").and_then(Value::as_str).map(str::to_owned).or(snap.window_id);
        }
        let mut nodes = Vec::new();
        if let Some(tree) = data.get("tree") {
            collect(tree, &mut nodes);
        }
        if let Some(found) = data.get("matches").and_then(Value::as_array) {
            nodes.extend(found.iter());
        }
        let mut draft: Option<(bool, String)> = None;
        for node in nodes {
            let Some(r) = node.get("ref_id").and_then(Value::as_str) else { continue };
            let states: Vec<&str> = node.get("states").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
            let info = RefInfo {
                role: node.get("role").and_then(Value::as_str).unwrap_or_default().to_owned(),
                name: node.get("name").and_then(Value::as_str).map(str::to_owned),
                description: node.get("description").and_then(Value::as_str).map(str::to_owned),
                value: node.get("value").and_then(Value::as_str).map(str::to_owned),
                secure: states.contains(&"secure"),
            };
            let editable = matches!(info.role.as_str(), "textfield" | "textarea" | "textbox") && !info.secure;
            let focused = states.contains(&"focused");
            if editable && (focused || info.value.as_deref().is_some_and(|v| !v.trim().is_empty())) && draft.as_ref().is_none_or(|(f, _)| !f || focused) {
                draft = Some((focused, r.to_owned()));
            }
            state.refs.insert(r.to_owned(), info);
        }
        if let Some((_, d)) = draft {
            snap.draft = Some(d);
        }
        state.snaps.insert(sid.clone(), snap);
        state.order.retain(|s| *s != sid);
        state.order.push_back(sid.clone());
        state.last = Some(sid);
        state.forget_old();
    }

    /// The ref a call means, with what it was and what its snapshot was of.
    fn lookup(&self, member: &str, args: &Value) -> Result<(String, RefInfo, Snap), String> {
        let raw = text(args, "ref").ok_or("Say which element with `ref` (from your snapshot, like e12).")?;
        let members = self.members();
        let state = members.get(member);
        let qualified = qualify(raw, state.and_then(|m| m.last.as_deref()))?;
        let found = state.and_then(|m| {
            let sid = qualified.trim_start_matches('@').split_once(':')?.0;
            Some((m.refs.get(&qualified)?.clone(), m.snaps.get(sid)?.clone()))
        });
        let Some((info, snap)) = found else {
            return Err(format!("{} isn't in any snapshot you took here: take a snapshot and use its refs.", compact::short_ref(&qualified, None)));
        };
        if snap.app.is_none() {
            return Err("equipe can't tell which app that ref is in: snapshot it with `app` first.".into());
        }
        Ok((qualified, info, snap))
    }

    fn bundle_of(&self, member: &str, app: &str) -> Option<String> {
        if let Some(known) = self.bundles.lock().unwrap_or_else(|e| e.into_inner()).get(app) {
            return known.clone();
        }
        let data = self.run(member, vec!["list-apps".into(), kv("--app", app)], None, Duration::from_secs(10)).ok()?;
        let list = data.get("apps").or(Some(&data)).and_then(Value::as_array).cloned().unwrap_or_default();
        let bundle = list
            .iter()
            .find(|a| a.get("name").and_then(Value::as_str).is_some_and(|n| n.eq_ignore_ascii_case(app)))
            .or(list.first())
            .and_then(|a| a.get("bundle_id").and_then(Value::as_str))
            .map(str::to_owned);
        self.bundles.lock().unwrap_or_else(|e| e.into_inner()).insert(app.to_owned(), bundle.clone());
        bundle
    }

    /// What `tool` with `args` would act on, read from the member's snapshots
    /// without acting: JSON in the shape of `equipe_core::desktop_gate::Scene`.
    pub fn scene(&self, member: &str, tool: &str, args: &Value) -> Result<Value, String> {
        match tool {
            "act" => {
                let (qualified, info, snap) = self.lookup(member, args)?;
                let app = snap.app.clone().unwrap_or_default();
                let mut scene = json!({ "app": { "name": app }, "ref": qualified });
                if let Some(bundle) = self.bundle_of(member, &app) {
                    scene["app"]["bundle_id"] = json!(bundle);
                }
                if let Some(window) = &snap.window {
                    scene["window"] = json!(window);
                }
                let element = element_json(&info);
                if matches!(text(args, "action").unwrap_or("click"), "type" | "set_value" | "clear") {
                    scene["focused"] = element.clone();
                    scene["target"] = element;
                } else {
                    scene["target"] = element;
                    let draft = snap.draft.as_ref().filter(|d| **d != qualified);
                    let members = self.members();
                    if let Some((d, field)) = draft.and_then(|d| Some((d, members.get(member)?.refs.get(d)?))) {
                        scene["focused"] = element_json(field);
                        scene["draft_ref"] = json!(d);
                    }
                }
                Ok(scene)
            }
            "notifications" => {
                let index = number(args, "index").ok_or("Say which notification with `index` (from a listing).")?;
                let members = self.members();
                let (_, app, title) = members
                    .get(member)
                    .and_then(|m| m.notifications.iter().find(|(i, ..)| *i == index))
                    .cloned()
                    .ok_or("List the notifications first (action list): equipe acts only on one it has shown you.")?;
                drop(members);
                let mut scene = json!({ "app": { "name": app } });
                if let Some(bundle) = self.bundle_of(member, &app) {
                    scene["app"]["bundle_id"] = json!(bundle);
                }
                if let Some(title) = title {
                    scene["window"] = json!(title);
                }
                if let Some(name) = text(args, "name") {
                    scene["target"] = json!({ "role": "AXButton", "title": name });
                }
                Ok(scene)
            }
            _ => Err(format!("{tool} acts on nothing the send gate checks.")),
        }
    }

    /// Whether the screen is still what the daemon's gate judged. The ref
    /// itself is re-identified by agent-desktop; after the person said yes,
    /// the conversation (the window's title) and the draft are read again live.
    fn still(&self, member: &str, tool: &str, args: &Value, expected: &Value) -> Result<(), String> {
        const CHANGED: &str = "The screen changed since equipe checked this input (another app, conversation, element or draft): nothing done. Snapshot again and call again.";
        let now = self.scene(member, tool, args)?;
        for key in ["app", "ref", "target"] {
            let mut want = expected.get(key).cloned();
            let mut got = now.get(key).cloned();
            for v in [&mut want, &mut got] {
                if let Some(o) = v.as_mut().and_then(Value::as_object_mut) {
                    o.remove("value");
                }
            }
            if want != got {
                return Err(CHANGED.into());
            }
        }
        if !flag(args, crate::controller::ASKED) {
            return Ok(());
        }
        let app = expected["app"]["name"].as_str().unwrap_or_default();
        if let Some(window) = expected.get("window").and_then(Value::as_str) {
            let data = self.run(member, vec!["list-windows".into(), kv("--app", app)], None, Duration::from_secs(10))?;
            let list = data.get("windows").or(Some(&data)).and_then(Value::as_array).cloned().unwrap_or_default();
            if !list.iter().any(|w| w.get("title").and_then(Value::as_str) == Some(window)) {
                return Err(CHANGED.into());
            }
        }
        if let (Some(draft), Some(want)) = (expected.get("draft_ref").and_then(Value::as_str), expected["focused"].get("value").and_then(Value::as_str)) {
            let data = self.run(member, vec!["get".into(), "--property=value".into(), "--".into(), draft.to_owned()], None, Duration::from_secs(10))?;
            let got = data.get("value").and_then(Value::as_str).or_else(|| data.get("result").and_then(Value::as_str));
            if got != Some(want) {
                return Err(CHANGED.into());
            }
        }
        Ok(())
    }

    fn act(&self, member: &str, args: &Value) -> Result<String, String> {
        let action = text(args, "action").unwrap_or("click");
        if !ACTIONS.contains(&action) {
            return Err(format!("No action {action:?}: one of {}.", ACTIONS.join(", ")));
        }
        let (qualified, _, _) = self.lookup(member, args)?;
        let headed = flag(args, "headed") || action == "double_click";
        let mut cli = Vec::new();
        if headed {
            cli.push("--headed".into());
        }
        cli.push(action.replace('_', "-"));
        let mut stdin = None;
        let mut option = None;
        match action {
            "type" | "set_value" => {
                let words = args.get("text").or_else(|| args.get("value")).and_then(Value::as_str).ok_or("Give the `text` to write.")?;
                // Never in the arguments: any process can read those.
                cli.push("--stdin".into());
                stdin = Some(words.to_owned());
            }
            "select" => option = Some(text(args, "value").or_else(|| text(args, "text")).ok_or("Give the option to pick as `value`.")?.to_owned()),
            "scroll" => {
                let direction = text(args, "direction").unwrap_or("down");
                if !["up", "down", "left", "right"].contains(&direction) {
                    return Err(format!("No direction {direction:?}: up, down, left or right."));
                }
                cli.push(kv("--direction", direction));
                cli.push(kv("--amount", number(args, "amount").unwrap_or(3).clamp(1, 50)));
            }
            _ => {}
        }
        // Past `--` nothing reads as a flag, whatever the member wrote.
        cli.push("--".into());
        cli.push(qualified.clone());
        cli.extend(option);
        let data = self.run(member, cli, stdin.as_deref(), Duration::from_secs(40))?;
        let what = format!("{} {}", action.replace('_', " "), compact::short_ref(&qualified, self.last(member).as_deref()));
        Ok(compact::action(&what, &data))
    }

    fn apps(&self, member: &str, args: &Value) -> Result<String, String> {
        match text(args, "action").unwrap_or("list") {
            "list" => {
                let mut cli = vec!["list-apps".to_owned()];
                if let Some(app) = text(args, "app") {
                    cli.push(kv("--app", app.to_owned()));
                }
                self.run(member, cli, None, Duration::from_secs(15)).map(|d| compact::apps(&d))
            }
            "launch" => {
                let app = text(args, "app").ok_or("Say which app with `app`.")?;
                let mut cli = vec!["launch".to_owned()];
                if flag(args, "activate") {
                    cli.push("--activate".into());
                }
                cli.extend(["--".to_owned(), app.to_owned()]);
                let data = self.run(member, cli, None, Duration::from_secs(45))?;
                let mut line = format!("{} running", data.get("app").and_then(Value::as_str).unwrap_or(app));
                match data.get("window") {
                    Some(w) => line.push_str(&format!(" · window {} {:?}", w["id"].as_str().unwrap_or("?"), compact::clip(w["title"].as_str().unwrap_or(""), 80))),
                    None => line.push_str(" · no window yet: `wait` for one (event window-opened), or launch with activate"),
                }
                Ok(line)
            }
            "close" => {
                let app = text(args, "app").ok_or("Say which app with `app`.")?;
                let data = self.run(member, vec!["close-app".into(), "--".into(), app.to_owned()], None, Duration::from_secs(30))?;
                Ok(compact::action(&format!("close {app}"), &data))
            }
            other => Err(format!("No apps action {other:?}: list, launch or close.")),
        }
    }

    fn windows(&self, member: &str, args: &Value) -> Result<String, String> {
        let action = text(args, "action").unwrap_or("list");
        let mut cli = vec![match action {
            "list" => "list-windows",
            "focus" => "focus-window",
            "minimize" => "minimize",
            "maximize" => "maximize",
            "restore" => "restore",
            "resize" => "resize-window",
            "move" => "move-window",
            other => return Err(format!("No windows action {other:?}: list, focus, minimize, maximize, restore, resize or move.")),
        }
        .to_owned()];
        if let Some(app) = text(args, "app") {
            cli.push(kv("--app", app.to_owned()));
        }
        if let Some(window) = text(args, "window").filter(|_| action != "list") {
            cli.push(kv("--window-id", window.to_owned()));
        }
        for (key, flag) in [("width", "--width"), ("height", "--height"), ("x", "--x"), ("y", "--y")] {
            if let Some(n) = number(args, key) {
                cli.push(format!("{flag}={n}"));
            }
        }
        let data = self.run(member, cli, None, Duration::from_secs(20))?;
        Ok(if action == "list" { compact::windows(&data) } else { compact::action(action, &data) })
    }

    fn wait(&self, member: &str, args: &Value) -> Result<String, String> {
        let timeout = number(args, "timeout_ms").unwrap_or(10_000).clamp(100, MAX_WAIT_MS);
        let mut cli = vec!["wait".to_owned()];
        if let Some(ms) = number(args, "ms") {
            cli.extend(["--".to_owned(), ms.min(MAX_WAIT_MS).to_string()]);
        } else {
            let last = self.last(member);
            if let Some(raw) = text(args, "ref") {
                cli.push(kv("--element", qualify(raw, last.as_deref())?));
                if let Some(p) = text(args, "predicate") {
                    cli.push(kv("--predicate", p.to_owned()));
                }
                if let Some(v) = text(args, "value") {
                    cli.push(kv("--value", v.to_owned()));
                }
            } else if let Some(event) = text(args, "event") {
                cli.push(kv("--event", event.to_owned()));
                if let Some(w) = text(args, "window") {
                    cli.push(kv("--window", w.to_owned()));
                }
            } else if let Some(w) = text(args, "window") {
                cli.push(kv("--window", w.to_owned()));
            } else if let Some(t) = text(args, "text") {
                cli.push(kv("--text", t.to_owned()));
            } else if flag(args, "menu") {
                cli.push("--menu".into());
            } else {
                return Err("Say what to wait for: `ms`, `ref`, `event`, `window`, `text` or `menu`.".into());
            }
            if let Some(app) = text(args, "app") {
                cli.push(kv("--app", app.to_owned()));
            }
            cli.push(kv("--timeout", timeout.to_string()));
        }
        let data = self.run(member, cli, None, Duration::from_millis(timeout + 15_000))?;
        Ok(format!("waited: {}", compact::generic(&data)))
    }

    fn notifications(&self, member: &str, args: &Value) -> Result<String, String> {
        match text(args, "action").unwrap_or("list") {
            "list" => {
                let mut cli = vec!["list-notifications".to_owned()];
                for key in ["app", "text"] {
                    if let Some(v) = text(args, key) {
                        cli.push(format!("--{key}={v}"));
                    }
                }
                cli.push(kv("--limit", number(args, "limit").unwrap_or(10).clamp(1, 50).to_string()));
                let data = self.run(member, cli, None, Duration::from_secs(20))?;
                let list = data.get("notifications").or(Some(&data)).and_then(Value::as_array).cloned().unwrap_or_default();
                let seen = list
                    .iter()
                    .filter_map(|n| {
                        Some((n.get("index")?.as_u64()?, n.get("app_name")?.as_str()?.to_owned(), n.get("title").and_then(Value::as_str).map(str::to_owned)))
                    })
                    .collect();
                self.members().entry(member.to_owned()).or_default().notifications = seen;
                Ok(compact::notifications(&data))
            }
            action @ ("act" | "dismiss") => {
                let index = number(args, "index").ok_or("Say which notification with `index`.")?;
                let app = self
                    .members()
                    .get(member)
                    .and_then(|m| m.notifications.iter().find(|(i, ..)| *i == index).map(|(_, app, _)| app.clone()))
                    .ok_or("List the notifications first (action list).")?;
                // The listing's app is the fingerprint: if the notification at
                // that index changed, agent-desktop presses nothing.
                let mut cli = vec!["--headed".to_owned()];
                if action == "act" {
                    let name = text(args, "name").ok_or("Say which button with `name` (from the listing's [..]).")?;
                    cli.extend(["notification-action".into(), kv("--expected-app", app), "--".into(), index.to_string(), name.to_owned()]);
                } else {
                    cli.extend(["dismiss-notification".into(), kv("--expected-app", app), "--".into(), index.to_string()]);
                }
                let data = self.run(member, cli, None, Duration::from_secs(20))?;
                Ok(compact::action(&format!("{action} notification {index}"), &data))
            }
            other => Err(format!("No notifications action {other:?}: list, act or dismiss.")),
        }
    }

    fn clipboard(&self, member: &str, args: &Value) -> Result<String, String> {
        match text(args, "action").unwrap_or("get") {
            "get" => {
                let data = self.run(member, vec!["clipboard-get".into(), "--format=text".into()], None, Duration::from_secs(10))?;
                Ok(match data.get("text").and_then(Value::as_str) {
                    Some(t) => format!("clipboard: {t}"),
                    None => "the clipboard holds no text".into(),
                })
            }
            "set" => {
                let words = args.get("text").and_then(Value::as_str).ok_or("Give the `text` to put on the clipboard.")?;
                let data = self.run(member, vec!["clipboard-set".into(), "--stdin".into()], Some(words), Duration::from_secs(10))?;
                Ok(compact::action("clipboard set", &data))
            }
            "clear" => {
                let data = self.run(member, vec!["clipboard-clear".into()], None, Duration::from_secs(10))?;
                Ok(compact::action("clipboard cleared", &data))
            }
            other => Err(format!("No clipboard action {other:?}: get, set or clear.")),
        }
    }
}

fn collect<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
    out.push(node);
    for child in node.get("children").and_then(Value::as_array).into_iter().flatten() {
        collect(child, out);
    }
}

/// A ref as the send gate reads an element.
fn element_json(info: &RefInfo) -> Value {
    let mut element = json!({ "role": gate_role(&info.role, info.secure) });
    if let Some(name) = info.name.as_deref().filter(|n| !n.trim().is_empty()) {
        element["title"] = json!(name);
    }
    if let Some(desc) = info.description.as_deref().filter(|n| !n.trim().is_empty()) {
        element["description"] = json!(desc);
    }
    if let Some(value) = info.value.as_deref().filter(|v| !v.is_empty() && !info.secure) {
        element["value"] = json!(value);
    }
    element
}

#[cfg(test)]
mod tests;
