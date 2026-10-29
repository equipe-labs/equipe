//! Requests for permission: the queue, the card's words, and the card — above
//! the composer for the oldest one, and in the peek for the one picked in
//! 나에게. Allow once is Y or A, always ⇧A, decline N. The queue and the card's
//! text are plain code, tested without a window; the drawing is below them.

use std::collections::HashMap;

use equipe_core::t;
use equipe_core::protocol::{Approval, Command, Decision, SessionId};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::motion;
use crate::theme::{MONO, Palette, R_LG, R_MD, SMALL, look};
use crate::ui;
use crate::{Screen, Shell};

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

    /// Every request still waiting, oldest first.
    pub fn open_items(&self) -> impl Iterator<Item = &Approval> {
        self.open()
    }

    pub fn get(&self, id: u64) -> Option<&Approval> {
        self.open().find(|a| a.id == id)
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
        let id = self.first()?.id;
        self.answer_id(squad, id, decision, None)
    }

    /// Answer one request, picked by its id — with what the person wrote, if anything.
    pub fn answer_id(&mut self, squad: &str, id: u64, decision: Decision, words: Option<String>) -> Option<Command> {
        let approval = self.get(id)?;
        if decision == Decision::Always && always_rule(approval).is_none() {
            return None;
        }
        let id = approval.id;
        self.answered.push(id);
        let words = words.map(|w| w.trim().to_owned()).filter(|w| !w.is_empty());
        Some(Command::Decide { squad: squad.to_owned(), approval: id, decision, words })
    }
}

/// The answer a key gives, as everywhere a need is answered: Y or A allows
/// once, ⇧A always, N declines.
pub fn decision_for(key: &str, shift: bool) -> Option<Decision> {
    match (key, shift) {
        ("a", true) => Some(Decision::Always),
        ("y" | "a", false) => Some(Decision::Allow),
        ("n", false) => Some(Decision::Deny),
        _ => None,
    }
}

/// The boxes the person writes to a member in, on its request: open once
/// "R 답장" is pressed, by request.
#[derive(Default)]
pub struct Words {
    pub fields: HashMap<u64, Entity<TextareaState>>,
    subscriptions: HashMap<u64, Subscription>,
}

impl Words {
    /// Whether one of the boxes has the keyboard: what is typed is the box's.
    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        self.fields.values().any(|f| f.read(cx).focus_handle(cx).is_focused(window))
    }

    /// What is written for request `id`, if its box is open.
    fn take(&mut self, id: u64, cx: &App) -> Option<String> {
        self.subscriptions.remove(&id);
        self.fields.remove(&id).map(|f| f.read(cx).value().to_string())
    }
}

// ── the card's words ──

/// A member signing in with a kept login (`equipe_core::vault`): "always"
/// lets that member use it without asking.
pub const SIGN_IN: &str = "Sign in";

/// What kind of thing is asked, as a noun phrase.
pub fn noun(tool: &str) -> &'static str {
    match tool {
        "Bash" => t!("approvals.noun_bash"),
        "Write" | "Edit" | "MultiEdit" => t!("approvals.noun_edit"),
        "WebFetch" => t!("approvals.noun_web_fetch"),
        equipe_core::desktop_gate::SEND => t!("approvals.noun_send"),
        equipe_core::permissions::BROWSER_CODE => t!("approvals.noun_browser_code"),
        SIGN_IN => t!("approvals.noun_sign_in"),
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
    pub is_sign_in: bool,
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
            always: if approval.tool == SIGN_IN { Some(approval.member.clone()) } else { always_rule(approval).map(|r| always_label(&r)) },
            is_send: approval.tool == equipe_core::desktop_gate::SEND,
            is_sign_in: approval.tool == SIGN_IN,
        }
    }
}

// ── drawing ──

impl Shell {
    /// Answer the oldest request.
    pub fn decide(&mut self, decision: Decision, cx: &mut Context<Self>) {
        if let Some(id) = self.model.approvals.first().map(|a| a.id) {
            self.decide_on(id, decision, cx);
        }
    }

    /// Answer one request, with what is written in its box.
    pub fn decide_on(&mut self, id: u64, decision: Decision, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        let words = self.approval_words.fields.get(&id).map(|f| f.read(cx).value().to_string());
        if let Some(command) = self.model.approvals.answer_id(&squad, id, decision, words) {
            self.approval_words.take(id, cx);
            self.send_command(command);
            cx.notify();
        }
    }

