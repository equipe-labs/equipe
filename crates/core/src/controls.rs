//! What the person can set from their screen, set from the conversation: the
//! PM reads the squad's settings (`settings`) and changes one when the person
//! asks (`change_setting`). What only narrows what the squad does is done at
//! once; what widens it — more done without asking, the PM accepting work,
//! a merge, a signal reading their data again — goes to the person as an
//! approval card that says exactly what changes, and only their tap does it.
//! The member's word never stands for their yes. No UI types here.

use serde_json::{Value, json};

use crate::protocol::Mode;
use crate::protocol::engines::Switching;

pub const SETTINGS: &str = "settings";
pub const CHANGE_SETTING: &str = "change_setting";

/// The tool an approval for a change is asked under: the person's yes makes
/// the daemon do it, not the member.
pub const SETTING_TOOL: &str = "Change setting";

/// One change, as the PM asked for it — names, not ids: the runtime finds
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// How much members may do on their own: the squad's default (`member`
    /// none), or one member's (`mode` none: back on the default).
    Mode { member: Option<String>, mode: Option<Mode> },
    /// The PM accepts and merges handed-in work itself.
    PmAccepts(bool),
    /// Hours the board may lie empty before the PM is woken; 0 off.
    IdleWake(u32),
    /// A member's model.
    Model { member: String, model: String },
    /// A member's effort; none: its engine's default.
    Effort { member: String, effort: Option<String> },
    /// How members move between engines as plans run low.
    EngineSwitching(Switching),
    /// Take a project off the squad (its folder stays).
    RemoveProject(String),
    /// The squad's HQ project, or none.
    Hq(Option<String>),
    RunRoutine(u32),
    DeleteRoutine(u32),
    /// A signal read again (`on`) or paused.
    Signal { project: String, name: String, on: bool },
    /// Take back an "always allow" rule: that kind of thing asks again.
    Disallow(String),
    /// Merge a done ticket's branch into main (and push it).
    Merge { ticket: u32, push: bool },
    /// Leave a done ticket's work on its branch.
    KeepOnBranch(u32),
}

/// How far a mode lets members go, for telling a wider one from a narrower.
fn reach(mode: Mode) -> u8 {
    match mode {
        Mode::ReadOnly => 0,
        Mode::Ask => 1,
        Mode::Auto => 2,
        Mode::Full => 3,
    }
}

impl Change {
    /// Whether only the person's tap may make it: it widens what the squad
    /// does without them (`now`: the mode it would replace).
    pub fn needs_yes(&self, now: Mode) -> bool {
        match self {
            Change::Mode { mode, .. } => mode.is_none_or(|m| reach(m) > reach(now)),
            Change::PmAccepts(on) => *on,
            Change::EngineSwitching(mode) => *mode == Switching::Auto,
            Change::Signal { on, .. } => *on,
            Change::Merge { .. } => true,
            _ => false,
        }
    }

