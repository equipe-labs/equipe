//! What waits for the person, in sight above the composer (T-62 §2.4): the
//! oldest request for permission with its Allow once / Always / Decline, and
//! under every conversation the card of work waiting for them. The queue and
//! the card's text are plain code, tested without a window; the drawing is
//! below them.

use equipe_core::t;
use equipe_core::protocol::{Approval, Command, Decision, SessionId, Ticket, TicketId, TicketStatus};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::{MONO, Palette, SMALL, look};
use crate::{Screen, Shell};

/// How many tickets the for-you card names before it counts the rest.
const CARD_SHOWN: usize = 3;

// ── the queue ──

/// Open requests for permission, oldest first. The oldest is the one the keys
/// answer. An answer given hides its request at once, so a second key press
/// cannot answer it twice; the daemon's `ApprovalResolved` takes it for good.
#[derive(Debug, Clone, Default)]
pub struct Queue {
    items: Vec<Approval>,
    answered: Vec<u64>,
}

impl Queue {
    /// The snapshot's requests replace everything.
    pub fn load(&mut self, items: Vec<Approval>) {
        self.items = items;
        self.answered.clear();
    }

    pub fn has(&self, id: u64) -> bool {
        self.items.iter().any(|a| a.id == id)
    }

    pub fn push(&mut self, approval: Approval) {
        if !self.has(approval.id) {
            self.items.push(approval);
        }
    }

    pub fn resolve(&mut self, id: u64) {
        self.items.retain(|a| a.id != id);
        self.answered.retain(|a| *a != id);
    }

    /// Answers that went into a dead link never come back: show them again.
    pub fn forget_answers(&mut self) {
        self.answered.clear();
    }

    fn open(&self) -> impl Iterator<Item = &Approval> {
        self.items.iter().filter(|a| !self.answered.contains(&a.id))
    }

    pub fn first(&self) -> Option<&Approval> {
        self.open().next()
    }

    pub fn first_for(&self, session: SessionId) -> Option<&Approval> {
        self.open().find(|a| a.session == session)
    }

    /// How many wait, the one on show included.
    pub fn len(&self) -> usize {
        self.open().count()
    }

    pub fn ids(&self) -> Vec<u64> {
        self.open().map(|a| a.id).collect()
    }

    /// Answer the oldest request: the command to send. `Always` only where it
    /// can become a rule; elsewhere it is not an answer at all.
    pub fn answer(&mut self, squad: &str, decision: Decision) -> Option<Command> {
        let approval = self.first()?;
        if decision == Decision::Always && always_rule(approval).is_none() {
            return None;
        }
        let id = approval.id;
        self.answered.push(id);
        Some(Command::Decide { squad: squad.to_owned(), approval: id, decision })
    }
}

/// The answer a key gives, as in the terminal console: y, a, n.
pub fn decision_for(key: &str) -> Option<Decision> {
    match key {
        "y" => Some(Decision::Allow),
        "a" => Some(Decision::Always),
        "n" => Some(Decision::Deny),
        _ => None,
    }
}

// ── the card's words ──

/// What kind of thing is asked, as a noun phrase.
pub fn noun(tool: &str) -> &'static str {
    match tool {
        "Bash" => t!("approvals.noun_bash"),
        "Write" | "Edit" | "MultiEdit" => t!("approvals.noun_edit"),
        "WebFetch" => t!("approvals.noun_web_fetch"),
        equipe_core::desktop_gate::SEND => t!("approvals.noun_send"),
        equipe_core::permissions::BROWSER_CODE => t!("approvals.noun_browser_code"),
        _ => t!("approvals.noun_other"),
    }
}

/// The rule an "always" answer would add, if there can be one.
fn always_rule(approval: &Approval) -> Option<String> {
    equipe_core::permissions::rule_for(&approval.tool, &approval.action)
}

