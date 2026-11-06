//! `FAKE_HISTORY=<n>`: the PM's history as a long day leaves it, `n` lines
//! of it — for measuring how the app holds up (see `docs/PERF-2026-10.md`).
//! Turns over the three projects and the whole squad: the person's message
//! (now and then with screenshots), a group of calls with their output and
//! changes, a long reply in Markdown (paragraphs, lists, bold, ticket
//! links, a table or a code block every few), and news between turns.
//! The same `n` always gives the same history. `FAKE_LONG_REPLY=1` makes the
//! PM's streamed answer a long one of these too.

use equipe_core::protocol::{Entry, ToolOutcome};

const SITE: &str = "/tmp/fake/site";

const SENTENCES: [&str; 12] = [
    "What is left on the board is sorted by priority, and I'm handing the urgent things to members first.",
    "frontend is finishing the product detail page's image gallery and will ask for a review this afternoon.",
    "backend changed the orders API's pagination and kept accepting the old format, so existing clients don't break.",
    "The tests all pass, but the end of the checkout flow is worth one more check with a real card.",
    "All three onboarding drafts from the designer are in; only the second screen's copy needs a little polish.",
    "A deploy is hard to undo, so it always waits for your yes — I'll tell you when it's ready.",
    "The search improvement you mentioned yesterday needs the index rebuilt, so it takes a little longer.",
    "The logs show requests piling up in the early hours, so how about keeping the cache longer just then?",
    "The App Store review contact is still empty, so the submission is on hold — only you can fill that in.",
    "The internationalization work gathers every string in one place first, then adds the translations.",
    "In the performance run the first screen took 1.2 s on average, and while scrolling most frames drew within 8 ms.",
    "If you have questions left, leave them right here. I'll keep them in mind when picking what's next.",
];

const ASKS: [&str; 8] = [
    "Sum up what's on for today",
    "How far along is the product detail gallery?",
    "Does the orders API change affect the clients?",
    "Give the second onboarding screen the same tone",
    "Check whether errors went up since yesterday's deploy",
    "Sort out this week's priorities again",
    "This screen looks off, can you check it?",
    "Find out why search is slow",
];

const CODE: &str = "```rust\n/// The orders, a page at a time.\npub async fn orders(db: &Db, after: Option<OrderId>, limit: u32) -> Result<Page<Order>> {\n    let limit = limit.clamp(1, 100);\n    let rows = db.query(\"select * from orders where id > $1 order by id limit $2\", (after.unwrap_or_default(), limit + 1)).await?;\n    let more = rows.len() > limit as usize;\n    Ok(Page { items: rows.into_iter().take(limit as usize).collect(), more })\n}\n```";

const TSX: &str = "```tsx\nexport function Gallery({ images }: { images: string[] }) {\n  const [at, setAt] = useState(0);\n  return (\n    <div className=\"flex gap-2 overflow-x-auto\">\n      {images.map((src, i) => <img key={src} src={src} onClick={() => setAt(i)} className={i === at ? 'ring-2' : ''} />)}\n    </div>\n  );\n}\n```";

const TABLE: &str = "| Ticket | Project | Owner | Status |\n|---|---|---|---|\n| T-47 | web | frontend | Waiting for review |\n| T-60 | app | designer | In progress |\n| T-70 | api | backend | In progress |\n| T-117 | app | frontend | Your turn |";

/// A small deterministic generator, so a history is the same each run.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A long reply: three to six paragraphs, a list, and now and then a table or code.
fn reply(rng: &mut Rng, turn: u64) -> String {
    let mut out = String::new();
    let ticket = 40 + turn % 90;
    out.push_str(&format!("Sure — here it is, as of **T-{ticket}**.\n\n"));
    for _ in 0..3 + rng.below(4) {
        for _ in 0..2 + rng.below(3) {
            out.push_str(SENTENCES[rng.below(SENTENCES.len() as u64) as usize]);
            out.push(' ');
        }
        out.push_str("\n\n");
    }
    if rng.below(3) == 0 {
        out.push_str("## Where it stands\n\n");
    }
    for k in 0..2 + rng.below(4) {
        out.push_str(&format!("- **T-{}** {}\n", ticket + k, SENTENCES[rng.below(SENTENCES.len() as u64) as usize]));
    }
    out.push('\n');
    match rng.below(6) {
        0 => out.push_str(&format!("{TABLE}\n\n")),
        1 => out.push_str(&format!("{CODE}\n\n")),
        2 => out.push_str(&format!("{TSX}\n\n")),
        _ => {}
    }
    out.push_str("I'll let you know when it's done.");
    out
}

