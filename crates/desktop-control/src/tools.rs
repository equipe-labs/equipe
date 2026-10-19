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

/// Every desktop tool.
pub fn tools() -> Vec<Value> {
    vec![
        tool(
            "screenshot",
            "See the screen, or one window. The image is sized for you; the coordinates you give the other tools are its pixels. Nothing is saved.",
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
            "Type text, Korean and emoji included, where the keyboard focus is. Refused if focus isn't in the app (or element) you mean; stops if focus moves away while typing.",
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
            "Press keys or shortcuts, one after another: \"cmd+s\", \"return\", \"cmd+a delete\". Refused if focus isn't in the app you mean.",
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
        ] {
            assert!(names.iter().any(|n| n == name), "{name}");
        }
    }
}
