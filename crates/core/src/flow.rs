//! How a person's request becomes shipped work (docs/FLOW.md): the rules,
//! with no runtime in them, each tested alone.
//!
//! - **One ticket, one driver.** The ticket a member puts on the board in
//!   answer to the person is the request's main ticket; that member drives
//!   it to production ([`equipe_protocol::TicketFlow::driver`]). Spec, build
//!   and checks are items of its checklist ([`default_steps`]), not tickets
//!   of their own.
//! - **The baton.** Whoever holds the ticket passes it on — designer to
//!   frontend, or to a helper for one step and back — on the same ticket,
//!   branch and worktree, without a new ticket or the PM ([`PASS_BATON`]).
//! - **A UI change is one slice** ([`is_slice`]): the designer's prototype
//!   and the build on one branch, checked against the prototype by equipe's
//!   precheck — no design QA ticket, a designer only on what it fails.
//! - **Members land their own work** when the automated gates pass and
//!   nothing about it needs a person ([`landing`]); the PM looks after the
//!   fact, in a digest, and keeps acceptance only for what the gate holds.
//! - **No tickets for meta work** ([`meta_refusal`]): checks of work under
//!   way go into its checklist; findings outside it go to a note or the
//!   backlog, unless they are S1/S2.
//! - **Lanes fit the machine** ([`machine_cap`]) and never overlap
//!   ([`overlaps`]); a long queue finishes before it starts ([`prefer_finishing`]).
//!
//! The runtime's side is `runtime::flow`.

use equipe_protocol::title::TicketLabel;
use equipe_protocol::{Ticket, TicketFlow, TicketId, TicketKind, TicketStep};
use serde_json::Value;

use crate::i18n::Catalog;

/// The board tool that moves the ticket a member holds to another member,
/// on the same ticket and branch.
pub const PASS_BATON: &str = "pass_baton";
/// The board tool that ticks off — or adds — items of the checklist of the
/// ticket a member holds.
pub const CHECK_STEPS: &str = "check_steps";

/// How a ticket handed on by [`PASS_BATON`] says so in its note, for the
/// member who picks it up: who passed it, then their note.
pub const BATON_FROM: &str = "Baton from ";

/// The roles of a UI slice: the designer prototypes, the frontend builds.
pub const DESIGNER: &str = "designer";
pub const FRONTEND: &str = "frontend";

/// A squad setting: members land their own work when its gates pass
/// (`self_land`, on unless the squad turned it off).
pub const SELF_LAND: &str = "self_land";

/// The role a name stands for: `frontend-2` is a frontend.
fn role_of(name: &str) -> &str {
    crate::board::role_of(name)
}

/// Whether a ticket made now is a UI change built as one slice: code, about
/// how something looks or a feature people see, from or for the designer —
/// so the prototype and the build share one branch.
pub fn is_slice(kind: TicketKind, label: Option<TicketLabel>, maker_role: &str, owner: Option<&str>) -> bool {
    let designed = maker_role == DESIGNER || owner.is_some_and(|o| role_of(o) == DESIGNER);
    kind == TicketKind::Code && designed && !matches!(label, Some(TicketLabel::Check | TicketLabel::Chore | TicketLabel::Deploy))
}

/// A checklist item.
fn step(text: &str, who: Option<&str>) -> TicketStep {
    TicketStep { text: text.to_owned(), who: who.map(str::to_owned), done: false, by: None, ..Default::default() }
}

/// The checklist a request's main ticket starts with, in the person's
/// language: a slice — prototype, build, the check against the prototype;
/// other code — the change and its tests; a task or an action — none (its
/// work is one step).
pub fn default_steps(slice: bool, kind: TicketKind, owner: Option<&str>, words: &Catalog) -> Vec<TicketStep> {
    if slice {
        return vec![
            step(words.get("flow.step.prototype"), Some(DESIGNER)),
            step(words.get("flow.step.build"), Some(FRONTEND)),
            step(words.get("flow.step.precheck"), None),
        ];
    }
    if kind != TicketKind::Code {
        return Vec::new();
    }
    let who = owner.map(role_of);
    vec![step(words.get("flow.step.change"), who), step(words.get("flow.step.tests"), who)]
}

/// The `steps` of a `create_ticket` or `check_steps` call: each a string,
/// or `{text, who}`. Empty texts are left out.
pub fn steps_arg(args: &Value, key: &str) -> Result<Vec<TicketStep>, String> {
    let Some(given) = args.get(key) else { return Ok(Vec::new()) };
    // One line on its own is a list of one; a list written as a string is that list.
    let items: Vec<Value> = match given {
        Value::Array(items) => items.clone(),
        Value::String(text) => crate::board::list_in_string(text).unwrap_or_else(|| vec![given.clone()]),
        Value::Object(_) => vec![given.clone()],
        Value::Null => Vec::new(),
        _ => return Err(format!("`{key}` is a list of lines, like {{\"{key}\": [\"Write the tests\", {{\"text\": \"Check it at 390px\", \"who\": \"designer\"}}]}}.")),
    };
    let mut out = Vec::new();
    for item in &items {
        let (text, who) = match item {
            Value::String(s) => (s.trim().to_owned(), None),
            Value::Object(o) => (
                o.get("text").and_then(Value::as_str).unwrap_or_default().trim().to_owned(),
                o.get("who").and_then(Value::as_str).map(str::trim).filter(|w| !w.is_empty()).map(str::to_owned),
            ),
            _ => return Err(format!("Each item of `{key}` is a line, or {{\"text\", \"who\"}} — like {{\"{key}\": [\"Write the tests\"]}}; {item} is neither.")),
        };
        if !text.is_empty() {
            out.push(TicketStep { text, who, done: false, by: None, ..Default::default() });
        }
    }
    Ok(out)
}