    /// Read from `change_setting`'s arguments.
    pub fn parse(args: &Value) -> Result<Change, String> {
        let s = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        let n = |key: &str| args.get(key).and_then(Value::as_u64);
        let value = || s("value").ok_or_else(|| "Give the new `value`.".to_owned());
        let member = || s("member").ok_or_else(|| "Which member? Give `member`.".to_owned());
        let ticket = || n("ticket").and_then(|t| u32::try_from(t).ok()).ok_or_else(|| "Which ticket? Give its number in `ticket`.".to_owned());
        let id = || n("id").and_then(|t| u32::try_from(t).ok()).ok_or_else(|| "Which routine? Give its `id` (from `settings`).".to_owned());
        let on = |v: &str| match v {
            "on" | "true" | "yes" => Ok(true),
            "off" | "false" | "no" => Ok(false),
            _ => Err(format!("`value` is on or off, not {v:?}.")),
        };
        let setting = s("setting").ok_or("Which setting? Give `setting`.")?;
        Ok(match setting.as_str() {
            "mode" => {
                let mode = match value()?.as_str() {
                    "read_only" => Some(Mode::ReadOnly),
                    "ask" => Some(Mode::Ask),
                    "auto" => Some(Mode::Auto),
                    "full" => Some(Mode::Full),
                    "default" if s("member").is_some() => None,
                    v => return Err(format!("A mode is read_only, ask, auto or full (default puts a member back on the squad's), not {v:?}.")),
                };
                Change::Mode { member: s("member"), mode }
            }
            "pm_accepts" => Change::PmAccepts(on(&value()?)?),
            "idle_wake_hours" => Change::IdleWake(value()?.parse().map_err(|_| "`value` is a number of hours; 0 turns it off.")?),
            "model" => Change::Model { member: member()?, model: value()? },
            "effort" => Change::Effort { member: member()?, effort: Some(value()?).filter(|v| v != "default") },
            "engine_switching" => Change::EngineSwitching(match value()?.as_str() {
                "off" => Switching::Off,
                "suggest" => Switching::Suggest,
                "auto" => Switching::Auto,
                v => return Err(format!("Engine switching is off, suggest or auto, not {v:?}.")),
            }),
            "remove_project" => Change::RemoveProject(s("project").ok_or("Which project? Give `project`.")?),
            "hq" => Change::Hq(s("project").filter(|p| p != "none")),
            "run_routine" => Change::RunRoutine(id()?),
            "delete_routine" => Change::DeleteRoutine(id()?),
            "signal" => Change::Signal { project: s("project").ok_or("Whose signal? Give its `project`.")?, name: s("name").ok_or("Which signal? Give its `name`.")?, on: on(&value()?)? },
            "disallow" => Change::Disallow(value()?),
            "merge" => Change::Merge { ticket: ticket()?, push: args.get("push").and_then(Value::as_bool).unwrap_or(true) },
            "keep_on_branch" => Change::KeepOnBranch(ticket()?),
            other => return Err(format!("There is no setting {other:?}; `settings` lists what can change.")),
        })
    }
}

