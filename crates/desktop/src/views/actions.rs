//! What the commands do in the window — whether they were typed (`/usage`),
//! picked from the menu bar, or came from ⌘K. One path for all of them:
//! `run_line` plans the line (`compose::plan`) and `apply_effect` carries out
//! what the plan asks for. The rules live in `compose`, `sheet` and `sidecar`;
//! this is the glue to the window, the daemon and the machine it runs on.

use std::path::PathBuf;

use equipe_core::t;
use gpui_kit::*;

use crate::compose::{self, Effect};
use crate::sheet::{self, After, Ask, Button, Outcome, Sheet, Then};
use crate::sidecar::{self, Job};
use crate::{Screen, Shell, export, usage};

impl Shell {
    /// Do what `line` (`/usage`, `/squad new shop`, …) says, as if typed and sent.
    pub fn run_line(&mut self, line: &str, window: &mut Window, cx: &mut Context<Self>) {
        let plan = compose::plan(&self.model, line, false);
        for command in plan.commands {
            self.send_command(command);
        }
        if plan.flash.is_some() {
            self.composer.flash = plan.flash;
        }
        if let Some(effect) = plan.effect {
            self.apply_effect(effect, window, cx);
        }
        cx.notify();
    }

    /// A menu item or ⌘K: the command line it stands for.
    pub fn menu_line(&mut self, line: &str, window: &mut Window, cx: &mut Context<Self>) {
        if line.ends_with(' ') || line == "/" {
            self.start_line(line, window, cx);
        } else {
            self.run_line(line, window, cx);
        }
    }

    /// A command that wants arguments: put it in the box, with its completions open.
    pub fn start_line(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = Screen::Chat;
        self.composer.set_text(text, window, cx);
        self.composer.focus(window, cx);
        self.composer.refresh_choices(&self.model, cx);
        cx.notify();
    }

    pub fn apply_effect(&mut self, effect: Effect, window: &mut Window, cx: &mut Context<Self>) {
        match effect {
            Effect::Select(session) => self.select_member(session, window, cx),
            Effect::ShowBoard => self.show(Screen::Board, cx),
            Effect::ShowDesign => self.open_design(cx),
            Effect::Find(words) => self.board_find(&words, cx),
            Effect::Focus(project) => self.set_focus(project, window, cx),
            Effect::SwitchSquad(name) => self.select_squad(&name, window, cx),
            Effect::Usage => {
                self.sheet = Some(sheet::usage_sheet(&usage::view(&self.model, usage::local_time)));
                self.confirm = None;
            }
            Effect::Help => {
                self.sheet = Some(sheet::help_sheet());
                self.confirm = None;
            }
            Effect::Export => self.export_conversation(),
            Effect::Quit => cx.quit(),
            Effect::OpenHome => self.open_home(),
            Effect::OpenMemory => self.open_memory(cx),
            Effect::Job(job, after) => self.run_job(job, after, cx),
            Effect::Ask(ask) => {
                self.sheet = None;
                self.confirm = Some(ask);
            }
            Effect::Suggest(on) => self.model.suggestions = on,
            Effect::Disk => self.open_disk(window, cx),
        }
        cx.notify();
    }

    // ── the daemon's machine ──

    /// The squad's folder, on the daemon's machine.
    fn squad_folder(&self) -> PathBuf {
        self.model
            .squad_list
            .iter()
            .find(|s| s.name == self.model.squad)
            .map(|s| s.home.clone())
            .unwrap_or_else(|| equipe_core::squad_home::path(&equipe_providers::credentials::equipe_home(), &self.model.squad))
    }

    /// `/home`: Finder on this Mac when the daemon is here; from another machine
    /// there is nothing to open — the path to reach it, and what the app can do.
    fn open_home(&mut self) {
        let dir = self.squad_folder();
        if !self.endpoint.is_remote() {
            let _ = std::process::Command::new("open").arg(&dir).spawn();
            self.composer.flash = Some(t!("actions.home_flash", path = dir.display()));
            return;
        }
        self.sheet = Some(Sheet {
            title: t!("actions.home_title").into(),
            lead: Some(t!("actions.home_where", machine = self.endpoint.label(), path = dir.display())),
            rows: Vec::new(),
            buttons: vec![
                Button::Copy(t!("actions.copy_path").into(), dir.display().to_string()),
                Button::Run(t!("actions.see_changes").into(), "/home changes".into()),
            ],
            foot: Some(t!("actions.home_remote").into()),
        });
    }

