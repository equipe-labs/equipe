//! `equipe pair` and `equipe devices`: asking the daemon, over its local
//! socket, to pair a phone or forget one.

use equipe_core::t;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;

async fn open() -> Result<(tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>, tokio::net::unix::OwnedWriteHalf), String> {
    crate::daemon::ensure_running().await.map_err(|err| t!("pair.daemon_didnt_start", err = err))?;
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
    let stream = stream.ok_or(t!("pair.daemon_not_answering"))?;
    let (reader, writer) = stream.into_split();
    Ok((BufReader::new(reader).lines(), writer))
}

async fn send(writer: &mut tokio::net::unix::OwnedWriteHalf, value: Value) -> Result<(), String> {
    writer.write_all(format!("{value}\n").as_bytes()).await.map_err(|err| err.to_string())
}

async fn next(lines: &mut tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>) -> Result<Value, String> {
    let line = lines.next_line().await.map_err(|err| err.to_string())?.ok_or(t!("pair.daemon_closed"))?;
    serde_json::from_str(&line).map_err(|err| err.to_string())
}

/// The QR code, in half-height blocks, to scan off the terminal.
pub fn qr(text: &str) -> String {
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
    let url = opened["qr"].as_str().ok_or(t!("pair.no_code"))?;
    println!("{}\n", t!("pair.scan_this"));
    println!("{}", qr(url));
    println!("{}\n{url}\n", t!("pair.paste_instead"));
    println!("{}", t!("pair.waiting_for_phone"));
    loop {
        let said = next(&mut lines).await?;
        if let Some(why) = said["error"].as_str() {
            return Err(why.to_owned());
        }
        if let Some(request) = said.get("request") {
            let name = request["name"].as_str().unwrap_or(t!("pair.a_device"));
            let platform = request["platform"].as_str().unwrap_or("?");
            let yes = dialoguer::Confirm::new()
                .with_prompt(t!("pair.confirm_pair", name = name, platform = platform))
                .default(false)
                .interact()
                .map_err(|err| err.to_string())?;
            send(&mut writer, json!({ "op": "answer", "approve": yes })).await?;
            continue;
        }
        if let Some(name) = said["paired"].as_str() {
            return Ok(t!("pair.paired", name = name));
        }
        if said.get("refused").is_some() {
            return Ok(t!("pair.not_paired").into());
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
        return Ok(t!("pair.no_devices").into());
    }
    let relay = said["relay"].as_str().unwrap_or("?");
    let mut out = vec![if said["connected"] == true { t!("pair.devices_header", relay = relay) } else { t!("pair.devices_header_not_connected", relay = relay) }];
    for d in devices {
        out.push(format!("  {}  ({})", d["name"].as_str().unwrap_or("?"), d["platform"].as_str().unwrap_or("?")));
    }
    out.push(t!("pair.devices_footer").into());
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
        t!("pair.forgot", name = name)
    } else {
        t!("pair.forgot_relay_unreachable", name = name)
    })
}

/// `equipe demo code`: the demo's standing code, as kept.
pub fn demo_code(home: &std::path::Path) -> Result<String, String> {
    let config = super::load(home).ok_or(t!("pair.demo_not_registered"))?;
    let secret = super::demo::secret(home, false).map_err(|err| err.to_string())?;
    let mut core = super::core::Core::new(Some(config));
    let url = core.open_standing(secret, super::demo::NAME)?;
    println!("{}", qr(&url));
    Ok(url)
}

/// `equipe demo reset`: forget every demo phone and start the script over —
/// through the running `equipe demo serve`, or on disk when none runs.
pub async fn demo_reset(home: &std::path::Path, new_code: bool) -> Result<String, String> {
    if let Ok(stream) = UnixStream::connect(home.join("demo.sock")).await {
        let (reader, mut writer) = stream.into_split();
        writer.write_all(format!("{}\n", json!({ "op": "reset", "new_code": new_code })).as_bytes()).await.map_err(|err| err.to_string())?;
        let mut lines = BufReader::new(reader).lines();
        let said = next(&mut lines).await?;
        if let Some(why) = said["error"].as_str() {
            return Err(why.to_owned());
        }
        let code = said["code"].as_str().map(|c| format!("\n{}", t!("pair.demo_new_code", code = c))).unwrap_or_default();
        let forgot = if said["told_relay"] == true { t!("pair.demo_forgot", n = said["forgot"]) } else { t!("pair.demo_forgot_relay_offline", n = said["forgot"]) };
        return Ok(format!("{forgot}{code}"));
    }
    let mut config = super::load(home).ok_or(t!("pair.demo_nothing_to_reset"))?;
    let forgot = config.devices.len();
    config.devices.clear();
    super::save(home, &config).map_err(|err| err.to_string())?;
    let code = if new_code {
        super::demo::secret(home, true).map_err(|err| err.to_string())?;
        format!("\n{}", t!("pair.demo_new_code_made"))
    } else {
        String::new()
    };
    Ok(format!("{}{code}", t!("pair.demo_not_running_forgot", n = forgot)))
}
