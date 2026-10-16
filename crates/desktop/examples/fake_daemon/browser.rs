//! `FAKE_BROWSER=1`: the shared Chrome as the browser panel sees it, with
//! nothing real opened — frontend on an App Store Connect–like page, backend
//! stopped at a "Just a moment…" wall (paused for the person, as the daemon
//! pauses a member at a human check), and one tab of the person's own. The
//! `browser_stream` connection gets drawn pictures of the tab watched: at the
//! panel's size (`fit`), as the daemon lays out any page a panel watches —
//! or, with `FAKE_FIT=driving` (how 0.47 did it), only when the person drives
//! it, a member's own at its 1280×800. A press on the wall passes it, as a person's click would.
//! `FAKE_BROWSER=empty`: no tabs at all. `FAKE_BROWSER=handover`: besides,
//! the PM has handed the person its browser at Apple's sign-in, asking them
//! to sign in to App Store Connect (`show_my_browser`, as a daemon on another
//! Mac would put it in the app's panel); given back, the PM carries on.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use equipe_core::protocol::stream::{self, Fit, FrameHeader, StreamIn};
use equipe_core::protocol::{BrowserPage, Command, Event, HumanCheck};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::broadcast;

const SQUAD: &str = "demo";
const FRONTEND: &str = "demo/frontend@squad";
const BACKEND: &str = "demo/backend@squad";
/// The PM's own browser, with `FAKE_BROWSER=handover`.
pub const PM: &str = "demo/pm";
/// What the PM asked the person to do in it.
pub const PM_WHY: &str = "App Store Connect(Umbriles Inc. 계정 소유자 Apple ID)에 로그인해 주세요. 끝나면 DSA '거래자' 신고는 제가 이어서 합니다.";
/// Ids as a host started now would give them: far from 1.
const FIRST: u64 = 1_879_000_000_000_001;

pub fn enabled() -> bool {
    std::env::var("FAKE_BROWSER").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// `FAKE_BROWSER=handover`: the PM's browser is in the person's hands.
pub fn handover() -> bool {
    std::env::var("FAKE_BROWSER").is_ok_and(|v| v == "handover")
}

struct Lane {
    id: &'static str,
    session: Option<u64>,
    member: Option<&'static str>,
    color: Option<&'static str>,
    pages: Vec<BrowserPage>,
}

struct State {
    lanes: Vec<Lane>,
    /// Lanes in the person's hands.
    held: HashSet<String>,
    next: u64,
    /// Bumped when what a page shows changes.
    seq: HashMap<u64, u64>,
}

fn page(id: u64, url: &str, title: &str, check: Option<HumanCheck>) -> BrowserPage {
    BrowserPage { id, url: url.into(), title: title.into(), check, ..Default::default() }
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        let empty = std::env::var("FAKE_BROWSER").is_ok_and(|v| v == "empty");
        let mut lanes = if empty {
            Vec::new()
        } else {
            vec![
                Lane {
                    id: FRONTEND,
                    session: Some(2),
                    member: Some("frontend (the whole squad)"),
                    color: Some("120, 190, 240"),
                    pages: vec![page(FIRST, "https://appstoreconnect.apple.com/apps/6740012345/distribution", "App Store Connect", None)],
                },
                Lane {
                    id: BACKEND,
                    session: Some(3),
                    member: Some("backend (the whole squad)"),
                    color: Some("140, 208, 150"),
                    pages: vec![
                        page(FIRST + 1, "https://dash.cloudflare.com/", "Just a moment...", Some(HumanCheck::Challenge)),
                        page(FIRST + 2, "https://developers.cloudflare.com/workers/", "Cloudflare Workers · Cloudflare Docs", None),
                    ],
                },
                Lane { id: "", session: None, member: None, color: None, pages: vec![page(FIRST + 3, "https://github.com/equipe-labs/equipe/pulls", "Pull requests · equipe-labs/equipe", None)] },
            ]
        };
        // Backend stopped at the wall: in the person's hands, as the daemon puts it.
        let mut held = if empty { HashSet::new() } else { HashSet::from([BACKEND.to_owned()]) };
        if handover() {
            lanes.insert(
                0,
                Lane {
                    id: PM,
                    session: Some(1),
                    member: Some("pm"),
                    color: Some("232, 190, 120"),
                    pages: vec![page(FIRST + 4, "https://idmsa.apple.com/appleauth/auth/signin?widgetKey=appstoreconnect", "Sign In - Apple", Some(HumanCheck::SignIn))],
                },
            );
            held.insert(PM.to_owned());
        }
        Mutex::new(State { lanes, held, next: FIRST + 10, seq: HashMap::new() })
    })
}

