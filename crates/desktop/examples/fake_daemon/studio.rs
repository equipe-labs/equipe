//! `FAKE_STUDIO=1`: the designer at work on T-205, the onboarding's welcome
//! screen, for the designer studio (`views/studio.rs`): its conversation —
//! the how-tos opened, the brief read, two references shot with what to
//! take from each, two directions proposed and critiqued by the other
//! engine, the person's pick, the screen built, a check that fails the
//! floor (the CTA's contrast) then passes, a hand-in refused for want of a
//! second look, a second look that asks for two fixes then passes — and
//! now in the shared browser. Its worktree's design folder holds what the
//! real tools leave there, written by the same code: the references, the
//! directions' log and pictures, the critique, the screen's check record
//! with its seven widths and the looks' verdicts. `design_studio` reads it
//! the daemon's way (`equipe_core::design::studio`); its pictures come
//! through `read_image`.
//!
//! - `waiting`: stopped at the directions, the person asked to pick one.
//! - `failing`: stopped at the check that fails the floor.
//! - `handed`: handed in, in review.
//! - `codex`: the same work by a Codex designer — its how-tos opened by
//!   reading their files, its commands as its shell ran them.

use std::path::{Path, PathBuf};

use equipe_core::design::directions::{self, Direction};
use equipe_core::design::lint::record::{self, Fix, Question as LookQuestion, Record, Verdict};
use equipe_core::design::lint::{self, Summary};
use equipe_core::design::second_look;
use equipe_core::protocol::decide::{Choice, Question, QuestionSet};
use equipe_core::protocol::rpc::{ImageRead, ReadImage};
use equipe_core::protocol::studio::{Studio, StudioAsk};
use equipe_core::protocol::engines::{ReviewWhy, Reviewed, Window};
use equipe_core::protocol::{Engine, Entry, Ticket, TicketStatus, ToolOutcome};
use image::{Rgba, RgbaImage};

pub const TICKET: u32 = 205;
pub const TITLE: &str = "Onboarding welcome screen, calmer";
const SCREEN: &str = "onboarding/welcome";
const WIDTHS: [u32; 7] = lint::WIDTHS;

pub fn mode() -> Option<String> {
    std::env::var("FAKE_STUDIO").ok().filter(|v| !v.is_empty() && v != "0")
}

pub fn on() -> bool {
    mode().is_some()
}

fn is(what: &str) -> bool {
    mode().as_deref() == Some(what)
}

pub fn codex() -> bool {
    is("codex")
}

/// At it right now (not resting, not waiting, not handed in).
pub fn working() -> bool {
    !is("waiting") && !is("handed")
}

/// The ticket's worktree, under the fake daemon's own home.
fn worktree() -> PathBuf {
    equipe_providers::credentials::equipe_home().join("studio").join(format!("designer-t{TICKET}"))
}

fn design() -> PathBuf {
    worktree().join("design")
}

fn unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// When the work began: 47 minutes before the daemon started.
fn start() -> u64 {
    static START: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *START.get_or_init(|| unix() - 47 * 60)
}

/// The ticket on the board.
pub fn ticket(make: impl Fn(u32, &str, &str, &str, &str, &str) -> Ticket) -> Option<Ticket> {
    let status = if is("waiting") {
        "waiting"
    } else if is("handed") {
        "review"
    } else {
        "in_progress"
    };
    let body = if qa() { "frontend's T-42 changes what users see: check its screens before the person reviews it." } else { "Make the first-run welcome calmer, with one clear next step.\n\n- [ ] The primary action is in the first screenful at every width\n- [ ] Light and dark" };
    let mut t = make(TICKET, status, "app", "designer", title(), body);
    t.worktree = Some(worktree());
    t.branch = Some(format!("equipe/designer-t{TICKET}"));
    if t.status == TicketStatus::Waiting {
        t.asks = vec!["Pick the welcome screen's direction".into()];
    }
    on().then_some(t)
}

// ── the conversation ──

fn tool(name: &str, target: &str, ok: bool, output: &str) -> Entry {
    Entry::Tool { tool: name.into(), target: Some(target.into()), outcome: Some(Box::new(ToolOutcome { ok, exit: Some(if ok { 0 } else { 1 }), ms: Some(900), output: output.into(), skipped: 0, diff: None })), label: None, background: false }
}

fn running(name: &str, target: &str) -> Entry {
    Entry::Tool { tool: name.into(), target: Some(target.into()), outcome: None, label: None, background: false }
}