    /// `/memory`: what the squad remembers — and, with the daemon here, its folder in Finder.
    fn open_memory(&mut self, cx: &mut Context<Self>) {
        let dir = equipe_core::squad_memory::dir(&self.squad_folder());
        if !self.endpoint.is_remote() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::process::Command::new("open").arg(&dir).spawn();
        }
        let squad = self.model.squad.clone();
        self.run_job(Job::Memory { squad, words: Vec::new() }, After::Memory, cx);
    }

    /// Run a side command on the daemon's machine, off the UI thread; its answer
    /// comes back through `finish_job`.
    pub fn run_job(&mut self, job: Job, after: After, cx: &mut Context<Self>) {
        let (endpoint, done) = (self.endpoint.clone(), self.jobs.clone());
        std::thread::spawn(move || {
            let answer = sidecar::run(&endpoint, &job);
            let _ = done.send((after, answer));
        });
        cx.notify();
    }

    pub fn finish_job(&mut self, after: After, answer: Result<Vec<String>, String>, window: &mut Window, cx: &mut Context<Self>) {
        // The first-run page's squad: why not shows on the page itself.
        if matches!(after, After::Squads { .. }) && self.welcome.state.creating {
            self.welcome.state.finished(&answer);
            if answer.is_err() {
                return cx.notify();
            }
        }
        let memory = matches!(after, After::Memory);
        match sheet::after(&after, answer) {
            Outcome::Flash(text) => self.composer.flash = Some(text),
            Outcome::Sheet(mut sheet) => {
                if memory {
                    let dir = equipe_core::squad_memory::dir(&self.squad_folder());
                    sheet.buttons.push(if self.endpoint.is_remote() {
                        Button::Copy(t!("actions.copy_path").into(), dir.display().to_string())
                    } else {
                        Button::Open(t!("actions.open_folder").into(), dir)
                    });
                }
                self.confirm = None;
                self.sheet = Some(sheet);
            }
            Outcome::Ask(ask) => {
                self.sheet = None;
                self.confirm = Some(ask);
            }
            // The squad's folders or the squads changed: look again, at the new squad if one was made.
            Outcome::Squads { say, open } => {
                self.composer.flash = Some(say);
                match open {
                    Some(name) => {
                        self.pending_team = Some(equipe_core::roles::starting_team());
                        self.select_squad(&name, window, cx);
                    }
                    None => self.reattach(window, cx),
                }
            }
        }
        cx.notify();
    }

    /// Follow the same squad again from scratch — what the daemon says about it now.
    pub fn reattach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut target = crate::daemon::Target::new(self.endpoint.clone(), env!("CARGO_PKG_VERSION"));
        target.prefer = Some(self.model.squad.clone());
        self.attach(target, window, cx);
    }

    /// A squad opened with nobody on it: the PM, then the team it was started with.
    pub fn start_team(&mut self) {
        let joining = self.pending_team.take();
        if !self.model.members.is_empty() {
            return;
        }
        for command in compose::team(&self.model, joining.unwrap_or_default()) {
            self.send_command(command);
        }
    }

    // ── questions and sheets ──

    pub fn answer_question(&mut self, yes: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Ask { then, .. }) = self.confirm.take() else { return };
        if yes {
            match then {
                Then::Send(commands) => {
                    for command in commands {
                        // Taken off the squad: said under the projects, with the way back.
                        if let equipe_core::protocol::Command::RemoveProject { path, .. } = &command {
                            self.board.removed_project = Some(super::sidebar::Removed { name: equipe_core::board::project_name(path), path: path.clone(), at: std::time::Instant::now() });
                        }
                        self.send_command(command);
                    }
                }
                Then::Job(job, after) => self.run_job(job, after, cx),
            }
        }
        self.composer.focus(window, cx);
        cx.notify();
    }

    pub fn close_info(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet = None;
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// A change row's button: the same question `/home undo n` asks.
    pub fn ask_undo(&mut self, squad: String, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(effect) = compose::undo(squad, n).effect {
            self.apply_effect(effect, window, cx);
        }
    }

    // ── /export ──

    /// The conversation on screen as a Markdown file on this Mac, opened.
    fn export_conversation(&mut self) {
        let Some(member) = self.model.selected_member().map(|m| m.name.clone()) else {
            self.composer.flash = Some(t!("actions.nothing_to_export").into());
            return;
        };
        let Some(convo) = self.model.viewed().and_then(|id| self.model.convos.get(&id)) else {
            self.composer.flash = Some(t!("actions.nothing_to_export").into());
            return;
        };
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        self.composer.flash = Some(match export::save(&export::folder(&home), &self.model.squad, &member, convo.transcript.entries()) {
            Ok(path) => {
                let _ = std::process::Command::new("open").arg(&path).spawn();
                t!("actions.exported", path = path.display())
            }
            Err(err) => t!("actions.export_failed", err = err),
        });
    }
}
