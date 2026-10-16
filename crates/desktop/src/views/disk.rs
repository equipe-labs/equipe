//! "디스크": the banner while the disk runs short, and the panel it opens —
//! from the banner, settings, `/disk` and a news row's 정리하기. The panel shows
//! the free space with a bar, the fixes worth making (most room first), and
//! what uses the space, grouped; a fix of equipe's asks first with the exact
//! list and its total, then says what it gave back. What only the person can
//! clear shows its command to copy, or Finder. The rules are `crate::disk`.

use std::time::{Duration, Instant};

use equipe_core::disk::{self as room, DiskItem, DiskLevel, DiskStatus};
use equipe_core::i18n::label;
use equipe_core::t;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::disk::{self, Fix, FixKind, Stage};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{ICON_SM, META, MONO, Palette, R_LG, R_MD, SMALL, UI};
use crate::ui::{self, Kind};
use crate::Shell;

/// The panel's width.
const WIDTH: f32 = 720.;
/// Rows a group lists before it sums up the rest.
const ROWS: usize = 6;
/// How long "복사됨" stays on the button.
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// The colour a level is drawn in: the bar, the pill, the banner's mark.
fn tone(p: Palette, level: DiskLevel) -> Rgba {
    match level {
        DiskLevel::Ok => p.accent,
        DiskLevel::Low => p.amber,
        DiskLevel::Critical => p.orange,
        DiskLevel::Full => p.danger(),
    }
}

/// A level's tone, faint: under the banner, behind the pill.
fn tint(p: Palette, level: DiskLevel) -> Rgba {
    Rgba { a: if p.is_dark() { 0.13 } else { 0.10 }, ..tone(p, level) }
}

fn size(bytes: u64, partial: bool) -> String {
    format!("{}{}", if partial { "≥ " } else { "" }, room::size(bytes))
}

/// The strip over the conversation while the disk is short.
pub fn banner(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let status = shell.disk.banner()?;
    let p = shell.palette;
    let lang = crate::i18n::lang();
    let line = label::disk_line(lang, status)?;
    let reclaim = shell.disk.report.as_ref().map(|r| r.reclaimable()).filter(|b| *b > 0);
    Some(
        h_flex()
            .id("disk-banner")
            .h(px(38.))
            .flex_none()
            .pl(px(shell.header_inset(16., cx)))
            .pr_3()
            .gap_2p5()
            .items_center()
            .bg(tint(p, status.level))
            .border_b_1()
            .border_color(p.hair)
            .text_size(px(SMALL))
            .window_control_area(WindowControlArea::Drag)
            .child(icon(Icon::Warning, ICON_SM, tone(p, status.level)))
            .child(div().flex_none().text_color(p.text).font_weight(FontWeight::SEMIBOLD).child(line))
            .child(div().flex_1().min_w_0().truncate().text_color(p.muted).children(reclaim.map(|b| t!("disk.banner_reclaim", size = room::size(b)))))
            .child(ui::button(p, "disk-banner-open", Kind::Secondary, t!("disk.banner_open")).on_click(cx.listener(|this, _, window, cx| this.open_disk(window, cx))))
            .child(ui::icon_button(p, "disk-banner-close", Icon::X, t!("disk.banner_close"), None).on_click(cx.listener(|this, _, _, cx| {
                this.disk.dismiss();
                cx.notify();
            })))
            .into_any_element(),
    )
}

