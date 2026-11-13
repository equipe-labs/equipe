//! The daily usage note (`crate::usage`, docs/USAGE.md): once a day, a short
//! word to the person and the PM when the last 24 hours show a waste signal —
//! a PM conversation that is never compacted, tokens spent in huge contexts,
//! on tickets that were cancelled, in rework loops. Said at most once a day
//! per squad, and only when a signal is over its threshold; the person can
//! switch it off (`equipe usage digest off`).

use super::*;
use crate::routines::{Clock, Local};
use crate::usage;

/// The local hour (and minute) from which the day's note is looked at: after
/// the night's work is in, before the person's day is far on.
const DIGEST_AT: (u8, u8) = (9, 0);

impl Runtime {
    /// The minute's look at the daily usage note: once per local day, from
    /// [`DIGEST_AT`] on, for each squad that wants it.
    pub(super) fn usage_digest_tick(&mut self, now: u64) {
        let clock = Local;
        let today = clock.parts(now);
        if (today.hour, today.minute) < DIGEST_AT {
            return;
        }
        let day = format!("{:04}-{:02}-{:02}", today.year, today.month, today.day);
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            let Some(journal) = self.journal(&squad) else { continue };
            if journal.usage_digest_day().as_deref() == Some(day.as_str()) {
                continue;
            }
            // Looked at for today, said or not: a failed read isn't retried every minute.
            if journal.set_usage_digest_day(&day).is_err() || !journal.usage_digest_on() {
                continue;
            }
            let ((from, to), (before_from, before_to)) = usage::digest_windows(now);
            let (Ok(last), Ok(before)) = (journal.usage_input(from, to), journal.usage_input(before_from, before_to)) else { continue };
            let digest = usage::digest(&last, &before, now);
            if !digest.worth_saying() {
                continue;
            }
            crate::daemon_log!("equipe: {squad}: usage note: {} signal(s) over their thresholds", digest.last.signals.len());
            let brief = format!("equipe: {}", usage::say_digest(crate::i18n::core(crate::i18n::Lang::En), &digest));
            let shown = usage::say_digest(crate::i18n::core(self.config.lang), &digest);
            self.brief_pm(&squad, brief, Some(shown));
        }
    }
}
