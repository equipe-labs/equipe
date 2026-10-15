//! The console's commands in one place: what `/help` lists and what completion
//! offers as the person types `/…` or `@…`.

use equipe_core::protocol::{Mode, TicketId};
use equipe_core::i18n::label;
use equipe_core::t;

pub struct Spec {
    pub name: &'static str,
    pub args: &'static str,
    pub about: &'static str,
}

/// Every command, its `about` (and the words in its `args`) in the language in use — asked afresh each
/// time, since the person may switch language while the console runs.
pub fn commands() -> [Spec; 30] {
    [
        Spec { name: "usage", args: "", about: t!("commands.usage") },
        Spec { name: "compact", args: "[member]", about: t!("commands.compact") },
        Spec { name: "model", args: "[member] <model>", about: t!("commands.model") },
        Spec { name: "clear", args: "[member]", about: t!("commands.clear") },
        Spec { name: "squad", args: "[name] · new <name> [folder] · add|remove <folder>", about: t!("commands.squad") },
        Spec { name: "home", args: "[changes · undo [n]]", about: t!("commands.home") },
        Spec { name: "memory", args: "[changes]", about: t!("commands.memory") },
        Spec { name: "project", args: "[name|all]", about: t!("commands.project") },
        Spec { name: "accept", args: "T-n", about: t!("commands.accept") },
        Spec { name: "merge", args: "T-n", about: t!("commands.merge") },
        Spec { name: "reopen", args: t!("commands.reopen_args"), about: t!("commands.reopen") },
        Spec { name: "resume", args: t!("commands.resume_args"), about: t!("commands.resume") },
        Spec { name: "cancel", args: "T-n", about: t!("commands.cancel") },
        Spec { name: "board", args: "", about: t!("commands.board") },
        Spec { name: "new", args: t!("commands.new_args"), about: t!("commands.new") },
        Spec { name: "title", args: t!("commands.title_args"), about: t!("commands.title") },
        Spec { name: "find", args: t!("commands.find_args"), about: t!("commands.find") },
        Spec { name: "tidy", args: "", about: t!("commands.tidy") },
        Spec { name: "add", args: "<role…> | custom <name> [--edits]", about: t!("commands.add") },
        Spec { name: "close", args: "", about: t!("commands.close") },
        Spec { name: "mode", args: "[member] read-only|ask|auto|full", about: t!("commands.mode") },
        Spec { name: "browser", args: "[watch|show|hide [member] · signin · max <n>]", about: t!("commands.browser") },
        Spec { name: "send-apps", args: "[add|remove <app> · reset]", about: t!("commands.send_apps") },
        Spec { name: "desktop", args: "stop|resume", about: t!("commands.desktop") },
        Spec { name: "suggest", args: "[on|off]", about: t!("commands.suggest") },
        Spec { name: "export", args: "", about: t!("commands.export") },
        Spec { name: "mouse", args: "", about: t!("commands.mouse") },
        Spec { name: "lang", args: "[code|auto]", about: t!("commands.lang") },
        Spec { name: "help", args: "", about: t!("commands.help") },
        Spec { name: "quit", args: "", about: t!("commands.quit") },
    ]
}

/// One completion: what the composer holds once it's taken, and how it is
/// listed. A `text` without a trailing space is whole: Enter on it runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub text: String,
    pub label: String,
    pub about: String,
}