/// The panel, while it's open.
pub fn sheet(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if !shell.disk.open {
        return None;
    }
    let p = shell.palette;
    let close = ui::icon_button(p, "disk-close", Icon::X, t!("disk.close"), Some("Esc")).on_click(cx.listener(|this, _, window, cx| this.close_disk(window, cx)));
    let header = h_flex()
        .gap_3()
        .items_start()
        .child(div().mt(px(2.)).size(px(32.)).rounded(px(R_MD)).bg(p.wash).border_1().border_color(p.hair).flex().items_center().justify_center().child(icon(Icon::Server, 17., p.text)))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_size(px(17.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD).child(t!("disk.title")))
                .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).truncate().child(t!("disk.subtitle", host = shell.endpoint.label()))),
        )
        .child(close);
    let body = match &shell.disk.stage {
        Stage::List => list(shell, cx),
        Stage::Confirm => confirm(shell, cx),
        Stage::Cleaning => {
            let mark = div().size(px(56.)).flex().items_center().justify_center().child(Spinner::new().with_size(px(22.)).color(Hsla::from(p.muted))).into_any_element();
            centered(p, "disk-cleaning", mark, t!("disk.cleaning").into(), None, None)
        }
        Stage::Done(cleaned) => {
            let mark = motion::pop(div().size(px(56.)).rounded_full().bg(p.accent).flex().items_center().justify_center().child(icon(Icon::Check, 26., p.on_accent)), "disk-done-mark", false).into_any_element();
            let mut text = t!("disk.done_text", free = room::size(cleaned.status.free));
            if !cleaned.skipped.is_empty() {
                text = format!("{text}\n{}", t!("disk.done_skipped", count = cleaned.skipped.len()));
            }
            let back = ui::button(p, "disk-done-back", Kind::Primary, t!("disk.done_back")).field().on_click(cx.listener(|this, _, _, cx| {
                this.disk.back();
                cx.notify();
            }));
            centered(p, "disk-done", mark, t!("disk.done_title", size = room::size(cleaned.freed)), Some(text), Some(back.into_any_element()))
        }
    };
    let foot = h_flex()
        .gap_3()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .pt_3()
        .child(icon(Icon::Shield, ICON_SM, p.muted))
        .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("disk.foot")))
        .when(shell.disk.stage == Stage::List, |f| {
            f.child(
                ui::button(p, "disk-rescan", Kind::Ghost, if shell.disk.looking { t!("disk.looking") } else { t!("disk.rescan") })
                    .icon(Icon::Retry)
                    .disabled(shell.disk.looking)
                    .on_click(cx.listener(|this, _, _, cx| this.rescan_disk(cx))),
            )
        });
    Some(crate::views::connect::modal_w(p, "disk", WIDTH, v_flex().gap_4().min_h_0().child(header).child(body).child(foot)))
}

