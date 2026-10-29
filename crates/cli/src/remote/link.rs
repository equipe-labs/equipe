//! In the daemon: the relay connection, the squads followed for paired
//! devices, and the local socket `equipe pair` and `equipe devices` talk to.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::gateway::{Client, Commands};
use equipe_core::protocol::Event;
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio_tungstenite::tungstenite::Message as Ws;

use equipe_core::t;

use super::core::{Action, Core};

/// What `equipe pair` / `equipe devices` ask, from their connection.
enum Ask {
    Pair(UnboundedSender<Value>),
    Answer(bool),
    /// The asking connection gave up: its pairing, if it has the one open, ends.
    Cancel(UnboundedSender<Value>),
    Devices(UnboundedSender<Value>),
    Remove(String, UnboundedSender<Value>),
    /// `equipe demo reset`, to a running `equipe demo serve`.
    DemoReset { new_code: bool, conn: UnboundedSender<Value> },
}

enum Input {
    Relay(Value),
    RelayGone,
    Ask(Ask),
    /// An event of a squad a device follows.
    Device(String, String, Event),
    /// A device's squad is followed, or couldn't be.
    Opened(String, String, Value, Result<(Commands, Box<equipe_core::protocol::rpc::Hello>), String>),
    /// An event of any squad, for pushes.
    Squad(String, Event),
    /// A member's picture a device asked for, read (`read_image`).
    ImageRead(String, String, u64, Result<equipe_core::protocol::rpc::ImageRead, String>),
    /// A folder's pictures a device asked for, listed (`list_images`).
    ImagesListed(String, String, u64, Result<equipe_core::protocol::rpc::ImageList, String>),
    /// The answer to a device's request, made off the loop (an `upload`'s).
    Answer(String, equipe_core::protocol::rpc::Message),
}

/// A member's picture for a phone: no bigger than `visuals::PHONE_SIDE`,
/// and smaller again until it fits one relay frame.
fn phone_image(home: &Path, ask: &equipe_core::protocol::rpc::ReadImage) -> Result<equipe_core::protocol::rpc::ImageRead, String> {
    use equipe_core::visuals;
    let mut read = visuals::serve(home, ask, Some(visuals::PHONE_SIDE))?;
    for side in [480, 320, 200] {
        if read.data.len() <= super::core::PHONE_IMAGE_MAX {
            break;
        }
        read = visuals::serve(home, ask, Some(side))?;
    }
    if read.data.len() > super::core::PHONE_IMAGE_MAX {
        return Err("the picture is too big for the phone".into());
    }
    Ok(read)
}