/// What an "always" answer would allow from now on, in short: `npm install`,
/// `docs.rs`, a folder.
pub fn always_label(rule: &str) -> String {
    let inner = rule.split_once('(').and_then(|(_, r)| r.strip_suffix(')')).unwrap_or(rule);
    let short = inner.trim_end_matches(":*").trim_start_matches("domain:");
    match short.strip_prefix('/').and_then(|p| p.strip_suffix("/**")) {
        Some(dir) => format!("/{dir}/"),
        None => short.to_owned(),
    }
}

/// The files a shell script writes with `cat > file`, `cat >> file` or `tee file`.
pub fn files_written(script: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in script.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            let target = match *word {
                ">" | ">>" if i > 0 && words[i - 1] == "cat" => words.get(i + 1),
                "tee" | "-a" if i > 0 && (words[i - 1] == "|" || words[i - 1] == "tee") => words.get(i + 1),
                _ => None,
            };
            if let Some(file) = target.filter(|f| !f.starts_with('<') && !f.starts_with('-')) {
                let file = file.trim_matches(['\'', '"']).to_owned();
                if !files.contains(&file) {
                    files.push(file);
                }
            }
        }
    }
    files
}

/// What the card shows of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub noun: &'static str,
    /// The start of the action: 12 lines of browser code, 8 of a message, 3 otherwise.
    pub lines: Vec<String>,
    pub more: usize,
    pub writes: Vec<String>,
    pub always: Option<String>,
    pub is_send: bool,
}

impl Card {
    pub fn of(approval: &Approval) -> Self {
        let shown = match approval.tool.as_str() {
            equipe_core::permissions::BROWSER_CODE => 12,
            equipe_core::desktop_gate::SEND => 8,
            _ => 3,
        };
        let all: Vec<&str> = approval.action.lines().collect();
        Self {
            noun: noun(&approval.tool),
            lines: all.iter().take(shown).map(|l| (*l).to_owned()).collect(),
            more: all.len().saturating_sub(shown),
            writes: files_written(&approval.action),
            always: always_rule(approval).map(|r| always_label(&r)),
            is_send: approval.tool == equipe_core::desktop_gate::SEND,
        }
    }
}

// ── the for-you card ──

/// What a ticket on the card asks of the person, right-aligned.
pub fn ask_of(ticket: &Ticket) -> String {
    match ticket.status {
        TicketStatus::Waiting => {
            let needs = ticket.note.as_deref().map(equipe_core::board::waiting_needs).unwrap_or_default();
            if needs.is_empty() { t!("approvals.ask_todo").into() } else { t!("approvals.ask_needs", needs = needs.join(", ")) }
        }
        _ => t!("approvals.ask_review", who = ticket.assignee.as_deref().or(ticket.owner.as_deref()).unwrap_or("—")),
    }
}

// ── drawing ──

impl Shell {
    /// Answer the oldest request.
    pub fn decide(&mut self, decision: Decision, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        if let Some(command) = self.model.approvals.answer(&squad, decision) {
            self.send_command(command);
            cx.notify();
        }
    }

    /// Open the screen a send would go from.
    pub fn open_preview(&self) {
        if let Some(path) = self.model.approvals.first().and_then(|a| a.preview.as_deref()) {
            self.open_file(path);
        }
    }

    /// The keys that answer: only on the conversation, with nothing typed — on
    /// the board the same letters will act on tickets, and an "a" meant to
    /// accept must never allow a command for good. True when the key was taken.
    pub fn on_approval_key(&mut self, key: &str, plain: bool, cx: &mut Context<Self>) -> bool {
        if !plain || self.screen != Screen::Chat || !self.composer.text(cx).is_empty() || self.model.approvals.first().is_none() {
            return false;
        }
        if let Some(decision) = decision_for(key) {
            self.decide(decision, cx);
            return true;
        }
        if key == "o" && self.model.approvals.first().is_some_and(|a| a.preview.is_some()) {
            self.open_preview();
            return true;
        }
        false
    }
}

