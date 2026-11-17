//! A design session, as the daemon and a designer would run one: started
//! (`Command::StartDesignSession`) — the kickoff to the designer (4), which
//! answers with a first rough take shown live at a sample page
//! (`http://localhost:5173/pricing`, drawn by `browser.rs` at the width the
//! screen asks for: run with `FAKE_BROWSER=1`) and one focused question — and
//! a message with pins answered by marking each done, the page shown again.
//! Nothing real is opened: the page is blocks drawn in the fake browser.

use std::time::Duration;

use equipe_core::design::session::{self, Marker};
use equipe_core::protocol::{Command, Event, ToolOutcome, TurnOutcome};
use tokio::sync::broadcast;

/// The designer's conversation.
const DESIGNER: u64 = 4;
/// The sample page the designer shows.
pub const PAGE: &str = "http://localhost:5173/pricing";

/// One step of the designer's turn: a command and what it printed.
fn ran(command: &str, output: String) -> [Event; 2] {
    [
        Event::ToolUse { session: DESIGNER, tool: "Bash".into(), target: Some(command.into()), label: None, background: false },
        Event::ToolResult { session: DESIGNER, tool: "Bash".into(), target: Some(command.into()), outcome: ToolOutcome { ok: true, output, ..ToolOutcome::default() } },
    ]
}

