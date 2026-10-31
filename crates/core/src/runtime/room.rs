//! The disk's room, as the daemon watches it: a look every minute, off the
//! loop ([`crate::disk::probe`]); the screens told only when the level
//! changes; members told when it gets critical, and new build work held back
//! until there's room again. A write that fails for want of room — a journal's,
//! a member's build — counts as full at once. What uses the space is looked at
//! when a screen asks, and given back on the person's click — but for the
//! build caches no build needs between builds, trimmed as soon as the disk is
//! short ([`crate::disk::trim_builds`]).

use super::*;
use crate::disk::{self, DiskCleaned, DiskLevel, DiskReport, DiskStatus};

/// How long a write that failed for want of room keeps the disk full, whatever a look says.
const NO_ROOM_HOLD: Duration = Duration::from_secs(5 * 60);
/// The build caches are trimmed on a short disk at most this often.
const TRIM_EVERY: Duration = Duration::from_secs(10 * 60);
/// With room to spare, what's certain to be unneeded is given back this often
/// (kept across restarts by a stamp's time, `cache/auto-clean`).
const AUTO_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// What the daemon knows of its disk.
#[derive(Default)]
pub(super) struct DiskWatch {
    /// As last looked at, with its level.
    pub status: DiskStatus,
    /// A write failed for want of room, then: full until a look begun after it says otherwise.
    pub no_room: Option<std::time::Instant>,
    /// A look under way, so they never pile up.
    pub looking: bool,
    /// A report under way, and who waits for it; `publish`: the screens get it too.
    pub reporting: bool,
    pub waiting: Vec<oneshot::Sender<DiskReport>>,
    pub publish: bool,
    /// A clean under way.
    pub cleaning: bool,
    /// When equipe last gave back, on its own, what it was sure of.
    pub auto_cleaned: Option<std::time::Instant>,
    /// A trim of the build caches under way, and when the last began.
    pub trimming: bool,
    pub trimmed: Option<std::time::Instant>,
    /// Members whose build failed for want of room, told once until there's room again.
    pub failed: std::collections::HashSet<SessionId>,
}

/// What a member's tool or turn said when the disk had no room.
pub(super) fn says_no_room(event: &Event) -> bool {
    let text = match event {
        Event::ToolResult { outcome, .. } if !outcome.ok => outcome.output.as_str(),
        Event::Error { message, .. } => message.as_str(),
        _ => return false,
    };
    ["No space left on device", "ENOSPC", "(os error 28)", "database or disk is full"].iter().any(|m| text.contains(m))
}

impl Runtime {
    /// The folders whose disk matters: equipe's home and every squad's projects.
    fn disk_paths(&mut self) -> Vec<PathBuf> {
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        let projects: Vec<PathBuf> = squads.iter().flat_map(|s| self.projects(s)).collect();
        disk::watched(&self.config.home, &projects)
    }

    /// How much room there is, looked at off the loop; one look at a time.
    pub(super) fn look_at_disk(&mut self) {
        if self.disk.looking {
            return;
        }
        self.disk.looking = true;
        let (paths, internal) = (self.disk_paths(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let asked = std::time::Instant::now();
            let _ = internal.send(Internal::Disk(disk::probe(&paths), asked));
        });
    }

    pub(super) fn on_disk(&mut self, status: Option<DiskStatus>, asked: std::time::Instant) {
        self.disk.looking = false;
        if let Some(status) = status {
            self.disk_measured(status, asked);
        }
    }

    /// A fresh count of the free space, begun at `asked`: its level, and what changes with it.
    fn disk_measured(&mut self, mut status: DiskStatus, asked: std::time::Instant) {
        // A write that failed after this look began still says full — and for a
        // while after any: writes failing over and over say so once, not each minute.
        let no_room = self.disk.no_room.is_some_and(|since| since >= asked || since.elapsed() < NO_ROOM_HOLD);
        let before = self.disk.status.level;
        status.level = disk::next_level(before, status.free, status.total, no_room);
        if status.level < DiskLevel::Full && !no_room {
            self.disk.no_room = None;
        }
        self.disk.status = DiskStatus { container_free: None, ..status };
        if self.disk.status.level != before {
            self.disk_level_changed(before);
        }
        self.trim_if_short();
        self.clean_certain_if_due();
    }

