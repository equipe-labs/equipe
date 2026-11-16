//! The desktop tools as a member's engine sees them: `mcp__desktop__…`.

use serde_json::{Value, json};

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
    })
}

const COORDS: &str = "in pixels of your last screenshot (the whole screen if you haven't taken one)";

fn app() -> Value {
    json!({ "type": "string", "description": "The app you mean, by name (\"TextEdit\") or bundle id. Default: the one you last opened, focused or looked into." })
}

fn element() -> Value {
    json!({ "type": "string", "description": "An element id from your last `elements` call, e.g. \"e12\"." })
}

/// In a messaging app an input that may send asks the person first, with this.
fn why() -> Value {
    json!({ "type": "string", "description": "In a messaging app (Teams, Slack, Mail…), what you are sending and to whom, in a line: an input that may send waits for the person's yes, and they read this." })
}

fn number(description: &str) -> Value {
    json!({ "type": "number", "description": description })
}

fn reference() -> Value {
    json!({ "type": "string", "description": "A ref from your snapshot or find, as printed (\"e12\" means your last snapshot; \"s8f3k2p9:e12\" names one)." })
}

/// The accessibility tools (agent-desktop): read an app as a tree, act on a
/// ref without the pointer, keyboard or focus. First, because members should
/// reach for them first: cheaper than screenshots, and they don't wait for the screen.
fn tree_tools() -> Vec<Value> {
    vec![
        tool(
            "snapshot",
            "Read an app as a compact tree: one line per element, a ref (e12) on each you can act on. Start here — far cheaper than a screenshot, and it doesn't take the screen. Big app: `skeleton` first, then drill into one branch with `root` (a ref; `+26` marks a cut branch). Interactive elements only unless `all`.",
            json!({
                "app": { "type": "string", "description": "The app, by name (\"Slack\") or bundle id. Default: the focused app." },
                "window": { "type": "string", "description": "A window id (w-123) from `windows`, when the app has several." },
                "root": reference(),
                "skeleton": { "type": "boolean", "description": "A shallow map with cut branches counted: for big apps." },
                "surface": { "type": "string", "description": "An overlay instead of the window: menu, menubar, sheet, popover, alert, focused, dock, menu-bar-extras." },
                "all": { "type": "boolean", "description": "Keep static text and containers too (to tell which row a button is in)." },
                "depth": { "type": "integer", "description": "Deepest level read (default 10)." },
                "bounds": { "type": "boolean", "description": "Add each element's screen rectangle (only when you need coordinates)." },
                "timeout_ms": { "type": "integer", "description": "How long the app may take to answer (default 3000; slow Electron apps want more)." },
            }),
            &[],
        ),
        tool(
            "find",
            "Find elements by role, name or text without reading the whole tree: the cheapest way to a ref you can name.",
            json!({
                "app": { "type": "string", "description": "The app to search." },
                "root": reference(),
                "window": { "type": "string", "description": "A window id from `windows`." },
                "surface": { "type": "string", "description": "An overlay to search (menubar, menu, sheet…)." },
                "role": { "type": "string", "description": "button, textfield, checkbox, menuitem, treeitem…" },
                "name": { "type": "string", "description": "The accessible name (fuzzy unless `exact`)." },
                "text": { "type": "string", "description": "Text anywhere in name, value or description." },
                "value": { "type": "string", "description": "The current value." },
                "exact": { "type": "boolean" },
                "limit": { "type": "integer", "description": "At most this many (default 10)." },
            }),
            &[],
        ),
        tool(
            "act",
            "Act on a ref, headless: no pointer, no keystrokes, no focus taken, so it doesn't wait for the screen or disturb the person. Then read again (snapshot with the same `root`) to check. STALE_REF: snapshot again. Retry only when the answer says it is safe. Text you type never shows in any process list or log.",
            json!({
                "ref": reference(),
                "action": { "type": "string", "enum": crate::engine::ACTIONS, "description": "Default click. type adds text at the caret, set_value replaces it; select picks `value` from a list or menu; check/uncheck over toggle. right_click and double_click wait for the screen." },
                "text": { "type": "string", "description": "For type and set_value." },
                "value": { "type": "string", "description": "For select: the option's text." },
                "direction": { "type": "string", "enum": ["up", "down", "left", "right"], "description": "For scroll." },
                "amount": { "type": "integer", "description": "For scroll (default 3)." },
                "headed": { "type": "boolean", "description": "Physical input instead (real pointer and keys; double_click always is). Only when headless fails with POLICY_DENIED or the app ignores it: it waits for the screen and the person." },
                "why": why(),
            }),
            &["ref"],
        ),
        tool(
            "apps",
            "Running apps (list), open one (launch — in the background unless `activate`), or quit one (close).",
            json!({
                "action": { "type": "string", "enum": ["list", "launch", "close"], "description": "Default list." },
                "app": { "type": "string", "description": "Name or bundle id." },
                "activate": { "type": "boolean", "description": "For launch: bring it forward and wait for its window (takes the screen)." },
            }),
            &[],
        ),
        tool(
            "windows",
            "Windows (list, with ids for `snapshot`), or focus, minimize, maximize, restore, resize, move one. Anything but list waits for the screen.",
            json!({
                "action": { "type": "string", "enum": ["list", "focus", "minimize", "maximize", "restore", "resize", "move"], "description": "Default list." },
                "app": { "type": "string" },
                "window": { "type": "string", "description": "A window id (w-123)." },
                "width": { "type": "integer" },
                "height": { "type": "integer" },
                "x": { "type": "integer" },
                "y": { "type": "integer" },
            }),
            &[],
        ),
        tool(
            "wait",
            "Wait for something instead of sleeping: an element (`ref` with `predicate`), a window by title, text in an app, a menu, an event (window-opened, window-closed, app-launched, focus-changed, surface-appeared…), or `ms`.",
            json!({
                "ms": { "type": "integer" },
                "ref": reference(),
                "predicate": { "type": "string", "enum": ["exists", "enabled", "visible", "actionable", "value"] },
                "value": { "type": "string", "description": "For predicate value." },
                "window": { "type": "string" },
                "text": { "type": "string" },
                "menu": { "type": "boolean" },
                "event": { "type": "string" },
                "app": { "type": "string" },
                "timeout_ms": { "type": "integer", "description": "Default 10000, at most 60000." },
            }),
            &[],
        ),
        tool(
            "notifications",
            "Notification Center: list (when it is open), or act on one (press a button by `name`) or dismiss it, by its `index` from your last listing. Acting waits for the screen.",
            json!({
                "action": { "type": "string", "enum": ["list", "act", "dismiss"], "description": "Default list." },
                "app": { "type": "string", "description": "For list: only this app's." },
                "text": { "type": "string", "description": "For list: only ones with this text." },
                "index": { "type": "integer" },
                "name": { "type": "string", "description": "For act: the button, as listed in [..]." },
                "why": why(),
            }),
            &[],
        ),
        tool(
            "clipboard",
            "Read the clipboard's text (get), or set or clear it (waits until the person isn't using the Mac). Set text never shows in any process list or log.",
            json!({
                "action": { "type": "string", "enum": ["get", "set", "clear"], "description": "Default get." },
                "text": { "type": "string" },
            }),
            &[],
        ),
    ]
}

