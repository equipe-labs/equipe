//! Sending in the person's name from a desktop app waits for their yes.
//!
//! A click at (x, y) or a Return doesn't say whether it sends anything, so the
//! gate goes by the app in front: in a messaging app (Teams, Slack, Mail…) an
//! input that may send — Return, a click on a button that isn't plainly
//! harmless, a drag — stops before it happens and goes to the person, with the
//! draft and a screenshot. Typing text without a line break only writes a draft
//! and goes ahead. Looking — screenshots, scrolling, the UI tree, switching
//! windows — is always free. Elsewhere (TextEdit…) nothing asks. Some screens
//! take no input at all, in any mode: payment, deleting an account, passwords
//! and security settings.
//!
//! The rules are code, not the model's judgement, and a send is never made a
//! rule: the person says yes each time.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The tool name a send shows under, among approvals. Never "always allowed".
pub const SEND: &str = "Send";

/// The squad's messaging apps until the person changes the list: by the name
/// macOS shows. An entry may also be a bundle id.
pub const DEFAULT_APPS: [&str; 14] = [
    "Microsoft Teams",
    "Slack",
    "Mail",
    "Microsoft Outlook",
    "Messages",
    "KakaoTalk",
    "zoom.us",
    "Discord",
    "Telegram",
    "WhatsApp",
    "LINE",
    "Signal",
    "Spark",
    "Webex",
];

/// Bundle ids of the defaults, so an app still counts when its shown name
/// changes (Teams has been "Microsoft Teams (work or school)").
const BUNDLES: [(&str, &str); 14] = [
    ("com.microsoft.teams2", "Microsoft Teams"),
    ("com.tinyspeck.slackmacgap", "Slack"),
    ("com.apple.mail", "Mail"),
    ("com.microsoft.Outlook", "Microsoft Outlook"),
    ("com.apple.MobileSMS", "Messages"),
    ("com.kakao.KakaoTalkMac", "KakaoTalk"),
    ("us.zoom.xos", "zoom.us"),
    ("com.hnc.Discord", "Discord"),
    ("ru.keepcoder.Telegram", "Telegram"),
    ("net.whatsapp.WhatsApp", "WhatsApp"),
    ("jp.naver.line.mac", "LINE"),
    ("org.whispersystems.signal-desktop", "Signal"),
    ("com.readdle.SparkDesktop", "Spark"),
    ("Cisco-Systems.Spark", "Webex"),
];

/// Apps that take no input from a member at all: where passwords live.
const BLOCKED_APPS: [&str; 6] = [
    "com.apple.keychainaccess",
    "com.apple.Passwords",
    "com.1password.1password",
    "com.bitwarden.desktop",
    "Keychain Access",
    "Passwords",
];

/// Screens no member types or clicks on, in any mode: paying, deleting an
/// account, passwords, security settings. Matched in the window's title (outside
/// messaging apps, where it is a conversation's name) and in the name of what
/// is clicked or typed into.
const BLOCKED_WORDS: [&str; 30] = [
    "password",
    "passcode",
    "passkey",
    "비밀번호",
    "암호 변경",
    "two-factor",
    "2단계 인증",
    "security settings",
    "privacy & security",
    "개인정보 보호 및 보안",
    "보안 설정",
    "touch id & password",
    "users & groups",
    "filevault",
    "delete account",
    "close account",
    "delete my account",
    "계정 삭제",
    "회원 탈퇴",
    "회원탈퇴",
    "payment",
    "checkout",
    "pay now",
    "buy now",
    "place order",
    "billing",
    "결제",
    "구매하기",
    "주문하기",
    "송금",
];

/// Names of buttons that reach someone: clicking one in a messaging app asks.
/// English ones are whole words ("call", not "recall").
const OUTWARD_WORDS: [&str; 26] = [
    "send", "share", "invite", "accept", "join", "post", "submit", "publish", "call", "upload", "attach", "forward",
    "보내기", "전송", "보냄", "공유", "초대", "수락", "참가", "참여", "게시", "제출", "통화", "업로드", "첨부", "전달",
];

/// AX roles of things a click acts on rather than places a cursor in.
const PRESSABLE: [&str; 6] = ["AXButton", "AXMenuButton", "AXPopUpButton", "AXCheckBox", "AXRadioButton", "AXMenuItem"];

/// An app as the desktop helper sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct App {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
}

/// A UI element, from the Accessibility tree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Element {
    /// `AXButton`, `AXTextArea`…
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// A text field's text: the draft about to go.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl Element {
    /// What the element is called, in every way AX says it.
    fn names(&self) -> impl Iterator<Item = &str> {
        [self.title.as_deref(), self.description.as_deref()].into_iter().flatten()
    }

    fn pressable(&self) -> bool {
        PRESSABLE.contains(&self.role.as_str())
    }
}

