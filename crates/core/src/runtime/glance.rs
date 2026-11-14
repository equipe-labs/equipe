//! Tickets the person can tell apart at a glance: the title rule a member's or
//! the PM's ticket is held to ([`equipe_protocol::title`]), the one-line ask a
//! ticket waiting on the person leads with, and `retitle_open_tickets`, which
//! brings the tickets written before the rule into line. equipe never writes a
//! title itself: the PM does, and equipe checks and applies it.

use super::*;
use equipe_protocol::title::{self, TicketLabel};

/// The history entry that keeps a ticket's title from before it was changed.
pub(super) const RETITLED: &str = "retitled";

/// The most open tickets one listing shows the PM.
const LISTED: usize = 40;

/// The `label` argument, if given.
pub(super) fn label_arg(args: &Value) -> Result<Option<TicketLabel>, String> {
    match board::arg_str(args, "label") {
        None => Ok(None),
        Some(text) => TicketLabel::parse(text)
            .map(Some)
            .ok_or_else(|| format!("A ticket's label is one of {}, not {text}.", TicketLabel::ALL.map(|l| l.key()).join(", "))),
    }
}

/// Whether `title` follows the title rule for a squad whose person writes in
/// `lang`: `Err` says what to change, for whoever wrote it. With no language
/// known there is none to hold the title to, and the rest of the rule applies.
pub(super) fn check_title(lang: Option<crate::i18n::Lang>, title: &str) -> Result<(), String> {
    match lang {
        Some(lang) => title::check(title, crate::i18n::core(lang).get("title.script"), lang.english_name()),
        None => title::check(title, "", ""),
    }
}

impl Runtime {
    /// The language the squad's person writes in, as last detected from what
    /// they wrote, if equipe speaks it: what a title is written in.
    pub(super) fn squad_language(&mut self, squad: &str) -> Option<crate::i18n::Lang> {
        self.journal(squad).and_then(|j| j.language()).and_then(|name| crate::i18n::Lang::from_english_name(&name))
    }

    /// The headline of a ticket put down for the person: their `ask`, then
    /// how long it takes (`effort_minutes`) in their language.
    pub(super) fn ask_headline(&mut self, squad: &str, args: &Value) -> Result<String, String> {
        let written = self.squad_language(squad);
        let lang = written.unwrap_or(self.config.lang);
        let ask = board::arg_str(args, "ask").ok_or("Say in `ask`, in one line, what the person has to do — it is what they read in their list instead of the ticket's title.")?;
        title::check_ask(ask)?;
        let words = crate::i18n::core(lang);
        if written.is_some() && !title::in_script(ask, words.get("title.script")) {
            let language = lang.english_name();
            return Err(format!("`ask` isn't in the person's language ({language}): write it in {language}."));
        }
        let minutes = args.get("effort_minutes").and_then(|m| m.as_u64().or_else(|| m.as_f64().map(|f| f.max(0.) as u64))).filter(|m| *m > 0);
        let minutes = minutes.ok_or("Say how long it takes the person, in whole minutes (`effort_minutes`, at least 1).")?;
        let effort = if minutes >= 60 {
            words.fill("ask.effort_hours", &[("n", minutes.div_ceil(60).to_string())])
        } else {
            words.fill("ask.effort_minutes", &[("n", minutes.to_string())])
        };
        Ok(title::headline(ask, &effort))
    }

    /// `retitle_open_tickets`: without `titles`, the open tickets whose title
    /// breaks the rule and why; with them — the PM's own words — each applied
    /// when it passes the rule, the old title kept in the ticket's history.
    pub(super) fn retitle_open_tickets(&mut self, squad: &str, member: &str, args: &Value) -> Result<String, String> {
        let lang = self.squad_language(squad);
        let (script, language) = lang.map_or(("", ""), |l| (crate::i18n::core(l).get("title.script"), l.english_name()));
        let open: Vec<Ticket> = self.tickets(squad).into_iter().filter(|t| !t.status.is_closed()).collect();
        let Some(given) = args.get("titles").and_then(Value::as_array) else {
            let broken: Vec<String> = open
                .iter()
                .filter_map(|t| {
                    let problems = title::problems(&t.title, script, language);
                    (!problems.is_empty()).then(|| format!("T-{} [{}] \"{}\" — {}", t.id, t.status.label(), t.title, problems.iter().map(title::Problem::message).collect::<Vec<_>>().join(" ")))
                })
                .collect();
            if broken.is_empty() {
                return Ok("Every open ticket's title already follows the title rule.".into());
            }
            let more = broken.len().saturating_sub(LISTED);
            let mut text = format!(
                "{} open ticket(s) have a title that breaks the rule. Write a short outcome title{in_language} for each — what changes or is checked, for whom — and call retitle_open_tickets again with `titles` (id, title, optional label); put what the old title carried (numbers, codes, steps) in the body with update_ticket if it matters. Nothing has changed yet.\n",
                broken.len(),
                in_language = if language.is_empty() { String::new() } else { format!(" in {language}") },
            );
            for line in broken.iter().take(LISTED) {
                text.push_str(&format!("\n{line}"));
            }
            if more > 0 {
                text.push_str(&format!("\n… and {more} more: ask again after this batch."));
            }
            return Ok(text);
        };
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        let (mut done, mut refused, mut changed) = (Vec::new(), Vec::new(), Vec::new());
        for item in given {
            let Some(id) = board::arg_id(item, "id") else {
                refused.push("an item without an `id`".to_owned());
                continue;
            };
            let Some(new) = board::arg_str(item, "title").map(str::trim) else {
                refused.push(format!("T-{id}: no `title`"));
                continue;
            };
            let Some(mut ticket) = open.iter().find(|t| t.id == id).cloned() else {
                refused.push(format!("T-{id}: not an open ticket"));
                continue;
            };
            let label = match label_arg(item) {
                Ok(label) => label,
                Err(why) => {
                    refused.push(format!("T-{id}: {why}"));
                    continue;
                }
            };
            if new != ticket.title
                && let Err(why) = title::check(new, script, language)
            {
                refused.push(format!("T-{id}: {why}"));
                continue;
            }
            if new != ticket.title {
                let _ = journal.ticket_event(id, member, RETITLED, &ticket.title);
                ticket.title = new.to_owned();
            }
            if label.is_some() {
                ticket.label = label;
            }
            journal.save_ticket(&ticket).map_err(|e| e.to_string())?;
            changed.push(ticket);
            done.push(format!("T-{id}"));
        }
        for ticket in changed {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
        }
        let mut reply = if done.is_empty() { "Nothing was retitled.".to_owned() } else { format!("Retitled {}: {} — each old title is kept in its history.", done.len(), done.join(", ")) };
        if !refused.is_empty() {
            reply.push_str(&format!("\nNot applied:\n- {}", refused.join("\n- ")));
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_is_one_of_the_six() {
        assert_eq!(label_arg(&json!({})), Ok(None));
        assert_eq!(label_arg(&json!({ "label": "Fix" })), Ok(Some(TicketLabel::Fix)));
        let err = label_arg(&json!({ "label": "qa" })).unwrap_err();
        assert!(err.contains("feature, fix, check, deploy, design, chore"), "{err}");
    }
}
