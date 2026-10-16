//! The conformance vectors the equipe app is built against: frames sealed with
//! fixed keys and fixed ephemeral keys, the QR code, pairing, `seq`, and the
//! JSON the daemon actually sends — all made here, by the daemon's own code,
//! and checked against it. Kept in `docs/remote-vectors/`; the app keeps a copy
//! in its `docs/vectors/`.
//!
//! `cargo test -p equipe vectors` fails when what is kept differs from what the
//! daemon makes now; `EQUIPE_WRITE_VECTORS=1 cargo test -p equipe vectors`
//! writes them again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use equipe_core::protocol::decide::{Answer, Choice, Question, QuestionSet};
use equipe_core::protocol::rpc::{Hello, Message};
use equipe_core::protocol::team::{Change, TeamChange};
use equipe_core::protocol::{
    Access, Approval, Decision, Engine, EnginePlan, Entry, Event, Member, Mode, PlanUsage, PlanWindow, SessionInfo, SessionState, Snapshot, SubagentInfo, Ticket, TicketKind, TicketStatus, TurnOutcome, Usage, WorkInfo,
};
use serde_json::{Value, json};

use super::super::{Config, Device, b64, sha256_hex, unb64};
use super::seal::{self, Keys, Kind};
use super::{Action, Core, Pairing, Request, slot};

/// The daemon's and the phone's clock in every vector, Unix ms.
const NOW: u64 = 1_760_000_000_000;
const RELAY: &str = "https://relay.equipe.im";
const MACHINE: &str = "abel-macmini";
const PHONE: &str = "Abel's iPhone";
const SQUAD: &str = "acme";
/// Frames the phones' libraries sealed for the daemon (device-to-daemon, the
/// vectors' keys and ids) — made once with the checks in equipe-app's
/// `docs/vectors/check/`, and opened here.
const SEALED_ELSEWHERE: [(&str, &str, &str); 2] = [
    ("CryptoKit (macOS 26)", "from cryptokit", "5jSEmf4kK8fQmEqiXY92Xhf4PMkS2n5Q-BXwN8difRoVPJBlChSDGGHIpSNOQYbuStI94SNa-ccswevghzo"),
    ("BouncyCastle 1.80", "from bouncycastle", "guzeA9K2_bVuxAoHfz-dKI3RouyB8-EEcAcJMBluVmF8txX52vVs9f5wmLEQGpd9mXwRLvwCbD2bge0MSUQfqMQ"),
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/remote-vectors")
}

/// 32 bytes named by `label`: SHA-256 of it.
fn bytes(label: &str) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(label.as_bytes()).into()
}

fn keys(label: &str) -> (String, Keys) {
    let seed = bytes(&format!("equipe vectors · {label} key"));
    (b64(&seed), seal::derive(&seed))
}

fn ikm(label: &str) -> [u8; 32] {
    bytes(&format!("equipe vectors · ephemeral · {label}"))
}

fn daemon_id() -> String {
    sha256_hex(b"equipe vectors \xc2\xb7 daemon id")[..32].to_owned()
}

fn device_id() -> String {
    sha256_hex(b"equipe vectors \xc2\xb7 device id")[..32].to_owned()
}

fn secret() -> [u8; 32] {
    bytes("equipe vectors · pairing secret")
}

fn token() -> String {
    b64(&bytes("equipe vectors · device token"))
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::PairRequest => "pair_request",
        Kind::PairAnswer => "pair_answer",
        Kind::ToDaemon => "device_to_daemon",
        Kind::ToDevice => "daemon_to_device",
    }
}

fn kind_of(name: &str) -> Kind {
    match name {
        "pair_request" => Kind::PairRequest,
        "pair_answer" => Kind::PairAnswer,
        "device_to_daemon" => Kind::ToDaemon,
        "daemon_to_device" => Kind::ToDevice,
        other => panic!("no kind {other}"),
    }
}

fn info(kind: Kind) -> &'static str {
    match kind {
        Kind::PairRequest => "equipe/1 pair-request",
        Kind::PairAnswer => "equipe/1 pair-answer",
        Kind::ToDaemon => "equipe/1 device-to-daemon",
        Kind::ToDevice => "equipe/1 daemon-to-device",
    }
}

/// The three parties' keys, by name.
fn all_keys() -> Vec<(&'static str, String, Keys)> {
    ["daemon", "device", "stranger"].into_iter().map(|name| {
        let (seed, keys) = keys(name);
        (name, seed, keys)
    }).collect()
}

fn key(name: &str) -> Keys {
    keys(name).1
}

/// A frame sealed with a fixed ephemeral key, and how to open it.
struct Sealed {
    name: String,
    kind: Kind,
    recipient: &'static str,
    sender: Option<&'static str>,
    psk: bool,
    aad: String,
    plaintext: String,
    ikm: [u8; 32],
    sealed: String,
}

fn seal_fixed(name: &str, kind: Kind, recipient: &'static str, sender: Option<&'static str>, psk: bool, aad: &str, plaintext: &str) -> Sealed {
    let ikm = ikm(name);
    let secret = secret();
    let sealed = seal::seal_with_ephemeral(kind, &key(recipient).public, sender.map(key).as_ref(), psk.then_some(&secret[..]), aad, plaintext.as_bytes(), &ikm).unwrap();
    // The daemon opens what it sealed.
    let opened = seal::open(kind, &key(recipient), sender.map(|s| key(s).public).as_deref(), psk.then_some(&secret[..]), aad, &sealed).unwrap();
    assert_eq!(opened, plaintext.as_bytes());
    Sealed { name: name.to_owned(), kind, recipient, sender, psk, aad: aad.to_owned(), plaintext: plaintext.to_owned(), ikm, sealed }
}

impl Sealed {
    fn to_json(&self) -> Value {
        let eph = seal::derive(&self.ikm);
        let frame = unb64(&self.sealed).unwrap();
        json!({
            "name": self.name,
            "kind": kind_name(self.kind),
            "mode": match (self.sender.is_some(), self.psk) { (false, false) => "base", (false, true) => "psk", (true, false) => "auth", (true, true) => "auth_psk" },
            "info": info(self.kind),
            "recipient": self.recipient,
            "sender": self.sender,
            "psk": self.psk,
            "aad": self.aad,
            "plaintext": self.plaintext,
            "ikm_e": b64(&self.ikm),
            "sk_e": eph.private,
            "pk_e": eph.public,
            "enc": b64(&frame[..32]),
            "ciphertext": b64(&frame[32..]),
            "sealed": self.sealed,
        })
    }
}

/// Opening `sealed` as `open` says, with the daemon's code.
fn open_as(open: &Value, sealed: &str) -> Result<Vec<u8>, String> {
    let psk = match &open["psk"] {
        Value::String(s) => Some(unb64(s).unwrap()),
        _ => None,
    };
    let sender = open["sender"].as_str().map(|s| key(s).public);
    seal::open(kind_of(open["kind"].as_str().unwrap()), &key(open["recipient"].as_str().unwrap()), sender.as_deref(), psk.as_deref(), open["aad"].as_str().unwrap(), sealed)
}

fn flip(sealed: &str, at: usize) -> String {
    let mut frame = unb64(sealed).unwrap();
    frame[at] ^= 0x01;
    b64(&frame)
}

// ─── seal.json ───────────────────────────────────────────────────────────

