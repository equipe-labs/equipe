//! Dev tool (`--features snapshot`): drive the window from a script and write
//! what it painted, so a change can be looked at — and a stream watched — without
//! a screen-capture or UI-automation permission.
//!
//! `EQUIPE_SCRIPT="wait:1500;say:안녕;wait:800;shot:/tmp/a.png"` — steps split on `;`:
//! `wait:<ms>` · `shot:<png>` · `say:<text>` (type it in the box and send) ·
//! `memo:<text>` (send it as a memo, as Tab does) · `type:<text>` (type, don't send) · `key:<y|a|n|o>` (answer a request) · `esc` (stop the member) · `up`/`down`
//! (history) · `project:<name|all>` · `select:<member>` · `board` · `chat` ·
//! `menu:<inbox|working|all|view0…>` (a sidebar item) · `cmenu:<attach|recent|to|model|effort|mode|context>` (a composer menu) · `bkey:<key>` (a work
//! screen's key; `shift-a`, `cmd-a`, `alt-v` with modifiers) · `pick:<n>` (a
//! ticket) · `find:<words>` · `note:<text>` (in the open words box) · `submit`
//! (its main button) · `size:<w>x<h>` (the window) · `hover:<x>,<y>` / `click:<x>,<y>` (the pointer, in points) · `theme:<light|dark>` · `sidebar` (hide/show) · `popup:account` · `settings[:<category>]` (settings, at general · squad · team · permissions · projects · browser · keys · about) · `ssearch:<words>` (its search) · `spop:<language|squad>` (a pop-up) · `open:<n>` (a ticket's sidebar row) · `folds` · `elsewhere[:<n>]` (open or close the nth fold of other projects' talk) · `chooser` (the machine chooser) · `connect:<host>` · `pass:<text>` (answer ssh's
//! password box) · `trust` / `decline` (answer ssh's yes/no) · `retry` · `attach:<file>[,<file>…]` · `ckey:<key>` (a key in the composer) · `light:<n>` (show attachment n big) · `drag:<path>[,…]` / `undrag` (the drop frame, as if dragged) ·
//! `turn:<row>[,<call>]` (open or close a group of calls, or one call in it) · `side:<n>` (ticket n beside the conversation) ·
//! `browser` (the browser panel, ⌘⇧B) · `bpick:<n>` (its nth tab) · `bclick:<fx>,<fy>` (a click on the page, at fractions of its picture) · `bkeys:<text>` (typed key by key, `~` for Enter) · `bime:ㅎ|하|한` (composed, then committed) · `press:<keystroke>` (one key through the window: `escape`, `cmd-,`) · `bopen:<url>` (a tab of the person's own) · `paste` (⌘V with what the clipboard holds) · `pair` (휴대폰 연결's sheet) · `pair:allow|deny|retry|close` (its buttons) · `quit`. Without a script, one shot after `EQUIPE_SNAPSHOT_MS`
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
                    shell.palette = crate::theme::Palette::new(if arg == "light" { WindowAppearance::Light } else { WindowAppearance::Dark });
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
                    shell.palette = crate::theme::Palette::new(if arg == "light" { WindowAppearance::Light } else { WindowAppearance::Dark });
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
                "browser" => shell.update(cx, |shell, cx| shell.toggle_browser(window, cx)),
                "bopen" => {
                    let panel = shell.read(cx).browser.clone();
                    panel.update(cx, |b, cx| b.open_tab(&arg, cx));
                }
                "bpick" => {
                    let panel = shell.read(cx).browser.clone();
                    panel.update(cx, |b, cx| b.pick(arg.parse().unwrap_or(1), window, cx));
                }
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
                    window.dispatch_event(PlatformInput::MouseMove(moved), cx);
                }
                // A click (press and release) at `x,y`.
                "click" => {
                    let (x, y) = arg.split_once(',').map(|(x, y)| (x.trim().parse().unwrap_or(0.), y.trim().parse().unwrap_or(0.))).unwrap_or((0f32, 0f32));
                    let at = point(px(x), px(y));
                    let down = MouseDownEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: 1, first_mouse: false };
                    window.dispatch_event(PlatformInput::MouseDown(down), cx);
                    let up = MouseUpEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: 1 };
                    window.dispatch_event(PlatformInput::MouseUp(up), cx);
                }
                other => eprintln!("script: no step {other}"),
            });
            // Let the window draw what the step did.
            cx.background_executor().timer(Duration::from_millis(60)).await;
        }
    })
    .detach();
}
