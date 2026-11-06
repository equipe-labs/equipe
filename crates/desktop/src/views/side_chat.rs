//! A side chat beside the conversation, as Claude's apps "Start a side
//! chat": words selected in a reply, "Side chat" on the bar over them (or
//! ⌘⇧J), and a panel opens at the right — the passage at its top (a click
//! goes back to it, flashing it), the side chat's words and steps as they
//! come (a search says what it looked for and the pages it found), its own
//! box, a close button. Nothing said in it reaches the conversation; "Bring
//! to main" puts its last answer over the composer as a quote, for the
//! person to send or not. One side chat a conversation; closed, it stops,
//! and the passage's reply keeps a mark that opens it again while the app
//! runs. Esc, or ✕, puts the keys back in the composer. The rules are
//! `crate::side_chat`'s.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use equipe_core::protocol::{Command, Entry, SessionId, SideChatId};
use equipe_core::t;
use gpui_kit::base::TextSelection;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::decide::Took;
use crate::compose::EnterGuard;
use crate::icons::{Icon, icon};
use crate::motion;
use crate::quote::Reply;
use crate::render::view::Look;
use crate::side_chat::{self as rules, SideConvo, Step};
use crate::theme::{ICON_SM, META, Palette, R_LG, R_MD, R_SM, SMALL, SMALL_LINE};
use crate::tips::tip;
use crate::{Screen, Shell};

/// The side chats' state on this window.
#[derive(Default)]
pub struct SideChatUi {
    /// Each conversation's side chat (one at a time), by the conversation.
    pub of: std::collections::HashMap<SessionId, SideChatId>,
    /// The conversation whose side chat shows beside it.
    pub open: Option<SessionId>,
    input: Option<Entity<TextareaState>>,
    guard: EnterGuard,
    scroll: ScrollHandle,
    focus: Option<FocusHandle>,
    /// How wide the conversation's pane was last drawn, the panel with it.
    pub room: Rc<Cell<f32>>,
    /// How many lines were drawn last: new ones scroll into sight.
    seen: Cell<usize>,
    _subscriptions: Vec<Subscription>,
}

impl SideChatUi {
    /// The side chat showing beside `viewed`, if one does.
    pub fn shown(&self, viewed: Option<SessionId>) -> Option<SideChatId> {
        let viewed = viewed?;
        (self.open == Some(viewed)).then(|| self.of.get(&viewed).copied()).flatten()
    }
}

impl Shell {
    /// The side chat on screen now, and its conversation.
    fn side_chat_shown(&self) -> Option<(SideChatId, &SideConvo)> {
        if self.screen != Screen::Chat {
            return None;
        }
        let side = self.side_chat.shown(self.model.viewed())?;
        self.model.side_chats.get(&side).map(|c| (side, c))
    }