fn path(rel: &str) -> String {
    worktree().join(rel).to_string_lossy().into_owned()
}

/// A how-to opened: Claude's Skill step, or a Codex member reading its file.
fn skill(name: &str) -> Entry {
    if codex() { tool("Read", &format!("{}/skills/{name}/SKILL.md", equipe_providers::credentials::equipe_home().display()), true, "") } else { tool("Skill", &format!("equipe:{name}"), true, "") }
}

fn failing() -> Summary {
    Summary {
        floor: (10, 11),
        finish: 1,
        widths: WIDTHS.to_vec(),
        failing: vec![("contrast".into(), "button.cta “Get started” is 2.47:1 (#8FB0E8 on #F6F5F1), under 4.5:1".into())],
        notes: vec![("touch-target".into(), "a.skip is 28×20".into())],
        screen: Some(SCREEN.into()),
        ..Summary::default()
    }
}

fn passing() -> Summary {
    Summary { floor: (11, 11), finish: 1, widths: WIDTHS.to_vec(), notes: vec![("touch-target".into(), "a.skip is 28×20".into())], screen: Some(SCREEN.into()), ..Summary::default() }
}

fn lint_run(s: &Summary) -> Entry {
    let out = format!("Drawing design/mockups/welcome.html at 360,390,768,1024,1280,1440,1920…\n{}", lint::mark_line(s));
    tool("Bash", &format!("equipe design lint design/mockups/welcome.html --render --screen {SCREEN}"), s.passes(), &out)
}

fn fixes() -> Verdict {
    Verdict {
        pass: false,
        summary: "Calm and clear at desktop widths; at 360 the illustration pushes the CTA to the fold's edge.".into(),
        fixes: vec![
            Fix { place: "Hero at 360·390".into(), what: "Shrink the illustration so “Get started” sits well inside the first screenful".into(), why: "It reads as below the fold".into() },
            Fix { place: "Skip link".into(), what: "Give it a 44 px target".into(), why: String::new() },
        ],
        questions: vec![LookQuestion { what: "How long the free trial is".into(), wording: "Start free".into() }],
        by: if codex() { "claude" } else { "codex" }.into(),
        widths: WIDTHS.to_vec(),
        reviewer: Some(reviewer(false)),
        ..Verdict::default()
    }
}

fn passed() -> Verdict {
    let fresh = reviewer(true);
    Verdict { pass: true, summary: "One clear next step at every width; the type keeps the warmth asked for.".into(), by: equipe_core::design::critique::member_engine_value(fresh.engine).into(), widths: WIDTHS.to_vec(), reviewer: Some(fresh), ..Verdict::default() }
}

/// Who reviewed: the other engine with room, or — its week nearly gone — a
/// fresh session of the designer's own.
fn reviewer(fresh: bool) -> Reviewed {
    let own = if codex() { Engine::Codex } else { Engine::Claude };
    let other = if codex() { Engine::Claude } else { Engine::Codex };
    if fresh {
        Reviewed { engine: own, member: Some(own), why: ReviewWhy::NearLimit { window: Window::Weekly, used: 92 } }
    } else {
        Reviewed { engine: other, member: Some(own), why: ReviewWhy::Room { used: Some(38) } }
    }
}

fn look_run(v: &Verdict) -> Entry {
    let out = format!("Fresh eyes on 7 picture(s) of {SCREEN}…\n{}{}", second_look::text(v), second_look::mark_line(SCREEN, v));
    tool("Bash", &format!("equipe design second-look {SCREEN} --criteria \"The primary action is in the first screenful at every width\""), v.pass, &out)
}

const REFUSED: &str = "Not yet — a screen you made goes to the person only checked and seen fresh:\n- onboarding/welcome has had no second look: `equipe design second-look onboarding/welcome`.\nThen hand in again. If you hand in again the same way, it goes in with these marked for the person as not cleared.";

