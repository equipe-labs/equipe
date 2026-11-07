//! What the person can set from their screen, set from the conversation: the
//! PM reads the squad's settings (`settings`) and changes one when the person
//! asks (`change_setting`). What only narrows what the squad does is done at
//! once; what widens it — more done without asking, the PM accepting work,
//! a merge, a signal reading their data again — goes to the person as an
//! approval card that says exactly what changes, and only their tap does it.
//! The member's word never stands for their yes. No UI types here.

use serde_json::{Value, json};

use crate::i18n::{Lang, core};
use crate::protocol::engines::{Reviewer, Switching};
use crate::protocol::{Engine, Mode};

pub const SETTINGS: &str = "settings";
pub const CHANGE_SETTING: &str = "change_setting";

/// The tool an approval for a change is asked under: the person's yes makes
/// the daemon do it, not the member.
pub const SETTING_TOOL: &str = "Change setting";

/// The tool an engine move the PM asked for is put to the person under: a
/// move as their own model picker makes it, so it is decided as one.
pub const ENGINE_TOOL: &str = "Move engine";

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
    /// A member's model, within the engine it runs on.
    Model { member: String, model: String },
    /// Members moved to another engine — on `model` and at `effort` there,
    /// when named (else what that engine recommends for each one's role).
    Engine { members: Vec<String>, engine: Engine, model: Option<String>, effort: Option<String> },
    /// A member's effort; none: its engine's default.
    Effort { member: String, effort: Option<String> },
    /// How members move between engines as plans run low.
    EngineSwitching(Switching),
    /// Which engine reviews the designer's work.
    DesignReviewer(Reviewer),
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
            // Always the other engine draws on its plan however low it runs:
            // the person's. Auto and the member's own only narrow it.
            Change::DesignReviewer(reviewer) => *reviewer == Reviewer::Other,
            Change::Signal { on, .. } => *on,
            // Another plan, another bill, another way of working: the person's, always.
            Change::Merge { .. } | Change::Engine { .. } => true,
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
            // The model named as `model` rather than `value`: read as meant.
            "model" => Change::Model { member: member()?, model: s("value").or_else(|| s("model")).ok_or("Give the new `value`.")? },
            "engine" => {
                let engine = match value()?.to_ascii_lowercase().as_str() {
                    "claude" | "claude code" | "claude_code" => Engine::Claude,
                    "codex" => Engine::Codex,
                    v => return Err(format!("An engine is claude or codex, not {v:?}.")),
                };
                let one = s("member");
                let mut members: Vec<String> = Vec::new();
                let named = args.get("members").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).chain(one.as_deref());
                for name in named.map(str::trim).filter(|n| !n.is_empty()) {
                    if !members.iter().any(|m| m == name) {
                        members.push(name.to_owned());
                    }
                }
                if members.is_empty() {
                    return Err("Which members? Give `members` (or one `member`).".to_owned());
                }
                Change::Engine { members, engine, model: s("model"), effort: s("effort").filter(|e| e != "default") }
            }
            "effort" => Change::Effort { member: member()?, effort: Some(s("value").or_else(|| s("effort")).ok_or("Give the new `value`.")?).filter(|v| v != "default") },
            "engine_switching" => Change::EngineSwitching(match value()?.as_str() {
                "off" => Switching::Off,
                "suggest" => Switching::Suggest,
                "auto" => Switching::Auto,
                v => return Err(format!("Engine switching is off, suggest or auto, not {v:?}.")),
            }),
            "design_reviewer" => Change::DesignReviewer(Reviewer::parse(&value()?).ok_or("The design reviewer is auto, same (the member's own engine, in a fresh session) or other (always the other engine).")?),
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

/// Whether a model runs on an engine, as far as its name and Codex's list say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFit {
    Fits,
    /// Another engine's: set it there by moving the member (`engine`).
    Other(Engine),
    /// Neither engine's, as far as is known.
    Unknown,
}