/// Free of total, with a bar and the level.
fn status_card(p: Palette, status: &DiskStatus, container: Option<u64>, reclaim: Option<u64>) -> AnyElement {
    let lang = crate::i18n::lang();
    let used = disk::used_fraction(status);
    let color = tone(p, status.level);
    let pill = div()
        .flex_none()
        .px_2()
        .h(px(22.))
        .flex()
        .items_center()
        .rounded(px(11.))
        .bg(tint(p, status.level))
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(label::disk_level(lang, status.level));
    // What can be done about it, else what the Mac's container says (the banner already says the level).
    let note = reclaim
        .filter(|b| *b > 0)
        .map(|b| t!("disk.banner_reclaim", size = room::size(b)))
        .or_else(|| container.filter(|c| c.abs_diff(status.free) > 1_000_000_000).map(|c| t!("disk.container", free = room::size(c))));
    v_flex()
        .gap_2p5()
        .p_4()
        .rounded(px(R_LG))
        .border_1()
        .border_color(p.hair)
        .child(
            h_flex()
                .gap_2()
                .items_end()
                .child(div().text_size(px(26.)).line_height(px(30.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(room::size(status.free)))
                .child(div().pb(px(3.)).text_size(px(UI)).text_color(p.muted).child(t!("disk.free_of", total = room::size(status.total))))
                .child(div().flex_1())
                .child(div().pb(px(3.)).child(pill)),
        )
        .child(div().h(px(8.)).w_full().rounded_full().bg(p.wash).child(div().h_full().w(relative(used.clamp(0.01, 1.))).rounded_full().bg(color)))
        .children(note.map(|n| div().text_size(px(12.)).line_height(px(17.)).text_color(p.muted).child(n)))
        .into_any_element()
}

/// The list stage: the status, the fixes, what uses the space.
fn list(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.disk;
    let status = ui.report.as_ref().map(|r| &r.status).or(ui.status.as_ref());
    let card = status.map(|s| status_card(p, s, ui.report.as_ref().and_then(|r| r.status.container_free), ui.report.as_ref().map(|r| r.reclaimable())));
    let Some(report) = &ui.report else {
        let mark = div().size(px(40.)).flex().items_center().justify_center().child(Spinner::new().with_size(px(20.)).color(Hsla::from(p.muted))).into_any_element();
        return v_flex().gap_4().children(card).child(div().h(px(180.)).child(centered(p, "disk-looking", mark, t!("disk.looking").into(), None, None))).into_any_element();
    };
    let fixes = disk::fixes(report);
    let reclaim = report.reclaimable();
    let all = (reclaim > 0).then(|| {
        ui::button(p, "disk-clean-all", Kind::Primary, t!("disk.clean_all", size = room::size(reclaim))).on_click(cx.listener(|this, _, _, cx| {
            this.disk.ask_all();
            cx.notify();
        }))
    });
    let fix_rows: Vec<AnyElement> = fixes.iter().enumerate().map(|(n, fix)| fix_row(shell, n, fix, cx)).collect();
    let fixes_block = v_flex()
        .gap_2()
        .child(
            h_flex()
                .items_center()
                .child(div().flex_1().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(t!("disk.fixes")))
                .children(all),
        )
        .child(if fix_rows.is_empty() {
            div().py_2().text_size(px(SMALL)).text_color(p.muted).child(t!("disk.nothing")).into_any_element()
        } else {
            v_flex().rounded(px(R_LG)).border_1().border_color(p.hair).children(fix_rows.into_iter().enumerate().map(|(n, row)| div().when(n > 0, |d| d.border_t_1().border_color(p.hair)).child(row))).into_any_element()
        });
    let lang = crate::i18n::lang();
    let groups = disk::groups(report).into_iter().enumerate().map(|(g, (group, items, total))| {
        let shown: Vec<AnyElement> = items.iter().take(ROWS).enumerate().map(|(n, item)| item_row(shell, g * 100 + n, item, cx)).collect();
        let rest = items.len().saturating_sub(ROWS);
        let rest_bytes: u64 = items.iter().skip(ROWS).map(|i| i.bytes).sum();
        v_flex()
            .gap_0p5()
            .child(
                h_flex()
                    .h(px(28.))
                    .items_center()
                    .border_b_1()
                    .border_color(p.hair)
                    .child(div().flex_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(label::disk_group(lang, group)))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(room::size(total))),
            )
            .children(shown)
            .when(rest > 0, |d| d.child(div().pl(px(2.)).h(px(26.)).flex().items_center().text_size(px(12.)).text_color(p.muted).child(t!("disk.more", count = rest, size = room::size(rest_bytes)))))
            .into_any_element()
    });
    let uses = v_flex().gap_3().child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(t!("disk.uses"))).children(groups);
    let partial = report.partial.then(|| div().text_size(px(12.)).text_color(p.muted).child(t!("disk.partial")));
    v_flex()
        .gap_4()
        .min_h_0()
        .children(card)
        .child(v_flex().id("disk-scroll").max_h(px(360.)).overflow_y_scroll().gap_5().child(fixes_block).child(uses).children(partial))
        .into_any_element()
}