fn tabs(lane: &Lane) -> Event {
    let agent = lane.pages.first().map(|p| p.id).filter(|_| !lane.id.is_empty());
    Event::BrowserTabs {
        squad: (!lane.id.is_empty()).then(|| SQUAD.to_owned()),
        lane: lane.id.to_owned(),
        session: lane.session,
        member: lane.member.map(str::to_owned),
        color: lane.color.map(str::to_owned),
        pages: lane.pages.clone(),
        agent_page: agent,
    }
}

/// What a screen attaching is told first: every lane's tabs, and who is held.
pub fn replay() -> Vec<Event> {
    if !enabled() {
        return Vec::new();
    }
    let s = state().lock().unwrap();
    let mut out: Vec<Event> = s.lanes.iter().map(tabs).collect();
    for lane in &s.held {
        // The PM's was handed over, saying what for; the daemon tells it so to every screen.
        if lane == PM {
            out.push(Event::BrowserControl { squad: Some(SQUAD.into()), lane: lane.clone(), by: Some("person".into()), why: Some(PM_WHY.into()), check: None });
            // `FAKE_WANTS=1`: the PM tried closing the tab the person is on.
            if std::env::var_os("FAKE_WANTS").is_some() {
                out.push(Event::BrowserWants { squad: Some(SQUAD.into()), lane: lane.clone(), page: FIRST + 4, action: "close".into() });
            }
            continue;
        }
        let check = s.lanes.iter().find(|l| l.id == lane).and_then(|l| l.pages.iter().find_map(|p| p.check));
        out.push(Event::BrowserControl { squad: Some(SQUAD.into()), lane: lane.clone(), by: Some("person".into()), why: None, check });
    }
    out
}

/// A browser command, as the daemon's host would answer it. `false`: not one.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    if !enabled() {
        return false;
    }
    let mut s = state().lock().unwrap();
    match command {
        Command::BrowserControl { lane, take } => {
            if *take {
                s.held.insert(lane.clone());
            } else {
                s.held.remove(lane);
            }
            let _ = events.send(Event::BrowserControl { squad: Some(SQUAD.into()), lane: lane.clone(), by: take.then(|| "person".into()), why: None, check: None });
            // The PM's handed back: the person signed in, and it is on App Store Connect.
            if !*take
                && lane == PM
                && let Some(pm) = s.lanes.iter_mut().find(|l| l.id == PM)
            {
                pm.pages = vec![page(FIRST + 4, "https://appstoreconnect.apple.com/business/eu-dsa", "App Store Connect", None)];
                let _ = events.send(tabs(pm));
                *s.seq.entry(FIRST + 4).or_default() += 1;
            }
        }
        Command::BrowserNewTab { url } => {
            let id = s.next;
            s.next += 1;
            let url = url.clone().unwrap_or_else(|| "about:blank".into());
            let title = if url == "about:blank" { String::new() } else { url.trim_start_matches("https://").split('/').next().unwrap_or_default().to_owned() };
            if let Some(mine) = s.lanes.iter_mut().find(|l| l.id.is_empty()) {
                mine.pages.push(page(id, &url, &title, None));
            } else {
                s.lanes.push(Lane { id: "", session: None, member: None, color: None, pages: vec![page(id, &url, &title, None)] });
            }
            let mine = s.lanes.iter().find(|l| l.id.is_empty()).unwrap();
            let _ = events.send(tabs(mine));
        }
        Command::BrowserCloseTab { page } => {
            for lane in s.lanes.iter_mut() {
                lane.pages.retain(|p| p.id != *page);
            }
            for lane in &s.lanes {
                let _ = events.send(tabs(lane));
            }
        }
        Command::BrowserNav { .. } | Command::BrowserDialog { .. } | Command::BrowserPopOut { .. } => {}
        _ => return false,
    }
    true
}

