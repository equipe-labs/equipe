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

use super::core::{Action, Core};

/// What `equipe pair` / `equipe devices` ask, from their connection.
enum Ask {
    Pair(UnboundedSender<Value>),
    Answer(bool),
    Cancel,
    Devices(UnboundedSender<Value>),
    Remove(String, UnboundedSender<Value>),
}

enum Input {
    Relay(Value),
    RelayGone,
    Ask(Ask),
    /// An event of a squad a device follows.
    Device(String, String, Event),
    /// A device's squad is followed, or couldn't be.
    Opened(String, String, Value, Result<(Commands, equipe_core::protocol::rpc::Hello), String>),
    /// An event of any squad, for pushes.
    Squad(String, Event),
}

/// Run in the daemon for as long as it lives.
pub async fn serve_forever(home: PathBuf, socket: PathBuf, version: String) {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Input>();
    if let Err(err) = listen(&home, input_tx.clone()) {
        eprintln!("remote: {err}");
        return;
    }
    let mut core = Core::new(super::load(&home));
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
                        match Client::connect(&socket, &version, &squad).await {
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
                            Err(err) => eprintln!("remote: following {squad}: {err}"),
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
                    if core.config.is_none() {
                        match super::register(&home).await {
                            Ok(config) => core.config = Some(config),
                            Err(why) => {
                                let _ = conn.send(json!({ "error": why }));
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
                                let _ = conn.send(json!({ "error": format!("Couldn't reach the relay: {err}") }));
                                continue;
                            }
                        }
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
                Ask::Cancel => {
                    pair_conn = None;
                    actions.extend(core.end_pairing());
                }
                Ask::Devices(conn) => {
                    let list: Vec<Value> = core
                        .config
                        .iter()
                        .flat_map(|c| c.devices.iter())
                        .map(|d| json!({ "name": d.name, "platform": d.platform, "added": d.added }))
                        .collect();
                    let _ = conn.send(json!({ "devices": list, "relay": super::relay_url(core.config.as_ref()), "connected": relay.is_some() }));
                }
                Ask::Remove(name, conn) => match core.remove_device(&name) {
                    Ok(out) => {
                        actions.extend(out);
                        let _ = conn.send(json!({ "removed": name, "told_relay": relay.is_some() }));
                    }
                    Err(why) => {
                        let _ = conn.send(json!({ "error": why }));
                    }
                },
            },
            Some(Input::Device(device, squad, event)) => actions.extend(core.on_event(&device, &squad, &event)),
            Some(Input::Opened(device, squad, id, result)) => {
                let hello = match result {
                    Ok((commands, hello)) => {
                        senders.insert((device.clone(), squad.clone()), commands);
                        Ok(hello)
                    }
                    Err(why) => Err(why),
                };
                actions.extend(core.opened(&device, &squad, &id, hello));
            }
            Some(Input::Squad(squad, event)) => actions.extend(core.on_squad_event(&squad, &event)),
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
                                let _ = tx.send(Input::Opened(device.clone(), squad.clone(), id, Ok((commands, hello))));
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
                        eprintln!("remote: couldn't keep the devices: {err}");
                    }
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
                    eprintln!("remote: {err}");
                    retry_at = tokio::time::Instant::now() + backoff;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                    if let Some(conn) = &pair_conn {
                        let _ = conn.send(json!({ "error": format!("Couldn't reach the relay: {err}") }));
                        pair_conn = None;
                        core.end_pairing();
                    }
                }
            }
        }
    }
}

/// The relay's socket for this daemon: frames in as `Input::Relay`, a sender out.
async fn connect(config: &super::Config, input: UnboundedSender<Input>) -> Result<UnboundedSender<Ws>, String> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
    let base = super::relay_url(Some(config)).replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
    let mut request = format!("{base}/v1/daemons/{}/connect", config.id).into_client_request().map_err(|err| err.to_string())?;
    request.headers_mut().insert("Authorization", format!("Bearer {}", config.token).parse().map_err(|_| "bad token")?);
    let (stream, _) = tokio_tungstenite::connect_async(request).await.map_err(|err| format!("the relay: {err}"))?;
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
            "cancel" => Ask::Cancel,
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
        let _ = input.send(Input::Ask(Ask::Cancel));
    }
    drop(out_tx);
    let _ = writer_task.await;
}