    /// "Side chat" on the bar (or ⌘⇧J): a side chat about the words
    /// selected, beside the conversation — the one about them already, if
    /// there is one; another conversation's side chat about other words goes.
    pub fn side_chat_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((session, passage)) = self.quote.take_offer() else { return };
        TextSelection::clear(window, cx);
        let same = self.side_chat.of.get(&session).is_some_and(|s| self.model.side_chats.get(s).is_some_and(|c| c.passage == passage));
        if !same {
            if let Some(old) = self.side_chat.of.remove(&session) {
                self.model.side_chats.remove(&old);
                self.send_command(Command::CloseSideChat { side: old });
            }
            let side = equipe_core::protocol::side_chat::new_id();
            self.model.side_chats.insert(side, SideConvo::new(session, passage.clone()));
            self.side_chat.of.insert(session, side);
            self.send_command(Command::StartSideChat { session, side, passage });
        }
        self.side_chat_show(session, window, cx);
    }

    /// The panel shown for `session`'s side chat, the keys in its box.
    pub fn side_chat_show(&mut self, session: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(side) = self.side_chat.of.get(&session).copied() else { return };
        // Beside the conversation, alone: no ticket, no subagent there too.
        self.chat.side.clear();
        self.chat.sub.close();
        self.side_chat.open = Some(session);
        self.side_chat.focus.get_or_insert_with(|| cx.focus_handle());
        self.side_chat.seen.set(0);
        let who = self.model.selected_member().map(|m| m.name.clone()).unwrap_or_default();
        let input = match self.side_chat.input.clone() {
            Some(input) => input,
            None => {
                let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 6).submit_on_enter(false));
                self.side_chat._subscriptions = vec![
                    cx.subscribe_in(&input, window, |_: &mut Shell, _, event: &InputEvent, _, cx| {
                        if matches!(event, InputEvent::Change) {
                            cx.notify();
                        }
                    }),
                    // The Enter that ends a composition is only that.
                    cx.observe_in(&input, window, |shell: &mut Shell, input, window, cx| {
                        let composing = input.update(cx, |s, cx| gpui_kit::EntityInputHandler::marked_text_range(s, window, cx).is_some());
                        shell.side_chat.guard.observe(composing, Instant::now());
                    }),
                ];
                self.side_chat.input = Some(input.clone());
                input
            }
        };
        input.update(cx, |s, cx| {
            s.set_placeholder(t!("side_chat.placeholder", name = who), window, cx);
            s.focus(window, cx);
        });
        // The passage lit softly while its side chat is open.
        if let Some(passage) = self.model.side_chats.get(&side).map(|c| c.passage.clone()) {
            self.quote_unlight(cx);
            self.quote_light(&passage, None, false, cx);
        }
        cx.notify();
    }

    /// ⏎ in its box (or its button): the person's message, sent.
    pub fn side_chat_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.side_chat.guard.allows_send(Instant::now()) {
            return;
        }
        let (Some((side, chat)), Some(input)) = (self.side_chat_shown(), self.side_chat.input.clone()) else { return };
        let text = input.read(cx).value().trim().to_owned();
        if text.is_empty() || chat.working {
            return;
        }
        let (session, passage) = (chat.session, chat.passage.clone());
        if let Some(chat) = self.model.side_chats.get_mut(&side) {
            // Working at once: the daemon's word comes a moment later.
            chat.working = true;
            chat.gone = false;
        }
        self.send_command(Command::SideChatSubmit { session, side, passage, text });
        input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        cx.notify();
    }

    /// ✕ or Esc: the side chat stops and goes from beside the conversation —
    /// its words kept, its passage's reply marked to open it again — and the
    /// keys go back to the composer.
    pub fn side_chat_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.side_chat.open.take() else { return };
        if let Some(side) = self.side_chat.of.get(&session).copied()
            && let Some(chat) = self.model.side_chats.get_mut(&side)
            && !chat.gone
        {
            chat.gone = true;
            chat.working = false;
            self.send_command(Command::CloseSideChat { side });
        }
        self.quote_unlight(cx);
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// "Bring to main": its last answer, a card over the composer, quoted —
    /// the person sends it or not.
    pub fn side_chat_bring(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(answer) = self.side_chat_shown().and_then(|(_, c)| c.last_answer()) else { return };
        let Some(quote) = crate::quote::trim(&rules::plain(&answer)) else { return };
        let at = match self.composer.replies.iter().position(|r| r.quote == quote) {
            Some(at) => at,
            None => {
                self.composer.replies.push(Reply::asked(quote));
                self.composer.replies.len() - 1
            }
        };
        self.composer.reveal_card(at);
        self.composer.focus(window, cx);
        self.save_drafts_soon(cx);
        cx.notify();
    }

    /// A click on its passage: back to the words in the conversation, flashing them.
    pub fn side_chat_jump(&mut self, cx: &mut Context<Self>) {
        let Some(passage) = self.side_chat_shown().map(|(_, c)| c.passage.clone()) else { return };
        // Covering the conversation (a narrow window), it makes way to show
        // the words — still going, its mark there to open it again.
        if rules::width(self.side_chat.room.get()).1 {
            self.side_chat.open = None;
            self.quote_unlight(cx);
        }
        self.quote_jump(usize::MAX, &passage, cx);
    }

    /// A keystroke, before anyone else sees it: in the side chat's box, ⏎
    /// sends and Esc closes it (⇧⏎ is a new line).
    pub fn on_side_chat_key(&mut self, key: &str, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> Took {
        let Some(input) = self.side_chat.input.clone().filter(|_| self.side_chat_shown().is_some()) else { return Took::Pass };
        let in_box = input.read(cx).focus_handle(cx).is_focused(window);
        let in_panel = self.side_chat.focus.as_ref().is_some_and(|f| f.contains_focused(window, cx));
        if !in_box && !in_panel {
            return Took::Pass;
        }
        match key {
            "escape" => {
                self.side_chat_close(window, cx);
                Took::Taken
            }
            "enter" if in_box && !m.shift && !m.alt && !m.control => {
                if input.update(cx, |s, cx| gpui_kit::EntityInputHandler::marked_text_range(s, window, cx).is_some()) {
                    return Took::Typing;
                }
                self.side_chat_send(window, cx);
                Took::Taken
            }
            _ if in_box => Took::Typing,
            _ => Took::Pass,
        }
    }

    /// Dev tool (`snapshot.rs`): words into its box.
    #[cfg(feature = "snapshot")]
    pub fn side_chat_type(&mut self, words: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = &self.side_chat.input {
            input.update(cx, |s, cx| s.set_value(words.to_owned(), window, cx));
            cx.notify();
        }
    }

    /// Dev tool (`snapshot.rs`): the latest reply's first words, a side chat at once.
    #[cfg(feature = "snapshot")]
    pub fn side_chat_open_on(&mut self, words: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.quote.offer_words(self.model.viewed(), words);
        self.side_chat_start(window, cx);
    }
}