    /// What equipe is sure nobody needs ([`disk::certain`]: a finished
    /// ticket's work, stale build folders, old logs) goes on its own: at once
    /// when the disk runs short, else every few hours. Off the loop; logged and
    /// told to the screens. The rest waits for the person's yes.
    fn clean_certain_if_due(&mut self) {
        let every = if self.disk.status.level.is_warning() { TRIM_EVERY } else { AUTO_EVERY };
        let stamp = self.config.home.join("cache").join("auto-clean");
        let last = std::fs::metadata(&stamp).and_then(|m| m.modified()).ok().and_then(|at| at.elapsed().ok());
        let due = self.disk.auto_cleaned.is_none_or(|at| at.elapsed() >= every) && last.is_none_or(|ago| ago >= every);
        if self.disk.cleaning || !due {
            return;
        }
        self.disk.cleaning = true;
        self.disk.auto_cleaned = Some(std::time::Instant::now());
        let (known, paths, internal) = (self.disk_known(), self.disk_paths(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let _ = std::fs::create_dir_all(stamp.parent().expect("the stamp has a folder"));
            let _ = std::fs::write(&stamp, b"");
            let cleaned = disk::clean_certain(&known);
            let asked = std::time::Instant::now();
            let status = disk::probe(&paths);
            let _ = internal.send(Internal::DiskCleaned(Box::new(cleaned), status, asked, None));
        });
    }

    /// Clear one of what isn't equipe's (a cache of the Mac's, a project's
    /// build output) with the command a fresh look lists for it, as the
    /// person said yes to. Answered like a clean.
    pub(super) fn disk_run(&mut self, id: String) {
        if self.disk.cleaning {
            return;
        }
        self.disk.cleaning = true;
        let (known, paths, internal) = (self.disk_known(), self.disk_paths(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let cleaned = disk::clear_other(&known, &id);
            crate::daemon_log!("equipe: the person cleared {id} with its command");
            let asked = std::time::Instant::now();
            let status = disk::probe(&paths);
            let _ = internal.send(Internal::DiskCleaned(Box::new(cleaned), status, asked, None));
        });
    }

    /// Short of room: what no build needs between builds goes at once, off the
    /// loop — members' incremental caches, and when nearly full their build
    /// folders untouched for days; each only while cargo isn't building there.
    /// Rebuilt when wanted, so nobody is asked; logged.
    fn trim_if_short(&mut self) {
        let level = self.disk.status.level;
        if !level.is_warning() || self.disk.trimming || self.disk.trimmed.is_some_and(|at| at.elapsed() < TRIM_EVERY) {
            return;
        }
        (self.disk.trimming, self.disk.trimmed) = (true, Some(std::time::Instant::now()));
        let (home, internal, deeper) = (self.config.home.clone(), self.internal.clone(), level.holds_builds());
        tokio::task::spawn_blocking(move || {
            let (freed, lines) = disk::trim_builds(&home, deeper);
            let _ = internal.send(Internal::DiskTrimmed(freed, lines));
        });
    }

    pub(super) fn on_disk_trimmed(&mut self, freed: u64, lines: Vec<String>) {
        self.disk.trimming = false;
        for line in &lines {
            crate::daemon_log!("equipe: the disk is short: gave back {line}");
        }
        if freed > 0 {
            crate::daemon_log!("equipe: the disk is short: {} of build caches given back", disk::size(freed));
            self.look_at_disk();
        }
    }

    /// The disk is full, as a write just said: told once — to the log, the
    /// screens and the members — and the daemon goes on serving; what it
    /// couldn't save is lost, nothing more.
    pub(super) fn disk_is_full(&mut self) {
        self.disk.no_room = Some(std::time::Instant::now());
        let before = self.disk.status.level;
        if before != DiskLevel::Full {
            self.disk.status.level = DiskLevel::Full;
            self.disk_level_changed(before);
        }
    }

    /// The level moved from `before` to what `self.disk.status` says.
    fn disk_level_changed(&mut self, before: DiskLevel) {
        let status = self.disk.status.clone();
        let level = status.level;
        let free = disk::size(status.free);
        crate::daemon_log!("equipe: the disk is {} ({free} free on {})", level_word(level), status.path.display());
        self.publish(Event::DiskLevel { status: status.clone() });
        let squads = self.followed_squads();
        // Older screens know a full disk by this error, and a full disk past by this notice.
        if level == DiskLevel::Full {
            let message = say!(self, "error.disk_full");
            for squad in &squads {
                self.squad_error(squad, message.clone());
            }
        } else if before == DiskLevel::Full {
            let message = say!(self, "notice.disk_room");
            for squad in &squads {
                self.squad_error(squad, message.clone());
            }
        }
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        if level.holds_builds() && !before.holds_builds() {
            // The PM above all: it plans the work, and tells the person.
            let line = format!(
                "The disk is nearly full: {free} free. Builds may fail. Don't start heavy builds (cargo build, npm install, xcodebuild) until there's room again, and tell the person: they can free space in equipe's Disk panel (/disk) or with `equipe disk clean`. equipe holds new code tickets back meanwhile."
            );
            for squad in &squads {
                let shown = say!(self, "news.disk_critical", free = free);
                for id in self.squad_members(squad) {
                    if self.pm_of(squad) != Some(id)
                        && let Some(session) = self.sessions.get_mut(&id)
                    {
                        session.briefing.push(line.clone());
                        self.save_waiting(id);
                    }
                }
                self.brief_pm(squad, line.clone(), Some(shown));
            }
        } else if !level.holds_builds() && before.holds_builds() {
            self.disk.failed.clear();
            let line = format!("The disk has room again ({free} free): builds can go on, and equipe starts the code tickets it held back.");
            for squad in &squads {
                let shown = say!(self, "news.disk_room", free = free);
                for id in self.squad_members(squad) {
                    if self.pm_of(squad) != Some(id)
                        && let Some(session) = self.sessions.get_mut(&id)
                    {
                        session.briefing.push(line.clone());
                        self.save_waiting(id);
                    }
                }
                self.brief_pm(squad, line.clone(), Some(shown));
            }
            let internal = self.internal.clone();
            let _ = internal.send(Internal::Dispatch);
        }
    }

