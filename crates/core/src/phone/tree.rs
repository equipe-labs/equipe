//! What is on a phone's screen, as its accessibility tree says: each element's
//! kind, label, id, value and frame, in points (iOS) or dp (Android), the same
//! units as the screenshot members are shown. Read from AXe's `describe-ui`
//! JSON on iOS and `uiautomator dump`'s XML on Android, written back compact,
//! and searched for what a member names ("Pay", `id:checkout`).

use serde_json::Value;

/// A rectangle on the screen, in points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Frame {
    pub fn center(self) -> (f64, f64) {
        (self.x + self.w / 2., self.y + self.h / 2.)
    }

    fn visible(self, screen: Frame) -> bool {
        self.w >= 1. && self.h >= 1. && self.x < screen.x + screen.w && self.y < screen.y + screen.h && self.x + self.w > screen.x && self.y + self.h > screen.y
    }
}

/// One element on the screen.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    /// `Button`, `StaticText`, `TextField`…; on Android the widget's short class name.
    pub kind: String,
    pub label: Option<String>,
    /// iOS's `accessibilityIdentifier`, Android's `resource-id` (after its `:id/`).
    pub id: Option<String>,
    pub value: Option<String>,
    pub frame: Frame,
    pub enabled: bool,
    /// How deep in the tree, for indenting.
    pub depth: usize,
}

impl Element {
    /// Whether it says anything a person could read or a member could aim at.
    fn named(&self) -> bool {
        self.label.is_some() || self.id.is_some() || self.value.is_some()
    }
}

/// A screen's elements, in tree order, and the screen's own frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Screen {
    pub size: Frame,
    pub elements: Vec<Element>,
}

fn text(value: Option<&Value>) -> Option<String> {
    let s = match value? {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    (!s.is_empty()).then_some(s)
}

/// AXe's `describe-ui`: an array of roots, each `{type, AXLabel, AXUniqueId, AXValue, frame {x,y,width,height}, enabled, children}`.
pub fn from_axe(json: &str) -> Result<Screen, String> {
    let roots: Value = serde_json::from_str(json.trim()).map_err(|e| format!("the screen's description was unreadable ({e})"))?;
    let roots = match roots {
        Value::Array(roots) => roots,
        root @ Value::Object(_) => vec![root],
        _ => return Err("the screen's description was empty".into()),
    };
    let mut screen = Screen::default();
    fn walk(node: &Value, depth: usize, out: &mut Vec<Element>) {
        let f = &node["frame"];
        let frame = Frame {
            x: f["x"].as_f64().unwrap_or(0.),
            y: f["y"].as_f64().unwrap_or(0.),
            w: f["width"].as_f64().unwrap_or(0.),
            h: f["height"].as_f64().unwrap_or(0.),
        };
        out.push(Element {
            kind: text(node.get("type")).unwrap_or_else(|| "Element".into()),
            label: text(node.get("AXLabel")),
            id: text(node.get("AXUniqueId")).or_else(|| text(node.get("AXIdentifier"))),
            value: text(node.get("AXValue")),
            frame,
            enabled: node["enabled"].as_bool().unwrap_or(true),
            depth,
        });
        for child in node["children"].as_array().into_iter().flatten() {
            walk(child, depth + 1, out);
        }
    }
    for root in &roots {
        walk(root, 0, &mut screen.elements);
    }
    screen.size = screen.elements.first().map(|e| e.frame).filter(|f| f.w > 0.).unwrap_or(Frame { x: 0., y: 0., w: 402., h: 874. });
    Ok(screen)
}

/// The value of `name="…"` in one XML start tag, unescaped.
fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let at = tag.find(&key)? + key.len();
    let end = tag[at..].find('"')? + at;
    let raw = &tag[at..end];
    let s = raw.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&#10;", "\n").replace("&amp;", "&");
    let s = s.trim().to_owned();
    (!s.is_empty()).then_some(s)
}

/// `[0,210][1080,420]` → its frame in pixels.
fn bounds(b: &str) -> Option<Frame> {
    let nums: Vec<f64> = b.split(|c: char| !c.is_ascii_digit() && c != '-').filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
    let [x1, y1, x2, y2] = nums[..] else { return None };
    Some(Frame { x: x1, y: y1, w: x2 - x1, h: y2 - y1 })
}

