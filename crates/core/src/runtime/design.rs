//! The squad's design systems on a screen (`crate::design`): asked for with
//! `Command::Design`, they are read off the loop and sent; from then on,
//! while any screen follows the squad, their folders are looked at every
//! [`LOOK_EVERY`] — metadata only — and sent again when one changed, so the
//! screen follows what the designer saves. Nothing is read for a squad no
//! screen asked about.

use std::time::Duration;

use equipe_protocol::design::DesignSystem;

use super::*;
use crate::design as ds;

/// How often a watched squad's design folders are looked at.
pub(super) const LOOK_EVERY: Duration = Duration::from_millis(1500);

/// A watched squad whose screen asked while a look was under way.
const OWED: u64 = u64::MAX;

/// A squad's design systems and where its own would be.
pub(super) type Catalog = (Vec<DesignSystem>, String);

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
            let stamp = ds::stamp(&places);
            let catalog = (always || stamp != last).then(|| ds::folder::catalog_of(&places, lang));
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
            Some((systems, folder)) => self.publish(Event::Design { squad, systems, folder }),
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
}