/// Run in the daemon for as long as it lives.
pub async fn serve_forever(home: PathBuf, socket: PathBuf, version: String) {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Input>();
    if let Err(err) = listen(&home, input_tx.clone()) {
        equipe_core::daemon_log!("remote: {err}");
        return;
    }
    let mut core = Core::new(super::load(&home));
    // Pieces are written off the loop, one file's after another.
    let uploads = std::sync::Arc::new(std::sync::Mutex::new(super::upload::Uploads::new(&home)));
    let mut relay: Option<UnboundedSender<Ws>> = None;
    let mut retry_at = tokio::time::Instant::now();
    let mut backoff = Duration::from_secs(2);
    let mut pair_conn: Option<UnboundedSender<Value>> = None;
    // For each device and squad it follows: the task carrying the squad's
    // events to it, and where its commands go.
    let mut tasks: HashMap<(String, String), tokio::task::JoinHandle<()>> = HashMap::new();
    let mut senders: HashMap<(String, String), Commands> = HashMap::new();
    let mut followed: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        let input = tokio::select! {
            Some(input) = input_rx.recv() => Some(input),
            _ = tick.tick() => None,
        };
        let now = super::now_ms();
        let mut actions = Vec::new();
        match input {
            None => {
                actions.extend(core.tick(now));
                // Not while a piece is being written: the next tick will do.
                if let Ok(mut uploads) = uploads.try_lock() {
                    uploads.sweep(now);
                }
                core.squads = equipe_core::squads::list(&home)
                    .into_iter()
                    .map(|s| (s.name, s.projects.iter().map(|p| equipe_core::board::project_name(p)).collect()))
                    .collect();
                // With devices to reach, every squad is followed for pushes.
                if core.has_devices() {
                    for (squad, _) in core.squads.clone() {
                        if followed.get(&squad).is_some_and(|h| !h.is_finished()) {
                            continue;
                        }
                        // A daemon slow to answer holds up the phones' link no longer than this.
                        let connected = tokio::time::timeout(Duration::from_secs(20), Client::connect(&socket, &version, &squad)).await;
                        let connected = connected.unwrap_or_else(|_| Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the daemon didn't answer")));
                        match connected {
                            Ok((client, hello)) => {
                                core.attach(&hello.snapshot);
                                let (_, mut events) = client.split();
                                let tx = input_tx.clone();
                                let name = squad.clone();
                                followed.insert(squad, tokio::spawn(async move {
                                    while let Some(event) = events.recv().await {
                                        if tx.send(Input::Squad(name.clone(), event)).is_err() {
                                            break;
                                        }
                                    }
                                }));
                            }
                            Err(err) => equipe_core::daemon_log!("remote: following {squad}: {err}"),
                        }
                    }
                }
            }
            Some(Input::Relay(frame)) => actions.extend(core.on_relay(&frame, now)),
            Some(Input::RelayGone) => {
                relay = None;
                retry_at = tokio::time::Instant::now() + backoff;
                backoff = (backoff * 2).min(Duration::from_secs(60));
                for (_, task) in tasks.drain() {
                    task.abort();
                }
                senders.clear();
            }
            Some(Input::Ask(ask)) => match ask {
                Ask::Pair(conn) => {
                    // Another screen's code is open: it stays, this one waits.
                    if core.pairing_under_way(now) && !pair_conn.as_ref().is_some_and(|c| c.same_channel(&conn)) {
                        let _ = conn.send(json!({ "error": t!("remote.pairing_under_way"), "why": "busy" }));
                        continue;
                    }
                    if core.config.is_none() {
                        match super::register(&home).await {
                            Ok(config) => core.config = Some(config),
                            Err(why) => {
                                let _ = conn.send(json!({ "error": why, "why": "relay" }));
                                continue;
                            }
                        }
                    }
                    // The code shows only once its slot is open at the relay:
                    // a phone quick to scan must find it there.
                    if relay.is_none()
                        && let Some(config) = &core.config
                    {
                        match connect(config, input_tx.clone()).await {
                            Ok(sender) => {
                                relay = Some(sender);
                                backoff = Duration::from_secs(2);
                            }
                            Err(err) => {
                                let _ = conn.send(json!({ "error": t!("link.couldnt_reach_relay", err = err), "why": "relay" }));
                                continue;
                            }
                        }
                    }
                    // The same screen asking again: its old code closes first.
                    if pair_conn.as_ref().is_some_and(|c| c.same_channel(&conn)) {
                        actions.extend(core.end_pairing());
                    }
                    match core.begin_pairing(now) {
                        Ok((_, opened)) => {
                            // The code goes to `equipe pair` when the relay says
                            // the slot is open (`pair_opened`).
                            pair_conn = Some(conn);
                            actions.extend(opened);
                        }
                        Err(why) => {
                            let _ = conn.send(json!({ "error": why }));
                        }
                    }
                }
                Ask::Answer(approve) => actions.extend(core.answer_pairing(approve, now)),
                // Only the connection whose code is open can close it.
                Ask::Cancel(conn) => {
                    if pair_conn.as_ref().is_some_and(|c| c.same_channel(&conn)) {
                        pair_conn = None;
                        actions.extend(core.end_pairing());
                    }
                }
                Ask::Devices(conn) => {
                    let _ = conn.send(json!({ "op": "devices", "devices": core.device_list(), "relay": super::relay_url(core.config.as_ref()), "connected": relay.is_some() }));
                }
                Ask::Remove(name, conn) => match core.remove_device(&name) {
                    Ok(out) => {
                        actions.extend(out);
                        let _ = conn.send(json!({ "op": "remove", "removed": name, "told_relay": relay.is_some() }));
                    }
                    Err(why) => {
                        let _ = conn.send(json!({ "op": "remove", "error": why }));
                    }
                },
                Ask::DemoReset { conn, .. } => {
                    let _ = conn.send(json!({ "error": t!("link.not_demo_daemon") }));
                }
            },
            Some(Input::Device(device, squad, event)) => actions.extend(core.on_event(&device, &squad, &event)),
            Some(Input::Opened(device, squad, id, result)) => {
                let hello = match result {
                    Ok((commands, hello)) => {
                        senders.insert((device.clone(), squad.clone()), commands);
                        Ok(*hello)
                    }
                    Err(why) => Err(why),
                };
                actions.extend(core.opened(&device, &squad, &id, hello));
            }
            Some(Input::Squad(squad, event)) => actions.extend(core.on_squad_event(&squad, &event)),
            Some(Input::ImageRead(device, squad, id, read)) => actions.extend(core.image_read(&device, &squad, id, read)),
            Some(Input::ImagesListed(device, squad, id, listed)) => actions.extend(core.images_listed(&device, &squad, id, listed)),
            Some(Input::Answer(device, message)) => actions.extend(core.answer(&device, &message)),
        }
        for action in actions {
            match action {
                Action::Relay(frame) => {
                    if let Some(relay) = &relay {
                        let _ = relay.send(Ws::text(frame.to_string()));
                    }
                }
                Action::Open { device, squad, id } => {
                    let key = (device.clone(), squad.clone());
                    let tx = input_tx.clone();
                    let socket = socket.clone();
                    let version = version.clone();
                    let task = tokio::spawn(async move {
                        match Client::connect(&socket, &version, &squad).await {
                            Ok((client, hello)) => {
                                let (commands, mut events) = client.split();
                                let _ = tx.send(Input::Opened(device.clone(), squad.clone(), id, Ok((commands, Box::new(hello)))));
                                while let Some(event) = events.recv().await {
                                    if tx.send(Input::Device(device.clone(), squad.clone(), event)).is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(err) => {
                                let _ = tx.send(Input::Opened(device, squad, id, Err(err.to_string())));
                            }
                        }
                    });
                    // A second hello for the same squad replaces the first.
                    if let Some(old) = tasks.insert(key, task) {
                        old.abort();
                    }
                }
                Action::Command { device, squad, command } => {
                    if let Some(commands) = senders.get(&(device, squad)) {
                        commands.send(command);
                    }
                }
                Action::ReadImage { device, squad, id, ask } => {
                    let (tx, home) = (input_tx.clone(), home.clone());
                    tokio::spawn(async move {
                        let read = tokio::task::spawn_blocking(move || phone_image(&home, &ask)).await.unwrap_or_else(|_| Err("the picture couldn't be read".into()));
                        let _ = tx.send(Input::ImageRead(device, squad, id, read));
                    });
                }
                Action::ListImages { device, squad, id, ask } => {
                    let (tx, home) = (input_tx.clone(), home.clone());
                    tokio::spawn(async move {
                        let listed = tokio::task::spawn_blocking(move || equipe_core::visuals::list(&home, &ask)).await.unwrap_or_else(|_| Err("the folder couldn't be read".into()));
                        let _ = tx.send(Input::ImagesListed(device, squad, id, listed));
                    });
                }
                Action::Close { device } => {
                    tasks.retain(|(d, _), task| {
                        if *d == device {
                            task.abort();
                        }
                        *d != device
                    });
                    senders.retain(|(d, _), _| *d != device);
                }
                Action::Local(value) => {
                    let finished = value.get("paired").is_some() || value.get("refused").is_some() || value.get("error").is_some();
                    if let Some(conn) = &pair_conn {
                        let _ = conn.send(value);
                    }
                    if finished {
                        pair_conn = None;
                    }
                }
                Action::Save => {
                    if let Some(config) = &core.config
                        && let Err(err) = super::save(&home, config)
                    {
                        equipe_core::daemon_log!("remote: couldn't keep the devices: {err}");
                    }
                }
                Action::Upload { device, id, piece } => {
                    let uploads = uploads.clone();
                    let tx = input_tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let taken = uploads.lock().unwrap_or_else(|e| e.into_inner()).piece(&device, *piece, super::now_ms());
                        let answer = match taken {
                            Ok(result) => equipe_core::protocol::rpc::Message::response(id, result),
                            Err(refusal) => refusal.answer(id),
                        };
                        let _ = tx.send(Input::Answer(device, answer));
                    });
                }
                Action::UploadCancel { device, upload_id } => {
                    let uploads = uploads.clone();
                    tokio::task::spawn_blocking(move || uploads.lock().unwrap_or_else(|e| e.into_inner()).cancel(&device, &upload_id));
                }
            }
        }
        // The relay, when there's someone to reach through it.
        if relay.is_none() && core.wants_relay() && tokio::time::Instant::now() >= retry_at
            && let Some(config) = &core.config
        {
            match connect(config, input_tx.clone()).await {
                Ok(sender) => {
                    relay = Some(sender);
                    backoff = Duration::from_secs(2);
                    // A pairing opened while it was away opens now.
                    if let Some(frame) = core.reopen_pairing() {
                        let _ = relay.as_ref().map(|r| r.send(Ws::text(frame.to_string())));
                    }
                }
                Err(err) => {
                    equipe_core::daemon_log!("remote: {err}");
                    retry_at = tokio::time::Instant::now() + backoff;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                    if let Some(conn) = &pair_conn {
                        let _ = conn.send(json!({ "error": t!("link.couldnt_reach_relay", err = err), "why": "relay" }));
                        pair_conn = None;
                        core.end_pairing();
                    }
                }
            }
        }
    }
}

