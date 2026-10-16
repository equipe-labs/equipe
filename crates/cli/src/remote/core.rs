//! What the daemon does for paired devices, as plain state: relay frames,
//! squad events and pairing in; frames for the relay, commands for squads and
//! answers for `equipe pair` out. The I/O is `link`'s.

use std::collections::{BTreeMap, HashMap, HashSet};

use equipe_core::protocol::rpc::{self, Hello, Message};
use equipe_core::protocol::{Command, Decision, Event, Snapshot, TicketId, TicketStatus};
use equipe_core::t;
use serde_json::{Value, json};

use super::seal::{self, Kind};
use super::{Config, Device};

/// How long a QR code can be scanned.
pub const PAIRING_MS: u64 = 5 * 60 * 1000;
/// Frames whose clock is further off than this are dropped.
const CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;
/// Of a request's command, this much travels in a push.
const PUSH_ACTION: usize = 800;
/// Of a question, and of an option's label, this much travels in a push; an
/// option's description stays for when the app opens the squad.
const PUSH_QUESTION: usize = 200;
const PUSH_LABEL: usize = 60;
/// A demo daemon's standing slot is opened again this often, each time for
/// the relay's longest (10 minutes).
const STANDING_REFRESH_MS: u64 = 4 * 60 * 1000;
const STANDING_SLOT_MS: u64 = 10 * 60 * 1000;
/// The largest frame the relay takes (`relay/src/frames.ts`, `MAX_FRAME_BYTES`).
pub const RELAY_FRAME_MAX: usize = 1024 * 1024;
/// About the most a message to a phone can say and still fit in one frame:
/// sealed (32 + 16 bytes), base64 (4/3), wrapped with its device and clock.
const RELAY_PLAIN_MAX: usize = (RELAY_FRAME_MAX - 512) / 4 * 3 - 48 - 256;
/// About the most a picture's bytes (base64) may come to and still reach a
/// phone in one frame: past it, a smaller copy is made.
pub const PHONE_IMAGE_MAX: usize = RELAY_PLAIN_MAX - 4096;
/// How many phones a demo daemon keeps; past that the oldest is forgotten.
pub const DEMO_DEVICES: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// A frame for the relay.
    Relay(Value),
    /// Follow `squad` for `device`, and answer its `hello` (`id`) with the snapshot.
    Open { device: String, squad: String, id: Value },
    /// A command from `device` for `squad`.
    Command { device: String, squad: String, command: Command },
    /// Read a member's picture `device` asked for (`id`), of `squad`; the
    /// answer goes back through [`Core::image_read`].
    ReadImage { device: String, squad: String, id: u64, ask: rpc::ReadImage },
    /// The device left: stop following squads for it.
    Close { device: String },
    /// For `equipe pair`, waiting at the terminal.
    Local(Value),
    /// The config changed: keep it.
    Save,
    /// A piece of a file from `device` (`upload`): keep it, then answer `id`
    /// with [`Core::answer`].
    Upload { device: String, id: u64, piece: Box<super::upload::Piece> },
    /// Throw away what came of a file from `device` so far.
    UploadCancel { device: String, upload_id: String },
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

/// A demo daemon's code: open for good, answered without asking anyone.
/// Only `equipe demo serve` opens one; `equipe daemon` never does.
#[derive(Debug)]
struct Standing {
    slot: String,
    secret: [u8; 32],
    /// The name phones are told they paired with.
    name: String,
    /// When `pair_open` was last sent.
    opened: u64,
}

#[derive(Debug, Default)]
pub struct Core {
    pub config: Option<Config>,
    online: HashSet<String>,
    seq_in: HashMap<String, u64>,
    seq_out: HashMap<String, u64>,
    pairing: Option<Pairing>,
    standing: Option<Standing>,
    /// Every squad, with its projects' names — for `squads`.
    pub squads: Vec<(String, Vec<String>)>,
    /// Ticket statuses per squad, to push only on a change.
    tickets: BTreeMap<String, HashMap<TicketId, TicketStatus>>,
    following: HashSet<(String, String)>,
    /// Members' browsers handed to the person, by lane: pushed once each.
    handed: HashSet<String>,
    /// What was too large for the relay, already said in the log.
    too_large: HashSet<String>,
    /// How often the relay said each thing went wrong.
    relay_errors: HashMap<String, u64>,
}

