//! Pictures kept past the folder they were saved in.
//!
//! A ticket's hand-in names pictures by their paths, and so does a waiting
//! decision. Many of those paths are inside the ticket's worktree (a
//! `handin-shots/` folder, a render beside the code) or a temp folder, and
//! are never committed: once the work lands, its worktree goes, and git and
//! the capture store (`crate::design::captures`) have nothing of them. So a
//! copy of each such picture is kept in the squad's own folder, beside its
//! journal, the moment it is named:
//!
//! - a ticket's: `<squad>/visuals/kept/tickets/T-<n>/<hash>.<ext>`
//! - a waiting decision's: `<squad>/visuals/kept/decisions/<hash>.<ext>`
//!
//! `<hash>` is of the path the picture was named by, so the path a ticket or
//! a decision recorded finds its copy with nothing else to look up, and the
//! journal's paths never change. Only a picture inside a worktree or a temp
//! folder is copied (links resolved): one in the squad's folder or a
//! project's checkout stays where it is. A ticket's copies stay while it is
//! open and for [`days`] after it closed ([`sweep`]); a decision's, while it
//! waits and for as long after.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::protocol::TicketId;

/// The most a ticket's copies may weigh: pictures past it aren't kept.
pub const TICKET_MAX: u64 = 200 << 20;

/// How many days a closed ticket's copies, and an answered decision's,
/// stay — unless `EQUIPE_KEEP_PICTURES_DAYS` says otherwise.
pub const DAYS: u64 = 30;

/// The days a closed ticket's copies stay ([`DAYS`], or the environment's).
pub fn days() -> u64 {
    std::env::var("EQUIPE_KEEP_PICTURES_DAYS").ok().and_then(|d| d.trim().parse().ok()).unwrap_or(DAYS)
}

/// `<home>/squads/<squad>/visuals/kept`.
pub fn dir(home: &Path, squad: &str) -> PathBuf {
    crate::journal::squad_dir(home, squad).join("visuals").join("kept")
}

fn tickets_dir(home: &Path, squad: &str) -> PathBuf {
    dir(home, squad).join("tickets")
}

/// Where ticket `id`'s copies are.
pub fn ticket_dir(home: &Path, squad: &str, id: TicketId) -> PathBuf {
    tickets_dir(home, squad).join(format!("T-{id}"))
}

/// Where waiting decisions' copies are.
pub fn decisions_dir(home: &Path, squad: &str) -> PathBuf {
    dir(home, squad).join("decisions")
}

/// The name a copy of the picture named `original` has: a hash of the path,
/// and its extension — `None` for a path that isn't a picture's.
pub fn name_of(original: &str) -> Option<String> {
    let ext = Path::new(original).extension()?.to_str()?.to_ascii_lowercase();
    super::mime_of(Path::new(original))?;
    let digest = ring::digest::digest(&ring::digest::SHA256, original.as_bytes());
    let hex: String = digest.as_ref()[..16].iter().map(|b| format!("{b:02x}")).collect();
    Some(format!("{hex}.{ext}"))
}

/// The folders whose pictures don't last: equipe's worktrees and this Mac's temp folders.
fn fragile(home: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = super::homes(home).iter().map(|h| h.join("worktrees")).collect();
    roots.extend(super::scratch_folders());
    roots
}

/// A copy of each of `images` that is saved in a worktree or a temp folder
/// into `into`, unless one as new is there; at most `most` bytes in all
/// there. How many were copied.
fn keep(home: &Path, images: &[String], into: &Path, most: u64) -> usize {
    let roots = fragile(home);
    let squads: Vec<PathBuf> = super::homes(home).iter().map(|h| h.join("squads")).collect();
    let mut copied = 0;
    for image in images {
        // Inside a worktree or a temp folder, a picture, links resolved: else
        // it lasts where it is — as one in the squads' own folders does.
        let Ok(real) = super::check(Path::new(image), &roots) else { continue };
        if squads.iter().any(|s| real.starts_with(s)) {
            continue;
        }
        let Some(name) = name_of(image) else { continue };
        let to = into.join(&name);
        let Ok(from) = std::fs::metadata(&real) else { continue };
        if std::fs::metadata(&to).is_ok_and(|kept| kept.len() == from.len() && kept.modified().ok() >= from.modified().ok()) {
            continue;
        }
        let held: u64 = std::fs::read_dir(into).into_iter().flatten().flatten().filter(|e| e.file_name() != name.as_str()).filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum();
        if held + from.len() > most {
            crate::daemon_log!("equipe: {image} wasn't kept: {} holds {} already", into.display(), crate::design::captures::size_words(held));
            continue;
        }
        if std::fs::create_dir_all(into).is_err() {
            return copied;
        }
        // Written aside, then put in place: a copy is whole or not there.
        let part = into.join(format!(".{name}.part"));
        match std::fs::copy(&real, &part).and_then(|_| std::fs::rename(&part, &to)) {
            Ok(()) => copied += 1,
            Err(err) => {
                let _ = std::fs::remove_file(&part);
                crate::daemon_log!("equipe: {image} wasn't kept in {}: {err}", into.display());
            }
        }
    }
    copied
}

