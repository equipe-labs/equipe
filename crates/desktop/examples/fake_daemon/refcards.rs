//! `FAKE_REFS=1`: the PM's conversation ends on the person asking for
//! components to reference and the PM showing six as a reference card
//! (`show_references`) — two from a component library, two private study
//! screens, two from a registry — with its searches before it. The pictures
//! are drawn here, wireframes of invented screens (boxes and bars: nothing of
//! any product's, no words, no logos), kept in the squad's picture store as
//! the daemon keeps fetched ones, and given by `read_image` through the
//! daemon's own code. The links are made-up pages; nothing is fetched.
//! `FAKE_REFS=fail`: one picture is gone from the store, for the card's
//! "couldn't load" with its retry.

use equipe_core::protocol::Entry;
use equipe_core::protocol::references::{RefCard, RefKind};
use equipe_core::protocol::rpc::{ImageRead, ReadImage};
use image::{Rgba, RgbaImage};

fn mode() -> Option<String> {
    std::env::var("FAKE_REFS").ok().filter(|v| !v.is_empty())
}

pub fn on() -> bool {
    mode().is_some()
}

pub const ASKED: &str = "I'm improving our pricing page. Can you show me components I could reference?";
pub const REPLY: &str = "The first two are closest to the three-column table you have; the study screens show how a plan picker reads on a phone. Heart the ones that feel right and I'll brief the designer.";