/// 1, 10, 100…
fn tenfold(mut n: u64) -> bool {
    while n >= 10 && n.is_multiple_of(10) {
        n /= 10;
    }
    n == 1
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

/// The QR code's address: where the relay is, which daemon, its key, the
/// pairing secret and the machine's name.
pub fn pairing_url(relay: &str, id: &str, public: &str, secret: &[u8], name: &str) -> String {
    format!("equipe://pair?r={}&d={id}&k={public}&s={}&n={}", encode(relay), super::b64(secret), encode(name))
}

/// The commands a phone may send, as it may send them: from a phone a request
/// is allowed once, never always; a member's questions are the person's to
/// answer or close from anywhere, and so is a change to the team the PM
/// proposed to approve or decline; starting, closing, merging and changing
/// modes stay at the console — and so does writing, editing and rearranging
/// tickets (the phone answers what the board asks; it doesn't plan it yet).
pub fn allowed(command: Command) -> Option<Command> {
    match command {
        Command::Decide { squad, approval, decision: Decision::Always } => Some(Command::Decide { squad, approval, decision: Decision::Allow }),
        c @ (Command::Submit { .. }
        | Command::Interrupt { .. }
        | Command::SendNow { .. }
        | Command::Accept { .. }
        | Command::Reopen { .. }
        | Command::Resume { .. }
        | Command::Cancel { .. }
        | Command::Decide { .. }
        | Command::AnswerQuestions { .. }
        | Command::DismissQuestions { .. }
        | Command::ApproveTeamChange { .. }
        | Command::DeclineTeamChange { .. }
        | Command::Focus { .. }
        | Command::Refresh { .. }
        | Command::Design { .. }) => Some(c),
        _ => None,
    }
}

/// The squad a command names, when it names one.
fn squad_of(command: &Command) -> Option<&str> {
    match command {
        Command::Accept { squad, .. }
        | Command::Merge { squad, .. }
        | Command::KeepOnBranch { squad, .. }
        | Command::Reopen { squad, .. }
        | Command::Resume { squad, .. }
        | Command::Cancel { squad, .. }
        | Command::SetMode { squad, .. }
        | Command::Decide { squad, .. }
        | Command::AnswerQuestions { squad, .. }
        | Command::DismissQuestions { squad, .. }
        | Command::ApproveTeamChange { squad, .. }
        | Command::DeclineTeamChange { squad, .. }
        | Command::Scale { squad, .. }
        | Command::Focus { squad, .. }
        | Command::ShowBrowser { squad, .. }
        | Command::Design { squad } => Some(squad),
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
        self.config.as_ref().is_some_and(|c| !c.devices.is_empty()) || self.pairing.is_some() || self.standing.is_some()
    }

    pub fn has_devices(&self) -> bool {
        self.config.as_ref().is_some_and(|c| !c.devices.is_empty())
    }

    /// Whether a code is open and still works.
    pub fn pairing_under_way(&self, now: u64) -> bool {
        self.pairing.as_ref().is_some_and(|p| p.expires > now)
    }

    /// The paired devices, for `equipe devices` and the screens: their id,
    /// name, platform, when paired, whether connected now, when last seen.
    pub fn device_list(&self) -> Vec<Value> {
        self.config
            .iter()
            .flat_map(|c| c.devices.iter())
            .map(|d| {
                let mut row = json!({ "id": d.id, "name": d.name, "platform": d.platform, "added": d.added, "online": self.online.contains(&d.id) });
                if d.seen > 0 {
                    row["seen"] = json!(d.seen);
                }
                row
            })
            .collect()
    }

    /// `equipe pair`: open a slot; the QR code's address, and what to tell the relay.
    pub fn begin_pairing(&mut self, now: u64) -> Result<(String, Vec<Action>), String> {
        let config = self.config.as_ref().ok_or(t!("remote.not_registered"))?;
        if self.pairing.as_ref().is_some_and(|p| p.expires > now) {
            return Err(t!("remote.pairing_under_way").into());
        }
        let secret = super::random::<32>();
        let slot = slot(&secret);
        let expires = now + PAIRING_MS;
        let url = pairing_url(&config.relay, &config.id, &config.public, &secret, &super::hostname());
        self.pairing = Some(Pairing { slot: slot.clone(), secret, expires, request: None, url: url.clone(), started: now, shown: false });
        Ok((url, vec![Action::Relay(json!({ "type": "pair_open", "slot": slot, "expires": expires }))]))
    }

    /// The person's answer to a pairing request.
    pub fn answer_pairing(&mut self, approve: bool, now: u64) -> Vec<Action> {
        let Some(pairing) = self.pairing.take() else {
            return vec![Action::Local(json!({ "error": t!("remote.nothing_to_answer") }))];
        };
        let (Some(request), Some(config)) = (pairing.request, self.config.as_mut()) else {
            return vec![Action::Local(json!({ "error": t!("remote.no_device_asked") }))];
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
        let name = request.name.clone();
        match admit(config, request, &pairing.secret, &pairing.slot, &super::hostname(), now) {
            Some(mut out) => {
                out.extend([close, Action::Local(json!({ "paired": name }))]);
                out
            }
            None => vec![close, Action::Local(json!({ "error": t!("remote.device_key_failed") }))],
        }
    }

    /// `equipe demo serve`: keep a code open for good, with this secret, and
    /// pair every phone that brings it. Its address, which never changes.
    pub fn open_standing(&mut self, secret: [u8; 32], name: &str) -> Result<String, String> {
        let config = self.config.as_ref().ok_or(t!("remote.not_registered"))?;
        let url = pairing_url(&config.relay, &config.id, &config.public, &secret, name);
        self.standing = Some(Standing { slot: slot(&secret), secret, name: name.to_owned(), opened: 0 });
        Ok(url)
    }

    /// The standing slot, opened again at the relay — on a new connection, or
    /// once it was answered and so closed.
    pub fn reopen_standing(&mut self, now: u64) -> Vec<Action> {
        match self.standing.as_mut() {
            Some(s) => {
                s.opened = now;
                vec![Action::Relay(json!({ "type": "pair_open", "slot": s.slot, "expires": now + STANDING_SLOT_MS }))]
            }
            None => vec![],
        }
    }

    /// The standing code stops working (a new one follows).
    pub fn close_standing(&mut self) -> Vec<Action> {
        match self.standing.take() {
            Some(s) => vec![Action::Relay(json!({ "type": "pair_close", "slot": s.slot }))],
            None => vec![],
        }
    }

    /// A phone at the standing slot: paired at once; the oldest phone goes
    /// past [`DEMO_DEVICES`].
    fn on_standing_request(&mut self, data: &str, now: u64) -> Vec<Action> {
        let (Some(standing), Some(config)) = (self.standing.as_ref(), self.config.as_mut()) else {
            return vec![];
        };
        let Some(request) = open_request(config, &standing.secret, data) else {
            return vec![];
        };
        let Some(mut out) = admit(config, request, &standing.secret, &standing.slot, &standing.name, now) else {
            return vec![];
        };
        while config.devices.len() > DEMO_DEVICES {
            let old = config.devices.remove(0);
            self.online.remove(&old.id);
            out.extend([Action::Relay(json!({ "type": "device_remove", "device": old.id })), Action::Close { device: old.id }]);
        }
        out.extend(self.reopen_standing(now));
        out
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
        if self.standing.as_ref().is_some_and(|s| now >= s.opened + STANDING_REFRESH_MS) {
            return self.reopen_standing(now);
        }
        if self.pairing.as_ref().is_some_and(|p| p.expires <= now) {
            let mut out = self.end_pairing();
            out.push(Action::Local(json!({ "error": t!("remote.code_expired"), "why": "expired" })));
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
        let config = self.config.as_mut().ok_or(t!("remote.no_devices_paired"))?;
        let Some(at) = config.devices.iter().position(|d| d.name == name || d.id == name) else {
            let names: Vec<&str> = config.devices.iter().map(|d| d.name.as_str()).collect();
            return Err(t!("remote.no_device_called", name = name, paired = if names.is_empty() { t!("remote.none").into() } else { names.join(", ") }));
        };
        let device = config.devices.remove(at);
        self.online.remove(&device.id);
        Ok(vec![Action::Save, Action::Relay(json!({ "type": "device_remove", "device": device.id })), Action::Close { device: device.id }])
    }

    /// The phone erased itself: forget it as `equipe devices remove` would. The relay
    /// already has; `device_remove` tells it the daemon has too, known device or not,
    /// so it stops telling.
    fn on_unpaired(&mut self, device: &str) -> Vec<Action> {
        if device.is_empty() {
            return vec![];
        }
        self.following.retain(|(d, _)| d != device);
        match self.device(device) {
            Some(_) => self.remove_device(device).unwrap_or_default(),
            None => vec![Action::Relay(json!({ "type": "device_remove", "device": device }))],
        }
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
        let frame = json!({ "type": "send", "device": device, "data": data });
        // The relay drops a frame past its limit and says only "frame is too
        // large": dropped here instead, said once for what it was.
        let size = frame.to_string().len();
        if size > RELAY_FRAME_MAX {
            let what = rpc.method.clone().unwrap_or_else(|| "an answer".into());
            if self.too_large.insert(what.clone()) {
                equipe_core::daemon_log!("remote: a {what} for a phone came to {} KB, more than the relay carries; not sent", size >> 10);
            }
            return None;
        }
        Some(Action::Relay(frame))
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
                // When it was last there, for the list of devices.
                let seen = self.config.as_mut().and_then(|c| c.devices.iter_mut().find(|d| d.id == device)).map(|d| d.seen = now / 1000).is_some();
                if frame["online"] == true {
                    self.online.insert(device);
                    vec![]
                } else {
                    self.online.remove(&device);
                    self.following.retain(|(d, _)| *d != device);
                    let mut out = vec![Action::Close { device }];
                    if seen {
                        out.push(Action::Save);
                    }
                    out
                }
            }
            "device_unpaired" => self.on_unpaired(frame["device"].as_str().unwrap_or_default()),
            "push_result" if frame["ok"] != true => {
                equipe_core::daemon_log!("remote: push to {} failed: {}", frame["device"], frame["error"]);
                vec![]
            }
            "error" => {
                // Said once, then at every tenfold: a loop shows, without filling the log.
                let message = frame["message"].to_string();
                let n = self.relay_errors.entry(message.clone()).or_insert(0);
                *n += 1;
                if tenfold(*n) {
                    equipe_core::daemon_log!("remote: the relay said {message} ({n}×)");
                }
                vec![]
            }
            _ => vec![],
        }
    }

    fn on_pair_request(&mut self, slot: &str, data: &str, now: u64) -> Vec<Action> {
        if self.standing.as_ref().is_some_and(|s| s.slot == slot) {
            return self.on_standing_request(data, now);
        }
        let (Some(pairing), Some(config)) = (self.pairing.as_mut(), self.config.as_ref()) else {
            return vec![];
        };
        if pairing.slot != slot || pairing.expires <= now || pairing.request.is_some() {
            return vec![];
        }
        let Some(request) = open_request(config, &pairing.secret, data) else {
            return vec![];
        };
        let said = json!({ "request": { "name": request.name, "platform": request.platform } });
        pairing.request = Some(request);
        vec![Action::Local(said)]
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
            // A member's picture: of the squad the frame is about and the phone follows.
            Some(rpc::READ_IMAGE) => {
                let Some(id) = id else { return vec![] };
                let Some(ask) = message.params_as::<rpc::ReadImage>() else {
                    return fail(self, -32602, "read_image needs { squad, path }");
                };
                let followed = self.following.contains(&(device.to_owned(), ask.squad.clone()));
                if squad.as_deref() != Some(ask.squad.as_str()) || !followed {
                    return fail(self, -32002, "say hello to that squad first");
                }
                vec![Action::ReadImage { device: device.to_owned(), squad: ask.squad.clone(), id, ask }]
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
            Some(rpc::UPLOAD) => {
                let Some(id) = id else { return vec![] };
                match message.params_as::<super::upload::Piece>() {
                    Some(piece) => vec![Action::Upload { device: device.to_owned(), id, piece: Box::new(piece) }],
                    None => fail(self, -32602, "upload needs { upload_id, name, kind, total, offset, data }"),
                }
            }
            Some("upload_cancel") => match message.params.as_ref().and_then(|p| p["upload_id"].as_str()) {
                Some(upload_id) if id.is_none() => vec![Action::UploadCancel { device: device.to_owned(), upload_id: upload_id.to_owned() }],
                _ => vec![],
            },
            _ => fail(self, -32601, "not something a device may ask"),
        }
    }

    /// The answer to a device's request, made elsewhere (an `upload`'s).
    pub fn answer(&mut self, device: &str, message: &Message) -> Vec<Action> {
        self.frame_for(device, None, message).into_iter().collect()
    }

    /// The squad a device asked for is followed: answer its `hello`.
    pub fn opened(&mut self, device: &str, squad: &str, id: &Value, hello: Result<Hello, String>) -> Vec<Action> {
        let Some(id) = id.as_u64() else { return vec![] };
        let message = match hello {
            Ok(mut hello) => {
                // A squad's snapshot can outgrow what the relay carries: its
                // conversations go with less of their past until it fits —
                // never not at all, or the phone asks again, and again.
                for keep in [None, Some(50), Some(10), Some(0)] {
                    if let Some(keep) = keep {
                        for session in &mut hello.snapshot.sessions {
                            let cut = session.recent.len().saturating_sub(keep);
                            session.recent.drain(..cut);
                        }
                    }
                    if Message::response(id, &hello).to_line().len() <= RELAY_PLAIN_MAX {
                        break;
                    }
                }
                // A phone learns here that this daemon takes its files, and how big.
                let mut result = serde_json::to_value(&hello).unwrap_or_default();
                result["uploads"] = super::upload::limits();
                Message::response(id, result)
            }
            Err(why) => {
                self.following.remove(&(device.to_owned(), squad.to_owned()));
                Message::failure(id, -32000, why)
            }
        };
        self.frame_for(device, Some(squad), &message).into_iter().collect()
    }

    /// A picture a device asked for (`read_image`), read: back to it.
    pub fn image_read(&mut self, device: &str, squad: &str, id: u64, read: Result<rpc::ImageRead, String>) -> Vec<Action> {
        let message = match read {
            Ok(read) => Message::response(id, read),
            Err(why) => Message::failure(id, -32000, why),
        };
        self.frame_for(device, Some(squad), &message).into_iter().collect()
    }

    /// An event of a squad a device follows: on to the device.
    pub fn on_event(&mut self, device: &str, squad: &str, event: &Event) -> Vec<Action> {
        // Pictures of a browser someone at the Mac watches stay on the Mac.
        if !self.online.contains(device) || matches!(event, Event::BrowserFrame { .. }) {
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
        self.push(squad, event, None)
    }

    /// An event of one device's own squad (a demo phone's): a push to it if
    /// it's away and the event needs the person.
    pub fn on_own_event(&mut self, device: &str, squad: &str, event: &Event) -> Vec<Action> {
        self.push(squad, event, Some(device))
    }

    fn push(&mut self, squad: &str, event: &Event, only: Option<&str>) -> Vec<Action> {
        let (title, body, category, collapse, event) = match event {
            Event::ApprovalRequested { approval, .. } => {
                let member = &approval.member;
                let title = match approval.tool.as_str() {
                    "Bash" => t!("remote.push_asks_run", member = member),
                    "Write" | "Edit" | "MultiEdit" => t!("remote.push_asks_edit", member = member),
                    "WebFetch" => t!("remote.push_asks_fetch", member = member),
                    "Confirm" => t!("remote.push_asks_confirm", member = member),
                    _ => t!("remote.push_asks_permission", member = member),
                };
                let mut short = approval.clone();
                short.action = short.action.chars().take(PUSH_ACTION).collect();
                let ticket = approval.ticket.map(|t| format!(" · T-{t}")).unwrap_or_default();
                (
                    title,
                    format!("{squad}{ticket}"),
                    "approval",
                    format!("approval-{}", approval.id),
                    Event::ApprovalRequested { squad: squad.to_owned(), approval: short },
                )
            }
            Event::QuestionsAsked { questions, .. } => {
                let mut short = questions.clone();
                for q in &mut short.questions {
                    q.question = q.question.chars().take(PUSH_QUESTION).collect();
                    for option in &mut q.options {
                        option.label = option.label.chars().take(PUSH_LABEL).collect();
                        option.description.clear();
                    }
                }
                let ticket = questions.ticket.map(|t| format!(" · T-{t}")).unwrap_or_default();
                (
                    t!("remote.push_decision", member = questions.member, count = questions.questions.len()),
                    format!("{squad}{ticket}"),
                    "decision",
                    format!("questions-{}", questions.id),
                    Event::QuestionsAsked { squad: squad.to_owned(), questions: short },
                )
            }
            Event::TeamChangeProposed { change, .. } => {
                let mut short = change.clone();
                short.reason = short.reason.chars().take(PUSH_ACTION).collect();
                let title = match change.change {
                    equipe_core::protocol::team::Change::Add => t!("remote.push_team_add", member = change.by, role = change.role, count = change.count),
                    equipe_core::protocol::team::Change::Remove => {
                        t!("remote.push_team_remove", member = change.by, name = change.member.clone().unwrap_or_else(|| change.role.clone()))
                    }
                };
                (title, squad.to_owned(), "team", format!("team-{}", change.id), Event::TeamChangeProposed { squad: squad.to_owned(), change: short })
            }
            Event::TicketChanged { ticket, .. } => {
                // A demo phone's board is its own.
                let board = only.map_or_else(|| squad.to_owned(), |device| format!("{squad}/{device}"));
                let before = self.tickets.entry(board).or_default().insert(ticket.id, ticket.status);
                if before == Some(ticket.status) {
                    return vec![];
                }
                let who = ticket.assignee.clone().or(ticket.owner.clone()).unwrap_or_else(|| t!("remote.a_member").into());
                let (title, category) = match ticket.status {
                    TicketStatus::Review => (t!("remote.push_review", who = who), "review"),
                    TicketStatus::Waiting => (t!("remote.push_waiting", who = who), "waiting"),
                    _ => return vec![],
                };
                let mut short = ticket.clone();
                short.body = String::new();
                short.note = short.note.map(|n| n.chars().take(PUSH_ACTION).collect());
                (title, format!("T-{} · {squad}", ticket.id), category, format!("ticket-{}", ticket.id), Event::TicketChanged { squad: squad.to_owned(), ticket: short })
            }
            // A member handed the person its browser (to sign in, a code, a
            // human check): the phone can't drive it — the push says where to.
            Event::BrowserControl { lane, by, why, check, .. } => {
                let key = format!("{}/{lane}", only.unwrap_or_default());
                if by.is_none() {
                    self.handed.remove(&key);
                    return vec![];
                }
                if (why.is_none() && check.is_none()) || !self.handed.insert(key) {
                    return vec![];
                }
                let member = lane.split_once('/').map_or(lane.as_str(), |(_, key)| key);
                let member = member.split_once('@').map_or(member, |(name, _)| name);
                (
                    t!("remote.push_browser", member = member),
                    t!("remote.push_browser_body", squad = squad),
                    "browser",
                    format!("browser-{lane}"),
                    event.clone(),
                )
            }
            _ => return vec![],
        };
        let Some(config) = self.config.as_ref() else { return vec![] };
        let away: Vec<Device> = config.devices.iter().filter(|d| !self.online.contains(&d.id) && only.is_none_or(|o| o == d.id)).cloned().collect();
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

/// The conformance vectors the equipe app is built against.
#[cfg(test)]
#[path = "vectors.rs"]
mod vectors;

/// A pairing request sealed with `secret`, read and made safe to show.
fn open_request(config: &Config, secret: &[u8; 32], data: &str) -> Option<Request> {
    let plain = seal::open(Kind::PairRequest, &config.keys(), None, Some(secret), &config.id, data).ok()?;
    let body = serde_json::from_slice::<Value>(&plain).ok()?;
    let (name, key) = (body["name"].as_str()?, body["key"].as_str()?);
    if body["type"] != "pair_request" || super::unb64(key).map(|k| k.len()) != Ok(32) {
        return None;
    }
    let name: String = name.chars().filter(|c| !c.is_control()).take(60).collect();
    let platform: String = body["platform"].as_str().unwrap_or("unknown").chars().filter(|c| c.is_ascii_alphanumeric()).take(20).collect();
    Some(Request { name, platform, key: key.to_owned() })
}

/// Pair `request`'s device: keep it, tell the relay its token, and answer it
/// on `slot`. `None` if its key didn't work.
fn admit(config: &mut Config, request: Request, secret: &[u8; 32], slot: &str, name: &str, now: u64) -> Option<Vec<Action>> {
    let id = super::hex(&super::random::<16>());
    let token = super::b64(&super::random::<32>());
    let body = json!({ "type": "paired", "device": id, "token": token, "name": name });
    let data = seal::seal(Kind::PairAnswer, &request.key, Some(&config.keys()), Some(secret), &config.id, body.to_string().as_bytes()).ok()?;
    config.devices.retain(|d| d.name != request.name || d.key != request.key);
    config.devices.push(Device { id: id.clone(), name: request.name, platform: request.platform, key: request.key, added: now / 1000, seen: 0 });
    Some(vec![
        Action::Save,
        Action::Relay(json!({ "type": "device_add", "device": id, "token_hash": super::sha256_hex(token.as_bytes()) })),
        Action::Relay(json!({ "type": "pair_send", "slot": slot, "data": data })),
    ])
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

        /// A new socket: its frames count from 1 again.
        pub fn reconnect(&mut self) {
            self.seq = 0;
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
    fn a_snapshot_too_large_for_the_relay_goes_with_less_of_the_past_and_a_huge_event_not_at_all() {
        use equipe_core::protocol::{Access, Engine, Entry, SessionInfo, SessionState};
        let (mut core, phone, _) = core_with_phone();
        let c = core.config.clone().unwrap();
        // Eight conversations of 200 long lines each: megabytes, as a busy squad's.
        let line = "x".repeat(2_000);
        let session = |id| SessionInfo {
            session: id,
            project: "/p".into(),
            member: None,
            engine: Engine::Claude,
            model: None,
            access: Access::ReadOnly,
            workspace: None,
            state: SessionState::Ready,
            title: None,
            tokens: 0,
            context: None,
            recent: (0..200).map(|_| Entry::Reply { text: line.clone() }).collect(),
            queued: vec![],
            queued_project: None,
            queued_in: vec![],
            steering: vec![],
            steering_in: vec![],
            subagents: vec![],
            lane: None,
            estimate: None,
            work: None,
            mode: None,
            effort: None,
        };
        let snapshot = Snapshot { squad: "acme".into(), sessions: (1..=8).map(session).collect(), ..Snapshot::default() };
        assert!(serde_json::to_string(&snapshot).unwrap().len() > 2 * RELAY_FRAME_MAX);
        let answered = core.opened(&phone.id, "acme", &json!(2), Ok(Hello { version: "v".into(), snapshot }));
        let [Action::Relay(frame)] = &answered[..] else { panic!("{answered:?}") };
        assert!(frame.to_string().len() <= RELAY_FRAME_MAX, "fits the relay");
        let got = phone.open(&c, &answered[0]);
        let sessions = got["rpc"]["result"]["snapshot"]["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 8, "every conversation still there");
        let kept = sessions[0]["recent"].as_array().unwrap().len();
        assert!(kept > 0 && kept < 200, "its latest lines: {kept}");
        // An event bigger than any frame is dropped, not sent for the relay to refuse.
        let huge = Event::TextDelta { session: 1, text: "y".repeat(2 * RELAY_FRAME_MAX) };
        assert!(core.on_event(&phone.id, "acme", &huge).is_empty());
        assert_eq!(core.on_event(&phone.id, "acme", &Event::TextDelta { session: 1, text: "ok".into() }).len(), 1);
    }

    #[test]
    fn the_relay_saying_the_same_thing_again_and_again_is_logged_at_each_tenfold() {
        assert_eq!((1..=1000).filter(|n| tenfold(*n)).collect::<Vec<_>>(), [1, 10, 100, 1000]);
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
        // A member's questions are the person's to answer, or close, from a phone too.
        let answer = json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "answer_questions", "squad": "acme", "id": 5, "answers": [{ "picked": [1] }, {}] } });
        let out = core.on_relay(&phone.send(&c, Some("acme"), answer, now), now);
        let answers = vec![equipe_core::protocol::decide::Answer { picked: vec![1], other: None }, Default::default()];
        assert_eq!(out, vec![Action::Command { device: phone.id.clone(), squad: "acme".into(), command: Command::AnswerQuestions { squad: "acme".into(), id: 5, answers, about: None } }]);
        let dismiss = json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "dismiss_questions", "squad": "acme", "id": 5 } });
        assert!(matches!(&core.on_relay(&phone.send(&c, Some("acme"), dismiss, now), now)[..], [Action::Command { command: Command::DismissQuestions { .. }, .. }]));
        let elsewhere = json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "dismiss_questions", "squad": "zeta", "id": 5 } });
        assert!(core.on_relay(&phone.send(&c, Some("acme"), elsewhere, now), now).is_empty(), "only the frame's squad");
        // A change to the team the PM proposed: the person's to approve or decline from a phone too.
        let approve = json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "approve_team_change", "squad": "acme", "id": 6 } });
        let out = core.on_relay(&phone.send(&c, Some("acme"), approve, now), now);
        assert_eq!(out, vec![Action::Command { device: phone.id.clone(), squad: "acme".into(), command: Command::ApproveTeamChange { squad: "acme".into(), id: 6 } }]);
        let decline = json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "decline_team_change", "squad": "acme", "id": 6 } });
        assert!(matches!(&core.on_relay(&phone.send(&c, Some("acme"), decline, now), now)[..], [Action::Command { command: Command::DeclineTeamChange { .. }, .. }]));
        let elsewhere = json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "approve_team_change", "squad": "zeta", "id": 6 } });
        assert!(core.on_relay(&phone.send(&c, Some("acme"), elsewhere, now), now).is_empty(), "only the frame's squad");
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
    fn a_phone_asks_for_a_members_picture_of_the_squad_it_follows_only() {
        use equipe_core::protocol::rpc::{ImageRead, ReadImage};
        let (mut core, mut phone, now) = core_with_phone();
        let c = core.config.clone().unwrap();
        let ask = |id: u64, squad: &str| json!({ "jsonrpc": "2.0", "id": id, "method": "read_image", "params": { "squad": squad, "path": "/h/squads/acme/home/brand/a.png", "max": 4000 } });
        // Not followed yet: refused, with why.
        let out = core.on_relay(&phone.send(&c, Some("acme"), ask(1, "acme"), now), now);
        assert!(phone.open(&c, &out[0])["rpc"]["error"]["message"].as_str().unwrap().contains("hello"));
        core.on_relay(&phone.send(&c, Some("acme"), json!({ "jsonrpc": "2.0", "id": 2, "method": "hello", "params": { "version": "app", "squad": "acme" } }), now), now);
        let out = core.on_relay(&phone.send(&c, Some("acme"), ask(3, "acme"), now), now);
        let wanted = ReadImage { squad: "acme".into(), path: "/h/squads/acme/home/brand/a.png".into(), max: Some(4000) };
        assert_eq!(out, vec![Action::ReadImage { device: phone.id.clone(), squad: "acme".into(), id: 3, ask: wanted }]);
        // Another squad than the frame's.
        let out = core.on_relay(&phone.send(&c, Some("acme"), ask(4, "zeta"), now), now);
        assert!(phone.open(&c, &out[0])["rpc"]["error"].is_object());
        // Read: back to the phone, sealed; or why not.
        let read = ImageRead { mime: "image/png".into(), size: 9, width: Some(2), height: Some(1), data: "AA==".into(), scaled: true };
        let out = core.image_read(&phone.id, "acme", 3, Ok(read));
        assert_eq!(phone.open(&c, &out[0])["rpc"]["result"]["mime"], "image/png");
        let out = core.image_read(&phone.id, "acme", 5, Err("gone".into()));
        assert_eq!(phone.open(&c, &out[0])["rpc"]["error"]["message"], "gone");
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
        let mut t = Ticket { id: 4, title: "x".into(), body: String::new(), status: TicketStatus::Review, owner: Some("backend".into()), assignee: None, kind: equipe_core::protocol::TicketKind::Task, project: None, depends_on: vec![], branch: None, worktree: None, merged: None, off_main: None, created_by: "pm".into(), note: None, asks: vec![], remaining: vec![], images: vec![], updated_at: None, priority: Default::default(), rank: 0 };
        assert_eq!(core.on_squad_event("acme", &Event::TicketChanged { squad: "acme".into(), ticket: t.clone() }).len(), 1);
        assert!(core.on_squad_event("acme", &Event::TicketChanged { squad: "acme".into(), ticket: t.clone() }).is_empty());
        t.status = TicketStatus::Done;
        assert!(core.on_squad_event("acme", &Event::TicketChanged { squad: "acme".into(), ticket: t }).is_empty());
        // A decision to make: who asks and how many — the questions travel sealed, cut short.
        use equipe_core::protocol::decide::{Choice, Question, QuestionSet};
        let long = "왜".repeat(500);
        let option = |label: &str| Choice { label: label.into(), description: "a long why".into(), ..Choice::default() };
        let set = QuestionSet {
            id: 11,
            session: 2,
            member: "backend".into(),
            ticket: Some(4),
            questions: vec![Question { question: long.clone(), header: "DB".into(), options: vec![option("Postgres"), option("SQLite")], multi_select: false, ..Question::default() }; 2],
            asked_at: 1,
        };
        let out = core.on_squad_event("acme", &Event::QuestionsAsked { squad: "acme".into(), questions: set });
        let Action::Relay(push) = &out[0] else { panic!() };
        assert_eq!((push["title"].as_str(), push["body"].as_str(), push["category"].as_str()), (Some("backend asks you to decide 2 things"), Some("acme · T-4"), Some("decision")));
        assert!(!push.to_string().contains("Postgres"));
        let opened = phone.open(&c, &out[0]);
        let questions = &opened["rpc"]["params"]["questions"]["questions"];
        assert_eq!(questions[0]["question"].as_str().unwrap().chars().count(), 200);
        assert_eq!((questions[0]["options"][0]["label"].as_str(), questions[0]["options"][0].get("description")), (Some("Postgres"), None));
        // A member hands the person its browser: who, and that it's the Mac's app to do it in — what for travels sealed.
        let why = "Umbriles Inc. 계정 소유자 Apple ID로 로그인해 주세요";
        let handed = Event::BrowserControl { squad: Some("acme".into()), lane: "acme/pm".into(), by: Some("person".into()), why: Some(why.into()), check: None };
        let out = core.on_squad_event("acme", &handed);
        let Action::Relay(push) = &out[0] else { panic!("{out:?}") };
        assert_eq!((push["title"].as_str(), push["category"].as_str()), (Some("pm is waiting for you in its browser"), Some("browser")));
        assert!(push["body"].as_str().unwrap().contains("equipe app on your Mac") && push["body"].as_str().unwrap().contains("acme"), "{push}");
        assert!(!push.to_string().contains("Umbriles"), "what for travels sealed only");
        assert_eq!(phone.open(&c, &out[0])["rpc"]["params"]["why"].as_str(), Some(why));
        assert!(core.on_squad_event("acme", &handed).is_empty(), "told again (a screen attached): pushed once");
        let taken = Event::BrowserControl { squad: Some("acme".into()), lane: "acme/qa".into(), by: Some("person".into()), why: None, check: None };
        assert!(core.on_squad_event("acme", &taken).is_empty(), "the person taking it themselves is no news");
        let back = Event::BrowserControl { squad: Some("acme".into()), lane: "acme/pm".into(), by: None, why: None, check: None };
        assert!(core.on_squad_event("acme", &back).is_empty());
        let check = Event::BrowserControl { squad: Some("acme".into()), lane: "acme/pm@site".into(), by: Some("person".into()), why: None, check: Some(equipe_core::protocol::HumanCheck::SignIn) };
        assert!(matches!(&core.on_squad_event("acme", &check)[..], [Action::Relay(p)] if p["title"] == "pm is waiting for you in its browser"));
        assert_eq!(core.on_squad_event("acme", &handed).len(), 1, "handed over again after it came back");
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

    #[test]
    fn the_list_says_which_device_is_there_and_when_one_was_last_seen() {
        let (mut core, phone, now) = core_with_phone();
        let list = core.device_list();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0]["id"].as_str(), list[0]["online"].as_bool(), list[0].get("seen")), (Some(phone.id.as_str()), Some(true), Some(&json!(now / 1000))));
        // It leaves: no longer there, its time kept with the devices.
        let later = now + 90_000;
        let out = core.on_relay(&json!({ "type": "presence", "device": phone.id, "online": false }), later);
        assert!(out.contains(&Action::Save) && out.contains(&Action::Close { device: phone.id.clone() }));
        let list = core.device_list();
        assert_eq!((list[0]["online"].as_bool(), list[0]["seen"].as_u64()), (Some(false), Some(later / 1000)));
        assert_eq!(core.config.as_ref().unwrap().devices[0].seen, later / 1000);
        // A stranger's presence keeps nothing.
        assert_eq!(core.on_relay(&json!({ "type": "presence", "device": "nobody", "online": false }), later), vec![Action::Close { device: "nobody".into() }]);
        // A device never seen leaves `seen` out of its row, and out of the file.
        let mut fresh = config();
        fresh.devices.push(Device { id: "x".into(), name: "x".into(), platform: "ios".into(), key: String::new(), added: 1, seen: 0 });
        assert!(Core::new(Some(fresh.clone())).device_list()[0].get("seen").is_none());
        assert!(!serde_json::to_string(&fresh).unwrap().contains("seen"));
    }

    #[test]
    fn a_phone_can_neither_pair_another_nor_forget_one() {
        for command in [Command::PairStart, Command::PairAnswer { approve: true }, Command::PairCancel, Command::Devices, Command::Unpair { device: "x".into() }] {
            assert_eq!(allowed(command.clone()), None, "{command:?} stays at the person's screens");
        }
    }

    #[test]
    fn a_phone_answers_tickets_but_writes_and_rearranges_none() {
        use equipe_core::protocol::{TicketDraft, TicketEdit};
        let squad = || "acme".to_owned();
        for command in [
            Command::CreateTicket { squad: squad(), draft: TicketDraft { title: "x".into(), ..Default::default() } },
            Command::EditTicket { squad: squad(), ticket: 1, changes: TicketEdit { title: Some("y".into()), ..Default::default() } },
            Command::MoveTicket { squad: squad(), ticket: 1, status: Some(TicketStatus::Done), before: None },
            Command::EditTickets { squad: squad(), tickets: vec![1, 2], changes: TicketEdit { title: Some("y".into()), ..Default::default() } },
            Command::CommentTicket { squad: squad(), ticket: 1, text: "z".into() },
            Command::TicketHistory { squad: squad(), ticket: 1 },
        ] {
            assert_eq!(allowed(command.clone()), None, "{command:?} stays at the person's screens for now");
        }
        assert!(allowed(Command::Accept { squad: squad(), ticket: 1 }).is_some());
    }

    #[test]
    fn a_code_under_way_is_known_and_one_run_out_says_so() {
        let mut core = Core::new(Some(config()));
        assert!(!core.pairing_under_way(1000));
        core.begin_pairing(1000).unwrap();
        assert!(core.pairing_under_way(1000) && !core.pairing_under_way(1000 + PAIRING_MS));
        let out = core.tick(1000 + PAIRING_MS);
        assert!(out.iter().any(|a| matches!(a, Action::Local(v) if v["why"] == "expired")), "{out:?}");
        assert!(!core.pairing_under_way(1000));
    }

    #[test]
    fn a_phone_that_unpaired_is_forgotten() {
        let (mut core, mut phone, now) = core_with_phone();
        let c = core.config.clone().unwrap();
        let out = core.on_relay(&json!({ "type": "device_unpaired", "device": phone.id }), now);
        assert!(out.contains(&Action::Save) && out.contains(&Action::Close { device: phone.id.clone() }));
        assert!(out.iter().any(|a| matches!(a, Action::Relay(f) if f["type"] == "device_remove" && f["device"] == phone.id.as_str())));
        assert!(core.config.as_ref().unwrap().devices.is_empty(), "gone from `equipe devices`");
        assert!(core.on_relay(&phone.send(&c, None, json!({ "jsonrpc": "2.0", "id": 1, "method": "squads" }), now), now).is_empty());
        // Told again on the next connection, after the daemon already forgot it: it says so once more.
        let out = core.on_relay(&json!({ "type": "device_unpaired", "device": phone.id }), now);
        assert_eq!(out, vec![Action::Relay(json!({ "type": "device_remove", "device": phone.id }))]);
    }

    /// A pairing request as the app sends it, for the code in `url`.
    fn request_for(url: &str, phone: &Phone, name: &str) -> (String, String) {
        let query: HashMap<String, String> = url.split_once('?').unwrap().1.split('&').map(|kv| {
            let (k, v) = kv.split_once('=').unwrap();
            (k.to_owned(), v.to_owned())
        }).collect();
        let secret = super::super::unb64(&query["s"]).unwrap();
        let request = json!({ "type": "pair_request", "name": name, "platform": "ios", "key": phone.keys.public });
        (slot(&secret), seal::seal(Kind::PairRequest, &query["k"], None, Some(&secret), &query["d"], request.to_string().as_bytes()).unwrap())
    }

    #[test]
    fn a_pairing_code_still_lasts_five_minutes_serves_one_phone_and_waits_for_the_persons_yes() {
        let mut core = Core::new(Some(config()));
        let (url, opened) = core.begin_pairing(1000).unwrap();
        assert!(matches!(&opened[..], [Action::Relay(f)] if f["expires"] == 1000 + PAIRING_MS));
        assert_eq!(PAIRING_MS, 5 * 60 * 1000);
        assert!(!url.contains("equipe%20demo"));
        let (slot_a, first) = request_for(&url, &Phone::new(), "first");
        // A request only asks the person: nothing is kept, nothing told to the relay.
        let asked = core.on_relay(&json!({ "type": "pair_recv", "slot": slot_a, "data": first }), 1000);
        assert_eq!(asked, vec![Action::Local(json!({ "request": { "name": "first", "platform": "ios" } }))]);
        assert!(core.config.as_ref().unwrap().devices.is_empty());
        // A second phone with the same code, before or after the answer, gets nothing.
        let (_, second) = request_for(&url, &Phone::new(), "second");
        assert!(core.on_relay(&json!({ "type": "pair_recv", "slot": slot_a, "data": second }), 1000).is_empty());
        let done = core.answer_pairing(true, 1000);
        assert!(done.iter().any(|a| matches!(a, Action::Relay(f) if f["type"] == "pair_close")), "the slot closes once answered");
        assert!(!done.iter().any(|a| matches!(a, Action::Relay(f) if f["type"] == "pair_open")), "and isn't opened again");
        assert!(core.on_relay(&json!({ "type": "pair_recv", "slot": slot_a, "data": second }), 2000).is_empty());
        assert_eq!(core.config.as_ref().unwrap().devices.len(), 1);
        // Past its time, a new code's request is ignored even before the minute hand closes it.
        let (url, _) = core.begin_pairing(10_000).unwrap();
        let (slot_b, late) = request_for(&url, &Phone::new(), "late");
        assert!(core.on_relay(&json!({ "type": "pair_recv", "slot": slot_b, "data": late }), 10_000 + PAIRING_MS).is_empty());
        // `equipe daemon` has no standing code: nothing keeps a slot open on its own.
        let mut core = Core::new(Some(config()));
        assert!(core.tick(10 * STANDING_REFRESH_MS).is_empty());
        assert!(core.reopen_standing(1).is_empty());
        assert!(!core.wants_relay());
    }

    #[test]
    fn a_demo_code_pairs_every_phone_at_once_and_stays_open() {
        let mut core = Core::new(Some(config()));
        let url = core.open_standing([7; 32], "equipe demo").unwrap();
        assert!(url.ends_with("&n=equipe%20demo"));
        assert_eq!(core.open_standing([7; 32], "equipe demo").unwrap(), url, "the same code every time");
        assert!(core.wants_relay());
        let opened = core.reopen_standing(1000);
        assert!(matches!(&opened[..], [Action::Relay(f)] if f["type"] == "pair_open" && f["expires"] == 1000 + STANDING_SLOT_MS));
        assert!(core.tick(1000 + STANDING_REFRESH_MS - 1).is_empty());
        assert!(matches!(&core.tick(1000 + STANDING_REFRESH_MS)[..], [Action::Relay(f)] if f["type"] == "pair_open"), "opened again before the relay closes it");
        let c = core.config.clone().unwrap();
        for (i, name) in ["reviewer 1", "reviewer 2"].iter().enumerate() {
            let phone = Phone::new();
            let (slot_now, data) = request_for(&url, &phone, name);
            let out = core.on_relay(&json!({ "type": "pair_recv", "slot": slot_now, "data": data }), 5000);
            assert!(out.contains(&Action::Save));
            assert!(out.iter().any(|a| matches!(a, Action::Relay(f) if f["type"] == "device_add")));
            assert!(matches!(out.last(), Some(Action::Relay(f)) if f["type"] == "pair_open"), "open again for the next phone");
            let send = out.iter().find_map(|a| match a { Action::Relay(f) if f["type"] == "pair_send" => Some(f.clone()), _ => None }).unwrap();
            let secret = [7u8; 32];
            let answer: Value = serde_json::from_slice(&seal::open(Kind::PairAnswer, &phone.keys, Some(&c.public), Some(&secret), &c.id, send["data"].as_str().unwrap()).unwrap()).unwrap();
            assert_eq!((answer["type"].as_str(), answer["name"].as_str()), (Some("paired"), Some("equipe demo")));
            assert_eq!(core.config.as_ref().unwrap().devices.len(), i + 1);
        }
        // Without the code, nothing.
        let wrong = seal::seal(Kind::PairRequest, &c.public, None, Some(&[1u8; 32]), &c.id, b"{}").unwrap();
        assert!(core.on_relay(&json!({ "type": "pair_recv", "slot": slot(&[7; 32]), "data": wrong }), 5000).is_empty());
        // A new code: the old slot closes.
        assert!(matches!(&core.close_standing()[..], [Action::Relay(f)] if f["type"] == "pair_close" && f["slot"] == slot(&[7; 32])));
    }

    #[test]
    fn a_demo_keeps_so_many_phones_and_pushes_each_only_its_own() {
        let mut core = Core::new(Some(config()));
        let url = core.open_standing([9; 32], "equipe demo").unwrap();
        let mut ids: Vec<String> = Vec::new();
        for i in 0..=DEMO_DEVICES {
            let (slot_now, data) = request_for(&url, &Phone::new(), &format!("phone {i}"));
            let out = core.on_relay(&json!({ "type": "pair_recv", "slot": slot_now, "data": data }), 1000);
            if i == DEMO_DEVICES {
                assert!(out.iter().any(|a| matches!(a, Action::Relay(f) if f["type"] == "device_remove" && f["device"] == ids[0].as_str())), "the oldest goes");
            }
            ids.push(core.config.as_ref().unwrap().devices.last().unwrap().id.clone());
        }
        assert_eq!(core.config.as_ref().unwrap().devices.len(), DEMO_DEVICES);
        let approval = Approval { id: 1, session: 2, member: "frontend".into(), ticket: Some(3), tool: "Bash".into(), action: "npm run build".into(), reason: None, preview: None };
        let out = core.on_own_event(&ids[5], "demo", &Event::ApprovalRequested { squad: "demo".into(), approval });
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], Action::Relay(f) if f["type"] == "push" && f["device"] == ids[5].as_str()));
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

