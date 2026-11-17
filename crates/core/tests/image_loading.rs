//! How fast a design screen's pictures come on a squad with many of them —
//! a measurement, not a check: run it by hand with
//! `cargo test -p equipe-core --test image_loading -- --ignored --nocapture`.
//!
//! The squad is made to look like a busy real one, with synthetic pictures:
//! a long board (most tickets done, many naming pictures), a couple of dozen
//! tickets at work or in review whose worktrees' design folders hold a few
//! thousand pictures each, and one ticket's design results — sixty renders
//! of the sizes designers make (most a phone or desktop screen, some whole
//! long pages). A design screen asks for every thumbnail at once, each on a
//! thread of its own, as the daemon's gateway answers them.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use equipe_core::protocol::rpc::ReadImage;
use equipe_core::protocol::Ticket;
use equipe_core::visuals;

const SQUAD: &str = "acme";
/// Tickets done, each naming pictures, as a long board's are.
const DONE: usize = 1200;
/// Tickets at work or in review, each with a worktree and a design folder.
const ACTIVE: usize = 23;
/// Pictures in each of those design folders.
const PER_FOLDER: usize = 2400;
/// The design results a screen shows at once.
const RESULTS: usize = 60;
/// Of those, whole long pages.
const TALL: usize = 8;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("equipe-imgload-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A render-like picture: soft gradients with a little texture, so it
/// compresses as a real screen does (not to nothing).
fn render(path: &Path, width: u32, height: u32, seed: u32) {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    let img = image::RgbImage::from_fn(width, height, |x, y| {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (state >> 28) as u8;
        let band = ((y / 40) % 7) as u8 * 9;
        image::Rgb([(x * 255 / width) as u8 ^ noise, 200u8.wrapping_sub(band), ((y * 255 / height) as u8).wrapping_add(noise)])
    });
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    img.save(path).unwrap();
}

/// The smallest PNG there is: a design folder's many small pictures.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15,
    0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00,
    0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn ticket(status: &str) -> Ticket {
    serde_json::from_value(serde_json::json!({ "id": 0, "title": "Work", "body": "", "status": status, "kind": "task", "depends_on": [], "owner": "designer", "created_by": "pm" })).unwrap()
}

struct Squad {
    home: PathBuf,
    results: Vec<String>,
    stale: Vec<String>,
}

fn squad() -> Squad {
    let home = scratch("home");
    let journal = equipe_core::journal::Journal::for_squad(&home, SQUAD).unwrap();
    // A long board: done tickets whose hand-ins named pictures, their worktrees long gone.
    for n in 0..DONE {
        let id = journal.create_ticket(&ticket("done")).unwrap();
        let mut t = journal.ticket(id).unwrap().unwrap();
        t.images = (0..3).map(|k| home.join(format!("worktrees/{SQUAD}.home/t{n}/outputs/shot-{k}.png")).to_string_lossy().into_owned()).collect();
        journal.save_ticket(&t).unwrap();
    }
    // Tickets at work and in review, each design folder full of pictures.
    let mut first = None;
    for n in 0..ACTIVE {
        let id = journal.create_ticket(&ticket(if n % 3 == 0 { "review" } else { "in_progress" })).unwrap();
        let worktree = home.join(format!("worktrees/{SQUAD}.home/active-{n}"));
        let design = worktree.join("design");
        for k in 0..PER_FOLDER {
            let dir = design.join(format!("checks/screen-{}", k / 40));
            if k % 40 == 0 {
                std::fs::create_dir_all(&dir).unwrap();
            }
            std::fs::write(dir.join(format!("{k}.png")), TINY_PNG).unwrap();
        }
        journal.set_ticket_work(id, &equipe_core::journal::TicketWork { worktree: Some(worktree.clone()), ..Default::default() }).unwrap();
        first.get_or_insert((id, design));
    }
    let (id, design) = first.unwrap();
    // One ticket's design results: screens, and a few whole long pages.
    let results: Vec<String> = (0..RESULTS)
        .map(|n| {
            let path = design.join(format!("directions/results/option-{n}.png"));
            if n < TALL { render(&path, 1280, 6000, n as u32) } else { render(&path, 768, 900, n as u32) }
            path.to_string_lossy().into_owned()
        })
        .collect();
    // Pictures its hand-in named that are gone since: asked for all the same.
    let stale: Vec<String> = (0..5).map(|n| design.join(format!("gone/old-{n}.png")).to_string_lossy().into_owned()).collect();
    let mut t = journal.ticket(id).unwrap().unwrap();
    t.images = stale.clone();
    journal.save_ticket(&t).unwrap();
    Squad { home, results, stale }
}

/// Each of `paths` asked for at `side` at once, a thread each: how long each
/// took from the start, in milliseconds, sorted, and how many came.
fn burst(home: &Path, paths: &[String], side: u32) -> (Vec<u128>, usize) {
    let barrier = Arc::new(Barrier::new(paths.len()));
    let start = Instant::now();
    let threads: Vec<_> = paths
        .iter()
        .map(|path| {
            let (home, barrier, ask) = (home.to_owned(), barrier.clone(), ReadImage { squad: SQUAD.into(), path: path.clone(), max: Some(side) });
            std::thread::spawn(move || {
                barrier.wait();
                let ok = visuals::serve(&home, &ask, None).is_ok();
                (start.elapsed().as_millis(), ok)
            })
        })
        .collect();
    let mut took: Vec<(u128, bool)> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    took.sort();
    let came = took.iter().filter(|(_, ok)| *ok).count();
    (took.into_iter().map(|(ms, _)| ms).collect(), came)
}

fn summary(what: &str, (took, came): &(Vec<u128>, usize)) {
    let at = |q: f64| took[((took.len() as f64 - 1.) * q).round() as usize];
    eprintln!("image-loading: {what}: {came}/{} came — first {} ms, p50 {} ms, p95 {} ms, last {} ms", took.len(), at(0.), at(0.5), at(0.95), at(1.));
}

#[test]
#[ignore = "a measurement: run with --ignored --nocapture"]
fn how_fast_a_design_screens_pictures_come_on_a_squad_with_many() {
    let made = Instant::now();
    let squad = squad();
    eprintln!(
        "image-loading: a squad of {DONE} done tickets, {ACTIVE} at work with {PER_FOLDER} pictures in each design folder, {RESULTS} design results ({TALL} long pages) — made in {} ms",
        made.elapsed().as_millis()
    );
    let mut looks: Vec<u128> = (0..3)
        .map(|_| {
            let at = Instant::now();
            let shown = visuals::shown(&squad.home, SQUAD);
            assert!(!shown.references.is_empty());
            at.elapsed().as_millis()
        })
        .collect();
    looks.sort();
    eprintln!("image-loading: working out what the squad shows: {} ms (median of 3)", looks[1]);

    let side = visuals::thumbs::THUMB_SIDE;
    let cold = burst(&squad.home, &squad.results, side);
    summary("the design results' thumbnails, none made yet", &cold);
    assert_eq!(cold.1, RESULTS, "every result shows");
    summary("the same, made already", &burst(&squad.home, &squad.results, side));
    std::thread::sleep(Duration::from_millis(5_200));
    summary("the same, made already, 5 s later", &burst(&squad.home, &squad.results, side));
    let mut mixed = squad.results.clone();
    mixed.extend(squad.stale.iter().cloned());
    std::thread::sleep(Duration::from_millis(5_200));
    summary("the same with 5 pictures gone since, 5 s later", &burst(&squad.home, &mixed, side));
    let inline: Vec<String> = squad.results.iter().take(12).cloned().collect();
    summary("12 of them side by side (1280), none made yet", &burst(&squad.home, &inline, 1280));
    let _ = std::fs::remove_dir_all(&squad.home);
}