/// The screen at the moment of an input, as the helper reads it just before.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scene {
    /// The app in front, which gets the input.
    pub app: App,
    /// Its front window's title: in a messaging app, usually the conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    /// What a click lands on (the AX hit test at the point, or the element by id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Element>,
    /// What has keyboard focus: where typing and keys go.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<Element>,
}

/// What a desktop tool is about to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Screenshots, the UI tree, scrolling, the window list, bringing a window
    /// or an app forward: nothing leaves.
    Look,
    Click,
    Drag,
    Type(String),
    /// A key or a combination: `return`, `cmd+return`, `cmd+shift+d`.
    Key(String),
}

impl Act {
    /// From the desktop tool's name (`mcp__desktop__click` or just `click`) and
    /// arguments. A tool the gate doesn't know is taken for a click: an input
    /// whose effect can't be told.
    pub fn of(tool: &str, input: &Value) -> Act {
        let name = tool.rsplit("__").next().unwrap_or(tool);
        let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        match name {
            "screenshot" | "elements" | "ui_tree" | "scroll" | "list_windows" | "focus_window" | "open_app" | "release" => Act::Look,
            "type" | "type_text" => Act::Type(text("text")),
            "key" => Act::Key(text(if input.get("keys").is_some() { "keys" } else { "key" })),
            "drag" => Act::Drag,
            _ => Act::Click,
        }
    }

    pub fn is_look(&self) -> bool {
        matches!(self, Act::Look)
    }
}

/// The gate's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Free,
    /// It may send: the person says yes first. Why, in a few words.
    Ask(&'static str),
    /// Never, in any mode. Why.
    Blocked(&'static str),
}

/// Whether `app` is one of the squad's messaging apps (`apps`, by name or
/// bundle id, any case).
pub fn messaging(app: &App, apps: &[String]) -> bool {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let known = app.bundle_id.as_deref().and_then(|id| BUNDLES.iter().find(|(b, _)| same(b, id)).map(|(_, name)| *name));
    apps.iter().any(|entry| {
        same(entry, &app.name)
            || app.bundle_id.as_deref().is_some_and(|id| same(entry, id))
            || known.is_some_and(|name| same(entry, name))
    })
}

/// Judge one desktop input against the squad's messaging apps.
pub fn judge(act: &Act, scene: &Scene, apps: &[String]) -> Verdict {
    if act.is_look() {
        return Verdict::Free;
    }
    let in_messaging = messaging(&scene.app, apps);
    if let Some(why) = blocked(act, scene, in_messaging) {
        return Verdict::Blocked(why);
    }
    if !in_messaging {
        return Verdict::Free;
    }
    match act {
        Act::Look => Verdict::Free,
        // A draft goes ahead; a line break may send it.
        Act::Type(text) if text.contains(['\n', '\r']) => Verdict::Ask("a line break can send the message"),
        Act::Type(_) => Verdict::Free,
        Act::Key(keys) => match key_sends(keys) {
            Some(why) => Verdict::Ask(why),
            // Space presses the focused button.
            None if presses(keys) => scene.focused.as_ref().map_or(Verdict::Free, click_on),
            None => Verdict::Free,
        },
        Act::Drag => Verdict::Ask("a drag can drop a file into a conversation"),
        Act::Click => scene.target.as_ref().map_or(Verdict::Ask("equipe can't tell what the click reaches"), click_on),
    }
}

/// A click on `element` in a messaging app.
fn click_on(element: &Element) -> Verdict {
    if element.names().any(outward) {
        return Verdict::Ask("the button reaches other people");
    }
    if element.pressable() && element.names().all(|n| n.trim().is_empty()) {
        return Verdict::Ask("a button without a name may be the send button");
    }
    Verdict::Free
}

/// The key, without modifiers, lower case: `return` from `Cmd+Return`.
fn key_parts(keys: &str) -> Vec<String> {
    keys.split(['+', '-', ' ']).filter(|p| !p.is_empty()).map(str::to_lowercase).collect()
}

/// Why a key combination may send, if it may.
fn key_sends(keys: &str) -> Option<&'static str> {
    let parts = key_parts(keys);
    let last = parts.last().map(String::as_str).unwrap_or_default();
    if matches!(last, "return" | "enter" | "kp_enter" | "keypadenter" | "↩" | "⏎") {
        return Some("Return can send the message");
    }
    // Mail's "Send Message".
    let has = |m: &[&str]| parts.iter().any(|p| m.contains(&p.as_str()));
    if last == "d" && has(&["cmd", "command", "⌘"]) && has(&["shift", "⇧"]) {
        return Some("⌘⇧D sends in Mail");
    }
    None
}

