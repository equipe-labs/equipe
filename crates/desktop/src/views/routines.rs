//! 정기 업무, in 현황: the squad's routines — the person's regular work that
//! the PM is handed on its schedule (`protocol::routines`). A row each: on or
//! off, its name, when it runs and runs next, run now, change, delete (a
//! second click). Adding one starts from a preset or blank, in a form in the
//! list's place. Only the person's commands make, change or run one.

use equipe_core::protocol::Command;
use equipe_core::protocol::routines::{Routine, Schedule};
use equipe_core::routines::{Clock as _, Local};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::Icon;
use crate::theme::{R_LG, R_MD, SMALL};
use crate::{Shell, ui};

/// The form a routine is made or changed in.
pub struct Editor {
    /// `0`: a new one.
    pub id: u32,
    pub title: Entity<InputState>,
    pub prompt: Entity<TextareaState>,
    pub time: Entity<InputState>,
    pub weekly: bool,
    pub weekday: u8,
    pub on: bool,
    /// What kept it from being saved.
    pub problem: Option<String>,
}

#[derive(Default)]
pub struct RoutinesUi {
    pub editor: Option<Editor>,
    /// A delete clicked once: the next click on it deletes.
    pub deleting: Option<u32>,
}

/// A routine to start from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    WeeklyReport,
    Dependencies,
    Morning,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::WeeklyReport, Preset::Dependencies, Preset::Morning];

    fn title(self) -> &'static str {
        match self {
            Preset::WeeklyReport => t!("routines.preset_weekly"),
            Preset::Dependencies => t!("routines.preset_deps"),
            Preset::Morning => t!("routines.preset_morning"),
        }
    }

    fn prompt(self) -> &'static str {
        match self {
            Preset::WeeklyReport => t!("routines.preset_weekly_prompt"),
            Preset::Dependencies => t!("routines.preset_deps_prompt"),
            Preset::Morning => t!("routines.preset_morning_prompt"),
        }
    }

    fn when(self) -> Schedule {
        match self {
            Preset::WeeklyReport => Schedule::Weekly { weekday: 1, hour: 9, minute: 0 },
            Preset::Dependencies => Schedule::Weekly { weekday: 5, hour: 16, minute: 0 },
            Preset::Morning => Schedule::Daily { hour: 9, minute: 0 },
        }
    }
}

fn day_word(weekday: u8) -> &'static str {
    match weekday {
        1 => t!("routines.day_1"),
        2 => t!("routines.day_2"),
        3 => t!("routines.day_3"),
        4 => t!("routines.day_4"),
        5 => t!("routines.day_5"),
        6 => t!("routines.day_6"),
        _ => t!("routines.day_7"),
    }
}

/// `09:00` from an hour and a minute.
fn clock(hour: u8, minute: u8) -> String {
    format!("{hour:02}:{minute:02}")
}

/// `09:00`, `9:00`, `0900`, `9` — an hour and a minute, or `None`.
pub fn parse_time(typed: &str) -> Option<(u8, u8)> {
    let typed = typed.trim();
    let (h, m) = match typed.split_once(':') {
        Some((h, m)) => (h, m),
        None if typed.len() == 4 => typed.split_at(2),
        None => (typed, "0"),
    };
    let (hour, minute) = (h.trim().parse::<u8>().ok()?, m.trim().parse::<u8>().ok()?);
    (hour < 24 && minute < 60).then_some((hour, minute))
}

/// `매일 09:00`, `매주 월 09:00`.
pub fn schedule_words(when: Schedule) -> String {
    match when {
        Schedule::Daily { hour, minute } => t!("routines.daily", time = clock(hour, minute)),
        Schedule::Weekly { weekday, hour, minute } => t!("routines.weekly", day = day_word(weekday), time = clock(hour, minute)),
    }
}