/// The reply in the conversation that holds a closed side chat's passage,
/// and so its mark: the latest row with the words.
pub fn marked_row(shell: &Shell) -> Option<usize> {
    let viewed = shell.model.viewed()?;
    if shell.side_chat.open == Some(viewed) {
        return None;
    }
    let side = shell.side_chat.of.get(&viewed)?;
    let passage = &shell.model.side_chats.get(side)?.passage;
    shell.chat.blocks.iter().rposition(|b| matches!(b, crate::convo::Block::Reply { .. }) && b.searchable().is_some_and(|t| crate::quote::holds(&t, passage)))
}

/// The mark under a reply whose passage has a side chat: a click opens it again.
pub fn mark(p: Palette, key: u64, handle: WeakEntity<Shell>) -> AnyElement {
    let chip = h_flex()
        .id(("side-chat-mark", key))
        .role(Role::Button)
        .mt(px(6.))
        .h(px(22.))
        .px(px(7.))
        .gap(px(5.))
        .items_center()
        .rounded(px(R_SM))
        .border_1()
        .border_color(p.hair)
        .text_size(px(META))
        .text_color(p.text2)
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .child(icon(Icon::AppWindow, 12., p.muted))
        .child(t!("side_chat.mark"))
        .on_click(move |_, window, cx| {
            handle
                .update(cx, |shell, cx| {
                    if let Some(viewed) = shell.model.viewed() {
                        shell.side_chat_show(viewed, window, cx);
                    }
                })
                .ok();
        });
    h_flex().child(tip(chip, p, t!("side_chat.mark_tip"), None)).into_any_element()
}

