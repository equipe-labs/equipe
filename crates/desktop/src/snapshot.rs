//! Dev tool (`--features snapshot`): drive the window from a script and write
//! what it painted, so a change can be looked at — and a stream watched — without
//! a screen-capture or UI-automation permission.
//!
//! `EQUIPE_SCRIPT="wait:1500;say:안녕;wait:800;shot:/tmp/a.png"` — steps split on `;`:
//! `wait:<ms>` · `shot:<png>` · `say:<text>` (type it in the box and send) ·
//! `memo:<text>` (send it as a memo, as Tab does) · `type:<text>` (type, don't send) · `key:<y|a|n|o>` (answer a request) · `esc` (stop the member) · `up`/`down`
//! (history) · `project:<name|all>` · `select:<member>` · `board` · `chat` ·
//! `bkey:<key>` (a board key) · `pick:<n>` (a ticket) · `find:<words>` · `note:<text>` (in the open question's box) · `submit` (its main button) · `folds` · `chooser` (the machine chooser) · `connect:<host>` · `pass:<text>` (answer ssh's
//! password box) · `trust` / `decline` (answer ssh's yes/no) · `retry` · `quit`. Without a script, one shot after `EQUIPE_SNAPSHOT_MS`
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
                "retry" => shell.update(cx, |shell, cx| shell.retry(cx)),
                other => eprintln!("script: no step {other}"),
            });
            // Let the window draw what the step did.
            cx.background_executor().timer(Duration::from_millis(60)).await;
        }
    })
    .detach();
}