/// The checklist as a member reads it: one line per item, numbered from 1.
pub fn checklist(flow: &TicketFlow) -> String {
    flow.steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mark = if s.done { "x" } else { " " };
            let who = s.who.as_deref().map(|w| format!(" — {w}")).unwrap_or_default();
            let by = s.by.as_deref().filter(|_| s.done).map(|b| format!(" (by {b})")).unwrap_or_default();
            format!("{}. [{mark}] {}{who}{by}", i + 1, s.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The checklist items `arg` names, numbered from 1: a number (`2`, `"2"`,
/// `"#2"`), or an item's own words (`"Write the tests"`), one or a list —
/// or a list written as a string. An item it can't find is refused with the
/// checklist and the call that names it, for `tool`'s argument `key`.
pub fn item_numbers(flow: &TicketFlow, ticket: TicketId, tool: &str, key: &str, arg: Option<&Value>) -> Result<Vec<usize>, String> {
    let Some(arg) = arg else { return Ok(Vec::new()) };
    let items: Vec<Value> = match arg {
        Value::Array(items) => items.clone(),
        Value::Null => Vec::new(),
        Value::String(text) => match crate::board::list_in_string(text) {
            Some(items) => items,
            // A whole item's words, or numbers: "2, 3".
            None if flow.steps.iter().any(|s| same_item(&s.text) == same_item(text)) => vec![arg.clone()],
            None => text.split([',', ';']).map(|t| Value::String(t.trim().to_owned())).filter(|t| t.as_str() != Some("")).collect(),
        },
        other => vec![other.clone()],
    };
    let mut out = Vec::new();
    for item in &items {
        let number = item
            .as_u64()
            .or_else(|| item.as_f64().filter(|f| *f >= 1.0 && f.fract() == 0.0).map(|f| f as u64))
            .or_else(|| item.as_str().and_then(|t| t.trim().trim_start_matches('#').trim_end_matches('.').parse().ok()))
            .map(|n| n as usize);
        let found = number.or_else(|| {
            let words = same_item(item.as_str().unwrap_or_default());
            flow.steps.iter().position(|s| !words.is_empty() && same_item(&s.text) == words).map(|at| at + 1)
        });
        match found {
            Some(n) if !out.contains(&n) => out.push(n),
            Some(_) => {}
            None => {
                let example = flow.steps.iter().position(|s| !s.done).unwrap_or(0) + 1;
                return Err(format!(
                    "`{key}` names checklist items by number, and {item} isn't one of T-{ticket}'s. Its checklist:\n{}\nCall it again with the numbers, like {}.",
                    if flow.steps.is_empty() { "(empty)".to_owned() } else { checklist(flow) },
                    crate::board::call(tool, &serde_json::json!({ key: [example] }))
                ));
            }
        }
    }
    Ok(out)
}

/// What `check_steps` says to a call that changes nothing — no `done`, `add`
/// or `replace` it could read: T-`ticket`'s checklist, and the calls that
/// tick, add and re-plan it, so the next call is right.
pub fn check_steps_help(ticket: TicketId, flow: &TicketFlow, unknown: Option<String>) -> String {
    let unknown = unknown.map(|u| format!(" {u}")).unwrap_or_default();
    if flow.steps.is_empty() {
        return format!(
            "Nothing was changed: give `done` (item numbers) or `add` (new items).{unknown} T-{ticket} has no checklist yet — add its items, like {}.",
            crate::board::call(CHECK_STEPS, &serde_json::json!({ "add": ["Build the sign-up form", "Test it at 390px and 1280px"] }))
        );
    }
    let open = flow.steps.iter().position(|s| !s.done).map_or(1, |at| at + 1);
    format!(
        "Nothing was changed: give `done` (item numbers from 1) or `add` (new items).{unknown} T-{ticket}'s checklist:\n{}\nTo tick item {open} off: {}. To add one: {}. Both in one call: {}.",
        checklist(flow),
        crate::board::call(CHECK_STEPS, &serde_json::json!({ "done": [open] })),
        crate::board::call(CHECK_STEPS, &serde_json::json!({ "add": ["Write the tests"] })),
        crate::board::call(CHECK_STEPS, &serde_json::json!({ "done": [open], "add": ["Write the tests"] })),
    )
}

/// How an item reads when two are compared: case, spacing and a closing
/// full stop aside — "Put it live." and "put it  live" are one item.
fn same_item(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches(['.', '。', '!', '！']).to_lowercase()
}

/// Tick off items `done` (numbered from 1) by `by`; take out the open items
/// `replace` (a re-plan: what `added` takes the place of); then add `added`
/// at the end — each one the checklist doesn't have already, so a member who
/// re-plans by listing its items again never leaves the same item twice.
/// What it says back, or why it can't.
pub fn tick(flow: &mut TicketFlow, done: &[usize], replace: &[usize], added: Vec<TicketStep>, by: &str) -> Result<String, String> {
    let has = flow.steps.len();
    if let Some(bad) = done.iter().chain(replace).find(|n| **n == 0 || **n > has) {
        return Err(format!("There is no item {bad}: the checklist has {has} (numbered from 1):\n{}", checklist(flow)));
    }
    if let Some(both) = replace.iter().find(|n| done.contains(n)) {
        return Err(format!("Item {both} can't be both done and replaced."));
    }
    if let Some(kept) = replace.iter().find(|n| flow.steps[**n - 1].done) {
        return Err(format!("Item {kept} is done — it stays. Only open items are replaced."));
    }
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    for n in done {
        let item = &mut flow.steps[n - 1];
        if !item.done {
            item.done_at = Some(at);
        }
        item.done = true;
        item.by = Some(by.to_owned());
        item.unconfirmed = false;
    }
    let mut gone: Vec<usize> = replace.to_vec();
    gone.sort_unstable();
    gone.dedup();
    for n in gone.iter().rev() {
        flow.steps.remove(n - 1);
    }
    let (mut added_n, mut repeated) = (0, 0);
    for item in added {
        let key = same_item(&item.text);
        if flow.steps.iter().any(|s| same_item(&s.text) == key) {
            repeated += 1;
            continue;
        }
        flow.steps.push(item);
        added_n += 1;
    }
    let merged = merge_repeats(flow);
    let (ticked, all) = flow.progress();
    let mut said = format!("{ticked} of {all} done.");
    if merged > 0 {
        said.push_str(&format!(" {merged} said twice, kept once."));
    }
    if added_n > 0 {
        said.push_str(&format!(" {added_n} added."));
    }
    if !gone.is_empty() {
        said.push_str(&format!(" {} replaced.", gone.len()));
    }
    if repeated > 0 {
        said.push_str(&format!(" {repeated} already on it, not added again."));
    }
    Ok(said)
}

/// The checklist with each item said once: a later item that says what an
/// earlier one does is folded into it (done if either was), so a plan
/// revised by appending never shows the same step twice. How many went.
pub fn merge_repeats(flow: &mut TicketFlow) -> usize {
    let mut kept: Vec<TicketStep> = Vec::new();
    let mut gone = 0;
    for step in std::mem::take(&mut flow.steps) {
        let key = same_item(&step.text);
        match kept.iter_mut().find(|k| same_item(&k.text) == key) {
            Some(first) => {
                gone += 1;
                if step.done && !first.done {
                    first.done = true;
                    first.by = step.by;
                    first.done_at = step.done_at;
                }
                if first.note.is_none() {
                    first.note = step.note;
                }
            }
            None => kept.push(step),
        }
    }
    flow.steps = kept;
    gone
}

/// What a member's tool call showed about its ticket's work, read from the
/// call's own answer: evidence its checklist is ticked by, so items don't
/// wait on the member remembering `check_steps` (Team Aman's T-65 ran twice,
/// deployed its preview four times and passed QA, and ticked none of 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// The "done" checklist passed (`qa_check`).
    Passed,
    /// A build is up on the product's private preview.
    Preview,
    /// It went live: a live deploy, a version promoted, main deployed.
    Live,
}

/// The platform tools a member's run shows its work with.
const PLATFORM: &str = "mcp__equipe-platform__";

/// The [`Evidence`] in a finished tool call, if any: its name, whether it
/// succeeded, and what it answered.
pub fn evidence_of(tool: &str, ok: bool, output: &str) -> Option<Evidence> {
    let name = tool.strip_prefix(PLATFORM)?;
    if !ok {
        return None;
    }
    // Spaces and escapes aside: the answer may come as JSON inside a text.
    let compact: String = output.chars().filter(|c| !c.is_whitespace() && *c != '\\').collect();
    match name {
        // Its answer says `ok` for the whole checklist; a failed one says what to fix next.
        "qa_check" => (compact.contains("\"ok\":true") && !compact.contains("\"ok\":false") && !output.contains("Fix every failed check")).then_some(Evidence::Passed),
        // Only an answer with the address it is at: a refusal is no deploy.
        "deploy_site" if compact.contains("\"url\":\"http") => Some(if compact.contains("\"preview\":true") { Evidence::Preview } else { Evidence::Live }),
        "promote_version" => Some(Evidence::Live),
        "deploy_main" => crate::go_live::cloud_answer(output).filter(|a| !a.live.is_empty()).map(|_| Evidence::Live),
        _ => None,
    }
}

/// Whether `text`, a checklist item, is putting the work live — by the
/// go-live words of every language equipe speaks (`flow.live_words`): whole
/// words where words are spaced, as written where they are not.
pub fn is_live_step(text: &str) -> bool {
    let lower = text.to_lowercase();
    crate::i18n::every("flow.live_words").iter().any(|w| {
        let w = w.to_lowercase();
        if w.chars().any(|c| c >= '\u{2E80}') {
            return lower.contains(&w);
        }
        lower.match_indices(&w).any(|(at, _)| {
            let before = lower[..at].chars().next_back();
            let after = lower[at + w.len()..].chars().next();
            before.is_none_or(|c| !c.is_alphanumeric()) && after.is_none_or(|c| !c.is_alphanumeric())
        })
    })
}

/// The items `evidence` ticks off during a run of `holder`: a passed check,
/// the holder's open items short of going live; going live, every open
/// go-live item, whoever it is named for. A preview alone ticks nothing
/// while the work goes on — it counts at the hand-in.
pub fn ticked_by(flow: &TicketFlow, holder: &str, evidence: Evidence) -> Vec<usize> {
    match evidence {
        Evidence::Passed => open_for(flow, holder).into_iter().filter(|n| !is_live_step(&flow.steps[n - 1].text)).collect(),
        Evidence::Preview => Vec::new(),
        Evidence::Live => flow.steps.iter().enumerate().filter(|(_, s)| !s.done && is_live_step(&s.text)).map(|(i, _)| i + 1).collect(),
    }
}

/// What a hand-in by `holder` does to its items still open, given what its
/// run showed (`shown`): `(confirmed, unconfirmed)`. Each item but a go-live
/// one is ticked — confirmed when the run passed a check or put a build up,
/// else on the member's word ([`TicketStep::unconfirmed`]). A go-live item
/// is ticked only by going live: otherwise it stays open, for the deploy
/// after landing to tick.
pub fn hand_in_ticks(flow: &TicketFlow, holder: &str, shown: &[Evidence]) -> (Vec<usize>, Vec<usize>) {
    let built = !shown.is_empty();
    let live = shown.contains(&Evidence::Live);
    let (mut confirmed, mut unconfirmed) = (Vec::new(), Vec::new());
    for n in open_for(flow, holder) {
        if is_live_step(&flow.steps[n - 1].text) {
            if live {
                confirmed.push(n);
            }
        } else if built {
            confirmed.push(n);
        } else {
            unconfirmed.push(n);
        }
    }
    (confirmed, unconfirmed)
}

/// The numbers (from 1) of the items still open for `holder`: what its
/// baton passed with no `done` ticks off.
pub fn open_for(flow: &TicketFlow, holder: &str) -> Vec<usize> {
    flow.steps.iter().enumerate().filter(|(_, s)| !s.done && s.who.as_deref().is_none_or(|w| w == holder || w == role_of(holder))).map(|(i, _)| i + 1).collect()
}

/// Whether a turn that began at `began` ticked nothing while `holder` still
/// has open items: its next turn is reminded to.
pub fn ticked_nothing(flow: &TicketFlow, holder: &str, began: u64) -> bool {
    !open_for(flow, holder).is_empty() && !flow.steps.iter().any(|s| s.done_at.is_some_and(|at| at >= began))
}

/// The reminder a run that ticked nothing gets with its next turn.
pub fn tick_reminder(ticket: TicketId, flow: &TicketFlow, holder: &str) -> String {
    let open: Vec<String> = open_for(flow, holder).iter().map(|n| format!("{n}. {}", flow.steps[n - 1].text)).collect();
    format!(
        "[equipe] T-{ticket}: you ticked nothing on its checklist last turn, and the person follows the work by it. First thing now, \
tick each item you finished with `check_steps` (`done`), and for one they can see or try give `note`: one plain line for them with \
where (\"The booking page is ready — try it at /book\"). Still open for you:\n{}",
        open.join("\n")
    )
}

/// The items left for `holder` (its role) — or for whoever holds it.
pub fn left_for<'a>(flow: &'a TicketFlow, holder: &str) -> Vec<&'a TicketStep> {
    flow.steps.iter().filter(|s| !s.done && s.who.as_deref().is_none_or(|w| w == holder || w == role_of(holder))).collect()
}