fn presses(keys: &str) -> bool {
    key_parts(keys).last().is_some_and(|k| k == "space")
}

fn outward(name: &str) -> bool {
    let name = name.to_lowercase();
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).collect();
    OUTWARD_WORDS.iter().any(|w| if w.is_ascii() { words.contains(w) } else { name.contains(w) })
}

/// Why no member may make this input, in any mode, if so.
fn blocked(act: &Act, scene: &Scene, in_messaging: bool) -> Option<&'static str> {
    let app = &scene.app;
    if BLOCKED_APPS.iter().any(|b| b.eq_ignore_ascii_case(&app.name) || app.bundle_id.as_deref().is_some_and(|id| b.eq_ignore_ascii_case(id))) {
        return Some("Passwords are the person's: equipe never types or clicks there.");
    }
    let typing = matches!(act, Act::Type(_) | Act::Key(_));
    if typing && scene.focused.as_ref().is_some_and(|f| f.role == "AXSecureTextField") {
        return Some("That's a password field: equipe never types into one.");
    }
    let on = match act {
        Act::Type(_) | Act::Key(_) => scene.focused.as_ref(),
        _ => scene.target.as_ref(),
    };
    let window = scene.window.as_deref().filter(|_| !in_messaging);
    let sensitive = |text: &str| {
        let text = text.to_lowercase();
        BLOCKED_WORDS.iter().any(|w| text.contains(w))
    };
    if window.into_iter().chain(on.into_iter().flat_map(Element::names)).any(sensitive) {
        return Some("Payment, account deletion, passwords and security settings are the person's to do: equipe never acts there.");
    }
    None
}

/// The line the person reads: what is about to happen, where, and the draft.
pub fn describe(act: &Act, scene: &Scene) -> String {
    describe_in(crate::i18n::Lang::En, act, scene)
}

/// [`describe`] in `lang`.
pub fn describe_in(lang: crate::i18n::Lang, act: &Act, scene: &Scene) -> String {
    let words = crate::i18n::core(lang);
    let app = scene.app.name.clone();
    let what = match act {
        Act::Type(text) => words.fill("desktop.act.type", &[("text", format!("{:?}", short(text, 200))), ("app", app)]),
        Act::Key(keys) => words.fill("desktop.act.press", &[("keys", keys.clone()), ("app", app)]),
        Act::Drag => words.fill("desktop.act.drag", &[("app", app)]),
        Act::Click | Act::Look => {
            let name = scene.target.as_ref().and_then(|t| t.names().find(|n| !n.trim().is_empty()));
            match name {
                Some(name) => words.fill("desktop.act.click_on", &[("name", format!("{name:?}")), ("app", app)]),
                None => words.fill("desktop.act.click", &[("app", app)]),
            }
        }
    };
    let mut line = what;
    if let Some(window) = scene.window.as_deref().filter(|w| !w.trim().is_empty()) {
        line.push_str(&format!(" · {}", short(window, 80)));
    }
    let draft = scene.focused.as_ref().and_then(|f| f.value.as_deref()).filter(|v| !v.trim().is_empty());
    if let Some(draft) = draft {
        line.push_str(&format!("\n{}", short(draft, 600)));
    }
    line
}

fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…")
}

/// The person's `/send-apps` words applied to the squad's list: `add <app>`,
/// `remove <app>`, `reset`. The new list, or what to type instead.
pub fn edit_apps(apps: &[String], words: &str) -> Result<Vec<String>, String> {
    let (verb, app) = words.trim().split_once(' ').map_or((words.trim(), ""), |(v, a)| (v, a.trim()));
    let mut apps = apps.to_vec();
    let found = apps.iter().position(|a| a.eq_ignore_ascii_case(app));
    match (verb, found) {
        ("reset", _) if app.is_empty() => Ok(DEFAULT_APPS.iter().map(|a| (*a).to_owned()).collect()),
        ("add", _) if app.is_empty() => Err("Which app? /send-apps add Notion".into()),
        ("add", Some(_)) => Err(format!("{app} is on the list already.")),
        ("add", None) => {
            apps.push(app.to_owned());
            Ok(apps)
        }
        ("remove", Some(i)) => {
            apps.remove(i);
            Ok(apps)
        }
        ("remove", None) => Err(format!("{app:?} isn't on the list.")),
        _ => Err("/send-apps add <app> · /send-apps remove <app> · /send-apps reset".into()),
    }
}

