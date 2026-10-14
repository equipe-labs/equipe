//! The conversation with a member (T-54 §3.2–3.3): your messages, the member's
//! replies as Markdown, tool steps gathered and folded, notes, errors and
//! commits — in a virtual list that follows the end while you are there.
//!
//! Streaming draws on the daemon's tick only (see `main.rs`): each tick the
//! blocks are rebuilt, the list told which rows differ, and the window drawn
//! once. When the last piece has come nothing asks for another frame.

use std::rc::Rc;

use equipe_core::protocol::SessionId;
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::base::{TextView, TextViewStyle};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{approvals, composer, header};
use crate::convo::{Block, RunRow, link_tickets, ticket_of_link};
use crate::state::{Model, Presence};
use crate::theme::{BODY, MONO, Palette, SMALL, look};
use crate::{Screen, Shell};

/// Reading width: about 48 Korean characters.
const COLUMN: f32 = 720.;

pub struct Chat {
    scroller: Entity<MessageScrollerState>,
    blocks: Rc<Vec<Block>>,
    /// Whose conversation `blocks` are, and the ticket it shows the work of.
    shown: Option<(SessionId, Option<u32>)>,
    seen_revision: u64,
    /// Tool runs shown whole rather than folded (⌘⇧O).
    pub expanded: bool,
    shown_expanded: bool,
    /// Tells one conversation's rows from another's.
    serial: u64,
}

impl Chat {
    pub fn new(cx: &mut Context<Shell>) -> Self {
        Self {
            scroller: cx.new(|cx| MessageScrollerState::new(0, cx)),
            blocks: Rc::new(Vec::new()),
            shown: None,
            seen_revision: 0,
            expanded: false,
            shown_expanded: false,
            serial: 0,
        }
    }

    /// Bring the list up to what the model shows. Cheap when nothing moved.
    pub fn sync(&mut self, model: &Model, cx: &mut App) {
        let key = model.viewed().map(|id| (id, model.opened));
        let moved = key != self.shown;
        if !moved && model.revision == self.seen_revision && self.expanded == self.shown_expanded {
            return;
        }
        let new = model.convo().map(|c| c.blocks()).unwrap_or_default();
        let old = std::mem::replace(&mut self.blocks, Rc::new(new));
        self.seen_revision = model.revision;
        let folds_changed = self.expanded != self.shown_expanded;
        self.shown_expanded = self.expanded;
        let n = self.blocks.len();
        if moved {
            self.shown = key;
            self.serial += 1;
            self.scroller.update(cx, |s, cx| s.reset(n, cx));
            return;
        }
        // Rows from the first that differs are new or changed (a streaming
        // reply is the last row growing); the list measures only those again.
        let same = old.iter().zip(self.blocks.iter()).take_while(|(a, b)| a == b).count();
        self.scroller.update(cx, |s, cx| {
            if old.len() != n {
                s.splice(same..old.len(), n - same, cx);
            } else if same < n {
                s.remeasure_items(same..n, cx);
            }
            if folds_changed {
                s.remeasure(cx);
            }
            cx.notify();
        });
    }