pub fn history() -> Vec<Entry> {
    if qa() {
        return qa_history();
    }
    let reply = |text: &str| Entry::Reply { text: text.into() };
    let shoot = |url: &str, name: &str| tool("Bash", &format!("equipe shoot {url} --out design/references/{name}/shot.png"), true, &format!("kept design/references/{name}/shot.png"));
    let mut out = vec![
        Entry::Note { undo: None, text: format!("── T-{TICKET} · {TITLE} — on branch equipe/designer-t{TICKET}") },
        Entry::User { text: "Make the onboarding's welcome screen calmer, with one clear next step.".into(), kept: Vec::new() },
        reply("I'll read how we work and the brief first, then look for references."),
        skill("design-process"),
        tool("Read", &path("design/brand.md"), true, ""),
        tool("Read", &path("design/tokens.json"), true, ""),
        skill("ui-floor"),
        if codex() { shoot("https://linear.app", "linear") } else { tool("WebSearch", "calm product onboarding first screen", true, "") },
        shoot("https://linear.app/method", "linear"),
        tool("Write", &path("design/references/linear/notes.md"), true, ""),
        shoot("https://press.stripe.com", "stripe-press"),
        tool("Write", &path("design/references/stripe-press/notes.md"), true, ""),
        reply("Two references kept, each with what to take from it. Now two directions that differ in structure, not only colour."),
        tool("Bash", "equipe design direction propose \"Calm ledger\" --layout \"one column, text first\" --type \"Inter 17/28\" --palette \"warm neutrals, one blue\" --density airy --motion still --ticket T-205", true, "Recorded."),
        tool("Bash", "equipe design direction propose \"Bold poster\" --layout \"full-bleed poster\" --type \"display serif 64\" --palette \"ink on orange\" --density dense --motion \"slow drift\" --ticket T-205", true, "Recorded."),
        tool("Bash", "equipe design critique design/directions/calm-ledger/welcome.png design/directions/bold-poster/welcome.png --reference design/references/linear/shot.png", true, "The poster shouts; the ledger reads, but its CTA is timid."),
        tool("mcp__equipe__ask_decision", "Which direction for the welcome screen?", true, ""),
    ];
    if is("waiting") {
        out.push(reply("Two directions are on your card — the ledger is my pick, the poster the bolder one."));
        return out;
    }
    out.extend([
        Entry::User { text: "Calm ledger — but keep the warmth of the poster's type.".into(), kept: Vec::new() },
        tool("Bash", "equipe design direction choose \"Calm ledger\" --ticket T-205", true, "Recorded."),
        tool("Write", &path("design/mockups/welcome.html"), true, ""),
        tool("Write", &path("design/mockups/welcome.css"), true, ""),
        lint_run(&failing()),
    ]);
    if is("failing") {
        out.push(reply("The CTA fails the floor's contrast (2.47:1). Moving it to the accent token."));
        return out;
    }
    out.extend([
        reply("The CTA failed the floor's contrast (2.47:1); it is on the accent token now."),
        tool("Edit", &path("design/mockups/welcome.css"), true, ""),
        lint_run(&passing()),
        tool("mcp__equipe__submit_for_review", "Onboarding welcome screen", false, REFUSED),
        look_run(&fixes()),
        reply("Fresh eyes asked for two fixes: a smaller illustration at phone widths, and a bigger skip target."),
        tool("Edit", &path("design/mockups/welcome.html"), true, ""),
        lint_run(&passing()),
        look_run(&passed()),
    ]);
    if is("handed") {
        out.push(tool("mcp__equipe__submit_for_review", "Onboarding welcome screen", true, ""));
        out.push(reply("Handed in: floor 11/11; the second look asked for two fixes and passed after them."));
        return out;
    }
    out.push(reply("Checking the screen as the app serves it, in the shared browser."));
    out.push(running("mcp__browser__browser_navigate", "http://localhost:5173/welcome"));
    out
}

/// When each line was said: spread over the 47 minutes since it began, the last a moment ago.
pub fn times(lines: &[Entry]) -> Vec<Option<u64>> {
    let n = lines.len().max(1) as u64;
    let span = unix().saturating_sub(start()).saturating_sub(20);
    (0..lines.len() as u64).map(|i| Some(start() + span * i / n)).collect()
}

fn time_of(lines: &[Entry], at: usize) -> u64 {
    times(lines).get(at).copied().flatten().unwrap_or_else(unix)
}

/// `FAKE_STUDIO=qa_ask`: a question of the designer's on another ticket.
fn elsewhere_set(now: u64) -> QuestionSet {
    QuestionSet {
        id: 9206,
        session: 4,
        member: "designer".into(),
        ticket: Some(60),
        questions: vec![Question {
            question: "Which tone for the onboarding's first line?".into(),
            header: "Onboarding tone".into(),
            options: vec![Choice { label: "Warm (Recommended)".into(), ..Choice::default() }, Choice { label: "Plain".into(), ..Choice::default() }],
            multi_select: false,
            ..Question::default()
        }],
        asked_at: now.saturating_sub(5 * 60),
        review: false,
        retro: None,
    }
}

