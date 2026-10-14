//! Makes a scratch squad with projects but no members, for trying the board
//! against a real daemon (`EQUIPE_HOME` pointing at a scratch home):
//!
//! ```text
//! export EQUIPE_HOME=$(mktemp -d)
//! cargo run -p equipe-desktop --example seed_board -- demo /path/to/repo [/another/repo…]
//! ```
//! Tickets are then put into `<home>/squads/demo/squad.db` with `sqlite3` while the
//! daemon is stopped (see docs/DESKTOP.md, "Trying the board").

use std::path::PathBuf;

use equipe_core::squads;
use equipe_providers::credentials::equipe_home;

fn main() {
    let mut args = std::env::args().skip(1);
    let squad = args.next().unwrap_or_else(|| "demo".into());
    let folders: Vec<PathBuf> = args.map(|a| PathBuf::from(a).canonicalize().expect("a folder that exists")).collect();
    assert!(!folders.is_empty(), "at least one repository");
    let home = equipe_home();
    for (n, folder) in folders.iter().enumerate() {
        let done = if n == 0 && !squads::exists(&home, &squad) { squads::create(&home, &squad, folder) } else { squads::add_project(&home, &squad, folder) };
        done.expect("the squad's projects");
    }
    eprintln!("squad {squad} in {}", home.display());
}