/// The next holder the checklist names after `holder`'s items: the role of
/// the first open item that isn't theirs.
pub fn next_holder(flow: &TicketFlow, holder: &str) -> Option<String> {
    flow.steps.iter().find(|s| !s.done && s.who.as_deref().is_some_and(|w| w != holder && w != role_of(holder))).and_then(|s| s.who.clone())
}

/// Why `to` can't take the baton from `holder` on `ticket`, if it can't.
pub fn baton_refusal(ticket: &Ticket, holder: &str, to: &str) -> Option<String> {
    if to.trim().is_empty() {
        return Some("Say who takes it: `to`, a role (frontend) or a member's name.".into());
    }
    if to == holder {
        return Some(format!("You hold T-{} already.", ticket.id));
    }
    if ticket.kind == TicketKind::Action {
        return Some(format!("T-{} acts in the world: it stays with one member from start to end. Hand it in, or hand it back.", ticket.id));
    }
    None
}

/// How many times a ticket may change hands between hand-ins. Past it the
/// baton has become a loop — two roles checking each other while the
/// checklist grows — and the holder hands in what it has instead.
pub const MAX_BATONS: usize = 8;

/// How many items a ticket's checklist may grow to. A checklist that keeps
/// growing is scope that belongs to the PM, not to the ticket's holders.
pub const MAX_STEPS: usize = 16;

/// How many times the ticket whose record is `kinds` (oldest first) changed
/// hands since it was last handed in or sent back.
pub fn batons_since_hand_in<'a>(kinds: impl IntoIterator<Item = &'a str>) -> usize {
    let mut passes = 0;
    for kind in kinds {
        match kind {
            crate::lead_time::BATON => passes += 1,
            "handed in" | "sent back" | "design QA: changes" => passes = 0,
            _ => {}
        }
    }
    passes
}

/// Why `pass_baton` stops a loop: after `passes` hand-overs since the last
/// hand-in, the holder hands in what it has.
pub fn baton_loop(ticket: TicketId, passes: usize) -> Option<String> {
    (passes >= MAX_BATONS).then(|| {
        format!(
            "T-{ticket} has changed hands {passes} times since it was last handed in: passing it on again is a loop, not progress. \
Hand it in now with `submit_for_review`: say what is done and what is left (with its evidence) in the summary. The PM decides \
whether what is left is worth more work, as a narrower ticket."
        )
    })
}

/// Why `check_steps` refuses to add `adding` items to a checklist of `has`.
pub fn steps_overflow(ticket: TicketId, has: usize, adding: usize) -> Option<String> {
    (adding > 0 && has + adding > MAX_STEPS).then(|| {
        format!(
            "T-{ticket}'s checklist would grow past {MAX_STEPS} items: that is new scope, not this ticket. Finish or tick what is there and \
hand it in; name what else you found in the summary for the PM."
        )
    })
}

/// The note the next holder starts with ([`BATON_FROM`]).
pub fn baton_note(from: &str, note: &str) -> String {
    format!("{BATON_FROM}{from}: {}", note.trim())
}