/// Markdown in the panel's own size.
fn words(p: Palette, id: (&'static str, u64), text: &str, color: Rgba) -> impl IntoElement {
    let look = Look { size: SMALL, line: SMALL_LINE, color, live: false };
    crate::render::view::nested(p, id, text, look)
}

/// A step: a search with the pages it found, a page read, a look at the files.
fn step_row(shell: &Shell, n: usize, step: &Step, running: bool) -> AnyElement {
    let p = shell.palette;
    let line = |glyph: Icon, said: String, what: String| {
        h_flex()
            .min_h(px(24.))
            .gap(px(7.))
            .items_center()
            .text_size(px(SMALL))
            .child(if running { motion::spinner(11., p.muted) } else { icon(glyph, 12., p.muted).into_any_element() })
            .child(div().flex_none().text_color(p.muted).child(said))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(p.text2).child(what))
    };
    match step {
        Step::Searched { query, sources } => {
            let said = if running { t!("side_chat.searching") } else { t!("side_chat.searched") };
            let found = sources.iter().enumerate().map(|(k, s)| {
                let url = s.url.clone();
                let row = h_flex()
                    .id(("side-chat-source", n * 64 + k))
                    .h(px(22.))
                    .pl(px(19.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(R_SM))
                    .text_size(px(META))
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.wash))
                    .child(div().flex_none().text_color(p.muted).child(s.site()))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(p.text2).child(s.title.clone()))
                    .child(icon(Icon::External, 11., p.muted))
                    .on_click(move |_, _, cx| cx.open_url(&url));
                tip(row, p, s.url.clone(), None).into_any_element()
            });
            v_flex().gap(px(1.)).child(line(Icon::Search, said.to_owned(), query.clone())).children(found).into_any_element()
        }
        Step::Read { url } => {
            let said = if running { t!("side_chat.reading") } else { t!("side_chat.read") };
            line(Icon::Globe, said.to_owned(), rules::site(url)).into_any_element()
        }
        Step::Looked { tool, target } => {
            let verb = crate::convo::tool_verb(tool);
            let said = crate::activity::phrase(verb, running);
            let base = shell.model.convos.get(&shell.model.viewed().unwrap_or_default()).map(|c| c.base.clone()).unwrap_or_default();
            line(Icon::File, said, crate::convo::relative(target, &base)).into_any_element()
        }
    }
}

/// The person's message in it: a quiet bubble at the right.
fn bubble(p: Palette, text: &str) -> AnyElement {
    h_flex()
        .w_full()
        .justify_end()
        .child(div().max_w(px(300.)).px(px(11.)).py(px(7.)).rounded(px(R_LG)).bg(p.wash).text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.text).child(text.to_owned()))
        .into_any_element()
}