    /// Open the box to write to the member on request `id` — why not, what
    /// instead, what to mind — and put the keyboard in it. ⏎ declines with
    /// the words, ⌘⏎ allows with them.
    pub fn open_words(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.approvals.get(id).is_none() {
            return;
        }
        let field = match self.approval_words.fields.get(&id) {
            Some(field) => field.clone(),
            None => {
                let field = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 6).submit_on_enter(true).placeholder(t!("approvals.words_placeholder")));
                let subscription = cx.subscribe_in(&field, window, move |shell, _, event: &InputEvent, _, cx| {
                    // Words in the box turn the buttons into "…하고 보내기".
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                    if let InputEvent::PressEnter { secondary, shift: false } = event {
                        let typed = shell.approval_words.fields.get(&id).is_some_and(|f| !f.read(cx).value().trim().is_empty());
                        if typed {
                            shell.decide_on(id, if *secondary { Decision::Allow } else { Decision::Deny }, cx);
                        }
                    }
                });
                self.approval_words.fields.insert(id, field.clone());
                self.approval_words.subscriptions.insert(id, subscription);
                field
            }
        };
        field.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// Close request `id`'s box, what was written dropped.
    pub fn close_words(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.approval_words.take(id, cx);
        if self.screen == Screen::Chat {
            self.composer.focus(window, cx);
        }
        cx.notify();
    }

    /// A key while a request's box has the keyboard: Esc closes it; the rest
    /// are the box's — never an answer by letter. True when taken.
    pub fn on_words_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> Option<bool> {
        let id = self.approval_words.fields.iter().find(|(_, f)| f.read(cx).focus_handle(cx).is_focused(window)).map(|(id, _)| *id)?;
        if key == "escape" {
            self.close_words(id, window, cx);
            return Some(true);
        }
        Some(false)
    }

    /// Open the screen a send would go from.
    pub fn open_preview(&self) {
        if let Some(path) = self.model.approvals.first().and_then(|a| a.preview.as_deref()) {
            self.open_file(path);
        }
    }

    /// The keys that answer: only on the conversation, with nothing typed — on
    /// the work screens the same letters act on what is picked. True when the
    /// key was taken.
    pub fn on_approval_key(&mut self, key: &str, shift: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.screen != Screen::Chat || !self.composer.text(cx).is_empty() || self.model.approvals.first().is_none() {
            return false;
        }
        if let Some(decision) = decision_for(key, shift) {
            self.decide(decision, cx);
            return true;
        }
        // R: write to the member instead of — or with — a yes or a no.
        if key == "r" && !shift {
            if let Some(id) = self.model.approvals.first().map(|a| a.id) {
                self.open_words(id, window, cx);
            }
            return true;
        }
        if key == "o" && !shift && self.model.approvals.first().is_some_and(|a| a.preview.is_some()) {
            self.open_preview();
            return true;
        }
        false
    }
}

/// The card's buttons: the app's one button, 28 tall, its key beside its name.
fn button(p: Palette, id: impl Into<ElementId>, label: &'static str, key: &'static str, primary: bool, on: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
    let kind = if primary { ui::Kind::Primary } else { ui::Kind::Secondary };
    ui::button(p, id, kind, label).key(key).on_click(move |_, window, cx| on(window, cx))
}

/// The oldest request, drawn as a card above the composer.
pub fn approval(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let id = shell.model.approvals.first()?.id;
    let p = shell.palette;
    // Each request rises into place as it comes to the front.
    Some(
        motion::rise(
            div().w_full().mb_3().p_3().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).shadow_sm().child(card(shell, id, true, cx)?),
            ("approval-in", id),
        )
        .into_any_element(),
    )
}