fn seal_vectors() -> Value {
    let d = daemon_id();
    let pair = d.clone();
    let link = format!("{d}:{}", device_id());
    let cases = vec![
        seal_fixed("pair-request", Kind::PairRequest, "daemon", None, true, &pair, &pair_request_plaintext()),
        seal_fixed("pair-answer", Kind::PairAnswer, "device", Some("daemon"), true, &pair, &paired_plaintext()),
        seal_fixed("device-to-daemon", Kind::ToDaemon, "daemon", Some("device"), false, &link, r#"{"rpc":{"id":1,"jsonrpc":"2.0","method":"squads"},"seq":1,"ts":1760000000000}"#),
        seal_fixed("daemon-to-device", Kind::ToDevice, "device", Some("daemon"), false, &link, r#"{"rpc":{"id":1,"jsonrpc":"2.0","result":[{"name":"acme","projects":["site","api"]}]},"seq":1,"ts":1760000000000}"#),
        seal_fixed("utf8-plaintext", Kind::ToDaemon, "daemon", Some("device"), false, &link, r#"{"rpc":{"jsonrpc":"2.0","method":"command","params":{"session":4242,"text":"안녕 — 배포 전에 확인해 줘 ✅","type":"submit"}},"seq":2,"squad":"acme","ts":1760000000000}"#),
    ];
    // Sealed by the stranger, passing for the device.
    let forged = seal_fixed("forged", Kind::ToDaemon, "daemon", Some("stranger"), false, &link, &cases[2].plaintext);
    let wrong_secret = b64(&bytes("equipe vectors · another secret"));
    let secret = b64(&secret());
    let open_of = |c: &Sealed| {
        json!({ "kind": kind_name(c.kind), "recipient": c.recipient, "sender": c.sender, "psk": if c.psk { json!(secret) } else { Value::Null }, "aad": c.aad })
    };
    let with = |c: &Sealed, change: Value| {
        let mut open = open_of(c);
        for (k, v) in change.as_object().unwrap() {
            open[k] = v.clone();
        }
        open
    };
    let [request, answer, to_daemon, to_device, _] = &cases[..] else { unreachable!() };
    let failures = vec![
        ("pair-request: another pairing secret", with(request, json!({ "psk": wrong_secret })), request.sealed.clone()),
        ("pair-request: opened without the pairing secret", with(request, json!({ "psk": null })), request.sealed.clone()),
        ("pair-request: opened as a pair answer", with(request, json!({ "kind": "pair_answer" })), request.sealed.clone()),
        ("pair-answer: another pairing secret", with(answer, json!({ "psk": wrong_secret })), answer.sealed.clone()),
        ("pair-answer: someone else than the daemon sealed it", with(answer, json!({ "sender": "stranger" })), answer.sealed.clone()),
        ("device-to-daemon: another sender's key expected", with(to_daemon, json!({ "sender": "stranger" })), to_daemon.sealed.clone()),
        ("device-to-daemon: opened without a sender (base mode)", with(to_daemon, json!({ "sender": null })), to_daemon.sealed.clone()),
        ("device-to-daemon: not the recipient", with(to_daemon, json!({ "recipient": "stranger" })), to_daemon.sealed.clone()),
        ("device-to-daemon: opened as daemon-to-device", with(to_daemon, json!({ "kind": "daemon_to_device" })), to_daemon.sealed.clone()),
        ("device-to-daemon: another device's aad", with(to_daemon, json!({ "aad": format!("{d}:{}", "0".repeat(32)) })), to_daemon.sealed.clone()),
        ("device-to-daemon: a stranger passing for the device", open_of(to_daemon), forged.sealed.clone()),
        ("device-to-daemon: a ciphertext byte flipped", open_of(to_daemon), flip(&to_daemon.sealed, 40)),
        ("device-to-daemon: the tag's last byte flipped", open_of(to_daemon), flip(&to_daemon.sealed, unb64(&to_daemon.sealed).unwrap().len() - 1)),
        ("device-to-daemon: an enc byte flipped", open_of(to_daemon), flip(&to_daemon.sealed, 5)),
        ("device-to-daemon: cut short (no tag)", open_of(to_daemon), b64(&unb64(&to_daemon.sealed).unwrap()[..40])),
        ("device-to-daemon: shorter than enc", open_of(to_daemon), b64(&unb64(&to_daemon.sealed).unwrap()[..20])),
        ("device-to-daemon: not base64url", open_of(to_daemon), "not base64url!".to_owned()),
        ("daemon-to-device: aad with the ids swapped", with(to_device, json!({ "aad": format!("{}:{d}", device_id()) })), to_device.sealed.clone()),
    ];
    let failures: Vec<Value> = failures
        .into_iter()
        .map(|(name, open, sealed)| {
            assert!(open_as(&open, &sealed).is_err(), "{name} opened");
            json!({ "name": name, "open": open, "sealed": sealed })
        })
        .collect();
    for c in &cases {
        assert_eq!(open_as(&open_of(c), &c.sealed).unwrap(), c.plaintext.as_bytes());
    }
    let elsewhere: Vec<Value> = SEALED_ELSEWHERE
        .iter()
        .map(|(by, plaintext, sealed)| {
            let open = with(to_daemon, json!({}));
            assert_eq!(open_as(&open, sealed).unwrap(), plaintext.as_bytes(), "{by}");
            json!({ "sealed_by": by, "open": open, "plaintext": plaintext, "sealed": sealed })
        })
        .collect();
    json!({
        "about": "HPKE (RFC 9180) single-shot frames: DHKEM(X25519, HKDF-SHA256) 0x0020, HKDF-SHA256 0x0001, ChaCha20Poly1305 0x0003. sealed = b64url(enc || ciphertext). Every case opens to plaintext with the recipient's private key and the sender's public key; sealing with the ephemeral key pair derived from ikm_e (DeriveKeyPair) gives exactly enc, ciphertext and sealed. Every failure must fail to open. sealed_elsewhere: frames other libraries sealed, which the daemon opens.",
        "suite": { "kem": 32, "kdf": 1, "aead": 3 },
        "psk_id": "equipe-pair",
        "keys": all_keys().into_iter().map(|(name, seed, k)| (name.to_owned(), json!({ "ikm": seed, "private": k.private, "public": k.public }))).collect::<serde_json::Map<_, _>>(),
        "pairing_secret": secret,
        "daemon_id": d,
        "device_id": device_id(),
        "cases": cases.iter().map(Sealed::to_json).collect::<Vec<_>>(),
        "failures": failures,
        "sealed_elsewhere": elsewhere,
    })
}

// ─── qr.json ─────────────────────────────────────────────────────────────

/// What the app takes from a QR code.
#[derive(Debug, PartialEq)]
struct Qr {
    relay: String,
    daemon: String,
    key: Vec<u8>,
    secret: Vec<u8>,
    machine: String,
}

fn percent_decode(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3).ok_or("a cut % escape")?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| "a bad % escape")?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "not UTF-8".to_owned())
}

/// The rules the app reads a QR code by — the reference the vectors are checked with.
fn parse_qr(uri: &str) -> Result<Qr, String> {
    let query = uri.strip_prefix("equipe://pair?").ok_or("not equipe://pair?")?;
    let mut fields: HashMap<String, String> = HashMap::new();
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').ok_or("a parameter without =")?;
        if fields.insert(k.to_owned(), percent_decode(v)?).is_some() {
            return Err(format!("{k} twice"));
        }
    }
    let field = |k: &str| fields.get(k).cloned().ok_or(format!("no {k}"));
    let relay = field("r")?;
    let local = ["http://localhost", "http://127.0.0.1"].iter().any(|p| relay.strip_prefix(p).is_some_and(|rest| rest.is_empty() || rest.starts_with(':') || rest.starts_with('/')));
    if !(relay.starts_with("https://") && relay.len() > "https://".len() || local) {
        return Err("the relay isn't https".into());
    }
    let daemon = field("d")?;
    if daemon.len() != 32 || !daemon.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err("d isn't 32 lowercase hex".into());
    }
    let b64_32 = |k: &str| -> Result<Vec<u8>, String> {
        let text = field(k)?;
        if !text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(format!("{k} isn't unpadded base64url"));
        }
        let bytes = unb64(&text)?;
        if bytes.len() != 32 {
            return Err(format!("{k} isn't 32 bytes"));
        }
        Ok(bytes)
    };
    let key = b64_32("k")?;
    let secret = b64_32("s")?;
    let machine = field("n")?;
    Ok(Qr { relay, daemon, key, secret, machine })
}

