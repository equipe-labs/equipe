//! The commands in one place and what completion offers as the person types
//! `/…` or `@…`. The list and the rules are the terminal console's
//! (`crates/cli/src/tui/commands.rs`), kept the same on purpose; if one changes,
//! change the other.

use equipe_core::i18n::label;
use equipe_core::protocol::{Mode, TicketId};
use equipe_core::t;

pub struct Spec {
    pub name: &'static str,
    pub args: &'static str,
    pub about: &'static str,
}

pub const COMMANDS: [Spec; 31] = [
    Spec { name: "usage", args: "", about: "commands.about_usage" },
    Spec { name: "scale", args: "[0-6|off]", about: "commands.about_scale" },
    Spec { name: "compact", args: "[member]", about: "commands.about_compact" },
    Spec { name: "model", args: "[member] <model>", about: "commands.about_model" },
    Spec { name: "clear", args: "[member]", about: "commands.about_clear" },
    Spec { name: "squad", args: "[name] · new <name> [folder] · add|remove <folder>", about: "commands.about_squad" },
    Spec { name: "home", args: "[changes · undo [n]]", about: "commands.about_home" },
    Spec { name: "memory", args: "[changes]", about: "commands.about_memory" },
    Spec { name: "project", args: "[name|all]", about: "commands.about_project" },
    Spec { name: "accept", args: "T-n", about: "commands.about_accept" },
    Spec { name: "merge", args: "T-n", about: "commands.about_merge" },
    Spec { name: "reopen", args: "T-n <what to change>", about: "commands.about_reopen" },
    Spec { name: "resume", args: "T-n [what you did]", about: "commands.about_resume" },
    Spec { name: "cancel", args: "T-n", about: "commands.about_cancel" },
    Spec { name: "board", args: "", about: "commands.about_board" },
    Spec { name: "design", args: "", about: "commands.about_design" },
    Spec { name: "find", args: "<words>", about: "commands.about_find" },
    Spec { name: "tidy", args: "", about: "commands.about_tidy" },
    Spec { name: "disk", args: "", about: "commands.about_disk" },
    Spec { name: "add", args: "<role…> | custom <name> [--edits]", about: "commands.about_add" },
    Spec { name: "close", args: "", about: "commands.about_close" },
    Spec { name: "mode", args: "[member] read-only|ask|auto|full", about: "commands.about_mode" },
    Spec { name: "browser", args: "[show|hide [member] · max <n>]", about: "commands.about_browser" },
    Spec { name: "send-apps", args: "[add|remove <app> · reset]", about: "commands.about_send_apps" },
    Spec { name: "desktop", args: "stop|resume", about: "commands.about_desktop" },
    Spec { name: "engine", args: "[member|all] <claude|codex>", about: "commands.about_engine" },
    Spec { name: "engines", args: "[auto|suggest|off|setup]", about: "commands.about_engines" },
    Spec { name: "export", args: "", about: "commands.about_export" },
    Spec { name: "mouse", args: "", about: "commands.about_mouse" },
    Spec { name: "help", args: "", about: "commands.about_help" },
    Spec { name: "quit", args: "", about: "commands.about_quit" },
];

/// One completion: what the composer holds once it's taken, and how it is
/// listed. A `text` without a trailing space is whole: Enter on it runs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    pub text: String,
    pub label: String,
    pub about: String,
    /// The characters of `label` the typed letters matched, to highlight.
    pub hits: Vec<usize>,
    /// Where it comes from, as a small badge: `equipe`, `터미널`, a project.
    pub badge: Option<String>,
    /// The heading it is listed under (`멤버`, `파일`, `티켓`).
    pub group: Option<&'static str>,
    /// The part of `text` that becomes one atomic token in the box once taken
    /// (a command's name), as a byte range.
    pub token: Option<std::ops::Range<usize>>,
}

impl Choice {
    pub fn new(text: String, label: String, about: String) -> Self {
        Self { text, label, about, ..Self::default() }
    }
}