/// A desktop input the daemon checks before handing it to the helper.
#[derive(Debug, Clone, Default)]
pub struct Check {
    pub squad: String,
    pub member: String,
    /// The desktop tool, as the member called it, and its arguments.
    pub tool: String,
    pub input: Value,
    pub scene: Scene,
    /// The screen right now (PNG), shown to the person if it asks.
    pub screen: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn apps() -> Vec<String> {
        DEFAULT_APPS.iter().map(|a| (*a).to_owned()).collect()
    }

    fn teams() -> Scene {
        Scene {
            app: App { name: "Microsoft Teams".into(), bundle_id: Some("com.microsoft.teams2".into()) },
            window: Some("Chat | Kim Minji".into()),
            target: None,
            focused: Some(Element { role: "AXTextArea".into(), value: Some("내일 회의 3시로 옮겨요".into()), ..Default::default() }),
        }
    }

    fn textedit() -> Scene {
        Scene { app: App { name: "TextEdit".into(), bundle_id: Some("com.apple.TextEdit".into()) }, ..Default::default() }
    }

    fn button(title: &str) -> Element {
        Element { role: "AXButton".into(), title: Some(title.into()), ..Default::default() }
    }

    fn with_target(mut scene: Scene, target: Element) -> Scene {
        scene.target = Some(target);
        scene
    }

    fn asks(v: Verdict) -> bool {
        matches!(v, Verdict::Ask(_))
    }

    #[test]
    fn looking_is_free_everywhere() {
        for tool in ["mcp__desktop__screenshot", "scroll", "elements", "focus_window", "open_app", "list_windows"] {
            assert_eq!(judge(&Act::of(tool, &json!({})), &teams(), &apps()), Verdict::Free, "{tool}");
        }
    }

    #[test]
    fn a_draft_goes_ahead_a_line_break_asks() {
        assert_eq!(judge(&Act::Type("내일 회의 3시로 옮겨요".into()), &teams(), &apps()), Verdict::Free);
        assert!(asks(judge(&Act::Type("ok\n".into()), &teams(), &apps())));
        assert!(asks(judge(&Act::Type("ok\r".into()), &teams(), &apps())));
    }

    #[test]
    fn return_asks_in_messaging_apps() {
        for keys in ["return", "Return", "cmd+return", "Cmd+Enter", "ctrl+enter", "shift+return", "kp_enter"] {
            assert!(asks(judge(&Act::Key(keys.into()), &teams(), &apps())), "{keys}");
        }
        let mail = Scene { app: App { name: "Mail".into(), bundle_id: None }, ..Default::default() };
        assert!(asks(judge(&Act::Key("cmd+shift+d".into()), &mail, &apps())));
        for keys in ["cmd+a", "cmd+v", "tab", "escape", "backspace", "down", "cmd+z"] {
            assert_eq!(judge(&Act::Key(keys.into()), &teams(), &apps()), Verdict::Free, "{keys}");
        }
    }

    #[test]
    fn space_on_a_focused_send_button_asks() {
        let mut scene = teams();
        scene.focused = Some(button("Send"));
        assert!(asks(judge(&Act::Key("space".into()), &scene, &apps())));
        scene.focused = Some(Element { role: "AXTextArea".into(), ..Default::default() });
        assert_eq!(judge(&Act::Key("space".into()), &scene, &apps()), Verdict::Free);
    }

    #[test]
    fn clicks_ask_only_on_what_reaches_people() {
        let all = apps();
        for name in ["Send", "Send (⌘↩)", "보내기", "전송", "Share screen", "Accept", "Join now", "Video call", "Upload"] {
            assert!(asks(judge(&Act::Click, &with_target(teams(), button(name)), &all)), "{name}");
        }
        for name in ["Chat", "Activity", "Recall", "Calendar", "Format"] {
            assert_eq!(judge(&Act::Click, &with_target(teams(), button(name)), &all), Verdict::Free, "{name}");
        }
        let field = Element { role: "AXTextArea".into(), description: Some("Type a message".into()), ..Default::default() };
        assert_eq!(judge(&Act::Click, &with_target(teams(), field), &all), Verdict::Free);
        let unnamed = Element { role: "AXButton".into(), ..Default::default() };
        assert!(asks(judge(&Act::Click, &with_target(teams(), unnamed), &all)), "an icon button with no name");
        assert!(asks(judge(&Act::Click, &teams(), &all)), "AX doesn't say what's there");
        assert!(asks(judge(&Act::Drag, &teams(), &all)));
    }

