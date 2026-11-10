//! "Delete squad…": from the squad switcher (the sidebar's foot) or Settings ›
//! Squad & machine. The window's squad only — the one whose members and
//! tickets it can count. The same question every destructive action asks
//! (`sheet::Ask`, its yes in red, never answered by ⏎): it names the squad and
//! says what happens — its members stop, its open tickets go with it, its
//! folder moves to `~/.equipe/deleted/` on the daemon's machine, projects are
//! left as they are. Yes sends `Command::DeleteSquad` over the window's own
//! connection, so a squad on another machine is deleted there, by its daemon.
//!
//! Every window hears `Event::SquadDeleted`: the squad leaves its switcher, and
//! a window that was on it opens the squad the daemon says comes next — or,
//! none left, the screen that starts a new one.

use equipe_core::protocol::{Command, TicketStatus};
use equipe_core::t;
use gpui_kit::*;

use crate::sheet::{Ask, Then};
use crate::state::{Conn, Model};
use crate::Target;
use crate::Shell;

/// How long a deletion may take before the window stops waiting: removing
/// big worktrees takes a while; a daemon too old to know the command never answers.
const WAIT: std::time::Duration = std::time::Duration::from_secs(180);

/// The question before `model`'s squad is deleted. `place` is where its folder
/// goes and `machine` the machine that is on; `only` when it is the last squad.
pub fn ask(model: &Model, place: &str, machine: &str, only: bool) -> Ask {
    let name = model.squad.as_str();
    let members = model.members.len();
    let open = model.tickets.iter().filter(|t| !t.status.is_closed()).count();
    let at_work = model.tickets.iter().filter(|t| t.status == TicketStatus::InProgress).count();
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
    body.push(t!("squad_delete.folder", path = place, machine = machine));
    body.push(t!("squad_delete.untouched").to_owned());
    if only {
        body.push(t!("squad_delete.only").to_owned());
    }
    Ask { title: t!("squad_delete.ask", name = name), body, yes: t!("squad_delete.yes").into(), then: Then::Send(vec![Command::DeleteSquad { squad: name.to_owned() }]) }
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

    /// "Delete squad…": the question, over the window.
    pub fn open_delete_squad(&mut self, cx: &mut Context<Self>) {
        crate::tips::hide(cx);
        self.board.popup = None;
        self.settings.pop = None;
        if !self.can_delete_squad() {
            return;
        }
        let only = self.squads.iter().all(|s| *s == self.model.squad);
        self.confirm = Some(ask(&self.model, &self.deleted_place(), &self.endpoint.label(), only));
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
    pub fn squad_deleted(&mut self, squad: &str, moved_to: &std::path::Path, next: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.squads.retain(|s| s != squad);
        if self.deleting_squad.as_deref() == Some(squad) {
            self.deleting_squad = None;
            let shown = if self.endpoint.is_remote() { moved_to.display().to_string() } else { tilde(moved_to) };
            self.composer.flash = Some(t!("squad_delete.done", name = squad, path = shown));
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

#[cfg(test)]
mod tests {
    use super::ask;
    use crate::sheet::Then;
    use crate::state::Model;
    use equipe_core::protocol::{Command, Ticket};

    fn ticket(id: u32, status: &str) -> Ticket {
        serde_json::from_value(serde_json::json!({"id": id, "title": "t", "body": "", "status": status, "owner": "pm", "assignee": null, "project": null, "depends_on": [], "branch": null, "created_by": "pm", "note": null})).unwrap()
    }

    #[test]
    fn the_question_names_the_squad_says_what_happens_and_its_yes_is_red() {
        let mut model = Model::new("acme");
        model.tickets = vec![ticket(1, "in_progress"), ticket(2, "todo"), ticket(3, "done")];
        let ask = ask(&model, "~/.equipe/deleted/", "이 Mac", false);
        assert_eq!(ask.title, "acme 스쿼드를 삭제할까요?");
        assert!(ask.destructive(), "its yes is red, and ⏎ doesn't answer it");
        assert_eq!(ask.then, Then::Send(vec![Command::DeleteSquad { squad: "acme".into() }]));
        assert_eq!(ask.body.len(), 3, "no members: not said; open tickets, the folder, the projects: {:?}", ask.body);
        assert!(ask.body[0].contains('2') && ask.body[0].contains('1'), "two open, one at work: {}", ask.body[0]);
        assert!(ask.body[1].contains("~/.equipe/deleted/") && ask.body[1].contains("이 Mac"), "{}", ask.body[1]);
        assert!(!ask.body.iter().any(|l| l.contains("acme")), "the squad is named once, in the title");
    }

    #[test]
    fn the_last_squad_says_a_new_one_comes_next() {
        let model = Model::new("solo");
        let last = ask(&model, "~/.equipe/deleted/", "studio", true);
        let not = ask(&model, "~/.equipe/deleted/", "studio", false);
        assert_eq!(last.body.len(), not.body.len() + 1);
        assert_eq!(not.body.len(), 2, "nothing at work, nobody on it: the folder and the projects only");
    }
}