/// The person's press on the wall: it passes.
fn pressed(page: u64, events: &broadcast::Sender<Event>) {
    let mut s = state().lock().unwrap();
    // A hand on a member's page takes it, as the host does.
    let lane_id = s.lanes.iter().find(|l| l.pages.iter().any(|p| p.id == page)).map(|l| l.id).filter(|l| !l.is_empty());
    if let Some(lane) = lane_id
        && s.held.insert(lane.to_owned())
    {
        let _ = events.send(Event::BrowserControl { squad: Some(SQUAD.into()), lane: lane.to_owned(), by: Some("person".into()), why: None, check: None });
    }
    let Some(lane) = s.lanes.iter_mut().find(|l| l.pages.iter().any(|p| p.id == page && p.check.is_some())) else { return };
    for p in lane.pages.iter_mut().filter(|p| p.id == page) {
        p.check = None;
        p.title = "Overview | Cloudflare".into();
        p.url = "https://dash.cloudflare.com/8f2c/home/domains".into();
    }
    let _ = events.send(tabs(lane));
    *s.seq.entry(page).or_default() += 1;
}

/// Whether the person drives `page` (their own tab, or a member's in their hands).
fn driven(page: u64) -> bool {
    let s = state().lock().unwrap();
    s.lanes.iter().find(|l| l.pages.iter().any(|p| p.id == page)).is_some_and(|l| l.id.is_empty() || s.held.contains(l.id))
}

/// The `browser_stream` connection, once answered: frames of the tab
/// watched as credit allows, the person's presses back.
pub async fn serve_stream(mut reader: impl AsyncRead + Unpin, mut writer: impl AsyncWrite + Unpin, events: broadcast::Sender<Event>) {
    let mut decoder = stream::Decoder::default();
    let mut buf = vec![0u8; 64 * 1024];
    let mut watching: Option<(u64, u32, u32, Option<Fit>)> = None;
    let mut credit = stream::INITIAL_CREDIT;
    let mut sent: Option<(u64, u64, bool, Option<Fit>)> = None;
    let mut seq = 0u64;
    let mut tick = tokio::time::interval(Duration::from_millis(150));
    loop {
        tokio::select! {
            read = reader.read(&mut buf) => {
                let n = match read { Ok(0) | Err(_) => return, Ok(n) => n };
                decoder.push(&buf[..n]);
                while let Ok(Some((kind, payload))) = decoder.message() {
                    if kind != stream::JSON { continue }
                    let Ok(message) = serde_json::from_slice::<StreamIn>(&payload) else { continue };
                    match message {
                        StreamIn::Watch { page, max_w, max_h, fit, .. } => watching = Some((page, max_w, max_h, fit)),
                        StreamIn::Unwatch => watching = None,
                        StreamIn::Credit { n } => credit = (credit + n).min(8),
                        StreamIn::Input { page, ev } => {
                            if let equipe_core::protocol::stream::InputEvent::Mouse { kind, .. } = &ev
                                && kind == "mousePressed"
                            {
                                pressed(page, &events);
                            }
                        }
                    }
                }
            }
            _ = tick.tick() => {}
        }
        let Some((page, max_w, max_h, fit)) = watching else { continue };
        if credit == 0 {
            continue;
        }
        let version = state().lock().unwrap().seq.get(&page).copied().unwrap_or(0);
        let drives = driven(page);
        if sent == Some((page, version, drives, fit)) {
            continue;
        }
        // At the panel's size, as the daemon lays out a page a panel watches; with
        // `FAKE_FIT=driving`, only one the person drives — the member's own otherwise.
        let old_rule = std::env::var("FAKE_FIT").is_ok_and(|v| v == "driving");
        let (css_w, css_h, scale) = match fit.filter(|_| drives || !old_rule) {
            Some(f) => (f.w as f64, f.h as f64, f.dpr as f64),
            None => (1280., 800., 1.),
        };
        let k = ((max_w as f64) / (css_w * scale)).min((max_h as f64) / (css_h * scale)).min(1.);
        let (w, h) = (((css_w * scale * k).round() as u32).max(1), ((css_h * scale * k).round() as u32).max(1));
        let Some(jpeg) = draw(page, w, h) else { continue };
        seq += 1;
        let header = FrameHeader { page, seq, w, h, css_w, css_h, sx: 0., sy: 0., top: 0. };
        if writer.write_all(&stream::encode(stream::FRAME, &stream::frame_payload(&header, &jpeg))).await.is_err() {
            return;
        }
        credit -= 1;
        sent = Some((page, version, drives, fit));
    }
}