/// What the member who picks a baton up is told at the start of its run.
pub fn picked_up(from_note: &str, flow: Option<&TicketFlow>, holder: &str) -> String {
    let mut out = format!(
        "{from_note}\nYou now hold this ticket. The work so far is on its branch, in this worktree — read `git log` and \
`git status` first and build on it; don't redo it.\n"
    );
    if let Some(flow) = flow.filter(|f| !f.steps.is_empty()) {
        let mine = left_for(flow, holder);
        out.push_str(&format!("Its checklist:\n{}\n", checklist(flow)));
        if !mine.is_empty() {
            out.push_str("Do your items, tick each off with `check_steps` as it is done, then pass the baton on (`pass_baton`) or hand it in.\n");
        }
    }
    out
}

/// How the work of a request's ticket is told to its holder at the start of
/// every run: its driver, its checklist, and how to move it on.
pub fn brief(flow: &TicketFlow, holder: &str, ticket: TicketId) -> String {
    let mut out = String::new();
    if let Some(driver) = &flow.driver {
        out.push_str(&format!("T-{ticket} carries the person's request to production; {driver} drives it. "));
    }
    if flow.slice {
        out.push_str(
            "It is one UI slice: the designer's prototype or spec and the build go on this same branch; equipe's precheck checks \
the built screens against the prototype when it is handed in — no design QA ticket. ",
        );
    }
    if !flow.steps.is_empty() {
        out.push_str(&format!("\nIts checklist:\n{}\n", checklist(flow)));
    }
    out.push_str(&format!(
        "Work that is part of it — a spec, the build, a check — is an item of this checklist (`check_steps` adds and ticks them), \
never a ticket of its own. When your items are done and another's are next, pass the baton with `pass_baton` (to {}): \
they carry on in this worktree. When everything is done, hand it in.",
        next_holder(flow, holder).unwrap_or_else(|| "the role named on the next item".to_owned())
    ));
    out
}

// ── no tickets for meta work ──

/// A ticket a member asks for, as the meta-work rule weighs it.
#[derive(Debug, Clone, Default)]
pub struct Asked<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub label: Option<TicketLabel>,
    /// A designer's severity of the finding it files (`S1`..`S4`).
    pub severity: Option<&'a str>,
    pub depends_on: &'a [TicketId],
    /// Why it is work of its own, though made while on another ticket.
    pub separate_why: Option<&'a str>,
    pub idea: bool,
}

/// What the member asking is doing.
#[derive(Debug, Clone, Copy, Default)]
pub struct Maker<'a> {
    pub role: &'a str,
    pub pm: bool,
    /// The ticket it is at work on, if any.
    pub working: Option<TicketId>,
    /// That ticket serves a request of the person's.
    pub working_serves_request: bool,
    /// The new ticket would serve the same request (it is made while working one of its tickets).
    pub same_request: bool,
}

/// Whether a ticket is meta work: a check — QA, verifying, reviewing — or a
/// polish finding (S3/S4).
pub fn is_meta(asked: &Asked<'_>) -> bool {
    asked.label == Some(TicketLabel::Check) || matches!(asked.severity.map(str::to_ascii_uppercase).as_deref(), Some("S3" | "S4"))
}

/// The open tickets a ticket's words name (`T-123`) or it depends on.
fn about(asked: &Asked<'_>, open: &[TicketId]) -> Vec<TicketId> {
    let mut named: Vec<TicketId> = asked.depends_on.to_vec();
    let text = format!("{} {}", asked.title, asked.body);
    let mut from = 0;
    while let Some((at, len)) = crate::board::ticket_ref_from(&text, from) {
        if let Some(id) = crate::board::parse_ticket_id(&text[at..at + len]) {
            named.push(id);
        }
        from = at + len;
    }
    named.retain(|t| open.contains(t));
    named.sort_unstable();
    named.dedup();
    named
}

/// Why `create_ticket` turns a ticket away under the meta-work budget, if
/// it does (English, for the member):
///
/// - a check or polish of the ticket it is at work on goes into that
///   ticket's checklist — out of its scope, into a note or the backlog;
/// - a check of another ticket under way goes into that one's checklist;
/// - the next part of a request it is at work on — the build after the
///   spec — is the same ticket's baton, unless `separate_why` says why it
///   is work of its own.
///
/// An S1/S2 finding is always a ticket. Ideas aren't weighed.
pub fn meta_refusal(asked: &Asked<'_>, maker: &Maker<'_>, open: &[TicketId]) -> Option<String> {
    if asked.idea || matches!(asked.severity.map(str::to_ascii_uppercase).as_deref(), Some("S1" | "S2")) {
        return None;
    }
    if is_meta(asked) {
        if let Some(on) = maker.working {
            return Some(format!(
                "A check or polish of the work you are on isn't a ticket of its own: add it to T-{on}'s checklist with `{CHECK_STEPS}` \
(`add`) and do it there. Something outside T-{on}'s scope goes into a `comment` or the design backlog — only an S1/S2 \
finding is a ticket now."
            ));
        }
        if let Some(of) = about(asked, open).first() {
            let how = if maker.pm { format!("put it in T-{of}'s checklist with `update_ticket` (`steps`)") } else { format!("ask T-{of}'s holder to add it to its checklist (a `comment` on T-{of})") };
            return Some(format!(
                "T-{of} is under way, and checking it is part of it, not a ticket of its own: {how}. Its holder checks it before it lands."
            ));
        }
        return None;
    }
    if maker.working_serves_request && maker.same_request && asked.separate_why.is_none_or(|w| w.trim().is_empty()) {
        let on = maker.working.unwrap_or_default();
        return Some(format!(
            "T-{on} carries the person's request, and the next part of it — the build after a spec, a fix of what you found in it — \
goes on T-{on} itself: add it to its checklist with `{CHECK_STEPS}` and pass the baton (`{PASS_BATON}`, to the role that does it). \
They carry on in the same worktree, with no new ticket and no wait for the PM. Only work that doesn't build on T-{on}'s branch is a \
ticket of its own: then say why in `separate_why`."
        ));
    }
    None
}

// ── members land their own work ──

/// Who lands work that is ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// Its member, now: the gates run as for the PM's acceptance.
    Member,
    /// The PM accepts it, as before — and why.
    Pm(Held),
}

/// Why work that is ready waits for the PM rather than landing itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// The person accepts the team's work in this squad.
    PersonAccepts,
    /// The squad turned self-landing off.
    Off,
    /// It needs a person: production data, permissions, money, legal, can't
    /// be taken back, a big visible change.
    Gated(crate::person_gate::Why),
    /// The PM's own work: never landed by whoever made it.
    PmsOwn,
    /// The person was promised to see it and say yes first ([`HOLD_LINE`]):
    /// it stays on its branch and preview until they do.
    PersonConfirms,
}

/// The line a ticket's body carries while it waits for the person's yes
/// before it lands and goes live: put there by `wait_for_person` on
/// `create_ticket`/`update_ticket`, or by equipe when the PM told the person
/// it would wait for their pick or confirmation (inhwadoen, 2026-11-28: "I'll
/// apply it once you confirm", and it went live twelve minutes later).
pub const HOLD_LINE: &str = "live: after the person's yes";

/// Whether a ticket's `body` holds it for the person's yes.
pub fn held_for_person(body: &str) -> bool {
    body.lines().any(|l| l.trim().eq_ignore_ascii_case(HOLD_LINE))
}

/// `body` held for the person's yes (`hold`) or let go.
pub fn with_hold(body: &str, hold: bool) -> String {
    let rest: Vec<&str> = body.lines().filter(|l| !l.trim().eq_ignore_ascii_case(HOLD_LINE)).collect();
    let rest = rest.join("\n").trim_end().to_owned();
    match (hold, rest.is_empty()) {
        (false, _) => rest,
        (true, true) => HOLD_LINE.to_owned(),
        (true, false) => format!("{rest}\n\n{HOLD_LINE}"),
    }
}

