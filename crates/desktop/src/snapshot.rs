//! Dev tool (`--features snapshot`): drive the window from a script and write
//! what it painted, so a change can be looked at — and a stream watched — without
//! a screen-capture or UI-automation permission.
//!
//! `EQUIPE_SCRIPT="wait:1500;say:안녕;wait:800;shot:/tmp/a.png"` — steps split on `;`:
//! `wait:<ms>` · `shot:<png>` · `say:<text>` (type it in the box and send) ·
//! `memo:<text>` (send it as a memo, as Tab does) · `type:<text>` (type, don't send) · `key:<y|a|n|o>` (answer a request) · `esc` (stop the member) · `up`/`down`
//! (history) · `project:<name|all>` · `shead:<project>` / `sfold:<project>` (a sidebar header's name, its chevron; empty: the shared folder) · `select:<member>` · `board` · `chat` ·
//! `menu:<inbox|working|all|view0…>` (a sidebar item) · `cmenu:<attach|recent|to|model|effort|mode|context>` (a composer menu) · `bkey:<key>` (a work
//! screen's key; `shift-a`, `cmd-a`, `alt-v` with modifiers) · `pick:<n>` (a
//! ticket) · `find:<words>` · `note:<text>` (in the open words box) · `submit`
//! (its main button) · `size:<w>x<h>` (the window) · `hover:<x>,<y>` / `click:<x>,<y>` (the pointer, in the screen's points — through the zoom, as the Mac sends it) · `zoom:<pct|in|out|actual>` (the window's size, as ⌘+ ⌘- ⌘0) · `theme:<light|dark>` · `sidebar` (hide/show) · `popup:account` · `settings[:<category>]` (settings, at general · squad · team · permissions · projects · browser · keys · about) · `ssearch:<words>` (its search) · `spop:<language|squad>` (a pop-up) · `open:<n>` (a ticket's sidebar row) · `folds` · `elsewhere[:<n>]` (open or close the nth fold of other projects' talk) · `chooser` (the machine chooser) · `connect:<host>` · `pass:<text>` (answer ssh's
//! password box) · `trust` / `decline` (answer ssh's yes/no) · `retry` · `attach:<file>[,<file>…]` · `ckey:<key>` (a key in the composer) · `light:<n>` (show attachment n big) · `drag:<path>[,…]` / `undrag` (the drop frame, as if dragged) ·
//! `turn:<row>[,<call>]` (open or close a group of calls, or one call in it) · `side:<n>` (ticket n beside the conversation) ·
//! `answer:<ticket>,<n>,<words>` / `aok:<ticket>,<n>` / `atick:<ticket>,<n>` / `apick:<ticket>,<n>,<done|yours|later|call|unknown>` / `anote:<ticket>,<n>,<words>` / `afree:<ticket>,<words>` / `asend:<ticket>` / `asent:<ticket>` (a waiting ticket's card: type in box n, its quick OK, tick thing n off, a pick, a note, the words to the member, send; open what was sent) ·
//! `dkey:<key>` (a member's questions' card: `1`–`9`, `up`, `down`, `space`, `enter`, `cmd-enter`, `tab`, `left`, `escape`) · `dtype:<words>` (in its box for the person's own words) · `decided` (open the latest decided row) · `dpick:<id>` (a set in 나에게, peeking) ·
//! `browser` (the browser panel, ⌘⇧B) · `bpick:<n>` (its nth tab) · `btake` / `bgive` (its control bar: 직접 하기, 돌려주기) · `breveal:<page>` (a page waiting for the person, as 나에게 opens it) · `bclick:<fx>,<fy>` (a click on the page, at fractions of its picture) · `bkeys:<text>` (typed key by key, `~` for Enter) · `bime:ㅎ|하|한` (composed, then committed) · `press:<keystroke>` (one key through the window: `escape`, `cmd-,`) · `bopen:<url>` (a tab of the person's own) · `paste` (⌘V with what the clipboard holds) · `pair` (휴대폰 연결's sheet) · `pair:allow|deny|retry|close` (its buttons) · `quit`. Without a script, one shot after `EQUIPE_SNAPSHOT_MS`
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
            let shell = shell.clone();
            let verb = verb.to_owned();
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
                "elsewhere" => shell.update(cx, |shell, cx| {
                    if let Some(fold) = shell.chat.nth_fold(arg.parse().unwrap_or(0)) {
                        shell.toggle_elsewhere(fold, cx);
                    }
                }),
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
                // 휴대폰 연결: the sheet, and its buttons.
                "pair" => shell.update(cx, |shell, cx| match arg.as_str() {
                    "allow" => shell.answer_pairing(true, cx),
                    "deny" => shell.answer_pairing(false, cx),
                    "retry" => shell.retry_pairing(cx),
                    "close" => shell.close_pairing(window, cx),
                    _ => shell.open_pairing(window, cx),
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
                "decided" => shell.update(cx, |shell, cx| shell.chat_open_decided(cx)),
                "asent" => shell.update(cx, |shell, cx| shell.chat_open_turn(arg.parse().unwrap_or(0), cx)),
                "dpick" => shell.update(cx, |shell, cx| {
                    shell.board.view.peek = true;
                    shell.board_pick(crate::state::Need::Questions(arg.parse().unwrap_or(0)), cx);
                }),
                "browser" => shell.update(cx, |shell, cx| shell.toggle_browser(window, cx)),
                "bopen" => {
                    let panel = shell.read(cx).browser.clone();
                    panel.update(cx, |b, cx| b.open_tab(&arg, cx));
                }
                "bpick" => {
                    let panel = shell.read(cx).browser.clone();
                    panel.update(cx, |b, cx| b.pick(arg.parse().unwrap_or(1), window, cx));
                }
                // The control bar's button: 직접 하기 (`btake`) or 돌려주기 (`bgive`).
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
                        _ => None,
                    };
                    cx.notify();
                }),
                // Settings, at a category (`settings:team`), or where it was left.
                "settings" => shell.update(cx, |shell, cx| shell.open_settings(crate::settings::Category::parse(&arg), window, cx)),
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
            // Let the window draw what the step did.
            cx.background_executor().timer(Duration::from_millis(60)).await;
        }
    })
    .detach();
}
