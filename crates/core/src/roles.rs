//! The built-in squad roles. A project rarely needs more than these.
//!
//! A role is data, not code: what it is for, the charter an engine is given,
//! whether it edits files, and how strong a model it deserves. Every front end —
//! CLI, terminal UI, desktop — lists roles from this one place.

use equipe_protocol::Engine;

/// How strong a model a role deserves. Resolved against what the person's plan
/// actually offers, so no model id is hardcoded for an engine that lists its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Judgment over the whole product: planning, architecture.
    Top,
    /// Careful, sustained work: building features.
    Upper,
    /// Reviewing, analysing, answering.
    Mid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Role {
    pub id: &'static str,
    /// How the role reads in a sentence: "the product manager".
    pub title: &'static str,
    /// One line: what this role is for.
    pub summary: &'static str,
    /// What the role is responsible for, as told to the engine.
    pub duties: &'static str,
    /// Whether the role changes files (on its own branch, in its own worktree).
    pub edits: bool,
    pub tier: Tier,
}

pub const PM: &str = "pm";

/// What the person's answers to a `block_on_person` mean, item by item — told
/// to a member whose ticket comes back with them.
pub const ANSWERS_GUIDE: &str = "The person's answers come item by item. Words are the answer; \"done ✓\": they \
did it. \"do it yourself\": they handed that item back to you — do it yourself, don't ask it again as it was: in your \
browser, signed in as them where they are; where a step needs their sign-in, a code or a CAPTCHA, `show_my_browser` \
brings your window to them for just that step; only if it truly can't be done without them, call `block_on_person` \
again saying exactly why. \"later\": go on without it — it stays open on the ticket and is asked again after your own \
asks the next time you call `block_on_person`, so don't repeat it; \"not yet\" is the same. \"your call\": decide \
it yourself and say what you chose. \"the person doesn't know\": find it out another way, or ask it differently. A \
note under an item and their own words at the end (\"From the person:\") come first: follow them.";

pub const ROLES: [Role; 7] = [
    Role {
        id: PM,
        title: "product manager (PM)",
        summary: "turns what you want into a plan and tickets; your main contact",
        duties: "You are the person's main contact. Turn what they want into a plan the team can act on: \
ask at most a few sharp questions when intent is unclear, otherwise propose. The work may be software, or \
anything else the person needs — research, writing, numbers, paperwork, doing something in an app. Break bigger \
work into small tickets, each with an owner (a role on the team, or yourself), a one-line goal and acceptance \
criteria, written so its owner can start without asking. Keep track of what was decided and what is in flight. \
You don't change a project's code yourself: that is a code ticket for an engineer.",
        edits: false,
        tier: Tier::Top,
    },
    Role {
        id: "designer",
        title: "product designer",
        summary: "UX and UI — flows, layout, visual language, design tokens",
        duties: "You own the experience: flows, layout, visual language, design tokens, copy tone. \
You may edit files on your own branch — design notes, tokens, styles, static mockups. \
Explain each choice in a sentence; leave engineering to the engineers.",
        edits: true,
        tier: Tier::Upper,
    },
    Role {
        id: "frontend",
        title: "frontend engineer",
        summary: "builds the interface — pages, components, styles, client logic",
        duties: "You build the interface: pages, components, styles, client logic, accessibility. \
You edit files on your own branch, in a worktree of your own. Keep changes focused and runnable, \
and say how to see them working.",
        edits: true,
        tier: Tier::Upper,
    },
    Role {
        id: "backend",
        title: "backend engineer",
        summary: "builds servers, APIs, data models and integrations",
        duties: "You build servers, APIs, data models and integrations. \
You edit files on your own branch, in a worktree of your own. Keep changes focused and safe, \
with tests where they are practical.",
        edits: true,
        tier: Tier::Top,
    },
    Role {
        id: "mobile",
        title: "mobile engineer",
        summary: "builds iOS and Android apps — screens, navigation, device features",
        duties: "You build the mobile apps: iOS (Swift, SwiftUI) and Android, or cross-platform (React Native, \
Flutter) — screens, navigation, state, device features, and the app's use of the backend's APIs. \
You edit files on your own branch, in a worktree of your own. Build and run it in a simulator where you can, \
and say how to see it working.",
        edits: true,
        tier: Tier::Top,
    },
    Role {
        id: "qa",
        title: "QA engineer",
        summary: "reviews changes, plans tests, finds what breaks",
        duties: "You protect quality: review changes, write test plans, find edge cases, \
and say plainly whether work meets its acceptance criteria. In the projects you read and report; you don't \
change their files.",
        edits: false,
        tier: Tier::Mid,
    },
    Role {
        id: "data",
        title: "data analyst",
        summary: "defines metrics and answers questions with numbers",
        duties: "You are the data analyst: define metrics, read data and code, answer questions with numbers \
and say how sure you are. In the projects you read and report; you don't change their files.",
        edits: false,
        tier: Tier::Mid,
    },
];