/// Whether `query`'s letters appear in `candidate` in order (any case): a
/// score — higher is better: a prefix, letters together, early — and which
/// characters of `candidate` matched.
pub fn fuzzy(query: &str, candidate: &str) -> Option<(i32, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let wanted: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let have: Vec<char> = candidate.chars().flat_map(char::to_lowercase).collect();
    let mut hits = Vec::with_capacity(wanted.len());
    let mut at = 0;
    for w in &wanted {
        let found = (at..have.len()).find(|i| have[*i] == *w)?;
        hits.push(found);
        at = found + 1;
    }
    let mut score = 100 - hits[0] as i32 * 4 - have.len() as i32 / 4;
    if hits[0] == 0 {
        score += 60;
    }
    for pair in hits.windows(2) {
        score += if pair[1] == pair[0] + 1 { 12 } else { -3 };
    }
    // Letters after a separator (`/`, `-`, `_`, `.`) count as the start of a word.
    for &h in &hits {
        if h > 0 && matches!(have[h - 1], '/' | '-' | '_' | '.' | ' ') {
            score += 8;
        }
    }
    Some((score, hits))
}

/// Commands that only the terminal console runs.
const TERMINAL_ONLY: [&str; 9] = ["usage", "squad", "home", "memory", "add", "send-apps", "export", "mouse", "quit"];

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
    /// Whether the folders a person types are on this disk — the daemon's
    /// machine is this one. Over SSH they are on another, which can't be listed here.
    pub local_folders: bool,
}

/// The modes as typed, in `Mode::ALL`'s order.
pub const MODES: [&str; 4] = ["read-only", "ask", "auto", "full"];

