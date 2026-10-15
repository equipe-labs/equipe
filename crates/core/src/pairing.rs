//! Pairing a phone from a screen. The daemon's phone link (the `equipe`
//! binary's `remote::link`, which holds the relay connection, the code and the
//! paired devices) answers on its own local socket, the one `equipe pair` and
//! `equipe devices` talk to. A screen asks over the protocol instead —
//! [`Command::PairStart`] and the rest — and the gateway carries each of its
//! asks to that socket and each answer back as an [`Event`], to that screen
//! alone. So the terminal and the app pair a phone the one same way, and a
//! window on another Mac pairs the phone with the daemon's machine.
//!
//! A code is a secret: it passes through here to the screen and is never
//! logged or kept.

use std::path::{Path, PathBuf};

use equipe_protocol::rpc::Message;
use equipe_protocol::{Command, Event, PairEnd, PairedDevice};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc::{self, UnboundedSender};

/// Where the phone link answers.
pub fn socket_path(home: &Path) -> PathBuf {
    home.join("remote.sock")
}

/// Whether `command` is about pairing phones (the gateway's, not the runtime's).
pub fn is_pairing(command: &Command) -> bool {
    matches!(command, Command::PairStart | Command::PairAnswer { .. } | Command::PairCancel | Command::Devices | Command::Unpair { .. })
}

/// What `command` asks the phone link, line by line.
pub fn asks(command: &Command) -> Vec<Value> {
    match command {
        Command::PairStart => vec![json!({ "op": "pair" })],
        Command::PairAnswer { approve } => vec![json!({ "op": "answer", "approve": approve })],
        Command::PairCancel => vec![json!({ "op": "cancel" })],
        Command::Devices => vec![json!({ "op": "devices" })],
        Command::Unpair { device } => vec![json!({ "op": "remove", "name": device })],
        _ => vec![],
    }
}

/// A line from the phone link, as the event a screen is told; `now_ms` is
/// this machine's clock, which the link's expiry is on. `None`: nothing to
/// tell (a device forgotten is told by the list asked for after it).
pub fn event_of(said: &Value, now_ms: u64) -> Option<Event> {
    let text = |key: &str| said[key].as_str().map(str::to_owned);
    match said["op"].as_str() {
        Some("devices") => {
            let devices = said["devices"].as_array().into_iter().flatten().filter_map(|d| serde_json::from_value::<PairedDevice>(d.clone()).ok()).collect();
            return Some(Event::Devices { devices, relay: text("relay").unwrap_or_default(), connected: said["connected"] == true });
        }
        Some("remove") => return None,
        _ => {}
    }
    if let Some(url) = text("qr") {
        let seconds = said["expires"].as_u64().map_or(0, |at| at.saturating_sub(now_ms) / 1000);
        return Some(Event::PairOffer { url, seconds });
    }
    if let Some(request) = said.get("request") {
        return Some(Event::PairAsked { name: request["name"].as_str().unwrap_or_default().to_owned(), platform: request["platform"].as_str().unwrap_or_default().to_owned() });
    }
    if let Some(name) = text("paired") {
        return Some(Event::Paired { name });
    }
    if said.get("refused").is_some() {
        return Some(Event::PairEnded { why: PairEnd::Refused, message: String::new() });
    }
    let message = text("error")?;
    let why = match said["why"].as_str() {
        Some("expired") => PairEnd::Expired,
        Some("busy") => PairEnd::Busy,
        Some("relay") => PairEnd::Relay,
        _ => PairEnd::Failed,
    };
    Some(Event::PairEnded { why, message })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// One screen's line to the phone link, opened on its first pairing command
/// and closed with the screen's connection — which, mid-pairing, closes the
/// code too.
pub struct Bridge {
    home: PathBuf,
    /// The language the screen speaks, for what the bridge itself says.
    pub lang: crate::i18n::Lang,
    link: Option<UnboundedSender<Value>>,
    /// What reads the link's answers, stopped with the bridge.
    reader: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if let Some(reader) = self.reader.take() {
            reader.abort();
        }
    }
}

impl Bridge {
    pub fn new(home: &Path, lang: Option<crate::i18n::Lang>) -> Self {
        Self { home: home.to_owned(), lang: lang.unwrap_or(crate::i18n::Lang::En), link: None, reader: None }
    }