/// `uiautomator dump`'s XML: `<node text content-desc resource-id class bounds enabled …>`,
/// nested. `scale` is the screen's pixels per dp: frames come back in dp.
pub fn from_uiautomator(xml: &str, scale: f64) -> Result<Screen, String> {
    let scale = if scale > 0. { scale } else { 1. };
    if !xml.contains("<hierarchy") {
        return Err("the screen's description was unreadable".into());
    }
    let mut screen = Screen::default();
    let mut depth = 0usize;
    let mut rest = xml;
    while let Some(at) = rest.find('<') {
        rest = &rest[at..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..=end];
        rest = &rest[end + 1..];
        if tag.starts_with("</node") {
            depth = depth.saturating_sub(1);
            continue;
        }
        if !tag.starts_with("<node") {
            continue;
        }
        let px = attr(tag, "bounds").and_then(|b| bounds(&b)).unwrap_or_default();
        let frame = Frame { x: px.x / scale, y: px.y / scale, w: px.w / scale, h: px.h / scale };
        let class = attr(tag, "class").unwrap_or_default();
        let kind = class.rsplit('.').next().unwrap_or("View").to_owned();
        let label = attr(tag, "content-desc").or_else(|| attr(tag, "text").filter(|_| !kind.contains("EditText")));
        let value = attr(tag, "text").filter(|_| kind.contains("EditText"));
        let id = attr(tag, "resource-id").map(|r| r.rsplit_once(":id/").map_or(r.clone(), |(_, id)| id.to_owned()));
        screen.elements.push(Element { kind, label, id, value, frame, enabled: attr(tag, "enabled").is_none_or(|e| e == "true"), depth });
        if !tag.ends_with("/>") {
            depth += 1;
        }
    }
    screen.size = screen
        .elements
        .iter()
        .map(|e| e.frame)
        .fold(Frame::default(), |a, f| Frame { x: 0., y: 0., w: a.w.max(f.x + f.w), h: a.h.max(f.y + f.h) });
    Ok(screen)
}

/// How many lines a compact tree shows at most.
pub const COMPACT_LINES: usize = 120;

fn quoted(s: &str) -> String {
    let one_line: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let cut: String = one_line.chars().take(80).collect();
    if cut.len() < one_line.len() { format!("\"{cut}…\"") } else { format!("\"{cut}\"") }
}

/// The screen as a member reads it: one line per element that says something
/// and is on screen — `Button "Pay" id=pay @ 20,700 362×50` — flat,
/// in the tree's reading order (unnamed containers between make nesting
/// misleading), the same thing said twice once, at most [`COMPACT_LINES`].
pub fn compact(screen: &Screen) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut seen: Vec<(String, Option<String>, Option<String>, i64, i64)> = Vec::new();
    let mut left_out = 0;
    let shown: Vec<&Element> = screen.elements.iter().filter(|e| e.named() && e.frame.visible(screen.size) && e.kind != "Application").collect();
    for e in shown {
        let key = (e.kind.clone(), e.label.clone(), e.value.clone(), e.frame.x.round() as i64, e.frame.y.round() as i64);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        if lines.len() == COMPACT_LINES {
            left_out += 1;
            continue;
        }
        let mut line = e.kind.clone();
        if let Some(label) = &e.label {
            line.push(' ');
            line.push_str(&quoted(label));
        }
        if let Some(id) = &e.id {
            line.push_str(&format!(" id={id}"));
        }
        if let Some(value) = e.value.as_deref().filter(|v| Some(*v) != e.label.as_deref()) {
            line.push_str(&format!(" value={}", quoted(value)));
        }
        if !e.enabled {
            line.push_str(" (disabled)");
        }
        let f = e.frame;
        line.push_str(&format!(" @ {},{} {}×{}", f.x.round() as i64, f.y.round() as i64, f.w.round() as i64, f.h.round() as i64));
        lines.push(line);
    }
    if lines.is_empty() {
        return "Nothing on the screen says anything to accessibility (an empty screen, a picture, or the app's views have no labels).".into();
    }
    if left_out > 0 {
        lines.push(format!("… and {left_out} more"));
    }
    lines.join("\n")
}

/// What a member aims at: an element by its label (or `id:<id>`), or a point.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Label(String),
    Id(String),
    Point(f64, f64),
}

impl Target {
    /// From a tool's arguments: `target` as words (`"Pay"`, `"id:pay"`) or `{x, y}`; or `x`/`y` beside it.
    pub fn of(args: &Value) -> Option<Self> {
        let point = |v: &Value| Some(Target::Point(v.get("x")?.as_f64()?, v.get("y")?.as_f64()?));
        match args.get("target") {
            Some(Value::String(s)) => {
                let s = s.trim();
                match s.strip_prefix("id:") {
                    Some(id) => Some(Target::Id(id.trim().to_owned())),
                    None if !s.is_empty() => Some(Target::Label(s.to_owned())),
                    None => None,
                }
            }
            Some(v @ Value::Object(_)) => point(v),
            _ => point(args),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Target::Label(l) => quoted(l),
            Target::Id(id) => format!("id={id}"),
            Target::Point(x, y) => format!("({}, {})", x.round(), y.round()),
        }
    }
}