/// `FAKE_STUDIO=waiting`: the pick, waiting on the person.
pub fn question_set(now: u64) -> Option<QuestionSet> {
    if is("qa_ask") {
        return Some(elsewhere_set(now));
    }
    is("waiting").then(|| QuestionSet {
        id: 9205,
        session: 4,
        member: "designer".into(),
        ticket: Some(TICKET),
        questions: vec![Question {
            question: "Which direction for the welcome screen?".into(),
            header: "Pick a direction".into(),
            options: vec![
                Choice { label: "Calm ledger (Recommended)".into(), description: "One column, text first, warm neutrals and one blue".into(), images: vec![path("design/directions/calm-ledger/welcome.png")], ..Choice::default() },
                Choice { label: "Bold poster".into(), description: "Full-bleed poster, a display serif, ink on orange".into(), images: vec![path("design/directions/bold-poster/welcome.png")], ..Choice::default() },
            ],
            multi_select: false,
            ..Question::default()
        }],
        asked_at: now.saturating_sub(4 * 60),
        review: false,
        retro: None,
    })
}


// ── `FAKE_STUDIO=qa`: a design QA ticket, as real ones go ──

/// A design QA: the designer reviewing frontend's T-42 on the ticket's phone
/// — the change read, simulator screenshots taken and opened, the phone
/// looked at and recorded, fresh eyes asked, a report with its findings in
/// the hand-in folder. `qa_ask`: meanwhile it asks the person something on
/// another ticket, so the sidebar says "Your turn" — and the studio too.
pub fn qa() -> bool {
    is("qa") || is("qa_ask")
}

pub const QA_TITLE: &str = "Design QA: T-42 the checkout's buttons at the largest text size";

/// The ticket's title, as the mode has it.
pub fn title() -> &'static str {
    if qa() { QA_TITLE } else { TITLE }
}

fn scratch(name: &str) -> PathBuf {
    equipe_providers::credentials::equipe_home().join("studio/scratch/t205").join(name)
}

fn recordings() -> PathBuf {
    equipe_providers::credentials::equipe_home().join("studio/recordings").join(format!("T-{TICKET}"))
}

fn outputs() -> PathBuf {
    worktree().join("outputs").join(format!("T-{TICKET} Design QA"))
}

fn labelled(name: &str, target: &str, label: &str, background: bool) -> Entry {
    Entry::Tool { tool: name.into(), target: Some(target.into()), outcome: Some(Box::new(ToolOutcome { ok: true, exit: Some(0), ms: Some(1_200), output: String::new(), skipped: 0, diff: None })), label: Some(label.into()), background }
}

fn qa_history() -> Vec<Entry> {
    let reply = |text: &str| Entry::Reply { text: text.into() };
    let s = |n: &str| scratch(n).to_string_lossy().into_owned();
    let o = |n: &str| outputs().join(n).to_string_lossy().into_owned();
    let mut out = vec![
        Entry::Note { undo: None, text: format!("── T-{TICKET} · {QA_TITLE} — on branch equipe/designer-t{TICKET}-1a2b") },
        tool("Skill", "equipe:design-qa", true, ""),
        labelled("Bash", "cd /tmp/fake/web; git log --oneline main..HEAD; git diff main...HEAD --stat", "See what T-42 changed", false),
        tool("Read", "/tmp/fake/web/src/checkout/PayButton.tsx", true, ""),
        tool("mcp__equipe__estimate", "", true, ""),
        reply("Building T-42's branch for the simulator; meanwhile I read the button styles."),
        labelled("Bash", &format!("{} > {} 2>&1", s("run.sh"), s("run.log")), "Build T-42 and shoot it at the largest text size", true),
        tool("mcp__phone__start", "iPhone 17e", true, ""),
        tool("mcp__phone__look", "Checkout", true, ""),
        tool("Bash", &format!("xcrun simctl io booted screenshot {}", s("checkout-ax5.png")), true, ""),
        tool("Read", &s("checkout-ax5.png"), true, ""),
        reply("At the largest text size the Pay button's label wraps onto two lines and the card icon spills out of its circle."),
        tool("mcp__phone__record", "Checkout → Pay", true, ""),
        tool("mcp__phone__frames", "checkout.mp4", true, ""),
        labelled("Bash", &format!("cd {}; python3 sheet.py", s("")), "Make a before/after sheet", false),
        tool("Read", &s("before-after-ax5.png"), true, ""),
        tool("Agent", "Fresh-eyes look at the checkout buttons", true, ""),
        reply("Fresh eyes agree: the label wrap is the blocker; the icon is a polish note."),
        tool("Write", &o("report.md"), true, ""),
        tool("Read", &o("1-checkout-ax5-before-after.png"), true, ""),
    ];
    out.push(Entry::Tool { tool: "mcp__phone__look".into(), target: Some("Checkout, dark".into()), outcome: None, label: None, background: false });
    out
}