/// When it runs next, as this Mac's clock reads it: `오늘 09:00`, `내일 09:00`, `10/24 (금) 16:00`.
pub fn next_words(at: u64, now: u64) -> String {
    let (when, today) = (Local.parts(at), Local.parts(now));
    let time = clock(when.hour, when.minute);
    let tomorrow = Local.parts(now + 24 * 60 * 60);
    let same = |a: equipe_core::routines::Parts, b: equipe_core::routines::Parts| (a.year, a.month, a.day) == (b.year, b.month, b.day);
    if same(when, today) {
        t!("routines.today", time = time)
    } else if same(when, tomorrow) {
        t!("routines.tomorrow", time = time)
    } else {
        format!("{}/{} ({}) {time}", when.month, when.day, day_word(when.weekday))
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The section: its head, the form or the rows, or what a routine is with presets.
pub fn section(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.overview.routines;
    let routines = &shell.model.routines;
    let head = h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(t!("routines.title")))
        .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(p.muted).child(t!("routines.subtitle")))
        .when(ui.editor.is_none(), |h| {
            h.child(ui::button(p, "routine-add", ui::Kind::Secondary, t!("routines.add")).icon(Icon::Plus).on_click(cx.listener(|this, _, window, cx| this.open_routine(None, None, window, cx))))
        });
    let body: AnyElement = match &ui.editor {
        Some(editor) => form(shell, editor, cx),
        None if routines.is_empty() => empty(shell, cx),
        None => {
            let now = now_secs();
            v_flex().w_full().rounded(px(R_LG)).border_1().border_color(p.hair).children(routines.iter().enumerate().map(|(i, r)| row(shell, r, i > 0, now, cx))).into_any_element()
        }
    };
    v_flex().w_full().gap_2().child(head).child(body).into_any_element()
}

/// None yet: what one is, and the presets to start from.
fn empty(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let presets = Preset::ALL.into_iter().map(|preset| {
        ui::button(p, SharedString::from(format!("routine-preset-{preset:?}")), ui::Kind::Secondary, preset.title()).on_click(cx.listener(move |this, _, window, cx| this.open_routine(None, Some(preset), window, cx)))
    });
    v_flex()
        .w_full()
        .p_4()
        .gap_3()
        .rounded(px(R_LG))
        .border_1()
        .border_color(p.hair)
        .child(div().text_size(px(13.)).line_height(px(20.)).text_color(p.text2).child(t!("routines.empty")))
        .child(h_flex().gap_2().flex_wrap().children(presets))
        .into_any_element()
}

fn row(shell: &Shell, r: &Routine, rule: bool, now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = r.id;
    let deleting = shell.overview.routines.deleting == Some(id);
    let mut when = schedule_words(r.when);
    if let Some(next) = r.next_run.filter(|_| r.on) {
        when = format!("{when} · {}", t!("routines.next", when = next_words(next, now)));
    }
    if let Some(last) = r.last_run {
        when = format!("{when} · {}", t!("routines.last", ago = crate::activity::ago(now.saturating_sub(last))));
    }
    let routine = r.clone();
    h_flex()
        .id(SharedString::from(format!("routine-{id}")))
        .w_full()
        .px_4()
        .py_3()
        .gap_3()
        .items_center()
        .when(rule, |r| r.border_t_1().border_color(p.hair))
        .child({
            let shell_handle = cx.entity().downgrade();
            ui::switch(p, format!("routine-on-{id}"), r.on).on_flip(move |on, _, cx| {
                let routine = Routine { on, ..routine.clone() };
                let _ = shell_handle.update(cx, |shell, cx| shell.save_routine(routine, cx));
            })
        })
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().truncate().text_size(px(13.5)).font_weight(FontWeight::MEDIUM).text_color(if r.on { p.text } else { p.muted }).child(r.title.clone()))
                .child(div().truncate().text_size(px(12.)).text_color(p.muted).child(when)),
        )
        .child(ui::button(p, SharedString::from(format!("routine-run-{id}")), ui::Kind::Ghost, t!("routines.run_now")).on_click(cx.listener(move |this, _, _, cx| this.run_routine(id, cx))))
        .child(ui::icon_button(p, SharedString::from(format!("routine-edit-{id}")), Icon::Pencil, t!("routines.edit"), None).on_click(cx.listener(move |this, _, window, cx| this.open_routine(Some(id), None, window, cx))))
        .child(if deleting {
            ui::button(p, SharedString::from(format!("routine-del-{id}")), ui::Kind::Danger, t!("routines.delete_sure")).on_click(cx.listener(move |this, _, _, cx| this.delete_routine(id, cx))).into_any_element()
        } else {
            ui::icon_button(p, SharedString::from(format!("routine-del-{id}")), Icon::X, t!("routines.delete"), None)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.overview.routines.deleting = Some(id);
                    cx.notify();
                }))
                .into_any_element()
        })
        .into_any_element()
}

