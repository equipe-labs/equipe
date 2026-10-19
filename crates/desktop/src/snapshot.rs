//! Dev tool (`--features snapshot`): drive the window from a script and write
//! what it painted, so a change can be looked at — and a stream watched — without
//! a screen-capture or UI-automation permission.
//!
//! `EQUIPE_SCRIPT="wait:1500;say:안녕;wait:800;shot:/tmp/a.png"` — steps split on `;`:
//! `wait:<ms>` · `shot:<png>` · `say:<text>` (type it in the box and send) ·
//! `memo:<text>` (send it as a memo, as Tab does) · `type:<text>` (type, don't send) · `key:<y|a|n|o>` (answer a request) · `esc` (stop the member) · `up`/`down`
//! (history; ↑ on an empty box reaches the bubbles waiting) · `pwithdraw` (take the last waiting bubble back) · `pnow` (send what waits now) · `project:<name|all>` · `shead:<project>` / `sfold:<project>` (a sidebar header's name, its chevron; empty: the shared folder) · `select:<member>` · `board` · `chat` ·
//! `menu:<inbox|working|all|view0…>` (a sidebar item) · `cmenu:<attach|recent|to|model|effort|mode|context>` (a composer menu) · `bkey:<key>` (a work
//! screen's key; `shift-a`, `cmd-a`, `alt-v` with modifiers) · `pick:<n>` (a
//! ticket; `ppick:<n>`, peeking) · `find:<words>` · `note:<text>` (in the open words box) · `submit`
//! (its main button) · `size:<w>x<h>` (the window) · `hover:<x>,<y>` / `click:<x>,<y>` (the pointer, in the screen's points — through the zoom, as the Mac sends it) · `zoom:<pct|in|out|actual>` (the window's size, as ⌘+ ⌘- ⌘0) · `theme:<light|dark>` · `sidebar` (hide/show) · `popup:account` · `popup:subagents:<member>` · `subagent:<member>` (its first subagent's row, clicked) · `settings[:<category>]` (settings, at general · engines · squad · team · permissions · projects · browser · keys · about) · `eng:<signin|cancel|signout|sure|keep|tab>:<codex|claude>` (an AI platform's card) · `ssearch:<words>` (its search) · `spop:<language|squad>` (a pop-up) · `open:<n>` (a ticket's sidebar row) · `folds` · `elsewhere[:<n>]` (open or close the nth fold of other projects' talk) · `chooser` (the machine chooser) · `connect:<host>` · `pass:<text>` (answer ssh's
//! password box) · `trust` / `decline` (answer ssh's yes/no) · `retry` · `attach:<file>[,<file>…]` · `ckey:<key>` (a key in the composer) · `light:<n>` (show attachment n big) · `drag:<path>[,…]` / `undrag` (the drop frame, as if dragged) ·
//! `sdrag:<x1>,<y1>,<x2>,<y2>` (a selection dragged out) · `peekw:<px>` (the peek held that wide) · `quote:<open|copy|type:<words>|send|close|jump>` (asking about the words selected) ·
//! `turn:<row>[,<call>]` (open or close a group of calls, or one call in it) · `side:<n>` (ticket n beside the conversation) ·
//! `answer:<ticket>,<n>,<words>` / `aok:<ticket>,<n>` / `atick:<ticket>,<n>` / `apick:<ticket>,<n>,<done|yours|later|call|unknown>` / `anote:<ticket>,<n>,<words>` / `afree:<ticket>,<words>` / `asend:<ticket>` / `asent:<ticket>` (a waiting ticket's card: type in box n, its quick OK, tick thing n off, a pick, a note, the words to the member, send; open what was sent) ·
//! `dkey:<key>` (a member's questions' card: `1`–`9`, `up`, `down`, `space`, `enter`, `cmd-enter`, `tab`, `left`, `escape`) · `dtype:<words>` (in its box for the person's own words) · `decided` (open the latest decided row) · `dpick:<id>` (a set in 나에게, peeking) · `dview:<n>` (option n's picture of the card on show, big) · `tview:<ticket>,<n>` (a hand-in's nth picture, big; then `press:right`, `press:enter`) · `rview:<n>` (the nth picture the latest reply showing pictures embeds, big) · `tpick:<id>` (a change to the team in 나에게, peeking) · `tyes` / `tno` (approve or decline the oldest change to the team) · `cpick:<id>` (a card to confirm in 나에게, peeking; `cpick` alone: the oldest) · `ctick:<ticket>` (tick or untick a ticket on the oldest card) · `cyes` / `cno` (confirm or decline the oldest card) · `eyes` / `eno` / `eundo` (the engine card: its yes, its no, undo) · `engines:<off|suggest|auto>` (the squad's engine switching) ·
//! `qopen` · `qtype:<n>,<words>` · `qsay:<n>,<words>` · `qsend` · `qesc` · `qenter` (a reply's list of questions as a form: open it, type in box n — past the items, the one for anything to add — tap a quick answer, send, cancel; ⏎ in the composer) ·
//! `design[:<overview|colors|type|spacing|radius|shadow|motion|assets|guidelines>]` (디자인 시스템, at a section) · `dsource:<squad|project:name>` (another system) · `ddraft` (its draft, or back) · `dchanges` (what changed, opened or closed) · `dlight:<n>` / `dlight:off` (asset n big) · `dplay:<token>` (a motion token played, as under the pointer) · `dask` (ask the designer for one) · `dtoc:<n>` (the guidelines' nth heading) ·
//! `mark:<label>` (a line in the log) · `scroll:<n>,<dy>` (n wheel turns a frame apart) · `wheel:<x>,<y>,<n>,<dy>` (the same with the pointer at `x,y`) · `browser` (the browser panel, ⌘⇧B) · `doc:<path>` (a document in the side panel) · `dfold[:open]` (fold it to a rail, or open it again) · `term[:<line>]` (the terminal, with a line typed at its prompt) · `tkeys:<text>` (typed into it, `~` for Enter) · `tmove` (the terminal to the bottom, or back to the right) · `bpick:<n>` (its nth tab) · `bclose:<n>` (its ✕) · `bwheel` (a wheel over the page) · `btake` / `bgive` (its control bar: 직접 하기, 돌려주기) · `breveal:<page>` (a page waiting for the person, as 나에게 opens it) · `bclick:<fx>,<fy>` (a click on the page, at fractions of its picture) · `bkeys:<text>` (typed key by key, `~` for Enter) · `bime:ㅎ|하|한` (composed, then committed) · `press:<keystroke>` (one key through the window: `escape`, `cmd-,`) · `bopen:<url>` (a tab of the person's own) · `paste` (⌘V with what the clipboard holds) · `pair` (휴대폰 연결's sheet) · `pair:allow|deny|retry|close` (its buttons) · `disk` (the 디스크 panel) · `disk:fix<n>|all|yes|back|rescan|close` (a fix's sheet, every fix's, its yes, back, look again, close) · `bell:<sheet|notasked|denied|allowed>` (the notifications sheet; what macOS allows) · `quit`. Without a script, one shot after `EQUIPE_SNAPSHOT_MS`
//! (default 3000) to `EQUIPE_SNAPSHOT`.