fn button(p: Palette, id: &'static str, label: &'static str, key: &'static str, primary: bool, on: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
    h_flex()
        .id(id)
        .h(px(30.))
        .px_3()
        .gap_2()
        .items_center()
        .rounded(px(6.))
        .text_size(px(SMALL))
        .cursor_pointer()
        .when(primary, |b| b.bg(p.primary).text_color(p.on_primary).font_weight(FontWeight::SEMIBOLD))
        .when(!primary, |b| b.border_1().border_color(p.hair).hover(move |s| s.bg(p.wash)))
        .on_click(move |_, window, cx| on(window, cx))
        .child(label)
        .child(div().text_size(px(12.)).opacity(0.6).child(key))
}

/// The oldest request, drawn as a card.
pub fn approval(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let approval = model.approvals.first()?;
    let card = Card::of(approval);
    let (mark, hue) = look(
        model.members.iter().find(|m| m.session == approval.session).map_or("", |m| m.role.as_str()),
        &approval.member,
    );
    let waiting = model.approvals.len();
    let on_chat = shell.screen == Screen::Chat;
    let handle = cx.entity().downgrade();
    let for_decisions = handle.clone();
    let act = move |d: Decision| {
        let handle = for_decisions.clone();
        move |_: &mut Window, cx: &mut App| {
            handle.update(cx, |shell, cx| shell.decide(d, cx)).ok();
        }
    };
    let preview = approval.preview.clone();
    Some(
        v_flex()
            .w_full()
            .mb_3()
            .p_4()
            .gap_2()
            .rounded(px(12.))
            .bg(p.raised)
            .border_1()
            .border_l_2()
            .border_color(p.hair)
            .child(
                h_flex()
                    .gap_2()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().text_color(p.accent).child(t!("approvals.waiting_on_you_mark")))
                    .child(div().text_color(p.member(hue)).child(format!("{mark} {}", approval.member)))
                    .children(approval.ticket.map(|t| div().text_color(p.muted).child(format!("T-{t}"))))
                    .child(div().flex_1())
                    .when(waiting > 1, |h| h.child(div().font_weight(FontWeight::NORMAL).text_color(p.muted).child(t!("approvals.queue", waiting = waiting))))
                    .when(waiting == 1 && on_chat, |h| h.child(div().font_weight(FontWeight::NORMAL).text_color(p.muted).child("Y / A / N"))),
            )
            .child(div().text_size(px(15.)).text_color(p.text).child(t!("approvals.allow_question", what = card.noun)))
            .child(
                v_flex()
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .bg(p.code_bg)
                    .border_1()
                    .border_color(p.hair)
                    .font_family(MONO)
                    .text_size(px(12.5))
                    .line_height(px(20.))
                    .children(card.lines.iter().map(|l| div().child(l.clone())))
                    .when(card.more > 0, |c| c.child(div().text_color(p.muted).child(t!("approvals.more_lines", n = card.more)))),
            )
            .when(!card.writes.is_empty(), |c| c.child(div().text_size(px(SMALL)).text_color(p.muted).child(t!("approvals.writes", files = card.writes.join(", ")))))
            .children(approval.reason.clone().map(|r| div().text_size(px(SMALL)).text_color(p.muted).child(r)))
            .children(preview.map(|path| {
                h_flex().gap_3().items_center().text_size(px(SMALL)).text_color(p.muted).child(div().min_w_0().child(t!("approvals.screen", path = path))).child(button(
                    p,
                    "preview",
                    t!("approvals.open_preview"),
                    "O",
                    false,
                    {
                        let handle = handle.clone();
                        move |_, cx| {
                            handle.update(cx, |shell, _| shell.open_file(&path)).ok();
                        }
                    },
                ))
            }))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(button(p, "allow", t!("approvals.allow_once"), "Y", true, act(Decision::Allow)))
                    .when(card.always.is_some(), |h| h.child(button(p, "always", t!("approvals.always"), "A", false, act(Decision::Always))))
                    .child(button(p, "deny", t!("approvals.deny"), "N", false, act(Decision::Deny)))
                    .child(div().flex_1())
                    .child(div().text_size(px(12.)).text_color(p.muted).child(match &card.always {
                        Some(label) => t!("approvals.always_rule", label = label),
                        None if card.is_send => t!("approvals.send_always_asks").to_owned(),
                        None => t!("approvals.once_only").to_owned(),
                    })),
            )
            .when(!on_chat, |c| c.child(div().text_size(px(12.)).text_color(p.muted).child(t!("approvals.board_keys_hint"))))
            .into_any_element(),
    )
}