fn qr_vectors() -> Value {
    let daemon = key("daemon");
    let d = daemon_id();
    let s = secret();
    let valid = [
        ("the daemon's code", RELAY, MACHINE),
        ("a name with spaces, an apostrophe and Hangul", RELAY, "Abel의 Mac mini's"),
        ("a relay on this machine, for development", "http://localhost:8787", MACHINE),
    ];
    let valid: Vec<Value> = valid
        .into_iter()
        .map(|(name, relay, machine)| {
            let uri = super::pairing_url(relay, &d, &daemon.public, &s, machine);
            let qr = parse_qr(&uri).unwrap();
            assert_eq!(qr, Qr { relay: relay.into(), daemon: d.clone(), key: unb64(&daemon.public).unwrap(), secret: s.to_vec(), machine: machine.into() });
            json!({ "name": name, "uri": uri, "relay": relay, "daemon": d, "key": daemon.public, "secret": b64(&s), "machine": machine, "slot": slot(&s) })
        })
        .collect();
    let good = super::pairing_url(RELAY, &d, &daemon.public, &s, MACHINE);
    let swap = |from: &str, to: &str| {
        assert!(good.contains(from), "{from}");
        good.replacen(from, to, 1)
    };
    let drop = |param: &str| {
        let (head, query) = good.split_once('?').unwrap();
        format!("{head}?{}", query.split('&').filter(|kv| !kv.starts_with(&format!("{param}="))).collect::<Vec<_>>().join("&"))
    };
    let k = &daemon.public;
    let sb = b64(&s);
    let invalid = vec![
        ("another scheme", swap("equipe://", "https://")),
        ("another host", swap("equipe://pair?", "equipe://pairing?")),
        ("no relay", drop("r")),
        ("no daemon id", drop("d")),
        ("no key", drop("k")),
        ("no secret", drop("s")),
        ("no machine name", drop("n")),
        ("the relay over plain http", swap("https%3A%2F%2F", "http%3A%2F%2F")),
        ("a daemon id in uppercase", swap(&format!("d={d}"), &format!("d={}", d.to_uppercase()))),
        ("a daemon id too short", swap(&format!("d={d}"), &format!("d={}", &d[..30]))),
        ("a key of 31 bytes", swap(&format!("k={k}"), &format!("k={}", b64(&unb64(k).unwrap()[..31])))),
        ("a secret of 33 bytes", swap(&format!("s={sb}"), &format!("s={}", b64(&[&s[..], &[0u8]].concat())))),
        ("a secret in standard base64", swap(&format!("s={sb}"), &format!("s={}", sb.replacen(|c: char| c.is_ascii_alphanumeric(), "+", 1)))),
        ("the daemon id twice", format!("{good}&d={d}")),
        ("a broken % escape", swap(&format!("n={MACHINE}"), "n=abel%2")),
    ];
    let invalid: Vec<Value> = invalid
        .into_iter()
        .map(|(name, uri)| {
            let why = parse_qr(&uri).expect_err(name);
            json!({ "name": name, "uri": uri, "why": why })
        })
        .collect();
    json!({
        "about": "equipe://pair?r=&d=&k=&s=&n= — every parameter required, each once, in any order; unknown ones ignored. Values are percent-decoded (UTF-8); the daemon escapes every byte but A-Z a-z 0-9 - _ . ~, and never writes +. r: https://, or http:// only to localhost / 127.0.0.1 (development). d: 32 lowercase hex. k, s: unpadded base64url of exactly 32 bytes. slot = first 32 hex chars of SHA-256(secret).",
        "valid": valid,
        "invalid": invalid,
    })
}

// ─── pairing.json ────────────────────────────────────────────────────────

fn pair_request_plaintext() -> String {
    json!({ "type": "pair_request", "name": PHONE, "platform": "ios", "key": key("device").public }).to_string()
}

/// The answer `answer_pairing` seals, with the vectors' device id, token and name.
fn paired_plaintext() -> String {
    json!({ "type": "paired", "device": device_id(), "token": token(), "name": MACHINE }).to_string()
}

/// A daemon registered with the relay, as the vectors have it.
fn config(devices: Vec<Device>) -> Config {
    let k = key("daemon");
    Config { relay: RELAY.into(), id: daemon_id(), token: "daemon-token".into(), private: k.private, public: k.public, devices }
}

fn device() -> Device {
    Device { id: device_id(), name: PHONE.into(), platform: "ios".into(), key: key("device").public, added: NOW / 1000, seen: 0 }
}

/// A core with a pairing under way on the vectors' secret.
fn pairing_core() -> Core {
    let mut core = Core::new(Some(config(vec![])));
    let s = secret();
    let c = core.config.clone().unwrap();
    core.pairing = Some(Pairing {
        slot: slot(&s),
        secret: s,
        expires: NOW + super::PAIRING_MS,
        request: None,
        url: super::pairing_url(RELAY, &c.id, &c.public, &s, MACHINE),
        started: NOW,
        shown: true,
    });
    core
}

fn relay_frames(actions: &[Action]) -> Vec<Value> {
    actions.iter().filter_map(|a| match a { Action::Relay(f) => Some(f.clone()), _ => None }).collect()
}

fn pairing_vectors() -> Value {
    let d = daemon_id();
    let s = secret();
    let request = seal_fixed("pair-request", Kind::PairRequest, "daemon", None, true, &d, &pair_request_plaintext());
    let paired = seal_fixed("pair-answer", Kind::PairAnswer, "device", Some("daemon"), true, &d, &paired_plaintext());
    let refused = seal_fixed("pair-refused", Kind::PairAnswer, "device", Some("daemon"), true, &d, &json!({ "type": "refused" }).to_string());
    let slot = slot(&s);
    let recv = json!({ "type": "pair_recv", "slot": slot, "data": request.sealed });

    // The daemon takes the request and asks the person.
    let mut core = pairing_core();
    let asked = core.on_relay(&recv, NOW);
    assert_eq!(asked, vec![Action::Local(json!({ "request": { "name": PHONE, "platform": "ios" } }))]);
    assert_eq!(core.pairing.as_ref().unwrap().request, Some(Request { name: PHONE.into(), platform: "ios".into(), key: key("device").public }));
    // Yes: what it sends the relay, with its random device id and token in
    // place of the vectors'.
    let said = core.answer_pairing(true, NOW);
    let sent = relay_frames(&said);
    let answer = sent.iter().find(|f| f["type"] == "pair_send").unwrap();
    let mut opened: Value = serde_json::from_slice(&seal::open(Kind::PairAnswer, &key("device"), Some(&d_public()), Some(&s), &d, answer["data"].as_str().unwrap()).unwrap()).unwrap();
    let (real_device, real_token) = (opened["device"].as_str().unwrap().to_owned(), opened["token"].as_str().unwrap().to_owned());
    assert_eq!(sent[0], json!({ "type": "device_add", "device": real_device, "token_hash": sha256_hex(real_token.as_bytes()) }));
    assert_eq!(sent[2], json!({ "type": "pair_close", "slot": slot }));
    assert_eq!(sent.len(), 3);
    opened["device"] = json!(device_id());
    opened["token"] = json!(token());
    opened["name"] = json!(MACHINE);
    assert_eq!(opened.to_string(), paired.plaintext, "the answer's shape");
    let added = json!({ "type": "device_add", "device": device_id(), "token_hash": sha256_hex(token().as_bytes()) });
    // No: refused, sealed the same way.
    let mut core = pairing_core();
    core.on_relay(&recv, NOW);
    let said_no = relay_frames(&core.answer_pairing(false, NOW));
    let no = seal::open(Kind::PairAnswer, &key("device"), Some(&d_public()), Some(&s), &d, said_no[0]["data"].as_str().unwrap()).unwrap();
    assert_eq!(no, refused.plaintext.as_bytes());
    assert_eq!(said_no[1], json!({ "type": "pair_close", "slot": slot }));
    // A request without the secret, or for another slot, is ignored.
    let mut core = pairing_core();
    let wrong = seal::seal_with_ephemeral(Kind::PairRequest, &d_public(), None, Some(&bytes("x")), &d, pair_request_plaintext().as_bytes(), &ikm("wrong")).unwrap();
    assert!(core.on_relay(&json!({ "type": "pair_recv", "slot": slot, "data": wrong }), NOW).is_empty());
    assert!(core.on_relay(&json!({ "type": "pair_recv", "slot": "0".repeat(32), "data": request.sealed }), NOW).is_empty());

    let base = RELAY.replacen("https://", "wss://", 1);
    json!({
        "about": "One pairing, start to end, between the vectors' daemon and device (keys in seal.json). The daemon makes the device id and token at random; here they are fixed, and the test checks the daemon's real answer has exactly this shape.",
        "qr": super::pairing_url(RELAY, &d, &d_public(), &s, MACHINE),
        "slot": slot,
        "1_device_connects": format!("{base}/v1/daemons/{d}/pair/{slot}"),
        "2_device_sends": {
            "plaintext": request.plaintext,
            "sealed": request.to_json(),
            "frame": { "type": "send", "data": request.sealed },
        },
        "3_daemon_receives": recv,
        "4_daemon_asks_the_person": { "request": { "name": PHONE, "platform": "ios" } },
        "5_yes": {
            "daemon_sends": [added, { "type": "pair_send", "slot": slot, "data": paired.sealed }, { "type": "pair_close", "slot": slot }],
            "device_receives": { "type": "recv", "data": paired.sealed },
            "plaintext": paired.plaintext,
            "sealed": paired.to_json(),
        },
        "5_no": {
            "daemon_sends": [{ "type": "pair_send", "slot": slot, "data": refused.sealed }, { "type": "pair_close", "slot": slot }],
            "device_receives": { "type": "recv", "data": refused.sealed },
            "plaintext": refused.plaintext,
            "sealed": refused.to_json(),
        },
        "6_device_connects": {
            "url": format!("{base}/v1/daemons/{d}/devices/{}/connect", device_id()),
            "authorization": format!("Bearer {}", token()),
        },
        "7_unpair": unpair_vector(),
        "ignored": [
            { "name": "sealed with another pairing secret", "frame": { "type": "pair_recv", "slot": slot, "data": wrong } },
            { "name": "for another slot", "frame": { "type": "pair_recv", "slot": "0".repeat(32), "data": request.sealed } },
        ],
    })
}