    /// Carry `command` to the phone link; its answers go to `out` as events.
    pub async fn send(&mut self, command: &Command, out: &UnboundedSender<Message>) {
        if self.link.as_ref().is_none_or(|l| l.is_closed()) {
            self.link = None;
            match UnixStream::connect(socket_path(&self.home)).await {
                Ok(stream) => {
                    let (link, reader) = Self::open(stream, out.clone());
                    if let Some(old) = self.reader.replace(reader) {
                        old.abort();
                    }
                    self.link = Some(link);
                }
                Err(_) => {
                    // Nothing to close or forget when nothing answers; a start or a list says why.
                    if matches!(command, Command::PairStart | Command::Devices | Command::Unpair { .. }) {
                        let event = Event::PairEnded { why: PairEnd::Unavailable, message: crate::i18n::core(self.lang).get("pairing.unavailable").to_owned() };
                        let _ = out.send(Message::event(&event));
                    }
                    return;
                }
            }
        }
        if let Some(link) = &self.link {
            for ask in asks(command) {
                let _ = link.send(ask);
            }
        }
    }

    fn open(stream: UnixStream, out: UnboundedSender<Message>) -> (UnboundedSender<Value>, tokio::task::JoinHandle<()>) {
        let (reader, mut writer) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
        // Weak: the line closes when the screen's side lets go of it.
        let again = tx.downgrade();
        tokio::spawn(async move {
            while let Some(ask) = rx.recv().await {
                if writer.write_all(format!("{ask}\n").as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(said) = serde_json::from_str::<Value>(&line) else { continue };
                // A device forgotten: the screen gets the list as it is now.
                if said["op"] == "remove"
                    && let Some(link) = again.upgrade()
                {
                    let _ = link.send(json!({ "op": "devices" }));
                }
                if let Some(event) = event_of(&said, now_ms())
                    && out.send(Message::event(&event)).is_err()
                {
                    break;
                }
            }
        });
        (tx, reader)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pairing_commands_go_to_the_phone_link_and_each_says_what_it_asks() {
        assert!(is_pairing(&Command::PairStart) && is_pairing(&Command::Unpair { device: "a".into() }));
        assert!(!is_pairing(&Command::Desktop { open: true }));
        assert_eq!(asks(&Command::PairStart), vec![json!({ "op": "pair" })]);
        assert_eq!(asks(&Command::PairAnswer { approve: false }), vec![json!({ "op": "answer", "approve": false })]);
        assert_eq!(asks(&Command::Unpair { device: "ab12".into() }), vec![json!({ "op": "remove", "name": "ab12" })]);
        assert!(asks(&Command::Desktop { open: true }).is_empty());
    }

    #[test]
    fn what_the_phone_link_says_becomes_events_for_the_screen() {
        let now = 1_760_000_000_000;
        assert_eq!(event_of(&json!({ "qr": "equipe://pair?s=x", "expires": now + 299_500 }), now), Some(Event::PairOffer { url: "equipe://pair?s=x".into(), seconds: 299 }));
        assert_eq!(event_of(&json!({ "qr": "u", "expires": now - 5 }), now), Some(Event::PairOffer { url: "u".into(), seconds: 0 }), "a late code has no time left, never a wrapped-around one");
        assert_eq!(event_of(&json!({ "request": { "name": "Abel's iPhone", "platform": "ios" } }), now), Some(Event::PairAsked { name: "Abel's iPhone".into(), platform: "ios".into() }));
        assert_eq!(event_of(&json!({ "paired": "Abel's iPhone" }), now), Some(Event::Paired { name: "Abel's iPhone".into() }));
        assert_eq!(event_of(&json!({ "refused": "x" }), now), Some(Event::PairEnded { why: PairEnd::Refused, message: String::new() }));
        for (why, end) in [("expired", PairEnd::Expired), ("busy", PairEnd::Busy), ("relay", PairEnd::Relay), ("other", PairEnd::Failed)] {
            assert_eq!(event_of(&json!({ "error": "words", "why": why }), now), Some(Event::PairEnded { why: end, message: "words".into() }));
        }
        assert_eq!(event_of(&json!({ "error": "words" }), now), Some(Event::PairEnded { why: PairEnd::Failed, message: "words".into() }));
        // A forgetting, right or wrong, is told by the list asked for after it.
        assert_eq!(event_of(&json!({ "op": "remove", "error": "no such device" }), now), None);
        let list = json!({ "op": "devices", "relay": "https://r", "connected": true, "devices": [
            { "id": "a1", "name": "iPhone", "platform": "ios", "added": 10, "online": true },
            { "id": "b2", "name": "Pixel", "platform": "android", "added": 20, "online": false, "seen": 30 },
            { "broken": true },
        ] });
        let Some(Event::Devices { devices, relay, connected }) = event_of(&list, now) else { panic!() };
        assert_eq!((relay.as_str(), connected, devices.len()), ("https://r", true, 2), "a row that doesn't read is left out");
        assert_eq!((devices[0].online, devices[0].seen, devices[1].seen), (true, None, Some(30)));
        assert_eq!(event_of(&json!({ "something": "else" }), now), None);
    }

    /// A stand-in phone link on `home`'s socket: answers `pair` with a code,
    /// then a phone asking; `answer` with paired; `devices` with a list;
    /// `remove` with removed. Every line it read goes to `seen`.
    async fn fake_link(home: &Path) -> mpsc::UnboundedReceiver<Value> {
        let listener = tokio::net::UnixListener::bind(socket_path(home)).unwrap();
        let (seen_tx, seen) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let seen_tx = seen_tx.clone();
                tokio::spawn(async move {
                    let (reader, mut writer) = stream.into_split();
                    let mut lines = BufReader::new(reader).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let ask: Value = serde_json::from_str(&line).unwrap();
                        let _ = seen_tx.send(ask.clone());
                        let said: Vec<Value> = match ask["op"].as_str().unwrap() {
                            "pair" => vec![json!({ "qr": "equipe://pair?s=secret", "expires": now_ms() + 300_000 }), json!({ "request": { "name": "iPhone", "platform": "ios" } })],
                            "answer" if ask["approve"] == true => vec![json!({ "paired": "iPhone" })],
                            "answer" => vec![json!({ "refused": "iPhone" })],
                            "devices" => vec![json!({ "op": "devices", "devices": [{ "id": "a1", "name": "iPhone", "platform": "ios", "added": 1 }], "relay": "https://r", "connected": true })],
                            "remove" => vec![json!({ "op": "remove", "removed": ask["name"], "told_relay": true })],
                            _ => vec![],
                        };
                        for line in said {
                            writer.write_all(format!("{line}\n").as_bytes()).await.unwrap();
                        }
                    }
                    let _ = seen_tx.send(json!({ "closed": true }));
                });
            }
        });
        seen
    }

    async fn next_event(rx: &mut mpsc::UnboundedReceiver<Message>) -> Event {
        let message = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.expect("an event in time").expect("the channel open");
        message.params_as::<Event>().expect("an event")
    }

    #[tokio::test]
    async fn a_screen_pairs_a_phone_through_the_link_and_lists_and_forgets_devices() {
        let home = std::env::temp_dir().join(format!("eqpair-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let mut seen = fake_link(&home).await;
        let (out, mut events) = mpsc::unbounded_channel();
        let mut bridge = Bridge::new(&home, None);
        bridge.send(&Command::PairStart, &out).await;
        let Event::PairOffer { url, seconds } = next_event(&mut events).await else { panic!() };
        assert_eq!(url, "equipe://pair?s=secret");
        assert!((298..=300).contains(&seconds), "{seconds}");
        assert_eq!(next_event(&mut events).await, Event::PairAsked { name: "iPhone".into(), platform: "ios".into() });
        bridge.send(&Command::PairAnswer { approve: true }, &out).await;
        assert_eq!(next_event(&mut events).await, Event::Paired { name: "iPhone".into() });
        bridge.send(&Command::Devices, &out).await;
        let Event::Devices { devices, connected, .. } = next_event(&mut events).await else { panic!() };
        assert_eq!((devices[0].id.as_str(), connected), ("a1", true));
        // Forgetting one is answered with the list, asked again by the bridge.
        bridge.send(&Command::Unpair { device: "a1".into() }, &out).await;
        assert!(matches!(next_event(&mut events).await, Event::Devices { .. }));
        let asked: Vec<String> = std::iter::from_fn(|| seen.try_recv().ok()).map(|v| v["op"].as_str().unwrap_or_default().to_owned()).collect();
        assert_eq!(asked, ["pair", "answer", "devices", "remove", "devices"]);
        // The screen leaving closes its line to the link (and so its code there).
        drop(bridge);
        drop(out);
        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), seen.recv()).await.unwrap().unwrap();
        assert_eq!(closed, json!({ "closed": true }));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[tokio::test]
    async fn with_no_phone_link_a_screen_is_told_so_plainly() {
        let home = std::env::temp_dir().join(format!("eqpair-none-{}", std::process::id()));
        let (out, mut events) = mpsc::unbounded_channel();
        let mut bridge = Bridge::new(&home, None);
        bridge.send(&Command::PairStart, &out).await;
        let Event::PairEnded { why, message } = next_event(&mut events).await else { panic!() };
        assert_eq!(why, PairEnd::Unavailable);
        assert!(!message.is_empty());
        // Closing what never opened says nothing.
        bridge.send(&Command::PairCancel, &out).await;
        assert!(events.try_recv().is_err());
    }
}