    /// Back to the end, following it again.
    pub fn follow_tail(&self, cx: &mut App) {
        self.scroller.update(cx, |s, cx| s.scroll_to_end(cx));
    }
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let body = match model.selected_member() {
        None => empty(p, "대화할 멤버가 아직 없습니다", "터미널에서 `equipe`를 실행해 스쿼드를 시작하세요.").into_any_element(),
        Some(member) if shell.chat.blocks.is_empty() => {
            let (title, text) = hello(model, member.role.as_str(), member.name.as_str(), member.presence);
            empty(p, title, text).into_any_element()
        }
        Some(member) => {
            let (mark, hue) = look(&member.role, &member.name);
            let look = Look { mark, name: member.name.clone(), color: p.member(hue) };
            conversation(shell, look, cx).into_any_element()
        }
    };
    // A ticket open: its number, title and where it stands, over its work.
    let banner = model.opened.and_then(|id| model.tickets.iter().find(|t| t.id == id)).map(|t| {
        let hint = match t.status {
            equipe_core::protocol::TicketStatus::Review => format!("리뷰 대기 · /accept T-{0} · /reopen T-{0} 이유", t.id),
            equipe_core::protocol::TicketStatus::Waiting => format!("사람 대기 · 할 일을 마치면 /resume T-{}", t.id),
            _ => "여기에 쓰면 작업 중인 멤버가 다음 단계에서 읽습니다".to_owned(),
        };
        v_flex()
            .w_full()
            .max_w(px(COLUMN))
            .px_4()
            .pt_3()
            .child(h_flex().gap_2().text_size(px(15.)).child(div().text_color(p.accent).child(format!("T-{}", t.id))).child(div().font_weight(FontWeight::SEMIBOLD).child(t.title.clone())))
            .child(div().text_size(px(12.)).text_color(p.muted).child(hint))
    });
    v_flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .children(banner)
        .child(body)
        .child(
            div()
                .w_full()
                .max_w(px(COLUMN))
                .px_4()
                .child(v_flex().children(approvals::for_you(shell, cx)).children(approvals::approval(shell, cx)))
                .child(composer::render(shell, cx)),
        )
}

/// What a blank conversation says. The PM's names the thread it is.
fn hello(model: &Model, role: &str, name: &str, presence: Presence) -> (&'static str, String) {
    if presence == Presence::Closed {
        return ("스쿼드를 떠난 멤버입니다", format!("{name}은(는) 더 이상 일하지 않습니다."));
    }
    match (role, model.focus.as_deref()) {
        ("pm", Some(project)) => ("PM에게 말하세요", format!("지금 보고 있는 {project} 이야기로 전달됩니다. 금방 끝나는 건 PM이 바로 답하고, 일은 티켓이 되어 왼쪽에 나타납니다.")),
        ("pm", None) => ("PM에게 만들고 싶은 것을 말하세요", "금방 끝나는 건 PM이 바로 답하고, 일은 티켓이 되어 왼쪽에 나타납니다. 티켓을 열면 그 작업을 보고 바로 지시할 수 있습니다.".into()),
        _ => ("직접 말을 걸 수 있습니다", "이 멤버에게 바로 말하면 PM도 알게 됩니다. 일은 보드에서 가져가 자기 작업 폴더에서 합니다.".into()),
    }
}

fn empty(p: Palette, title: &'static str, text: impl Into<SharedString>) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .w_full()
        .max_w(px(480.))
        .items_center()
        .justify_center()
        .gap_3()
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title))
        .child(div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).text_center().child(text.into()))
}

#[derive(Clone)]
struct Look {
    mark: &'static str,
    name: String,
    color: Rgba,
}

fn conversation(shell: &Shell, look: Look, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let blocks = shell.chat.blocks.clone();
    let expanded = shell.chat.expanded;
    let serial = shell.chat.serial;
    let handle = cx.entity().downgrade();
    div().flex_1().min_h_0().w_full().max_w(px(COLUMN)).px_4().child(
        MessageScroller::new("conversation", shell.chat.scroller.clone(), move |ix, _window, _cx| {
            row(p, &blocks[ix], ix, serial, expanded, &look, &handle)
        })
        .with_jump_button_transition(std::time::Duration::ZERO)
        .with_row_style(StyleRefinement::default().pb(px(8.)))
        .with_jump_button_label("↓ 맨 아래로"),
    )
}

pub type Handle = WeakEntity<Shell>;

