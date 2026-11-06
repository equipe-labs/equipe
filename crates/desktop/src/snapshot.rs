//! Dev tool (`--features snapshot`): drive the window from a script and write
//! what it painted, so a change can be looked at — and a stream watched — without
//! a screen-capture or UI-automation permission.
//!
//! `EQUIPE_SCRIPT="wait:1500;say:hello;wait:800;shot:/tmp/a.png"` — steps split on `;`:
//! `wait:<ms>` · `shot:<png>` · `say:<text>` (type it in the box and send) ·
//! `memo:<text>` (send it as a memo, as Tab does) · `type:<text>` (type, don't send) · `key:<y|a|n|o>` (answer a request) · `esc` (stop the member) · `up`/`down`
//! (history; ↑ on an empty box reaches the bubbles waiting) · `pwithdraw` (take the last waiting bubble back) · `pnow` (send what waits now) · `project:<name|all>` · `shead:<project>` / `sfold:<project>` (a sidebar header's name, its chevron; empty: the shared folder) · `select:<member>` · `board` · `chat` ·
//! `menu:<inbox|working|all|view0…>` (a sidebar item) · `cmenu:<attach|recent|to|model|effort|mode|context>` (a composer menu) · `bkey:<key>` (a work
//! screen's key; `shift-a`, `cmd-a`, `alt-v` with modifiers) · `pick:<n>` (a
//! ticket; `ppick:<n>`, peeking) · `find:<words>` · `note:<text>` (in the open words box) · `submit`
//! (its main button) · `size:<w>x<h>` (the window) · `hover:<x>,<y>` / `click:<x>,<y>` (the pointer, in the screen's points — through the zoom, as the Mac sends it) · `zoom:<pct|in|out|actual>` (the window's size, as ⌘+ ⌘- ⌘0) · `theme:<light|dark>` · `sidebar` (hide/show) · `popup:account` · `popup:subagents:<member>` · `subagent:<member>[,<n>]` (its first — or nth, from 0 — subagent's row, clicked) · `settings[:<category>]` (settings, at general · engines · squad · team · permissions · projects · browser · keys · about) · `words:<on|off|ask>` ("All by words" for the squad, or asked about) · `wlog` (the log of where words went wrong) · `wundo` (Undo, Cancel or Leave on what waits in words) · `eng:<signin|cancel|signout|sure|keep|tab>:<codex|claude>` (an AI platform's card) · `signin:<go|move|check|cancel|carry|far|paste:<code>|type:<code>>` (a lost sign-in's card in the conversation) · `ssearch:<words>` (its search) · `spop:<language|squad|subagents|provider>` (a pop-up) · `padd:<preset>` · `purl:<url>` · `pconnect` (a model provider's form: opened, its address, Connect) · `open:<n>` (a ticket's sidebar row) · `folds` · `elsewhere[:<n>]` (open or close the nth fold of other projects' talk) · `chooser` (the machine chooser) · `connect:<host>` · `pass:<text>` (answer ssh's
//! password box) · `trust` / `decline` (answer ssh's yes/no) · `retry` · `setup:<nogit|gitasked|missing|confirm|installing|signedout|waiting|ready|failed|squad|creating|error>` (the first run's steps) · `attach:<file>[,<file>…]` · `ckey:<key>` (a key in the composer) · `light:<n>` (show attachment n big) · `drag:<path>[,…]` / `undrag` (the drop frame, as if dragged) ·
//! `sdrag:<x1>,<y1>,<x2>,<y2>` (a selection dragged out) · `peekw:<px>` (the peek held that wide) · `quote:<open|reply|copy|type:<words>|write:<words>|add|cancel|edit:<n>|remove:<n>|peek:<n>|card:<quote>|<words>|send|close|jump>` (asking about — replying to — the words selected; a card's "Reply", ✕, the pointer on it; a card made at once) ·
//! `sidechat:<start|on:<words>|type:<words>|send|close|bring|jump|reopen>` (a side chat: about the words selected, or about `words` of the conversation as if selected; typed in, sent, closed, its last answer brought to the composer, its passage clicked, opened again from its mark) ·
//! `turn:<row>[,<call>]` (open or close a group of calls, or one call in it) · `side:<n>` (ticket n beside the conversation) ·
//! `answer:<ticket>,<n>,<words>` / `aok:<ticket>,<n>` / `atick:<ticket>,<n>` / `apick:<ticket>,<n>,<done|yours|later|call|unknown>` / `anote:<ticket>,<n>,<words>` / `afree:<ticket>,<words>` / `asend:<ticket>` / `asent:<ticket>` (a waiting ticket's card: type in box n, its quick OK, tick thing n off, a pick, a note, the words to the member, send; open what was sent) ·
//! `dkey:<key>` (a member's questions' card: `1`–`9`, `up`, `down`, `space`, `enter`, `cmd-enter`, `tab`, `left`, `escape`) · `dtype:<words>` (in its box for the person's own words) · `decided` (open the latest decided row) · `dpick:<id>` (a set in Needs you, peeking) · `dview:<n>` (option n's picture of the card on show, big) · `dtake:<n>` / `dmix:<n>[,<k>]` / `dagain` (a design choice's one click: option n's "This one", its mix menu opened or its kth line, "Again") · `tview:<ticket>,<n>` (a hand-in's nth picture, big; then `press:right`, `press:enter`) · `rview:<n>` (the nth picture the latest reply showing pictures embeds, big) · `tpick:<id>` (a change to the team in Needs you, peeking) · `tyes` / `tno` (approve or decline the oldest change to the team) · `rpick:<id>` (a routine the PM proposed in Needs you, peeking) · `ryes` / `rno` (turn on or decline the oldest) · `cpick:<id>` (a card to confirm in Needs you, peeking; `cpick` alone: the oldest) · `ctick:<ticket>` (tick or untick a ticket on the oldest card) · `cyes` / `cno` (confirm or decline the oldest card) · `eyes` / `eno` / `eundo` (the engine card: its yes, its no, undo) · `engines:<off|suggest|auto>` (the squad's engine switching) ·
//! `qopen` · `qtype:<n>,<words>` · `qsay:<n>,<words>` · `qsend` · `qesc` · `qenter` (a reply's list of questions as a form: open it, type in box n — past the items, the one for anything to add — tap a quick answer, send, cancel; ⏎ in the composer) ·
//! `metrics[:<dash|error|empty|connect|key|done>]` (Metrics, on a sample) · `mtype:<n>,<text>` (in its form's box n) ·
//! `design[:<overview|colors|type|spacing|radius|shadow|motion|assets|guidelines|taste>]` (Design system, at a section) · `dsource:<squad|project:name>` (another system) · `ddraft` (its draft, or back) · `dwork` (the work under way on it, or back) · `dchanges` (what changed, opened or closed) · `dlight:<n>` / `dlight:off` (asset n big) · `dplay:<token>` (a motion token played, as under the pointer) · `dask` (ask the designer for one) · `dtoc:<n>` (the guidelines' nth heading) ·
//! `dtab:<map|system>` (UI map or Design system) · `fflow:<id>` (a flow) · `fscreen:<id>` (a screen's card, clicked) · `fstate:<key>` (a state's chip; `+empty` for one missing) · `fmode:<light|dark>` · `fdata:<real|example>` (Real data or Example data) · `fwidth:<px>` · `fzoom` (the state big) · `fvideos` (the older walkthroughs, listed or not) · `fsay` ("Talk" on the open state) · `dsay:<token>` / `dsay:asset:<n>` ("Talk" on a colour or type style by name, or asset n) · `dtalk:<open|fold>` (the conversation beside the design screen) · `dunpoint:<n>` (chip n taken off) · `fkey:<left|right|up|down|escape>` ·
//! `dcompare[:off]` (Compare and review, begun or ended) · `dside:<now|after>` · `dstep[:prev]` (the next or previous changed screen) · `daccept` · `dredo[:<words>]` ("Ask again"'s box, with words) · `dredosend` · `dversions` (Version history, opened or closed) · `drestore:<n>` (row n asked back: its confirmation) · `dtaste:<add|edit:<n>|type:<words>|save|cancel|remove:<n>|confirm:<n>|evidence:<n>>` (a taste note: write a new one, rewrite note n, type in its box, keep it, put the box away, remove note n, say "That's right" to note n, open or close its evidence) ·
//! `cfocus` (the summary band's retry, focused by keyboard) · `mark:<label>` (a line in the log, with the process's CPU time and memory) · `a11yon` / `a11y:<json>` (the accessibility tree built, as for a screen reader; then written) · `scroll:<n>,<dy>` (n wheel turns a frame apart) · `wheel:<x>,<y>,<n>,<dy>` (the same with the pointer at `x,y`) · `pinch:<x>,<y>,<n>,<delta>` (a trackpad's pinch there, n frames of `delta` each) · `browser` (the browser panel, ⌘⇧B) · `doc:<path>` (a document in the side panel) · `docs` (briefs and plans, all in one place) · `docsfind:<words>` · `docsfilter:<project:<name>|squad|kind:<brief|plan>|status:<word>|clear>` · `docsopen:<n>` (its nth row, beside) · `docskeep:<set>,<n>` (keep copy n of a set saved under more than one project) · `dfold[:open]` (fold it to a rail, or open it again) · `term[:<line>]` (the terminal, with a line typed at its prompt) · `tkeys:<text>` (typed into it, `~` for Enter) · `tmove` (the terminal to the bottom, or back to the right) · `bpick:<n>` (its nth tab) · `bclose:<n>` (its ✕) · `bwheel` (a wheel over the page) · `btake` / `bgive` (its control bar: "Take over", "Give back") · `breveal:<page>` (a page waiting for the person, as Needs you opens it) · `bclick:<fx>,<fy>` (a click on the page, at fractions of its picture) · `bkeys:<text>` (typed key by key, `~` for Enter) · `bime:<step>|<step>|…` (an IME's steps composed, then the last committed: `bime:k|か`) · `press:<keystroke>` (one key through the window: `escape`, `cmd-,`) · `bopen:<url>` (a tab of the person's own) · `paste` (⌘V with what the clipboard holds) · `pair` ("Connect a phone"'s sheet) · `pair:allow|deny|retry|close` (its buttons) · `newsquad[:type:<name>|create|tab[:back]|close|failed|far|farout]` ("New squad…"'s sheet: a name, its Create, Tab, closed, a refusal shown; the window as if on another machine, attached or not) · `usage[:day|week|claude|codex|outside|cut:<tickets|members|projects>|hour[:<n>]|dismiss]` (the Usage screen: its stretch, its engine, outside equipe opened, a cut, an hour picked — the busiest, or the nth — and the fast burn's notice closed) · `disk` (the Disk panel) · `disk:fix<n>|all|yes|back|rescan|close` (a fix's sheet, every fix's, its yes, back, look again, close) · `bell:<sheet|notasked|denied|allowed>` (the notifications sheet; what macOS allows) · `newwindow[:<squad>]` (New Window, ⌘⇧N; a squad's "open in a new window") · `win:<n|first|last>` (the steps after it act on that window) · `titles[:<file>]` (every window's title, as the Window menu lists them) · `quit`. Without a script, one shot after `EQUIPE_SNAPSHOT_MS`
//! (default 3000) to `EQUIPE_SNAPSHOT`.
//!
//! `shot:` names its file with `{mode}` and `{width}` put in as the window is
//! drawn (`default.{mode}.{width}.png`); `lang:<code>` switches the words.
//!
//! `EQUIPE_SNAPSHOT_READONLY=1` is the real view (`readonly.rs`): the person's
//! own daemon, read-only, in their language and look — that of the screen
//! they used last, as it told the daemon (its steps come before the
//! script's), the window never shown; each `shot` gets a note beside it
//! (`.json`: the app, its version, `EQUIPE_SHOT_COMMIT`, the squad, whose look
//! — that screen's machine and version, or this Mac's own state — the
//! language, the mode and the size). `scripts/design-shot-live.sh` runs it.
//!
//! `EQUIPE_SNAPSHOT_HIDDEN=1`: the window is never shown either, and draws itself, as the real
//! view's does — but on whatever daemon `EQUIPE_HOME` names (the fake one), nothing held
//! back: to run a script, and measure it (`mark:`), with nothing on the screen.
//!
//! `EQUIPE_SNAPSHOT_SCALE=2` draws the window — and so every `shot` — at that scale, whatever
//! screen is attached (default: the screen's own): the PNG is exactly twice the pixels of a 1×
//! one, sharp. The frame is the app's own, so it needs no screen-recording permission.