/// Every desktop tool: the accessibility tools, then the screenshot and
/// physical-input tools for what the tree can't reach.
pub fn tools() -> Vec<Value> {
    let mut all = tree_tools();
    all.extend(pixel_tools());
    all
}

/// The desktop tools, with the real-Chrome ones when the person turned that
/// on for the squad ([`crate::chrome::enabled`]).
pub fn tools_for(chrome: bool) -> Vec<Value> {
    let mut all = tools();
    if chrome {
        all.extend(chrome_tools());
    }
    all
}

fn chrome_window() -> Value {
    json!({ "type": "string", "description": "Your window's id from chrome_open (w-4521). Default: the one you opened last." })
}

/// The person's own Google Chrome (docs/REAL-CHROME.md): a window of their
/// real profile, every sign-in already there, read and acted on through the
/// accessibility tree, headless.
pub fn chrome_tools() -> Vec<Value> {
    vec![
        tool(
            "chrome_profiles",
            "The person's Chrome profiles — each one's folder, name and Google account (masked) — and which profile this squad uses for which site. Pick one for chrome_open by the account a site needs; not plain which? ask the person (ask_decision) once, then pass `remember`.",
            json!({}),
            &[],
        ),
        tool(
            "chrome_open",
            "Open a web page in a new window of the person's own Google Chrome, in one of their profiles: their real browser, with their sign-ins already there — nothing to copy, no 'new device'. The window is yours alone until chrome_close; nobody else's windows are yours to touch. Opening shows Chrome once, so it waits until the person isn't using the Mac. A sign-in or a human check there is the person's: call show_my_browser and they do it in this same window.",
            json!({
                "url": { "type": "string", "description": "The page (https://…)." },
                "profile": { "type": "string", "description": "A profile from chrome_profiles: its folder (\"Profile 1\"), name or masked account. Default: the one this squad uses for the site, or the only one." },
                "remember": { "type": "boolean", "description": "Keep this profile as the squad's for the site (after the person chose it)." },
            }),
            &["url"],
        ),
        tool(
            "chrome_snapshot",
            "Read your Chrome window's page as a compact tree: one line per element, a ref (e12) on each you can act on. Big page: `skeleton` first, then `root` (a ref) to drill into one part. Interactive elements only unless `all`. A human check on the page is called out: never act on it.",
            json!({
                "window": chrome_window(),
                "root": reference(),
                "skeleton": { "type": "boolean" },
                "all": { "type": "boolean", "description": "Keep text and containers too." },
                "depth": { "type": "integer" },
                "timeout_ms": { "type": "integer", "description": "How long the page may take to answer (default 8000)." },
            }),
            &[],
        ),
        tool(
            "chrome_act",
            "Act on a ref from your chrome_snapshot, headless (no pointer or keyboard taken, the person keeps working): click a link or button, type or set_value into a field, select an option, check, scroll. Then chrome_snapshot again to check; a page that loads: `wait` (app \"Google Chrome\", `text`) first. Payment, password and security pages take no input; sending in the person's name (Gmail, Slack…) waits for their yes. Text you type never shows in any log.",
            json!({
                "ref": reference(),
                "action": { "type": "string", "enum": crate::chrome::ACTIONS, "description": "Default click." },
                "text": { "type": "string", "description": "For type and set_value." },
                "value": { "type": "string", "description": "For select: the option's text." },
                "direction": { "type": "string", "enum": ["up", "down", "left", "right"], "description": "For scroll." },
                "amount": { "type": "integer", "description": "For scroll (default 3)." },
                "why": why(),
            }),
            &["ref"],
        ),
        tool(
            "chrome_read",
            "The text of your Chrome window's page in reading order, without refs: to read an article, a table, a result. `find` keeps only the lines that contain it.",
            json!({
                "window": chrome_window(),
                "find": { "type": "string" },
                "max_chars": { "type": "integer", "description": "Default 12000." },
            }),
            &[],
        ),
        tool(
            "chrome_capture",
            "A picture of your Chrome window only — never the rest of the screen — to check how a page looks. Needs Screen Recording; chrome_snapshot and chrome_read are cheaper.",
            json!({ "window": chrome_window() }),
            &[],
        ),
        tool(
            "chrome_close",
            "Close your Chrome window when you're done with it (yours only).",
            json!({ "window": chrome_window() }),
            &[],
        ),
    ]
}