/// Whether `confirmed`, the person's yes the PM quotes, is in what they
/// said lately (`said`, newest last): a few words of theirs, as they wrote them.
pub fn yes_heard(confirmed: &str, said: &[String]) -> bool {
    let plain = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let quoted = plain(confirmed.trim_matches(|c: char| c == '"' || c == '“' || c == '”' || c.is_whitespace()));
    quoted.chars().count() >= 2 && said.iter().any(|s| plain(s).contains(&quoted))
}

/// Who lands `ticket`, ready (handed in, its design check passed), with
/// `changed` the files its branch changed.
pub fn landing(self_land: bool, pm_accepts: bool, ticket: &Ticket, changed: &[String], pms_own: bool) -> Landing {
    landing_with(self_land, pm_accepts, ticket, changed, &std::collections::BTreeMap::new(), pms_own)
}

/// [`landing`], with `sql` the contents of the SQL migrations its branch
/// changed, by path: one that only adds lands like code
/// ([`crate::person_gate::weigh_change`]).
pub fn landing_with(self_land: bool, pm_accepts: bool, ticket: &Ticket, changed: &[String], sql: &std::collections::BTreeMap<String, String>, pms_own: bool) -> Landing {
    if !pm_accepts {
        return Landing::Pm(Held::PersonAccepts);
    }
    if held_for_person(&ticket.body) {
        return Landing::Pm(Held::PersonConfirms);
    }
    if pms_own {
        return Landing::Pm(Held::PmsOwn);
    }
    if !self_land {
        return Landing::Pm(Held::Off);
    }
    match crate::person_gate::weigh_change(ticket, changed, sql) {
        Some(why) => Landing::Pm(Held::Gated(why)),
        None => Landing::Member,
    }
}

/// What the PM is told of work its members landed themselves, in one
/// digest: each with what changed, and how to revert or follow it up.
pub fn digest(landed: &[(TicketId, String, String, String)]) -> String {
    let lines: Vec<String> = landed.iter().map(|(id, title, by, what)| if what.is_empty() { format!("- T-{id} {title} — {by}") } else { format!("- T-{id} {title} — {by}: {what}") }).collect();
    format!(
        "[equipe: landed by their members since your last digest, their gates passed (tests, precheck, a clean merge):\n{}\n\
Look them over. Anything wrong: `revert_landed` (the ticket, and why) puts its revert at the front of its member's queue; anything \
to add: a follow-up ticket for its member. Nothing to do if they're fine — don't reply about each.]",
        lines.join("\n")
    )
}

/// How the digest marks work that carried an additive schema migration: it
/// landed like code, and the PM hears so here.
pub const ADDITIVE_MIGRATION: &str = "with an additive schema migration (new tables, indexes or columns; nothing dropped or rewritten)";

/// The board tool the PM reverts work a member landed with.
pub const REVERT_LANDED: &str = "revert_landed";

// ── lanes that fit the machine ──

/// How many runs a member may have at once on this machine: its lane
/// policy's `base`, eased off while the machine is loaded — the 1-minute
/// load over its cores at 1.0 takes one away, at 1.5 halves it, at 2.0
/// leaves one — and one while less than a tenth of its memory is free.
/// Read from the daemon's minute look at the machine (`crate::machine_load`),
/// which past its line holds new work back altogether.
pub fn machine_cap(base: usize, load1: Option<f64>, cpus: usize, free_memory: Option<f64>) -> usize {
    let base = base.max(1);
    if free_memory.is_some_and(|f| f < 0.10) {
        return 1;
    }
    let Some(load) = load1.filter(|_| cpus > 0).map(|l| l / cpus as f64) else { return base };
    if load >= 2.0 {
        1
    } else if load >= 1.5 {
        base.div_ceil(2).max(1)
    } else if load >= 1.0 {
        base.saturating_sub(1).max(1)
    } else {
        base
    }
}