/// Whether `model` runs on `engine`: Claude Code takes its families' aliases
/// and ids (`crate::models`); Codex takes what its own live list has
/// (`codex`, `None` when it couldn't be listed) and a connected provider's
/// models, which run on its agent.
pub fn model_fit(engine: Engine, model: &str, codex: Option<&[String]>) -> ModelFit {
    let model = model.trim();
    let provider = crate::model_providers::split_ref(model).is_some();
    let claude = !provider && crate::models::family(model).is_some();
    let listed = codex.is_some_and(|list| list.iter().any(|m| m == model));
    let lower = model.to_ascii_lowercase();
    // What Codex's ids look like, for when its list can't say.
    let codex_like = listed || provider || lower.starts_with("gpt-") || lower.contains("codex") || (lower.starts_with('o') && lower[1..].starts_with(|c: char| c.is_ascii_digit()));
    match engine {
        Engine::Claude if claude => ModelFit::Fits,
        Engine::Claude if codex_like => ModelFit::Other(Engine::Codex),
        Engine::Codex if listed || provider => ModelFit::Fits,
        Engine::Codex if claude => ModelFit::Other(Engine::Claude),
        _ => ModelFit::Unknown,
    }
}

/// An engine as people name it.
pub fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "Claude",
        Engine::Codex => "Codex",
    }
}

/// A member an engine move takes, as it stands when the card is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mover {
    pub name: String,
    pub from: Engine,
    /// Mid-turn.
    pub working: bool,
    /// On a ticket: it finishes it where it is.
    pub on_ticket: bool,
}

/// When a move to `to` reaches `m`, in the model picker's own words: the
/// daemon's rule — between tickets, else when the turn ends, else at once.
pub fn move_when(lang: Lang, m: &Mover, to: Engine) -> String {
    let words = core(lang);
    let (name, from, to) = (("name", m.name.clone()), ("from", engine_name(m.from).to_owned()), ("to", engine_name(to).to_owned()));
    if m.on_ticket {
        words.fill("control.engine_ticket", &[name, from, to])
    } else if m.working {
        words.fill("control.engine_turn_end", &[name, to])
    } else {
        words.fill("control.engine_free", &[name, to])
    }
}

/// An engine move's card: who moves where, on what model and effort there
/// (`model` as people name it), then when each one moves.
pub fn move_words(lang: Lang, movers: &[Mover], to: Engine, model: Option<&str>, effort: Option<&str>) -> String {
    let words = core(lang);
    let engine = ("engine", engine_name(to).to_owned());
    let names = movers.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ");
    let mut lines = vec![words.fill("control.engine", &[("members", names), engine.clone()])];
    lines.push(match model {
        Some(model) => words.fill("control.engine_model", &[("model", model.to_owned())]),
        None => words.fill("control.engine_model_default", &[engine]),
    });
    if let Some(effort) = effort {
        lines.push(words.fill("control.engine_effort", &[("effort", effort.to_owned())]));
    }
    lines.extend(movers.iter().map(|m| move_when(lang, m, to)));
    lines.join("\n")
}