/// Where `target` is on `screen`: the point to touch, or why it can't be said.
/// A label matches exactly first (case aside), then as a part of a label or
/// value; among several, one that is a control and enabled, then the
/// innermost. Off-screen elements don't count.
pub fn find(screen: &Screen, target: &Target) -> Result<(f64, f64), String> {
    if let Target::Point(x, y) = target {
        let s = screen.size;
        if *x < s.x || *y < s.y || *x > s.x + s.w || *y > s.y + s.h {
            return Err(format!("({x}, {y}) is off the screen, which is {}×{} points.", s.w.round(), s.h.round()));
        }
        return Ok((*x, *y));
    }
    let on_screen: Vec<&Element> = screen.elements.iter().filter(|e| e.frame.visible(screen.size) && e.kind != "Application").collect();
    let matches: Vec<&Element> = match target {
        Target::Id(id) => on_screen.iter().copied().filter(|e| e.id.as_deref() == Some(id.as_str())).collect(),
        Target::Label(label) => {
            let want = label.to_lowercase();
            let exact: Vec<&Element> = on_screen.iter().copied().filter(|e| e.label.as_ref().is_some_and(|l| l.to_lowercase() == want)).collect();
            if exact.is_empty() {
                on_screen
                    .iter()
                    .copied()
                    .filter(|e| [&e.label, &e.value].into_iter().flatten().any(|t| t.to_lowercase().contains(&want)))
                    .collect()
            } else {
                exact
            }
        }
        Target::Point(..) => unreachable!(),
    };
    let control = |e: &&Element| CONTROLS.iter().any(|c| e.kind.contains(c));
    let best: Vec<&Element> = {
        let controls: Vec<&Element> = matches.iter().copied().filter(|e| control(e) && e.enabled).collect();
        if controls.is_empty() { matches.clone() } else { controls }
    };
    match best.as_slice() {
        [] => Err(format!("Nothing on the screen is {} right now. Look again: it may not have appeared yet (wait for it), be further down (scroll), or be named differently.", target.describe())),
        [one] => Ok(one.frame.center()),
        many => {
            // Nested matches (a cell and its text): the innermost is the one.
            let deepest = many.iter().map(|e| e.depth).max().unwrap_or(0);
            let inner: Vec<&&Element> = many.iter().filter(|e| e.depth == deepest).collect();
            if inner.len() == 1 {
                return Ok(inner[0].frame.center());
            }
            let places: Vec<String> = many.iter().take(5).map(|e| format!("{} @ {},{}", e.kind, e.frame.x.round(), e.frame.y.round())).collect();
            Err(format!(
                "{} matches {} things on the screen ({}). Aim with its id (`id:<id>`) or its point instead.",
                target.describe(),
                many.len(),
                places.join("; ")
            ))
        }
    }
}

/// Kinds a person taps, on either platform.
const CONTROLS: [&str; 10] = ["Button", "Cell", "Link", "TextField", "Switch", "Toggle", "Tab", "CheckBox", "EditText", "Slider"];