/// `equipe demo serve`, in the foreground: a daemon of its own for store
/// reviewers, with a standing code and a scripted squad for each phone
/// (`demo`). It never connects to `equipe daemon` and runs no agent.
pub async fn serve_demo(home: PathBuf, version: String) -> Result<(), String> {
    let mut config = match super::load(&home) {
        Some(config) => config,
        None => super::register(&home).await?,
    };
    // `EQUIPE_RELAY` may point it elsewhere after it registered: the code says where it is.
    config.relay = super::relay_url(Some(&config));
    let secret = super::demo::secret(&home, false).map_err(|err| t!("link.couldnt_keep_demo_code", err = err))?;
    let mut core = Core::new(Some(config));
    core.squads = vec![(super::demo::SQUAD.to_owned(), vec!["website".into(), "server".into()])];
    let url = core.open_standing(secret, super::demo::NAME)?;
    show_code(&home, &url);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Input>();
    listen_demo(&home, input_tx.clone()).map_err(|err| t!("link.couldnt_open", path = home.join("demo.sock").display(), err = err))?;
    let mut demo = super::demo::Demo::default();
    // A reviewer may attach a file too: kept in the demo's own home, swept like any.
    let mut uploads = super::upload::Uploads::new(&home);
    let mut relay: Option<UnboundedSender<Ws>> = None;
    let mut retry_at = tokio::time::Instant::now();
    let mut backoff = Duration::from_secs(2);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        let input = tokio::select! {
            Some(input) = input_rx.recv() => Some(input),
            _ = tick.tick() => None,
        };
        let now = super::now_ms();
        let mut actions = Vec::new();
        // Events as they reach a phone: (device, event).
        let mut events: Vec<(String, Event)> = Vec::new();
        match input {
            None => {
                actions.extend(core.tick(now));
                events.extend(demo.due(now));
            }
            Some(Input::Relay(frame)) => actions.extend(core.on_relay(&frame, now)),
            Some(Input::RelayGone) => {
                relay = None;
                retry_at = tokio::time::Instant::now() + backoff;
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
            Some(Input::Ask(Ask::DemoReset { new_code, conn })) => {
                let names: Vec<String> = core.config.iter().flat_map(|c| c.devices.iter()).map(|d| d.id.clone()).collect();
                for id in &names {
                    actions.extend(core.remove_device(id).unwrap_or_default());
                }
                demo.reset();
                let mut said = json!({ "forgot": names.len(), "told_relay": relay.is_some() });
                if new_code {
                    actions.extend(core.close_standing());
                    match super::demo::secret(&home, true) {
                        Ok(secret) => match core.open_standing(secret, super::demo::NAME) {
                            Ok(url) => {
                                show_code(&home, &url);
                                actions.extend(core.reopen_standing(now));
                                said["code"] = json!(url);
                            }
                            Err(why) => said["error"] = json!(why),
                        },
                        Err(err) => said["error"] = json!(t!("link.couldnt_make_code", err = err)),
                    }
                }
                let _ = conn.send(said);
            }
            Some(_) => {}
        }
        while !actions.is_empty() || !events.is_empty() {
            for (device, event) in events.drain(..) {
                actions.extend(core.on_event(&device, super::demo::SQUAD, &event));
                actions.extend(core.on_own_event(&device, super::demo::SQUAD, &event));
            }
            for action in std::mem::take(&mut actions) {
                match action {
                    Action::Relay(frame) => {
                        if let Some(relay) = &relay {
                            let _ = relay.send(Ws::text(frame.to_string()));
                        }
                    }
                    Action::Open { device, squad, id } => {
                        let hello = demo.hello(&device, &version, now);
                        actions.extend(core.opened(&device, &squad, &id, Ok(hello)));
                    }
                    Action::Command { device, command, .. } => {
                        events.extend(demo.command(&device, command, now).into_iter().map(|e| (device.clone(), e)));
                    }
                    Action::Save => {
                        if let Some(config) = &core.config
                            && let Err(err) = super::save(&home, config)
                        {
                            equipe_core::daemon_log!("demo: couldn't keep the devices: {err}");
                        }
                    }
                    // The demo's squad shows no member's pictures.
                    Action::ReadImage { device, squad, id, .. } => {
                        actions.extend(core.image_read(&device, &squad, id, Err("the demo has no pictures".into())));
                    }
                    Action::ListImages { device, squad, id, .. } => {
                        actions.extend(core.images_listed(&device, &squad, id, Err("the demo has no pictures".into())));
                    }
                    Action::Upload { device, id, piece } => {
                        let answer = match uploads.piece(&device, *piece, now) {
                            Ok(result) => equipe_core::protocol::rpc::Message::response(id, result),
                            Err(refusal) => refusal.answer(id),
                        };
                        actions.extend(core.answer(&device, &answer));
                    }
                    Action::UploadCancel { device, upload_id } => uploads.cancel(&device, &upload_id),
                    Action::Close { .. } | Action::Local(_) => {}
                }
            }
        }
        if relay.is_none() && tokio::time::Instant::now() >= retry_at
            && let Some(config) = core.config.clone()
        {
            match connect(&config, input_tx.clone()).await {
                Ok(sender) => {
                    for action in core.reopen_standing(now) {
                        if let Action::Relay(frame) = action {
                            let _ = sender.send(Ws::text(frame.to_string()));
                        }
                    }
                    relay = Some(sender);
                    backoff = Duration::from_secs(2);
                    equipe_core::daemon_log!("demo: connected to the relay at {}", config.relay);
                }
                Err(err) => {
                    equipe_core::daemon_log!("demo: {err}");
                    retry_at = tokio::time::Instant::now() + backoff;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
    }
}

/// The code, on the terminal and in the demo's home for the review forms.
fn show_code(home: &Path, url: &str) {
    let _ = std::fs::write(home.join("pairing-url.txt"), format!("{url}\n"));
    if let Some(svg) = super::demo::qr_svg(url) {
        let _ = std::fs::write(home.join("pairing-qr.svg"), svg);
    }
    println!("{}\n", t!("link.demo_code_header"));
    println!("{}", super::cli::qr(url));
    println!("{url}\n");
    println!("{}", t!("link.also_in", path = home.join("pairing-url.txt").display()));
}

/// `equipe demo reset`'s socket: one JSON object per line each way.
fn listen_demo(home: &Path, input: UnboundedSender<Input>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let path = home.join("demo.sock");
    let _ = std::fs::remove_file(&path);
    std::fs::create_dir_all(home)?;
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let input = input.clone();
            tokio::spawn(async move {
                let (reader, mut writer) = stream.into_split();
                let Ok(Some(line)) = BufReader::new(reader).lines().next_line().await else { return };
                let Ok(ask) = serde_json::from_str::<Value>(&line) else { return };
                if ask["op"] != "reset" {
                    return;
                }
                let (tx, mut rx) = mpsc::unbounded_channel();
                if input.send(Input::Ask(Ask::DemoReset { new_code: ask["new_code"] == true, conn: tx })).is_err() {
                    return;
                }
                if let Some(said) = rx.recv().await {
                    let _ = writer.write_all(format!("{said}\n").as_bytes()).await;
                }
            });
        }
    });
    Ok(())
}