/// The card of what waits for the person, under any conversation: at most
/// three rows, each a click from its ticket on the board.
pub fn for_you(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let waiting = model.for_you_here();
    if waiting.is_empty() {
        return None;
    }
    let all_reviews = waiting.iter().all(|t| t.status == TicketStatus::Review);
    let heading = if all_reviews { t!("approvals.heading_reviews") } else { t!("approvals.heading_for_you") };
    let rows = waiting.iter().take(CARD_SHOWN).map(|t| {
        let id = t.id;
        let is_wait = t.status == TicketStatus::Waiting;
        let project = model.ticket_project_name(t);
        let hue = project.as_ref().and_then(|n| model.project_names().iter().position(|x| x == n)).map(|ix| p.member(crate::theme::PROJECT_HUES[ix % 8]));
        h_flex()
            .id(("for-you", id as u64))
            .h(px(32.))
            .px_3()
            .gap_3()
            .items_center()
            .text_size(px(SMALL))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .on_click(cx.listener(move |this, _, _, cx| this.open_ticket(id, cx)))
            .child(div().w(px(48.)).flex_none().font_family(MONO).text_color(p.accent).child(format!("T-{id}")))
            // One line each, cut short with "…": a row is 32 high, and a long
            // title or ask must never spill onto the rows below.
            .child(div().w(px(96.)).flex_none().whitespace_nowrap().overflow_hidden().text_ellipsis().text_color(hue.unwrap_or(p.muted)).child(project.unwrap_or_default()))
            .child(div().flex_1().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(t.title.clone()))
            .child(div().flex_shrink_0().max_w(px(260.)).whitespace_nowrap().overflow_hidden().text_ellipsis().text_color(p.muted).child(ask_of(t)))
            .when(is_wait, |r| r.child(resume(shell, p, id, cx)))
    });
    let more = waiting.len().saturating_sub(CARD_SHOWN);
    Some(
        v_flex()
            .w_full()
            .mb_3()
            .py_2()
            .rounded(px(12.))
            .bg(p.raised)
            .border_1()
            .border_l_2()
            .border_color(p.hair)
            .child(div().px_3().pb_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.accent).child(format!("{heading} · {}", waiting.len())))
            .children(rows)
            .when(more > 0, |c| {
                c.child(
                    h_flex()
                        .id("for-you-more")
                        .h(px(32.))
                        .px_3()
                        .gap_3()
                        .text_size(px(SMALL))
                        .text_color(p.muted)
                        .items_center()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.show(Screen::Board, cx)))
                        .child(format!("+{more}"))
                        .child(t!("approvals.see_all_on_board")),
                )
            })
            .into_any_element(),
    )
}

/// "I did this — resume": the ticket goes back to its member. What was done
/// can be told in `/resume T-n <what>`; the button sends it with no word.
fn resume(shell: &Shell, p: Palette, id: TicketId, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let _ = shell;
    div()
        .id(("resume", id as u64))
        .flex_none()
        .px_2()
        .h(px(24.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(p.hair)
        .text_size(px(12.))
        .hover(move |s| s.bg(p.wash))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.resume_ticket(id, cx);
        }))
        .child(t!("approvals.done_resume"))
}

