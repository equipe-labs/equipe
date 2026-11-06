//! Where this machine's plans went this week, as the PM's `plan_use` reads it:
//! the engines' transcripts read into a ledger of its own (in a temporary
//! folder unless `--ledger-home` names one), attributed with the squads of an
//! equipe home (default: `~/.equipe`), printed for the person running it.
//! It only reads the transcripts and journals, and sends nothing anywhere.
//!
//! ```sh
//! cargo run --release -p equipe-core --example plan_use -- [--home <equipe home>] [--ledger-home <dir>] [--day]
//!     [--used <claude week %> --resets <unix>]
//! ```

use std::path::PathBuf;
use std::time::Instant;

use equipe_core::plan_use::{self, ledger, transcripts};
use equipe_core::protocol::plan_use::Span;
use equipe_core::protocol::{Engine, PlanWindow};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let home = value("--home").map(PathBuf::from).unwrap_or_else(equipe_providers::credentials::equipe_home);
    let ledger_home = value("--ledger-home").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join(format!("equipe-plan-use-{}", std::process::id())));
    std::fs::create_dir_all(&ledger_home).expect("a folder for the ledger");
    let span = if args.iter().any(|a| a == "--day") { Span::Day } else { Span::Week };
    let week = value("--used").zip(value("--resets")).and_then(|(u, r)| Some(PlanWindow { used_percent: u.parse().ok()?, resets_at: r.parse().ok()? }));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);

    let started = Instant::now();
    let ledger = ledger::Ledger::open(&plan_use::ledger_path(&ledger_home)).expect("the ledger");
    let scanned = transcripts::scan(&ledger, &transcripts::Roots::of_this_machine(), now, None);
    eprintln!("read {} files, {} MB, {} requests in {:.1}s", scanned.files, scanned.bytes / 1_000_000, scanned.requests, started.elapsed().as_secs_f64());
    let again = Instant::now();
    let rescanned = transcripts::scan(&ledger, &transcripts::Roots::of_this_machine(), now, None);
    eprintln!("a second pass read {} files, {} bytes in {:.2}s", rescanned.files, rescanned.bytes, again.elapsed().as_secs_f64());
    drop(ledger);

    // The ledger is read from `ledger_home`; the squads from `home`.
    let looked = Instant::now();
    let mut reports = Vec::new();
    for (engine, week) in [(Engine::Claude, week), (Engine::Codex, None)] {
        let since = now - 8 * 24 * 3600;
        let squads = plan_use::report::Squads::load(&home, since);
        let hours = ledger::Ledger::open(&plan_use::ledger_path(&ledger_home)).and_then(|l| l.hours(plan_use::report::engine_word(engine), since - 3600)).unwrap_or_default();
        let rows = plan_use::report::rows(engine, hours, &squads, since);
        reports.push(plan_use::report::build(&plan_use::report::Input { engine, span, now, week, rows: &rows, squads: &squads, reading: false }));
    }
    eprintln!("looked in {:.2}s", looked.elapsed().as_secs_f64());
    println!("{}", plan_use::answer(&reports));
    for r in &reports {
        let out = r.outside.as_ref().map_or(0, |o| o.share_bp);
        println!("\n{:?}: {} requests, {} tokens ({} output, {:.1}% cache reads); outside equipe {:.1}% of the load", r.engine, r.calls, r.tokens.total(), r.tokens.output, r.tokens.cache_read as f64 * 100.0 / r.tokens.total().max(1) as f64, f64::from(out) / 100.0);
    }
}
