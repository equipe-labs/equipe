//! The kinds of work the person stopped (`crate::stops` has the rules).
//! "Stop this" on a request or a ticket — or the person's words to a member,
//! who calls `stop_work` — cancels the open tickets of its chain and keeps a
//! stop in force for the squad: a member's new ticket of that kind (made
//! while working the chain, serving its request, named by its terms or
//! paths) is turned away with why, and a design review's fixes of it are not
//! filed, until the person lifts it. Every member's turn starts with the
//! stops in force; the screens list them under "Your requests".

use equipe_protocol::requests::{Link, RequestId, Stop, StopId};
use equipe_protocol::rpc::ToolResult;

use super::*;
use crate::stops as rules;

impl Runtime {
    /// `squad`'s stops in force, the oldest first.
    pub(super) fn stops_in_force(&self, squad: &str) -> Vec<Stop> {
        self.journals.get(squad).map(|j| j.stops().in_force()).unwrap_or_default()
    }

    fn publish_stops(&mut self, squad: &str) {
        let stops = self.stops_in_force(squad);
        self.publish(Event::StopsChanged { squad: squad.to_owned(), stops });
    }

    /// What every member's turn starts with while stops are in force.
    pub(super) fn stops_context(&self, squad: &str) -> Option<String> {
        rules::context(&self.stops_in_force(squad))
    }

    /// The chain a stop is made on: every ticket of `request`; or `ticket`
    /// with the rest of its part of a request when it serves one; each with
    /// the design follow-ups that came of it.
    fn stop_chain(&self, squad: &str, request: Option<RequestId>, ticket: Option<TicketId>) -> (Vec<TicketId>, Vec<TicketId>) {
        let Some(journal) = self.journals.get(squad) else { return (Vec::new(), Vec::new()) };
        let row = request.or_else(|| ticket.and_then(|t| journal.requests().of_ticket(t))).and_then(|r| journal.requests().get(r));
        let mut chain: Vec<TicketId> = Vec::new();
        let mut checks: Vec<TicketId> = Vec::new();
        match (&row, request, ticket) {
            (Some(row), Some(_), _) => chain.extend(row.tickets.iter().map(|l| l.ticket)),
            (Some(row), None, Some(t)) => {
                let part = crate::requests::part_of(&row.tickets, t);
                chain.extend(row.tickets.iter().filter(|l| crate::requests::part_of(&row.tickets, l.ticket) == part).map(|l| l.ticket));
            }
            (None, _, Some(t)) => chain.push(t),
            _ => {}
        }
        if let Some(row) = &row {
            checks.extend(row.tickets.iter().filter(|l| l.link == Link::Check).map(|l| l.ticket));
        }
        for t in chain.clone() {
            for f in journal.stops().follow_ups_of(t) {
                if !chain.contains(&f) {
                    chain.push(f);
                }
            }
        }
        (chain, checks)
    }