/// A fix: what it is, how much, and its button.
fn fix_row(shell: &Shell, n: usize, fix: &Fix, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let lang = crate::i18n::lang();
    let report = shell.disk.report.as_ref();
    let item = |id: &String| report.and_then(|r| r.items.iter().find(|i| &i.id == id));
    let count = fix.ids.len();
    let (glyph, title, desc): (Icon, String, String) = match fix.kind {
        FixKind::Worktrees => (Icon::Folder, t!("disk.fix_worktrees").into(), t!("disk.fix_worktrees_desc", count = count)),
        FixKind::Builds => (Icon::Terminal, t!("disk.fix_builds").into(), t!("disk.fix_builds_desc", count = count)),
        FixKind::Logs => (Icon::FileText, t!("disk.fix_logs").into(), t!("disk.fix_logs_desc").into()),
        FixKind::Yourself => {
            let it = fix.ids.first().and_then(item);
            let kind = it.map_or("", |i| label::disk_kind(lang, i.kind));
            (Icon::Info, t!("disk.fix_yourself", name = it.map_or(String::new(), |i| i.label.clone())), t!("disk.fix_yourself_desc", kind = kind))
        }
    };
    let button = match fix.kind {
        FixKind::Yourself => fix.ids.first().and_then(item).map(|i| yourself_button(shell, ("disk-fix-own", n), i, cx)),
        _ => {
            let ids = fix.ids.clone();
            Some(
                ui::button(p, ("disk-fix", n), Kind::Secondary, t!("disk.clean"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.disk.ask(&ids);
                        cx.notify();
                    }))
                    .into_any_element(),
            )
        }
    };
    h_flex()
        .min_h(px(56.))
        .px_3()
        .py_2()
        .gap_3()
        .items_center()
        .child(div().flex_none().size(px(28.)).rounded(px(R_MD)).bg(p.wash).flex().items_center().justify_center().child(icon(glyph, ICON_SM, p.text2)))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().truncate().text_size(px(UI)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(title))
                .child(div().truncate().text_size(px(12.)).text_color(p.muted).child(desc)),
        )
        .child(div().flex_none().text_size(px(UI)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(room::size(fix.bytes)))
        // One width for every fix's button, so the sizes line up.
        .child(h_flex().flex_none().w(px(128.)).justify_end().children(button))
        .into_any_element()
}