/// Ticket `id`'s pictures kept past its worktree (the module's doc).
pub fn ticket(home: &Path, squad: &str, id: TicketId, images: &[String]) -> usize {
    keep(home, images, &ticket_dir(home, squad, id), TICKET_MAX)
}

/// A waiting decision's pictures kept past the folder they were saved in.
pub fn decision(home: &Path, squad: &str, images: &[String]) -> usize {
    keep(home, images, &decisions_dir(home, squad), u64::MAX)
}

/// The copy of `original`, named by `tickets` or a waiting decision
/// (`decision`), when one is kept.
pub fn copy_of(home: &Path, squad: &str, tickets: &[TicketId], decision: bool, original: &str) -> Option<PathBuf> {
    let name = name_of(original)?;
    let decisions = decision.then(|| decisions_dir(home, squad));
    tickets.iter().map(|id| ticket_dir(home, squad, *id)).chain(decisions).map(|d| d.join(&name)).find(|p| p.is_file())
}

/// Whether a ticket's copies still stay `now`: while it is open, and for
/// `days` after it last changed (`updated_at`) once closed.
pub fn stays(closed: bool, updated_at: Option<u64>, now: u64, days: u64) -> bool {
    !closed || updated_at.is_none_or(|at| now.saturating_sub(at) < days * 86_400)
}

/// Copies no longer wanted, removed: tickets' not among `tickets`, and
/// decisions' not among `waiting` untouched for `days`. What went, for the log.
pub fn sweep(home: &Path, squad: &str, tickets: &[TicketId], waiting: &[String], days: u64) -> Vec<String> {
    let mut gone = (0usize, 0usize);
    for entry in std::fs::read_dir(tickets_dir(home, squad)).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_prefix("T-").and_then(|n| n.parse::<TicketId>().ok()) else { continue };
        if !tickets.contains(&id) && std::fs::remove_dir_all(entry.path()).is_ok() {
            gone.0 += 1;
        }
    }
    let wanted: Vec<String> = waiting.iter().filter_map(|w| name_of(w)).collect();
    let old = |m: &std::fs::Metadata| m.modified().ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|ago| ago >= Duration::from_secs(days * 86_400));
    for entry in std::fs::read_dir(decisions_dir(home, squad)).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !wanted.contains(&name) && entry.metadata().is_ok_and(|m| m.is_file() && old(&m)) && std::fs::remove_file(entry.path()).is_ok() {
            gone.1 += 1;
        }
    }
    if gone == (0, 0) {
        return Vec::new();
    }
    vec![format!("{squad}: removed the kept pictures of {} closed tickets and {} past decisions", gone.0, gone.1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-keep-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn png(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbImage::from_pixel(4, 4, image::Rgb([10, 20, 30])).save(path).unwrap();
    }

    fn named(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn only_a_worktrees_pictures_are_copied_and_found_by_their_path() {
        let home = home("copied");
        let in_worktree = home.join("worktrees/site/t7-ab12-1/handin-shots/menu.png");
        let in_folder = home.join("squads/acme/home/brand/logo.png");
        png(&in_worktree);
        png(&in_folder);
        let images = vec![named(&in_worktree), named(&in_folder), "/nowhere/gone.png".to_owned()];
        assert_eq!(ticket(&home, "acme", 7, &images), 1, "the worktree's alone");
        assert_eq!(ticket(&home, "acme", 7, &images), 0, "kept already, as new");
        let copy = copy_of(&home, "acme", &[7], false, &images[0]).expect("kept");
        assert!(copy.starts_with(ticket_dir(&home, "acme", 7)) && copy.extension().unwrap() == "png");
        assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(&in_worktree).unwrap());
        assert!(copy_of(&home, "acme", &[7], false, &images[1]).is_none(), "the squad's folder lasts");
        assert!(copy_of(&home, "acme", &[8], false, &images[0]).is_none(), "another ticket's");
        assert_eq!(decision(&home, "acme", &images[..1]), 1);
        assert!(copy_of(&home, "acme", &[], true, &images[0]).is_some_and(|c| c.starts_with(decisions_dir(&home, "acme"))));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_link_out_of_the_worktree_is_not_copied() {
        let home = home("link");
        let secret = home.join("squads/acme/secret.png");
        png(&secret);
        let link = home.join("worktrees/site/t7-ab12-1/shot.png");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert_eq!(ticket(&home, "acme", 7, &[named(&link)]), 0);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_tickets_copies_stay_past_its_close_then_go() {
        let home = home("sweep");
        let shot = home.join("worktrees/site/t7-ab12-1/a.png");
        png(&shot);
        let images = vec![named(&shot)];
        ticket(&home, "acme", 7, &images);
        ticket(&home, "acme", 9, &images);
        assert!(stays(true, Some(1_000), 1_000 + 29 * 86_400, 30) && !stays(true, Some(1_000), 1_000 + 31 * 86_400, 30));
        assert!(stays(false, Some(1_000), u64::MAX, 30), "open");
        let swept = sweep(&home, "acme", &[9], &[], 30);
        assert_eq!(swept.len(), 1, "{swept:?}");
        assert!(!ticket_dir(&home, "acme", 7).exists() && ticket_dir(&home, "acme", 9).exists());
        std::fs::remove_dir_all(home).unwrap();
    }
}