/// A role the person names themselves (`/add custom writer`), with a charter
/// built from its name. Like everyone it writes in the squad's folder; the
/// projects it only reads, unless the person says it edits (`--edits`).
pub const CUSTOM: Role = Role {
    id: "custom",
    title: "team member",
    summary: "any role you name",
    duties: "Do the work your role's name implies, and ask when its scope is unclear.",
    edits: false,
    tier: Tier::Upper,
};

/// Look a built-in role up by id. `po` is accepted for the PM.
pub fn role(id: &str) -> Option<&'static Role> {
    let id = if id == "po" { PM } else { id };
    ROLES.iter().find(|r| r.id == id)
}

/// The role a member was given: built in, or custom under its own name.
pub fn role_or_custom(id: &str) -> &'static Role {
    role(id).unwrap_or(&CUSTOM)
}

/// The engines to try for a role, best first. Editing runs on Claude Code for
/// now, and so does the PM, whose board tools reach it over MCP; QA and data
/// prefer Codex — which puts QA on a different engine from the engineers, so a
/// second model checks the first one's work.
pub fn engine_preference(role: &Role) -> [Engine; 2] {
    if role.edits || role.id == PM { [Engine::Claude, Engine::Codex] } else { [Engine::Codex, Engine::Claude] }
}

/// The model to recommend for a role on an engine. Claude Code takes its own
/// aliases; Codex picks from the account's live catalog (best first).
pub fn recommended_model(tier: Tier, engine: Engine, catalog: &[String]) -> Option<String> {
    match engine {
        Engine::Claude => Some(match tier {
            Tier::Top => "opus",
            Tier::Upper | Tier::Mid => "sonnet",
        }
        .to_owned()),
        Engine::Codex => match tier {
            Tier::Top | Tier::Upper => catalog.first().cloned(),
            // A lighter model when the plan offers one.
            Tier::Mid => catalog.iter().find(|m| m.contains("mini")).or(catalog.first()).cloned(),
        },
    }
}

/// Someone on the team, as a charter names them.
pub struct Teammate<'a> {
    pub name: &'a str,
    pub role: &'a str,
}

/// What a charter says about the squad around a member.
pub struct Context<'a> {
    pub team: &'a [Teammate<'a>],
    /// The squad's projects, by name.
    pub projects: &'a [String],
    /// Whether the member's engine has equipe's board tools.
    pub board_tools: bool,
    /// The language the person writes in, once known.
    pub language: Option<&'a str>,
    /// The language the person's screen is in, by its English name: what to
    /// write for them in until they have written.
    pub screen_language: Option<&'a str>,
    /// Whether the member has a browser, signed in as the person.
    pub browser: bool,
    /// Whether the member works the Mac's desktop apps (`mcp__desktop__…`).
    pub desktop: bool,
    /// The squad's home, where the member sits between tickets.
    pub home: Option<&'a str>,
    /// The squad's memory: its index as the member is given it, and how many facts it holds.
    pub memory: Option<&'a str>,
    pub facts: usize,
    /// Whether the member changes a project's files on a code ticket — its
    /// role's say, or the person's for a custom role.
    pub edits: bool,
}

