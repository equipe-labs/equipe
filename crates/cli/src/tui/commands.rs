//! The console's commands in one place: what `/help` lists and what completion
//! offers as the person types `/…` or `@…`.

use equipe_core::protocol::TicketId;

pub struct Spec {
    pub name: &'static str,
    pub args: &'static str,
    pub about: &'static str,
}

pub const COMMANDS: [Spec; 12] = [
    Spec { name: "accept", args: "T-n", about: "Approve handed-in work" },
    Spec { name: "reopen", args: "T-n <what to change>", about: "Send work back with what to change" },
    Spec { name: "cancel", args: "T-n", about: "Drop a ticket" },
    Spec { name: "board", args: "", about: "The board (Ctrl-B)" },
    Spec { name: "add", args: "<role…> [codex|claude]", about: "Bring in teammates" },
    Spec { name: "close", args: "", about: "Remove the selected member" },
    Spec { name: "mode", args: "[member] plan|ask|auto|full", about: "How much members do on their own" },
    Spec { name: "browser", args: "[show|hide [member]]", about: "Sign in to sites; show or hide members' browsers" },
    Spec { name: "export", args: "", about: "Open the conversation in your editor" },
    Spec { name: "mouse", args: "", about: "Let the terminal select text" },
    Spec { name: "help", args: "", about: "Keys and commands" },
    Spec { name: "quit", args: "", about: "Leave — the squad keeps working" },
];

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
    /// Tickets still open, for /cancel.
    pub open: Vec<(TicketId, &'a str)>,
    /// The squad's members: name and role.
    pub members: Vec<(&'a str, &'a str)>,
    pub roles: Vec<&'a str>,
}

const MODES: [&str; 4] = ["plan", "ask", "auto", "full"];

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
            None => COMMANDS
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
        // What to change follows the number.
        ("reopen", 1) => tickets(&ctx.reopenable, " "),
        ("mode", 1) => {
            let mut choices: Vec<Choice> = MODES.iter().filter(|w| starts(w)).map(|m| word(m, "for the whole squad", "")).collect();
            choices.extend(ctx.members.iter().filter(|(m, _)| starts(m)).map(|(m, _)| word(m, "just this member", " ")));
            choices
        }
        ("mode", 2) if ctx.members.iter().any(|(m, _)| *m == words[0]) => {
            MODES.iter().chain(["squad"].iter()).filter(|w| starts(w)).map(|m| word(m, "", "")).collect()
        }
        ("add", _) => ctx.roles.iter().filter(|w| starts(w)).map(|r| word(r, "", " ")).collect(),
        ("browser", 1) => [("show", "browsers on this Mac's screen"), ("hide", "browsers hidden again")]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context<'static> {
        Context {
            review: vec![(8, "Translations"), (10, "PostHog")],
            reopenable: vec![(8, "Translations"), (10, "PostHog"), (3, "Deploy")],
            open: vec![(5, "SEO check")],
            members: vec![("pm", "pm"), ("frontend", "frontend")],
            roles: vec!["pm", "designer", "frontend", "backend", "qa", "data"],
        }
    }

    fn texts(choices: &[Choice]) -> Vec<&str> {
        choices.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn a_slash_lists_the_commands_that_fit() {
        assert_eq!(texts(&complete("/re", &ctx())), ["/reopen "]);
        assert_eq!(complete("/", &ctx()).len(), COMMANDS.len());
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
        assert_eq!(texts(&complete("/mode f", &ctx())), ["/mode full", "/mode frontend "]);
        assert_eq!(texts(&complete("/mode frontend a", &ctx())), ["/mode frontend ask", "/mode frontend auto"]);
        assert_eq!(texts(&complete("/add designer q", &ctx())), ["/add designer qa "]);
        assert_eq!(texts(&complete("/browser show f", &ctx())), ["/browser show frontend"]);
        assert_eq!(texts(&complete("@fr", &ctx())), ["@frontend "]);
    }
}
