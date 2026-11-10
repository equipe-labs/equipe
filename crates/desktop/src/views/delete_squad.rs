//! "Delete squad…": from the squad switcher (the sidebar's foot) or Settings ›
//! Squad & machine. The window's squad only — the one whose members and
//! tickets it can count. The same question every destructive action asks
//! (`sheet::Ask`, its yes in red, never answered by ⏎): it names the squad and
//! says what happens — its members stop, its open tickets go with it, what can
//! be made again (worktrees, build caches) goes at once, its folder moves to
//! `~/.equipe/deleted/` on the daemon's machine for a week, projects are left
//! as they are — and how much room that gives back, as the daemon measures it
//! (`rpc::SQUAD_FOOTPRINT`). A switch under it deletes it for good, now. Yes
//! sends `Command::DeleteSquad` over the window's own connection, so a squad
//! on another machine is deleted there, by its daemon.
//!
//! Every window hears `Event::SquadDeleted`: the squad leaves its switcher, and
//! a window that was on it opens the squad the daemon says comes next — or,
//! none left, the screen that starts a new one.

use equipe_core::protocol::rpc::{self, SquadFootprint, SquadFootprintAsk};
use equipe_core::protocol::{Command, TicketStatus};
use equipe_core::t;
use gpui_kit::*;

use crate::sheet::{Ask, AskOption, Then};
use crate::state::{Conn, Model};
use crate::Target;
use crate::Shell;

/// How long a deletion may take before the window stops waiting: removing
/// big worktrees takes a while; a daemon too old to know the command never answers.
const WAIT: std::time::Duration = std::time::Duration::from_secs(180);
/// How long the question waits for the daemon to measure the squad's folders.
const LOOK_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The question while it is on screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Asking {
    pub squad: String,
    /// Deleted for good, now: nothing kept to restore.
    pub purge: bool,
    /// What it gives back, once measured; `None` while the daemon looks (or
    /// when it can't say: an older daemon).
    pub footprint: Option<SquadFootprint>,
    /// The daemon couldn't say.
    pub unknown: bool,
}

/// The question before `model`'s squad is deleted. `place` is where its folder
/// goes and `machine` the machine that is on; `only` when it is the last squad.
pub fn ask(model: &Model, place: &str, machine: &str, only: bool, asking: &Asking) -> Ask {
    let name = model.squad.as_str();
    let members = model.members.len();
    let open = model.tickets.iter().filter(|t| !t.status.is_closed()).count();
    let at_work = model.tickets.iter().filter(|t| t.status == TicketStatus::InProgress).count();
    let days = equipe_core::squads::KEPT_DAYS;
    let mut body = Vec::new();
    if members > 0 {
        body.push(t!("squad_delete.members", count = members as u64));
    }
    if open > 0 {
        body.push(match at_work {
            0 => t!("squad_delete.tickets", count = open as u64),
            n => format!("{} {}", t!("squad_delete.tickets", count = open as u64), t!("squad_delete.at_work", count = n as u64)),
        });
    }
    body.push(if asking.purge { t!("squad_delete.folder_purge", machine = machine) } else { t!("squad_delete.folder", path = place, machine = machine, days = days) });
    body.push(t!("squad_delete.untouched").to_owned());
    body.push(space(asking, days));
    if only {
        body.push(t!("squad_delete.only").to_owned());
    }
    Ask {
        title: t!("squad_delete.ask", name = name),
        body,
        yes: if asking.purge { t!("squad_delete.yes_purge").into() } else { t!("squad_delete.yes").into() },
        then: Then::Send(vec![Command::DeleteSquad { squad: name.to_owned(), purge: asking.purge }]),
        option: Some(AskOption { label: t!("squad_delete.purge").into(), on: asking.purge }),
    }
}

/// How much room deleting gives back, as the question says it.
fn space(asking: &Asking, days: u64) -> String {
    let Some(look) = asking.footprint else {
        return if asking.unknown { t!("squad_delete.space_unknown").to_owned() } else { t!("squad_delete.measuring").to_owned() };
    };
    let size = |bytes: u64| format!("{}{}", if look.complete { "" } else { "≥ " }, equipe_core::disk::size(bytes));
    if asking.purge {
        t!("squad_delete.space_purge", freed = size(look.freed + look.kept))
    } else {
        t!("squad_delete.space", freed = size(look.freed), kept = size(look.kept), days = days)
    }
}