/// Screenshots, coordinates, the real pointer and keyboard: one member at a
/// time, after the person has left the Mac alone.
fn pixel_tools() -> Vec<Value> {
    vec![
        tool(
            "screenshot",
            "See the screen, or one window, as an image — only when you need pixels (a canvas, an app with a thin tree): `snapshot` is cheaper. The image is sized for you; the coordinates you give click, drag and scroll are its pixels. Nothing is saved.",
            json!({
                "window": { "type": "string", "description": "A window id from list_windows (\"w123\"): capture just that window." },
                "app": { "type": "string", "description": "Capture this app's front window." },
            }),
            &[],
        ),
        tool(
            "list_windows",
            "The windows on screen, front to back: id, app, title, where.",
            json!({ "app": { "type": "string", "description": "Only this app's windows." } }),
            &[],
        ),
        tool(
            "open_app",
            "Open an app (or bring it up if it runs) and bring it to the front.",
            json!({ "app": { "type": "string", "description": "Name (\"TextEdit\") or bundle id (\"com.apple.TextEdit\")." } }),
            &["app"],
        ),
        tool(
            "focus_window",
            "Bring an app, or one of its windows, to the front so clicks and keys go there.",
            json!({ "window": { "type": "string", "description": "A window id from list_windows." }, "app": app() }),
            &[],
        ),
        tool(
            "elements",
            "The app's UI as Accessibility sees it: role, name, value, where, and an id to click or type into. Steadier than coordinates.",
            json!({
                "app": app(),
                "window": { "type": "string", "description": "A window id: only that window (default: the app's front window)." },
                "max": { "type": "integer", "description": "At most this many elements (default 150)." },
            }),
            &[],
        ),
        tool(
            "click",
            &format!("Click an element, or a point {COORDS}. Refused if the point isn't in the app you mean."),
            json!({
                "element": element(),
                "x": number("X, with y, when there is no element."),
                "y": number("Y."),
                "app": app(),
                "button": { "type": "string", "enum": ["left", "right"], "description": "Default left." },
                "why": why(),
            }),
            &[],
        ),
        tool(
            "double_click",
            &format!("Double-click an element or a point {COORDS}."),
            json!({ "element": element(), "x": number("X."), "y": number("Y."), "app": app(), "why": why() }),
            &[],
        ),
        tool(
            "drag",
            &format!("Press at one point and release at another, {COORDS}."),
            json!({
                "from_x": number("Start x."),
                "from_y": number("Start y."),
                "to_x": number("End x."),
                "to_y": number("End y."),
                "app": app(),
                "why": why(),
            }),
            &["from_x", "from_y", "to_x", "to_y"],
        ),
        tool(
            "scroll",
            &format!("Scroll over an element or a point {COORDS}."),
            json!({
                "element": element(),
                "x": number("X."),
                "y": number("Y."),
                "down": number("Lines down (negative: up). Default 5."),
                "right": number("Lines right (negative: left)."),
                "app": app(),
            }),
            &[],
        ),
        tool(
            "type_text",
            "Type text, any script and emoji included, where the keyboard focus is. Refused if focus isn't in the app (or element) you mean; stops if focus moves away while typing.",
            json!({
                "text": { "type": "string", "description": "What to type. A newline presses return." },
                "element": json!({ "type": "string", "description": "Focus this text field first (id from `elements`)." }),
                "app": app(),
                "why": why(),
            }),
            &["text"],
        ),
        tool(
            "key",
            "Press keys or shortcuts on the real keyboard, one after another: \"cmd+s\" (Ctrl+S on Windows), \"return\", \"cmd+a delete\". Refused if focus isn't in the app you mean. Waits for the screen.",
            json!({ "keys": { "type": "string", "description": "Shortcuts separated by spaces." }, "app": app(), "why": why() }),
            &["keys"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_tool_the_ticket_names_is_offered() {
        let names: Vec<String> = super::tools().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect();
        for name in [
            "screenshot",
            "click",
            "double_click",
            "drag",
            "scroll",
            "type_text",
            "key",
            "open_app",
            "list_windows",
            "focus_window",
            "elements",
            "snapshot",
            "find",
            "act",
            "apps",
            "windows",
            "wait",
            "notifications",
            "clipboard",
        ] {
            assert!(names.iter().any(|n| n == name), "{name}");
        }
        for name in crate::engine::TOOLS {
            assert!(crate::engine::handles(name) && names.iter().any(|n| n == name), "{name}");
        }
        let unique: std::collections::BTreeSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "no two tools share a name");
    }

    #[test]
    fn the_real_chrome_tools_are_offered_only_when_turned_on() {
        let names = |tools: Vec<serde_json::Value>| -> Vec<String> { tools.iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect() };
        assert_eq!(names(super::tools_for(false)), names(super::tools()));
        let on = names(super::tools_for(true));
        for name in crate::chrome::TOOLS {
            assert!(crate::chrome::handles(name) && on.iter().any(|n| n == name), "{name}");
            assert!(!names(super::tools()).iter().any(|n| n == name), "{name} only when on");
        }
        let unique: std::collections::BTreeSet<&String> = on.iter().collect();
        assert_eq!(unique.len(), on.len(), "no two tools share a name");
    }
}