/// The side chat's panel, beside the conversation (or over it, when the
/// window is too narrow for both).
pub fn panel(shell: &Shell, cx: &mut Context<Shell>) -> Option<(AnyElement, bool)> {
    let (side, chat) = shell.side_chat_shown()?;
    let p = shell.palette;
    let (width, covers) = rules::width(shell.side_chat.room.get());
    let who = shell.model.selected_member().map(|m| m.name.clone()).unwrap_or_default();
    let key = side;

    let can_bring = chat.last_answer().is_some();
    let bring = crate::ui::button(p, "side-chat-bring", crate::ui::Kind::Ghost, t!("side_chat.bring"))
        .icon(Icon::ArrowLeft)
        .tip(t!("side_chat.bring_tip"), None)
        .disabled(!can_bring)
        .on_click(cx.listener(|shell, _, window, cx| shell.side_chat_bring(window, cx)));
    let close = crate::ui::icon_button(p, "side-chat-close", Icon::X, t!("side_chat.close"), Some("Esc")).on_click(cx.listener(|shell, _, window, cx| shell.side_chat_close(window, cx)));
    let head = h_flex()
        .h(px(44.))
        .flex_none()
        .pl_4()
        .pr_2()
        .gap_1()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(icon(Icon::AppWindow, ICON_SM, p.text2))
        .child(
            h_flex()
                .min_w_0()
                .flex_1()
                .pl_1()
                .gap_1p5()
                .items_baseline()
                .text_size(px(SMALL))
                .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("side_chat.title")))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(p.muted).child(who.clone())),
        )
        .child(bring)
        .child(close);

    // The passage, a click away from where it is.
    let passage = super::quote::passage(p, &chat.passage, 4)
        .id("side-chat-passage")
        .p(px(10.))
        .rounded(px(R_MD))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .on_click(cx.listener(|shell, _, _, cx| shell.side_chat_jump(cx)));
    let passage = tip(passage, p, t!("quote.jump"), None);
    // What it is, and — on an engine with no search — that it doesn't search.
    let about = match chat.search {
        Some(false) => h_flex()
            .gap(px(7.))
            .items_start()
            .text_size(px(META))
            .line_height(px(17.))
            .text_color(p.text2)
            .child(div().pt(px(2.)).child(icon(Icon::Info, 12., p.muted)))
            .child(div().flex_1().min_w_0().child(t!("side_chat.no_search", name = who)))
            .into_any_element(),
        _ => div().text_size(px(META)).line_height(px(17.)).text_color(p.muted).child(t!("side_chat.private")).into_any_element(),
    };

    // Its words and steps, as they come.
    let entries = chat.transcript.entries();
    let mut rows: Vec<AnyElement> = Vec::new();
    let last_tool = entries.iter().rposition(|e| matches!(e, Entry::Tool { .. }));
    let last = entries.len().saturating_sub(1);
    for (n, entry) in entries.iter().enumerate() {
        match entry {
            Entry::User { text } => rows.push(bubble(p, text)),
            Entry::Reply { text } if !text.trim().is_empty() => rows.push(div().child(words(p, ("side-chat-said", key ^ (n as u64) << 1), text, p.prose)).into_any_element()),
            Entry::Tool { tool, target, outcome, .. } => {
                let running = chat.working && outcome.is_none() && last_tool == Some(n) && n == last;
                rows.push(step_row(shell, n, &rules::step(tool, target.as_deref(), outcome.as_deref()), running));
            }
            Entry::Error { message, .. } => rows.push(div().text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.danger()).child(message.clone()).into_any_element()),
            _ => {}
        }
    }
    // At work and nothing to show for it yet: a quiet line that says so.
    let busy = (chat.working && !matches!(entries.last(), Some(Entry::Reply { .. }) | Some(Entry::Tool { outcome: None, .. })))
        .then(|| h_flex().h(px(24.)).gap(px(7.)).items_center().text_size(px(SMALL)).text_color(p.muted).child(motion::spinner(11., p.muted)).child(t!("side_chat.thinking")));
    let hello = (!chat.said_anything() && !chat.working).then(|| {
        div().text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.muted).child(if chat.search == Some(false) { t!("side_chat.hello_offline", name = who) } else { t!("side_chat.hello", name = who) })
    });
    let count = rows.len() + usize::from(busy.is_some());
    if count > shell.side_chat.seen.get() {
        shell.side_chat.scroll.scroll_to_bottom();
    }
    shell.side_chat.seen.set(count);
    let body = v_flex()
        .id("side-chat-body")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&shell.side_chat.scroll)
        .px_4()
        .pt_3()
        .pb_4()
        .gap_3()
        .child(passage)
        .child(about)
        .children(hello)
        .children(rows)
        .children(busy);

    // Its own box.
    let input = shell.side_chat.input.clone()?;
    let empty = input.read(cx).value().trim().is_empty();
    let send = div()
        .id("side-chat-send")
        .size(px(26.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(R_SM))
        .bg(if empty || chat.working { p.wash } else { p.primary })
        .when(!empty && !chat.working, |b| b.cursor_pointer())
        .child(icon(Icon::ArrowUp, 13., if empty || chat.working { p.muted } else { p.on_primary }))
        .on_click(cx.listener(|shell, _, window, cx| shell.side_chat_send(window, cx)));
    let send = tip(send, p, t!("side_chat.send"), Some("⏎"));
    let field = v_flex()
        .mx_3()
        .mb_3()
        .px(px(10.))
        .pt(px(8.))
        .pb(px(6.))
        .gap(px(4.))
        .flex_none()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .text_size(px(14.))
        .line_height(px(21.))
        .child(Textarea::new(&input).appearance(false).bordered(false))
        .child(h_flex().gap_2().items_center().child(div().flex_1().text_size(px(META)).text_color(p.muted).child(t!("side_chat.keys"))).child(send));

    let mut panel = v_flex()
        .id("side-chat")
        .role(Role::Complementary)
        .aria_label(t!("side_chat.title"))
        .w(px(width))
        .flex_none()
        .h_full()
        .bg(p.bg)
        .when(!covers, |d| d.border_l_1().border_color(p.hair))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    if let Some(focus) = &shell.side_chat.focus {
        panel = panel.track_focus(focus);
    }
    Some((motion::fade(panel.child(head).child(body).child(field), ("side-chat-in", side as usize), motion::BASE).into_any_element(), covers))
}