    /// Every member of `squad`, lanes left out.
    fn squad_members(&self, squad: &str) -> Vec<SessionId> {
        self.sessions.iter().filter(|(_, s)| s.squad.as_deref() == Some(squad) && s.lane.is_none() && s.member.is_some()).map(|(&id, _)| id).collect()
    }

    /// A member's build or turn failed for want of room: the disk is full,
    /// and its conversation says so plainly, once, with the way to fix it.
    pub(super) fn member_out_of_room(&mut self, id: SessionId) {
        self.disk_is_full();
        if !self.disk.failed.insert(id) {
            return;
        }
        let Some(name) = self.sessions.get(&id).and_then(|s| s.name().map(str::to_owned)) else { return };
        let text = say!(self, "news.disk_build_failed", name = name);
        self.publish(Event::Notice { session: id, text });
        if let Some(session) = self.sessions.get_mut(&id) {
            session.briefing.push(
                "Your last build failed because the disk is full (No space left on device). Don't retry it: tell the person, who frees space in equipe's Disk panel (/disk), then carry on."
                    .to_owned(),
            );
        }
        self.save_waiting(id);
    }

    /// Whether new work on `ticket` waits for room: code builds, tasks don't.
    pub(super) fn held_for_room(&self, kind: TicketKind) -> bool {
        self.disk.status.level.holds_builds() && kind == TicketKind::Code
    }

    /// What equipe knows, for a look at the disk: cheap, from what it holds.
    fn disk_known(&mut self) -> disk::Known {
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        let mut known = disk::Known { home: self.config.home.clone(), user_home: std::env::var_os("HOME").map(PathBuf::from), ..Default::default() };
        for squad in &squads {
            let tickets = self.journal(squad).map(|j| (j.tickets().unwrap_or_default(), j.projects().unwrap_or_default()));
            let Some((tickets, projects)) = tickets else { continue };
            for t in tickets {
                let worktree = t.worktree.clone().or_else(|| self.journal(squad).and_then(|j| j.ticket_work(t.id).ok()).and_then(|w| w.worktree));
                known.tickets.push(disk::KnownTicket { squad: squad.clone(), id: t.id, status: t.status, merged: t.merged, worktree, project: t.project.clone() });
            }
            known.projects.extend(projects.into_iter().map(|p| (squad.clone(), p)));
        }
        known.in_use = self.sessions.values().filter_map(|s| s.work.as_ref()?.sandbox.as_ref().map(|w| w.path.clone())).collect();
        known.building = self
            .sessions
            .values()
            .filter(|s| s.is_working())
            .filter_map(|s| Some(journal::build_dir(&self.config.home, s.squad.as_deref()?, s.name()?)))
            .collect();
        known
    }

    /// Look at what uses the disk, off the loop: `reply` gets it, and with
    /// `publish` every screen does. One look at a time; who asks meanwhile
    /// gets the one under way.
    pub(super) fn disk_report(&mut self, reply: Option<oneshot::Sender<DiskReport>>, publish: bool) {
        self.disk.waiting.extend(reply);
        self.disk.publish |= publish;
        if self.disk.reporting {
            return;
        }
        self.disk.reporting = true;
        let (known, paths, internal) = (self.disk_known(), self.disk_paths(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let asked = std::time::Instant::now();
            let status = disk::probe(&paths).unwrap_or_default();
            let report = disk::report(&known, status, disk::SCAN_BUDGET);
            let _ = internal.send(Internal::DiskReport(Box::new(report), asked));
        });
    }