fn d_public() -> String {
    key("daemon").public
}

// ─── frames.json: seq and ts ─────────────────────────────────────────────

/// A paired phone, connected, with `SQUAD` among the squads.
/// The phone erases itself ("Erase this phone"); the daemon forgets it and says so.
fn unpair_vector() -> Value {
    let told = json!({ "type": "device_unpaired", "device": device_id() });
    let mut core = connected_core();
    let sends = relay_frames(&core.on_relay(&told, NOW));
    assert!(core.config.as_ref().unwrap().devices.is_empty());
    json!({
        "about": "Not sealed. The relay forgets the device token and push token, answers unpaired and closes the socket (1000). It tells the daemon now, or on each connection until the daemon answers device_remove.",
        "device_sends": { "type": "unpair" },
        "device_receives": { "type": "unpaired" },
        "daemon_receives": told,
        "daemon_sends": sends,
    })
}

fn connected_core() -> Core {
    let mut core = Core::new(Some(config(vec![device()])));
    core.squads = vec![(SQUAD.into(), vec!["site".into(), "api".into()]), ("home".into(), vec![])];
    core.on_relay(&json!({ "type": "presence", "device": device_id(), "online": true }), NOW);
    core
}

fn link_aad() -> String {
    format!("{}:{}", daemon_id(), device_id())
}

/// What the phone sends: the plaintext, sealed with a fixed ephemeral key, as
/// the relay hands it to the daemon.
fn phone_frame(name: &str, sender: &'static str, ts: u64, seq: u64, squad: Option<&str>, rpc: Value) -> (Sealed, Value) {
    let mut plain = json!({ "ts": ts, "seq": seq, "rpc": rpc });
    if let Some(squad) = squad {
        plain["squad"] = json!(squad);
    }
    let sealed = seal_fixed(name, Kind::ToDaemon, "daemon", Some(sender), false, &link_aad(), &plain.to_string());
    let recv = json!({ "type": "recv", "device": device_id(), "data": sealed.sealed });
    (sealed, recv)
}

/// A frame the daemon sealed for the phone, opened, its clock set to `NOW`.
fn opened(frame: &Value) -> Value {
    let mut plain: Value = serde_json::from_slice(&seal::open(Kind::ToDevice, &key("device"), Some(&d_public()), None, &link_aad(), frame["data"].as_str().unwrap()).unwrap()).unwrap();
    assert!(plain["ts"].as_u64().unwrap().abs_diff(super::super::now_ms()) < 60_000, "the daemon's clock");
    plain["ts"] = json!(NOW);
    plain
}

fn frame_vectors() -> Value {
    let squads = json!({ "jsonrpc": "2.0", "id": 1, "method": "squads" });
    let skew = super::CLOCK_SKEW_MS;
    let mut core = connected_core();
    let mut steps = Vec::new();
    let step = |core: &mut Core, steps: &mut Vec<Value>, name: &str, sender: &'static str, ts: u64, seq: u64, accepted: bool, why: &str| {
        let (sealed, recv) = phone_frame(name, sender, ts, seq, None, squads.clone());
        let out = relay_frames(&core.on_relay(&recv, NOW));
        assert_eq!(!out.is_empty(), accepted, "{name}");
        let answer = out.first().map(|f| {
            assert_eq!((f["type"].as_str(), f["device"].as_str()), (Some("send"), Some(device_id().as_str())));
            opened(f)
        });
        steps.push(json!({ "name": name, "why": why, "accepted": accepted, "plaintext": sealed.plaintext, "daemon_receives": recv, "daemon_answers": answer }));
    };
    step(&mut core, &mut steps, "seq 1", "device", NOW, 1, true, "the first frame on the connection");
    step(&mut core, &mut steps, "seq 1 again", "device", NOW, 1, false, "not above the last seq seen");
    step(&mut core, &mut steps, "seq 2, from too long ago", "device", NOW - skew - 1, 2, false, "ts more than 5 minutes off; a dropped frame doesn't move seq");
    step(&mut core, &mut steps, "seq 2, from too far ahead", "device", NOW + skew + 1, 2, false, "ts more than 5 minutes off");
    step(&mut core, &mut steps, "seq 2, 5 minutes off exactly", "device", NOW - skew, 2, true, "5 minutes off is still in");
    step(&mut core, &mut steps, "seq 5", "device", NOW, 5, true, "seq may skip; it only has to go up");
    step(&mut core, &mut steps, "seq 4", "device", NOW, 4, false, "below the last seq seen");
    step(&mut core, &mut steps, "seq 6 sealed by a stranger", "stranger", NOW, 6, false, "doesn't open with the device's key");
    step(&mut core, &mut steps, "seq 6", "device", NOW, 6, true, "the stranger's frame didn't move seq");
    let presence_off = json!({ "type": "presence", "device": device_id(), "online": false });
    let presence_on = json!({ "type": "presence", "device": device_id(), "online": true });
    assert_eq!(core.on_relay(&presence_off, NOW), vec![Action::Close { device: device_id() }, Action::Save], "the device closes, and when it was last seen is kept");
    assert!(core.on_relay(&presence_on, NOW).is_empty());
    steps.push(json!({ "name": "the phone reconnects", "relay_tells_daemon": [presence_off, presence_on], "why": "a new WebSocket: both directions count from 1 again" }));
    step(&mut core, &mut steps, "seq 1 on the new connection", "device", NOW, 1, true, "counting starts over; the daemon's answer is its seq 1 too");
    step(&mut core, &mut steps, "seq 0", "device", NOW, 0, false, "seq starts at 1");

    // What the phone must drop from the daemon, by the same rules.
    let mut core = connected_core();
    let mut from_daemon = Vec::new();
    for seq in 1..=3 {
        let (_, recv) = phone_frame(&format!("ask {seq}"), "device", NOW, seq, None, json!({ "jsonrpc": "2.0", "id": seq, "method": "squads" }));
        let out = relay_frames(&core.on_relay(&recv, NOW));
        let plain = opened(&out[0]);
        assert_eq!(plain["seq"], seq);
        from_daemon.push(plain);
    }
    let seal_down = |name: &str, plain: &Value, sender: &'static str| seal_fixed(name, Kind::ToDevice, "device", Some(sender), false, &link_aad(), &plain.to_string());
    let mut stale = from_daemon[2].clone();
    stale["seq"] = json!(4);
    stale["ts"] = json!(NOW - skew - 1);
    let down = [
        ("daemon seq 1", seal_down("daemon seq 1", &from_daemon[0], "daemon"), true, "the first frame on the connection"),
        ("daemon seq 1 again", seal_down("daemon seq 1", &from_daemon[0], "daemon"), false, "not above the last seq seen"),
        ("daemon seq 3", seal_down("daemon seq 3", &from_daemon[2], "daemon"), true, "seq may skip"),
        ("daemon seq 2", seal_down("daemon seq 2", &from_daemon[1], "daemon"), false, "below the last seq seen"),
        ("daemon seq 4 from too long ago", seal_down("daemon seq 4 stale", &stale, "daemon"), false, "ts more than 5 minutes off the phone's clock"),
        ("seq 4 sealed by a stranger", seal_down("stranger seq 4", &json!({ "ts": NOW, "seq": 4, "rpc": { "jsonrpc": "2.0", "method": "event", "params": { "type": "notice", "session": 1, "text": "hi" } } }), "stranger"), false, "doesn't open with the daemon's key"),
    ];
    let to_device: Vec<Value> = down
        .into_iter()
        .map(|(name, sealed, accepted, why)| json!({ "name": name, "why": why, "accepted": accepted, "plaintext": sealed.plaintext, "device_receives": { "type": "recv", "data": sealed.sealed } }))
        .collect();

    // A push's data: seq 0, outside the connection.
    let mut core = connected_core();
    core.on_relay(&json!({ "type": "presence", "device": device_id(), "online": false }), NOW);
    let pushed = relay_frames(&core.on_squad_event(SQUAD, &Event::ApprovalRequested { squad: SQUAD.into(), approval: approval() }));
    let push_plain = opened(&pushed[0]);
    assert_eq!(push_plain["seq"], 0);
    let push = seal_fixed("push data", Kind::ToDevice, "device", Some("daemon"), false, &link_aad(), &push_plain.to_string());
    let mut push_frame = pushed[0].clone();
    push_frame["data"] = json!(push.sealed);

    json!({
        "about": "seq and ts, as the daemon applies them to a device's frames (device_to_daemon, run in order on one daemon — checked against it) and as the phone must apply them to the daemon's (daemon_to_device, in order). The clock is now for both. Keys and ids are seal.json's; aad is <daemon id>:<device id>.",
        "now": NOW,
        "clock_skew_ms": skew,
        "device_to_daemon": steps,
        "daemon_to_device": to_device,
        "push": {
            "about": "A push's data is a daemon-to-device frame with seq 0: shown, not counted, and not checked against the connection's seq.",
            "daemon_sends": push_frame,
            "plaintext": push.plaintext,
            "sealed": push.to_json(),
        },
    })
}