use std::time::Duration;

use gpui_kit::*;

use crate::Shell;

/// The note a real-view picture is kept with (`equipe_core::design::flows`'s
/// `ShotNote`, and what it was drawn in): the app and the build that shot
/// it — the commit as the runner says (`EQUIPE_SHOT_COMMIT`) — and the squad,
/// language, mode and size.
fn shot_note(squad: &str, mode: &str, width: u32, height: u32) -> String {
    let commit = std::env::var("EQUIPE_SHOT_COMMIT").ok().filter(|c| !c.trim().is_empty());
    let scale = std::env::var("EQUIPE_SNAPSHOT_SCALE").ok().and_then(|s| s.parse::<f32>().ok());
    let note = serde_json::json!({
        "app": equipe_core::design::flows::DESKTOP_APP,
        "version": env!("CARGO_PKG_VERSION"),
        "commit": commit,
        "live": true,
        "squad": squad,
        // Whose look it was shot in: the person's screen (its machine and version), or this Mac's own state.
        "look": crate::readonly::look().map(crate::readonly::Look::note),
        "lang": crate::i18n::lang().code(),
        "mode": mode,
        "width": width,
        "height": height,
        "scale": scale,
    });
    serde_json::to_string_pretty(&note).unwrap_or_default()
}

pub fn run(mut handle: AnyWindowHandle, mut shell: Entity<Shell>, cx: &mut App) {
    let script = match (std::env::var("EQUIPE_SCRIPT"), std::env::var("EQUIPE_SNAPSHOT")) {
        (Ok(script), _) => script,
        (_, Ok(path)) => format!("wait:{};shot:{path};quit", std::env::var("EQUIPE_SNAPSHOT_MS").unwrap_or_else(|_| "3000".into())),
        _ => return,
    };
    // The real view (`readonly.rs`): the window brought to the person's look
    // first; the script's own steps after it may say otherwise.
    let script = match crate::readonly::look() {
        Some(look) => format!("{};{script}", look.steps()),
        None => script,
    };
    let started = std::time::Instant::now();
    // The real view's window is never shown, so the screen asks it for no
    // frames: it draws them itself, as a shown one would be asked to, a
    // frame's time apart while it waits and after each step.
    let hidden = crate::readonly::hidden();
    let frame = move |handle: AnyWindowHandle, cx: &mut AsyncApp| {
        if hidden {
            let _ = cx.update_window(handle, |_, window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
        }
    };
    cx.spawn(async move |cx| {
        for step in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let (verb, arg) = step.split_once(':').unwrap_or((step, ""));
            let arg = arg.to_owned();
            if verb == "wait" {
                let until = std::time::Instant::now() + Duration::from_millis(arg.parse().unwrap_or(500));
                while hidden && std::time::Instant::now() < until {
                    cx.background_executor().timer(Duration::from_millis(16)).await;
                    frame(handle, cx);
                }
                let left = until.saturating_duration_since(std::time::Instant::now());
                cx.background_executor().timer(left).await;
                continue;
            }
            // What the steps before it changed, drawn before the picture is taken.
            if verb == "shot" || verb == "shotlast" {
                frame(handle, cx);
                frame(handle, cx);
            }
            // `win:<n|first|last>`: the steps after it act on that window (in
            // the order the windows opened).
            if verb == "win" {
                let found = cx.update(|_| {
                    let open = crate::windows::shells();
                    let at = match arg.as_str() {
                        "first" => Some(0),
                        "last" => open.len().checked_sub(1),
                        n => n.parse().ok(),
                    };
                    at.and_then(|at| open.get(at).cloned()).and_then(|(h, s)| Some((h, s.upgrade()?)))
                });
                match found {
                    Some((h, s)) => (handle, shell) = (h, s),
                    _ => eprintln!("win: no window {arg}"),
                }
                continue;
            }
            if verb == "quit" {
                cx.update(|cx| cx.quit());
                return;
            }
            // A line in the log the measurements are cut at (`docs/PERF-2026-10.md`),
            // with the process's CPU time so far and the clock, to divide one by the other.
            if verb == "mark" {
                let mb = crate::mem::phys_footprint_bytes().map_or(0., |b| b as f64 / 1_048_576.);
                eprintln!("perf mark {arg} cpu {}ms at {}ms mem {mb:.1}MB", process_cpu().as_millis(), started.elapsed().as_millis());
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
                    // A hidden window draws each turn itself, as the screen would ask it to.
                    frame(handle, cx);
                }
                continue;
            }
            // `pinch:<x>,<y>,<n>,<delta>`: a trackpad's pinch at `x,y` (the
            // screen's points, as `wheel:`), n frames apart, each growing the
            // zoom by `delta` of itself (negative: out) — begun, then ended.
            if verb == "pinch" {
                let n: Vec<f32> = arg.split(',').map(|v| v.trim().parse().unwrap_or(f32::NAN)).collect();
                let at = match &n[..] {
                    [x, y, ..] if x.is_finite() && y.is_finite() => Some(point(px(*x), px(*y))),
                    _ => None,
                };
                let frames = n.get(2).filter(|n| n.is_finite()).map_or(20, |n| *n as usize);
                let delta = n.get(3).copied().filter(|d| d.is_finite()).unwrap_or(0.05);
                for frame in 0..frames + 2 {
                    let (phase, delta) = match frame {
                        0 => (TouchPhase::Started, 0.),
                        f if f == frames + 1 => (TouchPhase::Ended, 0.),
                        _ => (TouchPhase::Moved, delta),
                    };
                    let _ = cx.update_window(handle, |_, window, cx| {
                        let size = window.viewport_size();
                        let at = at.unwrap_or_else(|| point(size.width * 0.5, size.height * 0.5));
                        let pinch = PinchEvent { position: at, delta, modifiers: Modifiers::default(), phase };
                        let pinch = window.input_from_platform(PlatformInput::Pinch(pinch));
                        window.dispatch_event(pinch, cx);
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
                "windows" => eprintln!("windows: {}", cx.windows().len()),
                // New Window (⌘⇧N), or `newwindow:<squad>` (a squad's "open in a new window").
                "newwindow" => shell.update(cx, |shell, cx| shell.new_window((!arg.is_empty()).then(|| arg.clone()), cx)),
                // Every window's title, as the Mac's Window menu lists them; with a path, written there too.
                "titles" => {
                    // The window running the step is busy: its own title is read here.
                    let here = window.window_handle().window_id();
                    let titles: Vec<String> = crate::windows::shells()
                        .iter()
                        .filter_map(|(h, _)| if h.window_id() == here { Some(window.window_title()) } else { h.update(cx, |_, window, _| window.window_title()).ok() })
                        .collect();
                    eprintln!("titles: {titles:?}");
                    if !arg.is_empty() {
                        eprintln!("titles {arg}: {:?}", std::fs::write(&arg, titles.join("\n") + "\n"));
                    }
                }
                // What a screen reader — or an agent reading the accessibility
                // tree — is told: on from the next frame, then written as JSON.
                "a11yon" => window.activate_a11y(),
                "a11y" => match window.debug_a11y_tree_json() {
                    Some(json) => eprintln!("a11y {arg}: {:?}", std::fs::write(&arg, json)),
                    None => eprintln!("a11y: no tree yet (a11yon, then a frame)"),
                },
                // `{mode}` and `{width}` in the path: as the window is drawn now.
                "shot" => {
                    let dark = gpui_kit::component::Theme::global(cx).mode.is_dark();
                    let size = window.bounds().size;
                    let (width, height) = (f32::from(size.width).round() as u32, f32::from(size.height).round() as u32);
                    let mode = if dark { "dark" } else { "light" };
                    let path = arg.replace("{mode}", mode).replace("{width}", &width.to_string());
                    match window.render_to_image() {
                        Ok(image) => {
                            eprintln!("shot {path}: {:?}", image.save(&path));
                            // The real view's note beside it: what shot it, and in what.
                            if crate::readonly::on() {
                                let note = shot_note(&shell.read(cx).model.squad, mode, width, height);
                                let at = std::path::Path::new(&path).with_extension("json");
                                eprintln!("note {}: {:?}", at.display(), std::fs::write(&at, note));
                            }
                        }
                        Err(err) => eprintln!("shot failed: {err}"),
                    }
                }
                // The window opened last (⌘-click), as it looks.
                "shotlast" => match cx.windows().last().map(|w| w.update(cx, |_, window, _| window.render_to_image())) {
                    Some(Ok(Ok(image))) => eprintln!("shot {arg}: {:?}", image.save(&arg)),
                    _ => eprintln!("shot failed"),
                },
                // The window's words in a language (`lang:en`), as settings switch them.
                "lang" => {
                    if let Some(lang) = equipe_core::i18n::Lang::parse(&arg) {
                        crate::i18n::set(lang);
                        crate::set_menus(cx);
                        shell.update(cx, |_, cx| cx.notify());
                    }
                }
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
                    shell.on_approval_key(&arg, false, window, cx);
                }),
                // A new conversation with the PM (`newtalk` the squad's, `newtalk:<project>`).
                "newtalk" => shell.update(cx, |shell, cx| shell.start_conversation((!arg.is_empty()).then(|| arg.clone()), window, cx)),
                "esc" => shell.update(cx, |shell, cx| shell.interrupt(cx)),
                "up" | "down" => shell.update(cx, |shell, cx| {
                    shell.on_composer_key(&verb, &Modifiers::default(), window, cx);
                }),
                "project" => shell.update(cx, |shell, cx| shell.focus_project((arg != "all").then(|| arg.clone()), window, cx)),
                "shead" | "sfold" => shell.update(cx, |shell, cx| {
                    let click = if verb == "shead" { crate::board::HeadClick::Name } else { crate::board::HeadClick::Chevron };
                    shell.side_head_click(click, (!arg.is_empty()).then(|| arg.clone()), window, cx);
                }),
                // A project page's tab (`ptab:work`, `ptab:docs`, `ptab:talks`).
                "ptab" => shell.update(cx, |shell, cx| {
                    shell.board.project_tab = match arg.as_str() {
                        "work" => crate::board::ProjectTab::Work,
                        "docs" => crate::board::ProjectTab::Docs,
                        _ => crate::board::ProjectTab::Talks,
                    };
                    cx.notify();
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
                // `progress`: Progress; `retro:<day>`: a past retro from its list, the PM's conversation at it.
                "progress" => shell.update(cx, |shell, cx| shell.open_progress(cx)),
                "retro" => shell.update(cx, |shell, cx| {
                    // No day: the latest, as its row lists it first.
                    let latest = || shell.model.pm().and_then(|pm| shell.model.convos.get(&pm)).and_then(|c| crate::retro::past(c.transcript.entries()).into_iter().next()).map(|p| p.to);
                    let to = if arg.is_empty() { latest().unwrap_or_default() } else { arg.clone() };
                    shell.open_retro(&to, window, cx);
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
                    let text: String = (1..=lines).map(|n| format!("line {n}: the log goes on here\n")).collect();
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
                // The first run's steps, held at a state: `missing` · `confirm` · `installing` · `signedout` · `waiting` · `ready` (the AI is connected) · `failed` (the install) · `squad` (the squad form) · `creating` · `error` (the squad didn't go through).
                "setup" => shell.update(cx, |shell, cx| {
                    use crate::setup::{Claude, Doing};
                    let s = &mut shell.setup;
                    s.probed = true;
                    // No daemon runs here: hold the screen's place, as a Mac with no squad has it.
                    shell.model.conn = crate::state::Conn::Down { reason: crate::failure::Failure::NoSquads, since: crate::state::Clock::now(), had_snapshot: false };
                    let s = &mut shell.setup;
                    (s.claude, s.codex, s.doing, s.confirm_install, s.failed, s.squad_step, s.git, s.git_asked) = (Some(Claude::Missing), Some(false), Doing::Idle, false, None, false, Some(true), false);
                    match arg.as_str() {
                        "nogit" => s.git = Some(false),
                        "gitasked" => (s.git, s.git_asked) = (Some(false), true),
                        "confirm" => s.confirm_install = true,
                        "installing" => s.doing = Doing::Installing,
                        "signedout" => s.claude = Some(Claude::SignedOut),
                        "waiting" => (s.claude, s.doing) = (Some(Claude::SignedOut), Doing::ClaudeLogin),
                        "ready" => (s.claude, s.codex) = (Some(Claude::SignedIn), Some(true)),
                        "squad" | "creating" | "error" => {
                            (s.claude, s.codex, s.squad_step) = (Some(Claude::SignedIn), Some(true), true);
                            shell.welcome.busy = arg == "creating";
                            shell.welcome.failed = (arg == "error").then(|| format!("{}\nname \"my-squad\" is already taken", equipe_core::t!("welcome.failed_create")));
                        }
                        "failed" => s.failed = Some((Doing::Installing, "curl: (6) Could not resolve host: claude.ai — the connection to https://claude.ai/install.sh could not be made, check the network and try once more later".into())),
                        _ => {}
                    }
                    cx.notify();
                }),
                // Design system: the screen at a section, another system, its draft, what changed, an asset big.
                // Two routines on this screen alone (no daemon asked), the second off.
                "rseed" => shell.update(cx, |shell, cx| {
                    use equipe_core::protocol::routines::{Routine, Schedule};
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                    shell.model.routines = vec![
                        Routine { id: 1, title: "Weekly report".into(), prompt: "-".into(), when: Schedule::Weekly { weekday: 1, hour: 9, minute: 0 }, on: true, last_run: Some(now - 6 * 86_400), next_run: Some(now + 86_400) },
                        Routine { id: 2, title: "Dependency check".into(), prompt: "-".into(), when: Schedule::Weekly { weekday: 5, hour: 16, minute: 0 }, on: false, last_run: None, next_run: Some(now + 4 * 86_400) },
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
                "roadmap" => shell.update(cx, |shell, cx| shell.open_roadmap(cx)),
                // Metrics on a sample, no daemon asked: `metrics:dash|error|empty|connect|key|done`;
                // `mtype:<n>,<text>` types in the form's box n (a key shows hidden).
                "metrics" => shell.update(cx, |shell, cx| shell.metrics_demo(&arg, window, cx)),
                "mtype" => shell.update(cx, |shell, cx| {
                    let (n, text) = arg.split_once(',').unwrap_or(("0", ""));
                    shell.metrics_type(n.parse().unwrap_or(0), text, window, cx);
                }),
                // Two logins on this screen alone (no daemon asked); `lform` opens the form.
                "lseed" => shell.update(cx, |shell, cx| {
                    use equipe_core::protocol::vault::{Login, Use};
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                    shell.model.logins = vec![
                        Login { id: 1, site: "dash.cloudflare.com".into(), url: Some("https://dash.cloudflare.com/login".into()), username: "abel@ironact.net".into(), note: Some("DNS".into()), allowed: vec!["demo/backend".into()], created_at: now, uses: vec![Use { at: now - 3600, squad: "demo".into(), member: "backend".into(), outcome: "signed_in".into() }] },
                        Login { id: 2, site: "github.com".into(), url: None, username: "abel-ko".into(), note: None, allowed: vec![], created_at: now, uses: vec![Use { at: now - 86_400, squad: "demo".into(), member: "frontend".into(), outcome: "declined".into() }] },
                    ];
                    cx.notify();
                }),
                "lform" => shell.update(cx, |shell, cx| shell.open_login_form(None, window, cx)),
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
                "dwork" => shell.update(cx, |shell, cx| {
                    let on = !shell.design.working;
                    shell.design_view_working(on, cx);
                }),
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
                // UI map: its tab, a flow, a screen, a state, the look, the box.
                "dtab" => shell.update(cx, |shell, cx| {
                    if let Some(tab) = crate::flows::Tab::parse(&arg) {
                        shell.design_set_tab(tab, cx);
                    }
                }),
                "dcompare" => shell.update(cx, |shell, cx| shell.design_compare(arg != "off", cx)),
                "dside" => shell.update(cx, |shell, cx| {
                    if let Some(side) = crate::compare::Side::parse(&arg) {
                        shell.design_side(side, cx);
                    }
                }),
                "dstep" => shell.update(cx, |shell, cx| shell.design_step_changed(arg != "prev", cx)),
                "daccept" => shell.update(cx, |shell, cx| shell.design_accept_draft(cx)),
                "dredo" => shell.update(cx, |shell, cx| {
                    shell.design_redo(true, window, cx);
                    if !arg.is_empty() {
                        shell.design_redo.update(cx, |s, cx| s.set_value(arg.clone(), window, cx));
                    }
                }),
                "dredosend" => shell.update(cx, |shell, cx| shell.design_redo_send(window, cx)),
                "dversions" => shell.update(cx, |shell, cx| shell.design_versions(!shell.design.versions_open, cx)),
                "drestore" => shell.update(cx, |shell, cx| shell.design_restore_ask(arg.parse().unwrap_or(1), cx)),
                "fflow" => shell.update(cx, |shell, cx| shell.flows_pick_flow(&arg, cx)),
                "fscreen" => shell.update(cx, |shell, cx| shell.flows_toggle_screen(&arg, cx)),
                "fstate" => shell.update(cx, |shell, cx| shell.flows_chip(&arg, cx)),
                "fmode" => shell.update(cx, |shell, cx| shell.flows_mode(&arg, cx)),
                "fdata" => shell.update(cx, |shell, cx| shell.flows_data(if arg == "example" { crate::flows::Data::Example } else { crate::flows::Data::Real }, cx)),
                "fwidth" => shell.update(cx, |shell, cx| shell.flows_width(arg.parse().unwrap_or(390), cx)),
                "fzoom" => shell.update(cx, |shell, cx| shell.flows_zoom(true, cx)),
                "fvideos" => shell.update(cx, |shell, cx| {
                    shell.design.map.older_open = !shell.design.map.older_open;
                    cx.notify();
                }),
                // Talk while looking: a thing pointed at, the panel folded or open, a chip taken off.
                "fsay" => shell.update(cx, |shell, cx| shell.flows_point(window, cx)),
                "dsay" => shell.update(cx, |shell, cx| shell.design_point_named(&arg, window, cx)),
                "dtalk" => shell.update(cx, |shell, cx| shell.design_talk_fold(arg == "fold", window, cx)),
                "dunpoint" => shell.update(cx, |shell, cx| shell.design_unpoint(arg.parse().unwrap_or(0), cx)),
                "fkey" => shell.update(cx, |shell, cx| {
                    shell.on_flows_key(&arg, window, cx);
                }),
                // Taste & decisions: `dtaste:add`, `dtaste:edit:<n>`, `dtaste:type:<words>`, `dtaste:save`, `dtaste:cancel`, `dtaste:remove:<n>`, `dtaste:confirm:<n>`, `dtaste:evidence:<n>`.
                "dtaste" => shell.update(cx, |shell, cx| {
                    let (what, rest) = arg.split_once(':').unwrap_or((arg.as_str(), ""));
                    let nth = |shell: &crate::Shell| shell.design.current_taste().and_then(|t| t.notes.get(rest.parse::<usize>().unwrap_or(0))).map(|n| n.text.clone());
                    match what {
                        "add" => shell.design_taste_begin(None, window, cx),
                        "edit" => {
                            if let Some(was) = nth(shell) {
                                shell.design_taste_begin(Some(was), window, cx);
                            }
                        }
                        "type" => shell.design_taste_type(rest, window, cx),
                        "save" => shell.design_taste_save(window, cx),
                        "cancel" => shell.design_taste_cancel(cx),
                        "confirm" => {
                            if let Some(text) = nth(shell) {
                                shell.design_taste_send(equipe_core::protocol::design::TasteChange::Confirm { text }, cx);
                            }
                        }
                        "evidence" => {
                            if let Some(text) = nth(shell) {
                                shell.design_taste_evidence(&text, cx);
                            }
                        }
                        "remove" => {
                            if let Some(text) = nth(shell) {
                                shell.design_taste_send(equipe_core::protocol::design::TasteChange::Remove { text }, cx);
                            }
                        }
                        _ => {}
                    }
                }),
                "dtoc" => shell.update(cx, |shell, cx| {
                    let n: usize = arg.parse().unwrap_or(0);
                    let section = shell.design.current().and_then(|s| s.guidelines.as_deref()).map(crate::design::headings).and_then(|h| h.get(n).map(|h| h.section));
                    if let Some(section) = section {
                        shell.design.toc_at = n;
                        shell.design.scroll.scroll_to_item(section);
                        cx.notify();
                    }
                }),
                // Connect a phone: the sheet, and its buttons.
                "pair" => shell.update(cx, |shell, cx| match arg.as_str() {
                    "allow" => shell.answer_pairing(true, cx),
                    "deny" => shell.answer_pairing(false, cx),
                    "retry" => shell.retry_pairing(cx),
                    "close" => shell.close_pairing(window, cx),
                    _ => shell.open_pairing(window, cx),
                }),
                // New squad…: the sheet; a name typed; its Create; Tab; a refusal
                // from the far machine; the window as if on another machine
                // (`far`, attached; `farout`, the line down) — drawn only, nothing reached.
                "newsquad" => shell.update(cx, |shell, cx| {
                    let (what, rest) = arg.split_once(':').unwrap_or((arg.as_str(), ""));
                    match what {
                        "type" => shell.new_squad.name.update(cx, |s, cx| s.set_value(rest.to_owned(), window, cx)),
                        "create" => shell.create_squad(crate::views::welcome::Which::New, window, cx),
                        "tab" => shell.new_squad_tab(rest == "back", window, cx),
                        "close" => shell.close_new_squad(window, cx),
                        "failed" => shell.new_squad.failed = Some(format!("{}\nssh: connect to host studio port 22: Operation timed out", equipe_core::t!("welcome.failed_create"))),
                        "far" | "farout" => {
                            if let Ok(endpoint) = crate::target::Endpoint::parse("abel@studio") {
                                shell.endpoint = endpoint;
                            }
                            if what == "farout" {
                                shell.model.conn = crate::state::Conn::Down { reason: crate::failure::Failure::Closed, since: crate::state::Clock::now(), had_snapshot: true };
                            }
                        }
                        _ => shell.open_new_squad(window, cx),
                    }
                    cx.notify();
                }),
                // Disk: the panel, a fix's sheet, every fix's, yes, back, close.
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
                // `akeys:117,1,talk now` presses each key as the keyboard would — through
                // every shortcut on the way — into the box `answer:117,1,` focused a frame before.
                "akeys" => {
                    let mut parts = arg.splitn(3, ',');
                    let ticket: u32 = parts.next().and_then(|t| t.parse().ok()).unwrap_or(0);
                    let n: usize = parts.next().and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).saturating_sub(1);
                    let words = parts.next().unwrap_or_default().to_owned();
                    for ch in words.chars() {
                        let key = if ch == ' ' { "space".to_owned() } else { ch.to_string() };
                        if let Ok(mut stroke) = Keystroke::parse(&key) {
                            stroke.key_char = Some(ch.to_string());
                            window.dispatch_keystroke(stroke, cx);
                        }
                    }
                    let typed = shell.read(cx).turn.fields.get(&(ticket, n)).map(|f| f.read(cx).value().to_string()).unwrap_or_default();
                    eprintln!("akeys typed: {typed:?}");
                }
                // `nebiuskey:<file>`: the Nebius card's box holds the key that file
                // keeps, as if pasted — never on the script's line, never logged.
                "nebiuskey" => shell.update(cx, |shell, cx| {
                    if let Ok(key) = std::fs::read_to_string(&arg) {
                        let input = shell.settings.nebius_key.clone();
                        input.update(cx, |s, cx| s.set_value(key.trim().to_owned(), window, cx));
                    }
                }),
                // `padd:<preset>`: "Add a provider"'s form for it, as its row in the menu opens it;
                // `purl:<url>` types its address; `pconnect` is its Connect (the key's box is `nebiuskey:`).
                "padd" => shell.update(cx, |shell, cx| {
                    if let Some(preset) = equipe_core::model_providers::preset(&arg) {
                        shell.open_provider_form(preset.id, window, cx);
                    }
                }),
                "purl" => shell.update(cx, |shell, cx| {
                    let input = shell.settings.provider_url.clone();
                    input.update(cx, |s, cx| s.set_value(arg.clone(), window, cx));
                }),
                "pconnect" => shell.update(cx, |shell, cx| shell.connect_provider(window, cx)),
                // `keys:words` presses each key as the keyboard would, into what has the keyboard.
                "keys" => {
                    for ch in arg.chars() {
                        let key = if ch == ' ' { "space".to_owned() } else { ch.to_string() };
                        if let Ok(mut stroke) = Keystroke::parse(&key) {
                            stroke.key_char = Some(ch.to_string());
                            window.dispatch_keystroke(stroke, cx);
                        }
                    }
                }
                // `afile:117,1,/path/to/file` attaches a file to ask 1's answer of T-117, as a drop on its box does.
                "afile" => shell.update(cx, |shell, cx| {
                    let mut parts = arg.splitn(3, ',');
                    let ticket: u32 = parts.next().and_then(|t| t.parse().ok()).unwrap_or(0);
                    let n: usize = parts.next().and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).saturating_sub(1);
                    let path = std::path::PathBuf::from(parts.next().unwrap_or_default());
                    shell.attach_sources_to(crate::views::composer::AttachTo::Turn(ticket, n), vec![crate::attach::Source::File(path)], cx);
                }),
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
                "dtake" | "dmix" | "dagain" => shell.update(cx, |shell, cx| shell.decide_script_react(&verb, &arg, window, cx)),
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
                // A change to the team the PM proposed: picked in Needs you (peeking), or answered.
                "tpick" => shell.update(cx, |shell, cx| {
                    shell.board.view.peek = true;
                    shell.board_pick(crate::state::Need::Team(arg.parse().unwrap_or(0)), cx);
                }),
                // A routine the PM proposed: picked in Needs you (peeking), or answered.
                "rpick" => shell.update(cx, |shell, cx| {
                    shell.board.view.peek = true;
                    shell.board_pick(crate::state::Need::Routine(arg.parse().unwrap_or(0)), cx);
                }),
                "ryes" | "rno" => shell.update(cx, |shell, cx| {
                    if let Some(id) = shell.model.routine_asks.first().map(|r| r.id) {
                        shell.routine_decide(id, verb == "ryes", cx);
                    }
                }),
                "tyes" | "tno" => shell.update(cx, |shell, cx| {
                    if let Some(id) = shell.model.team.first().map(|c| c.id) {
                        shell.team_decide(id, verb == "tyes", cx);
                    }
                }),
                // Work to confirm: picked in Needs you (peeking), a ticket ticked off, or answered.
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
                // Briefs and plans, all in one place: `docs`, then `docsfind:<words>`,
                // `docsfilter:<project:<name>|squad|kind:<brief|plan>|status:<draft|in_review|settled|dropped>|clear>`,
                // `docsopen:<n>` (the nth row, beside) and `docskeep:<set>,<n>` (keep copy n of set n).
                "docs" => shell.update(cx, |shell, cx| shell.open_docs(cx)),
                "docsfind" => shell.update(cx, |shell, cx| {
                    shell.docs_search.update(cx, |s, cx| s.set_value(arg.clone(), window, cx));
                    shell.docs_typed(cx);
                }),
                "docsfilter" => shell.update(cx, |shell, cx| {
                    use crate::docs::{Filter, Whose};
                    let (what, value) = arg.split_once(':').unwrap_or((arg.as_str(), ""));
                    let filter = &mut shell.shelf.filter;
                    match what {
                        "project" => filter.whose = Whose::Project(value.to_owned()),
                        "squad" => filter.whose = Whose::Squad,
                        "kind" => filter.kind = [equipe_core::protocol::briefs::DocKind::Brief, equipe_core::protocol::briefs::DocKind::Plan].into_iter().find(|k| k.word() == value),
                        "status" => filter.status = Some(crate::board::doc_status(value).1),
                        _ => *filter = Filter::default(),
                    }
                    cx.notify();
                }),
                "docsopen" => shell.update(cx, |shell, cx| {
                    let query = shell.docs_query(cx);
                    let path = crate::docs::listed(&shell.docs, &shell.shelf.filter, &query, shell.shelf.found_for(&query)).get(arg.parse().unwrap_or(0)).map(|d| std::path::PathBuf::from(&d.path));
                    if let Some(path) = path {
                        shell.open_doc(path, cx);
                    }
                }),
                "docskeep" => shell.update(cx, |shell, cx| {
                    let (set, n) = arg.split_once(',').map(|(a, b)| (a.trim().parse().unwrap_or(0), b.trim().parse().unwrap_or(0))).unwrap_or((0usize, 0usize));
                    if let Some(path) = shell.shelf.copies.get(set).and_then(|c| c.paths.get(n)).cloned() {
                        shell.keep_doc_copy(&path, cx);
                    }
                }),
                "slide" => shell.update(cx, |shell, cx| shell.present(arg.parse().unwrap_or(0), cx)),
                "usage" => shell.update(cx, |shell, cx| shell.snapshot_usage(&arg, cx)),
                // The header's member list, open.
                "members" => shell.update(cx, |shell, cx| shell.toggle_popover(crate::views::board::Popover::Members, cx)),
                "rail" => shell.update(cx, |shell, cx| {
                    shell.chat.rail_open = arg != "close";
                    cx.notify();
                }),
                "dfold" => shell.update(cx, |shell, cx| shell.fold_doc(arg != "open", cx)),
                "term" => shell.update(cx, |shell, cx| shell.open_terminal((!arg.is_empty()).then(|| arg.to_owned()), window, cx)),
                "tmove" => shell.update(cx, |shell, cx| shell.move_terminal(cx)),
                // What ⌘C would copy from the terminal, written to a file.
                "tcopy" => shell.update(cx, |shell, _| {
                    let text = shell.terminal.as_ref().and_then(|ui| ui.term.selected()).unwrap_or_default();
                    let _ = std::fs::write(&arg, text);
                }),
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
                // The control bar's button: "Take over" (`btake`) or "Give back" (`bgive`).
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
                // A member's page waiting for the person, opened as Needs you or a notification would.
                // "Open the browser" on a hand-over row: `bhanded:demo/pm`.
                "bhanded" => shell.update(cx, |shell, cx| shell.reveal_handed(&arg, cx)),
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
                // The summary band's "Retry" reached by the keyboard, as Tab does.
                "cfocus" => {
                    window.dispatch_keystroke(Keystroke::parse("f13").unwrap(), cx);
                    let handle = shell.read(cx).chat.compact_focus.0.clone();
                    window.focus(&handle, cx);
                    shell.update(cx, |_, cx| cx.notify());
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
                        // `popup:subagents:<member>`: all of a member's subagents, as "+N more" opens them.
                        other => other.strip_prefix("subagents:").and_then(|name| shell.model.members.iter().find(|m| m.name == name)).map(|m| crate::views::sidebar::Popup::Subagents(m.session)),
                    };
                    cx.notify();
                }),
                "subagent" => shell.update(cx, |shell, cx| {
                    // `subagent:<member>[,<n>]`: its nth (from 0), else its first.
                    let (name, nth) = arg.split_once(',').map_or((arg.as_str(), 0), |(name, n)| (name, n.parse().unwrap_or(0)));
                    let found = shell.model.members.iter().find(|m| m.name == name).and_then(|m| shell.model.subagents_of(m.session).get(nth).map(|(s, sub)| (m.session, *s, sub.id.clone())));
                    if let Some((member, session, id)) = found {
                        shell.open_subagent(member, session, &id, window, cx);
                    }
                }),
                // Settings, at a category (`settings:team`), or where it was left.
                "settings" => shell.update(cx, |shell, cx| shell.open_settings(crate::settings::Category::parse(&arg), window, cx)),
                // Deciding in words: "All by words" on or off, or asked about (`words:on|off|ask`); the log of where
                // words went wrong (`wlog`); the card's button on what waits in words (`wundo`).
                "words" => shell.update(cx, |shell, cx| match arg.as_str() {
                    "on" => shell.set_all_by_words(true, cx),
                    "off" => shell.set_all_by_words(false, cx),
                    _ => {
                        shell.settings.words_consent = true;
                        cx.notify();
                    }
                }),
                "wlog" => shell.update(cx, |shell, cx| shell.open_words_log(cx)),
                "wundo" => shell.update(cx, |shell, cx| {
                    if let Some(id) = shell.model.words.front().map(|l| l.id) {
                        shell.words_stop(id, cx);
                    }
                }),
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
                // (Sign in, Cancel, Sign out and its Sign out or Keep; `tab` moves the keys' focus to its first button).
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
                // The conversation's card for a lost sign-in: `signin:<go|move|check|cancel|carry|far|paste:<code>|type:<code>>`
                // (its sign in again, "Start again in the login session", "Check again", "Cancel", "Carry on"; `far`, the window as if on another Mac; a code typed and sent, or only typed).
                "signin" => shell.update(cx, |shell, cx| {
                    use crate::views::signin::Offer;
                    let engine = shell.model.convo().and_then(|c| crate::views::signin::live(&c.blocks())).map_or(equipe_core::protocol::Engine::Claude, |(_, e)| e);
                    let (act, rest) = arg.split_once(':').unwrap_or((arg.as_str(), ""));
                    match act {
                        "go" => shell.signin_offer(engine, Offer::SignIn, window, cx),
                        "move" => shell.signin_offer(engine, Offer::Move, window, cx),
                        "check" => shell.signin_offer(engine, Offer::Check, window, cx),
                        "cancel" => shell.signin_offer(engine, Offer::Cancel, window, cx),
                        "carry" => shell.signin_offer(engine, Offer::CarryOn, window, cx),
                        // The window as if on another Mac than the daemon's: drawn only, nothing reached.
                        "far" => {
                            if let Ok(endpoint) = crate::target::Endpoint::parse("mac-mini") {
                                shell.endpoint = endpoint;
                            }
                        }
                        "type" | "paste" => {
                            if let Some(input) = shell.signin_code.clone() {
                                input.update(cx, |s, cx| s.set_value(rest.to_owned(), window, cx));
                            }
                            if act == "paste" {
                                shell.signin_paste(engine, window, cx);
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
                        "subagents" => Some(crate::views::settings::Pop::Subagents),
                        "provider" => Some(crate::views::settings::Pop::AddProvider),
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
                // `cmdclick:` and `shiftclick:` the same with ⌘ or ⇧ held (a new tab, a new window).
                "click" | "cmdclick" | "shiftclick" => {
                    let (x, y) = arg.split_once(',').map(|(x, y)| (x.trim().parse().unwrap_or(0.), y.trim().parse().unwrap_or(0.))).unwrap_or((0f32, 0f32));
                    let at = point(px(x), px(y));
                    let modifiers = Modifiers { platform: verb == "cmdclick", shift: verb == "shiftclick", ..Default::default() };
                    let down = MouseDownEvent { button: MouseButton::Left, position: at, modifiers, click_count: 1, first_mouse: false };
                    let down = window.input_from_platform(PlatformInput::MouseDown(down));
                    window.dispatch_event(down, cx);
                    let up = MouseUpEvent { button: MouseButton::Left, position: at, modifiers, click_count: 1 };
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
                // Replying: `quote:reply` (the bar's "Reply"), `quote:write:<words>` (in its box),
                // `quote:add` (⏎), `quote:cancel` (Esc), `quote:edit:<n>` / `quote:remove:<n>` (card n's "Reply" / ✕),
                // `quote:peek:<n>` (the pointer on card n; `quote:peek:` off it), `quote:card:<quote>|<words>`.
                "quote" => shell.update(cx, |shell, cx| {
                    let (what, words) = arg.split_once(':').unwrap_or((arg.as_str(), ""));
                    match what {
                        "open" => shell.quote_open(window, cx),
                        "reply" => shell.quote_reply(window, cx),
                        "write" => shell.quote_type(&words.replace("\\n", "\n"), window, cx),
                        "add" => shell.quote_add(window, cx),
                        "cancel" => shell.quote_cancel(window, cx),
                        "edit" => shell.quote_edit(words.parse().unwrap_or(0), window, cx),
                        "remove" => shell.remove_card(words.parse().unwrap_or(0), cx),
                        "peek" => {
                            shell.composer.peek = words.parse().ok();
                            cx.notify();
                        }
                        "card" => {
                            let (quote, said) = words.split_once('|').unwrap_or((words, ""));
                            shell.quote_card(&quote.replace("\\n", "\n"), &said.replace("\\n", "\n"), cx);
                        }
                        "copy" => shell.quote_copy(cx),
                        "type" => {
                            shell.composer.set_text(words, window, cx);
                            cx.notify();
                        }
                        "send" => shell.send(window, cx),
                        "close" => {
                            shell.composer.replies.clear();
                            cx.notify();
                        }
                        "jump" => shell.quote_jump_latest(cx),
                        _ => {}
                    }
                }),
                // A side chat about words selected (`views/side_chat.rs`).
                "sidechat" => shell.update(cx, |shell, cx| {
                    let (what, words) = arg.split_once(':').unwrap_or((arg.as_str(), ""));
                    match what {
                        "start" => shell.side_chat_start(window, cx),
                        "on" => shell.side_chat_open_on(&words.replace("\\n", "\n"), window, cx),
                        "type" => shell.side_chat_type(&words.replace("\\n", "\n"), window, cx),
                        "send" => shell.side_chat_send(window, cx),
                        "close" => shell.side_chat_close(window, cx),
                        "bring" => shell.side_chat_bring(window, cx),
                        "jump" => shell.side_chat_jump(cx),
                        "reopen" => {
                            if let Some(viewed) = shell.model.viewed() {
                                shell.side_chat_show(viewed, window, cx);
                            }
                        }
                        _ => {}
                    }
                }),
                // Tickets: the layout, the sheet, a menu, the page, a choice, a drag by hand.
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