/// Whether `target` is on the screen now.
pub fn has(screen: &Screen, target: &Target) -> bool {
    !matches!(target, Target::Point(..)) && find(screen, target).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AXE: &str = r#"[{"type":"Application","AXLabel":" ","frame":{"x":0,"y":0,"width":402,"height":874},"children":[
        {"type":"Group","AXLabel":null,"frame":{"x":0,"y":0,"width":402,"height":874},"children":[
            {"type":"StaticText","AXLabel":"Cart","frame":{"x":20,"y":100,"width":80,"height":30},"children":[]},
            {"type":"Button","AXLabel":"Pay","AXUniqueId":"pay","frame":{"x":20,"y":700,"width":362,"height":50},"enabled":true,"children":[
                {"type":"StaticText","AXLabel":"Pay","frame":{"x":180,"y":712,"width":40,"height":24},"children":[]}
            ]},
            {"type":"TextField","AXLabel":"Coupon","AXValue":"SAVE10","frame":{"x":20,"y":600,"width":362,"height":40},"children":[]},
            {"type":"Button","AXLabel":"Hidden","frame":{"x":0,"y":0,"width":0,"height":0},"children":[]},
            {"type":"Button","AXLabel":"Delete","frame":{"x":20,"y":200,"width":100,"height":40},"children":[]},
            {"type":"Button","AXLabel":"Delete","frame":{"x":20,"y":260,"width":100,"height":40},"children":[]}
        ]}
    ]}]"#;

    #[test]
    fn axe_s_tree_reads_as_elements_in_points() {
        let screen = from_axe(AXE).unwrap();
        assert_eq!(screen.size, Frame { x: 0., y: 0., w: 402., h: 874. });
        let pay = screen.elements.iter().find(|e| e.id.as_deref() == Some("pay")).unwrap();
        assert_eq!(pay.kind, "Button");
        assert_eq!(pay.frame.center(), (201., 725.));
        let coupon = screen.elements.iter().find(|e| e.kind == "TextField").unwrap();
        assert_eq!(coupon.value.as_deref(), Some("SAVE10"));
    }

    #[test]
    fn the_compact_tree_shows_what_is_on_screen_once_with_where() {
        let text = compact(&from_axe(AXE).unwrap());
        assert!(text.contains("Button \"Pay\" id=pay @ 20,700 362×50"), "{text}");
        assert!(text.contains("TextField \"Coupon\" value=\"SAVE10\""), "{text}");
        assert!(!text.contains("Hidden"), "off screen: left out\n{text}");
        assert!(!text.contains("Application"), "{text}");
        assert!(text.contains("\nStaticText \"Pay\""), "its button's text too, on a line of its own: {text}");
    }

    #[test]
    fn a_label_finds_the_control_and_an_ambiguous_one_says_how_to_aim() {
        let screen = from_axe(AXE).unwrap();
        assert_eq!(find(&screen, &Target::Label("pay".into())), Ok((201., 725.)), "the button, not its text; case aside");
        assert_eq!(find(&screen, &Target::Id("pay".into())), Ok((201., 725.)));
        assert_eq!(find(&screen, &Target::Label("Coup".into())), Ok((201., 620.)), "a part of a label");
        let two = find(&screen, &Target::Label("Delete".into())).unwrap_err();
        assert!(two.contains("matches 2 things") && two.contains("id:"), "{two}");
        let none = find(&screen, &Target::Label("Hidden".into())).unwrap_err();
        assert!(none.contains("Nothing on the screen"), "off screen doesn't count: {none}");
        assert!(find(&screen, &Target::Point(500., 10.)).is_err(), "off the screen");
        assert_eq!(find(&screen, &Target::Point(10., 10.)), Ok((10., 10.)));
    }

    #[test]
    fn a_target_is_words_an_id_or_a_point() {
        use serde_json::json;
        assert_eq!(Target::of(&json!({"target": "Pay"})), Some(Target::Label("Pay".into())));
        assert_eq!(Target::of(&json!({"target": "id:pay"})), Some(Target::Id("pay".into())));
        assert_eq!(Target::of(&json!({"target": {"x": 3, "y": 4}})), Some(Target::Point(3., 4.)));
        assert_eq!(Target::of(&json!({"x": 3, "y": 4.5})), Some(Target::Point(3., 4.5)));
        assert_eq!(Target::of(&json!({})), None);
    }

    #[test]
    fn uiautomator_s_dump_reads_in_dp() {
        let xml = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?><hierarchy rotation="0"><node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="com.x" content-desc="" enabled="true" bounds="[0,0][1080,2400]"><node index="0" text="Pay &amp; go" resource-id="com.x:id/pay" class="android.widget.Button" package="com.x" content-desc="" enabled="true" bounds="[42,1800][1038,1932]" /><node index="1" text="SAVE10" resource-id="com.x:id/coupon" class="android.widget.EditText" content-desc="Coupon" enabled="false" bounds="[42,1500][1038,1600]" /></node></hierarchy>"#;
        let screen = from_uiautomator(xml, 2.625).unwrap();
        assert_eq!(screen.size.w.round(), 411.);
        let pay = &screen.elements[1];
        assert_eq!((pay.kind.as_str(), pay.label.as_deref(), pay.id.as_deref()), ("Button", Some("Pay & go"), Some("pay")));
        assert_eq!(pay.depth, 1);
        let coupon = &screen.elements[2];
        assert_eq!((coupon.label.as_deref(), coupon.value.as_deref(), coupon.enabled), (Some("Coupon"), Some("SAVE10"), false));
        let (x, y) = find(&screen, &Target::Label("Pay & go".into())).unwrap();
        assert_eq!(((x * 2.625).round(), (y * 2.625).round()), (540., 1866.));
        assert!(from_uiautomator("ERROR: could not get idle state.", 1.).is_err());
    }
}