// ─── messages.json: what the daemon says, screen by screen ───────────────

fn approval() -> Approval {
    Approval { id: 17, session: 4242, member: "backend".into(), ticket: Some(38), tool: "Bash".into(), action: "git push origin main".into(), reason: Some("배포 브랜치에 올립니다".into()), preview: None }
}

/// The reference's two questions, from the member on T-38.
fn questions() -> QuestionSet {
    let choice = |label: &str, description: &str| Choice { label: label.into(), description: description.into(), ..Choice::default() };
    QuestionSet {
        id: 5_000_001,
        session: 4242,
        member: "backend".into(),
        ticket: Some(38),
        questions: vec![
            Question {
                question: "로그인 토큰은 어디에 둘까요?".into(),
                header: "토큰".into(),
                options: vec![choice("httpOnly 쿠키 (추천)", "스크립트가 못 읽어 안전합니다"), choice("localStorage", "간단하지만 XSS에 노출됩니다")],
                multi_select: false,
                images: Vec::new(),
            },
            Question {
                question: "어떤 로그인을 먼저 낼까요?".into(),
                header: "범위".into(),
                options: vec![choice("이메일", ""), choice("Google", ""), choice("Apple", "")],
                multi_select: true,
                images: Vec::new(),
            },
        ],
        asked_at: NOW / 1000,
    }
}

/// Pictures a designer made, in its worktree of the squad's folder.
const LOGO_A: &str = "/Users/you/.equipe/worktrees/acme.home/t144-5e6f7a8b/brand/logo-blue-dot.png";
const LOGO_B: &str = "/Users/you/.equipe/worktrees/acme.home/t144-5e6f7a8b/brand/logo-grey-dot.png";

/// A visual choice: the designer's logos, each option with its picture.
fn visual_questions() -> QuestionSet {
    let choice = |label: &str, description: &str, images: &[&str]| Choice { label: label.into(), description: description.into(), images: images.iter().map(|i| (*i).to_owned()).collect() };
    QuestionSet {
        id: 5_000_002,
        session: 4243,
        member: "designer".into(),
        ticket: Some(144),
        questions: vec![Question {
            question: "로고 마스터는 어느 안으로 할까요?".into(),
            header: "로고".into(),
            options: vec![choice("e. + 파란 원 점 (추천)", "사이트와 앱을 하나로", &[LOGO_A]), choice("e. 회색 점 유지", "지금 앱 그대로", &[LOGO_B])],
            multi_select: false,
            images: Vec::new(),
        }],
        asked_at: NOW / 1000,
    }
}

/// The PM proposes one more backend for T-41, which waits while backend is on T-38.
fn team_change() -> TeamChange {
    TeamChange {
        id: 6_000_001,
        change: Change::Add,
        role: "backend".into(),
        count: 1,
        member: None,
        engine: None,
        model: None,
        effort: None,
        reason: "T-41이 4시간째 기다려요 (backend는 T-38 중)".into(),
        unblocks: vec![41],
        by: "pm".into(),
        session: 1001,
        proposed_at: NOW / 1000,
        busy_on: None,
    }
}

fn ticket(status: TicketStatus) -> Ticket {
    Ticket {
        id: 38,
        title: "로그인 API".into(),
        body: "이메일 로그인, 토큰 발급.".into(),
        status,
        kind: TicketKind::Code,
        owner: Some("backend".into()),
        assignee: Some("backend".into()),
        project: Some("/Users/you/projects/api".into()),
        depends_on: vec![],
        branch: Some("equipe/backend-t38-1a2b3c4d".into()),
        worktree: Some("/Users/you/.equipe/worktrees/api/t38-1a2b3c4d".into()),
        merged: None,
        off_main: None,
        created_by: "pm".into(),
        note: None,
        asks: vec![],
        remaining: Vec::new(), images: Vec::new(),
        updated_at: None,
        priority: Default::default(), rank: 0,
    }
}

fn disk_status() -> equipe_core::protocol::disk::DiskStatus {
    use equipe_core::protocol::disk::{DiskLevel, DiskStatus};
    DiskStatus { level: DiskLevel::Critical, free: 4_200_000_000, total: 494_384_795_648, path: "/Users/abel/.equipe".into(), container_free: None }
}

fn snapshot() -> Snapshot {
    Snapshot {
        squad: SQUAD.into(),
        projects: vec!["/Users/you/projects/site".into(), "/Users/you/projects/api".into()],
        sessions: vec![
            SessionInfo {
                session: 1001,
                project: "/Users/you/.equipe/squads/acme/home".into(),
                member: Some(Member { role: "pm".into(), name: "pm".into() }),
                engine: Engine::Claude,
                model: Some("claude-opus-5-5".into()),
                access: Access::ReadOnly,
                workspace: None,
                state: SessionState::Ready,
                title: Some("이번 주 계획".into()),
                tokens: 18200,
                context: Some((41000, 200000)),
                recent: vec![Entry::User { text: "이번 주 계획 정리해 줘".into() }, Entry::Reply { text: "보드에 세 장 올렸어요.".into() }],
                queued: vec![],
                queued_project: None,
                queued_in: vec![],
                steering: vec![],
                steering_in: vec![],
                lane: None,
                estimate: None,
                work: None,
                mode: None,
                effort: None,
                subagents: Vec::new(),
            },
            SessionInfo {
                session: 4242,
                project: "/Users/you/projects/api".into(),
                member: Some(Member { role: "backend".into(), name: "backend".into() }),
                engine: Engine::Claude,
                model: Some("claude-sonnet-5-5".into()),
                access: Access::EditFiles,
                workspace: None,
                state: SessionState::Working,
                title: None,
                tokens: 5300,
                context: None,
                recent: vec![Entry::Tool { tool: "Bash".into(), target: Some("cargo test".into()), outcome: None }],
                queued: vec![],
                queued_project: None,
                queued_in: vec![],
                steering: vec![],
                steering_in: vec![],
                lane: None,
                estimate: None,
                work: Some(WorkInfo { ticket: 38, project: "/Users/you/projects/api".into(), workspace: None }),
                mode: None,
                effort: None,
                // Handed side work, still at it: a screen attaching now lists it under its member.
                subagents: vec![SubagentInfo { id: "toolu_01".into(), what: "Read the API's error logs".into(), step: Some("Grep TimeoutError".into()), secs: 48 }],
            },
        ],
        tickets: vec![ticket(TicketStatus::InProgress)],
        mode: Mode::Auto,
        approvals: vec![approval()],
        questions: vec![questions()],
        team_changes: vec![team_change()],
        scale: Some(0),
        plan: Some(claude_plan()),
        // Every engine's plan as last read: Claude's (as `plan`), and Codex's —
        // whose plan has no five-hour window here, only the weekly one.
        plans: vec![
            EnginePlan { engine: Engine::Claude, usage: claude_plan(), plan_type: Some("max".into()), at: (NOW / 1000) as i64 - 120 },
            EnginePlan { engine: Engine::Codex, usage: codex_plan(), plan_type: Some("pro".into()), at: (NOW / 1000) as i64 - 30 },
        ],
        suggestions_off: false,
        desktop: Default::default(),
        allowed: vec![],
        browsers_shown: false,
        disk: Some(disk_status()),
    }
}

/// The Claude plan's windows, as Claude Code reports them.
fn claude_plan() -> PlanUsage {
    PlanUsage { five_hour: Some(PlanWindow { used_percent: 62, resets_at: 1_760_018_000 }), seven_day: Some(PlanWindow { used_percent: 41, resets_at: 1_760_400_000 }) }
}

/// The Codex plan's: a weekly window only.
fn codex_plan() -> PlanUsage {
    PlanUsage { five_hour: None, seven_day: Some(PlanWindow { used_percent: 18, resets_at: 1_760_500_000 }) }
}