/// What completion draws on.
#[derive(Default)]
pub struct Context<'a> {
    /// Tickets waiting for the person's review: number and title.
    pub review: Vec<(TicketId, &'a str)>,
    /// Tickets that can be sent back: in review or done.
    pub reopenable: Vec<(TicketId, &'a str)>,
    /// Accepted tickets whose work isn't on main yet, for /merge.
    pub unmerged: Vec<(TicketId, &'a str)>,
    /// Tickets waiting on the person.
    pub waiting: Vec<(TicketId, &'a str)>,
    /// Tickets still open, for /cancel.
    pub open: Vec<(TicketId, &'a str)>,
    /// The squad's members: name and role.
    pub members: Vec<(&'a str, &'a str)>,
    pub roles: Vec<&'a str>,
    /// Every squad: its name and what it works on, the one shown first.
    pub squads: Vec<(String, String)>,
    /// The squad's mode, and each member's own where it differs.
    pub mode: Option<Mode>,
    /// The squad's projects: name, and how each is doing.
    pub projects: Vec<(String, String)>,
    pub member_modes: Vec<(&'a str, Mode)>,
    /// The models each member's engine offers, and the selected member's.
    pub member_models: Vec<(&'a str, Vec<String>)>,
    pub selected_models: Vec<String>,
}

const MODES: [&str; 4] = ["read-only", "ask", "auto", "full"];

/// The word that picks `mode` in `/mode`: `read-only`, `ask`, `auto`, `full`.
pub fn mode_word(mode: Mode) -> &'static str {
    MODES[Mode::ALL.iter().position(|m| *m == mode).unwrap_or(0)]
}

/// What could complete `text`, best first. Empty when nothing would, or when
/// the text is already whole.
pub fn complete(text: &str, ctx: &Context<'_>) -> Vec<Choice> {
    let choices = if let Some(name) = text.strip_prefix('@') {
        if name.contains(char::is_whitespace) {
            return Vec::new();
        }
        ctx.members
            .iter()
            .filter(|(m, _)| m.starts_with(&name.to_lowercase()))
            .map(|(m, role)| Choice { text: format!("@{m} "), label: format!("@{m}"), about: (*role).to_owned() })
            .collect()
    } else if let Some(command) = text.strip_prefix('/') {
        match command.split_once(' ') {
            None => commands()
                .iter()
                .filter(|c| c.name.starts_with(&command.to_lowercase()))
                .map(|c| Choice {
                    text: if c.args.is_empty() { format!("/{}", c.name) } else { format!("/{} ", c.name) },
                    label: format!("/{} {}", c.name, c.args).trim_end().to_owned(),
                    about: c.about.to_owned(),
                })
                .collect(),
            Some((name, rest)) => arguments(name, rest, ctx),
        }
    } else {
        Vec::new()
    };
    // Nothing to offer once the text is what the only choice would make it.
    match choices.as_slice() {
        [only] if only.text.trim_end() == text.trim_end() => Vec::new(),
        _ => choices,
    }
}

fn arguments(name: &str, rest: &str, ctx: &Context<'_>) -> Vec<Choice> {
    let words: Vec<&str> = rest.split(' ').collect();
    let typed = words.last().copied().unwrap_or("");
    let before = &rest[..rest.len() - typed.len()];
    let lead = format!("/{name} {before}");
    let word = |w: &str, about: &str, then: &str| Choice { text: format!("{lead}{w}{then}"), label: w.to_owned(), about: about.to_owned() };
    let tickets = |list: &[(TicketId, &str)], then: &str| -> Vec<Choice> {
        list.iter()
            .filter(|(id, title)| {
                let t = typed.trim_start_matches(['T', 't', '-']);
                format!("{id}").starts_with(t) || (!typed.is_empty() && title.to_lowercase().contains(&typed.to_lowercase()))
            })
            .map(|(id, title)| word(&format!("T-{id}"), title, then))
            .collect()
    };
    let starts = |w: &str| w.starts_with(&typed.to_lowercase());
    match (name, words.len()) {
        ("accept", 1) => tickets(&ctx.review, ""),
        ("cancel", 1) => tickets(&ctx.open, ""),
        ("merge", 1) => tickets(&ctx.unmerged, ""),
        // What to change follows the number.
        ("reopen", 1) => tickets(&ctx.reopenable, " "),
        ("resume", 1) => tickets(&ctx.waiting, " "),
        // Each mode says what it lets members do, and which one is on now.
        ("mode", 1) => {
            let mut choices: Vec<Choice> =
                MODES.iter().filter(|w| starts(w)).map(|m| word(m, &mode_about(m, ctx.mode, t!("commands.the_squad")), "")).collect();
            choices.extend(ctx.members.iter().filter(|(m, _)| starts(m)).map(|(m, _)| {
                let own = ctx.member_modes.iter().find(|(name, _)| name == m).map(|(_, mode)| *mode);
                let now = match (own, ctx.mode) {
                    (Some(mode), _) => t!("commands.mode_member_own", mode = label::mode(crate::i18n::lang(), mode)),
                    (None, Some(mode)) => t!("commands.mode_member_squad", mode = label::mode(crate::i18n::lang(), mode)),
                    (None, None) => t!("commands.mode_member").to_owned(),
                };
                word(m, &now, " ")
            }));
            choices
        }
        ("mode", 2) if ctx.members.iter().any(|(m, _)| *m == words[0]) => {
            let own = ctx.member_modes.iter().find(|(name, _)| *name == words[0]).map(|(_, mode)| *mode);
            let mut choices: Vec<Choice> =
                MODES.iter().filter(|w| starts(w)).map(|m| word(m, &mode_about(m, own, words[0]), "")).collect();
            if starts("squad") {
                let squad = ctx.mode.map_or(String::new(), |m| format!(" ({})", label::mode(crate::i18n::lang(), m)));
                choices.push(word("squad", &t!("commands.back_on_squad_mode", squad = squad), ""));
            }
            choices
        }
        ("add", _) => ctx.roles.iter().filter(|w| starts(w)).map(|r| word(r, "", " ")).collect(),
        ("compact" | "clear", 1) => ctx.members.iter().filter(|(m, _)| starts(m)).map(|(m, _)| word(m, "", "")).collect(),
        // A member, then a model of its engine — or straight a model, for the member selected.
        ("model", 1) => {
            let mut choices: Vec<Choice> = ctx.members.iter().filter(|(m, _)| starts(m)).map(|(m, role)| word(m, role, " ")).collect();
            choices.extend(ctx.selected_models.iter().filter(|m| starts(m)).map(|m| word(m, t!("commands.for_selected_member"), "")));
            choices
        }
        ("model", 2) => ctx
            .member_models
            .iter()
            .find(|(name, _)| *name == words[0])
            .map(|(_, models)| models.iter().filter(|m| starts(m)).map(|m| word(m, "", "")).collect())
            .unwrap_or_default(),
        ("home", 1) => [("changes", t!("commands.home_changes"), ""), ("undo", t!("commands.home_undo"), "")]
            .iter()
            .filter(|(w, _, _)| starts(w))
            .map(|(w, about, end)| word(w, about, end))
            .collect(),
        ("memory", 1) => [("changes", t!("commands.memory_changes"), "")]
            .iter()
            .filter(|(w, _, _)| starts(w))
            .map(|(w, about, end)| word(w, about, end))
            .collect(),
        ("squad", 1) => {
            let mut choices: Vec<Choice> = ctx.squads.iter().filter(|(n, _)| starts(n)).map(|(n, about)| word(n, about, "")).collect();
            if starts("new") {
                choices.push(word("new", t!("commands.squad_new_about"), " "));
            }
            if starts("add") {
                choices.push(word("add", t!("commands.squad_add_about"), " "));
            }
            if starts("remove") {
                choices.push(word("remove", t!("commands.squad_remove_about"), " "));
            }
            choices
        }
        ("squad", 2) if words[0] == "add" => folders(typed).into_iter().map(|f| word(&f, "", "")).collect(),
        ("squad", 2) if words[0] == "remove" => ctx.projects.iter().filter(|(p, _)| starts(p)).map(|(p, _)| word(p, "", "")).collect(),
        // A picker: any part of the name finds a project.
        ("project", 1) => {
            let wanted = typed.to_lowercase();
            let mut choices: Vec<Choice> = ctx.projects.iter().filter(|(p, _)| p.to_lowercase().contains(&wanted)).map(|(p, about)| word(p, about, "")).collect();
            if "all".contains(&wanted) {
                choices.push(word("all", t!("commands.every_project"), ""));
            }
            choices
        }
        ("squad", 3) if words[0] == "new" => folders(typed).into_iter().map(|f| word(&f, "", "")).collect(),
        ("browser", 1) => [
            ("watch", t!("commands.browser_watch")),
            ("show", t!("commands.browser_show")),
            ("hide", t!("commands.browser_hide")),
            ("signin", t!("commands.browser_signin")),
            ("max", t!("commands.browser_max")),
        ]
            .iter()
            .filter(|(w, _)| starts(w))
            .map(|(w, about)| word(w, about, ""))
            .collect(),
        ("desktop", 1) => [("stop", t!("commands.desktop_stop")), ("resume", t!("commands.desktop_resume"))]
            .iter()
            .filter(|(w, _)| starts(w))
            .map(|(w, about)| word(w, about, ""))
            .collect(),
        ("lang", 1) => {
            let system = t!("lang.system", name = crate::lang::name(equipe_core::i18n::Lang::resolve_auto(equipe_core::i18n::Front::Terminal)));
            let mut choices: Vec<Choice> = if starts("auto") { vec![word("auto", &system, "")] } else { Vec::new() };
            choices.extend(
                equipe_core::i18n::Lang::ALL
                    .iter()
                    .filter(|l| starts(&l.code().to_lowercase()))
                    .map(|l| word(l.code(), l.native_name(), "")),
            );
            choices
        }
        ("suggest", 1) => [("on", t!("commands.suggest_on")), ("off", t!("commands.suggest_off"))]
            .iter()
            .filter(|(w, _)| starts(w))
            .map(|(w, about)| word(w, about, ""))
            .collect(),
        ("browser", 2) if matches!(words[0], "show" | "hide" | "watch") => {
            ctx.members.iter().filter(|(m, _)| starts(m)).map(|(m, _)| word(m, "", "")).collect()
        }
        _ => Vec::new(),
    }
}

/// What a mode lets members do — with "now" on the one `who` is on.
fn mode_about(word: &str, now: Option<Mode>, who: &str) -> String {
    let Some(mode) = Mode::parse(word) else {
        return String::new();
    };
    let about = label::mode_short(crate::i18n::lang(), mode);
    if now == Some(mode) { t!("commands.mode_now_for", who = who, about = about) } else { about.to_owned() }
}

/// Folders that complete a typed path — `~/Pro` → `~/Projects/` — not the
/// hidden ones unless asked for.
fn folders(typed: &str) -> Vec<String> {
    let home = dirs::home_dir().unwrap_or_default();
    let (dir, prefix) = match typed.rfind('/') {
        Some(at) => (&typed[..=at], &typed[at + 1..]),
        None => ("", typed),
    };
    let on_disk = match dir.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if dir.is_empty() => std::env::current_dir().unwrap_or_default(),
        None => std::path::PathBuf::from(dir),
    };
    let Ok(entries) = std::fs::read_dir(&on_disk) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(prefix) && (prefix.starts_with('.') || !name.starts_with('.')))
        .collect();
    // Sorted by name, before the `/`: `equipe` comes ahead of `equipe-site`
    // (`-` sorts before `/`), so the folder typed whole is the first choice.
    found.sort();
    found.truncate(30);
    found.into_iter().map(|name| format!("{dir}{name}/")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_mode_is_named_by_the_word_that_picks_it() {
        for mode in Mode::ALL {
            assert_eq!(Mode::parse(mode_word(mode)), Some(mode));
        }
    }

    fn ctx() -> Context<'static> {
        Context {
            review: vec![(8, "Translations"), (10, "PostHog")],
            reopenable: vec![(8, "Translations"), (10, "PostHog"), (3, "Deploy")],
            waiting: vec![(11, "Analytics")],
            open: vec![(5, "SEO check")],
            unmerged: vec![(3, "Deploy")],
            members: vec![("pm", "pm"), ("frontend", "frontend")],
            roles: vec!["pm", "designer", "frontend", "backend", "qa", "data"],
            squads: vec![("equipe-squad".into(), "equipe-site".into()), ("ios".into(), "app".into())],
            mode: Some(Mode::Auto),
            projects: vec![("site".into(), String::new()), ("app".into(), String::new())],
            member_modes: vec![("frontend", Mode::Full)],
            member_models: Vec::new(),
            selected_models: Vec::new(),
        }
    }

    fn texts(choices: &[Choice]) -> Vec<&str> {
        choices.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn a_slash_lists_the_commands_that_fit() {
        assert_eq!(texts(&complete("/reo", &ctx())), ["/reopen "]);
        assert_eq!(texts(&complete("/resume ", &ctx())), ["/resume T-11 "], "the tickets waiting on the person");
        assert_eq!(complete("/", &ctx()).len(), commands().len());
        assert_eq!(texts(&complete("/bo", &ctx())), ["/board"], "a command without arguments is whole");
        assert!(complete("/board", &ctx()).is_empty(), "nothing left to complete");
        assert!(complete("hello", &ctx()).is_empty());
    }

    #[test]
    fn arguments_come_from_the_board_and_the_squad() {
        assert_eq!(texts(&complete("/accept ", &ctx())), ["/accept T-8", "/accept T-10"]);
        assert_eq!(texts(&complete("/accept T-1", &ctx())), ["/accept T-10"]);
        assert_eq!(texts(&complete("/accept post", &ctx())), ["/accept T-10"], "a title finds its ticket");
        assert_eq!(texts(&complete("/reopen 3", &ctx())), ["/reopen T-3 "], "what to change comes next");
        assert_eq!(texts(&complete("/merge ", &ctx())), ["/merge T-3"], "only accepted work not on main yet");
        assert_eq!(texts(&complete("/mode f", &ctx())), ["/mode full", "/mode frontend "]);
        assert_eq!(texts(&complete("/mode frontend a", &ctx())), ["/mode frontend ask", "/mode frontend auto"]);
        let modes = complete("/mode ", &ctx());
        assert!(modes[2].about.starts_with("now, for the squad · works alone, sandboxed"), "{:?}", modes[2]);
        assert!(modes.iter().any(|c| c.label == "frontend" && c.about.contains("full access of its own")), "{modes:?}");
        assert!(complete("/mode pm s", &ctx()).iter().any(|c| c.about.contains("back on the squad's mode (auto)")));
        assert_eq!(texts(&complete("/add designer q", &ctx())), ["/add designer qa "]);
        assert_eq!(texts(&complete("/browser show f", &ctx())), ["/browser show frontend"]);
        assert_eq!(texts(&complete("@fr", &ctx())), ["@frontend "]);
        assert_eq!(
            texts(&complete("/squad ", &ctx())),
            ["/squad equipe-squad", "/squad ios", "/squad new ", "/squad add ", "/squad remove "]
        );
        assert_eq!(texts(&complete("/browser m", &ctx())), ["/browser max"]);
        assert_eq!(texts(&complete("/project ", &ctx())), ["/project site", "/project app", "/project all"]);
        assert_eq!(texts(&complete("/squad remove a", &ctx())), ["/squad remove app"]);
        let tmp = std::env::temp_dir().join(format!("equipe-folders-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("web")).unwrap();
        std::fs::create_dir_all(tmp.join("wallet")).unwrap();
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        let typed = format!("/squad new shop {}/w", tmp.display());
        assert_eq!(complete(&typed, &ctx()).iter().map(|c| c.label.rsplit('/').nth(1).unwrap()).collect::<Vec<_>>(), ["wallet", "web"]);
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn a_folder_typed_whole_comes_before_its_longer_neighbours() {
        let tmp = std::env::temp_dir().join(format!("equipe-siblings-{}", std::process::id()));
        for dir in ["equipe", "equipe-labs-profile", "equipe-site"] {
            std::fs::create_dir_all(tmp.join(dir)).unwrap();
        }
        let typed = format!("/squad add {}/equipe", tmp.display());
        let labels: Vec<String> = complete(&typed, &ctx()).into_iter().map(|c| c.label).collect();
        let names: Vec<&str> = labels.iter().map(|l| l.rsplit('/').nth(1).unwrap()).collect();
        assert_eq!(names, ["equipe", "equipe-labs-profile", "equipe-site"]);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