fn row(p: Palette, block: &Block, ix: usize, serial: u64, expanded: bool, look: &Look, handle: &Handle) -> AnyElement {
    let id = serial << 24 | ix as u64;
    match block {
        Block::User(text) => h_flex()
            .gap_3()
            .px_4()
            .py_3()
            .mt_3()
            .rounded(px(8.))
            .bg(p.wash)
            .text_size(px(BODY))
            .line_height(px(24.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(div().flex_none().text_color(p.accent).child("›"))
            .child(div().min_w_0().flex_1().child(text.clone()))
            .into_any_element(),
        Block::Reply { text, first } => v_flex()
            .gap_1()
            .when(*first, |col| {
                col.mt_3().child(
                    h_flex()
                        .gap_2()
                        .text_size(px(SMALL))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(look.color)
                        .child(look.mark)
                        .child(look.name.clone()),
                )
            })
            .child(markdown(p, ("reply", id), text, BODY, 24., p.text, handle))
            .into_any_element(),
        Block::Steps(lines) => v_flex()
            .font_family(MONO)
            .text_size(px(12.5))
            .line_height(px(20.))
            .text_color(p.muted)
            .children(crate::convo::run_rows(lines, expanded).into_iter().enumerate().map(|(n, r)| match r {
                RunRow::Line(line) => h_flex()
                    .gap_2()
                    .when(line.edits, |l| l.text_color(p.text2))
                    .child(div().w(px(12.)).flex_none().text_color(if line.edits { p.accent } else { p.muted }).child(if line.edits { "✎" } else { "·" }))
                    .child(div().w(px(52.)).flex_none().child(line.verb.clone()))
                    .child(div().min_w_0().child(line.text()))
                    .into_any_element(),
                RunRow::Folded(hidden) => {
                    let handle = handle.clone();
                    h_flex()
                        .id(("unfold", id << 4 | n as u64))
                        .gap_2()
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            handle.update(cx, |shell, cx| shell.toggle_folds(cx)).ok();
                        })
                        .child(div().w(px(12.)).flex_none().child("·"))
                        .child(format!("{hidden}단계 더 · 펼치기 ⌘⇧O"))
                        .into_any_element()
                }
            }))
            .into_any_element(),
        Block::Note(text) => {
            let chapter = text.starts_with("── ") || text.starts_with("─ worked");
            div()
                .when(chapter, |d| d.mt_3())
                .text_color(p.muted)
                .child(markdown(p, ("note", id), text, SMALL, 20., p.muted, handle))
                .into_any_element()
        }
        Block::Error { message, next } => v_flex()
            .gap_1()
            .text_size(px(BODY))
            .line_height(px(24.))
            .child(h_flex().gap_2().child(div().flex_none().font_weight(FontWeight::SEMIBOLD).child("✗")).child(div().font_weight(FontWeight::SEMIBOLD).child(message.clone())))
            .children(next.clone().map(|next| {
                h_flex().gap_2().pl_5().text_size(px(SMALL)).child(div().text_color(p.muted).child("→")).child(div().font_family(MONO).font_weight(FontWeight::SEMIBOLD).child(next))
            }))
            .into_any_element(),
        Block::Landed { branch, stat } => v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .text_size(px(SMALL))
                    .child(div().text_color(p.accent).child("✓"))
                    .child("변경을 커밋했습니다 ·")
                    .child(div().font_family(MONO).font_weight(FontWeight::SEMIBOLD).child(branch.clone())),
            )
            .child(v_flex().pl_5().font_family(MONO).text_size(px(12.5)).line_height(px(20.)).text_color(p.muted).children(stat.lines().map(|l| div().child(l.trim().to_owned()))))
            .into_any_element(),
        Block::Memo(text) => v_flex()
            .text_color(p.muted)
            .child(h_flex().gap_3().text_size(px(BODY)).line_height(px(24.)).child(div().flex_none().child("›")).child(div().min_w_0().child(text.clone())))
            .child(div().pl_5().text_size(px(SMALL)).child("메모 · 이번 턴이 끝나면 읽습니다"))
            .into_any_element(),
        Block::Steering(text) => v_flex()
            .text_color(p.muted)
            .child(h_flex().gap_3().text_size(px(BODY)).line_height(px(24.)).child(div().flex_none().text_color(p.accent).child("›")).child(div().min_w_0().child(text.clone())))
            .child(div().pl_5().text_size(px(SMALL)).child("조종 · 다음 단계에서 읽힘, 하던 일은 멈추지 않습니다"))
            .into_any_element(),
    }
}