/// The PM's tools for the squad's settings.
pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": SETTINGS,
            "description": "Read the squad's settings, as the person sees them on their screen: its projects and HQ, \
how much members may do on their own (the squad's mode and each member's), each member's engine, model and effort, \
whether you accept the team's work yourself, how long an empty board waits before you are woken, engine switching, \
routines, signals, and the \"always allow\" rules. Read it before you change one, and to answer what is set.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": CHANGE_SETTING,
            "description": "Change one of the squad's settings, only when the person asked for that change. One that \
only narrows what the squad does happens at once. One that widens it — a mode that lets members do more on their own, \
you accepting the team's work, engine switching on auto, a signal on again, a merge — is put to the person as an \
approval card saying exactly what changes; it happens only if they tap yes, and this call waits for their answer. \
Never ask them to tell you yes instead. Settings: mode (value read_only | ask | auto | full, or default with member), \
pm_accepts (on | off), idle_wake_hours (a number, 0 off), model (member, value), effort (member, value, or default), \
engine_switching (off | suggest | auto), remove_project (project), hq (project, or none), run_routine (id), \
delete_routine (id), signal (project, name, value on | off), disallow (value: the rule as `settings` lists it), \
merge (ticket, push), keep_on_branch (ticket). Adding a project is `add_project`; adding a routine, `propose_routine`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "setting": { "type": "string", "enum": ["mode", "pm_accepts", "idle_wake_hours", "model", "effort", "engine_switching", "remove_project", "hq", "run_routine", "delete_routine", "signal", "disallow", "merge", "keep_on_branch"] },
                    "value": { "type": "string", "description": "The new value, as the setting takes it." },
                    "member": { "type": "string", "description": "The member, for mode (left out: the squad's), model and effort." },
                    "project": { "type": "string", "description": "The project's name, for remove_project, hq and signal." },
                    "name": { "type": "string", "description": "The signal's name." },
                    "id": { "type": "integer", "description": "The routine's id." },
                    "ticket": { "type": "integer", "description": "The ticket's number, for merge and keep_on_branch." },
                    "push": { "type": "boolean", "description": "merge: push main to origin too (default true)." },
                    "why": { "type": "string", "description": "What the person said they want, in one line — shown on a card that asks them." },
                },
                "required": ["setting"],
            },
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(args: Value) -> Change {
        Change::parse(&args).unwrap()
    }

    #[test]
    fn what_narrows_the_squad_happens_at_once_and_what_widens_it_waits_for_the_persons_tap() {
        let mode = |m: &str| change(json!({ "setting": "mode", "value": m, "member": "frontend" }));
        assert!(!mode("ask").needs_yes(Mode::Auto), "less done alone: at once");
        assert!(!mode("auto").needs_yes(Mode::Auto), "as it is");
        assert!(mode("full").needs_yes(Mode::Auto));
        assert!(mode("auto").needs_yes(Mode::Ask), "more done alone");
        assert!(change(json!({ "setting": "mode", "value": "default", "member": "qa" })).needs_yes(Mode::ReadOnly), "the default may be wider");
        assert!(change(json!({ "setting": "pm_accepts", "value": "on" })).needs_yes(Mode::Auto));
        assert!(!change(json!({ "setting": "pm_accepts", "value": "off" })).needs_yes(Mode::Auto));
        assert!(change(json!({ "setting": "merge", "ticket": 12 })).needs_yes(Mode::Auto));
        assert!(change(json!({ "setting": "signal", "project": "site", "name": "posthog", "value": "on" })).needs_yes(Mode::Auto));
        assert!(!change(json!({ "setting": "signal", "project": "site", "name": "posthog", "value": "off" })).needs_yes(Mode::Auto));
        assert!(change(json!({ "setting": "engine_switching", "value": "auto" })).needs_yes(Mode::Auto));
        for args in [
            json!({ "setting": "idle_wake_hours", "value": "6" }),
            json!({ "setting": "model", "member": "pm", "value": "opus" }),
            json!({ "setting": "effort", "member": "backend", "value": "high" }),
            json!({ "setting": "remove_project", "project": "old-site" }),
            json!({ "setting": "hq", "project": "none" }),
            json!({ "setting": "run_routine", "id": 3 }),
            json!({ "setting": "delete_routine", "id": 3 }),
            json!({ "setting": "disallow", "value": "Bash(npm test:*)" }),
            json!({ "setting": "keep_on_branch", "ticket": 9 }),
        ] {
            assert!(!change(args.clone()).needs_yes(Mode::Auto), "{args}");
        }
    }

    #[test]
    fn a_setting_is_read_as_the_pm_names_it_and_a_wrong_one_says_what_would_do() {
        assert_eq!(change(json!({ "setting": "mode", "value": "ask" })), Change::Mode { member: None, mode: Some(Mode::Ask) });
        assert_eq!(change(json!({ "setting": "effort", "member": "qa", "value": "default" })), Change::Effort { member: "qa".into(), effort: None });
        assert_eq!(change(json!({ "setting": "hq", "project": "site" })), Change::Hq(Some("site".into())));
        assert_eq!(change(json!({ "setting": "merge", "ticket": 4, "push": false })), Change::Merge { ticket: 4, push: false });
        let err = |args: Value| Change::parse(&args).unwrap_err();
        assert!(err(json!({ "setting": "mode", "value": "default" })).contains("default puts a member"), "the squad has no default but its own");
        assert!(err(json!({ "setting": "idle_wake_hours", "value": "soon" })).contains("hours"));
        assert!(err(json!({ "setting": "model", "value": "opus" })).contains("member"));
        assert!(err(json!({ "setting": "colour" })).contains("`settings`"));
        let tools = tools();
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, [SETTINGS, CHANGE_SETTING]);
        let listed = tools[1]["inputSchema"]["properties"]["setting"]["enum"].as_array().unwrap();
        for setting in listed {
            let doc = tools[1]["description"].as_str().unwrap();
            assert!(doc.contains(setting.as_str().unwrap()), "{setting} is explained");
        }
    }
}