/// The places a ticket's words name, as written (`src/app/x.tsx`, `design/`).
pub fn paths(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .split(|c: char| c.is_whitespace() || matches!(c, '`' | '(' | ')' | '[' | ']' | ',' | ';' | '"' | '\'' | '“' | '”'))
        .map(|w| w.trim_end_matches(['.', ':', '!', '?']))
        .filter(|w| w.contains('/') && !w.contains("://") && w.chars().any(char::is_alphabetic) && !w.starts_with('/'))
        .map(|w| w.trim_start_matches("./").trim_end_matches('/').to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Whether two tickets' places overlap: one is, or is inside, the other.
/// Nothing named on either side: they don't, as far as can be told.
pub fn overlaps(a: &[String], b: &[String]) -> bool {
    let inside = |x: &str, y: &str| x == y || x.starts_with(&format!("{y}/"));
    a.iter().any(|x| b.iter().any(|y| inside(x, y) || inside(y, x)))
}

/// Whether a member with `running` runs starts no new ticket of its own
/// role now, so the queue finishes before it starts: the ready queue is
/// longer than twice what the squad runs at once, and it holds work under
/// way for it — a baton passed, work sent back — that comes first.
pub fn prefer_finishing(ready: usize, capacity: usize, running: usize) -> bool {
    running > 0 && ready > capacity.saturating_mul(2)
}

#[cfg(test)]
mod tests {

    #[test]
    fn checklist_items_are_named_by_number_or_by_their_words_and_a_wrong_one_is_shown_the_call() {
        use equipe_protocol::{TicketFlow, TicketStep};
        use serde_json::json;
        let item = |t: &str| TicketStep { text: t.into(), who: None, done: false, by: None, ..Default::default() };
        let flow = TicketFlow { steps: vec![item("Build the form"), item("Write the tests.")], ..Default::default() };
        let named = |v: serde_json::Value| super::item_numbers(&flow, 4, super::CHECK_STEPS, "done", Some(&v));
        assert_eq!(named(json!([2, 1])), Ok(vec![2, 1]));
        assert_eq!(named(json!("2")), Ok(vec![2]));
        assert_eq!(named(json!("1, 2")), Ok(vec![1, 2]));
        assert_eq!(named(json!("[2]")), Ok(vec![2]));
        assert_eq!(named(json!(["write the tests"])), Ok(vec![2]));
        assert_eq!(named(json!("Build the form")), Ok(vec![1]));
        let wrong = named(json!(["Ship it"])).unwrap_err();
        assert!(wrong.contains("1. [ ] Build the form") && wrong.contains(r#"check_steps({"done":[1]})"#), "{wrong}");
        let help = super::check_steps_help(4, &flow, None);
        assert!(help.contains(r#"check_steps({"done":[1]})"#) && help.contains(r#"check_steps({"add":["Write the tests"]})"#), "{help}");
        let empty = super::check_steps_help(4, &TicketFlow::default(), None);
        assert!(empty.contains("no checklist yet") && empty.contains("check_steps({\"add\":"), "{empty}");
        // One line on its own is a list of one.
        assert_eq!(super::steps_arg(&json!({ "add": "Check it at 390px" }), "add").unwrap().len(), 1);
    }

    #[test]
    fn a_baton_passed_back_and_forth_past_its_limit_is_a_loop_and_a_hand_in_resets_it() {
        let mut kinds = vec!["created", "assigned"];
        kinds.extend(std::iter::repeat_n(crate::lead_time::BATON, MAX_BATONS - 1));
        assert_eq!(batons_since_hand_in(kinds.iter().copied()), MAX_BATONS - 1);
        assert!(baton_loop(7, MAX_BATONS - 1).is_none(), "under the limit it passes");
        kinds.push(crate::lead_time::BATON);
        let passes = batons_since_hand_in(kinds.iter().copied());
        let why = baton_loop(7, passes).expect("at the limit it stops");
        assert!(why.contains("submit_for_review") && why.contains("T-7"), "{why}");
        // Handed in and sent back: the count starts again.
        kinds.extend(["handed in", "sent back", crate::lead_time::BATON]);
        assert_eq!(batons_since_hand_in(kinds.iter().copied()), 1);
    }

    #[test]
    fn a_checklist_stops_growing_past_its_limit() {
        assert!(steps_overflow(3, MAX_STEPS - 1, 1).is_none());
        assert!(steps_overflow(3, MAX_STEPS, 0).is_none(), "ticking is always fine");
        assert!(steps_overflow(3, MAX_STEPS, 1).is_some_and(|w| w.contains("T-3")));
    }
    use super::*;
    use crate::i18n::{Lang, core};
    use equipe_protocol::TicketStatus;
    use serde_json::json;

    fn ticket(id: TicketId, title: &str) -> Ticket {
        Ticket { summary: None,
            id,
            title: title.into(),
            body: String::new(),
            status: TicketStatus::Review,
            kind: TicketKind::Code,
            label: None,
            owner: Some("frontend".into()),
            assignee: Some("frontend".into()),
            project: None,
            depends_on: vec![],
            branch: Some("equipe/frontend-1".into()),
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            landing: false,
            created_by: "designer".into(),
            note: None,
            asks: vec![],
            remaining: vec![],
            images: vec![],
            priority: Default::default(),
            rank: 0,
            updated_at: None,
            started_at: None,
            app: None,
            design_qa: None,
            plan: None,
            model: None,
            flow: None,
        }
    }

    #[test]
    fn a_designer_s_ui_change_is_one_slice_with_prototype_build_and_precheck() {
        assert!(is_slice(TicketKind::Code, Some(TicketLabel::Design), DESIGNER, None));
        assert!(is_slice(TicketKind::Code, Some(TicketLabel::Feature), "pm", Some("designer-2")));
        assert!(!is_slice(TicketKind::Code, Some(TicketLabel::Check), DESIGNER, None));
        assert!(!is_slice(TicketKind::Task, Some(TicketLabel::Design), DESIGNER, None));
        assert!(!is_slice(TicketKind::Code, Some(TicketLabel::Fix), "backend", Some("backend")));
        let words = core(Lang::En);
        let steps = default_steps(true, TicketKind::Code, Some("designer"), words);
        assert_eq!(steps.iter().map(|s| s.who.as_deref()).collect::<Vec<_>>(), vec![Some(DESIGNER), Some(FRONTEND), None]);
        assert!(steps.iter().all(|s| !s.text.is_empty() && !s.text.starts_with("flow.")));
        assert_eq!(default_steps(false, TicketKind::Code, Some("backend-2"), words)[0].who.as_deref(), Some("backend"));
        assert!(default_steps(false, TicketKind::Task, Some("pm"), words).is_empty());
    }

    #[test]
    fn the_baton_moves_through_the_checklist() {
        let words = core(Lang::En);
        let mut flow = TicketFlow { driver: Some("designer".into()), steps: default_steps(true, TicketKind::Code, None, words), slice: true, ..Default::default() };
        assert_eq!(next_holder(&flow, "designer").as_deref(), Some(FRONTEND));
        assert_eq!(left_for(&flow, "designer-2").len(), 2, "its own item and the one nobody is named on");
        assert_eq!(tick(&mut flow, &[1], &[], vec![], "designer").unwrap(), "1 of 3 done.");
        assert!(tick(&mut flow, &[4], &[], vec![], "designer").is_err());
        let added = steps_arg(&json!({ "add": ["Empty state", { "text": "Check on a phone", "who": "qa" }] }), "add").unwrap();
        assert_eq!(tick(&mut flow, &[], &[], added, "frontend").unwrap(), "1 of 5 done. 2 added.");
        let text = checklist(&flow);
        assert!(text.starts_with("1. [x] "), "{text}");
        assert!(text.contains("(by designer)"));
        assert!(text.contains("5. [ ] Check on a phone — qa"));
        let told = brief(&flow, "designer", 7);
        assert!(told.contains("designer drives it") && told.contains("pass_baton") && told.contains("(to frontend)"), "{told}");
        let picked = picked_up(&baton_note("designer", "Prototype in design/sessions/x/"), Some(&flow), "frontend-2");
        assert!(picked.starts_with("Baton from designer: Prototype in design/sessions/x/"));
        assert!(picked.contains("check_steps"));
    }

    #[test]
    fn a_re_plan_replaces_items_and_never_repeats_one() {
        let steps = |texts: &[&str]| texts.iter().map(|t| step(t, None)).collect::<Vec<_>>();
        let mut flow = TicketFlow { steps: steps(&["Pick the logo", "Put it in the header", "실제 주소에 올리기"]), ..Default::default() };
        tick(&mut flow, &[1], &[], vec![], "designer").unwrap();
        // The frontend lists its own plan: the item already there isn't added again.
        let said = tick(&mut flow, &[], &[], steps(&["Put it in the header.", "Add the favicon", "실제 주소에  올리기"]), "frontend").unwrap();
        assert_eq!(said, "1 of 4 done. 1 added. 2 already on it, not added again.");
        assert_eq!(flow.steps.iter().filter(|s| s.text.starts_with("실제")).count(), 1);
        // A re-plan names the open items its new ones take the place of.
        let said = tick(&mut flow, &[], &[2, 4], steps(&["Header and app icon"]), "frontend").unwrap();
        assert_eq!(said, "1 of 3 done. 1 added. 2 replaced.");
        assert_eq!(flow.steps.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["Pick the logo", "실제 주소에 올리기", "Header and app icon"]);
        assert!(tick(&mut flow, &[], &[1], vec![], "frontend").unwrap_err().contains("done — it stays"));
        assert!(tick(&mut flow, &[2], &[2], vec![], "frontend").is_err());
        assert!(tick(&mut flow, &[], &[9], vec![], "frontend").is_err());
    }

    #[test]
    fn a_baton_needs_someone_else_and_never_leaves_an_action() {
        let t = ticket(3, "Input box");
        assert!(baton_refusal(&t, "designer", "frontend").is_none());
        assert!(baton_refusal(&t, "designer", "designer").is_some());
        assert!(baton_refusal(&t, "designer", " ").is_some());
        let action = Ticket { kind: TicketKind::Action, ..t };
        assert!(baton_refusal(&action, "qa", "frontend").is_some());
    }

    #[test]
    fn checks_of_work_under_way_go_into_its_checklist_not_new_tickets() {
        let open = [10, 11];
        let check = Asked { title: "Verify the composer on phones", label: Some(TicketLabel::Check), ..Default::default() };
        // On a ticket: into its checklist.
        let working = Maker { role: "designer", working: Some(10), ..Default::default() };
        let said = meta_refusal(&check, &working, &open).unwrap();
        assert!(said.contains("T-10's checklist") && said.contains(CHECK_STEPS), "{said}");
        // Polish found while working: the same.
        let polish = Asked { title: "Tighten the spacing", label: Some(TicketLabel::Design), severity: Some("s3"), ..Default::default() };
        assert!(meta_refusal(&polish, &working, &open).is_some());
        // S1/S2 is always a ticket.
        let broken = Asked { severity: Some("S2"), ..polish.clone() };
        assert!(meta_refusal(&broken, &working, &open).is_none());
        // The PM, at its desk, checking an open ticket: into that one's checklist.
        let desk = Maker { role: "pm", pm: true, ..Default::default() };
        let of_11 = Asked { title: "Check T-11 on Safari", label: Some(TicketLabel::Check), ..Default::default() };
        assert!(meta_refusal(&of_11, &desk, &open).unwrap().contains("update_ticket"));
        let depends = [10];
        let after_10 = Asked { title: "QA pass", label: Some(TicketLabel::Check), depends_on: &depends, ..Default::default() };
        assert!(meta_refusal(&after_10, &desk, &open).is_some());
        // A check of nothing under way stands alone; an idea is never weighed.
        let baseline = Asked { title: "Measure the site's speed baseline", label: Some(TicketLabel::Check), ..Default::default() };
        assert!(meta_refusal(&baseline, &desk, &open).is_none());
        assert!(meta_refusal(&Asked { idea: true, ..check }, &working, &open).is_none());
    }

    #[test]
    fn the_next_part_of_a_request_is_the_same_ticket_s_baton_unless_it_stands_alone() {
        let build = Asked { title: "Build the new input box", label: Some(TicketLabel::Feature), ..Default::default() };
        let on_request = Maker { role: "designer", working: Some(5), working_serves_request: true, same_request: true, ..Default::default() };
        let said = meta_refusal(&build, &on_request, &[]).unwrap();
        assert!(said.contains(PASS_BATON) && said.contains("separate_why"), "{said}");
        let separate = Asked { separate_why: Some("A backend endpoint in another repository"), ..build.clone() };
        assert!(meta_refusal(&separate, &on_request, &[]).is_none());
        // Not on a request's ticket: as before.
        let plain = Maker { role: "designer", working: Some(5), ..Default::default() };
        assert!(meta_refusal(&build, &plain, &[]).is_none());
    }

    #[test]
    fn members_land_their_own_work_unless_a_person_must_say_yes() {
        let t = ticket(4, "Tidy the composer's toolbar");
        assert_eq!(landing(true, true, &t, &["src/composer.tsx".into()], false), Landing::Member);
        assert_eq!(landing(false, true, &t, &[], false), Landing::Pm(Held::Off));
        assert_eq!(landing(true, false, &t, &[], false), Landing::Pm(Held::PersonAccepts));
        assert_eq!(landing(true, true, &t, &[], true), Landing::Pm(Held::PmsOwn));
        assert_eq!(landing(true, true, &t, &["db/migrations/0042_roles.sql".into()], false), Landing::Pm(Held::Gated(crate::person_gate::Why::Data)));
        // Read from its SQL: a migration that only adds lands like code; one that drops waits.
        let sql = |text: &str| std::collections::BTreeMap::from([("db/migrations/0042_roles.sql".to_owned(), text.to_owned())]);
        let migration: Vec<String> = vec!["db/migrations/0042_roles.sql".into(), "src/db/roles.ts".into()];
        assert_eq!(landing_with(true, true, &t, &migration, &sql("CREATE TABLE roles (id int);"), false), Landing::Member);
        assert_eq!(landing_with(true, true, &t, &migration, &sql("DROP TABLE roles;"), false), Landing::Pm(Held::Gated(crate::person_gate::Why::Data)));
        assert_eq!(landing(true, true, &t, &["src/auth/session.ts".into()], false), Landing::Pm(Held::Gated(crate::person_gate::Why::Security)));
        let deploy = Ticket { label: Some(TicketLabel::Deploy), ..t.clone() };
        assert_eq!(landing(true, true, &deploy, &[], false), Landing::Pm(Held::Gated(crate::person_gate::Why::Irreversible)));
        let many: Vec<String> = (0..13).map(|i| format!("src/screens/s{i}.tsx")).collect();
        assert_eq!(landing(true, true, &t, &many, false), Landing::Pm(Held::Gated(crate::person_gate::Why::Visible)));
        let said = digest(&[(4, "Tidy the toolbar".into(), "frontend".into(), "The toolbar fits on phones.".into())]);
        assert!(said.contains("T-4 Tidy the toolbar — frontend: The toolbar fits on phones.") && said.contains(REVERT_LANDED));
    }

    #[test]
    fn a_hand_in_or_a_baton_ticks_its_holders_items_and_a_turn_that_ticked_nothing_is_reminded() {
        // Team Aman, 2026-11-28: Kai built 4 of the 8 steps and ticked none; the person saw "Steps 0 of 8".
        let item = |t: &str, who: Option<&str>| TicketStep { text: t.into(), who: who.map(str::to_owned), ..Default::default() };
        let mut flow = TicketFlow {
            steps: vec![item("Spec the page", Some("designer")), item("Build the page", None), item("Wire the booking", None), item("Check on phones", Some("qa"))],
            ..Default::default()
        };
        assert_eq!(open_for(&flow, "frontend"), vec![2, 3], "its own and anyone's, not the designer's or QA's");
        assert!(ticked_nothing(&flow, "frontend", 0));
        let open = open_for(&flow, "frontend");
        let said = tick(&mut flow, &open, &[], Vec::new(), "frontend").unwrap();
        assert!(said.starts_with("2 of 4 done"), "{said}");
        assert!(flow.steps[1].done_at.is_some() && flow.steps[0].done_at.is_none());
        assert!(!ticked_nothing(&flow, "frontend", 0), "nothing left for it");
        assert!(ticked_nothing(&flow, "designer", u64::MAX), "the designer's item is open and nothing was ticked since");
        let reminder = tick_reminder(9, &flow, "designer");
        assert!(reminder.contains("T-9") && reminder.contains("1. Spec the page") && reminder.contains("`check_steps`") && reminder.contains("`note`"), "{reminder}");
    }

    /// Team Aman's T-65 as it was on 2026-11-28 (its steps' own words).
    fn t65() -> TicketFlow {
        let item = |t: &str, who: Option<&str>| TicketStep { text: t.into(), who: who.map(str::to_owned), ..Default::default() };
        TicketFlow {
            steps: vec![
                item("Rebuild the screens from the person's own design, wired to the real data", None),
                item("Add a create-meeting form in the same style", None),
                item("Add a light option, calm motion for people who ask for less, readable text", None),
                item("Compare each screen with the design, side by side, on phone and computer", Some("designer")),
                item("Put it live", None),
                item("Turn the stars gold when a meeting is locked, as in the demo", None),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn what_a_run_showed_ticks_its_checklist_without_check_steps() {
        // T-65 ran twice (89 and 103 tool calls), deployed its preview 4 times, passed QA 3 times, never called
        // check_steps, and was handed in with 0 of 6 ticked.
        let mut flow = t65();
        let qa_pass = r#"{"ok":true,"checks":[{"id":"loads","ok":true}],"url":"https://review--asap.example.app"}"#;
        let qa_fail = r#"{"ok":false,"checks":[{"id":"loads","ok":false}],"next":"Fix every failed check, deploy again and run qa_check again. The work is not done until it passes."}"#;
        let preview = r#"{"app":"asap","url":"https://review--asap.example.app","preview":true}"#;
        let live = r#"{"app":"asap","url":"https://asap.example.app","version":"v12"}"#;
        assert_eq!(evidence_of("mcp__equipe-platform__qa_check", true, qa_pass), Some(Evidence::Passed));
        assert_eq!(evidence_of("mcp__equipe-platform__qa_check", true, &format!("{{\"text\":{}}}", serde_json::to_string(qa_pass).unwrap())), Some(Evidence::Passed), "JSON inside a text");
        assert_eq!(evidence_of("mcp__equipe-platform__qa_check", true, qa_fail), None, "a failed check shows nothing");
        assert_eq!(evidence_of("mcp__equipe-platform__deploy_site", true, preview), Some(Evidence::Preview));
        assert_eq!(evidence_of("mcp__equipe-platform__deploy_site", true, live), Some(Evidence::Live));
        assert_eq!(evidence_of("mcp__equipe-platform__deploy_site", true, "refused: this build is still the starter screen"), None, "a refusal is no deploy");
        assert_eq!(evidence_of("mcp__equipe-platform__deploy_site", false, live), None, "a failed call shows nothing");
        assert_eq!(evidence_of("Bash", true, live), None, "only the platform's tools");
        assert_eq!(evidence_of("mcp__equipe-platform__deploy_main", true, r#"{"deployed":[{"app":"asap"}],"skipped":[]}"#), Some(Evidence::Live));
        assert_eq!(evidence_of("mcp__equipe-platform__deploy_main", true, r#"{"deployed":[],"skipped":[{"app":"asap","why":"no recorded commit"}]}"#), None, "nothing went live");

        // A preview while the work goes on ticks nothing; the check passing ticks the holder's items short of going live.
        assert!(ticked_by(&flow, "frontend", Evidence::Preview).is_empty());
        let passed = ticked_by(&flow, "frontend", Evidence::Passed);
        assert_eq!(passed, vec![1, 2, 3, 6], "not the designer's item, not going live");
        tick(&mut flow, &passed, &[], Vec::new(), "frontend").unwrap();
        assert_eq!(flow.progress(), (4, 6));
        // Going live ticks the go-live item, whoever holds it.
        assert_eq!(ticked_by(&flow, "equipe", Evidence::Live), vec![5]);
    }

    #[test]
    fn a_hand_in_ticks_what_its_run_showed_and_marks_the_rest_as_on_its_word() {
        let flow = t65();
        // Built and checked: every item of the holder's is confirmed but going live, which waits for the deploy.
        assert_eq!(hand_in_ticks(&flow, "frontend", &[Evidence::Preview, Evidence::Passed]), (vec![1, 2, 3, 6], vec![]));
        assert_eq!(hand_in_ticks(&flow, "frontend", &[Evidence::Live]), (vec![1, 2, 3, 5, 6], vec![]), "it went live in the run");
        // Nothing shown: ticked on its word, never going live.
        assert_eq!(hand_in_ticks(&flow, "frontend", &[]), (vec![], vec![1, 2, 3, 6]));
        // An explicit tick is the member's own and clears the mark.
        let mut flow = flow;
        flow.steps[0].done = true;
        flow.steps[0].unconfirmed = true;
        tick(&mut flow, &[1], &[], Vec::new(), "frontend").unwrap();
        assert!(!flow.steps[0].unconfirmed);
    }

    #[test]
    fn going_live_is_told_by_its_words_in_every_language() {
        for live in ["Put it live", "Put asap live on its free address", "Publish the new menu", "Mettre en ligne la page", "실제 사이트에 올리기", "라이브로 내보내기", "本番に公開する", "上线新版本"] {
            assert!(is_live_step(live), "{live}");
        }
        for not in ["Deliver the booking form", "Make the dashboard feel alive", "Add a light option", "Olive green buttons", "Tests and checks pass"] {
            assert!(!is_live_step(not), "{not}");
        }
    }

    #[test]
    fn a_revised_plan_says_each_step_once() {
        let item = |t: &str, done: bool| TicketStep { text: t.into(), done, ..Default::default() };
        let mut flow = TicketFlow { steps: vec![item("Build the form", false), item("Write the tests", true), item("build the form.", true), item("Write the tests", false)], ..Default::default() };
        assert_eq!(merge_repeats(&mut flow), 2);
        let steps: Vec<(&str, bool)> = flow.steps.iter().map(|s| (s.text.as_str(), s.done)).collect();
        assert_eq!(steps, [("Build the form", true), ("Write the tests", true)], "done if either was");
    }

    #[test]
    fn work_promised_to_the_person_first_waits_for_their_yes() {
        // inhwadoen, 2026-11-28: "I'll apply it once you confirm", and it was live twelve minutes later.
        let t = ticket(36, "Apply look A and the shutter dial to the site");
        let held = Ticket { body: with_hold("Apply A to the home page.", true), ..t.clone() };
        assert!(held_for_person(&held.body) && held.body.starts_with("Apply A to the home page."), "{}", held.body);
        assert_eq!(landing(true, true, &held, &["src/home.tsx".into()], false), Landing::Pm(Held::PersonConfirms));
        assert_eq!(landing(true, true, &held, &[], true), Landing::Pm(Held::PersonConfirms), "the PM's own too");
        assert_eq!(landing(true, false, &held, &[], false), Landing::Pm(Held::PersonAccepts), "a squad where the person accepts already waits for them");
        let lifted = Ticket { body: with_hold(&held.body, false), ..held.clone() };
        assert_eq!(lifted.body, "Apply A to the home page.");
        assert_eq!(landing(true, true, &lifted, &["src/home.tsx".into()], false), Landing::Member);
        assert_eq!(with_hold(&with_hold("x", true), true), with_hold("x", true), "held once");
        assert_eq!(with_hold("", true), HOLD_LINE);
        // Their yes, quoted as they wrote it, in what they said lately.
        let said = vec!["시안 다 봤어".to_owned(), "좋아, A안으로 반영해줘!".to_owned()];
        assert!(yes_heard("A안으로 반영해줘", &said));
        assert!(yes_heard("“좋아, A안으로  반영해줘!”", &said));
        assert!(!yes_heard("네, 반영해 주세요", &said), "words they never wrote");
        assert!(!yes_heard("", &said) && !yes_heard("좋", &said));
    }

    #[test]
    fn lanes_ease_off_on_a_loaded_machine() {
        assert_eq!(machine_cap(3, Some(4.0), 8, Some(0.5)), 3);
        assert_eq!(machine_cap(3, Some(8.0), 8, None), 2);
        assert_eq!(machine_cap(4, Some(12.0), 8, None), 2);
        assert_eq!(machine_cap(3, Some(60.0), 10, None), 1, "the owner's Mac at load 60 on 10 cores");
        assert_eq!(machine_cap(3, None, 8, Some(0.05)), 1);
        assert_eq!(machine_cap(0, None, 8, None), 1);
        assert_eq!(machine_cap(6, None, 0, None), 6);
    }

    #[test]
    fn lanes_of_one_member_never_share_places() {
        let a = paths("Fix `src/app/composer.tsx` and src/app/menu.tsx.");
        assert_eq!(a, vec!["src/app/composer.tsx", "src/app/menu.tsx"]);
        assert!(overlaps(&a, &paths("Restyle src/app/")));
        assert!(!overlaps(&a, &paths("Change server/api/routes.rs")));
        assert!(!overlaps(&a, &[]));
        assert!(!overlaps(&paths("See https://example.com/a/b"), &a));
        assert!(prefer_finishing(40, 12, 2));
        assert!(!prefer_finishing(40, 12, 0));
        assert!(!prefer_finishing(10, 12, 2));
    }
}