fn form(shell: &Shell, editor: &Editor, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let field = |label: String, input: AnyElement| v_flex().w_full().gap_1().child(div().text_size(px(12.)).text_color(p.muted).child(label)).child(input);
    let boxed = |child: AnyElement| div().w_full().px_3().py_2().rounded(px(R_MD)).border_1().border_color(p.hair).bg(p.code_bg).text_size(px(13.)).child(child).into_any_element();
    let every = super::settings::segmented(
        shell,
        "routine-every",
        vec![(false, t!("routines.every_day").to_owned()), (true, t!("routines.every_week").to_owned())],
        editor.weekly,
        |this, weekly, _, cx| {
            if let Some(e) = this.overview.routines.editor.as_mut() {
                e.weekly = weekly;
            }
            cx.notify();
        },
        cx,
    );
    let days = editor.weekly.then(|| {
        h_flex().gap_1().children((1..=7u8).map(|d| {
            let on = editor.weekday == d;
            div()
                .id(SharedString::from(format!("routine-day-{d}")))
                .w(px(30.))
                .h(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(R_MD))
                .cursor_pointer()
                .text_size(px(12.))
                .when(on, |b| b.bg(p.primary).text_color(p.on_primary))
                .when(!on, |b| b.border_1().border_color(p.hair).text_color(p.text2).hover(move |s| s.bg(p.wash)))
                .child(day_word(d))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(e) = this.overview.routines.editor.as_mut() {
                        e.weekday = d;
                    }
                    cx.notify();
                }))
        }))
    });
    // As tall as the segments beside it.
    let time = div()
        .w(px(76.))
        .h(px(30.))
        .px_2()
        .flex()
        .items_center()
        .overflow_hidden()
        .rounded(px(R_MD))
        .border_1()
        .border_color(p.hair)
        .bg(p.code_bg)
        .text_size(px(13.))
        .child(Input::new(&editor.time).appearance(false).bordered(false));
    let when = h_flex().gap_3().flex_wrap().items_center().child(every).children(days).child(time);
    let problem = editor.problem.clone().map(|why| div().text_size(px(SMALL)).text_color(p.danger()).child(why));
    v_flex()
        .w_full()
        .p_4()
        .gap_3()
        .rounded(px(R_LG))
        .border_1()
        .border_color(p.hair)
        .child(field(t!("routines.name").into(), boxed(Input::new(&editor.title).appearance(false).bordered(false).into_any_element())))
        .child(field(t!("routines.what").into(), boxed(Textarea::new(&editor.prompt).appearance(false).bordered(false).into_any_element())))
        .child(field(t!("routines.when").into(), when.into_any_element()))
        .children(problem)
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(div().flex_1().text_size(px(12.)).text_color(p.muted).child(t!("routines.form_hint")))
                .child(ui::button(p, "routine-cancel", ui::Kind::Ghost, t!("routines.cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.overview.routines.editor = None;
                    cx.notify();
                })))
                .child(ui::button(p, "routine-save", ui::Kind::Primary, t!("routines.save")).on_click(cx.listener(|this, _, _, cx| this.save_routine_form(cx)))),
        )
        .into_any_element()
}

