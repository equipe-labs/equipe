//! What the daemon does for paired devices, as plain state: relay frames,
//! squad events and pairing in; frames for the relay, commands for squads and
//! answers for `equipe pair` out. The I/O is `link`'s.

use std::collections::{BTreeMap, HashMap, HashSet};

use equipe_core::protocol::rpc::{self, Hello, Message};
use equipe_core::protocol::{Command, Decision, Event, Snapshot, TicketId, TicketStatus};
use serde_json::{Value, json};

use super::seal::{self, Kind};
use super::{Config, Device};

/// How long a QR code can be scanned.
pub const PAIRING_MS: u64 = 5 * 60 * 1000;
/// Frames whose clock is further off than this are dropped.
const CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;
/// Of a request's command, this much travels in a push.
const PUSH_ACTION: usize = 800;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// A frame for the relay.
    Relay(Value),
    /// Follow `squad` for `device`, and answer its `hello` (`id`) with the snapshot.
    Open { device: String, squad: String, id: Value },
    /// A command from `device` for `squad`.
    Command { device: String, squad: String, command: Command },
    /// The device left: stop following squads for it.
    Close { device: String },
    /// For `equipe pair`, waiting at the terminal.
    Local(Value),
    /// The config changed: keep it.
    Save,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub name: String,
    pub platform: String,
    pub key: String,
}

#[derive(Debug)]
struct Pairing {
    slot: String,
    secret: [u8; 32],
    expires: u64,
    request: Option<Request>,
    /// The QR code's address, shown once the relay says the slot is open.
    url: String,
    started: u64,
    shown: bool,
}

#[derive(Debug, Default)]
pub struct Core {
    pub config: Option<Config>,
    online: HashSet<String>,
    seq_in: HashMap<String, u64>,
    seq_out: HashMap<String, u64>,
    pairing: Option<Pairing>,
    /// Every squad, with its projects' names — for `squads`.
    pub squads: Vec<(String, Vec<String>)>,
    /// Ticket statuses per squad, to push only on a change.
    tickets: BTreeMap<String, HashMap<TicketId, TicketStatus>>,
    following: HashSet<(String, String)>,
}