/// `path` with this Mac's home as `~`.
fn tilde(path: &std::path::Path) -> String {
    match std::env::var_os("HOME").map(std::path::PathBuf::from).and_then(|h| path.strip_prefix(h).ok().map(std::path::Path::to_owned)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

impl Shell {
    /// Where a deleted squad's folder goes, as the question says it.
    fn deleted_place(&self) -> String {
        if self.endpoint.is_remote() {
            return "~/.equipe/deleted/".to_owned();
        }
        format!("{}/", tilde(&equipe_core::squads::deleted_dir(&equipe_providers::credentials::equipe_home())))
    }

    /// Whether the window can delete its squad now: attached to one, none being deleted.
    pub fn can_delete_squad(&self) -> bool {
        matches!(self.model.conn, Conn::Connected { .. }) && !self.model.squad.is_empty() && self.deleting_squad.is_none()
    }

    /// "Delete squad…": the question, over the window — and the daemon asked
    /// what deleting gives back, said once it is measured.
    pub fn open_delete_squad(&mut self, cx: &mut Context<Self>) {
        crate::tips::hide(cx);
        self.board.popup = None;
        self.settings.pop = None;
        if !self.can_delete_squad() {
            return;
        }
        let squad = self.model.squad.clone();
        let asking = Asking { squad: squad.clone(), purge: false, footprint: None, unknown: false };
        let only = self.squads.iter().all(|s| *s == self.model.squad);
        self.confirm = Some(ask(&self.model, &self.deleted_place(), &self.endpoint.label(), only, &asking));
        self.delete_asking = Some(asking);
        cx.notify();
        let Some(uploader) = self.link.as_ref().map(|l| l.uploader()) else { return };
        let asked = squad.clone();
        let look = cx.background_spawn(async move {
            uploader.call(rpc::SQUAD_FOOTPRINT, SquadFootprintAsk { squad: asked }, LOOK_WAIT).and_then(|v| serde_json::from_value::<SquadFootprint>(v).map_err(|e| e.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let answer = look.await;
            this.update(cx, |this, cx| {
                let Some(asking) = this.delete_asking.as_mut().filter(|a| a.squad == squad) else { return };
                match answer {
                    Ok(footprint) => asking.footprint = Some(footprint),
                    Err(_) => asking.unknown = true,
                }
                this.ask_delete_again();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The question drawn again from what is known now — unless it was
    /// answered or closed meanwhile.
    fn ask_delete_again(&mut self) {
        let Some(asking) = self.delete_asking.clone() else { return };
        let ours = self.confirm.as_ref().is_some_and(|a| matches!(&a.then, Then::Send(cs) if cs.iter().any(|c| matches!(c, Command::DeleteSquad { squad, .. } if *squad == asking.squad))));
        if asking.squad != self.model.squad || !ours {
            self.delete_asking = None;
            return;
        }
        let only = self.squads.iter().all(|s| *s == self.model.squad);
        self.confirm = Some(ask(&self.model, &self.deleted_place(), &self.endpoint.label(), only, &asking));
    }

    /// The question's switch: deleted for good, now — or kept for a week.
    pub fn flip_ask_option(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(asking) = self.delete_asking.as_mut() {
            asking.purge = on;
            self.ask_delete_again();
        }
        cx.notify();
    }

    /// Yes: sent; said while the daemon works; given up on if it never answers.
    pub(super) fn deleting(&mut self, squad: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.deleting_squad = Some(squad.to_owned());
        self.composer.flash = Some(t!("squad_delete.deleting", name = squad));
        let asked = squad.to_owned();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(WAIT).await;
            this.update(cx, |this, cx| {
                if this.deleting_squad.as_deref() == Some(asked.as_str()) {
                    this.deleting_squad = None;
                    this.composer.flash = Some(t!("squad_delete.no_answer", name = asked));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// A squad was deleted, by this window or another: gone from the
    /// switcher; a window on it moves on (deferred: this runs inside the
    /// connection's own pump, which moving on replaces). Whether to redraw.
    #[allow(clippy::too_many_arguments)]
    pub fn squad_deleted(&mut self, squad: &str, moved_to: &std::path::Path, next: Option<String>, freed: u64, purged: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.squads.retain(|s| s != squad);
        if self.deleting_squad.as_deref() == Some(squad) {
            self.deleting_squad = None;
            let shown = if self.endpoint.is_remote() { moved_to.display().to_string() } else { tilde(moved_to) };
            self.composer.flash = Some(done_words(squad, &shown, freed, purged));
        }
        if self.model.squad == squad {
            let flash = self.composer.flash.clone();
            cx.defer_in(window, move |this, window, cx| {
                let mut target = Target::new(this.endpoint.clone(), env!("CARGO_PKG_VERSION"));
                target.prefer = next;
                this.attach(target, window, cx);
                this.composer.flash = flash;
                this.composer.focus(window, cx);
                cx.notify();
            });
        }
        true
    }

    /// The daemon couldn't delete it: said, if this window asked.
    pub fn squad_not_deleted(&mut self, squad: &str, why: &str) -> bool {
        if self.deleting_squad.as_deref() != Some(squad) {
            return false;
        }
        self.deleting_squad = None;
        self.composer.flash = Some(why.to_owned());
        true
    }
}

/// What the window says once its squad is deleted.
fn done_words(squad: &str, shown: &str, freed: u64, purged: bool) -> String {
    let freed = equipe_core::disk::size(freed);
    if purged {
        t!("squad_delete.done_purge", name = squad, freed = freed)
    } else {
        t!("squad_delete.done", name = squad, path = shown, freed = freed, days = equipe_core::squads::KEPT_DAYS)
    }
}

#[cfg(test)]
mod tests {
    use super::{Asking, ask, done_words};
    use crate::sheet::Then;
    use crate::state::Model;
    use equipe_core::protocol::rpc::SquadFootprint;
    use equipe_core::protocol::{Command, Ticket};

    fn asking(purge: bool, footprint: Option<SquadFootprint>) -> Asking {
        Asking { squad: "acme".into(), purge, footprint, unknown: false }
    }

    fn ticket(id: u32, status: &str) -> Ticket {
        serde_json::from_value(serde_json::json!({"id": id, "title": "t", "body": "", "status": status, "owner": "pm", "assignee": null, "project": null, "depends_on": [], "branch": null, "created_by": "pm", "note": null})).unwrap()
    }

    #[test]
    fn the_question_names_the_squad_says_what_happens_and_its_yes_is_red() {
        let mut model = Model::new("acme");
        model.tickets = vec![ticket(1, "in_progress"), ticket(2, "todo"), ticket(3, "done")];
        let ask = ask(&model, "~/.equipe/deleted/", "이 Mac", false, &asking(false, None));
        assert_eq!(ask.title, "acme 스쿼드를 삭제할까요?");
        assert!(ask.destructive(), "its yes is red, and ⏎ doesn't answer it");
        assert_eq!(ask.then, Then::Send(vec![Command::DeleteSquad { squad: "acme".into(), purge: false }]));
        assert_eq!(ask.body.len(), 4, "no members: not said; open tickets, the folder, the projects, the room: {:?}", ask.body);
        assert!(ask.body[0].contains('2') && ask.body[0].contains('1'), "two open, one at work: {}", ask.body[0]);
        assert!(ask.body[1].contains("~/.equipe/deleted/") && ask.body[1].contains("이 Mac") && ask.body[1].contains('7'), "kept a week: {}", ask.body[1]);
        assert!(!ask.body.iter().any(|l| l.contains("acme")), "the squad is named once, in the title");
        assert_eq!(ask.option.as_ref().map(|o| o.on), Some(false), "kept to restore unless the person says otherwise");
    }

    #[test]
    fn the_question_says_how_much_room_it_gives_back_and_for_good_keeps_nothing() {
        let model = Model::new("acme");
        let look = SquadFootprint { freed: 12_400_000_000, kept: 41_000_000, complete: true };
        let kept = ask(&model, "~/.equipe/deleted/", "이 Mac", false, &asking(false, Some(look)));
        let room = kept.body.iter().find(|l| l.contains("12 GB")).expect("what it frees");
        assert!(room.contains("41 MB") && room.contains('7'), "and what it keeps, for how long: {room}");
        let measuring = ask(&model, "~/.equipe/deleted/", "이 Mac", false, &asking(false, None));
        assert_eq!(measuring.body.len(), kept.body.len(), "said while measured, in its place");
        let gone = ask(&model, "~/.equipe/deleted/", "이 Mac", false, &asking(true, Some(look)));
        assert_eq!(gone.then, Then::Send(vec![Command::DeleteSquad { squad: "acme".into(), purge: true }]));
        assert!(gone.destructive() && gone.yes != kept.yes, "its yes says it: {}", gone.yes);
        assert!(gone.body.iter().any(|l| l.contains("12 GB")) && !gone.body.iter().any(|l| l.contains("~/.equipe/deleted/")), "{:?}", gone.body);
        let partly = ask(&model, "~/.equipe/deleted/", "이 Mac", false, &asking(false, Some(SquadFootprint { complete: false, ..look })));
        assert!(partly.body.iter().any(|l| l.contains("≥ 12 GB")), "at least, when the look ran out of time");
        let unknown = ask(&model, "~/.equipe/deleted/", "이 Mac", false, &Asking { unknown: true, ..asking(false, None) });
        assert_ne!(unknown.body, measuring.body, "an older daemon can't say: not measuring forever");
    }

    #[test]
    fn once_deleted_the_window_says_what_it_gave_back() {
        let kept = done_words("acme", "~/.equipe/deleted/acme-20261111-090000", 12_400_000_000, false);
        assert!(kept.contains("12 GB") && kept.contains("~/.equipe/deleted/acme-20261111-090000") && kept.contains('7'), "{kept}");
        let gone = done_words("acme", "~/.equipe/deleted/acme-20261111-090000", 12_400_000_000, true);
        assert!(gone.contains("12 GB") && !gone.contains("~/.equipe/deleted/"), "{gone}");
    }

    #[test]
    fn the_last_squad_says_a_new_one_comes_next() {
        let model = Model::new("solo");
        let last = ask(&model, "~/.equipe/deleted/", "studio", true, &asking(false, None));
        let not = ask(&model, "~/.equipe/deleted/", "studio", false, &asking(false, None));
        assert_eq!(last.body.len(), not.body.len() + 1);
        assert_eq!(not.body.len(), 3, "nothing at work, nobody on it: the folder, the projects and the room only");
    }
}