/// A phone screen with the checkout's Pay button: at the largest text size
/// the label wraps and the icon spills out of its circle.
fn phone(ax5: bool, dark: bool) -> RgbaImage {
    let (w, h) = (195, 422);
    let (bg, ink, soft) = if dark { ([0x15, 0x15, 0x13, 0xFF], [0xF3, 0xF0, 0xE9, 0xFF], [0x4A, 0x48, 0x43, 0xFF]) } else { (BG, INK, SOFT) };
    let mut img = RgbaImage::from_pixel(w, h, Rgba(bg));
    fill(&mut img, 12, 14, 60, 8, ink, 3);
    for i in 0..4u32 {
        fill(&mut img, 12, 50 + i * 46, w - 24, 34, soft, 8);
    }
    let big = if ax5 { 2 } else { 1 };
    // The Pay button: one line, or two when the text is at its largest.
    let bh = 40 * big;
    fill(&mut img, 12, h - 24 - bh, w - 24, bh, BLUE, 12);
    fill(&mut img, 52, h - 24 - bh + 14, 90, 10, [0xFF, 0xFF, 0xFF, 0xFF], 4);
    if ax5 {
        fill(&mut img, 52, h - 24 - bh + 34, 60, 10, [0xFF, 0xFF, 0xFF, 0xFF], 4);
    }
    // The card icon in its circle, spilling at the largest size.
    fill(&mut img, 22, h - 24 - bh + 10, 22, 22, [0xFF, 0xFF, 0xFF, 0xFF], 11);
    let g = if ax5 { 30 } else { 12 };
    fill(&mut img, 33 - g / 2, h - 24 - bh + 21 - g / 4, g, g / 2, ORANGE, 2);
    img
}

/// Two screens side by side, before and after.
fn sheet(a: &RgbaImage, b: &RgbaImage) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(a.width() + b.width() + 36, a.height().max(b.height()) + 24, Rgba([0xFF, 0xFF, 0xFF, 0xFF]));
    image::imageops::overlay(&mut img, a, 12, 12);
    image::imageops::overlay(&mut img, b, i64::from(a.width()) + 24, 12);
    img
}

fn seed_qa() {
    let lines = history();
    let at = |needle: &str| lines.iter().position(|e| matches!(e, Entry::Tool { target: Some(t), .. } if t.contains(needle))).map(|i| time_of(&lines, i)).unwrap_or_else(unix);
    // The squad's shared design folder, with an old check of another ticket's: never this one's.
    let _ = std::fs::write(design().join("tokens.json"), "{}");
    let old = Record { screen: "settings/notifications".into(), at: start() - 3 * 86_400, rendered: true, summary: passing(), ..Record::default() };
    let _ = record::write(&design(), &old);
    let shots = [
        (scratch("checkout-ax5.png"), phone(true, false), "checkout-ax5.png"),
        (scratch("checkout-ax5-dark.png"), phone(true, true), "booted screenshot"),
        (scratch("before-after-ax5.png"), sheet(&phone(false, false), &phone(true, false)), "sheet.py"),
        (outputs().join("1-checkout-ax5-before-after.png"), sheet(&phone(false, false), &phone(true, false)), "report.md"),
        (outputs().join("2-checkout-ax5-dark.png"), phone(true, true), "report.md"),
    ];
    for (path, img, step) in shots {
        save(&img, &path);
        touch(&path, at(step));
    }
    for n in 0..4u32 {
        let frame = recordings().join(format!("checkout/{:02}-{}.0s.png", n + 1, n));
        save(&phone(n >= 2, false), &frame);
        touch(&frame, at("checkout.mp4") - u64::from(3 - n));
    }
    let report = "# T-205 Design QA — T-42 checkout buttons at the largest text size\n\nT-42's branch on an iPhone 17e simulator, light and dark, default and largest text. Verdict: **change and again**.\n\n## Shot\n- Checkout, default and largest text, light and dark\n- The Pay flow, recorded\n\n## Blocking\n1. **The Pay button's label wraps onto two lines at the largest text size** — let the button grow, or shorten the label to \"Pay\".\n2. The card icon spills out of its 22pt circle at the largest size: scale it with the text.\n\n## Polish\n- The total's figures aren't tabular, so they jump as the amount changes.\n";
    let _ = std::fs::create_dir_all(outputs());
    let _ = std::fs::write(outputs().join("report.md"), report);
    touch(&outputs().join("report.md"), at("report.md"));
}