use std::time::Duration;

use gpui_kit::*;

use crate::Shell;

pub fn run(handle: AnyWindowHandle, shell: Entity<Shell>, cx: &mut App) {
    let script = match (std::env::var("EQUIPE_SCRIPT"), std::env::var("EQUIPE_SNAPSHOT")) {
        (Ok(script), _) => script,
        (_, Ok(path)) => format!("wait:{};shot:{path};quit", std::env::var("EQUIPE_SNAPSHOT_MS").unwrap_or_else(|_| "3000".into())),
        _ => return,
    };
    let started = std::time::Instant::now();
    cx.spawn(async move |cx| {
        for step in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let (verb, arg) = step.split_once(':').unwrap_or((step, ""));
            let arg = arg.to_owned();
            if verb == "wait" {
                cx.background_executor().timer(Duration::from_millis(arg.parse().unwrap_or(500))).await;
                continue;
            }
            if verb == "quit" {
                cx.update(|cx| cx.quit());
                return;
            }
            // A line in the log the measurements are cut at (`docs/PERF-2026-10.md`),
            // with the process's CPU time so far and the clock, to divide one by the other.
            if verb == "mark" {
                eprintln!("perf mark {arg} cpu {}ms at {}ms", process_cpu().as_millis(), started.elapsed().as_millis());
                continue;
            }
            // `scroll:<n>,<dy>`: n turns of the wheel, a frame apart, by dy points
            // (up is positive), at the window's middle — as a trackpad sends them.
            // `wheel:<x>,<y>,<n>,<dy>`: the same at `x,y`, in the screen's points
            // (through the zoom, as `hover:` is).
            if verb == "scroll" || verb == "wheel" {
                let n: Vec<f32> = arg.split(',').map(|v| v.trim().parse().unwrap_or(f32::NAN)).collect();
                let (at, n, dy) = match (verb, &n[..]) {
                    ("wheel", [x, y, rest @ ..]) => (Some(point(px(*x), px(*y))), rest.first().copied(), rest.get(1).copied()),
                    (_, [n, dy, ..]) => (None, Some(*n), Some(*dy)),
                    _ => (None, None, None),
                };
                let n = n.filter(|n| n.is_finite()).map_or(60, |n| n as usize);
                let dy = dy.filter(|d| d.is_finite()).unwrap_or(-40.);
                for _ in 0..n {
                    let _ = cx.update_window(handle, |_, window, cx| {
                        let size = window.viewport_size();
                        let at = at.unwrap_or_else(|| point(size.width * 0.45, size.height * 0.5));
                        let wheel = ScrollWheelEvent { position: at, delta: ScrollDelta::Pixels(point(px(0.), px(dy))), modifiers: Modifiers::default(), touch_phase: TouchPhase::Moved };
                        let wheel = window.input_from_platform(PlatformInput::ScrollWheel(wheel));
                        window.dispatch_event(wheel, cx);
                    });
                    cx.background_executor().timer(Duration::from_millis(16)).await;
                }
                continue;
            }
            let shell = shell.clone();
            let verb = verb.to_owned();
            let started = std::time::Instant::now();
            let timed = verb.clone();
            let _ = cx.update_window(handle, move |_, window, cx| match verb.as_str() {
                "shot" => match window.render_to_image() {
                    Ok(image) => eprintln!("shot {arg}: {:?}", image.save(&arg)),
                    Err(err) => eprintln!("shot failed: {err}"),
                },
                "say" | "type" | "memo" => shell.update(cx, |shell, cx| {
                    shell.composer.set_text(&arg, window, cx);
                    shell.on_composer_event(&gpui_kit::component::input::InputEvent::Change, window, cx);
                    if verb == "say" {
                        shell.send(window, cx);
                    } else if verb == "memo" {
                        shell.send_as(true, window, cx);
                    }
                }),
                "key" => shell.update(cx, |shell, cx| {
                    shell.on_approval_key(&arg, false, cx);
                }),
                "esc" => shell.update(cx, |shell, cx| shell.interrupt(cx)),
                "up" | "down" => shell.update(cx, |shell, cx| {
                    shell.on_composer_key(&verb, &Modifiers::default(), window, cx);
                }),
                "project" => shell.update(cx, |shell, cx| shell.focus_project((arg != "all").then(|| arg.clone()), window, cx)),
                "shead" | "sfold" => shell.update(cx, |shell, cx| {
                    let click = if verb == "shead" { crate::board::HeadClick::Name } else { crate::board::HeadClick::Chevron };
                    shell.side_head_click(click, (!arg.is_empty()).then(|| arg.clone()), window, cx);
                }),
                "select" => shell.update(cx, |shell, cx| {
                    if let Some(m) = shell.model.members.iter().find(|m| m.name == arg) {
                        let session = m.session;
                        shell.select_member(session, window, cx);
                    }
                }),
                "bkey" => shell.update(cx, |shell, cx| {
                    let mut modifiers = Modifiers::default();
                    let mut key = arg.as_str();
                    while let Some((m, rest)) = key.split_once('-').filter(|(_, rest)| !rest.is_empty()) {
                        match m {
                            "shift" => modifiers.shift = true,
                            "cmd" => modifiers.platform = true,
                            "alt" => modifiers.alt = true,
                            "ctrl" => modifiers.control = true,
                            _ => break,
                        }
                        key = rest;
                    }
                    shell.on_board_key(key, &modifiers, window, cx);
                }),
                "menu" => shell.update(cx, |shell, cx| {
                    let menu = match arg.as_str() {
                        "inbox" => crate::board::Menu::Inbox,
                        "working" => crate::board::Menu::Working,
                        "all" => crate::board::Menu::All,
                        view => crate::board::Menu::View(view.trim_start_matches("view").parse().unwrap_or(0)),
                    };
                    shell.open_side_menu(menu, cx);
                }),
                "size" => {
                    if let Some((w, h)) = arg.split_once('x').and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))) {
                        window.resize(size(px(w), px(h)));
                    }
                }
                "pick" => shell.update(cx, |shell, cx| shell.board_pick(crate::state::Need::Ticket(arg.parse().unwrap_or(0)), cx)),
                "find" => shell.update(cx, |shell, cx| shell.board_find(&arg, cx)),
                "note" => shell.update(cx, |shell, cx| shell.board_type(&arg, window, cx)),
                "submit" => shell.update(cx, |shell, cx| shell.board_submit(window, cx)),
                "board" => shell.update(cx, |shell, cx| shell.show(crate::Screen::Board, cx)),
                "chat" => shell.update(cx, |shell, cx| shell.show(crate::Screen::Chat, cx)),
                "folds" => shell.update(cx, |shell, cx| shell.toggle_folds(cx)),
                "chooser" => shell.update(cx, |shell, cx| shell.open_chooser(window, cx)),
                "connect" => shell.update(cx, |shell, cx| shell.connect_typed(&arg, window, cx)),
                "pass" => shell.update(cx, |shell, cx| {
                    shell.ask.input.update(cx, |s, cx| s.set_value(arg.clone(), window, cx));
                    shell.submit_secret(window, cx);
                }),
                "trust" => shell.update(cx, |shell, cx| shell.answer_prompt(crate::askpass::Answer::Yes, window, cx)),
                "decline" => shell.update(cx, |shell, cx| shell.answer_prompt(crate::askpass::Answer::Cancel, window, cx)),
                "attach" => shell.update(cx, |shell, cx| shell.attach_sources(arg.split(',').map(|p| crate::attach::Source::File(p.into())).collect(), cx)),
                "cmenu" => shell.update(cx, |shell, cx| {
                    use crate::views::composer::MenuKind as K;
                    let kind = match arg.as_str() {
                        "model" => K::Model,
                        "effort" => K::Effort,
                        "mode" => K::Mode,
                        "to" => K::To,
                        "context" => K::Context,
                        "recent" => K::Recent,
                        _ => K::Attach,
                    };
                    shell.open_menu(kind, window, cx);
                }),
                "ckey" => shell.update(cx, |shell, cx| {
                    let taken = shell.on_composer_key(&arg, &Modifiers::default(), window, cx);
                    eprintln!("ckey {arg}: taken = {taken}");
                }),
                // What waits for the member: the last bubble taken back, or all of it sent now.
                "pwithdraw" => shell.update(cx, |shell, cx| {
                    let last = shell.chat.blocks.iter().rev().find_map(|b| if let crate::convo::Block::Pending { text, .. } = b { Some(text.clone()) } else { None });
                    if let Some(text) = last {
                        shell.withdraw_pending(text, window, cx);
                    }
                }),
                "pnow" => shell.update(cx, |shell, cx| shell.send_pending_now(cx)),
                "light" => shell.update(cx, |shell, cx| {
                    let id = shell.composer.strip.items().get(arg.parse::<usize>().unwrap_or(0)).map(|i| i.id);
                    shell.show_big(id, cx);
                    cx.notify();
                }),
                "drag" => shell.update(cx, |shell, cx| {
                    let paths: Vec<std::path::PathBuf> = arg.split(',').map(Into::into).collect();
                    shell.drag_moved(&ExternalPaths(paths.into()), cx);
                    shell.composer.drag_shown = true;
                    cx.notify();
                }),
                "pastelong" => shell.update(cx, |shell, cx| {
                    let lines = arg.parse::<usize>().unwrap_or(60);
                    let text: String = (1..=lines).map(|n| format!("{n}번째 줄: 로그 내용이 여기에 있습니다\n")).collect();
                    let taken = shell.paste_item(&ClipboardItem::new_string(text), cx);
                    eprintln!("pastelong: taken = {taken}");
                }),
                "theme" => shell.update(cx, |shell, cx| {
                    let appearance = if arg == "light" { WindowAppearance::Light } else { WindowAppearance::Dark };
                    shell.palette = crate::theme::Palette::new(appearance);
                    shell.palette.apply_to_kit(appearance, cx);
                    // As the app does on a change of appearance: the browser panel too.
                    shell.sync_browser(window, cx);
                    cx.notify();
                }),
                "undrag" => shell.update(cx, |shell, cx| {
                    shell.composer.drag_shown = false;
                    cx.notify();
                }),
                "paste" => shell.update(cx, |shell, cx| {
                    let taken = cx.read_from_clipboard().is_some_and(|item| shell.paste_item(&item, cx));
                    eprintln!("paste: images taken = {taken}");
                }),
                "retry" => shell.update(cx, |shell, cx| shell.retry(cx)),
                // 디자인 시스템: the screen at a section, another system, its draft, what changed, an asset big.
                // 현황; `overview:<project>` marks that project the HQ here first (no daemon asked).
                "overview" => shell.update(cx, |shell, cx| {
                    if !arg.is_empty() {
                        shell.model.hq = shell.model.projects.iter().find(|p| equipe_core::board::project_name(p) == arg).cloned();
                        shell.model.order_projects();
                    }
                    shell.open_overview(cx);
                }),
                // Two routines on this screen alone (no daemon asked), the second off.
                "rseed" => shell.update(cx, |shell, cx| {
                    use equipe_core::protocol::routines::{Routine, Schedule};
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                    shell.model.routines = vec![
                        Routine { id: 1, title: "주간 리포트".into(), prompt: "-".into(), when: Schedule::Weekly { weekday: 1, hour: 9, minute: 0 }, on: true, last_run: Some(now - 6 * 86_400), next_run: Some(now + 86_400) },
                        Routine { id: 2, title: "의존성 점검".into(), prompt: "-".into(), when: Schedule::Weekly { weekday: 5, hour: 16, minute: 0 }, on: false, last_run: None, next_run: Some(now + 4 * 86_400) },
                    ];
                    cx.notify();
                }),
                // The routine form: `rnew` blank, `rnew:weekly|deps|morning` from a preset.
                "rnew" => shell.update(cx, |shell, cx| {
                    use crate::views::routines::Preset;
                    let preset = match arg.as_str() {
                        "weekly" => Some(Preset::WeeklyReport),
                        "deps" => Some(Preset::Dependencies),
                        "morning" => Some(Preset::Morning),
                        _ => None,
                    };
                    shell.open_routine(None, preset, window, cx);
                }),
                // The sidebar this wide: `sidew:210`.
                "sidew" => shell.update(cx, |shell, cx| {
                    shell.board.sidebar_width = arg.parse().unwrap_or(crate::views::sidebar::WIDTH);
                    cx.notify();
                }),
                "design" => shell.update(cx, |shell, cx| {
                    shell.open_design(cx);
                    if let Some(section) = crate::design::Section::parse(&arg) {
                        shell.design_section(section, cx);
                    }
                }),
                "dsource" => shell.update(cx, |shell, cx| shell.design_pick_source(&arg, cx)),
                "ddraft" => shell.update(cx, |shell, cx| shell.design_toggle_draft(cx)),
                "dchanges" => shell.update(cx, |shell, cx| {
                    shell.design.changes_open = !shell.design.changes_open;
                    cx.notify();
                }),
                "dlight" => shell.update(cx, |shell, cx| shell.design_lightbox(arg.parse().ok(), cx)),
                "dplay" => shell.update(cx, |shell, cx| {
                    shell.design.plays += 1;
                    shell.design.playing = Some((arg.clone(), shell.design.plays));
                    cx.notify();
                }),
                "dask" => shell.update(cx, |shell, cx| shell.design_ask_designer(window, cx)),
                "dtoc" => shell.update(cx, |shell, cx| {
                    let n: usize = arg.parse().unwrap_or(0);
                    let section = shell.design.current().and_then(|s| s.guidelines.as_deref()).map(crate::design::headings).and_then(|h| h.get(n).map(|h| h.section));
                    if let Some(section) = section {
                        shell.design.toc_at = n;
                        shell.design.scroll.scroll_to_item(section);
                        cx.notify();
                    }
                }),
                // 휴대폰 연결: the sheet, and its buttons.
                "pair" => shell.update(cx, |shell, cx| match arg.as_str() {
                    "allow" => shell.answer_pairing(true, cx),
                    "deny" => shell.answer_pairing(false, cx),
                    "retry" => shell.retry_pairing(cx),
                    "close" => shell.close_pairing(window, cx),
                    _ => shell.open_pairing(window, cx),
                }),
                // 디스크: the panel, a fix's sheet, every fix's, yes, back, close.
                "disk" => shell.update(cx, |shell, cx| {
                    match arg.as_str() {
                        "all" => shell.disk.ask_all(),
                        "yes" => return shell.confirm_disk(cx),
                        "back" => shell.disk.back(),
                        "close" => return shell.close_disk(window, cx),
                        "rescan" => return shell.rescan_disk(cx),
                        fix if fix.starts_with("fix") => {
                            let n: usize = fix["fix".len()..].parse().unwrap_or(0);
                            let ids = shell.disk.report.as_ref().and_then(|r| crate::disk::fixes(r).get(n).map(|f| f.ids.clone())).unwrap_or_default();
                            shell.disk.ask(&ids);
                        }
                        _ => return shell.open_disk(window, cx),
                    }
                    cx.notify();
                }),
                // The conversation's: light or dark whatever the system says,
                // ⌘F with words, a row scrolled to.
                "appearance" => shell.update(cx, |shell, cx| {
                    let appearance = if arg == "light" { WindowAppearance::Light } else { WindowAppearance::Dark };
                    shell.palette = crate::theme::Palette::new(appearance);
                    shell.palette.apply_to_kit(appearance, cx);
                    cx.notify();
                }),
                "seek" => shell.update(cx, |shell, cx| {
                    shell.find_open(window, cx);
                    shell.find_type(&arg, window, cx);
                }),
                "row" => shell.update(cx, |shell, cx| shell.chat.scroll_to(arg.parse().unwrap_or(0), cx)),
                "turn" => shell.update(cx, |shell, cx| {
                    let (ix, call) = arg.split_once(',').map_or((arg.as_str(), None), |(ix, call)| (ix, call.parse().ok()));
                    shell.chat_turn(ix.parse().unwrap_or(0), call, cx);
                }),
                "side" => shell.update(cx, |shell, cx| shell.side_open(arg.parse().unwrap_or(0), cx)),
                // A waiting ticket's card: `answer:117,1,words` types in its first box,
                // `aok:117,3` presses the quick OK, `atick:55,1` ticks a thing off,
                // `apick:117,2,yours` picks (done, yours, later, call, unknown),
                // `anote:117,2,words` opens a note and types in it, `afree:117,words`
                // types the words to the member, `asend:117` sends it.
                "answer" | "aok" | "atick" | "apick" | "anote" | "afree" | "asend" => shell.update(cx, |shell, cx| {
                    let mut parts = arg.splitn(3, ',');
                    let ticket: u32 = parts.next().and_then(|t| t.parse().ok()).unwrap_or(0);
                    let n: usize = parts.next().and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).saturating_sub(1);
                    let words = parts.next().unwrap_or_default().to_owned();
                    match verb.as_str() {
                        "answer" => shell.turn_say(ticket, n, &words, window, cx),
                        "aok" => shell.turn_say(ticket, n, equipe_core::t!("turn.ok"), window, cx),
                        "atick" => shell.turn_pick(ticket, n, crate::yourturn::Pick::Done, window, cx),
                        "apick" => {
                            use crate::yourturn::Pick;
                            let pick = match words.as_str() {
                                "done" => Pick::Done,
                                "yours" => Pick::Yours,
                                "later" => Pick::Later,
                                "call" => Pick::YourCall,
                                _ => Pick::DontKnow,
                            };
                            shell.turn_pick(ticket, n, pick, window, cx);
                        }
                        "anote" => shell.turn_note_say(ticket, n, &words, window, cx),
                        "afree" => {
                            let words = arg.split_once(',').map(|(_, w)| w).unwrap_or_default();
                            shell.turn_free(ticket, words, window, cx);
                        }
                        _ => shell.turn_send(ticket, window, cx),
                    }
                }),
                // A member's questions: the card's keys, and words of the person's own.
                "dkey" => shell.update(cx, |shell, cx| {
                    let taken = shell.decide_script_key(&arg, window, cx);
                    eprintln!("dkey {arg}: taken = {taken}");
                }),
                "dtype" => shell.update(cx, |shell, cx| shell.decide_type(&arg, window, cx)),
                // A member's pictures big: option n's of the card on show, or a hand-in's nth.
                "dview" => shell.update(cx, |shell, cx| shell.view_script(None, arg.parse().unwrap_or(1), cx)),
                "rview" => shell.update(cx, |shell, cx| shell.reply_view_script(arg.parse().unwrap_or(1), cx)),
                // The big view zoomed to `vzoom:2` at its centre.
                "vzoom" => shell.update(cx, |shell, cx| {
                    if let Some(path) = shell.viewer.as_ref().map(|v| v.item().path.clone()) {
                        shell.viewer_zoom.set(&path, arg.parse().unwrap_or(2.), (0., 0.));
                        cx.notify();
                    }
                }),
                "tview" => shell.update(cx, |shell, cx| {
                    let (ticket, n) = arg.split_once(',').unwrap_or((arg.as_str(), "1"));
                    shell.view_script(ticket.parse().ok(), n.parse().unwrap_or(1), cx);
                }),
                "decided" => shell.update(cx, |shell, cx| shell.chat_open_decided(cx)),
                "asent" => shell.update(cx, |shell, cx| shell.chat_open_turn(arg.parse().unwrap_or(0), cx)),
                // A ticket picked, peeking (Space on its row).
                "ppick" => shell.update(cx, |shell, cx| {
                    shell.board.view.peek = true;
                    shell.board_pick(crate::state::Need::Ticket(arg.parse().unwrap_or(0)), cx);
                }),
                "dpick" => shell.update(cx, |shell, cx| {
                    shell.board.view.peek = true;
                    shell.board_pick(crate::state::Need::Questions(arg.parse().unwrap_or(0)), cx);
                }),
                // A change to the team the PM proposed: picked in 나에게 (peeking), or answered.
                "tpick" => shell.update(cx, |shell, cx| {
                    shell.board.view.peek = true;
                    shell.board_pick(crate::state::Need::Team(arg.parse().unwrap_or(0)), cx);
                }),
                "tyes" | "tno" => shell.update(cx, |shell, cx| {
                    if let Some(id) = shell.model.team.first().map(|c| c.id) {
                        shell.team_decide(id, verb == "tyes", cx);
                    }
                }),
                // Work to confirm: picked in 나에게 (peeking), a ticket ticked off, or answered.
                "cpick" => shell.update(cx, |shell, cx| {
                    let id = arg.parse().ok().or_else(|| shell.model.accepts.ids().first().copied()).unwrap_or(0);
                    shell.board.view.peek = true;
                    shell.board_pick(crate::state::Need::Accept(id), cx);
                }),
                "ctick" => shell.update(cx, |shell, cx| {
                    if let Some(id) = shell.model.accepts.ids().first().copied() {
                        shell.accept_toggle(id, arg.parse().unwrap_or(0), cx);
                    }
                }),
                "cyes" | "cno" => shell.update(cx, |shell, cx| {
                    if let Some(id) = shell.model.accepts.ids().first().copied() {
                        shell.accept_decide(id, verb == "cyes", cx);
                    }
                }),
                // The engine card answered; the squad's engine switching set.
                "eyes" | "eno" | "eundo" => shell.update(cx, |shell, cx| {
                    let answer = match verb.as_str() {
                        "eyes" => crate::switching::Answer::Apply,
                        "eundo" => crate::switching::Answer::Undo,
                        _ => crate::switching::Answer::Leave,
                    };
                    if let Some(id) = shell.model.engines.first().map(|p| p.id) {
                        shell.engines_decide(id, answer, cx);
                    }
                }),
                "engines" => shell.update(cx, |shell, cx| {
                    if let Some(mode) = equipe_core::protocol::engines::Switching::parse(&arg) {
                        shell.set_switching(mode, cx);
                    }
                }),
                // A reply's list of questions answered in place.
                "qopen" => shell.update(cx, |shell, cx| shell.asklist_open(None, window, cx)),
                "qtype" => shell.update(cx, |shell, cx| {
                    let (n, words) = arg.split_once(',').unwrap_or(("0", ""));
                    shell.asklist_type(n.parse().unwrap_or(0), words, window, cx);
                }),
                "qsay" => shell.update(cx, |shell, cx| {
                    let (n, words) = arg.split_once(',').unwrap_or(("0", ""));
                    shell.asklist_say(n.parse().unwrap_or(0), words, window, cx);
                }),
                "qsend" => shell.update(cx, |shell, cx| shell.asklist_send(window, cx)),
                "qesc" => shell.update(cx, |shell, cx| shell.asklist_cancel(window, cx)),
                "qenter" => shell.update(cx, |shell, cx| {
                    let took = shell.on_asklist_key("enter", &Modifiers::default(), window, cx);
                    eprintln!("qenter: {took:?}");
                }),
                "browser" => shell.update(cx, |shell, cx| shell.toggle_browser(window, cx)),
                "doc" => shell.update(cx, |shell, cx| shell.open_doc(std::path::PathBuf::from(&arg), cx)),
                "dfold" => shell.update(cx, |shell, cx| shell.fold_doc(arg != "open", cx)),
                "term" => shell.update(cx, |shell, cx| shell.open_terminal((!arg.is_empty()).then(|| arg.to_owned()), window, cx)),
                "tmove" => shell.update(cx, |shell, cx| shell.move_terminal(cx)),
                "tkeys" => shell.update(cx, |shell, _| {
                    if let Some(ui) = shell.terminal.as_ref() {
                        ui.term.write(arg.replace('~', "\r"));
                    }
                }),
                "bopen" => {
                    let panel = shell.read(cx).browser.clone();
                    panel.update(cx, |b, cx| b.open_tab(&arg, cx));
                }
                "bpick" => {
                    let panel = shell.read(cx).browser.clone();
                    panel.update(cx, |b, cx| b.pick(arg.parse().unwrap_or(1), window, cx));
                }
                // The control bar's button: 직접 하기 (`btake`) or 돌려주기 (`bgive`).
                // The nth tab's ✕, as the person presses it; a wheel over the page.
                "bclose" => {
                    let panel = shell.read(cx).browser.clone();
                    panel.update(cx, |b, cx| b.close_nth(arg.parse().unwrap_or(1), cx));
                }
                "bwheel" => {
                    let panel = shell.read(cx).browser.clone();
                    panel.update(cx, |b, cx| b.wheel(cx));
                }
                "btake" | "bgive" => {
                    let panel = shell.read(cx).browser.clone();
                    panel.update(cx, |b, cx| b.hand(verb == "btake", window, cx));
                }
                // A member's page waiting for the person, opened as 나에게 or a notification would.
                "breveal" => shell.update(cx, |shell, cx| shell.reveal_browser(arg.parse().unwrap_or(0), cx)),
                "bclick" => {
                    // A real press and release, where `fx,fy` (fractions) fall on the page's picture.
                    let panel = shell.read(cx).browser.clone();
                    let (fx, fy) = arg.split_once(',').map(|(x, y)| (x.parse().unwrap_or(0.5), y.parse().unwrap_or(0.5))).unwrap_or((0.5f32, 0.5f32));
                    match panel.read(cx).picture() {
                        Some(r) => {
                            let at = point(r.origin.x + r.size.width * fx, r.origin.y + r.size.height * fy);
                            let down = MouseDownEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: 1, first_mouse: false };
                            window.dispatch_event(PlatformInput::MouseDown(down), cx);
                            let up = MouseUpEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: 1 };
                            window.dispatch_event(PlatformInput::MouseUp(up), cx);
                        }
                        None => eprintln!("bclick: no picture yet"),
                    }
                }
                "bkeys" => {
                    // Typed key by key, as a keyboard would: `~` is Enter.
                    for c in arg.chars() {
                        let key = match c {
                            '~' => "enter".to_owned(),
                            ' ' => "space".to_owned(),
                            c => c.to_string(),
                        };
                        if let Ok(stroke) = Keystroke::parse(&key) {
                            window.dispatch_keystroke(stroke, cx);
                        }
                    }
                }
                // One keystroke through the window, as the keyboard sends it: `press:escape`, `press:cmd-,`.
                "press" => {
                    if let Ok(stroke) = Keystroke::parse(&arg) {
                        window.dispatch_keystroke(stroke, cx);
                    }
                }
                "bime" => {
                    let panel = shell.read(cx).browser.clone();
                    let steps: Vec<&str> = arg.split('|').collect();
                    panel.update(cx, |b, cx| b.compose(&steps, window, cx));
                }
                "sidebar" => shell.update(cx, |shell, cx| shell.toggle_sidebar(cx)),
                // The window's size: `zoom:150`, or a step as the keys take it.
                "zoom" => shell.update(cx, |shell, cx| match arg.as_str() {
                    "in" => shell.zoom(crate::zoom::Step::In, window, cx),
                    "out" => shell.zoom(crate::zoom::Step::Out, window, cx),
                    "actual" => shell.zoom(crate::zoom::Step::Actual, window, cx),
                    pct => shell.set_zoom(pct.trim_end_matches('%').parse().unwrap_or(crate::zoom::ACTUAL), window, cx),
                }),
                "popup" => shell.update(cx, |shell, cx| {
                    shell.board.popup = match arg.as_str() {
                        "account" => Some(crate::views::sidebar::Popup::Account),
                        // `popup:subagents:<member>`: all of a member's subagents, as "+N개 더" opens them.
                        other => other.strip_prefix("subagents:").and_then(|name| shell.model.members.iter().find(|m| m.name == name)).map(|m| crate::views::sidebar::Popup::Subagents(m.session)),
                    };
                    cx.notify();
                }),
                "subagent" => shell.update(cx, |shell, cx| {
                    let found = shell.model.members.iter().find(|m| m.name == arg).and_then(|m| shell.model.subagents_of(m.session).first().map(|(s, sub)| (m.session, *s, sub.id.clone())));
                    if let Some((member, session, id)) = found {
                        shell.open_subagent(member, session, &id, window, cx);
                    }
                }),
                // Settings, at a category (`settings:team`), or where it was left.
                "settings" => shell.update(cx, |shell, cx| shell.open_settings(crate::settings::Category::parse(&arg), window, cx)),
                // Notifications as macOS would have them (`bell:notasked|denied|allowed`), or the sheet asking (`bell:sheet`).
                "bell" => shell.update(cx, |shell, cx| {
                    match arg.as_str() {
                        "sheet" => shell.bell.sheet = true,
                        "notasked" => shell.bell.permission = crate::notify::Permission::NotAsked,
                        "denied" => shell.bell.permission = crate::notify::Permission::Denied,
                        "allowed" => shell.bell.permission = crate::notify::Permission::Allowed,
                        _ => {}
                    }
                    cx.notify();
                }),
                // An AI platform's card in settings: `eng:<signin|cancel|signout|sure|keep|tab>:<codex|claude>`
                // (로그인, 취소, 로그아웃 and its 로그아웃 or 유지; `tab` moves the keys' focus to its first button).
                "eng" => shell.update(cx, |shell, cx| {
                    let (act, which) = arg.split_once(':').unwrap_or((arg.as_str(), "codex"));
                    let engine = if which == "claude" { equipe_core::protocol::Engine::Claude } else { equipe_core::protocol::Engine::Codex };
                    match act {
                        "signin" => shell.engine_sign_in(engine, cx),
                        "cancel" => shell.engine_cancel(engine, cx),
                        "signout" => shell.engines.ask_sign_out(engine),
                        "sure" => shell.engine_sign_out(engine, cx),
                        "keep" => shell.engines.keep(engine),
                        "tab" => {
                            let at = crate::engines::PLATFORMS.iter().position(|p| p.engine == engine).unwrap_or(0);
                            if let Some(focus) = shell.settings.engine_keys.get(at * crate::views::engines::KEYS) {
                                window.focus(focus, cx);
                            }
                        }
                        _ => {}
                    }
                    cx.notify();
                }),
                // Words in settings' search.
                "ssearch" => shell.update(cx, |shell, cx| {
                    shell.settings.search.update(cx, |s, cx| s.set_value(arg.clone(), window, cx));
                    cx.notify();
                }),
                // A settings pop-up open: `spop:language`, `spop:squad`.
                "spop" => shell.update(cx, |shell, cx| {
                    shell.settings.pop = match arg.as_str() {
                        "squad" => Some(crate::views::settings::Pop::Squad),
                        "none" => None,
                        _ => Some(crate::views::settings::Pop::Language),
                    };
                    cx.notify();
                }),
                "open" => shell.update(cx, |shell, cx| shell.open_ticket_row(arg.parse().unwrap_or(0), window, cx)),
                // The pointer moved to `x,y` (points from the window's top left):
                // hover fills and, after their delay, tooltips.
                "hover" => {
                    let (x, y) = arg.split_once(',').map(|(x, y)| (x.trim().parse().unwrap_or(0.), y.trim().parse().unwrap_or(0.))).unwrap_or((0f32, 0f32));
                    let moved = MouseMoveEvent { position: point(px(x), px(y)), pressed_button: None, modifiers: Modifiers::default() };
                    let moved = window.input_from_platform(PlatformInput::MouseMove(moved));
                    window.dispatch_event(moved, cx);
                }
                // A click (press and release) at `x,y`.
                "click" => {
                    let (x, y) = arg.split_once(',').map(|(x, y)| (x.trim().parse().unwrap_or(0.), y.trim().parse().unwrap_or(0.))).unwrap_or((0f32, 0f32));
                    let at = point(px(x), px(y));
                    let down = MouseDownEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: 1, first_mouse: false };
                    let down = window.input_from_platform(PlatformInput::MouseDown(down));
                    window.dispatch_event(down, cx);
                    let up = MouseUpEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: 1 };
                    let up = window.input_from_platform(PlatformInput::MouseUp(up));
                    window.dispatch_event(up, cx);
                }
                // A selection dragged out with the pointer, from `x1,y1` to `x2,y2`.
                "sdrag" => {
                    let n: Vec<f32> = arg.split(',').map(|v| v.trim().parse().unwrap_or(0.)).collect();
                    let [x1, y1, x2, y2] = n[..] else { return };
                    let (from, to) = (point(px(x1), px(y1)), point(px(x2), px(y2)));
                    let down = MouseDownEvent { button: MouseButton::Left, position: from, modifiers: Modifiers::default(), click_count: 1, first_mouse: false };
                    let down = window.input_from_platform(PlatformInput::MouseDown(down));
                    window.dispatch_event(down, cx);
                    for step in 1..=8 {
                        let t = step as f32 / 8.;
                        let at = point(px(x1 + (x2 - x1) * t), px(y1 + (y2 - y1) * t));
                        let moved = MouseMoveEvent { position: at, pressed_button: Some(MouseButton::Left), modifiers: Modifiers::default() };
                        let moved = window.input_from_platform(PlatformInput::MouseMove(moved));
                        window.dispatch_event(moved, cx);
                    }
                    let up = MouseUpEvent { button: MouseButton::Left, position: to, modifiers: Modifiers::default(), click_count: 1 };
                    let up = window.input_from_platform(PlatformInput::MouseUp(up));
                    window.dispatch_event(up, cx);
                }
                // The peek held at a width (`peekw:` alone lets it go).
                "peekw" => shell.update(cx, |shell, cx| {
                    shell.board.peek_held = arg.parse().ok();
                    cx.notify();
                }),
                // Asking about words selected: `quote:open` (the bar's button), `quote:copy`,
                // `quote:type:<words>`, `quote:send`, `quote:close`, `quote:jump` (the latest quote in a message of yours).
                "quote" => shell.update(cx, |shell, cx| {
                    let (what, words) = arg.split_once(':').unwrap_or((arg.as_str(), ""));
                    match what {
                        "open" => shell.quote_open(window, cx),
                        "copy" => shell.quote_copy(cx),
                        "type" => shell.quote_type(words, window, cx),
                        "send" => shell.quote_send(window, cx),
                        "close" => shell.quote_close(window, cx),
                        "jump" => shell.quote_jump_latest(cx),
                        _ => {}
                    }
                }),
                // 티켓: the layout, the sheet, a menu, the page, a choice, a drag by hand.
                "layout" => shell.update(cx, |shell, cx| shell.set_layout(if arg == "board" { crate::tickets::Layout::Board } else { crate::tickets::Layout::List }, cx)),
                "tsheet" => shell.update(cx, |shell, cx| shell.open_sheet(window, cx)),
                "ttitle" | "tbody" | "tcrit" => shell.update(cx, |shell, cx| {
                    match verb.as_str() {
                        "ttitle" => shell.board.tk.sheet_title.update(cx, |s, cx| s.set_value(arg.clone(), window, cx)),
                        "tbody" => shell.board.tk.sheet_body.update(cx, |s, cx| s.set_value(arg.replace("\\n", "\n"), window, cx)),
                        _ => {
                            shell.board.tk.sheet_criterion.update(cx, |s, cx| s.set_value(arg.clone(), window, cx));
                            shell.add_criterion(window, cx);
                        }
                    }
                    cx.notify();
                }),
                "tcreate" => shell.update(cx, |shell, cx| shell.create_from_sheet(window, cx)),
                // The quick line (C), with words typed in it.
                "quick" => shell.update(cx, |shell, cx| {
                    shell.open_quick(window, cx);
                    shell.board.tk.quick_input.update(cx, |s, cx| s.set_value(arg.clone(), window, cx));
                    cx.notify();
                }),
                "tmenu" => shell.update(cx, |shell, cx| {
                    use crate::views::tickets::EditKind as K;
                    let (kind, rest) = arg.split_once(',').unwrap_or((arg.as_str(), ""));
                    let kind = match kind {
                        "prio" => K::Priority,
                        "status" => K::Status,
                        "who" => K::Assignee,
                        "project" => K::Project,
                        "kind" => K::Kind,
                        _ => K::Depends,
                    };
                    let (targets, sheet) = match rest {
                        "sheet" => (Vec::new(), true),
                        "bulk" => (shell.board.view.chosen.iter().copied().collect(), false),
                        id => (vec![id.parse().unwrap_or(0)], false),
                    };
                    let code = match kind {
                        K::Priority => "prio",
                        K::Status => "status",
                        K::Assignee => "who",
                        K::Project => "project",
                        K::Kind => "kind",
                        K::Depends => "deps",
                    };
                    let key = if sheet { format!("sheet-{code}") } else if rest == "bulk" { format!("bulk-{code}") } else if shell.board.view.page.is_some() { format!("page-{code}-{rest}") } else { format!("{code}-{rest}") };
                    shell.open_edit_menu(kind, targets, key.into(), sheet, window, cx);
                }),
                "page" => shell.update(cx, |shell, cx| {
                    let (id, tab) = arg.split_once(',').unwrap_or((arg.as_str(), "overview"));
                    shell.board_pick(crate::state::Need::Ticket(id.parse().unwrap_or(0)), cx);
                    shell.board.view.page = Some(match tab {
                        "activity" => crate::board::Tab::Activity,
                        "changes" => crate::board::Tab::Changes,
                        _ => crate::board::Tab::Overview,
                    });
                    cx.notify();
                }),
                "tedit" => shell.update(cx, |shell, cx| shell.edit_title(arg.parse().unwrap_or(0), window, cx)),
                "tbodyedit" => shell.update(cx, |shell, cx| shell.edit_body(arg.parse().unwrap_or(0), window, cx)),
                "tcomment" => shell.update(cx, |shell, cx| {
                    shell.board.tk.comment.update(cx, |s, cx| s.set_value(arg.clone(), window, cx));
                    cx.notify();
                }),
                "choose" => shell.update(cx, |shell, cx| {
                    shell.board.view.chosen = arg.split(',').filter_map(|n| n.trim().parse().ok()).collect();
                    cx.notify();
                }),
                // A drag by hand: the button pressed at `x,y`, moved with it held, let go.
                "mdown" | "mmove" | "mup" => {
                    let (x, y) = arg.split_once(',').map(|(x, y)| (x.trim().parse().unwrap_or(0.), y.trim().parse().unwrap_or(0.))).unwrap_or((0f32, 0f32));
                    let at = point(px(x), px(y));
                    let input = match verb.as_str() {
                        "mdown" => PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: 1, first_mouse: false }),
                        "mmove" => PlatformInput::MouseMove(MouseMoveEvent { position: at, pressed_button: Some(MouseButton::Left), modifiers: Modifiers::default() }),
                        _ => PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: 1 }),
                    };
                    window.dispatch_event(input, cx);
                }
                other => eprintln!("script: no step {other}"),
            });
            eprintln!("perf step {timed} {:?}", started.elapsed());
            // Let the window draw what the step did.
            cx.background_executor().timer(Duration::from_millis(60)).await;
        }
    })
    .detach();
}

/// The process's CPU time so far, user and system, every thread.
fn process_cpu() -> Duration {
    // SAFETY: `usage` is a valid rusage for the call to fill.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let time = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
    time(usage.ru_utime) + time(usage.ru_stime)
}