/// One request: who asks, what exactly, why, and the answers.
pub fn card(shell: &Shell, id: u64, queue_line: bool, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let approval = model.approvals.get(id)?;
    let card = Card::of(approval);
    let (mark, hue) = look(model.members.iter().find(|m| m.session == approval.session).map_or("", |m| m.role.as_str()), &approval.member);
    let waiting = model.approvals.len();
    let handle = cx.entity().downgrade();
    let for_decisions = handle.clone();
    let act = move |d: Decision| {
        let handle = for_decisions.clone();
        move |_: &mut Window, cx: &mut App| {
            handle.update(cx, |shell, cx| shell.decide_on(id, d, cx)).ok();
        }
    };
    let preview = approval.preview.clone();
    // The box to write to the member in, once R opened it: its words go with the answer.
    let words = shell.approval_words.fields.get(&id).cloned();
    let typed = words.as_ref().is_some_and(|f| !f.read(cx).value().trim().is_empty());
    let (allow_label, allow_key, deny_label, deny_key) = if typed {
        (t!("approvals.allow_with_words"), "⌘⏎", t!("approvals.deny_with_words"), "⏎")
    } else {
        (t!("approvals.allow_once"), "A", t!("approvals.deny"), "N")
    };
    let words_box = words.map(|field| {
        let close = handle.clone();
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .px_3()
                    .py(px(6.))
                    .rounded(px(R_MD))
                    .bg(p.bg)
                    .border_1()
                    .border_color(p.accent)
                    .child(div().flex_1().min_w_0().text_size(px(14.)).line_height(px(21.)).child(Textarea::new(&field).appearance(false).bordered(false)))
                    .child(
                        div()
                            .id(("words-close", id))
                            .flex_none()
                            .mt(px(2.))
                            .px_1()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .hover(|s| s.text_color(p.text).bg(p.wash))
                            .child("✕")
                            .on_click(move |_, window, cx| {
                                close.update(cx, |shell, cx| shell.close_words(id, window, cx)).ok();
                            }),
                    ),
            )
            .child(div().text_size(px(12.)).text_color(p.muted).child(t!("approvals.words_hint")))
    });
    let opened = words_box.is_some();
    let open_words = handle.clone();
    Some(
        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().text_color(p.accent).child(t!("approvals.waiting_on_you_mark")))
                    .child(div().text_color(p.member(hue)).child(format!("{mark} {}", approval.member)))
                    .children(approval.ticket.map(|t| div().text_color(p.muted).child(format!("T-{t}"))))
                    .child(div().flex_1())
                    .when(queue_line && waiting > 1, |h| h.child(div().font_weight(FontWeight::NORMAL).text_color(p.muted).child(t!("approvals.queue", waiting = waiting)))),
            )
            .child(div().text_size(px(15.)).text_color(p.text).child(t!("approvals.allow_question", what = card.noun)))
            .child(
                v_flex()
                    .px_3()
                    .py_2()
                    .rounded(px(R_MD))
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
                    ("preview", id),
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
            .children(words_box)
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    .child(button(p, ("allow", id), allow_label, allow_key, true, act(Decision::Allow)))
                    .when(card.always.is_some() && !typed, |h| h.child(button(p, ("always", id), t!("approvals.always"), "⇧A", false, act(Decision::Always))))
                    .child(button(p, ("deny", id), deny_label, deny_key, false, act(Decision::Deny)))
                    .when(!opened, |h| {
                        h.child(button(p, ("words", id), t!("approvals.reply"), "R", false, move |window, cx| {
                            open_words.update(cx, |shell, cx| shell.open_words(id, window, cx)).ok();
                        }))
                    })
                    .child(div().flex_1())
                    .when(!typed, |h| h.child(div().text_size(px(12.)).text_color(p.muted).child(match &card.always {
                        Some(member) if card.is_sign_in => t!("approvals.sign_in_always", member = member),
                        Some(label) => t!("approvals.always_rule", label = label),
                        None if card.is_send => t!("approvals.send_always_asks").to_owned(),
                        None => t!("approvals.once_only").to_owned(),
                    }))),
            )
            .into_any_element(),
    )
}

/// The card of work waiting for the person above the composer gave way to
/// 나에게 in the sidebar, which holds the same list with its keys: nothing here.
pub fn for_you(_shell: &Shell, _cx: &mut Context<Shell>) -> Option<AnyElement> {
    None
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
        assert_eq!(q.answer("acme", Decision::Allow), Some(Command::Decide { squad: "acme".into(), approval: 1, decision: Decision::Allow, words: None }));
        // Answered: hidden at once, so the next key is for the next request.
        assert_eq!(q.first().map(|a| a.id), Some(2));
        assert_eq!(q.answer("acme", Decision::Deny), Some(Command::Decide { squad: "acme".into(), approval: 2, decision: Decision::Deny, words: None }));
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
    fn keys_answer_as_every_need_is_answered() {
        assert_eq!(decision_for("y", false), Some(Decision::Allow));
        assert_eq!(decision_for("a", false), Some(Decision::Allow), "A is yes: once");
        assert_eq!(decision_for("a", true), Some(Decision::Always), "⇧A: always");
        assert_eq!(decision_for("n", false), Some(Decision::Deny));
        assert_eq!(decision_for("x", false), None);
    }

    #[test]
    fn a_request_is_answered_by_its_id_too() {
        let mut q = queue(vec![approval(1, 2, "Bash", "ls"), approval(2, 3, "Bash", "pwd")]);
        assert_eq!(q.answer_id("acme", 2, Decision::Deny, None), Some(Command::Decide { squad: "acme".into(), approval: 2, decision: Decision::Deny, words: None }));
        assert_eq!(q.open_items().map(|a| a.id).collect::<Vec<_>>(), vec![1]);
        assert!(q.get(2).is_none());
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