impl Shell {
    pub fn resume_ticket(&mut self, ticket: TicketId, cx: &mut Context<Self>) {
        self.send_command(Command::Resume { squad: self.model.squad.clone(), ticket, note: String::new() });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's glob brings its own `#[test]`.
    use super::{Card, Queue, always_label, decision_for};
    use equipe_core::protocol::{Approval, Command, Decision, SessionId};

    fn approval(id: u64, session: SessionId, tool: &str, action: &str) -> Approval {
        Approval { id, session, member: "m".into(), ticket: Some(9), tool: tool.into(), action: action.into(), reason: None, preview: None }
    }

    fn queue(items: Vec<Approval>) -> Queue {
        let mut q = Queue::default();
        q.load(items);
        q
    }

    #[test]
    fn the_oldest_request_is_the_one_answered() {
        let mut q = queue(vec![approval(1, 2, "Bash", "ls"), approval(2, 3, "Bash", "pwd")]);
        assert_eq!(q.len(), 2);
        assert_eq!(q.answer("acme", Decision::Allow), Some(Command::Decide { squad: "acme".into(), approval: 1, decision: Decision::Allow }));
        // Answered: hidden at once, so the next key is for the next request.
        assert_eq!(q.first().map(|a| a.id), Some(2));
        assert_eq!(q.answer("acme", Decision::Deny), Some(Command::Decide { squad: "acme".into(), approval: 2, decision: Decision::Deny }));
        assert_eq!(q.answer("acme", Decision::Allow), None);
        // The daemon's word clears them; a lost link shows the unresolved again.
        q.resolve(1);
        q.forget_answers();
        assert_eq!(q.ids(), vec![2]);
    }

    #[test]
    fn always_is_no_answer_where_there_is_no_rule() {
        let mut q = queue(vec![approval(1, 2, "Bash", "ls && rm x")]);
        assert_eq!(q.answer("acme", Decision::Always), None);
        assert_eq!(q.len(), 1, "still waiting");
        let mut q = queue(vec![approval(1, 2, "Bash", "npm install")]);
        assert!(q.answer("acme", Decision::Always).is_some());
    }

    #[test]
    fn a_request_is_not_queued_twice_and_is_found_by_member() {
        let mut q = queue(vec![approval(1, 2, "Bash", "ls")]);
        q.push(approval(1, 2, "Bash", "ls"));
        q.push(approval(2, 5, "Bash", "pwd"));
        assert_eq!(q.len(), 2);
        assert_eq!(q.first_for(5).map(|a| a.id), Some(2));
        assert!(q.first_for(9).is_none());
    }

    #[test]
    fn keys_answer_as_in_the_terminal() {
        assert_eq!(decision_for("y"), Some(Decision::Allow));
        assert_eq!(decision_for("a"), Some(Decision::Always));
        assert_eq!(decision_for("n"), Some(Decision::Deny));
        assert_eq!(decision_for("x"), None);
    }

    #[test]
    fn the_card_shows_the_start_of_the_action_by_its_kind() {
        let script = (1..=20).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
        let bash = Card::of(&approval(1, 2, "Bash", &script));
        assert_eq!((bash.lines.len(), bash.more), (3, 17));
        let code = Card::of(&approval(1, 2, equipe_core::permissions::BROWSER_CODE, &script));
        assert_eq!((code.lines.len(), code.more), (12, 8));
        let send = Card::of(&approval(1, 2, equipe_core::desktop_gate::SEND, &script));
        assert_eq!((send.lines.len(), send.more), (8, 12));
        assert!(send.is_send && send.always.is_none());
    }

    #[test]
    fn the_card_names_files_a_script_writes_and_what_always_would_allow() {
        let card = Card::of(&approval(1, 2, "Bash", "cat > a.txt <<EOF\nx\nEOF\necho hi | tee -a b.log"));
        assert_eq!(card.writes, vec!["a.txt", "b.log"]);
        assert_eq!(Card::of(&approval(1, 2, "Bash", "npm install left-pad")).always.as_deref(), Some("npm install"));
    }

    #[test]
    fn always_reads_short() {
        assert_eq!(always_label("WebFetch(domain:docs.rs)"), "docs.rs");
    }
}
