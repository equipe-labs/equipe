//! The squad's design systems on a screen (`crate::design`): asked for with
//! `Command::Design`, they are read off the loop and sent; from then on,
//! while any screen follows the squad, their folders are looked at every
//! [`LOOK_EVERY`] — metadata only — and sent again when one changed, so the
//! screen follows what the designer saves. Nothing is read for a squad no
//! screen asked about. The person's taste notes and the decisions made ride
//! along (`crate::taste`); the person changes a note with `Command::Taste`.

use std::time::Duration;

use equipe_protocol::design::{Decision, DesignSystem, Taste, TasteChange};

use super::*;
use crate::design as ds;

/// How often a watched squad's design folders are looked at.
pub(super) const LOOK_EVERY: Duration = Duration::from_millis(1500);

/// A watched squad whose screen asked while a look was under way.
const OWED: u64 = u64::MAX;

/// A squad's design systems, where its own would be, and its taste.
pub(super) type Catalog = (Vec<DesignSystem>, String, Vec<Taste>);

impl Runtime {
    /// A screen asked: read and send them, and watch them from now on.
    pub(super) fn design_asked(&mut self, squad: String) {
        if !crate::squads::valid_name(&squad) {
            return;
        }
        self.design_watch.entry(squad.clone()).or_insert(0);
        self.look_at_design(&squad, true);
    }

    /// Look at `squad`'s folders off the loop; read them when they changed
    /// (or `always`).
    fn look_at_design(&mut self, squad: &str, always: bool) {
        if !self.design_busy.insert(squad.to_owned()) {
            // Asked while a look is under way: an ask always gets an answer,
            // so it is owed once that look ends.
            if always {
                self.design_watch.insert(squad.to_owned(), OWED);
            }
            return;
        }
        let (home, lang, internal) = (self.config.home.clone(), self.config.lang, self.internal.clone());
        let last = self.design_watch.get(squad).copied().unwrap_or(0);
        let squad = squad.to_owned();
        tokio::task::spawn_blocking(move || {
            let places = ds::places(&home, &squad);
            let stamp = ds::stamp(&places) ^ crate::taste::store::stamp(&home, &squad).rotate_left(1);
            let catalog = (always || stamp != last).then(|| {
                let (systems, folder) = ds::folder::catalog_of(&places, lang);
                (systems, folder, crate::taste::store::of(&home, &squad, &places))
            });
            let _ = internal.send(Internal::Design(squad, stamp, catalog));
        });
    }

    pub(super) fn on_design(&mut self, squad: String, stamp: u64, catalog: Option<Catalog>) {
        self.design_busy.remove(&squad);
        let owed = self.design_watch.get(&squad) == Some(&OWED);
        if let Some(last) = self.design_watch.get_mut(&squad) {
            *last = stamp;
        }
        match catalog {
            Some((systems, folder, taste)) => self.publish(Event::Design { squad, systems, folder, taste }),
            None if owed => self.look_at_design(&squad, true),
            None => {}
        }
    }

    /// Every watched squad looked at again; one no screen follows any more is let go.
    pub(super) fn design_tick(&mut self) {
        let followed: std::collections::HashSet<String> = self.subscribers.iter().filter(|(_, s)| !s.is_closed()).filter_map(|(f, _)| f.clone()).collect();
        self.design_watch.retain(|squad, _| followed.contains(squad));
        let squads: Vec<String> = self.design_watch.keys().cloned().collect();
        for squad in squads {
            self.look_at_design(&squad, false);
        }
    }

    /// The person changed a note of their taste: made, and the screens told —
    /// or told why not.
    pub(super) fn taste_changed(&mut self, squad: String, project: Option<String>, change: TasteChange) {
        if !crate::squads::valid_name(&squad) {
            return;
        }
        let language = self.journals.get(&squad).and_then(Journal::language);
        let today = squad_memory::date(now_secs());
        match crate::taste::store::change(&self.config.home, &squad, project.as_deref(), &change, &today, language.as_deref()) {
            Ok(()) => self.look_at_design(&squad, true),
            Err(crate::taste::store::Refused::Invalid(why)) => {
                let message = crate::i18n::core(self.config.lang).fill(why.key(), &[("max", equipe_protocol::design::NOTE_MAX.to_string())]);
                self.squad_error(&squad, message);
            }
            Err(crate::taste::store::Refused::NoProject) => {
                let message = say!(self, "taste.no_project", name = project.unwrap_or_default());
                self.squad_error(&squad, message);
            }
            Err(crate::taste::store::Refused::Failed(why)) => {
                let message = say!(self, "taste.not_saved", why = why);
                self.squad_error(&squad, message);
            }
        }
    }

    /// Keep a design decision in the squad's record (`project` none) or a
    /// project's, the chosen option's `picture` copied beside it, by `by`;
    /// the screens following the squad see it at once. For a pick made on a
    /// visual question: `decision.why` in the person's words when they gave
    /// any, `decision.date` empty for today. What the disk refused is logged.
    pub(super) fn record_decision(&mut self, squad: &str, project: Option<&str>, decision: &Decision, picture: Option<&Path>, by: &str) -> Option<PathBuf> {
        match crate::taste::record(&self.config.home, squad, project, decision, picture, by) {
            Ok(path) => {
                if self.design_watch.contains_key(squad) {
                    self.look_at_design(squad, true);
                }
                Some(path)
            }
            Err(why) => {
                crate::daemon_log!("equipe: {squad}'s design decision wasn't kept: {why}");
                None
            }
        }
    }
}