/// Markdown at `size`, in the palette's colours; headings stay body-sized (a
/// conversation is not a document), `T-n` is a way to the board.
pub fn markdown(p: Palette, id: (&'static str, u64), text: &str, size: f32, line: f32, color: Rgba, handle: &Handle) -> impl IntoElement + use<> {
    let handle = handle.clone();
    let muted = p.muted;
    let to_hsla = |c: Rgba| -> Hsla { c.into() };
    let code_block = StyleRefinement::default().bg(to_hsla(p.code_bg)).border_1().border_color(to_hsla(p.hair)).rounded(px(8.));
    let table = StyleRefinement::default().bg(to_hsla(p.code_bg)).border_color(to_hsla(p.hair)).text_color(to_hsla(p.text));
    let table_head = StyleRefinement::default().bg(to_hsla(p.raised)).text_color(to_hsla(p.text)).font_weight(FontWeight::SEMIBOLD);
    let table_cell = StyleRefinement::default().text_color(to_hsla(p.text2)).border_color(to_hsla(p.hair));
    let style = TextViewStyle::default()
        .with_foreground(to_hsla(color))
        .with_muted_foreground(to_hsla(p.muted))
        .with_link(to_hsla(p.accent))
        .with_selection(to_hsla(p.accent_bg))
        .with_border(to_hsla(p.hair))
        .with_code_background(to_hsla(p.code_bg))
        .with_code_block(code_block)
        .with_table(table)
        .with_table_head(table_head)
        .with_table_cell(table_cell)
        .with_inline_code(HighlightStyle { color: Some(to_hsla(p.accent)), background_color: Some(to_hsla(p.code_bg)), ..Default::default() })
        .with_heading(move |_| StyleRefinement::default().text_size(px(size)).font_weight(FontWeight::SEMIBOLD))
        .with_dark(p.bg.r < 0.5);
    div().text_size(px(size)).line_height(px(line)).text_color(color).child(
        TextView::markdown(ElementId::NamedInteger(id.0.into(), id.1), link_tickets(text))
            .style(style)
            .selectable(true)
            .code_block_actions(move |block, _, _| {
                let code = block.code();
                div()
                    .id("copy")
                    .px_2()
                    .text_size(px(12.))
                    .text_color(muted)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(code.to_string())))
                    .child("copy")
            })
            .on_link_click(move |url, _, _, cx| {
                if let Some(ticket) = ticket_of_link(url) {
                    handle.update(cx, |shell, cx| shell.open_ticket(ticket, cx)).ok();
                }
            }),
    )
}

impl Shell {
    pub fn toggle_folds(&mut self, cx: &mut Context<Self>) {
        self.chat.expanded = !self.chat.expanded;
        self.chat.sync(&self.model, cx);
        cx.notify();
    }

    /// A line of the sidebar was clicked: the PM, someone, or a ticket — its
    /// work in the conversation's place, or the board when there is none.
    pub fn open_side(&mut self, side: crate::state::Side, window: &mut Window, cx: &mut Context<Self>) {
        if !self.model.open(side) {
            if let crate::state::Side::Ticket(id) = side {
                self.open_ticket(id, cx);
            }
            return;
        }
        self.screen = Screen::Chat;
        self.refresh(window, cx);
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// A ticket number was clicked: show it on the board.
    pub fn open_ticket(&mut self, ticket: u32, cx: &mut Context<Self>) {
        self.board_ticket = Some(ticket);
        self.show(Screen::Board, cx);
    }
}

/// The chat screen: header, then the conversation and the composer.
pub fn screen(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    v_flex().flex_1().min_h_0().child(header::render(shell, cx)).child(render(shell, cx))
}