fn picture(n: u64) -> String {
    let path = format!("{SITE}/history-{n}.png");
    if !std::path::Path::new(&path).exists() {
        let (w, h) = (1600u32, 1000u32);
        let mut img = image::RgbaImage::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let t = x as f32 / w as f32;
            let s = y as f32 / h as f32;
            *px = image::Rgba([(60. + 150. * t) as u8, (90. + 100. * s) as u8, (40 * n % 255) as u8, 255]);
        }
        let _ = std::fs::create_dir_all(SITE);
        let _ = img.save(&path);
    }
    path
}

/// The PM's history, `n` lines of it.
pub fn history(n: usize) -> Vec<Entry> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut out = Vec::with_capacity(n + 16);
    let projects = [Some("app"), Some("web"), Some("api"), None];
    // `FAKE_HISTORY_PICTURES=<n>`: that many different screenshots, not six
    // over and over — every one a picture of its own, as a real day's are.
    let pictures = std::env::var("FAKE_HISTORY_PICTURES").ok().and_then(|v| v.parse::<u64>().ok()).filter(|&n| n > 0).unwrap_or(6);
    let mut turn = 0u64;
    while out.len() < n {
        if turn.is_multiple_of(6) {
            let project = projects[rng.below(4) as usize];
            out.push(Entry::Thread { project: project.map(str::to_owned) });
        }
        let mut said = ASKS[rng.below(ASKS.len() as u64) as usize].to_owned();
        if turn % 12 == 5 {
            for k in 0..1 + rng.below(3) {
                said.push_str(&format!(" [image: {}]", picture((turn + k) % pictures)));
            }
        }
        out.push(Entry::User { text: said });
        for k in 0..rng.below(9) {
            let file = format!("{SITE}/src/{}.rs", ["orders", "gallery", "search", "onboarding", "limit"][((turn + k) % 5) as usize]);
            let entry = match rng.below(4) {
                0 => Entry::Tool { tool: "Read".into(), target: Some(file), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(40), ..Default::default() })), label: None, background: false },
                1 => {
                    let output: String = (0..5 + rng.below(25)).map(|l| format!("test orders::page_{l} ... ok\n")).collect();
                    Entry::Tool { tool: "Bash".into(), target: Some("cargo test -p api".into()), outcome: Some(Box::new(ToolOutcome { ok: true, exit: Some(0), ms: Some(8_400), output, ..Default::default() })), label: None, background: false }
                }
                2 => {
                    let diff = format!("diff --git a/{file} b/{file}\n--- a/{file}\n+++ b/{file}\n@@ -10,3 +10,4 @@\n     let limit = limit.clamp(1, 100);\n-    let rows = db.query(SQL, (after, limit)).await?;\n+    let rows = db.query(SQL, (after, limit + 1)).await?;\n+    let more = rows.len() > limit as usize;\n");
                    Entry::Tool { tool: "Edit".into(), target: Some(file), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(12), diff: Some(diff), ..Default::default() })), label: None, background: false }
                }
                _ => Entry::Tool { tool: "Grep".into(), target: Some("fn orders".into()), outcome: Some(Box::new(ToolOutcome { ok: true, ms: Some(90), ..Default::default() })), label: None, background: false },
            };
            out.push(entry);
        }
        let mut text = reply(&mut rng, turn);
        // With `FAKE_HISTORY_PICTURES`, every other reply shows a screenshot
        // of its own too, embedded (`read_image`, as from the daemon).
        if pictures != 6 && turn.is_multiple_of(2) {
            text.push_str(&format!("\n\n![screen]({})", picture(1000 + turn % pictures)));
        }
        out.push(Entry::Reply { text });
        if rng.below(3) == 0 {
            for k in 0..1 + rng.below(3) {
                let t = 40 + (turn + k) % 90;
                out.push(Entry::Note { text: format!("frontend handed in T-{t} for your review · Product detail page image gallery.") });
            }
        }
        turn += 1;
    }
    out.truncate(n);
    out
}

/// The long reply `FAKE_LONG_REPLY=1` streams.
pub fn long_reply() -> String {
    let mut rng = Rng(0xDEAD_BEEF_1234_5678);
    let mut text = reply(&mut rng, 7);
    text.push_str("\n\n");
    text.push_str(&reply(&mut rng, 8));
    text
}

/// `read_image` for the history's screenshots (`FAKE_HISTORY_PICTURES`).
pub fn read(ask: &equipe_core::protocol::rpc::ReadImage) -> Result<equipe_core::protocol::rpc::ImageRead, String> {
    if !ask.path.starts_with(SITE) {
        return Err(format!("{} isn't a history picture", ask.path));
    }
    equipe_core::visuals::read(std::path::Path::new(&ask.path), &[std::path::PathBuf::from(SITE)], ask.max)
}