/// A page drawn in blocks, by what it is: each tab looks like itself.
fn draw(page: u64, w: u32, h: u32) -> Option<Vec<u8>> {
    let (url, check) = {
        let s = state().lock().unwrap();
        let p = s.lanes.iter().flat_map(|l| l.pages.iter()).find(|p| p.id == page)?;
        (p.url.clone(), p.check)
    };
    let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([255, 255, 255]));
    let mut rect = |x: f32, y: f32, rw: f32, rh: f32, c: [u8; 3]| {
        let (x0, y0) = ((x * w as f32) as u32, (y * h as f32) as u32);
        let (x1, y1) = (((x + rw) * w as f32).min(w as f32) as u32, ((y + rh) * h as f32).min(h as f32) as u32);
        for yy in y0..y1 {
            for xx in x0..x1 {
                img.put_pixel(xx, yy, image::Rgb(c));
            }
        }
    };
    if matches!(check, Some(HumanCheck::Challenge | HumanCheck::Captcha)) {
        // The wall: a plain page, the site's name, a box to tick, a footer.
        rect(0.08, 0.16, 0.42, 0.05, [40, 40, 40]);
        rect(0.08, 0.26, 0.6, 0.025, [120, 120, 120]);
        rect(0.08, 0.36, 0.5, 0.11, [250, 250, 250]);
        rect(0.08, 0.36, 0.5, 0.004, [210, 210, 210]);
        rect(0.08, 0.466, 0.5, 0.004, [210, 210, 210]);
        rect(0.11, 0.39, 0.04, 0.05, [230, 230, 230]);
        rect(0.45, 0.395, 0.1, 0.04, [243, 128, 32]);
        rect(0.08, 0.86, 0.84, 0.002, [220, 220, 220]);
        rect(0.35, 0.89, 0.3, 0.02, [170, 170, 170]);
    } else if url.contains("idmsa.apple.com") {
        // Apple's sign-in: the mark, a heading, two fields, a button.
        rect(0., 0., 1., 0.06, [245, 245, 247]);
        rect(0.46, 0.14, 0.08, 0.1, [29, 29, 31]);
        rect(0.3, 0.3, 0.4, 0.04, [29, 29, 31]);
        rect(0.32, 0.4, 0.36, 0.07, [250, 250, 252]);
        rect(0.32, 0.4, 0.36, 0.003, [210, 210, 215]);
        rect(0.32, 0.467, 0.36, 0.003, [210, 210, 215]);
        rect(0.32, 0.5, 0.36, 0.07, [250, 250, 252]);
        rect(0.32, 0.567, 0.36, 0.003, [210, 210, 215]);
        rect(0.4, 0.62, 0.2, 0.06, [0, 113, 227]);
        rect(0.36, 0.75, 0.28, 0.02, [0, 102, 204]);
    } else if url.contains("appstoreconnect") {
        rect(0., 0., 1., 0.07, [245, 245, 247]);
        rect(0.03, 0.02, 0.18, 0.03, [30, 30, 30]);
        rect(0.05, 0.12, 0.12, 0.12, [0, 113, 227]);
        rect(0.2, 0.13, 0.4, 0.04, [30, 30, 30]);
        rect(0.2, 0.19, 0.25, 0.025, [134, 134, 139]);
        for i in 0..5 {
            rect(0.05, 0.32 + i as f32 * 0.1, 0.9, 0.07, if i % 2 == 0 { [245, 245, 247] } else { [255, 255, 255] });
            rect(0.07, 0.345 + i as f32 * 0.1, 0.3, 0.02, [60, 60, 60]);
        }
    } else if url.contains("cloudflare.com") {
        rect(0., 0., 1., 0.07, [255, 255, 255]);
        rect(0., 0.07, 1., 0.002, [222, 222, 222]);
        rect(0., 0.07, 0.2, 0.93, [248, 248, 248]);
        rect(0.02, 0.02, 0.12, 0.03, [243, 128, 32]);
        for i in 0..6 {
            rect(0.03, 0.12 + i as f32 * 0.06, 0.14, 0.025, [120, 120, 120]);
        }
        rect(0.25, 0.12, 0.4, 0.045, [30, 30, 30]);
        for i in 0..4 {
            rect(0.25, 0.22 + i as f32 * 0.12, 0.7, 0.09, [246, 246, 246]);
            rect(0.27, 0.25 + i as f32 * 0.12, 0.25, 0.025, [37, 99, 235]);
        }
    } else {
        rect(0., 0., 1., 0.06, [36, 41, 47]);
        rect(0.03, 0.015, 0.05, 0.03, [255, 255, 255]);
        rect(0.06, 0.1, 0.5, 0.04, [31, 35, 40]);
        for i in 0..7 {
            rect(0.06, 0.2 + i as f32 * 0.09, 0.88, 0.07, [246, 248, 250]);
            rect(0.08, 0.22 + i as f32 * 0.09, 0.45, 0.022, [9, 105, 218]);
        }
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg).ok()?;
    Some(out)
}