/// The PM's tools for the squad's settings.
pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": SETTINGS,
            "description": "Read the squad's settings, as the person sees them on their screen: its projects and HQ, \
how much members may do on their own (the squad's mode and each member's), each member's engine, model and effort, \
whether you accept the team's work yourself, how long an empty board waits before you are woken, engine switching, \
who reviews the designer's work, routines, signals, and the \"always allow\" rules. Read it before you change one, and to answer what is set.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": CHANGE_SETTING,
            "description": "Change one of the squad's settings, only when the person asked for that change. One that \
only narrows what the squad does happens at once. One that widens it — a mode that lets members do more on their own, \
you accepting the team's work, engine switching on auto, a signal on again, a merge, a move to another engine — is put \
to the person as an approval card saying exactly what changes; it happens only if they tap yes, and this call waits for \
their answer. Never ask them to tell you yes instead. Settings: mode (value read_only | ask | auto | full, or default \
with member), pm_accepts (on | off), idle_wake_hours (a number, 0 off), model (member, value: a model of the engine \
the member runs on), effort (member, value, or default), engine (members — every one in one call — value claude | \
codex, and optionally model and effort there), engine_switching (off | suggest | auto), design_reviewer (who reviews \
the designer's work: auto — the other engine when it is signed in with room, else the member's own in a fresh session — | \
same | other), remove_project (project), \
hq (project, or none), run_routine (id), delete_routine (id), signal (project, name, value on | off), disallow (value: \
the rule as `settings` lists it), merge (ticket, push), keep_on_branch (ticket). Never set another engine's model \
through `model`: moving a member to it is `engine`. Adding a project is `add_project`; adding a routine, \
`propose_routine`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "setting": { "type": "string", "enum": ["mode", "pm_accepts", "idle_wake_hours", "model", "effort", "engine", "engine_switching", "design_reviewer", "remove_project", "hq", "run_routine", "delete_routine", "signal", "disallow", "merge", "keep_on_branch"] },
                    "value": { "type": "string", "description": "The new value, as the setting takes it." },
                    "member": { "type": "string", "description": "The member, for mode (left out: the squad's), model, effort and engine." },
                    "members": { "type": "array", "items": { "type": "string" }, "description": "engine: the members to move, on one card." },
                    "model": { "type": "string", "description": "engine: the model there (left out: what that engine recommends for each one's role)." },
                    "effort": { "type": "string", "description": "engine: the effort there (left out: its default)." },
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
        assert!(change(json!({ "setting": "design_reviewer", "value": "other" })).needs_yes(Mode::Auto), "always the other plan: the person's");
        for args in [
            json!({ "setting": "idle_wake_hours", "value": "6" }),
            json!({ "setting": "design_reviewer", "value": "same" }),
            json!({ "setting": "design_reviewer", "value": "auto" }),
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
        assert_eq!(change(json!({ "setting": "design_reviewer", "value": "same" })), Change::DesignReviewer(Reviewer::Same));
        assert_eq!(change(json!({ "setting": "merge", "ticket": 4, "push": false })), Change::Merge { ticket: 4, push: false });
        let err = |args: Value| Change::parse(&args).unwrap_err();
        assert!(err(json!({ "setting": "mode", "value": "default" })).contains("default puts a member"), "the squad has no default but its own");
        assert!(err(json!({ "setting": "idle_wake_hours", "value": "soon" })).contains("hours"));
        assert!(err(json!({ "setting": "model", "value": "opus" })).contains("member"));
        assert!(err(json!({ "setting": "colour" })).contains("`settings`"));
        assert!(err(json!({ "setting": "design_reviewer", "value": "gemini" })).contains("auto, same"));
        let tools = tools();
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, [SETTINGS, CHANGE_SETTING]);
        let listed = tools[1]["inputSchema"]["properties"]["setting"]["enum"].as_array().unwrap();
        for setting in listed {
            let doc = tools[1]["description"].as_str().unwrap();
            assert!(doc.contains(setting.as_str().unwrap()), "{setting} is explained");
        }
    }

    #[test]
    fn an_engine_move_names_its_members_and_engine_and_always_waits_for_the_persons_tap() {
        let moved = change(json!({ "setting": "engine", "members": ["designer", "mobile", "mobile-2"], "value": "codex" }));
        assert_eq!(moved, Change::Engine { members: vec!["designer".into(), "mobile".into(), "mobile-2".into()], engine: Engine::Codex, model: None, effort: None });
        for now in [Mode::ReadOnly, Mode::Ask, Mode::Auto, Mode::Full] {
            assert!(moved.needs_yes(now), "never at once, in any mode");
        }
        // One member as `member`, both at once, each once; the model and effort there.
        let one = change(json!({ "setting": "engine", "member": "qa", "members": ["backend", "qa"], "value": "Claude", "model": "opus", "effort": "high" }));
        assert_eq!(one, Change::Engine { members: vec!["backend".into(), "qa".into()], engine: Engine::Claude, model: Some("opus".into()), effort: Some("high".into()) });
        assert_eq!(change(json!({ "setting": "engine", "member": "qa", "value": "codex", "effort": "default" })), Change::Engine { members: vec!["qa".into()], engine: Engine::Codex, model: None, effort: None });
        let err = |args: Value| Change::parse(&args).unwrap_err();
        assert!(err(json!({ "setting": "engine", "value": "codex" })).contains("`members`"));
        assert!(err(json!({ "setting": "engine", "members": [], "value": "codex" })).contains("`members`"));
        assert!(err(json!({ "setting": "engine", "member": "qa", "value": "gemini" })).contains("claude or codex"));
        // A model named as `model` for the model setting is read as its value.
        assert_eq!(change(json!({ "setting": "model", "member": "qa", "model": "sonnet" })), Change::Model { member: "qa".into(), model: "sonnet".into() });
        let doc = tools()[1]["description"].as_str().unwrap().to_owned();
        assert!(doc.contains("Never set another engine's model through `model`") && doc.contains("a move to another engine"), "{doc}");
    }

    #[test]
    fn a_model_fits_the_engine_that_runs_it_and_another_engines_is_told_apart() {
        let codex: Vec<String> = ["gpt-6.1-sol", "gpt-6-luna"].map(String::from).to_vec();
        let list = Some(codex.as_slice());
        for claude in ["opus", "sonnet", "claude-opus-5-5", "claude-sonnet-4-5-20250929", "claude-opus-5-5[1m]"] {
            assert_eq!(model_fit(Engine::Claude, claude, list), ModelFit::Fits, "{claude}");
            assert_eq!(model_fit(Engine::Codex, claude, list), ModelFit::Other(Engine::Claude), "{claude}");
        }
        assert_eq!(model_fit(Engine::Codex, "gpt-6.1-sol", list), ModelFit::Fits);
        assert_eq!(model_fit(Engine::Claude, "gpt-6.1-sol", list), ModelFit::Other(Engine::Codex), "the bug: a Codex model set on a Claude member");
        // Without Codex's list, its ids still read as its own — never as Claude's.
        assert_eq!(model_fit(Engine::Claude, "gpt-6.1-sol", None), ModelFit::Other(Engine::Codex));
        assert_eq!(model_fit(Engine::Claude, "o4-mini", None), ModelFit::Other(Engine::Codex));
        assert_eq!(model_fit(Engine::Codex, "gpt-6.1-sol", None), ModelFit::Unknown, "not on a list that couldn't be read");
        assert_eq!(model_fit(Engine::Codex, "gpt-7-nova", list), ModelFit::Unknown, "not on the list");
        // A provider's model runs on Codex's agent: setting it on Claude would move the member.
        assert_eq!(model_fit(Engine::Codex, "@openrouter/qwen/qwen3-coder", list), ModelFit::Fits);
        assert_eq!(model_fit(Engine::Claude, "@openrouter/qwen/qwen3-coder", list), ModelFit::Other(Engine::Codex));
        assert_eq!(model_fit(Engine::Claude, "@openrouter/anthropic/claude-opus-5", list), ModelFit::Other(Engine::Codex));
        assert_eq!(model_fit(Engine::Claude, "llama-4", list), ModelFit::Unknown);
    }

    #[test]
    fn an_engine_moves_card_says_who_moves_where_and_when_each_one_does_in_the_pickers_words() {
        let mover = |name: &str, working: bool, on_ticket: bool| Mover { name: name.into(), from: Engine::Claude, working, on_ticket };
        let movers = [mover("designer", false, true), mover("mobile", true, false), mover("mobile-2", false, false)];
        let card = move_words(Lang::En, &movers, Engine::Codex, None, None);
        assert_eq!(
            card.lines().collect::<Vec<_>>(),
            [
                "Move designer, mobile, mobile-2 to Codex",
                "Model there: the one Codex recommends for each one's role",
                "designer keeps Claude for the ticket in progress and switches to Codex from its next ticket, its work handed over.",
                "mobile switches to Codex when this turn ends, its conversation handed over.",
                "mobile-2 switches to Codex as soon as it's free, its conversation handed over.",
            ]
        );
        let picked = move_words(Lang::En, &movers[2..], Engine::Codex, Some("GPT-6.1 Sol"), Some("high"));
        assert_eq!(picked.lines().take(3).collect::<Vec<_>>(), ["Move mobile-2 to Codex", "Model there: GPT-6.1 Sol", "Effort there: high"]);
        // In the person's language, its particles fitted to each name.
        let ko = move_words(Lang::Ko, &movers, Engine::Codex, None, None);
        let ko: Vec<&str> = ko.lines().collect();
        assert_eq!(ko[0], "designer, mobile, mobile-2를 Codex로 옮겨요");
        assert_eq!(ko[2], "designer는 진행 중인 티켓은 Claude로 마치고 다음 티켓부터 Codex로 옮겨요. 하던 일은 넘겨줘요.");
        assert_eq!(ko[3], "mobile은 이번 턴이 끝나면 Codex로 옮겨요. 대화 내용은 넘겨줘요.");
        assert_eq!(ko[4], "mobile-2는 손이 비는 대로 Codex로 옮겨요. 대화 내용은 넘겨줘요.");
        // Every language words each case.
        for lang in Lang::ALL {
            let card = move_words(lang, &movers, Engine::Codex, Some("GPT-6.1 Sol"), Some("high"));
            assert_eq!(card.lines().count(), 6, "{lang:?}: {card}");
            assert!(!card.contains('{') && card.contains("mobile-2") && card.contains("GPT-6.1 Sol"), "{lang:?}: {card}");
        }
    }
}