/// The relay's socket for this daemon: frames in as `Input::Relay`, a sender out.
async fn connect(config: &super::Config, input: UnboundedSender<Input>) -> Result<UnboundedSender<Ws>, String> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
    let base = super::relay_url(Some(config)).replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
    let mut request = format!("{base}/v1/daemons/{}/connect", config.id).into_client_request().map_err(|err| err.to_string())?;
    request.headers_mut().insert("Authorization", format!("Bearer {}", config.token).parse().map_err(|_| "bad token")?);
    let (stream, _) = tokio_tungstenite::connect_async(request).await.map_err(|err| t!("link.the_relay", err = err))?;
    let (mut sink, mut source) = stream.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Ws>();
    tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                out = rx.recv() => match out {
                    Some(message) => if sink.send(message).await.is_err() { break },
                    None => break,
                },
                _ = ping.tick() => if sink.send(Ws::Ping(Vec::new().into())).await.is_err() { break },
            }
        }
    });
    tokio::spawn(async move {
        while let Some(Ok(message)) = source.next().await {
            if let Ws::Text(text) = message
                && let Ok(frame) = serde_json::from_str::<Value>(&text)
                && input.send(Input::Relay(frame)).is_err()
            {
                return;
            }
        }
        let _ = input.send(Input::RelayGone);
    });
    Ok(tx)
}

