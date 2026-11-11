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
}