fn message_vectors() -> Value {
    let mut core = connected_core();
    let mut seq = 0;
    let mut examples = serde_json::Map::new();
    // The phone sends `rpc` about `squad`; what the daemon answers, or does.
    let mut ask = |core: &mut Core, examples: &mut serde_json::Map<String, Value>, name: &str, squad: Option<&str>, rpc: Value| -> Vec<Action> {
        seq += 1;
        let (sealed, recv) = phone_frame(name, "device", NOW, seq, squad, rpc);
        let out = core.on_relay(&recv, NOW);
        let does = match &out[..] {
            [] => json!("nothing: dropped without an answer"),
            [Action::Relay(f)] => json!({ "answers": opened(f) }),
            [Action::Open { squad, id, .. }] => json!({ "follows": squad, "answers_later": id }),
            [Action::Command { squad, command, .. }] => json!({ "to_squad": squad, "command": Message::command(command) }),
            [Action::ReadImage { squad, ask, .. }] => json!({ "reads_picture_of": squad, "asked": ask, "max_for_a_phone": equipe_core::visuals::PHONE_SIDE }),
            other => panic!("{name}: {other:?}"),
        };
        examples.insert(name.to_owned(), json!({ "sends": serde_json::from_str::<Value>(&sealed.plaintext).unwrap(), "daemon": does }));
        out
    };
    let tell = |core: &mut Core, examples: &mut serde_json::Map<String, Value>, name: &str, event: Event| {
        let out = relay_frames(&core.on_event(&device_id(), SQUAD, &event));
        examples.insert(name.to_owned(), json!({ "receives": opened(&out[0]) }));
    };
    let pm = 1001;

    // Squads, then one squad's snapshot.
    ask(&mut core, &mut examples, "squads", None, json!({ "jsonrpc": "2.0", "id": 1, "method": "squads" }));
    let out = ask(&mut core, &mut examples, "hello", Some(SQUAD), json!({ "jsonrpc": "2.0", "id": 2, "method": "hello", "params": { "version": "equipe-app/1.0 (ios)", "squad": SQUAD } }));
    let [Action::Open { id, .. }] = &out[..] else { panic!() };
    let answered = relay_frames(&core.opened(&device_id(), SQUAD, id, Ok(Hello { version: "0.10.1".into(), snapshot: snapshot() })));
    examples["hello"]["daemon"]["answers"] = opened(&answered[0]);
    ask(&mut core, &mut examples, "hello: no such squad", Some("nope"), json!({ "jsonrpc": "2.0", "id": 3, "method": "hello", "params": { "version": "equipe-app/1.0 (ios)", "squad": "nope" } }));
    let failed = relay_frames(&core.opened(&device_id(), "home", &json!(9), Err("the daemon couldn't follow home".into())));
    examples.insert("hello: the daemon can't follow the squad".into(), json!({ "receives": opened(&failed[0]) }));
    ask(&mut core, &mut examples, "a method a device may not use", None, json!({ "jsonrpc": "2.0", "id": 4, "method": "shutdown" }));

    // The board.
    tell(&mut core, &mut examples, "event: ticket changed", Event::TicketChanged { squad: SQUAD.into(), ticket: ticket(TicketStatus::Review) });

    // Approvals.
    tell(&mut core, &mut examples, "event: approval requested", Event::ApprovalRequested { squad: SQUAD.into(), approval: approval() });
    let decide = |decision: &str| json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "decide", "squad": SQUAD, "approval": 17, "decision": decision } });
    ask(&mut core, &mut examples, "decide: allow", Some(SQUAD), decide("allow"));
    ask(&mut core, &mut examples, "decide: always is sent as allow", Some(SQUAD), decide("always"));
    ask(&mut core, &mut examples, "decide: deny", Some(SQUAD), decide("deny"));
    tell(&mut core, &mut examples, "event: approval resolved", Event::ApprovalResolved { squad: SQUAD.into(), approval: 17, decision: Decision::Allow });

    // A member's questions: answered — a pick, several with a note, or skipped — or closed.
    tell(&mut core, &mut examples, "event: questions asked", Event::QuestionsAsked { squad: SQUAD.into(), questions: questions() });
    let answer = |answers: Value| json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "answer_questions", "squad": SQUAD, "id": 5_000_001, "answers": answers } });
    ask(&mut core, &mut examples, "answer questions", Some(SQUAD), answer(json!([{ "picked": [0] }, { "picked": [0, 1], "other": "Apple은 다음 달에" }])));
    ask(&mut core, &mut examples, "answer questions: one skipped", Some(SQUAD), answer(json!([{ "other": "세션 쿠키로" }, {}])));
    ask(&mut core, &mut examples, "dismiss questions", Some(SQUAD), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "dismiss_questions", "squad": SQUAD, "id": 5_000_001 } }));
    let answered = vec![Answer { picked: vec![0], other: None }, Answer { picked: vec![0, 1], other: Some("Apple은 다음 달에".into()) }];
    tell(&mut core, &mut examples, "event: questions answered", Event::QuestionsClosed { squad: SQUAD.into(), id: 5_000_001, answers: Some(answered) });
    tell(&mut core, &mut examples, "event: questions closed", Event::QuestionsClosed { squad: SQUAD.into(), id: 5_000_001, answers: None });
    // A visual choice: each option with its pictures, which the phone asks for, as thumbnails.
    tell(&mut core, &mut examples, "event: visual questions asked", Event::QuestionsAsked { squad: SQUAD.into(), questions: visual_questions() });
    let picture = |id: u64, squad: &str| json!({ "jsonrpc": "2.0", "id": id, "method": "read_image", "params": { "squad": squad, "path": LOGO_A, "max": 480 } });
    let out = ask(&mut core, &mut examples, "read image", Some(SQUAD), picture(20, SQUAD));
    let [Action::ReadImage { id, .. }] = &out[..] else { panic!("{out:?}") };
    let read = equipe_core::protocol::rpc::ImageRead { mime: "image/png".into(), size: 48_213, width: Some(1024), height: Some(1024), data: "iVBORw0KGgo=".into(), scaled: true };
    examples["read image"]["daemon"]["answers"] = opened(&relay_frames(&core.image_read(&device_id(), SQUAD, *id, Ok(read)))[0]);
    let refused = relay_frames(&core.image_read(&device_id(), SQUAD, 21, Err(format!("{LOGO_A} isn't a picture {SQUAD} shows"))));
    examples.insert("read image: a picture the squad doesn't show".into(), json!({ "receives": opened(&refused[0]) }));
    ask(&mut core, &mut examples, "read image: a squad not followed", Some("home"), picture(22, "home"));
    // A hand-in of visual work: its pictures over the note.
    let mut shown = ticket(TicketStatus::Review);
    shown.images = vec![LOGO_A.into(), LOGO_B.into()];
    tell(&mut core, &mut examples, "event: ticket handed in with pictures", Event::TicketChanged { squad: SQUAD.into(), ticket: shown });

    // A change to the team the PM proposed: approved or declined by the person.
    tell(&mut core, &mut examples, "event: team change proposed", Event::TeamChangeProposed { squad: SQUAD.into(), change: team_change() });
    ask(&mut core, &mut examples, "approve team change", Some(SQUAD), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "approve_team_change", "squad": SQUAD, "id": 6_000_001 } }));
    ask(&mut core, &mut examples, "decline team change", Some(SQUAD), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "decline_team_change", "squad": SQUAD, "id": 6_000_001 } }));
    tell(&mut core, &mut examples, "event: team change approved", Event::TeamChangeClosed { squad: SQUAD.into(), id: 6_000_001, approved: true, members: vec!["backend-2".into()] });
    tell(&mut core, &mut examples, "event: team change declined", Event::TeamChangeClosed { squad: SQUAD.into(), id: 6_000_001, approved: false, members: vec![] });

    // Review.
    ask(&mut core, &mut examples, "accept", Some(SQUAD), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "accept", "squad": SQUAD, "ticket": 38 } }));
    ask(&mut core, &mut examples, "reopen", Some(SQUAD), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "reopen", "squad": SQUAD, "ticket": 38, "note": "에러 메시지를 한국어로" } }));
    let mut done = ticket(TicketStatus::Done);
    done.merged = Some(false);
    done.note = Some("로그인 API와 테스트".into());
    tell(&mut core, &mut examples, "event: ticket accepted", Event::TicketChanged { squad: SQUAD.into(), ticket: done });

    // Talking with the PM.
    ask(&mut core, &mut examples, "submit", Some(SQUAD), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "submit", "session": pm, "text": "로그인 화면 다음에 뭐 할까?" } }));
    ask(&mut core, &mut examples, "interrupt", Some(SQUAD), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "interrupt", "session": pm } }));
    tell(&mut core, &mut examples, "event: user message", Event::UserMessage { session: pm, text: "로그인 화면 다음에 뭐 할까?".into() });
    tell(&mut core, &mut examples, "event: message queued", Event::MessageQueued { session: pm, text: "그리고 테스트도".into(), project: None });
    tell(&mut core, &mut examples, "event: text delta", Event::TextDelta { session: pm, text: "보드를 보면".into() });
    tell(&mut core, &mut examples, "event: tool use", Event::ToolUse { session: pm, tool: "board".into(), target: None });
    // Side work handed to a subagent: it starts, takes steps, hands back (the snapshot lists those still at it).
    tell(&mut core, &mut examples, "event: subagent started", Event::SubagentStarted { session: pm, id: "toolu_02".into(), what: "Compare the two login flows".into() });
    tell(&mut core, &mut examples, "event: subagent step", Event::SubagentStep { session: pm, id: "toolu_02".into(), step: "Read src/login.tsx".into() });
    tell(&mut core, &mut examples, "event: subagent ended", Event::SubagentEnded { session: pm, id: "toolu_02".into() });
    tell(&mut core, &mut examples, "event: turn ended", Event::TurnEnded { session: pm, outcome: TurnOutcome::Completed, usage: Some(Usage { input_tokens: Some(1200), output_tokens: Some(340), ..Usage::default() }) });
    tell(&mut core, &mut examples, "event: turn interrupted", Event::TurnEnded { session: pm, outcome: TurnOutcome::Interrupted, usage: None });
    // Each engine's plan usage, for every screen; a phone that knows no `engine` reads Claude's.
    tell(&mut core, &mut examples, "event: plan usage (claude)", Event::PlanUsage { usage: claude_plan(), engine: Engine::Claude });
    tell(&mut core, &mut examples, "event: plan usage (codex)", Event::PlanUsage { usage: codex_plan(), engine: Engine::Codex });
    tell(&mut core, &mut examples, "event: notice", Event::Notice { session: pm, text: "↪ You → backend: 로그인 API 먼저".into() });
    tell(&mut core, &mut examples, "event: error", Event::Error { session: Some(pm), message: "Claude Code isn't signed in".into(), next: Some("claude login".into()) });
    // What the screen shows goes with the message: a project, or (null) the whole squad.
    ask(&mut core, &mut examples, "submit: about a project", Some(SQUAD), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "submit", "session": pm, "text": "웹사이트는 어떻게 되어가?", "about": "web" } }));
    ask(&mut core, &mut examples, "submit: about the whole squad", Some(SQUAD), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "submit", "session": pm, "text": "전체 상황은?", "about": null } }));
    tell(&mut core, &mut examples, "event: steer queued", Event::SteerQueued { session: pm, text: "푸터는 빼 줘".into(), project: Some("web".into()) });
    // What waits, sent now: the turn stops and it goes as one message.
    ask(&mut core, &mut examples, "send now", Some(SQUAD), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "send_now", "session": pm, "texts": ["푸터는 빼 줘", "그리고 테스트도"] } }));
    tell(&mut core, &mut examples, "event: queue sent", Event::QueueSent { session: pm, texts: vec!["푸터는 빼 줘".into(), "그리고 테스트도".into()] });

    // The disk's room, told on a change of level (and in the snapshot while it's short).
    tell(&mut core, &mut examples, "event: disk level", Event::DiskLevel { status: disk_status() });

    // Not from a phone.
    ask(&mut core, &mut examples, "merge is refused", Some(SQUAD), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "merge", "squad": SQUAD, "ticket": 38, "push": true } }));
    ask(&mut core, &mut examples, "a command for another squad than the frame's", Some(SQUAD), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "accept", "squad": "home", "ticket": 1 } }));
    ask(&mut core, &mut examples, "a command for a squad not followed", Some("home"), json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "accept", "squad": "home", "ticket": 1 } }));

    // Pushes, to a phone away.
    core.on_relay(&json!({ "type": "presence", "device": device_id(), "online": false }), NOW);
    for (name, event) in [
        ("push: approval", Event::ApprovalRequested { squad: SQUAD.into(), approval: approval() }),
        ("push: review", Event::TicketChanged { squad: SQUAD.into(), ticket: ticket(TicketStatus::Review) }),
        ("push: waiting", Event::TicketChanged { squad: SQUAD.into(), ticket: ticket(TicketStatus::Waiting) }),
        ("push: decision", Event::QuestionsAsked { squad: SQUAD.into(), questions: questions() }),
        ("push: team change", Event::TeamChangeProposed { squad: SQUAD.into(), change: team_change() }),
        ("push: disk", Event::DiskLevel { status: disk_status() }),
    ] {
        let out = relay_frames(&core.on_squad_event(SQUAD, &event));
        let mut push = out[0].clone();
        let data = opened(&push);
        push["data"] = json!("<sealed daemon-to-device frame: data_opened>");
        examples.insert(name.into(), json!({ "daemon_sends": push, "data_opened": data }));
    }
    json!({
        "about": "What the daemon sends and does, screen by screen: each example ran through the daemon's code (remote::core) with the vectors' keys. sends: the plaintext the phone seals. daemon: what came of it — answers (the plaintext it sealed back, ts set to now), to_squad (the command the squad got), or nothing. receives: an event as the phone opens it. The hello snapshot is a made-up squad, serialized by the daemon.",
        "now": NOW,
        "examples": examples,
    })
}