/// The local socket: one JSON object per line each way.
fn listen(home: &Path, input: UnboundedSender<Input>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let path = super::local_socket(home);
    let _ = std::fs::remove_file(&path);
    std::fs::create_dir_all(home)?;
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve_local(stream, input.clone()));
        }
    });
    Ok(())
}

async fn serve_local(stream: UnixStream, input: UnboundedSender<Input>) {
    let (reader, mut writer) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    let writer_task = tokio::spawn(async move {
        while let Some(value) = out_rx.recv().await {
            if writer.write_all(format!("{value}\n").as_bytes()).await.is_err() {
                break;
            }
        }
    });
    let mut lines = BufReader::new(reader).lines();
    let mut pairing = false;
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(ask) = serde_json::from_str::<Value>(&line) else { continue };
        let ask = match ask["op"].as_str().unwrap_or_default() {
            "pair" => {
                pairing = true;
                Ask::Pair(out_tx.clone())
            }
            "answer" => Ask::Answer(ask["approve"] == true),
            "cancel" => Ask::Cancel(out_tx.clone()),
            "devices" => Ask::Devices(out_tx.clone()),
            "remove" => Ask::Remove(ask["name"].as_str().unwrap_or_default().to_owned(), out_tx.clone()),
            _ => continue,
        };
        if input.send(Input::Ask(ask)).is_err() {
            break;
        }
    }
    // `equipe pair` left (Ctrl-C) before it finished: the slot closes.
    if pairing {
        let _ = input.send(Input::Ask(Ask::Cancel(out_tx.clone())));
    }
    drop(out_tx);
    let _ = writer_task.await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A relay on this machine that opens every slot it's asked to; every
    /// frame the daemon sends it goes to the result.
    async fn fake_relay() -> (String, mpsc::UnboundedReceiver<Value>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (frames_tx, frames) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let frames_tx = frames_tx.clone();
                tokio::spawn(async move {
                    let Ok(ws) = tokio_tungstenite::accept_async(tcp).await else { return };
                    let (mut sink, mut source) = ws.split();
                    while let Some(Ok(message)) = source.next().await {
                        let Ws::Text(text) = message else { continue };
                        let frame: Value = serde_json::from_str(&text).unwrap();
                        if frame["type"] == "pair_open" {
                            let _ = sink.send(Ws::text(json!({ "type": "pair_opened", "slot": frame["slot"] }).to_string())).await;
                        }
                        let _ = frames_tx.send(frame);
                    }
                });
            }
        });
        (format!("http://{addr}"), frames)
    }

    /// One line to the daemon's local socket, as `equipe pair` or a screen's bridge has.
    struct Line {
        lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
        writer: tokio::net::unix::OwnedWriteHalf,
    }

    impl Line {
        async fn open(home: &Path) -> Self {
            for _ in 0..100 {
                if let Ok(stream) = UnixStream::connect(super::super::local_socket(home)).await {
                    let (reader, writer) = stream.into_split();
                    return Self { lines: BufReader::new(reader).lines(), writer };
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            panic!("the daemon's local socket never opened");
        }

        async fn ask(&mut self, ask: Value) -> Value {
            self.writer.write_all(format!("{ask}\n").as_bytes()).await.unwrap();
            self.said().await
        }

        async fn said(&mut self) -> Value {
            let line = tokio::time::timeout(Duration::from_secs(5), self.lines.next_line()).await.expect("an answer in time").unwrap().expect("the line open");
            serde_json::from_str(&line).unwrap()
        }
    }

    async fn frame(frames: &mut mpsc::UnboundedReceiver<Value>) -> (String, String) {
        let frame = tokio::time::timeout(Duration::from_secs(5), frames.recv()).await.expect("a frame in time").unwrap();
        (frame["type"].as_str().unwrap_or_default().to_owned(), frame["slot"].as_str().unwrap_or_default().to_owned())
    }

    #[tokio::test]
    async fn a_code_belongs_to_the_line_that_opened_it_and_closes_with_it() {
        let (relay, mut frames) = fake_relay().await;
        let home = PathBuf::from(format!("/tmp/eqlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let keys = super::super::seal::generate();
        let config = super::super::Config { relay, id: "d1".into(), token: "t".into(), private: keys.private, public: keys.public, devices: vec![] };
        super::super::save(&home, &config).unwrap();
        tokio::spawn(serve_forever(home.clone(), home.join("equipe.sock"), "test".into()));

        // A screen opens a code: shown once the relay has its slot open.
        let mut screen = Line::open(&home).await;
        let first = screen.ask(json!({ "op": "pair" })).await;
        let url = first["qr"].as_str().unwrap_or_else(|| panic!("{first}")).to_owned();
        let (kind, slot) = frame(&mut frames).await;
        assert_eq!(kind, "pair_open");
        // `equipe pair` meanwhile is told one is open, and can't close it.
        let mut terminal = Line::open(&home).await;
        let busy = terminal.ask(json!({ "op": "pair" })).await;
        assert_eq!(busy["why"], "busy", "{busy}");
        terminal.writer.write_all(b"{\"op\":\"cancel\"}\n").await.unwrap();
        let listed = terminal.ask(json!({ "op": "devices" })).await;
        assert_eq!((listed["op"].as_str(), listed["connected"].as_bool(), listed["devices"].as_array().map(Vec::len)), (Some("devices"), Some(true), Some(0)));
        // The screen asks again (its code ran out on screen): the old slot
        // closes and a new code opens.
        let again = screen.ask(json!({ "op": "pair" })).await;
        assert_ne!(again["qr"].as_str().unwrap(), url, "a new code, not the old one");
        assert_eq!(frame(&mut frames).await, ("pair_close".into(), slot.clone()), "the terminal's cancel didn't touch the screen's code; the screen's new ask did");
        let (kind, second) = frame(&mut frames).await;
        assert_eq!(kind, "pair_open");
        // The screen goes away: its code closes, and the terminal may open one.
        drop(screen);
        assert_eq!(frame(&mut frames).await, ("pair_close".into(), second));
        assert!(terminal.ask(json!({ "op": "pair" })).await["qr"].is_string());
        let _ = std::fs::remove_dir_all(&home);
    }
}