async fn reply(events: &broadcast::Sender<Event>, steps: Vec<Event>, words: &str) {
    for step in steps {
        let _ = events.send(step);
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    let _ = events.send(Event::TextDelta { session: DESIGNER, text: words.into() });
    let usage = equipe_core::protocol::Usage { input_tokens: Some(9_400), output_tokens: Some(1_850), ..Default::default() };
    let _ = events.send(Event::TurnEnded { session: DESIGNER, outcome: TurnOutcome::Completed, usage: Some(usage) });
}

/// The taste tuner's set in the design folder `dir`: a plain screen and, for
/// each axis, one towards each end — drawn in blocks, each holding the other
/// axes — with `tuner/set.json` placing them, as a designer would prepare it.
pub fn seed_tuner(dir: &std::path::Path) {
    use equipe_core::protocol::design::TasteAxis;
    let pics = dir.join("references/tuner");
    let _ = std::fs::create_dir_all(&pics);
    let mut items = vec![("plain".to_owned(), "Sample — plain".to_owned(), None)];
    for axis in TasteAxis::ALL {
        for (end, v) in [("lo", -70i32), ("hi", 70)] {
            let name = format!("{}-{end}", axis.key());
            let words = if v < 0 { axis.ends().0 } else { axis.ends().1 };
            items.push((name, format!("Sample — {words}"), Some((axis, v))));
        }
    }
    let mut set = Vec::new();
    for (name, source, axis) in &items {
        let at = |a: TasteAxis| axis.filter(|(x, _)| *x == a).map_or(0, |(_, v)| v);
        let (w, h) = (480u32, 320u32);
        let dark = at(TasteAxis::Contrast) > 0;
        let bg = if dark { [28, 30, 36] } else { [250, 248, 244] };
        let ink = if dark { [235, 235, 240] } else { [36, 36, 40] };
        let accent = if at(TasteAxis::Color) > 0 { [232, 84, 60] } else if at(TasteAxis::Color) < 0 { [120, 120, 124] } else { [40, 90, 170] };
        let mut img = image::RgbImage::from_pixel(w, h, image::Rgb(bg));
        let mut rect = |x: f32, y: f32, rw: f32, rh: f32, c: [u8; 3], round: bool| {
            let (x0, y0, x1, y1) = ((x * w as f32) as i32, (y * h as f32) as i32, ((x + rw) * w as f32) as i32, ((y + rh) * h as f32) as i32);
            let r = if round { ((y1 - y0).min(x1 - x0) / 3).max(0) } else { 0 };
            for yy in y0.max(0)..y1.min(h as i32) {
                for xx in x0.max(0)..x1.min(w as i32) {
                    let (dx, dy) = ((x0 + r - xx).max(xx - (x1 - 1 - r)).max(0), (y0 + r - yy).max(yy - (y1 - 1 - r)).max(0));
                    if dx * dx + dy * dy <= r * r {
                        img.put_pixel(xx as u32, yy as u32, image::Rgb(c));
                    }
                }
            }
        };
        let round = at(TasteAxis::Ornament) > 0;
        let head = 0.07 + 0.05 * (at(TasteAxis::TypeScale) as f32 / 70.);
        let shift = if at(TasteAxis::Layout) > 0 { 0.18 } else { 0. };
        rect(0.06, 0.06, 0.2, 0.05, ink, false);
        rect(0.06 + shift, 0.18, 0.55, head, ink, false);
        let rows = if at(TasteAxis::Density) > 0 { 6 } else if at(TasteAxis::Density) < 0 { 2 } else { 3 };
        let gap = 0.5 / rows as f32;
        for r in 0..rows {
            let y = 0.18 + head + 0.06 + r as f32 * gap;
            rect(0.06, y, 0.88 - shift * (r % 2) as f32, gap * 0.6, if dark { [48, 50, 58] } else { [236, 233, 226] }, round);
        }
        rect(0.06 + shift, 0.86, 0.22, 0.08, accent, round);
        if round {
            rect(0.8, 0.06, 0.12, 0.12, accent, true);
        }
        let file = pics.join(format!("{name}.png"));
        let _ = img.save(&file);
        let axes: serde_json::Map<String, serde_json::Value> = axis.iter().map(|(a, v)| (a.key().to_owned(), serde_json::json!(v))).collect();
        set.push(serde_json::json!({ "picture": format!("references/tuner/{name}.png"), "source": source, "axes": axes }));
    }
    let _ = std::fs::create_dir_all(dir.join("tuner"));
    let _ = std::fs::write(dir.join(equipe_core::taste::tuner::SET_FILE), serde_json::to_string_pretty(&serde_json::json!({ "items": set })).unwrap_or_default());
}

/// `StartDesignSession` and a message with pins to the designer. `false`: not one.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    match command {
        Command::StartDesignSession { project, page, from, .. } => {
            let page = equipe_core::protocol::design_session::page_name(page).unwrap_or_else(|| "Pricing".into());
            let _ = events.send(Event::DesignSessionStarted { squad: "demo".into(), designer: DESIGNER, project: project.clone(), page: page.clone(), from: from.clone() });
            let text = session::kickoff(&page, project.as_deref(), from, None, &format!("Let's work on {page} together, in short rounds."));
            let events = events.clone();
            tokio::spawn(async move {
                let _ = events.send(Event::UserMessage { session: DESIGNER, text, kept: vec![] });
                tokio::time::sleep(Duration::from_millis(200)).await;
                let steps: Vec<Event> = [
                    ran("equipe design refs search lapa pricing", "Pricing pages: Product A (two plans, quiet), Product B (comparison table), Product C (one hero plan).".into()),
                    ran("equipe design critique design/sessions/pricing/r1-1440.png --goal \"a calm pricing page\"", "Critique kept: design/critiques/r1.md — the plan cards compete with the hero; the CTA is clear.".into()),
                    ran("equipe design session show design/sessions/pricing/index.html", format!("Live beside the session: {PAGE}.\n{}", Marker::Show(PAGE.into()).line())),
                ]
                .into_iter()
                .flatten()
                .collect();
                reply(&events, steps, "First rough take is live: three references (A's calm cards, B's table, C's single hero), one take from A. The critic said the cards compete with the hero, so I quietened them.\n\n**Why:** calm and readable at a glance. **Pros:** one clear CTA; plans compare in one row. **Cons:** the table hides details on phones.\n\nOne question: should the plans sit **side by side** (fast to compare) or **stacked** (more room for each)?").await;
            });
            true
        }
        Command::Submit { session: DESIGNER, text, .. } if text.trim_start().starts_with(session::PINS_OPEN) => {
            let (pins, _) = session::split_pins(text);
            let text = text.clone();
            let events = events.clone();
            tokio::spawn(async move {
                let _ = events.send(Event::UserMessage { session: DESIGNER, text, kept: vec![] });
                tokio::time::sleep(Duration::from_millis(200)).await;
                let mut steps: Vec<Event> = Vec::new();
                for (n, note) in &pins {
                    let what = format!("Addressed: {note}");
                    steps.extend(ran(&format!("equipe design session done {n} \"{what}\""), format!("Pin {n} marked done.\n{}", Marker::Done { n: *n, what: what.clone() }.line())));
                }
                steps.extend(ran("equipe design session show design/sessions/pricing/index.html", Marker::Show(PAGE.into()).line()));
                reply(&events, steps, "Done — each pin is ticked with what changed, and the page is reloaded. Tighter or airier spacing between the plans?").await;
            });
            true
        }
        _ => false,
    }
}