    pub(super) fn on_disk_report(&mut self, mut report: DiskReport, asked: std::time::Instant) {
        self.disk.reporting = false;
        if report.status.total > 0 {
            self.disk_measured(report.status.clone(), asked);
        }
        report.status = DiskStatus { container_free: report.status.container_free, ..self.disk.status.clone() };
        for reply in std::mem::take(&mut self.disk.waiting) {
            let _ = reply.send(report.clone());
        }
        if std::mem::take(&mut self.disk.publish) {
            self.publish(Event::DiskReport { report });
        }
    }

    /// Give back what the person picked, off the loop: only what a fresh
    /// look still says is safe. `reply` gets what it did; without one the
    /// screens do, then a fresh report.
    pub(super) fn disk_clean(&mut self, ids: Vec<String>, reply: Option<oneshot::Sender<DiskCleaned>>) {
        if self.disk.cleaning {
            if let Some(reply) = reply {
                let _ = reply.send(DiskCleaned { status: self.disk.status.clone(), ..Default::default() });
            }
            return;
        }
        self.disk.cleaning = true;
        let (known, paths, internal) = (self.disk_known(), self.disk_paths(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let cleaned = disk::clean(&known, &ids);
            for id in &cleaned.removed {
                crate::daemon_log!("equipe: the person cleaned {id}");
            }
            let asked = std::time::Instant::now();
            let status = disk::probe(&paths);
            let _ = internal.send(Internal::DiskCleaned(Box::new(cleaned), status, asked, reply));
        });
    }

    pub(super) fn on_disk_cleaned(&mut self, mut cleaned: DiskCleaned, status: Option<DiskStatus>, asked: std::time::Instant, reply: Option<oneshot::Sender<DiskCleaned>>) {
        self.disk.cleaning = false;
        // Room made by hand counts at once, whatever a failed write said before.
        if !cleaned.removed.is_empty() {
            self.disk.no_room = None;
        }
        if let Some(status) = status {
            self.disk_measured(status, asked);
        }
        cleaned.status = self.disk.status.clone();
        if cleaned.auto {
            // Nothing it was sure of: nothing to say.
            if cleaned.removed.is_empty() {
                return;
            }
            crate::daemon_log!("equipe: gave back on its own what nobody needs: {} ({} items)", disk::size(cleaned.freed), cleaned.removed.len());
            self.publish(Event::DiskCleaned { cleaned });
            return;
        }
        crate::daemon_log!("equipe: the person gave back {} ({} items)", disk::size(cleaned.freed), cleaned.removed.len());
        match reply {
            Some(reply) => {
                let _ = reply.send(cleaned);
            }
            None => {
                self.publish(Event::DiskCleaned { cleaned });
                self.disk_report(None, true);
            }
        }
    }

    /// A ticket merged or cancelled: build output left in a worktree of its
    /// (one whose removal failed, or kept) goes now, off the loop — rebuilt
    /// when wanted; the worktree and its branch stay. Logged.
    pub(super) fn clear_finished_builds(&mut self, squad: &str, ticket: &Ticket) {
        let folder = if ticket.kind.in_home() {
            format!("{squad}.home")
        } else {
            match ticket.project.as_deref().and_then(Path::file_name) {
                Some(name) => name.to_string_lossy().into_owned(),
                None => return,
            }
        };
        let (home, prefix) = (self.config.home.clone(), format!("t{}-", ticket.id));
        // Another squad's open ticket of the same number may own a bare `T<id>` simulator.
        let shared = self.journals.keys().filter(|s| s.as_str() != squad).cloned().collect::<Vec<_>>().iter().any(|other| {
            self.journal(other).and_then(|j| j.tickets().ok()).is_some_and(|ts| ts.iter().any(|t| t.id == ticket.id && !matches!(t.status, TicketStatus::Done | TicketStatus::Cancelled)))
        });
        let (id, owner) = (ticket.id, squad.to_owned());
        tokio::task::spawn_blocking(move || {
            for line in disk::clear_simulators(id, &owner, shared) {
                crate::daemon_log!("equipe: {line} (its ticket is done with)");
            }
            let dir = home.join("worktrees").join(folder);
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    for line in disk::clear_builds(&home, &entry.path()) {
                        crate::daemon_log!("equipe: {line} (its ticket is done with)");
                    }
                }
            }
        });
    }

    /// The disk as the snapshot says it: only when it's short.
    pub(super) fn disk_for_snapshot(&self) -> Option<DiskStatus> {
        self.disk.status.level.is_warning().then(|| self.disk.status.clone())
    }
}

fn level_word(level: DiskLevel) -> &'static str {
    match level {
        DiskLevel::Ok => "fine again",
        DiskLevel::Low => "low",
        DiskLevel::Critical => "nearly full",
        DiskLevel::Full => "full",
    }
}