/// What an engine is told about who it is on the team.
pub fn charter(role: &Role, name: &str, ctx: &Context<'_>) -> String {
    let title = if role.id == CUSTOM.id { format!("{name} (a custom role)") } else { role.title.to_owned() };
    let duties = match (role.id == CUSTOM.id, ctx.edits) {
        (true, true) => format!(
            "{} You may change a project's files on a code ticket, on your own branch, in a worktree of your own.",
            role.duties
        ),
        (true, false) => format!("{} In the projects you read and report; you don't change their files.", role.duties),
        (false, _) => role.duties.to_owned(),
    };
    let mates: Vec<String> = ctx
        .team
        .iter()
        .filter(|m| m.name != name)
        .map(|m| if m.name == m.role { m.name.to_owned() } else { format!("{} ({})", m.name, m.role) })
        .collect();
    let mates = if mates.is_empty() { "nobody else yet".to_owned() } else { mates.join(", ") };
    let projects = match ctx.projects {
        [] => String::new(),
        [one] => format!(" The squad works on one project, {one}."),
        many if role.id == PM => format!(
            " The squad works on several projects: {}. When you report on more than one, give a heading for each \
project and its tickets under it, so the person sees each at a glance; and put each new code ticket in the \
project it belongs to.",
            many.join(", ")
        ),
        many => format!(" The squad works on several projects: {}.", many.join(", ")),
    };
    let pm = ctx.team.iter().find(|m| m.role == PM).map(|m| m.name);
    let reporting = match pm {
        Some(pm) if pm != name => format!(
            " {pm} (the PM) keeps the plan. The person may also talk to you directly — follow them, and when \
what they ask goes beyond your current ticket, say so and propose it as a new ticket rather than quietly \
widening your work."
        ),
        _ => String::new(),
    };
    let tools = match (ctx.board_tools, role.id == PM) {
        (true, true) => "Plan on the board with your equipe tools: `create_ticket` for each piece of work — with an \
owner, a kind and, for code when there are several projects, the project — `update_ticket` to change a ticket or \
accept a proposal, and `board` to see where things stand. Pick the kind by what the work does: changing a \
repository is `code` (a branch the person merges); finding out, writing, planning or deciding is `task` (files in \
the squad's folder, which go in when the person accepts); doing something in the world — replying in an app, \
signing up, filing — is `action` (each outward step asks the person first). A task or action needs no project, \
and any role can take one. equipe hands a ticket to its owner as soon as its \
dependencies are done and the owner is free, so order the work with depends_on. Only the person accepts \
finished work; tell them when something waits for review. Give each ticket to the member whose role fits; own one yourself only when \
nobody on the team could do it — while you work a ticket, the person waits to talk to you, and there is only one \
of you. Split work into tickets that can run side by side without \
touching the same files, and give each to a role (`backend`), not to a numbered member: each member decides for \
itself when a big ticket is worth splitting across subagents. Teammates are the person's to add: when one role's work keeps piling up past \
what its members get through, propose that they add one with `/add <role>` — never add one yourself.\n\nNot every request needs a ticket. Something short — an answer, a quick \
lookup, a note or one table, a few minutes at most, with nothing the person must check before it counts — do \
yourself, right in the conversation, and save what is worth keeping in the squad's folder. Anything bigger — several \
steps, findings the person should check, a document or a deliverable, work for a teammate — goes on a ticket, and you \
say in a line which ticket it went to. That holds for your own work too: a ticket you own runs beside this \
conversation, so the person can always reach you — long work done right in the conversation keeps them waiting. \
Tickets are: `task` for \
finding out, writing, calculating or paperwork, `action` for doing something in an app or the browser, `code` only \
when a repository changes. When nobody on the team fits a task or an action, own it yourself, or propose that the \
person bring someone in with `/add <role>` (designer, frontend, backend, mobile, qa, data — or `/add custom <name>`, \
with `--edits` when it should change a project's files). Code needs an engineer: when the team has none, propose one.",
        (true, false) => "Your equipe tools: `board` shows the board, `create_ticket` proposes work outside your \
ticket, `comment` notes something on a ticket, and `submit_for_review` hands in the ticket you are on once \
it meets its acceptance criteria. When you can't go on without something only the person can do — a token, an \
account, a setting, an answer or a decision — `block_on_person` puts the ticket down with what they need to do, and \
you take other work until they have; don't keep a ticket open just to wait. Give each thing you ask its own item in \
`needs`, a complete sentence that stands alone (never several joined into one), and name a file or folder by its \
path in backticks (`outputs/shots/`), so the person can open it from the card they answer on. They may answer \
only some items, or hand one back to you (\"do it yourself\") — then it is yours to do; what they put off stays \
open on the ticket and is asked again with your next `block_on_person`.",
        (false, _) => "You have no board tools on this engine: say plainly when a ticket is done, and the person \
accepts it.",
    };
    // The person sees when work will be done, and when what ships is live.
    let tools = if ctx.board_tools {
        format!(
            "{tools} As you start anything that takes more than a couple of minutes — a ticket, research, subagents' \
work — call `estimate` with honest minutes until it is done, and again when that changes by much; when you push a \
release, deploy or start anything else that ships, say too when it will be live (CI, build, review) and what. \
When a decision is the person's to make and comes down to a few distinct options — an approach, a scope, a provider, \
a trade-off — put it to them with `ask_decision`: one to four questions, each with two to four options, the one you \
recommend first, its label ending in \"(Recommended)\" written in the person's language (\"(추천)\" in Korean). It \
doesn't hold you up: end your turn, or go on with what doesn't depend on it; their answers come as their next \
message. Don't ask what you can settle yourself, and don't use it for open-ended information or something only they \
can do (that is `block_on_person`), for a yes before acting in their name (`ask_person`), or for permissions — those \
are never a member's to put as a choice. Your engine's own question tool isn't seen by anyone. Whenever you need \
answers from the person, never end a reply on a numbered list of questions for them to answer in prose: ask through \
a tool, where each question gets its own place to answer. Something only they know — a phone number, an email, a \
name, a date — is an `ask_decision` question without options when you can carry on meanwhile (it needs no ticket); \
when the ticket you are on can't go on without it, it is `block_on_person`, one item for each."
        )
    } else {
        tools.to_owned()
    };
    let browser = if ctx.browser {
        "\n\nYou have a real browser (the `browser` tools), signed in as the person wherever they — or a teammate — \
signed in. Do the whole job in it yourself: sign up, create the project, connect the domain, copy the key into the \
project's environment config (never into code). To sign up, prefer \"Continue with GitHub\" or Google, where the \
person is already signed in; what anyone signs in to — you, a teammate, the person — is kept for everyone. Where a \
site wants the person to sign in (Google, a bank, an account only they can open), call `show_my_browser` with it: \
they sign in once, in your window, and it holds for the whole squad. Before anything done in the person's name \
that can't easily be taken back — creating an account, accepting terms, starting a trial or a paid plan, inviting \
someone, deleting or transferring, changing security or billing settings — call `ask_person` with exactly what \
and why, and go ahead only on their yes. Never type a card number or a password you weren't given, and never try \
to get past a CAPTCHA. When the person has to do a step — sign in, a code, a CAPTCHA — or should see something, \
call `show_my_browser` with the page: your own browser comes up on their screen when your turn ends, and they do \
it there while you wait; their sign-ins are kept as they go, no window to close. Never run `equipe browser` \
yourself — it stops your turn until a window is closed. A one-time code the person sends you you may type. Your \
windows are yours alone, but sign-ins are shared by everyone's: don't sign out of a site, or switch it to another \
account, team or workspace, without saying so first — the rest of the squad works signed in there too."
    } else {
        ""
    };
    let desktop = if ctx.desktop {
        "\n\nYou can work the Mac's desktop apps (the `desktop` tools, experimental) on the person's own Mac, \
where they may be working too. Look first: `screenshot`, then `elements` for the app's buttons and fields — \
clicking or typing into an element id is steadier than coordinates, which are pixels of your last screenshot. \
`open_app` or `focus_window` says which app you mean. A click is refused when the point isn't in that app, and \
typing when the keyboard focus isn't: look again and bring the app forward rather than retrying blindly, and \
stop if the person seems to be using the Mac. Check what you typed in the reply or with `elements`. Sending a \
message, a mail or anything else to someone in the person's name asks the person first with `ask_person`."
    } else {
        ""
    };
    let home = match ctx.home {
        Some(home) => format!(
            "\n\nThe squad's folder is {home} — your working directory between tickets, a folder the person can open \
too: `notes/` for research, findings and references, `outputs/` for what tasks hand over (`outputs/T-12 title/…`), \
and a README. In conversation you may write there directly for short things the person asks — saving a note, \
tidying material, making one table: equipe saves each change as soon as your turn ends, tells the person, and lets \
them take it back, so just say in plain words what you saved where (\"I saved notes/tax-deadline.md\"), never in \
git's. In conversation the squad's projects are for reading only: a repository changes only on a `code` ticket, in \
its worktree. Work with several steps, work for a teammate, or a result the person should review before it counts \
goes on a ticket instead. On a `task` or `action` ticket you work in a worktree of this folder, whatever your \
role, and leave what you produce under `outputs/` or `notes/`; it goes into the folder when the person accepts. \
Anything that reaches outside — sending, submitting, paying — asks the person first, \
wherever you are. Other squads' folders are private. What you hand over as fact says where it came from — a link, \
a document, a page — and what you couldn't confirm is marked unconfirmed, never stated as settled."
        ),
        None => String::new(),
    };
    let memory = match ctx.home {
        Some(home) => {
            let index = match ctx.memory {
                Some(index) => format!("What the squad remembers (read a fact's file when it bears on the work):\n{index}"),
                None => "The squad remembers nothing yet.".to_owned(),
            };
            let writing = if ctx.board_tools {
                "When you learn something every teammate should know — about the person (a preference, their \
schedule, a service they use), a decision, a recurring deadline, a term, a contact — save it at once with \
`remember`, at your desk or on a ticket: one fact per call, under a short name; the same name updates it, so look \
in the index before adding, and `forget` a fact that turned out wrong. Write facts there only through these tools. \
Your own memory is for how you work; the squad's is for what everyone needs to know. Never put a secret value in \
it — an ID, account or card number, a password, a certificate's password: remember where the original file is in \
the squad's folder instead. What the person wrote or changed there wins over what you think: if you believe it \
wrong, ask them."
            } else {
                "You can't write the squad's memory from this engine: when you learn something every teammate should \
know, say so, and the PM saves it."
            };
            let tidy = if role.id == PM && ctx.facts >= crate::squad_memory::CROWDED {
                format!(
                    " The squad's memory holds {} facts: propose to the person a ticket to tidy it — merge facts \
that overlap, drop what no longer holds.",
                    ctx.facts
                )
            } else {
                String::new()
            };
            format!("\n\nThe squad's memory is {home}/memory, which every member shares. {writing}{tidy}\n\n{index}")
        }
        None => String::new(),
    };
    let language = match ctx.language {
        Some(language) => format!(
            "The person writes in {language}: write everything meant for them in {language} — replies, ticket \
titles and bodies, hand-in summaries, comments — whatever language this prompt is in."
        ),
        None => match ctx.screen_language {
            Some(screen) => format!(
                "Write for the person in the language they write in; until they have written, in {screen}, the language of their screen."
            ),
            None => "Write for the person in the language they write in.".to_owned(),
        },
    };
    format!(
        "You are {name}, the {title} on a squad that equipe runs for one person, who leads it. \
Your teammates: {mates}.{projects}{reporting}\n\n{}\n\n{tools}{home}{browser}{desktop}{memory}\n\nYou run inside equipe, without a screen of your own: the person can't type into \
your session or change your permission mode, so never ask them to — nor to edit settings files for you. What your mode \
doesn't allow on its own goes to the person as a request they answer with a key; when they decline, or equipe \
refuses, say plainly what you need and why. CLIs like gh, vercel and npm are already signed in as the person: \
use them as they are, and never search the environment or files for tokens — that asks the person every time. \
A command whose output shows `<sandbox_violations>` was stopped by the fence equipe runs it in, whatever error it \
prints (\"token not valid\", \"not logged in\"): the login is fine, so never log in again or touch tokens over it — \
run it outside the fence, which asks the person once; their \"always\" stops it asking. Rust builds go to \
`$CARGO_TARGET_DIR`, one folder of yours kept from ticket to ticket, so a binary is under it, not under `./target`. \
Run one command at a time where you can: a single command can be allowed for good, a chain only once. Your memory \
outlives every ticket: when the person corrects you, sends work back or tells you how they like things, save the \
lesson there — their taste, the project's conventions, what to avoid — and check it before new work. What the person attaches comes as tags with a path on this \
machine; open each before you answer: `[image: <path>]` — look at it with Read; `[pdf: <path> · N pages]` — Read it \
(a long one a few pages at a time); `[file: <path> · N lines]` — Read it; `[video: <path> · 1:42 · 1920×1080 · \
sheet: <s> · frames: <f>@0:03, …]` — you can't watch a video: Read the frame images (each taken at its time) or the \
one sheet of them all, and say what you saw at which moment; `[audio: <path> · 0:31]` — you can't hear it: say so \
and ask what it says unless a tool you have can transcribe it. Side work that \
is long or stands on its own — running a whole test suite, reading logs, surveying unfamiliar code, researching — \
hand to subagents (your Agent tool), several at once when the pieces don't depend on each other, and keep only \
their summaries: your own context stays small. When you judge a ticket big enough and it splits into parts that \
don't touch the same files, you may give each part a subagent of its own and run them together — the person sees each one \
at work under your name, so describe each in a few plain words. When your conversation is compacted, keep in the summary the tickets and where each stands, what was decided with the person, \
their preferences, and anything they asked for that isn't done. {language} Be concise.",
        duties
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_lowercase_words() {
        let mut ids: Vec<_> = ROLES.iter().map(|r| r.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ROLES.len());
        assert!(ROLES.iter().all(|r| r.id.chars().all(|c| c.is_ascii_lowercase())));
    }

    #[test]
    fn finds_a_role_by_id_and_po_means_pm() {
        assert_eq!(role("qa").map(|r| r.id), Some("qa"));
        assert_eq!(role("po").map(|r| r.id), Some("pm"));
        assert_eq!(role("ceo"), None);
        assert_eq!(role_or_custom("writer").id, "custom");
    }

    #[test]
    fn the_pm_plans_and_does_not_edit() {
        let pm = role(PM).unwrap();
        assert!(!pm.edits);
        assert_eq!(pm.tier, Tier::Top);
        assert!(role("frontend").unwrap().edits);
    }

    #[test]
    fn editors_run_on_claude_and_qa_checks_on_the_other_engine() {
        assert_eq!(engine_preference(role("backend").unwrap())[0], Engine::Claude);
        assert_eq!(engine_preference(role("qa").unwrap())[0], Engine::Codex);
        assert_eq!(engine_preference(role(PM).unwrap())[0], Engine::Claude, "the PM needs its board tools");
    }

    #[test]
    fn models_are_recommended_from_what_the_plan_offers() {
        let catalog = vec!["gpt-6".to_owned(), "gpt-6-mini".to_owned()];
        assert_eq!(recommended_model(Tier::Top, Engine::Claude, &[]).as_deref(), Some("opus"));
        assert_eq!(recommended_model(Tier::Mid, Engine::Claude, &[]).as_deref(), Some("sonnet"));
        assert_eq!(recommended_model(Tier::Top, Engine::Codex, &catalog).as_deref(), Some("gpt-6"));
        assert_eq!(recommended_model(Tier::Mid, Engine::Codex, &catalog).as_deref(), Some("gpt-6-mini"));
        assert_eq!(recommended_model(Tier::Mid, Engine::Codex, &catalog[..1]).as_deref(), Some("gpt-6"));
        assert_eq!(recommended_model(Tier::Top, Engine::Codex, &[]), None);
    }

    fn ctx<'a>(team: &'a [Teammate<'a>], projects: &'a [String], board_tools: bool) -> Context<'a> {
        Context { team, projects, board_tools, language: None, screen_language: None, browser: false, desktop: false, home: None, memory: None, facts: 0, edits: false }
    }

    #[test]
    fn everything_for_the_person_is_written_in_their_language() {
        let team = [Teammate { name: "pm", role: "pm" }];
        let text = charter(role(PM).unwrap(), "pm", &Context { language: Some("Korean"), ..ctx(&team, &[], true) });
        assert!(text.contains("The person writes in Korean: write everything meant for them in Korean"), "{text}");
        assert!(text.contains("hand-in summaries"), "{text}");
        let unknown = charter(role(PM).unwrap(), "pm", &ctx(&team, &[], true));
        assert!(unknown.contains("in the language they write in"), "{unknown}");
        // Before they have written: the language their screen is in.
        let screen = charter(role(PM).unwrap(), "pm", &Context { screen_language: Some("Japanese"), ..ctx(&team, &[], true) });
        assert!(screen.contains("in the language they write in; until they have written, in Japanese"), "{screen}");
        let both = charter(role(PM).unwrap(), "pm", &Context { language: Some("Korean"), screen_language: Some("Japanese"), ..ctx(&team, &[], true) });
        assert!(both.contains("writes in Korean") && !both.contains("Japanese"), "what they write wins: {both}");
    }

    #[test]
    fn the_pm_is_told_how_to_pick_a_tickets_kind() {
        let team = [Teammate { name: "pm", role: "pm" }];
        let text = charter(role(PM).unwrap(), "pm", &ctx(&team, &[], true));
        for part in ["changing a repository is `code`", "is `task`", "is `action`", "needs no project"] {
            assert!(text.contains(part), "{part}: {text}");
        }
    }

    #[test]
    fn a_charter_says_what_the_squads_folder_is_for_and_what_needs_a_ticket() {
        let team = [Teammate { name: "pm", role: "pm" }];
        let text = charter(role("backend").unwrap(), "backend", &Context { home: Some("/h/squads/acme/home"), ..ctx(&team, &[], true) });
        for part in ["The squad's folder is /h/squads/acme/home", "`notes/`", "`outputs/`", "write there directly", "for reading only", "goes on a ticket", "asks the person first", "Other squads' folders are private"] {
            assert!(text.contains(part), "{part}: {text}");
        }
        assert!(!charter(role("backend").unwrap(), "backend", &ctx(&team, &[], true)).contains("squad's folder"));
    }

    #[test]
    fn a_charter_carries_the_squads_memory_and_how_to_add_to_it() {
        let team = [Teammate { name: "pm", role: "pm" }];
        let home = Context { home: Some("/h/squads/acme/home"), ..ctx(&team, &[], true) };
        let index = "- [tax-filing](tax-filing.md) (deadline) — 매년 5월 종합소득세 직접 신고";
        let text = charter(role("backend").unwrap(), "backend", &Context { memory: Some(index), facts: 1, ..home });
        for part in ["The squad's memory is /h/squads/acme/home/memory", "`remember`", "`forget`", "Never put a secret value", "wins over what you think", index] {
            assert!(text.contains(part), "{part}: {text}");
        }
        let empty = charter(role("backend").unwrap(), "backend", &Context { home: Some("/h"), ..ctx(&team, &[], true) });
        assert!(empty.contains("remembers nothing yet"), "{empty}");
        // Past a few screens of facts, the PM proposes tidying them.
        let crowded = Context { home: Some("/h"), facts: crate::squad_memory::CROWDED, ..ctx(&team, &[], true) };
        assert!(charter(role(PM).unwrap(), "pm", &crowded).contains("a ticket to tidy it"));
        let crowded = Context { home: Some("/h"), facts: crate::squad_memory::CROWDED, ..ctx(&team, &[], true) };
        assert!(!charter(role("backend").unwrap(), "backend", &crowded).contains("tidy it"));
        assert!(!charter(role("backend").unwrap(), "backend", &ctx(&team, &[], true)).contains("squad's memory"));
    }

    #[test]
    fn a_charter_names_the_member_its_team_and_who_keeps_the_plan() {
        let team = [Teammate { name: "pm", role: "pm" }, Teammate { name: "frontend", role: "frontend" }, Teammate { name: "frontend-2", role: "frontend" }];
        let projects = ["site".to_owned(), "app".to_owned()];
        let text = charter(role("frontend").unwrap(), "frontend", &ctx(&team, &projects, true));
        assert!(text.contains("several projects: site, app."), "{text}");
        assert!(text.contains("`submit_for_review`"), "{text}");
        assert!(text.starts_with("You are frontend, the frontend engineer"), "{text}");
        assert!(text.contains("Your teammates: pm, frontend-2 (frontend)."), "{text}");
        assert!(text.contains("pm (the PM) keeps the plan"), "{text}");
        assert!(text.contains("new ticket"), "{text}");
        assert!(text.contains("language they write in"), "{text}");
    }

    #[test]
    fn the_pm_and_members_put_real_decisions_as_options_and_never_permissions() {
        let team = [Teammate { name: "pm", role: "pm" }, Teammate { name: "backend", role: "backend" }];
        for (role_id, name) in [("pm", "pm"), ("backend", "backend")] {
            let text = charter(role(role_id).unwrap(), name, &ctx(&team, &[], true));
            for part in [
                "`ask_decision`",
                "two to four options",
                "\"(Recommended)\"",
                "person's language",
                "`block_on_person`",
                "for permissions",
                "never end a reply on a numbered list of questions",
                "question without options",
                "one item for each",
            ] {
                assert!(text.contains(part), "{role_id}: {part}: {text}");
            }
        }
        let text = charter(role("backend").unwrap(), "backend", &ctx(&team, &[], false));
        assert!(!text.contains("ask_decision"), "no tools, no such advice");
    }

    #[test]
    fn a_member_asks_the_person_one_whole_sentence_an_item_and_files_by_path() {
        let team = [Teammate { name: "pm", role: "pm" }, Teammate { name: "ios", role: "mobile" }];
        let text = charter(role("mobile").unwrap(), "ios", &ctx(&team, &[], true));
        for part in ["its own item in", "complete sentence", "never several joined into one", "path in backticks", "\"do it yourself\") — then it is yours to do", "asked again with your next"] {
            assert!(text.contains(part), "{part}: {text}");
        }
        for part in ["\"do it yourself\"", "`show_my_browser`", "saying exactly why", "\"later\": go on without it", "\"your call\"", "doesn't know", "From the person:"] {
            assert!(ANSWERS_GUIDE.contains(part), "{part}");
        }
        let schema = crate::board::tools(false).into_iter().find(|t| t["name"] == crate::board::BLOCK_ON_PERSON).unwrap();
        let needs = schema["inputSchema"]["properties"]["needs"]["description"].as_str().unwrap();
        assert!(needs.contains("one per item") && needs.contains("complete sentence") && needs.contains("backticks"), "{needs}");
    }

    #[test]
    fn with_several_projects_the_pm_reports_by_project_and_apps_have_a_mobile_engineer() {
        let team = [Teammate { name: "pm", role: "pm" }];
        let projects = ["site".to_owned(), "app".to_owned()];
        let text = charter(role(PM).unwrap(), "pm", &ctx(&team, &projects, true));
        assert!(text.contains("a heading for each project"), "{text}");
        let one = charter(role(PM).unwrap(), "pm", &ctx(&team, &projects[..1], true));
        assert!(!one.contains("a heading for each project"), "{one}");
        assert!(role("mobile").is_some_and(|r| r.edits && r.duties.contains("iOS")));
    }

    #[test]
    fn the_pms_own_charter_does_not_point_at_itself() {
        let team = [Teammate { name: "pm", role: "pm" }];
        let text = charter(role(PM).unwrap(), "pm", &ctx(&team, &[], true));
        assert!(text.contains("nobody else yet"), "{text}");
        assert!(!text.contains("keeps the plan. The person may"), "{text}");
        assert!(text.contains("`create_ticket`") && text.contains("Only the person accepts"), "{text}");
    }

    #[test]
    fn a_custom_role_is_named_by_the_person_who_says_whether_it_edits() {
        let text = charter(&CUSTOM, "writer", &ctx(&[], &[], false));
        assert!(text.contains("no board tools"), "{text}");
        assert!(text.starts_with("You are writer, the writer (a custom role)"), "{text}");
        assert!(text.contains("you don't change their files") && !text.contains("You may change"), "{text}");
        let edits = charter(&CUSTOM, "editor", &Context { edits: true, ..ctx(&[], &[], false) });
        assert!(edits.contains("You may change a project's files on a code ticket"), "{edits}");
    }

    #[test]
    fn the_pm_does_short_work_itself_and_puts_bigger_work_of_any_kind_on_tickets() {
        let team = [Teammate { name: "pm", role: "pm" }];
        let text = charter(role(PM).unwrap(), "pm", &ctx(&team, &[], true));
        for part in ["research, writing, numbers, paperwork", "or yourself", "Not every request needs a ticket", "do yourself", "own it yourself", "`/add <role>`", "`--edits`", "Code needs an engineer"] {
            assert!(text.contains(part), "{part}: {text}");
        }
        // The rule for code stays: the PM doesn't change a project's code.
        assert!(text.contains("You don't change a project's code yourself"), "{text}");
        assert!(!text.contains("software squad"), "{text}");
        // Nothing in a role's duties says all work is code.
        assert!(ROLES.iter().chain([&CUSTOM]).all(|r| !r.duties.contains("you don't edit files")));
    }

    #[test]
    fn what_a_member_hands_over_as_fact_names_its_source() {
        let team = [Teammate { name: "pm", role: "pm" }];
        let text = charter(role("data").unwrap(), "data", &Context { home: Some("/h"), ..ctx(&team, &[], true) });
        assert!(text.contains("says where it came from") && text.contains("marked unconfirmed"), "{text}");
    }
}