// ── the design folder ──

const BG: [u8; 4] = [0xF6, 0xF5, 0xF1, 0xFF];
const INK: [u8; 4] = [0x1D, 0x1C, 0x19, 0xFF];
const SOFT: [u8; 4] = [0xC9, 0xC6, 0xBE, 0xFF];
const BLUE: [u8; 4] = [0x21, 0x60, 0xAB, 0xFF];
const PALE: [u8; 4] = [0x8F, 0xB0, 0xE8, 0xFF];
const WARM: [u8; 4] = [0xE9, 0xDF, 0xCF, 0xFF];
const ORANGE: [u8; 4] = [0xF0, 0x6A, 0x2B, 0xFF];

fn fill(img: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, c: [u8; 4], r: u32) {
    if w == 0 || h == 0 {
        return;
    }
    let (iw, ih) = img.dimensions();
    let r = r.min((w - 1) / 2).min((h - 1) / 2);
    for py in y..(y + h).min(ih) {
        for px in x..(x + w).min(iw) {
            // Rounded corners: what lies outside a corner's circle stays as it was.
            let cx = px.clamp(x + r, x + w - 1 - r);
            let cy = py.clamp(y + r, y + h - 1 - r);
            let (dx, dy) = (px as f32 - cx as f32, py as f32 - cy as f32);
            if dx * dx + dy * dy <= (r * r) as f32 + 0.5 {
                img.put_pixel(px, py, Rgba(c));
            }
        }
    }
}

fn save(img: &RgbaImage, path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = img.save(path);
}

/// The welcome screen at `width`, drawn at half size: a top bar, the words,
/// the CTA, and an illustration beside them where there is room.
fn welcome(width: u32, cta: [u8; 4], dark: bool) -> RgbaImage {
    let h = match width {
        360 => 780,
        390 => 844,
        768 => 1024,
        1024 => 768,
        1280 => 800,
        1440 => 900,
        _ => 1080,
    };
    let (w, h) = (width / 2, h / 2);
    let (bg, ink, soft) = if dark { ([0x15, 0x15, 0x13, 0xFF], [0xF3, 0xF0, 0xE9, 0xFF], [0x4A, 0x48, 0x43, 0xFF]) } else { (BG, INK, SOFT) };
    let mut img = RgbaImage::from_pixel(w, h, Rgba(bg));
    let pad = (w / 14).max(12);
    // The top bar: a mark and a skip link.
    fill(&mut img, pad, pad, 18, 18, ink, 4);
    fill(&mut img, w - pad - 34, pad + 5, 34, 8, soft, 3);
    let wide = width >= 1024;
    let col = if wide { w * 46 / 100 } else { w - 2 * pad };
    let top = if wide { h / 4 } else { h * 2 / 5 };
    // The words: a heading of two lines, then three of body.
    fill(&mut img, pad, top, col * 9 / 10, (h / 22).max(10), ink, 3);
    fill(&mut img, pad, top + (h / 22).max(10) + 8, col * 6 / 10, (h / 22).max(10), ink, 3);
    let body = top + 2 * (h / 22).max(10) + 26;
    for (i, frac) in [95u32, 88, 60].iter().enumerate() {
        fill(&mut img, pad, body + i as u32 * 14, col * frac / 100, 6, soft, 3);
    }
    // The one next step.
    fill(&mut img, pad, body + 64, (col / 2).clamp(90, 160), 22, cta, 6);
    // The illustration: beside the words when wide, above them when not.
    if wide {
        fill(&mut img, w * 56 / 100, h / 5, w * 34 / 100, h * 3 / 5, WARM, 14);
        fill(&mut img, w * 64 / 100, h * 2 / 5, w * 18 / 100, h / 5, BLUE, 40);
    } else {
        fill(&mut img, pad, pad + 40, w - 2 * pad, top - pad - 60, WARM, 12);
        fill(&mut img, w / 2 - 30, pad + 40 + (top - pad - 60) / 3, 60, 60, BLUE, 30);
    }
    img
}