/// A picture `w` by `h`: a ground, then boxes, each `(x0, y0, x1, y1, colour)` in fractions.
fn draw(w: u32, h: u32, ground: [u8; 3], boxes: &[(f32, f32, f32, f32, [u8; 3])]) -> Vec<u8> {
    let mut img = RgbaImage::from_pixel(w, h, Rgba([ground[0], ground[1], ground[2], 255]));
    for &(x0, y0, x1, y1, c) in boxes {
        let (ax, ay, bx, by) = ((x0 * w as f32) as u32, (y0 * h as f32) as u32, (x1 * w as f32) as u32, (y1 * h as f32) as u32);
        for y in ay.min(h)..by.min(h) {
            for x in ax.min(w)..bx.min(w) {
                img.put_pixel(x, y, Rgba([c[0], c[1], c[2], 255]));
            }
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).expect("a PNG in memory");
    out.into_inner()
}

/// Three plans side by side, the middle one raised and lit.
fn pricing(dark: bool, accent: [u8; 3]) -> Vec<u8> {
    let (ground, card, line) = if dark { ([0x14, 0x16, 0x1B], [0x1F, 0x23, 0x2B], [0x3A, 0x40, 0x4C]) } else { ([0xF6, 0xF7, 0xF9], [0xFF, 0xFF, 0xFF], [0xD9, 0xDD, 0xE3]) };
    let mut boxes = vec![(0.32, 0.08, 0.68, 0.13, line)];
    for (i, x) in [0.06f32, 0.37, 0.68].into_iter().enumerate() {
        let lift = if i == 1 { 0.03 } else { 0.0 };
        boxes.push((x, 0.2 - lift, x + 0.26, 0.92 - lift, if i == 1 { accent } else { line }));
        boxes.push((x + 0.006, 0.2 - lift + 0.008, x + 0.254, 0.92 - lift - 0.008, card));
        boxes.push((x + 0.03, 0.27 - lift, x + 0.14, 0.31 - lift, line));
        boxes.push((x + 0.03, 0.35 - lift, x + 0.2, 0.43 - lift, if i == 1 { accent } else { line }));
        for k in 0..4 {
            let y = 0.5 + k as f32 * 0.07 - lift;
            boxes.push((x + 0.03, y, x + 0.22, y + 0.025, line));
        }
        boxes.push((x + 0.03, 0.8 - lift, x + 0.23, 0.86 - lift, if i == 1 { accent } else { line }));
    }
    draw(640, 480, ground, &boxes)
}

/// A comparison table: a header row, then rows of ticks.
fn table(accent: [u8; 3]) -> Vec<u8> {
    let line = [0xDE, 0xE2, 0xE8];
    let mut boxes = vec![(0.06, 0.08, 0.94, 0.2, [0xEE, 0xF1, 0xF5])];
    for c in 0..3 {
        let x = 0.42 + c as f32 * 0.18;
        boxes.push((x, 0.11, x + 0.12, 0.16, if c == 1 { accent } else { [0xB8, 0xBF, 0xC9] }));
    }
    for r in 0..7 {
        let y = 0.25 + r as f32 * 0.1;
        boxes.push((0.06, y + 0.08, 0.94, y + 0.083, line));
        boxes.push((0.08, y + 0.025, 0.32, y + 0.05, [0xC4, 0xCA, 0xD3]));
        for c in 0..3 {
            if (r + c) % 3 != 0 {
                let x = 0.47 + c as f32 * 0.18;
                boxes.push((x, y + 0.02, x + 0.025, y + 0.055, if c == 1 { accent } else { [0x8E, 0x97, 0xA4] }));
            }
        }
    }
    draw(640, 480, [0xFF, 0xFF, 0xFF], &boxes)
}

/// A phone's plan picker: stacked plans, one chosen, a button at the foot.
fn phone(accent: [u8; 3], tall: bool) -> Vec<u8> {
    let (w, h) = if tall { (390, 780) } else { (390, 640) };
    let mut boxes = vec![(0.08, 0.06, 0.6, 0.1, [0x2A, 0x2E, 0x36])];
    for i in 0..3 {
        let y = 0.16 + i as f32 * 0.2;
        boxes.push((0.08, y, 0.92, y + 0.16, if i == 1 { accent } else { [0xE1, 0xE4, 0xEA] }));
        boxes.push((0.09, y + 0.01, 0.91, y + 0.15, [0xFF, 0xFF, 0xFF]));
        boxes.push((0.14, y + 0.04, 0.5, y + 0.07, [0x3A, 0x3F, 0x48]));
        boxes.push((0.14, y + 0.09, 0.7, y + 0.11, [0xC9, 0xCE, 0xD6]));
        boxes.push((0.78, y + 0.05, 0.86, y + 0.09, if i == 1 { accent } else { [0xE1, 0xE4, 0xEA] }));
    }
    boxes.push((0.08, 0.84, 0.92, 0.92, accent));
    draw(w, h, [0xF7, 0xF8, 0xFA], &boxes)
}

/// A toggle between monthly and yearly over two plans.
fn toggle(accent: [u8; 3]) -> Vec<u8> {
    let boxes = [
        (0.36, 0.1, 0.64, 0.18, [0xE6, 0xE9, 0xEE]),
        (0.37, 0.115, 0.5, 0.165, [0xFF, 0xFF, 0xFF]),
        (0.12, 0.28, 0.48, 0.88, [0xE6, 0xE9, 0xEE]),
        (0.52, 0.28, 0.88, 0.88, accent),
        (0.53, 0.29, 0.87, 0.87, [0x1B, 0x1E, 0x25]),
        (0.16, 0.36, 0.3, 0.41, [0x9A, 0xA2, 0xAE]),
        (0.56, 0.36, 0.7, 0.41, [0xEE, 0xEE, 0xF2]),
        (0.16, 0.76, 0.44, 0.82, [0x9A, 0xA2, 0xAE]),
        (0.56, 0.76, 0.84, 0.82, accent),
    ];
    draw(640, 420, [0xFA, 0xFA, 0xFB], &boxes)
}

/// The cards, their pictures drawn and kept in the squad's store.
fn cards() -> Vec<RefCard> {
    let home = equipe_providers::credentials::equipe_home();
    let blue = [0x4F, 0x6B, 0xED];
    let green = [0x23, 0x9A, 0x6B];
    let pictures = [pricing(false, blue), pricing(true, [0x8B, 0x5C, 0xF6]), phone(green, false), phone(blue, true), table(blue), toggle([0xE0, 0x7A, 0x2E])];
    let mut paths: Vec<String> = pictures.iter().map(|bytes| equipe_core::references::store(&home, "demo", bytes).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()).collect();
    if mode().as_deref() == Some("fail") {
        let gone = std::path::Path::new(&paths[3]).with_file_name("ffffffffffffffff.png");
        paths[3] = gone.to_string_lossy().into_owned();
    }
    let card = |n: usize, title: &str, site: &str, link: &str, kind: RefKind, why: &str, take: &str, leave: &str| RefCard {
        picture: paths[n].clone(),
        title: title.into(),
        site: site.into(),
        link: link.into(),
        kind,
        why: why.into(),
        take: take.into(),
        leave: leave.into(),
        pattern: "pricing-table".into(),
    };
    vec![
        card(0, "Three-tier pricing", "21st.dev", "https://21st.dev/example/pricing-three-tier", RefKind::Component, "Same three plans as yours, with the middle one lifted so the eye lands there first.", "Raised recommended plan, one accent", "Gradient borders"),
        card(1, "Pricing with dark hero", "21st.dev", "https://21st.dev/example/pricing-dark", RefKind::Component, "Shows the plans holding up on a dark section, which your landing page uses.", "Price set large, features as quiet rows", "Glow behind the cards"),
        card(2, "Plan picker on a phone", "Mobbin", "https://mobbin.com/example/plan-picker", RefKind::Screen, "How a three-plan choice reads at 390 points without a table.", "Stacked plans, the choice outlined", "Their illustration header"),
        card(3, "Upgrade sheet", "Mobbin", "https://mobbin.com/example/upgrade-sheet", RefKind::Screen, "The upgrade moment inside the app, where most of your plan changes happen.", "One button pinned to the foot", ""),
        card(4, "Feature comparison table", "shadcn/ui", "https://ui.shadcn.com/docs/components/table", RefKind::Pattern, "For the long feature list under the plans: ticks in columns instead of bullets.", "Sticky plan header, ticks not words", "Zebra rows"),
        card(5, "Monthly / yearly toggle", "shadcn/ui", "https://ui.shadcn.com/example/billing-toggle", RefKind::Component, "", "Saving shown on the yearly side", "Toggle above the fold only"),
    ]
}

/// The PM's lines: the person's ask, its searches, the card, its reply.
pub fn history() -> Vec<Entry> {
    let head = equipe_core::i18n::core(equipe_core::i18n::Lang::En).fill("notice.references", &[("name", "PM".to_owned()), ("count", "6".to_owned())]);
    let note = format!("{head}\n{}", equipe_core::references::note_body(&cards()));
    let tool = |tool: &str, target: &str| Entry::Tool { tool: tool.into(), target: Some(target.into()), outcome: None, label: None, background: false };
    vec![
        Entry::User { text: ASKED.into(), kept: Vec::new() },
        tool("mcp__21st__search_components", "pricing table"),
        tool("mcp__mobbin__search_screens", "plan picker"),
        tool("mcp__equipe__show_references", "6 references"),
        Entry::Note { text: note, undo: None },
        Entry::Reply { text: REPLY.into() },
    ]
}

/// A picture of the card: from the squad's store, as the daemon gives it.
pub fn read(ask: &ReadImage) -> Result<ImageRead, String> {
    equipe_core::visuals::serve(&equipe_providers::credentials::equipe_home(), ask, None)
}