    #[test]
    fn other_apps_never_ask() {
        let all = apps();
        assert_eq!(judge(&Act::Key("return".into()), &textedit(), &all), Verdict::Free);
        assert_eq!(judge(&Act::Type("a\nb".into()), &textedit(), &all), Verdict::Free);
        assert_eq!(judge(&Act::Click, &textedit(), &all), Verdict::Free);
        assert_eq!(judge(&Act::Click, &with_target(textedit(), button("Send")), &all), Verdict::Free);
    }

    #[test]
    fn the_list_is_the_squads() {
        let only_slack = vec!["slack".to_owned()];
        assert_eq!(judge(&Act::Key("return".into()), &teams(), &only_slack), Verdict::Free);
        let slack = Scene { app: App { name: "Slack".into(), bundle_id: None }, ..Default::default() };
        assert!(asks(judge(&Act::Key("return".into()), &slack, &only_slack)));
        // An added app, by bundle id.
        let notion = Scene { app: App { name: "Notion".into(), bundle_id: Some("notion.id".into()) }, ..Default::default() };
        assert!(asks(judge(&Act::Key("return".into()), &notion, &["notion.id".to_owned()])));
        // A renamed default still counts by its bundle id.
        let renamed = Scene { app: App { name: "Microsoft Teams (work or school)".into(), bundle_id: Some("com.microsoft.teams2".into()) }, ..Default::default() };
        assert!(asks(judge(&Act::Key("return".into()), &renamed, &apps())));
    }

    #[test]
    fn some_screens_take_no_input_in_any_app() {
        let all = apps();
        let mut login = textedit();
        login.focused = Some(Element { role: "AXSecureTextField".into(), ..Default::default() });
        assert!(matches!(judge(&Act::Type("hunter2".into()), &login, &all), Verdict::Blocked(_)));
        let settings = Scene {
            app: App { name: "System Settings".into(), bundle_id: Some("com.apple.systempreferences".into()) },
            window: Some("Privacy & Security".into()),
            ..Default::default()
        };
        assert!(matches!(judge(&Act::Click, &settings, &all), Verdict::Blocked(_)));
        let shop = with_target(textedit(), button("Pay now"));
        assert!(matches!(judge(&Act::Click, &shop, &all), Verdict::Blocked(_)));
        let keychain = Scene { app: App { name: "Keychain Access".into(), bundle_id: None }, ..Default::default() };
        assert!(matches!(judge(&Act::Click, &keychain, &all), Verdict::Blocked(_)));
        // Looking at such a screen is still fine.
        assert_eq!(judge(&Act::Look, &settings, &all), Verdict::Free);
        // A conversation's name in a messaging app isn't a screen: "결제팀" chats go on asking, not blocking.
        let mut chat = teams();
        chat.window = Some("결제팀".into());
        assert_eq!(judge(&Act::Type("확인했습니다".into()), &chat, &all), Verdict::Free);
        assert!(asks(judge(&Act::Key("return".into()), &chat, &all)));
    }

    #[test]
    fn the_person_edits_the_list() {
        let list = vec!["Slack".to_owned()];
        assert_eq!(edit_apps(&list, "add Microsoft Teams").unwrap(), ["Slack", "Microsoft Teams"]);
        assert!(edit_apps(&list, "add slack").is_err());
        assert_eq!(edit_apps(&list, "remove slack").unwrap(), Vec::<String>::new());
        assert!(edit_apps(&list, "remove Zoom").is_err());
        assert_eq!(edit_apps(&list, "reset").unwrap(), apps());
        assert!(edit_apps(&list, "add").is_err());
        assert!(edit_apps(&list, "frobnicate").is_err());
    }

    #[test]
    fn tools_read_as_acts() {
        assert_eq!(Act::of("mcp__desktop__type_text", &json!({ "text": "hi" })), Act::Type("hi".into()));
        assert_eq!(Act::of("key", &json!({ "keys": "cmd+return" })), Act::Key("cmd+return".into()));
        assert_eq!(Act::of("key", &json!({ "key": "return" })), Act::Key("return".into()));
        assert_eq!(Act::of("double_click", &json!({})), Act::Click);
        assert_eq!(Act::of("something_new", &json!({})), Act::Click);
    }

    #[test]
    fn the_person_reads_what_and_where_and_the_draft() {
        let line = describe(&Act::Key("return".into()), &teams());
        assert_eq!(line, "press return in Microsoft Teams · Chat | Kim Minji\n내일 회의 3시로 옮겨요");
        assert!(describe(&Act::Click, &with_target(teams(), button("Send"))).starts_with("click \"Send\" in Microsoft Teams"));
    }
}