/// A direction's welcome, as proposed: the ledger or the poster.
fn direction(poster: bool) -> RgbaImage {
    let (w, h) = (640, 400);
    if !poster {
        return welcome(1280, BLUE, false);
    }
    let mut img = RgbaImage::from_pixel(w, h, Rgba(ORANGE));
    fill(&mut img, 40, 60, 460, 70, INK, 4);
    fill(&mut img, 40, 150, 380, 70, INK, 4);
    fill(&mut img, 40, 300, 150, 34, INK, 17);
    fill(&mut img, 520, 40, 80, 80, [0xFF, 0xFF, 0xFF, 0xFF], 40);
    img
}

/// A reference's shot: a quiet dark product page, or a book's spread.
fn reference(book: bool) -> RgbaImage {
    let (w, h) = (640, 400);
    if book {
        let mut img = RgbaImage::from_pixel(w, h, Rgba([0xEF, 0xE8, 0xDA, 0xFF]));
        fill(&mut img, 60, 50, 240, 300, [0x2B, 0x3A, 0x55, 0xFF], 4);
        fill(&mut img, 340, 80, 220, 18, INK, 2);
        for i in 0..7 {
            fill(&mut img, 340, 120 + i * 18, 220 - (i % 3) * 30, 6, SOFT, 2);
        }
        return img;
    }
    let mut img = RgbaImage::from_pixel(w, h, Rgba([0x0F, 0x10, 0x14, 0xFF]));
    fill(&mut img, 40, 30, 20, 20, [0xE6, 0xE6, 0xF0, 0xFF], 5);
    fill(&mut img, 40, 120, 360, 30, [0xF2, 0xF2, 0xF7, 0xFF], 4);
    fill(&mut img, 40, 165, 280, 10, [0x6B, 0x6E, 0x7B, 0xFF], 3);
    fill(&mut img, 40, 210, 110, 28, [0xE6, 0xE6, 0xF0, 0xFF], 6);
    fill(&mut img, 420, 90, 180, 240, [0x1C, 0x1E, 0x26, 0xFF], 10);
    img
}