/// What could complete `text`, best first. Empty when nothing would, or when
/// the text is already whole.
pub fn complete(text: &str, ctx: &Context<'_>) -> Vec<Choice> {
    let choices = if let Some(name) = text.strip_prefix('@') {
        if name.contains(char::is_whitespace) {
            return Vec::new();
        }
        let mut found: Vec<(i32, Choice)> = ctx
            .members
            .iter()
            .filter_map(|(m, role)| {
                let (score, hits) = fuzzy(name, m)?;
                Some((score, Choice { hits: hits.into_iter().map(|h| h + 1).collect(), group: Some(t!("commands.group_members")), ..Choice::new(format!("@{m} "), format!("@{m}"), (*role).to_owned()) }))
            })
            .collect();
        found.sort_by_key(|(score, _)| -score);
        found.into_iter().map(|(_, c)| c).collect()
    } else if let Some(command) = text.strip_prefix('/') {
        match command.split_once(' ') {
            None => {
                let mut found: Vec<(i32, Choice)> = COMMANDS
                    .iter()
                    .filter_map(|c| {
                        let (score, hits) = fuzzy(command, c.name)?;
                        let here = !TERMINAL_ONLY.contains(&c.name);
                        let text = if c.args.is_empty() { format!("/{}", c.name) } else { format!("/{} ", c.name) };
                        let choice = Choice {
                            hits: hits.into_iter().map(|h| h + 1).collect(),
                            badge: Some(if here { "equipe".into() } else { t!("commands.badge_terminal").into() }),
                            token: (!c.args.is_empty()).then(|| 0..c.name.len() + 1),
                            ..Choice::new(text, format!("/{} {}", c.name, c.args).trim_end().to_owned(), crate::i18n::catalog().get(c.about).to_owned())
                        };
                        // The ones that work here first, then the best match.
                        Some((score + if here { 1000 } else { 0 }, choice))
                    })
                    .collect();
                // With nothing typed yet, the list keeps its own order.
                if !command.is_empty() {
                    found.sort_by_key(|(score, _)| -score);
                } else {
                    found.sort_by_key(|(score, _)| -(score / 1000));
                }
                found.into_iter().map(|(_, c)| c).collect()
            }
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
    let word = |w: &str, about: &str, then: &str| Choice::new(format!("{lead}{w}{then}"), w.to_owned(), about.to_owned());
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
                choices.push(word("squad", &t!("commands.mode_back_on_squad", squad = squad), ""));
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
                choices.push(word("new", t!("commands.squad_new"), " "));
            }
            if starts("add") {
                choices.push(word("add", t!("commands.squad_add"), " "));
            }
            if starts("remove") {
                choices.push(word("remove", t!("commands.squad_remove"), " "));
            }
            choices
        }
        ("squad", 2) if words[0] == "add" && ctx.local_folders => folders(typed).into_iter().map(|f| word(&f, "", "")).collect(),
        ("squad", 2) if words[0] == "remove" => ctx.projects.iter().filter(|(p, _)| starts(p)).map(|(p, _)| word(p, "", "")).collect(),
        // A picker: any part of the name finds a project.
        ("project", 1) => {
            let wanted = typed.to_lowercase();
            let mut choices: Vec<Choice> = ctx.projects.iter().filter(|(p, _)| p.to_lowercase().contains(&wanted)).map(|(p, about)| word(p, about, "")).collect();
            if "all".contains(&wanted) {
                choices.push(word("all", t!("commands.project_all"), ""));
            }
            choices
        }
        ("squad", 3) if words[0] == "new" && ctx.local_folders => folders(typed).into_iter().map(|f| word(&f, "", "")).collect(),
        ("browser", 1) => [("show", t!("commands.browser_show")), ("hide", t!("commands.browser_hide")), ("max", t!("commands.browser_max"))]
            .iter()
            .filter(|(w, _)| starts(w))
            .map(|(w, about)| word(w, about, ""))
            .collect(),
        ("engines", 1) => [
            ("suggest", t!("engines.mode_suggest_about")),
            ("auto", t!("engines.mode_auto_about")),
            ("off", t!("engines.mode_off_about")),
            ("setup", t!("engines.settings_setup")),
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
        ("suggest", 1) => [("on", "guess your next message after an answer"), ("off", "no guesses")]
            .iter()
            .filter(|(w, _)| starts(w))
            .map(|(w, about)| word(w, about, ""))
            .collect(),
        ("browser", 2) if matches!(words[0], "show" | "hide") => {
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
    let on = if now == Some(mode) { t!("commands.mode_now_for", who = who) } else { String::new() };
    format!("{on}{}", label::mode_short(crate::i18n::lang(), mode))
}

/// Folders that complete a typed path — `~/Pro` → `~/Projects/` — not the
/// hidden ones unless asked for.
fn folders(typed: &str) -> Vec<String> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
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
            local_folders: true,
        }
    }

    fn texts(choices: &[Choice]) -> Vec<&str> {
        choices.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn a_slash_lists_the_commands_that_fit() {
        assert_eq!(texts(&complete("/reo", &ctx())), ["/reopen "]);
        assert_eq!(texts(&complete("/resume ", &ctx())), ["/resume T-11 "], "the tickets waiting on the person");
        assert_eq!(complete("/", &ctx()).len(), COMMANDS.len());
        assert_eq!(texts(&complete("/bo", &ctx())), ["/board", "/browser "], "a command without arguments is whole; the prefix first");
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
        assert!(modes[2].about.starts_with("스쿼드의 현재 모드 · 샌드박스 안에서 혼자 일함"), "{:?}", modes[2]);
        assert!(modes.iter().any(|c| c.label == "frontend" && c.about.contains("자체 모드 전체 권한")), "{modes:?}");
        assert!(complete("/mode pm s", &ctx()).iter().any(|c| c.about.contains("스쿼드 모드로 되돌리기 (자동)")));
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
    fn letters_in_order_find_a_command_and_say_which_matched() {
        assert_eq!(fuzzy("rpn", "reopen").map(|(_, hits)| hits), Some(vec![0, 3, 5]));
        assert!(fuzzy("xyz", "reopen").is_none());
        let (prefix, _) = fuzzy("re", "reopen").unwrap();
        let (inside, _) = fuzzy("re", "merge").unwrap();
        assert!(prefix > inside, "a prefix ranks first");
        let rows = complete("/rpn", &ctx());
        assert_eq!(texts(&rows), ["/reopen "]);
        assert_eq!(rows[0].hits, vec![1, 4, 6], "positions in the label, after its slash");
        assert_eq!(rows[0].token, Some(0..7), "`/reopen` becomes one token in the box");
        assert_eq!(rows[0].badge.as_deref(), Some("equipe"));
        let usage = complete("/usage", &ctx());
        assert!(usage.is_empty() || usage[0].badge.as_deref() != Some("equipe"));
        // The ones that work here come before the terminal's.
        let all = complete("/", &ctx());
        let first_terminal = all.iter().position(|c| c.badge.as_deref() != Some("equipe")).unwrap();
        assert!(all[first_terminal..].iter().all(|c| c.badge.as_deref() != Some("equipe")));
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