/// The pairing slot for a secret: what the relay knows it by.
pub fn slot(secret: &[u8]) -> String {
    super::sha256_hex(secret)[..32].to_owned()
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The commands a phone may send, as it may send them: from a phone a request
/// is allowed once, never always; starting, closing, merging and changing
/// modes stay at the console.
pub fn allowed(command: Command) -> Option<Command> {
    match command {
        Command::Decide { squad, approval, decision: Decision::Always } => Some(Command::Decide { squad, approval, decision: Decision::Allow }),
        c @ (Command::Submit { .. }
        | Command::Interrupt { .. }
        | Command::Accept { .. }
        | Command::Reopen { .. }
        | Command::Resume { .. }
        | Command::Cancel { .. }
        | Command::Decide { .. }
        | Command::Focus { .. }
        | Command::Refresh { .. }) => Some(c),
        _ => None,
    }
}

/// The squad a command names, when it names one.
fn squad_of(command: &Command) -> Option<&str> {
    match command {
        Command::Accept { squad, .. }
        | Command::Merge { squad, .. }
        | Command::Reopen { squad, .. }
        | Command::Resume { squad, .. }
        | Command::Cancel { squad, .. }
        | Command::SetMode { squad, .. }
        | Command::Decide { squad, .. }
        | Command::Scale { squad, .. }
        | Command::Focus { squad, .. }
        | Command::ShowBrowser { squad, .. } => Some(squad),
        _ => None,
    }
}

impl Core {
    pub fn new(config: Option<Config>) -> Self {
        Self { config, ..Self::default() }
    }

    fn device(&self, id: &str) -> Option<&Device> {
        self.config.as_ref()?.devices.iter().find(|d| d.id == id)
    }

    /// Whether the relay is worth a connection: devices to reach, or one pairing.
    pub fn wants_relay(&self) -> bool {
        self.config.as_ref().is_some_and(|c| !c.devices.is_empty()) || self.pairing.is_some()
    }

    pub fn has_devices(&self) -> bool {
        self.config.as_ref().is_some_and(|c| !c.devices.is_empty())
    }

    /// `equipe pair`: open a slot; the QR code's address, and what to tell the relay.
    pub fn begin_pairing(&mut self, now: u64) -> Result<(String, Vec<Action>), String> {
        let config = self.config.as_ref().ok_or("not registered with the relay")?;
        if self.pairing.as_ref().is_some_and(|p| p.expires > now) {
            return Err("A pairing is already under way — finish it, or wait a few minutes".into());
        }
        let secret = super::random::<32>();
        let slot = slot(&secret);
        let expires = now + PAIRING_MS;
        let url = format!(
            "equipe://pair?r={}&d={}&k={}&s={}&n={}",
            encode(&config.relay),
            config.id,
            config.public,
            super::b64(&secret),
            encode(&super::hostname())
        );
        self.pairing = Some(Pairing { slot: slot.clone(), secret, expires, request: None, url: url.clone(), started: now, shown: false });
        Ok((url, vec![Action::Relay(json!({ "type": "pair_open", "slot": slot, "expires": expires }))]))
    }

    /// The person's answer to a pairing request.
    pub fn answer_pairing(&mut self, approve: bool, now: u64) -> Vec<Action> {
        let Some(pairing) = self.pairing.take() else {
            return vec![Action::Local(json!({ "error": "nothing to answer" }))];
        };
        let (Some(request), Some(config)) = (pairing.request, self.config.as_mut()) else {
            return vec![Action::Local(json!({ "error": "no device asked yet" }))];
        };
        let keys = config.keys();
        let aad = config.id.clone();
        let close = Action::Relay(json!({ "type": "pair_close", "slot": pairing.slot }));
        let answer = |body: Value| seal::seal(Kind::PairAnswer, &request.key, Some(&keys), Some(&pairing.secret), &aad, body.to_string().as_bytes());
        if !approve {
            let mut out = Vec::new();
            if let Ok(data) = answer(json!({ "type": "refused" })) {
                out.push(Action::Relay(json!({ "type": "pair_send", "slot": pairing.slot, "data": data })));
            }
            out.extend([close, Action::Local(json!({ "refused": request.name }))]);
            return out;
        }
        let id = super::hex(&super::random::<16>());
        let token = super::b64(&super::random::<32>());
        let Ok(data) = answer(json!({ "type": "paired", "device": id, "token": token, "name": super::hostname() })) else {
            return vec![close, Action::Local(json!({ "error": "the device's key didn't work" }))];
        };
        config.devices.retain(|d| d.name != request.name || d.key != request.key);
        config.devices.push(Device { id: id.clone(), name: request.name.clone(), platform: request.platform.clone(), key: request.key, added: now / 1000 });
        vec![
            Action::Save,
            Action::Relay(json!({ "type": "device_add", "device": id, "token_hash": super::sha256_hex(token.as_bytes()) })),
            Action::Relay(json!({ "type": "pair_send", "slot": pairing.slot, "data": data })),
            close,
            Action::Local(json!({ "paired": request.name })),
        ]
    }

    /// A pairing under way, to open again on a relay connection made since.
    pub fn reopen_pairing(&self) -> Option<Value> {
        self.pairing.as_ref().map(|p| json!({ "type": "pair_open", "slot": p.slot, "expires": p.expires }))
    }

    /// The person gave up, or the time ran out.
    pub fn end_pairing(&mut self) -> Vec<Action> {
        match self.pairing.take() {
            Some(p) => vec![Action::Relay(json!({ "type": "pair_close", "slot": p.slot }))],
            None => vec![],
        }
    }

    /// The minute hand: a pairing past its time ends.
    pub fn tick(&mut self, now: u64) -> Vec<Action> {
        if self.pairing.as_ref().is_some_and(|p| p.expires <= now) {
            let mut out = self.end_pairing();
            out.push(Action::Local(json!({ "error": "The code expired — run equipe pair again" })));
            return out;
        }
        // A relay that doesn't say the slot opened: show the code after a moment anyway.
        if let Some(p) = self.pairing.as_mut().filter(|p| !p.shown && now >= p.started + 3000) {
            p.shown = true;
            return vec![Action::Local(json!({ "qr": p.url, "expires": p.expires }))];
        }
        vec![]
    }

    /// `equipe devices remove <name>`.
    pub fn remove_device(&mut self, name: &str) -> Result<Vec<Action>, String> {
        let config = self.config.as_mut().ok_or("No devices are paired")?;
        let Some(at) = config.devices.iter().position(|d| d.name == name || d.id == name) else {
            let names: Vec<&str> = config.devices.iter().map(|d| d.name.as_str()).collect();
            return Err(format!("No device called {name} · paired: {}", if names.is_empty() { "none".into() } else { names.join(", ") }));
        };
        let device = config.devices.remove(at);
        self.online.remove(&device.id);
        Ok(vec![Action::Save, Action::Relay(json!({ "type": "device_remove", "device": device.id })), Action::Close { device: device.id }])
    }

    /// Seal `rpc` for a device, about `squad`, as the next frame on its connection.
    fn frame_for(&mut self, device: &str, squad: Option<&str>, rpc: &Message) -> Option<Action> {
        let config = self.config.as_ref()?;
        let to = self.device(device)?.key.clone();
        let seq = self.seq_out.entry(device.to_owned()).or_insert(0);
        *seq += 1;
        let mut plain = json!({ "ts": super::now_ms(), "seq": *seq, "rpc": rpc });
        if let Some(squad) = squad {
            plain["squad"] = json!(squad);
        }
        let aad = format!("{}:{device}", config.id);
        let data = seal::seal(Kind::ToDevice, &to, Some(&config.keys()), None, &aad, plain.to_string().as_bytes()).ok()?;
        Some(Action::Relay(json!({ "type": "send", "device": device, "data": data })))
    }

    /// A frame from the relay.
    pub fn on_relay(&mut self, frame: &Value, now: u64) -> Vec<Action> {
        match frame["type"].as_str().unwrap_or_default() {
            "recv" => self.on_device_frame(frame["device"].as_str().unwrap_or_default(), frame["data"].as_str().unwrap_or_default(), now),
            "pair_opened" => match self.pairing.as_mut().filter(|p| !p.shown && frame["slot"] == p.slot.as_str()) {
                Some(p) => {
                    p.shown = true;
                    vec![Action::Local(json!({ "qr": p.url, "expires": p.expires }))]
                }
                None => vec![],
            },
            "pair_recv" => self.on_pair_request(frame["slot"].as_str().unwrap_or_default(), frame["data"].as_str().unwrap_or_default(), now),
            "presence" => {
                let device = frame["device"].as_str().unwrap_or_default().to_owned();
                // A new connection counts its frames from 1.
                self.seq_in.remove(&device);
                self.seq_out.remove(&device);
                if frame["online"] == true {
                    self.online.insert(device);
                    vec![]
                } else {
                    self.online.remove(&device);
                    self.following.retain(|(d, _)| *d != device);
                    vec![Action::Close { device }]
                }
            }
            "push_result" if frame["ok"] != true => {
                eprintln!("remote: push to {} failed: {}", frame["device"], frame["error"]);
                vec![]
            }
            "error" => {
                eprintln!("remote: the relay said {}", frame["message"]);
                vec![]
            }
            _ => vec![],
        }
    }

    fn on_pair_request(&mut self, slot: &str, data: &str, now: u64) -> Vec<Action> {
        let (Some(pairing), Some(config)) = (self.pairing.as_mut(), self.config.as_ref()) else {
            return vec![];
        };
        if pairing.slot != slot || pairing.expires <= now || pairing.request.is_some() {
            return vec![];
        }
        let Ok(plain) = seal::open(Kind::PairRequest, &config.keys(), None, Some(&pairing.secret), &config.id, data) else {
            return vec![];
        };
        let Ok(body) = serde_json::from_slice::<Value>(&plain) else {
            return vec![];
        };
        let (Some(name), Some(key)) = (body["name"].as_str(), body["key"].as_str()) else {
            return vec![];
        };
        if body["type"] != "pair_request" || super::unb64(key).map(|k| k.len()) != Ok(32) {
            return vec![];
        }
        let name: String = name.chars().filter(|c| !c.is_control()).take(60).collect();
        let platform: String = body["platform"].as_str().unwrap_or("unknown").chars().filter(|c| c.is_ascii_alphanumeric()).take(20).collect();
        pairing.request = Some(Request { name: name.clone(), platform: platform.clone(), key: key.to_owned() });
        vec![Action::Local(json!({ "request": { "name": name, "platform": platform } }))]
    }

    fn on_device_frame(&mut self, device: &str, data: &str, now: u64) -> Vec<Action> {
        let (Some(config), Some(known)) = (self.config.as_ref(), self.device(device)) else {
            return vec![];
        };
        let aad = format!("{}:{device}", config.id);
        let Ok(plain) = seal::open(Kind::ToDaemon, &config.keys(), Some(&known.key), None, &aad, data) else {
            return vec![];
        };
        let Ok(frame) = serde_json::from_slice::<Value>(&plain) else {
            return vec![];
        };
        let (ts, seq) = (frame["ts"].as_u64().unwrap_or(0), frame["seq"].as_u64().unwrap_or(0));
        let last = self.seq_in.get(device).copied().unwrap_or(0);
        if ts.abs_diff(now) > CLOCK_SKEW_MS || seq <= last {
            return vec![];
        }
        self.seq_in.insert(device.to_owned(), seq);
        self.online.insert(device.to_owned());
        let Ok(message) = serde_json::from_value::<Message>(frame["rpc"].clone()) else {
            return vec![];
        };
        let squad = frame["squad"].as_str().map(str::to_owned);
        self.on_rpc(device, squad, message)
    }

    fn on_rpc(&mut self, device: &str, squad: Option<String>, message: Message) -> Vec<Action> {
        let id = message.id;
        let fail = |core: &mut Self, code: i64, why: &str| -> Vec<Action> {
            match id {
                Some(id) => core.frame_for(device, squad.as_deref(), &Message::failure(id, code, why)).into_iter().collect(),
                None => vec![],
            }
        };
        match message.method.as_deref() {
            Some(rpc::HELLO) => {
                let Some(id) = id else { return vec![] };
                let wanted = message.params.as_ref().and_then(|p| p["squad"].as_str()).map(str::to_owned);
                match wanted.filter(|s| self.squads.iter().any(|(name, _)| name == s)) {
                    Some(squad) => {
                        self.following.insert((device.to_owned(), squad.clone()));
                        vec![Action::Open { device: device.to_owned(), squad, id: json!(id) }]
                    }
                    None => fail(self, -32602, "no such squad"),
                }
            }
            Some("squads") => {
                let Some(id) = id else { return vec![] };
                let list: Vec<Value> = self.squads.iter().map(|(name, projects)| json!({ "name": name, "projects": projects })).collect();
                self.frame_for(device, None, &Message::response(id, list)).into_iter().collect()
            }
            Some(rpc::COMMAND) => {
                let Some(command) = message.params_as::<Command>() else {
                    return vec![];
                };
                let Some(squad) = squad.filter(|s| self.following.contains(&(device.to_owned(), s.clone()))) else {
                    return vec![];
                };
                if squad_of(&command).is_some_and(|named| named != squad) {
                    return vec![];
                }
                match allowed(command) {
                    Some(command) => vec![Action::Command { device: device.to_owned(), squad, command }],
                    None => vec![],
                }
            }
            _ => fail(self, -32601, "not something a device may ask"),
        }
    }

    /// The squad a device asked for is followed: answer its `hello`.
    pub fn opened(&mut self, device: &str, squad: &str, id: &Value, hello: Result<Hello, String>) -> Vec<Action> {
        let Some(id) = id.as_u64() else { return vec![] };
        let message = match hello {
            Ok(hello) => Message::response(id, hello),
            Err(why) => {
                self.following.remove(&(device.to_owned(), squad.to_owned()));
                Message::failure(id, -32000, why)
            }
        };
        self.frame_for(device, Some(squad), &message).into_iter().collect()
    }

    /// An event of a squad a device follows: on to the device.
    pub fn on_event(&mut self, device: &str, squad: &str, event: &Event) -> Vec<Action> {
        if !self.online.contains(device) {
            return vec![];
        }
        self.frame_for(device, Some(squad), &Message::event(event)).into_iter().collect()
    }

    /// Following every squad for pushes: what each is now.
    pub fn attach(&mut self, snapshot: &Snapshot) {
        self.tickets.insert(snapshot.squad.clone(), snapshot.tickets.iter().map(|t| (t.id, t.status)).collect());
    }

    /// An event in any squad: a push to each device away, when it needs the person.
    pub fn on_squad_event(&mut self, squad: &str, event: &Event) -> Vec<Action> {
        let (title, body, category, collapse, event) = match event {
            Event::ApprovalRequested { approval, .. } => {
                let verb = match approval.tool.as_str() {
                    "Bash" => "to run a command",
                    "Write" | "Edit" | "MultiEdit" => "to edit a file",
                    "WebFetch" => "to fetch a page",
                    "Confirm" => "you to confirm something",
                    _ => "for permission",
                };
                let mut short = approval.clone();
                short.action = short.action.chars().take(PUSH_ACTION).collect();
                let ticket = approval.ticket.map(|t| format!(" · T-{t}")).unwrap_or_default();
                (
                    format!("{} asks {verb}", approval.member),
                    format!("{squad}{ticket}"),
                    "approval",
                    format!("approval-{}", approval.id),
                    Event::ApprovalRequested { squad: squad.to_owned(), approval: short },
                )
            }
            Event::TicketChanged { ticket, .. } => {
                let before = self.tickets.entry(squad.to_owned()).or_default().insert(ticket.id, ticket.status);
                if before == Some(ticket.status) {
                    return vec![];
                }
                let who = ticket.assignee.clone().or(ticket.owner.clone()).unwrap_or_else(|| "A member".into());
                let (title, category) = match ticket.status {
                    TicketStatus::Review => (format!("{who} handed in work to review"), "review"),
                    TicketStatus::Waiting => (format!("{who} needs you"), "waiting"),
                    _ => return vec![],
                };
                let mut short = ticket.clone();
                short.body = String::new();
                short.note = short.note.map(|n| n.chars().take(PUSH_ACTION).collect());
                (title, format!("T-{} · {squad}", ticket.id), category, format!("ticket-{}", ticket.id), Event::TicketChanged { squad: squad.to_owned(), ticket: short })
            }
            _ => return vec![],
        };
        let Some(config) = self.config.as_ref() else { return vec![] };
        let away: Vec<Device> = config.devices.iter().filter(|d| !self.online.contains(&d.id)).cloned().collect();
        let mut out = Vec::new();
        for device in away {
            // Shown, not counted: seq 0.
            let plain = json!({ "ts": super::now_ms(), "seq": 0, "squad": squad, "rpc": Message::event(&event) });
            let aad = format!("{}:{}", config.id, device.id);
            let Ok(data) = seal::seal(Kind::ToDevice, &device.key, Some(&config.keys()), None, &aad, plain.to_string().as_bytes()) else {
                continue;
            };
            out.push(Action::Relay(json!({
                "type": "push", "device": device.id, "title": title.chars().take(200).collect::<String>(),
                "body": body.chars().take(200).collect::<String>(), "category": category, "collapse": collapse, "data": data,
            })));
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use equipe_core::protocol::{Approval, Ticket};

    /// What a phone does: its keys, and the sealing on its side.
    pub struct Phone {
        pub keys: seal::Keys,
        pub id: String,
        seq: u64,
    }

    impl Phone {
        pub fn new() -> Self {
            Self { keys: seal::generate(), id: String::new(), seq: 0 }
        }

        pub fn send(&mut self, config: &Config, squad: Option<&str>, rpc: Value, ts: u64) -> Value {
            self.seq += 1;
            let mut plain = json!({ "ts": ts, "seq": self.seq, "rpc": rpc });
            if let Some(squad) = squad {
                plain["squad"] = json!(squad);
            }
            let aad = format!("{}:{}", config.id, self.id);
            let data = seal::seal(Kind::ToDaemon, &config.public, Some(&self.keys), None, &aad, plain.to_string().as_bytes()).unwrap();
            json!({ "type": "recv", "device": self.id, "data": data })
        }

        pub fn open(&self, config: &Config, action: &Action) -> Value {
            let Action::Relay(frame) = action else { panic!("not for the relay: {action:?}") };
            let aad = format!("{}:{}", config.id, self.id);
            serde_json::from_slice(&seal::open(Kind::ToDevice, &self.keys, Some(&config.public), None, &aad, frame["data"].as_str().unwrap()).unwrap()).unwrap()
        }
    }

    fn config() -> Config {
        let keys = seal::generate();
        Config { relay: "https://relay.test".into(), id: "d1".into(), token: "t".into(), private: keys.private, public: keys.public, devices: vec![] }
    }

    /// Pair a phone through `core`, as the app and the person would.
    pub fn pair(core: &mut Core, phone: &mut Phone, now: u64) {
        let (url, opened) = core.begin_pairing(now).unwrap();
        assert!(matches!(&opened[..], [Action::Relay(f)] if f["type"] == "pair_open"));
        let query: HashMap<String, String> = url.split_once('?').unwrap().1.split('&').map(|kv| {
            let (k, v) = kv.split_once('=').unwrap();
            (k.to_owned(), v.to_owned())
        }).collect();
        let config = core.config.clone().unwrap();
        assert_eq!(query["d"], config.id);
        assert_eq!(query["k"], config.public);
        let secret = super::super::unb64(&query["s"]).unwrap();
        let request = json!({ "type": "pair_request", "name": "Abel's iPhone", "platform": "ios", "key": phone.keys.public });
        let data = seal::seal(Kind::PairRequest, &query["k"], None, Some(&secret), &config.id, request.to_string().as_bytes()).unwrap();
        let asked = core.on_relay(&json!({ "type": "pair_recv", "slot": slot(&secret), "data": data }), now);
        assert_eq!(asked, vec![Action::Local(json!({ "request": { "name": "Abel's iPhone", "platform": "ios" } }))]);
        let done = core.answer_pairing(true, now);
        let send = done.iter().find_map(|a| match a { Action::Relay(f) if f["type"] == "pair_send" => Some(f.clone()), _ => None }).unwrap();
        let answer: Value = serde_json::from_slice(&seal::open(Kind::PairAnswer, &phone.keys, Some(&config.public), Some(&secret), &config.id, send["data"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(answer["type"], "paired");
        phone.id = answer["device"].as_str().unwrap().to_owned();
        let added = done.iter().find_map(|a| match a { Action::Relay(f) if f["type"] == "device_add" => Some(f.clone()), _ => None }).unwrap();
        assert_eq!(added["token_hash"], super::super::sha256_hex(answer["token"].as_str().unwrap().as_bytes()));
        assert!(done.contains(&Action::Save) && done.contains(&Action::Local(json!({ "paired": "Abel's iPhone" }))));
    }

    fn core_with_phone() -> (Core, Phone, u64) {
        let mut core = Core::new(Some(config()));
        core.squads = vec![("acme".into(), vec!["site".into()])];
        let mut phone = Phone::new();
        let now = super::super::now_ms();
        pair(&mut core, &mut phone, now);
        core.on_relay(&json!({ "type": "presence", "device": phone.id, "online": true }), now);
        (core, phone, now)
    }

    #[test]
    fn a_phone_pairs_with_the_secret_and_the_persons_yes() {
        let (core, phone, _) = core_with_phone();
        let devices = &core.config.as_ref().unwrap().devices;
        assert_eq!(devices.len(), 1);
        assert_eq!((devices[0].name.as_str(), devices[0].key.as_str()), ("Abel's iPhone", phone.keys.public.as_str()));
        // A no pairs nothing.
        let mut core = Core::new(Some(config()));
        let (url, _) = core.begin_pairing(1).unwrap();
        let secret = super::super::unb64(url.split("&s=").nth(1).unwrap().split('&').next().unwrap()).unwrap();
        let c = core.config.clone().unwrap();
        let data = seal::seal(Kind::PairRequest, &c.public, None, Some(&secret), &c.id, json!({ "type": "pair_request", "name": "x", "key": Phone::new().keys.public }).to_string().as_bytes()).unwrap();
        core.on_relay(&json!({ "type": "pair_recv", "slot": slot(&secret), "data": data }), 1);
        let out = core.answer_pairing(false, 1);
        assert!(out.contains(&Action::Local(json!({ "refused": "x" }))));
        assert!(core.config.unwrap().devices.is_empty());
    }

    #[test]
    fn the_code_shows_once_the_relay_has_the_slot_open() {
        let mut core = Core::new(Some(config()));
        let (url, _) = core.begin_pairing(1000).unwrap();
        let slot_now = core.pairing.as_ref().unwrap().slot.clone();
        assert!(core.on_relay(&json!({ "type": "pair_opened", "slot": "other" }), 1000).is_empty());
        assert_eq!(core.on_relay(&json!({ "type": "pair_opened", "slot": slot_now }), 1000), vec![Action::Local(json!({ "qr": url, "expires": 1000 + PAIRING_MS }))]);
        assert!(core.tick(5000).is_empty(), "shown once");
        // An older relay that never says: shown after a moment.
        let mut core = Core::new(Some(config()));
        core.begin_pairing(1000).unwrap();
        assert!(core.tick(2000).is_empty());
        assert!(matches!(&core.tick(4000)[..], [Action::Local(v)] if v["qr"].is_string()));
    }

    #[test]
    fn a_request_without_the_secret_or_after_the_time_is_ignored() {
        let mut core = Core::new(Some(config()));
        let (_, _) = core.begin_pairing(1000).unwrap();
        let c = core.config.clone().unwrap();
        let slot_now = core.pairing.as_ref().unwrap().slot.clone();
        let wrong = seal::seal(Kind::PairRequest, &c.public, None, Some(&[1u8; 32]), &c.id, b"{}").unwrap();
        assert!(core.on_relay(&json!({ "type": "pair_recv", "slot": slot_now, "data": wrong }), 1000).is_empty());
        // Expired: the slot closes and the person is told.
        let out = core.tick(1000 + PAIRING_MS);
        assert!(out.iter().any(|a| matches!(a, Action::Relay(f) if f["type"] == "pair_close")));
        assert!(out.iter().any(|a| matches!(a, Action::Local(v) if v["error"].is_string())));
    }

    #[test]
    fn a_paired_phone_follows_a_squad_and_sends_what_a_phone_may() {
        let (mut core, mut phone, now) = core_with_phone();
        let c = core.config.clone().unwrap();
        // squads, then hello.
        let out = core.on_relay(&phone.send(&c, None, json!({ "jsonrpc": "2.0", "id": 1, "method": "squads" }), now), now);
        assert_eq!(phone.open(&c, &out[0])["rpc"]["result"][0]["name"], "acme");
        let out = core.on_relay(&phone.send(&c, Some("acme"), json!({ "jsonrpc": "2.0", "id": 2, "method": "hello", "params": { "version": "app", "squad": "acme" } }), now), now);
        assert_eq!(out, vec![Action::Open { device: phone.id.clone(), squad: "acme".into(), id: json!(2) }]);
        let answered = core.opened(&phone.id, "acme", &json!(2), Ok(Hello { version: "v".into(), snapshot: Snapshot { squad: "acme".into(), ..Snapshot::default() } }));
        assert_eq!(phone.open(&c, &answered[0])["rpc"]["result"]["snapshot"]["squad"], "acme");
        // "Always" from a phone is once.
        let decide = json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "decide", "squad": "acme", "approval": 3, "decision": "always" } });
        let out = core.on_relay(&phone.send(&c, Some("acme"), decide, now), now);
        assert_eq!(out, vec![Action::Command { device: phone.id.clone(), squad: "acme".into(), command: Command::Decide { squad: "acme".into(), approval: 3, decision: Decision::Allow } }]);
        // Merging stays at the console; another squad than the frame's is refused.
        let merge = json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "merge", "squad": "acme", "ticket": 1, "push": true } });
        assert!(core.on_relay(&phone.send(&c, Some("acme"), merge, now), now).is_empty());
        let other = json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "accept", "squad": "zeta", "ticket": 1 } });
        assert!(core.on_relay(&phone.send(&c, Some("acme"), other, now), now).is_empty());
        // Events go on to it, sealed, counted.
        let out = core.on_event(&phone.id, "acme", &Event::Notice { session: 1, text: "hi".into() });
        let opened = phone.open(&c, &out[0]);
        assert_eq!((opened["squad"].as_str(), opened["rpc"]["method"].as_str()), (Some("acme"), Some("event")));
        assert!(opened["seq"].as_u64().unwrap() >= 3);
    }

    #[test]
    fn replayed_stale_or_foreign_frames_are_dropped() {
        let (mut core, mut phone, now) = core_with_phone();
        let c = core.config.clone().unwrap();
        let ask = json!({ "jsonrpc": "2.0", "id": 1, "method": "squads" });
        let frame = phone.send(&c, None, ask.clone(), now);
        assert_eq!(core.on_relay(&frame, now).len(), 1);
        assert!(core.on_relay(&frame, now).is_empty(), "the same frame again");
        assert!(core.on_relay(&phone.send(&c, None, ask.clone(), now - CLOCK_SKEW_MS - 1), now).is_empty(), "from too long ago");
        let mut stranger = Phone::new();
        stranger.id = phone.id.clone();
        assert!(core.on_relay(&stranger.send(&c, None, ask, now), now).is_empty(), "someone else's key");
    }

    #[test]
    fn a_device_away_gets_a_push_that_names_who_not_what() {
        let (mut core, phone, now) = core_with_phone();
        let c = core.config.clone().unwrap();
        let approval = Approval { id: 9, session: 2, member: "backend".into(), ticket: Some(4), tool: "Bash".into(), action: "rm -rf build".into(), reason: None, preview: None };
        let event = Event::ApprovalRequested { squad: "acme".into(), approval };
        // Connected, it hears it over the socket instead.
        assert!(core.on_squad_event("acme", &event).is_empty());
        core.on_relay(&json!({ "type": "presence", "device": phone.id, "online": false }), now);
        let out = core.on_squad_event("acme", &event);
        let Action::Relay(push) = &out[0] else { panic!() };
        assert_eq!((push["type"].as_str(), push["title"].as_str(), push["body"].as_str()), (Some("push"), Some("backend asks to run a command"), Some("acme · T-4")));
        assert!(!push.to_string().contains("rm -rf"), "the command travels sealed only");
        let opened = phone.open(&c, &out[0]);
        assert_eq!((opened["seq"].as_u64(), opened["rpc"]["params"]["approval"]["action"].as_str()), (Some(0), Some("rm -rf build")));
        // A ticket handed in pushes once.
        let mut t = Ticket { id: 4, title: "x".into(), body: String::new(), status: TicketStatus::Review, owner: Some("backend".into()), assignee: None, kind: equipe_core::protocol::TicketKind::Task, project: None, depends_on: vec![], branch: None, worktree: None, merged: None, created_by: "pm".into(), note: None };
        assert_eq!(core.on_squad_event("acme", &Event::TicketChanged { squad: "acme".into(), ticket: t.clone() }).len(), 1);
        assert!(core.on_squad_event("acme", &Event::TicketChanged { squad: "acme".into(), ticket: t.clone() }).is_empty());
        t.status = TicketStatus::Done;
        assert!(core.on_squad_event("acme", &Event::TicketChanged { squad: "acme".into(), ticket: t }).is_empty());
    }

    #[test]
    fn a_removed_device_is_forgotten_and_dropped() {
        let (mut core, mut phone, now) = core_with_phone();
        let c = core.config.clone().unwrap();
        let out = core.remove_device("Abel's iPhone").unwrap();
        assert!(out.contains(&Action::Save) && out.contains(&Action::Close { device: phone.id.clone() }));
        assert!(out.iter().any(|a| matches!(a, Action::Relay(f) if f["type"] == "device_remove")));
        assert!(core.on_relay(&phone.send(&c, None, json!({ "jsonrpc": "2.0", "id": 1, "method": "squads" }), now), now).is_empty());
        assert!(core.remove_device("nobody").is_err());
    }

    /// The whole way, against a relay running locally (`npx wrangler dev` in
    /// `relay/`) and a daemon with its own home pointed at it:
    /// `EQUIPE_E2E_RELAY=http://localhost:8787 EQUIPE_E2E_HOME=/tmp/eqt cargo test -p equipe e2e -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn e2e_a_phone_pairs_through_the_relay_and_reads_the_squads() {
        use futures_util::{SinkExt as _, StreamExt as _};
        use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
        use tokio_tungstenite::tungstenite::Message as Ws;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
        let relay = std::env::var("EQUIPE_E2E_RELAY").expect("EQUIPE_E2E_RELAY");
        let home = std::path::PathBuf::from(std::env::var("EQUIPE_E2E_HOME").expect("EQUIPE_E2E_HOME"));
        let ws_base = relay.replacen("http://", "ws://", 1).replacen("https://", "wss://", 1);
        // `equipe pair`, by hand.
        let stream = tokio::net::UnixStream::connect(super::super::local_socket(&home)).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(reader).lines();
        writer.write_all(b"{\"op\":\"pair\"}\n").await.unwrap();
        let opened: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        let url = opened["qr"].as_str().unwrap_or_else(|| panic!("{opened}")).to_owned();
        let query: HashMap<String, String> = url.split_once('?').unwrap().1.split('&').map(|kv| {
            let (k, v) = kv.split_once('=').unwrap();
            (k.to_owned(), v.to_owned())
        }).collect();
        let (daemon, daemon_key) = (query["d"].clone(), query["k"].clone());
        let secret = super::super::unb64(&query["s"]).unwrap();
        // The phone, on the pairing slot.
        let phone = Phone::new();
        let (mut slot_ws, _) = tokio_tungstenite::connect_async(format!("{ws_base}/v1/daemons/{daemon}/pair/{}", slot(&secret))).await.unwrap();
        let request = json!({ "type": "pair_request", "name": "e2e phone", "platform": "test", "key": phone.keys.public });
        let data = seal::seal(Kind::PairRequest, &daemon_key, None, Some(&secret), &daemon, request.to_string().as_bytes()).unwrap();
        slot_ws.send(Ws::text(json!({ "type": "send", "data": data }).to_string())).await.unwrap();
        let asked: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(asked["request"]["name"], "e2e phone");
        writer.write_all(b"{\"op\":\"answer\",\"approve\":true}\n").await.unwrap();
        let answer = loop {
            let Some(Ok(Ws::Text(text))) = slot_ws.next().await else { panic!("the slot closed without an answer") };
            let frame: Value = serde_json::from_str(&text).unwrap();
            if frame["type"] == "recv" {
                break frame;
            }
        };
        let answer: Value = serde_json::from_slice(&seal::open(Kind::PairAnswer, &phone.keys, Some(&daemon_key), Some(&secret), &daemon, answer["data"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(answer["type"], "paired");
        let device = answer["device"].as_str().unwrap().to_owned();
        let token = answer["token"].as_str().unwrap().to_owned();
        // As a device: its squads, then one's snapshot.
        let mut request = format!("{ws_base}/v1/daemons/{daemon}/devices/{device}/connect").into_client_request().unwrap();
        request.headers_mut().insert("Authorization", format!("Bearer {token}").parse().unwrap());
        let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let config = Config { relay, id: daemon.clone(), token: String::new(), private: String::new(), public: daemon_key.clone(), devices: vec![] };
        let mut phone = Phone { keys: phone.keys, id: device.clone(), seq: 0 };
        let ask = |phone: &mut Phone, squad: Option<&str>, rpc: Value| {
            let frame = phone.send(&config, squad, rpc, super::super::now_ms());
            Ws::text(json!({ "type": "send", "data": frame["data"] }).to_string())
        };
        ws.send(ask(&mut phone, None, json!({ "jsonrpc": "2.0", "id": 1, "method": "squads" }))).await.unwrap();
        let mut squads = None;
        let mut snapshot = None;
        while let Some(Ok(message)) = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next()).await.unwrap() {
            let Ws::Text(text) = message else { continue };
            let frame: Value = serde_json::from_str(&text).unwrap();
            if frame["type"] != "recv" {
                continue;
            }
            let opened = phone.open(&config, &Action::Relay(frame));
            match opened["rpc"]["id"].as_u64() {
                Some(1) => {
                    let name = opened["rpc"]["result"][0]["name"].as_str().unwrap().to_owned();
                    ws.send(ask(&mut phone, Some(&name), json!({ "jsonrpc": "2.0", "id": 2, "method": "hello", "params": { "version": "e2e", "squad": name } }))).await.unwrap();
                    squads = Some(name);
                }
                Some(2) => {
                    snapshot = Some(opened["rpc"]["result"]["snapshot"]["squad"].as_str().unwrap().to_owned());
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(snapshot, squads);
        // Forgotten, it can't connect again.
        let (mut lines2, mut writer2) = {
            let stream = tokio::net::UnixStream::connect(super::super::local_socket(&home)).await.unwrap();
            let (r, w) = stream.into_split();
            (tokio::io::BufReader::new(r).lines(), w)
        };
        writer2.write_all(b"{\"op\":\"remove\",\"name\":\"e2e phone\"}\n").await.unwrap();
        let removed: Value = serde_json::from_str(&lines2.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(removed["told_relay"], true, "{removed}");
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let mut again = format!("{ws_base}/v1/daemons/{daemon}/devices/{device}/connect").into_client_request().unwrap();
        again.headers_mut().insert("Authorization", format!("Bearer {token}").parse().unwrap());
        assert!(tokio_tungstenite::connect_async(again).await.is_err());
    }
}