// ─── uploads.json: a phone's files, a piece at a time ────────────────────

/// Where the vectors say the daemon keeps uploads, and the stamp it names them with.
const UPLOADS_HOME: &str = "/Users/abel/.equipe";

/// A daemon's answer with this run's home and clock put back to the vectors'.
fn kept_as_vectors(value: &Value, home: &Path) -> Value {
    let text = value.to_string().replace(&home.display().to_string(), UPLOADS_HOME);
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("/uploads/") {
        let (head, tail) = rest.split_at(at + "/uploads/".len());
        out.push_str(head);
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 13 && tail[digits..].starts_with('-') {
            out.push_str(&NOW.to_string());
            rest = &tail[digits..];
        } else {
            rest = tail;
        }
    }
    out.push_str(rest);
    serde_json::from_str(&out).unwrap()
}

/// Run `steps` (`[name, rpc]`) through a daemon with a home of its own: what
/// the phone sealed for each, and what came back, opened.
fn run_uploads(steps: &[(String, Value)]) -> Vec<Value> {
    let home = std::env::temp_dir().join(format!("equipe-upload-vectors-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let mut core = connected_core();
    // No ffmpeg: the video's answer is the same on every machine.
    let mut uploads = super::super::upload::Uploads::with_tools(&home, vec![]);
    let mut out = Vec::new();
    for (seq, (name, rpc)) in steps.iter().enumerate() {
        let (sealed, recv) = phone_frame(name, "device", NOW, seq as u64 + 1, None, rpc.clone());
        let mut answer = Value::Null;
        for action in core.on_relay(&recv, NOW) {
            match action {
                Action::Upload { device, id, piece } => {
                    let message = match uploads.piece(&device, *piece, NOW) {
                        Ok(result) => Message::response(id, result),
                        Err(refusal) => refusal.answer(id),
                    };
                    let frames = relay_frames(&core.answer(&device, &message));
                    answer = kept_as_vectors(&opened(&frames[0]), &home);
                }
                Action::UploadCancel { device, upload_id } => uploads.cancel(&device, &upload_id),
                other => panic!("{name}: {other:?}"),
            }
        }
        out.push(json!({ "name": name, "sends": serde_json::from_str::<Value>(&sealed.plaintext).unwrap(), "daemon_answers": answer }));
    }
    let _ = std::fs::remove_dir_all(&home);
    out
}

fn upload_steps() -> Vec<(String, Value)> {
    use base64::Engine as _;
    let data = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut id = 0;
    let mut up = |name: &str, params: Value| {
        id += 1;
        (name.to_owned(), json!({ "jsonrpc": "2.0", "id": id, "method": "upload", "params": params }))
    };
    let photo = b"\xff\xd8\xff\xe0 a tiny JPEG \xff\xd9";
    let piece = |offset: usize, len: usize| json!({ "upload_id": "3f1c2a9e-0d4b-4c55-9a7e-61b2c8e0aa01", "name": "IMG_0421.JPG", "kind": "image", "total": photo.len(), "offset": offset, "data": data(&photo[offset..offset + len]) });
    vec![
        up("a photo: the first piece", piece(0, 10)),
        up("the first piece again (its answer was lost)", piece(0, 10)),
        up("a piece past where the daemon is", piece(15, 4)),
        up("the last piece: kept, with its tag", piece(10, photo.len() - 10)),
        up("the last piece again: the same answer", piece(10, photo.len() - 10)),
        up("a sound with what the phone heard", json!({ "upload_id": "voice-memo-0001", "name": "Voice memo.m4a", "kind": "file", "total": 4, "offset": 0, "data": data(b"\0\0\0\x18"), "transcript": "내일 회의는 3시로 옮겨 주세요.\n자료는 금요일까지.", "duration": 31.4 })),
        up("a video, on a Mac without ffmpeg", json!({ "upload_id": "clip-0001", "name": "IMG_0500.MOV", "kind": "video", "total": 4, "offset": 0, "data": data(b"\0\0\0\x14"), "transcript": "여기 버튼이 안 눌려요", "duration": 12.0 })),
        up("a PDF", json!({ "upload_id": "spec-0001", "name": "기획서 v2.pdf", "kind": "pdf", "total": 8, "offset": 0, "data": data(b"%PDF-1.7"), "pages": 12 })),
        up("a text file: its lines counted", json!({ "upload_id": "notes-0001", "name": "notes.md", "kind": "file", "total": 14, "offset": 0, "data": data(b"one\ntwo\nthree\n") })),
        up("an image too large", json!({ "upload_id": "big-0001", "name": "a.png", "kind": "image", "total": super::super::upload::IMAGE_MAX + 1, "offset": 0, "data": data(b"x") })),
        up("a file too large", json!({ "upload_id": "big-0002", "name": "a.zip", "kind": "file", "total": super::super::upload::FILE_MAX + 1, "offset": 0, "data": data(b"x") })),
        up("a piece going past the total", json!({ "upload_id": "big-0003", "name": "a.zip", "kind": "file", "total": 1, "offset": 0, "data": data(b"xy") })),
        up("a name the kind doesn't take", json!({ "upload_id": "bad-0001", "name": "run.sh", "kind": "image", "total": 1, "offset": 0, "data": data(b"x") })),
        up("a piece for a file the daemon doesn't know", json!({ "upload_id": "gone-0001", "name": "a.zip", "kind": "file", "total": 20, "offset": 10, "data": data(b"x") })),
        up("a file under way", json!({ "upload_id": "cancel-0001", "name": "a.zip", "kind": "file", "total": 20, "offset": 0, "data": data(b"0123456789") })),
        ("cancelled".to_owned(), json!({ "jsonrpc": "2.0", "method": "upload_cancel", "params": { "upload_id": "cancel-0001" } })),
        up("its next piece, after the cancel", json!({ "upload_id": "cancel-0001", "name": "a.zip", "kind": "file", "total": 20, "offset": 10, "data": data(b"0123456789") })),
    ]
}

fn upload_vectors() -> Value {
    let steps = run_uploads(&upload_steps());
    let photo = steps[3]["daemon_answers"]["rpc"]["result"]["tag"].as_str().unwrap().to_owned();
    let voice = steps[5]["daemon_answers"]["rpc"]["result"]["tag"].as_str().unwrap().to_owned();
    let submit = json!({ "jsonrpc": "2.0", "method": "command", "params": { "type": "submit", "squad": SQUAD, "session": 1001, "text": format!("이거 봐 주세요 {photo} {voice}") } });
    let sealed = seal_fixed("an upload piece", Kind::ToDaemon, "daemon", Some("device"), false, &link_aad(), &steps[0]["sends"].to_string());
    json!({
        "about": "A phone's files, a piece at a time (docs/remote-protocol.md, Attachments). Run in order on one daemon whose uploads are at /Users/abel/.equipe/uploads and whose clock is now — paths and stamps put back to those; checked against the daemon. sends: the plaintext the phone seals (device_to_daemon). daemon_answers: what it sealed back, opened; null when there is no answer. The video ran without ffmpeg, so it has no frames and the notice says so; with ffmpeg its tag carries `1920×1080 · frames: <path>@0:02, …` after the length.",
        "now": NOW,
        "limits": super::super::upload::limits(),
        "steps": steps,
        "then_submit": { "about": "Each finished tag, then a space, after what was typed (the desktop's attach::message): sent as submit's text.", "sends": submit },
        "sealed_piece": sealed.to_json(),
    })
}

// ─── the files ───────────────────────────────────────────────────────────

fn files() -> Vec<(&'static str, Value)> {
    vec![("seal.json", seal_vectors()), ("qr.json", qr_vectors()), ("pairing.json", pairing_vectors()), ("frames.json", frame_vectors()), ("messages.json", message_vectors()), ("uploads.json", upload_vectors())]
}

#[test]
fn vectors_are_what_the_daemon_makes() {
    let write = std::env::var_os("EQUIPE_WRITE_VECTORS").is_some();
    for (name, value) in files() {
        let text = serde_json::to_string_pretty(&value).unwrap() + "\n";
        let path = dir().join(name);
        if write {
            std::fs::create_dir_all(dir()).unwrap();
            std::fs::write(&path, &text).unwrap();
        } else {
            let kept = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(kept == text, "{} isn't what the daemon makes now: EQUIPE_WRITE_VECTORS=1 cargo test -p equipe vectors writes it again", path.display());
        }
    }
}

/// The kept files, read back and checked with the daemon's code.
#[test]
fn the_daemon_passes_the_kept_vectors() {
    if std::env::var_os("EQUIPE_WRITE_VECTORS").is_some() {
        return; // being written alongside
    }
    let read = |name: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(dir().join(name)).unwrap()).unwrap() };
    let seals = read("seal.json");
    for case in seals["cases"].as_array().unwrap() {
        let open = json!({ "kind": case["kind"], "recipient": case["recipient"], "sender": case["sender"], "psk": if case["psk"] == true { seals["pairing_secret"].clone() } else { Value::Null }, "aad": case["aad"] });
        let sealed = case["sealed"].as_str().unwrap();
        assert_eq!(open_as(&open, sealed).unwrap(), case["plaintext"].as_str().unwrap().as_bytes(), "{}", case["name"]);
        let kind = kind_of(case["kind"].as_str().unwrap());
        let secret = unb64(seals["pairing_secret"].as_str().unwrap()).unwrap();
        let ikm: [u8; 32] = unb64(case["ikm_e"].as_str().unwrap()).unwrap().try_into().unwrap();
        let sender = case["sender"].as_str().map(key);
        let again = seal::seal_with_ephemeral(kind, &key(case["recipient"].as_str().unwrap()).public, sender.as_ref(), (case["psk"] == true).then_some(&secret[..]), case["aad"].as_str().unwrap(), case["plaintext"].as_str().unwrap().as_bytes(), &ikm).unwrap();
        assert_eq!(again, sealed, "{}", case["name"]);
    }
    for other in seals["sealed_elsewhere"].as_array().unwrap() {
        assert_eq!(open_as(&other["open"], other["sealed"].as_str().unwrap()).unwrap(), other["plaintext"].as_str().unwrap().as_bytes(), "{}", other["sealed_by"]);
    }
    for failure in seals["failures"].as_array().unwrap() {
        assert!(open_as(&failure["open"], failure["sealed"].as_str().unwrap()).is_err(), "{}", failure["name"]);
    }
    let qr = read("qr.json");
    for case in qr["valid"].as_array().unwrap() {
        let parsed = parse_qr(case["uri"].as_str().unwrap()).unwrap();
        assert_eq!((parsed.daemon.as_str(), b64(&parsed.secret)), (case["daemon"].as_str().unwrap(), case["secret"].as_str().unwrap().to_owned()));
        assert_eq!(slot(&parsed.secret), case["slot"].as_str().unwrap());
    }
    for case in qr["invalid"].as_array().unwrap() {
        assert!(parse_qr(case["uri"].as_str().unwrap()).is_err(), "{}", case["name"]);
    }
    let pairing = read("pairing.json");
    let mut core = pairing_core();
    assert_eq!(core.on_relay(&pairing["3_daemon_receives"], NOW), vec![Action::Local(pairing["4_daemon_asks_the_person"].clone())]);
    let unpair = &pairing["7_unpair"];
    assert_eq!(Value::Array(relay_frames(&connected_core().on_relay(&unpair["daemon_receives"], NOW))), unpair["daemon_sends"]);
    let uploads = read("uploads.json");
    let steps: Vec<(String, Value)> = uploads["steps"].as_array().unwrap().iter().map(|s| (s["name"].as_str().unwrap().to_owned(), s["sends"]["rpc"].clone())).collect();
    for (ran, kept) in run_uploads(&steps).iter().zip(uploads["steps"].as_array().unwrap()) {
        assert_eq!(ran["daemon_answers"], kept["daemon_answers"], "{}", kept["name"]);
    }
    let frames = read("frames.json");
    let mut core = connected_core();
    for step in frames["device_to_daemon"].as_array().unwrap() {
        if let Some(presence) = step["relay_tells_daemon"].as_array() {
            for p in presence {
                core.on_relay(p, NOW);
            }
            continue;
        }
        let out = relay_frames(&core.on_relay(&step["daemon_receives"], NOW));
        assert_eq!(!out.is_empty(), step["accepted"] == true, "{}", step["name"]);
        if let Some(f) = out.first() {
            assert_eq!(opened(f), step["daemon_answers"], "{}", step["name"]);
        }
    }
}
