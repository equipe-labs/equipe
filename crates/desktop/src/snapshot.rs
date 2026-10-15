//! Dev tool (`--features snapshot`): drive the window from a script and write
//! what it painted, so a change can be looked at — and a stream watched — without
//! a screen-capture or UI-automation permission.
//!
//! `EQUIPE_SCRIPT="wait:1500;say:안녕;wait:800;shot:/tmp/a.png"` — steps split on `;`:
//! `wait:<ms>` · `shot:<png>` · `say:<text>` (type it in the box and send) ·
//! `memo:<text>` (send it as a memo, as Tab does) · `type:<text>` (type, don't send) · `key:<y|a|n|o>` (answer a request) · `esc` (stop the member) · `up`/`down`
//! (history) · `project:<name|all>` · `select:<member>` · `board` · `chat` ·
//! `bkey:<key>` (a board key) · `pick:<n>` (a ticket) · `find:<words>` · `note:<text>` (in the open question's box) · `submit` (its main button) · `folds` · `chooser` (the machine chooser) · `connect:<host>` · `pass:<text>` (answer ssh's
//! password box) · `trust` / `decline` (answer ssh's yes/no) · `retry` · `attach:<file>[,<file>…]` · `menu:<attach|recent|to|model|effort|mode|context>` · `ckey:<key>` (a key in the composer) · `light:<n>` (show attachment n big) · `drag:<path>[,…]` / `undrag` (the drop frame, as if dragged) · `theme:<light|dark>` · `paste` (⌘V with what the clipboard holds) · `quit`. Without a script, one shot after `EQUIPE_SNAPSHOT_MS`
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
                    shell.on_approval_key(&arg, true, cx);
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
                    shell.on_board_key(&arg, true, window, cx);
                }),
                "pick" => shell.update(cx, |shell, cx| shell.board_pick(arg.parse().unwrap_or(0), cx)),
                "find" => shell.update(cx, |shell, cx| shell.board_find(&arg, cx)),
                "note" => shell.update(cx, |shell, cx| shell.board_type(&arg, window, cx)),
                "cancel" => shell.update(cx, |shell, cx| shell.board_ask_cancel(cx)),
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
                "menu" => shell.update(cx, |shell, cx| {
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
                "size" => {
                    let (w, h) = arg.split_once('x').and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))).unwrap_or((1200., 760.));
                    window.resize(size(px(w), px(h)));
                }
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
                other => eprintln!("script: no step {other}"),
            });
            // Let the window draw what the step did.
            cx.background_executor().timer(Duration::from_millis(60)).await;
        }
    })
    .detach();
}