impl Shell {
    /// The form: routine `id`'s to change, or a new one — from `preset`, or blank.
    pub fn open_routine(&mut self, id: Option<u32>, preset: Option<Preset>, window: &mut Window, cx: &mut Context<Self>) {
        let existing = id.and_then(|id| self.model.routines.iter().find(|r| r.id == id).cloned());
        let (title, prompt, when, on) = match (&existing, preset) {
            (Some(r), _) => (r.title.clone(), r.prompt.clone(), r.when, r.on),
            (None, Some(p)) => (p.title().to_owned(), p.prompt().to_owned(), p.when(), true),
            (None, None) => (String::new(), String::new(), Schedule::Weekly { weekday: 1, hour: 9, minute: 0 }, true),
        };
        let (hour, minute) = when.hour_minute();
        let title_input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("routines.name_placeholder")));
        title_input.update(cx, |s, cx| s.set_value(title, window, cx));
        let prompt_input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 10).submit_on_enter(false).placeholder(t!("routines.what_placeholder")));
        prompt_input.update(cx, |s, cx| s.set_value(prompt, window, cx));
        let time = cx.new(|cx| InputState::new(window, cx).placeholder("09:00"));
        time.update(cx, |s, cx| s.set_value(clock(hour, minute), window, cx));
        let weekday = if let Schedule::Weekly { weekday, .. } = when { weekday } else { 1 };
        self.overview.routines.editor = Some(Editor { id: existing.map_or(0, |r| r.id), title: title_input.clone(), prompt: prompt_input, time, weekly: matches!(when, Schedule::Weekly { .. }), weekday, on, problem: None });
        self.overview.routines.deleting = None;
        title_input.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// The form's routine to the daemon, when it reads right.
    fn save_routine_form(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.overview.routines.editor.as_mut() else { return };
        let title = editor.title.read(cx).value().trim().to_owned();
        let prompt = editor.prompt.read(cx).value().trim().to_owned();
        let time = parse_time(&editor.time.read(cx).value());
        let problem = match (&time, title.is_empty(), prompt.is_empty()) {
            (_, true, _) => Some(t!("routines.need_name")),
            (_, _, true) => Some(t!("routines.need_what")),
            (None, _, _) => Some(t!("routines.need_time")),
            _ => None,
        };
        if let Some(why) = problem {
            editor.problem = Some(why.to_owned());
            return cx.notify();
        }
        let (hour, minute) = time.unwrap_or_default();
        let when = if editor.weekly { Schedule::Weekly { weekday: editor.weekday, hour, minute } } else { Schedule::Daily { hour, minute } };
        let routine = Routine { id: editor.id, title, prompt, when, on: editor.on, last_run: None, next_run: None };
        self.overview.routines.editor = None;
        self.save_routine(routine, cx);
    }

    pub fn save_routine(&mut self, routine: Routine, cx: &mut Context<Self>) {
        self.send_command(Command::SaveRoutine { squad: self.model.squad.clone(), routine });
        cx.notify();
    }

    fn run_routine(&mut self, id: u32, cx: &mut Context<Self>) {
        self.send_command(Command::RunRoutine { squad: self.model.squad.clone(), id });
        let name = self.model.routines.iter().find(|r| r.id == id).map(|r| r.title.clone()).unwrap_or_default();
        self.composer.flash = Some(t!("routines.ran", name = name));
        cx.notify();
    }

    fn delete_routine(&mut self, id: u32, cx: &mut Context<Self>) {
        self.overview.routines.deleting = None;
        self.send_command(Command::DeleteRoutine { squad: self.model.squad.clone(), id });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's own `test` would stand in for the standard one.
    use super::{Local, Schedule, next_words, parse_time, schedule_words};
    use equipe_core::routines::Clock as _;

    #[test]
    fn a_time_reads_as_it_is_typed() {
        assert_eq!(parse_time("09:00"), Some((9, 0)));
        assert_eq!(parse_time(" 9:5 "), Some((9, 5)));
        assert_eq!(parse_time("1630"), Some((16, 30)));
        assert_eq!(parse_time("7"), Some((7, 0)));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("9:60"), None);
        assert_eq!(parse_time("아침"), None);
    }

    #[test]
    fn a_routine_says_when_it_runs() {
        assert_eq!(schedule_words(Schedule::Weekly { weekday: 1, hour: 9, minute: 0 }), "매주 월 09:00");
        assert_eq!(schedule_words(Schedule::Daily { hour: 18, minute: 30 }), "매일 18:30");
        let now = Local.unix(equipe_core::routines::Parts { year: 2026, month: 10, day: 19, weekday: 1, hour: 8, minute: 0 });
        assert_eq!(next_words(now + 3600, now), "오늘 09:00");
        assert_eq!(next_words(now + 25 * 3600, now), "내일 09:00");
        assert_eq!(next_words(now + 4 * 24 * 3600 + 8 * 3600, now), "10/23 (금) 16:00");
    }
}