/// Write the folder as the real tools leave it.
pub fn seed() {
    if !on() {
        return;
    }
    let dir = design();
    let _ = std::fs::remove_dir_all(equipe_providers::credentials::equipe_home().join("studio"));
    let _ = std::fs::create_dir_all(&dir);
    if qa() {
        return seed_qa();
    }
    let _ = std::fs::write(dir.join("tokens.json"), r##"{ "color": { "$type": "color", "bg": { "$value": "#F6F5F1" }, "ink": { "$value": "#1D1C19" }, "accent": { "$value": "#2160AB" } } }"##);
    let _ = std::fs::write(dir.join("brand.md"), "# Brief\n\nReference: Linear's method pages — the quiet density.\nLocked: the logo.\nThesis: calm, one next step.\n");
    let lines = history();
    // The references, each with its shot and what to take from it.
    save(&reference(false), &dir.join("references/linear/shot.png"));
    let _ = std::fs::write(dir.join("references/linear/notes.md"), "- https://linear.app/method\nTake: the quiet density and one accent at a time.\nLeave: the dark-only look.\n");
    save(&reference(true), &dir.join("references/stripe-press/shot.png"));
    let _ = std::fs::write(dir.join("references/stripe-press/notes.md"), "- https://press.stripe.com\nTake: the warmth of book typography.\nLeave: the long pages.\n");
    // The two directions, and the pick once made.
    let at = |needle: &str| lines.iter().position(|e| matches!(e, Entry::Tool { target: Some(t), .. } if t.contains(needle))).map(|i| time_of(&lines, i)).unwrap_or_else(unix);
    let chosen = !is("waiting");
    let d = |name: &str, layout: &str, type_: &str, palette: &str, density: &str, motion: &str, when: u64, chosen: bool| Direction {
        name: name.into(),
        ticket: Some(format!("T-{TICKET}")),
        at: when,
        chosen,
        layout: layout.into(),
        type_: type_.into(),
        palette: palette.into(),
        density: density.into(),
        motion: motion.into(),
        references: Vec::new(),
    };
    let _ = directions::write(
        &dir,
        &[
            d("Calm ledger", "one column, text first", "Inter 17/28", "warm neutrals, one blue", "airy", "still", at("propose \"Calm ledger\""), false),
            d("Bold poster", "full-bleed poster", "display serif 64", "ink on orange", "dense", "slow drift", at("propose \"Bold poster\""), false),
            d("Calm ledger", "one column, text first", "Inter 17/28", "warm neutrals, one blue", "airy", "still", if chosen { at("choose \"Calm ledger\"") } else { 0 }, chosen),
        ][..if chosen { 3 } else { 2 }],
    );
    save(&direction(false), &dir.join("directions/calm-ledger/welcome.png"));
    save(&direction(true), &dir.join("directions/bold-poster/welcome.png"));
    // Each file as old as the step that made it.
    for (file, step) in [
        ("references/linear/shot.png", "linear/shot.png"),
        ("references/linear/notes.md", "linear/notes.md"),
        ("references/stripe-press/shot.png", "stripe-press/shot.png"),
        ("references/stripe-press/notes.md", "stripe-press/notes.md"),
        ("directions/calm-ledger/welcome.png", "propose \"Calm ledger\""),
        ("directions/bold-poster/welcome.png", "propose \"Bold poster\""),
    ] {
        touch(&dir.join(file), at(step));
    }
    let _ = std::fs::create_dir_all(dir.join("critiques"));
    let critic = equipe_core::design::reviewer::mark(&reviewer(false));
    let _ = std::fs::write(dir.join(format!("critiques/{}.md", at("design critique"))), critic + "\n## The whole\nThe poster shouts and could be any launch page; the ledger reads at a glance.\n\n## Details\n- Ledger: the CTA is timid next to the heading.\n- Poster: the serif's warmth is worth keeping.\n");
    if is("waiting") {
        return;
    }
    // The screen's check: failing, or passed with both looks.
    let pictures: Vec<String> = WIDTHS
        .iter()
        .map(|w| {
            let rel = format!("design/checks/{SCREEN}/{w}.png");
            save(&welcome(*w, if is("failing") { PALE } else { BLUE }, false), &worktree().join(&rel));
            rel
        })
        .collect();
    let checks: Vec<usize> = lines.iter().enumerate().filter(|(_, e)| matches!(e, Entry::Tool { target: Some(t), .. } if t.contains("design lint"))).map(|(i, _)| i).collect();
    let looks: Vec<usize> = lines.iter().enumerate().filter(|(_, e)| matches!(e, Entry::Tool { target: Some(t), .. } if t.contains("second-look"))).map(|(i, _)| i).collect();
    let summary = if is("failing") { failing() } else { passing() };
    let mut rec = Record { screen: SCREEN.into(), at: checks.last().map_or_else(unix, |i| time_of(&lines, *i)), rendered: true, summary, pictures, new: true, ..Record::default() };
    if !is("failing") {
        rec.first = Some(Verdict { pass: true, first: true, at: rec.at.saturating_sub(300), summary: "The first screenful leads with the one next step.".into(), by: passed().by, ..Verdict::default() });
        rec.verdict = Some(Verdict { at: looks.last().map_or_else(unix, |i| time_of(&lines, *i)), ..passed() });
    }
    let _ = record::write(&dir, &rec);
    // A mockup kept beside it, made before the check.
    save(&welcome(1280, BLUE, true), &dir.join("mockups/welcome-dark.png"));
    touch(&dir.join("mockups/welcome-dark.png"), rec.at.saturating_sub(600));
}

/// `path` last changed at `at` (unix seconds).
fn touch(path: &Path, at: u64) {
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(at);
    let _ = std::fs::File::options().write(true).open(path).and_then(|f| f.set_modified(when));
}

/// `design_studio`: the work gathered as the daemon gathers it — its
/// design folder, the phone's recordings, its hand-in folder, what its
/// conversation opened.
pub fn answer(ask: &StudioAsk) -> Result<Studio, String> {
    if !on() || ask.ticket != TICKET {
        return Ok(Studio { ticket: ask.ticket, ..Studio::default() });
    }
    let entries = history();
    let lines = entries.iter().cloned().zip(times(&entries)).collect();
    Ok(equipe_core::design::studio::gather(&equipe_core::design::studio::Gather {
        ticket: ask.ticket,
        design: Some(design()),
        worktree: Some(worktree()),
        recordings: Some(recordings()),
        images: Vec::new(),
        cwd: Some(worktree()),
        lines,
        also: Vec::new(),
    }))
}

/// A picture of the folder's, checked as the daemon checks one.
pub fn read_image(ask: &ReadImage) -> Result<ImageRead, String> {
    let root = std::fs::canonicalize(equipe_providers::credentials::equipe_home().join("studio")).map_err(|e| e.to_string())?;
    if !on() || !Path::new(&ask.path).starts_with(&root) {
        return Err(format!("{} isn't one of the studio's", ask.path));
    }
    equipe_core::visuals::read(Path::new(&ask.path), &[root], ask.max)
}