/// For what only the person clears: its command to copy, or Finder (on this
/// Mac; over SSH the path, to copy).
fn yourself_button(shell: &Shell, id: (&'static str, usize), item: &DiskItem, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let copied = |text: &str| shell.disk.copied.as_ref().is_some_and(|(c, at)| c == text && at.elapsed() < COPIED_FOR);
    match (&item.command, shell.endpoint.is_remote()) {
        (Some(command), _) => {
            let done = copied(command);
            let command = command.clone();
            ui::button(p, id, Kind::Secondary, if done { t!("disk.copied") } else { t!("disk.copy") })
                .icon(if done { Icon::Check } else { Icon::Copy })
                .tip(command.clone(), None)
                .on_click(cx.listener(move |this, _, _, cx| this.copy_disk(&command, cx)))
                .into_any_element()
        }
        (None, false) => {
            let path = item.path.clone();
            ui::button(p, id, Kind::Secondary, t!("disk.reveal"))
                .icon(Icon::Folder)
                .on_click(move |_, _, _| drop(std::process::Command::new("open").arg("-R").arg(&path).spawn()))
                .into_any_element()
        }
        (None, true) => {
            let path = item.path.display().to_string();
            let done = copied(&path);
            ui::button(p, id, Kind::Secondary, if done { t!("disk.copied") } else { t!("disk.copy_path") })
                .icon(if done { Icon::Check } else { Icon::Copy })
                .on_click(cx.listener(move |this, _, _, cx| this.copy_disk(&path, cx)))
                .into_any_element()
        }
    }
}

/// One thing that takes space: its words, why, how much.
fn item_row(shell: &Shell, n: usize, item: &DiskItem, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let lang = crate::i18n::lang();
    let why = label::disk_why(lang, item.why);
    let action = (item.command.is_some()).then(|| {
        let command = item.command.clone().unwrap_or_default();
        let done = shell.disk.copied.as_ref().is_some_and(|(c, at)| *c == command && at.elapsed() < COPIED_FOR);
        ui::icon_button(p, ("disk-item-copy", n), if done { Icon::Check } else { Icon::Copy }, if done { t!("disk.copied").to_owned() } else { format!("{} · {command}", t!("disk.copy")) }, None)
            .on_click(cx.listener(move |this, _, _, cx| this.copy_disk(&command, cx)))
    });
    h_flex()
        .id(("disk-item", n))
        .h(px(30.))
        .px(px(2.))
        .gap_3()
        .items_center()
        .text_size(px(SMALL))
        .child(div().flex_1().min_w_0().truncate().text_color(p.text).child(item.label.clone()))
        .child(div().flex_none().max_w(px(220.)).truncate().text_size(px(12.)).text_color(if item.safe { p.accent } else { p.muted }).child(why))
        .child(div().flex_none().w(px(72.)).text_right().font_weight(FontWeight::MEDIUM).text_color(p.text2).child(size(item.bytes, item.partial)))
        .child(div().flex_none().w(px(28.)).children(action))
        .into_any_element()
}

/// The sheet that asks first: exactly what goes, and the total.
fn confirm(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let lang = crate::i18n::lang();
    let (items, total) = shell.disk.plan();
    let rows = items.iter().enumerate().map(|(n, item)| {
        h_flex()
            .px_3()
            .py_2()
            .gap_3()
            .items_center()
            .when(n > 0, |d| d.border_t_1().border_color(p.hair))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(h_flex().gap_2().items_center().child(div().truncate().text_size(px(UI)).text_color(p.text).child(item.label.clone())).child(div().flex_none().text_size(px(12.)).text_color(p.muted).child(label::disk_why(lang, item.why))))
                    .child(div().truncate().font_family(MONO).text_size(px(META - 0.5)).text_color(p.muted).child(item.path.display().to_string())),
            )
            .child(div().flex_none().text_size(px(UI)).font_weight(FontWeight::MEDIUM).text_color(p.text2).child(size(item.bytes, item.partial)))
    });
    v_flex()
        .gap_3()
        .child(div().text_size(px(16.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).child(t!("disk.confirm_title", count = items.len())))
        .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(t!("disk.confirm_text")))
        .child(v_flex().id("disk-plan").max_h(px(300.)).overflow_y_scroll().rounded(px(R_LG)).border_1().border_color(p.hair).children(rows))
        .child(
            h_flex()
                .px_3()
                .items_center()
                .child(div().flex_1().text_size(px(SMALL)).text_color(p.muted).child(t!("disk.total")))
                .child(div().text_size(px(UI)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(room::size(total))),
        )
        .child(
            h_flex()
                .pt_1()
                .gap_2()
                .justify_end()
                .child(ui::button(p, "disk-confirm-no", Kind::Secondary, t!("disk.cancel")).field().on_click(cx.listener(|this, _, _, cx| {
                    this.disk.back();
                    cx.notify();
                })))
                .child(ui::button(p, "disk-confirm-yes", Kind::Danger, t!("disk.confirm_yes", size = room::size(total))).field().on_click(cx.listener(|this, _, _, cx| this.confirm_disk(cx)))),
        )
        .into_any_element()
}

/// A stage that is a moment: a mark, a line, words, a button.
fn centered(p: Palette, id: &'static str, mark: AnyElement, title: String, text: Option<String>, button: Option<AnyElement>) -> AnyElement {
    let inner = v_flex()
        .w_full()
        .items_center()
        .gap_2()
        .child(mark)
        .child(div().pt_1().text_size(px(16.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).text_center().child(title))
        .children(text.map(|t| div().max_w(px(460.)).text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).text_center().child(t)))
        .children(button.map(|b| div().pt_3().child(b)));
    v_flex().min_h(px(200.)).justify_center().child(motion::rise(inner, id)).into_any_element()
}

// ── what the panel does ──

impl Shell {
    /// The banner's 정리하기, settings' 디스크, `/disk`, a news row: the panel opens and the daemon looks.
    pub fn open_disk(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        crate::tips::hide(cx);
        self.board.popup = None;
        if !self.disk.open {
            for command in self.disk.open() {
                self.send_command(command);
            }
        }
        cx.notify();
    }

    pub fn close_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Esc on the sheet that asks goes back to the list; on the list, it closes.
        if matches!(self.disk.stage, Stage::Confirm | Stage::Done(_)) {
            self.disk.back();
        } else {
            self.disk.close();
            if self.screen != crate::Screen::Settings {
                self.composer.focus(window, cx);
            }
        }
        cx.notify();
    }

    pub fn rescan_disk(&mut self, cx: &mut Context<Self>) {
        for command in self.disk.rescan() {
            self.send_command(command);
        }
        cx.notify();
    }

    pub fn confirm_disk(&mut self, cx: &mut Context<Self>) {
        for command in self.disk.confirm() {
            self.send_command(command);
        }
        cx.notify();
    }

    /// A command (or a path) to the clipboard, "복사됨" on its button for a moment.
    pub fn copy_disk(&mut self, text: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        self.disk.copied = Some((text.to_owned(), Instant::now()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
        cx.notify();
    }
}