    /// Stop a kind of work: its chain's open tickets cancelled, a stop kept,
    /// the PM told, the screens shown. `terms` left empty are taken from the
    /// chain's titles (else the words).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn stop_work(
        &mut self,
        squad: &str,
        by: &str,
        words: &str,
        terms: Vec<String>,
        paths: Vec<String>,
        request: Option<RequestId>,
        ticket: Option<TicketId>,
        all: bool,
    ) -> Result<Stop, String> {
        let (chain, checks) = self.stop_chain(squad, request, ticket);
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        let tickets: Vec<Ticket> = chain.iter().filter_map(|t| journal.ticket(*t).ok().flatten()).collect();
        let words = match words.trim() {
            "" => request
                .and_then(|r| journal.requests().get(r).map(|row| row.title))
                .or_else(|| ticket.and_then(|t| tickets.iter().find(|x| x.id == t).map(|x| x.title.clone())))
                .ok_or("Say what to stop.")?,
            words => words.to_owned(),
        };
        let terms: Vec<String> = terms.into_iter().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).collect();
        let terms = if !terms.is_empty() || !paths.is_empty() || all {
            terms
        } else {
            let titles: Vec<&str> = tickets.iter().filter(|t| !checks.contains(&t.id)).map(|t| t.title.as_str()).collect();
            if titles.is_empty() { rules::terms_of(&[words.as_str()]) } else { rules::terms_of(&titles) }
        };
        let mut stop = Stop { id: 0, words, terms, paths, request, tickets: chain, by: by.to_owned(), since: now_secs(), cancelled: Vec::new(), refused: 0, all };
        stop.id = journal.stops().create(&stop).map_err(|e| e.to_string())?;
        // Its open tickets go now: whoever works one stops.
        let open: Vec<TicketId> = tickets.iter().filter(|t| !t.status.is_closed()).map(|t| t.id).collect();
        for t in &open {
            self.cancel(squad, *t).await;
        }
        stop.cancelled = open;
        if let Some(journal) = self.journals.get(squad) {
            let _ = journal.stops().set_cancelled(stop.id, &stop.cancelled);
        }
        // What of the chain is committed but not on main, kept so it never lands later inside other work;
        // and what of it the live site already shows, for the person to decide on.
        let (heads, live) = self.stopped_heads(squad, &tickets);
        if !heads.is_empty()
            && let Some(journal) = self.journals.get(squad)
        {
            let _ = journal.stops().set_heads(stop.id, &heads);
        }
        let cancelled = if stop.cancelled.is_empty() {
            "nothing was open".to_owned()
        } else {
            format!("cancelled {}", stop.cancelled.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", "))
        };
        let live_line = if live.is_empty() {
            String::new()
        } else {
            format!(
                " Already on the live site: {}. Tell the person in plain words that it is live and ask whether to keep it or take it off — \
until they answer, neither merge it nor deploy anything that adds to it.",
                live.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ")
            )
        };
        let line = if stop.all {
            let in_flight = self.in_flight(squad);
            let flight = if in_flight.is_empty() {
                "Nothing else was under way.".to_owned()
            } else {
                format!(
                    "Under way when it began, held as it is (nothing of it lands or goes live): {}.",
                    in_flight.iter().map(|(t, title, status)| format!("T-{t} “{title}” ({status})")).collect::<Vec<_>>().join("; ")
                )
            };
            format!(
                "The person paused all the team's work (S-{}): “{}” — {cancelled}. Until they say go, no ticket starts — equipe's own \
checks, speed and design reviews too — nothing is merged and nothing is deployed, but work for what they ask after this. {flight}{live_line} \
Tell them in plain words what was under way and that it waits for their go. When they say go, call `stop_work` with `lift`.",
                stop.id, stop.words
            )
        } else {
            format!(
                "The person stopped “{}” (S-{}): {cancelled}. equipe turns away new tickets of it until they lift the stop — don't plan it, file it, or follow it up.{live_line}",
                stop.words, stop.id
            )
        };
        let shown = say!(self, "news.stopped", words = stop.words, count = stop.cancelled.len());
        if by == "you" {
            // Stopped from a screen ("Stop this" on the plan, the progress strip or the build card): the PM says so to the
            // person in one line, now — the screen already shows it stopped, so nothing longer and no other work.
            if let Some(pm) = self.pm_of(squad) {
                self.show_news(pm, shown);
                let text = format!(
                    "[Stopped by the person] {line}\n\nReply to the person in ONE short line, in their language: it's stopped, what was \
done so far is kept, and they can ask for it again here any time. No tools, no other work in this turn."
                );
                self.hand_to_pm(squad, pm, text).await;
            }
        } else {
            self.brief_pm(squad, line, Some(shown));
        }
        crate::daemon_log!("equipe: {squad}: stop S-{} by {by}: “{}”, terms {:?}, {} ticket(s) cancelled", stop.id, stop.words, stop.terms, stop.cancelled.len());
        self.publish_stops(squad);
        Ok(stop)
    }

    /// The person's "Stop this" ([`Command::StopWork`]).
    pub(super) async fn person_stops(&mut self, squad: &str, request: Option<RequestId>, ticket: Option<TicketId>, words: &str) {
        if request.is_none() && ticket.is_none() && words.trim().is_empty() {
            return self.squad_error(squad, say!(self, "error.stop_what"));
        }
        let all = request.is_none() && ticket.is_none() && rules::pauses_all(words);
        if let Err(err) = self.stop_work(squad, "you", words, Vec::new(), Vec::new(), request, ticket, all).await {
            self.squad_error(squad, err);
        }
    }

    /// The person lifts stop `id` ([`Command::LiftStop`]): that work may be filed again.
    pub(super) fn lift_stop(&mut self, squad: &str, id: StopId) {
        let Some(journal) = self.journal(squad) else { return };
        let Some(stop) = journal.stops().get(id) else { return };
        if !journal.stops().lift(id, now_secs()).unwrap_or(false) {
            return;
        }
        let line = if stop.all {
            format!("The person said go (S-{id} lifted): the team's work goes on — what was held starts, lands and goes live as usual.")
        } else {
            format!("The person lifted the stop on “{}” (S-{id}): that work may be put on the board again when it is asked for.", stop.words)
        };
        let shown = say!(self, "news.stop_lifted", words = stop.words);
        self.brief_pm(squad, line, Some(shown));
        self.publish_stops(squad);
        if stop.all {
            self.carry_on_after_pause(squad);
        }
    }

    /// The person's "Undo" on a stop they just made ([`Command::UndoStop`]):
    /// lifted, and what it cancelled back on the board, each to go on from
    /// its branch. A pause of all work is only lifted (nothing was cancelled).
    pub(super) async fn undo_stop(&mut self, squad: &str, id: StopId) {
        let Some(stop) = self.journal(squad).and_then(|j| j.stops().get(id)) else { return };
        if !self.stops_in_force(squad).iter().any(|s| s.id == id) {
            return;
        }
        if stop.all {
            return self.lift_stop(squad, id);
        }
        let Some(journal) = self.journal(squad) else { return };
        if !journal.stops().lift(id, now_secs()).unwrap_or(false) {
            return;
        }
        let mut back = Vec::new();
        for t in &stop.cancelled {
            let Some(mut ticket) = self.find_ticket(squad, *t).filter(|x| x.status == TicketStatus::Cancelled) else { continue };
            ticket.status = TicketStatus::Todo;
            ticket.assignee = None;
            ticket.note = Some("Brought back by the person (they took back their stop): go on from the work on its branch.".into());
            if let Some(journal) = self.journal(squad) {
                let _ = journal.save_ticket(&ticket);
                let _ = journal.ticket_event(ticket.id, "you", "restored", "stop taken back");
            }
            back.push(ticket.id);
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
        }
        let again = if back.is_empty() {
            "nothing it cancelled needed to come back".to_owned()
        } else {
            format!("back on the board: {} — each goes on from its branch", back.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", "))
        };
        let line = format!("The person took back their stop on “{}” (S-{id}): {again}. Carry on as before; no need to tell them.", stop.words);
        let shown = say!(self, "news.stop_lifted", words = stop.words);
        self.brief_pm(squad, line, Some(shown));
        crate::daemon_log!("equipe: {squad}: stop S-{id} taken back, {} ticket(s) back on the board", back.len());
        self.publish_stops(squad);
        self.dispatch(squad).await;
    }

    // ── a pause of all work ──

    /// The pause of all work in force, if any.
    pub(super) fn pause_in_force(&self, squad: &str) -> Option<Stop> {
        rules::pause(&self.stops_in_force(squad)).cloned()
    }

    /// The pause `ticket` waits for: one is in force and the ticket doesn't
    /// serve what the person asked after it began.
    pub(super) fn pause_holding(&self, squad: &str, ticket: TicketId) -> Option<Stop> {
        let pause = self.pause_in_force(squad)?;
        let journal = self.journals.get(squad)?;
        let asked = journal.requests().of_ticket(ticket).and_then(|r| journal.requests().get(r)).map(|row| row.asked_at);
        (!rules::goes_on_in(&pause, asked)).then_some(pause)
    }

    /// Before a dispatch: while a pause holds, only tickets for what the person
    /// asked since start.
    pub(super) fn hold_for_pause(&self, squad: &str, tickets: &mut Vec<Ticket>) {
        if self.pause_in_force(squad).is_none() {
            return;
        }
        tickets.retain(|t| t.status != TicketStatus::Todo || self.pause_holding(squad, t.id).is_none());
    }

    /// Before a dispatch: tickets nobody asked for start only once they serve
    /// one of the person's requests (the PM linked it after their yes).
    pub(super) fn hold_unrequested(&self, squad: &str, tickets: &mut Vec<Ticket>) {
        let Some(journal) = self.journals.get(squad) else { return };
        let gated = journal.unrequested();
        if gated.is_empty() {
            return;
        }
        for t in &gated {
            if journal.requests().of_ticket(*t).is_some() || journal.ticket(*t).ok().flatten().is_none_or(|x| x.status.is_closed()) {
                let _ = journal.set_unrequested(*t, false);
            }
        }
        let still = journal.unrequested();
        tickets.retain(|t| t.status != TicketStatus::Todo || !still.contains(&t.id));
    }

    /// What `accept_work` is told for a ticket a pause holds: it isn't landed.
    pub(super) fn pause_refusal(pause: &Stop, ticket: TicketId) -> String {
        format!(
            "T-{ticket} isn't landed: the person paused all the team's work (S-{}) until they say go. It stays as it is; tell them in plain \
words that it is ready and waits for their go — never merge or deploy it before.",
            pause.id
        )
    }

    /// The open tickets under way in `squad`: (id, title, status).
    fn in_flight(&mut self, squad: &str) -> Vec<(TicketId, String, &'static str)> {
        self.tickets(squad)
            .iter()
            .filter_map(|t| {
                let status = match t.status {
                    TicketStatus::InProgress => "in progress",
                    TicketStatus::Review => "handed in",
                    TicketStatus::Done if t.not_on_main() => "accepted, not merged",
                    _ => return None,
                };
                Some((t.id, t.title.clone(), status))
            })
            .take(12)
            .collect()
    }

    /// Of `tickets`, the branch tips not on main yet — (ticket, commit) — and
    /// which tickets the live site already carries (a cloud team's live
    /// commits contain their tip, or their landing).
    fn stopped_heads(&self, squad: &str, tickets: &[Ticket]) -> (Vec<(TicketId, String)>, Vec<TicketId>) {
        let live_commits = crate::sprint::platform::Platform::discover().and_then(|p| crate::sprint::platform::live_commits_cached(&p)).unwrap_or_default();
        let mut heads = Vec::new();
        let mut live = Vec::new();
        for t in tickets {
            let (Some(branch), Some(repo)) = (t.branch.as_deref(), t.project.as_deref().and_then(|p| worktree::repo_root(p).ok())) else { continue };
            let Some(tip) = rev_parse(&repo, branch) else { continue };
            let on_main = rev_parse(&repo, "main").is_some_and(|main| crate::cloud_live::contains(&repo, &tip, &main));
            if !on_main {
                heads.push((t.id, tip.clone()));
            }
            if live_commits.iter().any(|l| crate::cloud_live::has_commit(&repo, l) && crate::cloud_live::contains(&repo, &tip, l)) {
                live.push(t.id);
            }
        }
        let _ = squad;
        (heads, live)
    }

    /// The person's stopped work `ticket`'s branch would bring to main: the
    /// first stopped commit it contains that main doesn't, as (stop, ticket).
    pub(super) fn carries_stopped(&self, squad: &str, ticket: &Ticket) -> Option<(StopId, TicketId)> {
        let heads = self.journals.get(squad)?.stops().heads_in_force();
        if heads.is_empty() {
            return None;
        }
        let (branch, repo) = (ticket.branch.as_deref()?, ticket.project.as_deref().and_then(|p| worktree::repo_root(p).ok())?);
        let tip = rev_parse(&repo, branch)?;
        let main = rev_parse(&repo, "main");
        heads.into_iter().find(|(_, of, commit)| {
            *of != ticket.id
                && crate::cloud_live::has_commit(&repo, commit)
                && crate::cloud_live::contains(&repo, commit, &tip)
                && !main.as_deref().is_some_and(|m| crate::cloud_live::contains(&repo, commit, m))
        }).map(|(stop, of, _)| (stop, of))
    }

    /// The pause lifted: runs it held go on, and what waited starts.
    fn carry_on_after_pause(&mut self, squad: &str) {
        let idle: Vec<(SessionId, String, TicketId)> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.squad.as_deref() == Some(squad) && s.turn.is_none() && s.run_of.is_some())
            .filter_map(|(&id, s)| {
                let work = s.work.as_ref().filter(|w| !w.preparing && !w.end.stopped && !w.end.nudged)?;
                Some((id, s.name()?.to_owned(), work.ticket))
            })
            .collect();
        for (id, name, ticket) in idle {
            self.nudge(id, squad, &name, ticket, false);
        }
        let _ = self.internal.send(Internal::Dispatch);
    }

    /// `stop_work` with `lift`: the person said go (or lifts a stop) in their own words.
    fn lift_by_words(&mut self, squad: &str, id: StopId) -> Result<String, String> {
        let stop = self.journals.get(squad).and_then(|j| j.stops().get(id)).ok_or_else(|| format!("S-{id} isn't in force."))?;
        self.lift_stop(squad, id);
        Ok(if stop.all {
            "Lifted: the team's work goes on — what was held starts, lands and goes live as usual. Tell the person in one plain line.".to_owned()
        } else {
            format!("Lifted S-{id}: “{}” may be put on the board again when it is asked for.", stop.words)
        })
    }

    /// The stop a new ticket would break, if any — counted, and the screens
    /// told. `lineage`: the tickets it comes of; `request`: the one it serves.
    pub(super) fn stop_for(&mut self, squad: &str, title: &str, body: &str, lineage: &[TicketId], request: Option<RequestId>) -> Option<Stop> {
        let stops = self.stops_in_force(squad);
        let stop = rules::stopping(&stops, title, body, lineage, request)?.clone();
        self.stop_refused(squad, &stop, title);
        Some(stop)
    }

    /// `stop` turned away a ticket titled `title`: counted, and the screens told.
    pub(super) fn stop_refused(&mut self, squad: &str, stop: &Stop, title: &str) {
        if let Some(journal) = self.journals.get(squad) {
            let _ = journal.stops().refused(stop.id);
        }
        crate::daemon_log!("equipe: {squad}: “{title}” turned away by stop S-{} (“{}”)", stop.id, stop.words);
        self.publish_stops(squad);
    }

    /// Whether the person asked for this again, in their own words to the
    /// member of `id` since `stop` began: then its ticket goes on the board.
    pub(super) fn asked_again(&self, id: SessionId, stop: &Stop) -> bool {
        let member = self.sessions.get(&id).and_then(|s| s.run_of.or(s.lane.as_ref().map(|(of, _)| *of))).unwrap_or(id);
        self.person_said_to(member).into_iter().any(|(text, at)| at > stop.since && rules::named_by(stop, &text, &text))
    }

    /// `stop_work`: the person told this member to stop a kind of work.
    pub(super) async fn stop_work_tool(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let answer = self.stop_work_answer(&call).await;
        let _ = reply.send(match answer {
            Ok(text) => ToolResult::ok(text),
            Err(text) => ToolResult::error(text),
        });
    }

    async fn stop_work_answer(&mut self, call: &ToolCall) -> Result<String, String> {
        let id = self.caller_id(call).ok_or_else(|| format!("{} isn't on the squad {}.", call.member, call.squad))?;
        let member = self.sessions.get(&id).and_then(|s| s.run_of.or(s.lane.as_ref().map(|(of, _)| *of))).unwrap_or(id);
        let now = now_secs();
        if !self.person_said_to(member).iter().any(|(_, at)| now.saturating_sub(*at) <= crate::requests::ASKED_WITHIN) {
            return Err("Only the person stops a kind of work, in their own words to you. If you think something should stop, ask them.".into());
        }
        let args = &call.arguments;
        if let Some(id) = board::arg_id(args, "lift") {
            return self.lift_by_words(&call.squad, id as StopId);
        }
        let words = board::arg_str(args, "words").map(str::trim).filter(|w| !w.is_empty()).ok_or("Give `words`: what the person said to stop, in their words.")?;
        let terms = board::arg_strs(args, "terms");
        let paths = board::arg_strs(args, "paths");
        let request = self.request_arg(&call.squad, args)?.filter(|r| *r != crate::requests::NO_REQUEST);
        let ticket = board::arg_id(args, "ticket");
        if let Some(t) = ticket
            && self.find_ticket(&call.squad, t).is_none()
        {
            return Err(format!("There is no T-{t}."));
        }
        // "Pause everything", in the PM's call or in the person's own words: all of the team's work.
        let all = board::arg_str(args, "scope") == Some("all")
            || rules::pauses_all(words)
            || self.person_said_to(member).iter().any(|(text, at)| now.saturating_sub(*at) <= crate::requests::ASKED_WITHIN && rules::pauses_all(text));
        if !all && terms.is_empty() && paths.is_empty() && request.is_none() && ticket.is_none() {
            return Err("Say what it covers: `terms` (words its tickets' titles have, e.g. \"component gallery\"), `paths`, a `ticket` of it or its `request` — or `scope: \"all\"` when they pause all work.".into());
        }
        if all && let Some(pause) = self.pause_in_force(&call.squad) {
            if ticket.is_some() || request.is_some() {
                let _ = self.stop_work(&call.squad, &call.member, words, terms, paths, request, ticket, false).await?;
            }
            return Ok(format!("All work is paused already (S-{}): nothing starts, lands or goes live until the person says go. Tell them so in one plain line.", pause.id));
        }
        let stop = self.stop_work(&call.squad, &call.member, words, terms, paths, request, ticket, all).await?;
        if stop.all {
            return Ok(format!(
                "Paused all work as S-{}. {}; nothing starts, lands or goes live until the person says go (`stop_work` with `lift`: {}). Tell them in plain words what was under way and that it waits for their go.",
                stop.id,
                if stop.cancelled.is_empty() { "Nothing was cancelled".to_owned() } else { format!("Cancelled {}", stop.cancelled.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ")) },
                stop.id
            ));
        }
        let cancelled = if stop.cancelled.is_empty() {
            "No ticket of it was open".to_owned()
        } else {
            format!("Cancelled {}", stop.cancelled.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", "))
        };
        Ok(format!(
            "Stopped as S-{}. {cancelled}; new tickets of it are turned away (titles with {}) until the person lifts the stop. Tell them so in one plain line, with no stop or ticket numbers.",
            stop.id,
            if stop.terms.is_empty() { "—".to_owned() } else { stop.terms.iter().map(|t| format!("“{t}”")).collect::<Vec<_>>().join(" or ") }
        ))
    }
}

/// `rev` in the repository at `repo`, as a full commit id.
fn rev_parse(repo: &std::path::Path, rev: &str) -> Option<String> {
    let out = std::process::Command::new("git").arg("-C").arg(repo).args(["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    let id = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !id.is_empty()).then_some(id)
}
