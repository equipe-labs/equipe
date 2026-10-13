//! `equipe pair` and `equipe devices`: asking the daemon, over its local
//! socket, to pair a phone or forget one.

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;

async fn open() -> Result<(tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>, tokio::net::unix::OwnedWriteHalf), String> {
    crate::daemon::ensure_running().await.map_err(|err| format!("The daemon didn't start: {err}"))?;
    let path = super::local_socket(&equipe_providers::credentials::equipe_home());
    // A daemon just started opens its socket a moment later.
    let mut stream = None;
    for _ in 0..40 {
        if let Ok(s) = UnixStream::connect(&path).await {
            stream = Some(s);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let stream = stream.ok_or("The daemon isn't answering for devices — `equipe daemon restart` starts this build")?;
    let (reader, writer) = stream.into_split();
    Ok((BufReader::new(reader).lines(), writer))
}

async fn send(writer: &mut tokio::net::unix::OwnedWriteHalf, value: Value) -> Result<(), String> {
    writer.write_all(format!("{value}\n").as_bytes()).await.map_err(|err| err.to_string())
}

async fn next(lines: &mut tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>) -> Result<Value, String> {
    let line = lines.next_line().await.map_err(|err| err.to_string())?.ok_or("The daemon closed the connection")?;
    serde_json::from_str(&line).map_err(|err| err.to_string())
}

/// The QR code, in half-height blocks, to scan off the terminal.
fn qr(text: &str) -> String {
    use qrcode::render::unicode::Dense1x2;
    match qrcode::QrCode::new(text.as_bytes()) {
        Ok(code) => code.render::<Dense1x2>().dark_color(Dense1x2::Light).light_color(Dense1x2::Dark).quiet_zone(true).build(),
        Err(_) => String::new(),
    }
}

/// `equipe pair`.
pub async fn pair() -> Result<String, String> {
    let (mut lines, mut writer) = open().await?;
    send(&mut writer, json!({ "op": "pair" })).await?;
    let opened = next(&mut lines).await?;
    if let Some(why) = opened["error"].as_str() {
        return Err(why.to_owned());
    }
    let url = opened["qr"].as_str().ok_or("The daemon sent no code")?;
    println!("Scan this with the equipe app (Settings › Pair with a computer). It works for 5 minutes.\n");
    println!("{}", qr(url));
    println!("The app can't scan? Paste this into it instead:\n{url}\n");
    println!("Waiting for the phone…");
    loop {
        let said = next(&mut lines).await?;
        if let Some(why) = said["error"].as_str() {
            return Err(why.to_owned());
        }
        if let Some(request) = said.get("request") {
            let name = request["name"].as_str().unwrap_or("a device");
            let platform = request["platform"].as_str().unwrap_or("?");
            let yes = dialoguer::Confirm::new()
                .with_prompt(format!("Pair {name} ({platform})? It can follow your squads, answer requests once, accept work and talk to members"))
                .default(false)
                .interact()
                .map_err(|err| err.to_string())?;
            send(&mut writer, json!({ "op": "answer", "approve": yes })).await?;
            continue;
        }
        if let Some(name) = said["paired"].as_str() {
            return Ok(format!("Paired {name}. `equipe devices` lists paired devices; `equipe devices remove \"{name}\"` forgets it."));
        }
        if said.get("refused").is_some() {
            return Ok("Not paired.".into());
        }
    }
}

/// `equipe devices`.
pub async fn list() -> Result<String, String> {
    let (mut lines, mut writer) = open().await?;
    send(&mut writer, json!({ "op": "devices" })).await?;
    let said = next(&mut lines).await?;
    let devices = said["devices"].as_array().cloned().unwrap_or_default();
    if devices.is_empty() {
        return Ok("No devices paired · `equipe pair` pairs a phone".into());
    }
    let mut out = vec![format!("Paired devices · relay {}{}", said["relay"].as_str().unwrap_or("?"), if said["connected"] == true { "" } else { " (not connected)" })];
    for d in devices {
        out.push(format!("  {}  ({})", d["name"].as_str().unwrap_or("?"), d["platform"].as_str().unwrap_or("?")));
    }
    out.push("`equipe devices remove <name>` forgets one at once".into());
    Ok(out.join("\n"))
}

/// `equipe devices remove <name>`.
pub async fn remove(name: &str) -> Result<String, String> {
    let (mut lines, mut writer) = open().await?;
    send(&mut writer, json!({ "op": "remove", "name": name })).await?;
    let said = next(&mut lines).await?;
    if let Some(why) = said["error"].as_str() {
        return Err(why.to_owned());
    }
    Ok(if said["told_relay"] == true {
        format!("Forgot {name}: it can't reach this machine any more.")
    } else {
        format!("Forgot {name}: this machine won't open anything from it. (The relay wasn't reachable to drop it too; nothing it sends can be read.)")
    })
}
