//! How a person's request becomes shipped work, as the daemon runs it
//! (`crate::flow` has the rules, docs/FLOW.md the why): the request's main
//! ticket gets its driver and checklist; the baton moves it between members
//! on the same ticket and branch; members land their own work once its gates
//! pass and nothing in it needs a person, and the PM gets a digest of what
//! landed, to revert or follow up.

use equipe_protocol::requests::Link;
use equipe_protocol::{TicketFlow, TicketStep};

use super::turn_end::{HandBack, Reason};
use super::*;
use crate::flow as rules;

/// The PM's digest of landed work waits this long after the first in it
/// (unix seconds), so it comes as one message rather than a wake per ticket…
const DIGEST_AFTER: i64 = 30 * 60;
/// …unless this many landed meanwhile.
const DIGEST_AT: usize = 5;

impl Runtime {
    /// Ticket `ticket`, just made by member `maker` — the one answering the
    /// person (its request's `asked` ticket), or one given `steps` — gets
    /// its driver and checklist. A line for the tool's reply when it did.
    pub(super) fn start_flow(&mut self, squad: &str, maker: SessionId, ticket: &Ticket, steps: Vec<TicketStep>) -> Option<String> {
        if matches!(ticket.status, TicketStatus::Idea) {
            return None;
        }
        let (name, role, pm) = {
            let s = self.sessions.get(&self.member_of(maker))?;
            let m = s.member.as_ref()?;
            (m.name.clone(), m.role.clone(), s.is_pm())
        };
        let journal = self.journal(squad)?;
        let asked = journal.requests().link_of(ticket.id) == Some(Link::Asked);
        if !asked && steps.is_empty() {
            return None;
        }
        let slice = asked && rules::is_slice(ticket.kind, ticket.label, &role, ticket.owner.as_deref());
        let given = !steps.is_empty();
        let mut steps = if given { steps } else { rules::default_steps(slice, ticket.kind, ticket.owner.as_deref(), crate::i18n::core(self.config.lang)) };
        // The designer answering the person with the build for the frontend did the spec at its desk.
        if slice
            && !given
            && role == rules::DESIGNER
            && ticket.owner.as_deref().is_some_and(|o| board::role_of(o) != rules::DESIGNER)
            && let Some(first) = steps.first_mut()
        {
            first.done = true;
            first.by = Some(name.clone());
        }
        // The PM shapes; the member it gives the request to drives it.
        let driver = if pm { ticket.owner.clone() } else { Some(name.clone()) };
        let flow = TicketFlow { driver: driver.clone(), steps, slice, ..Default::default() };
        let journal = self.journal(squad)?;
        if let Err(err) = journal.set_ticket_flow(ticket.id, Some(&flow)) {
            crate::daemon_log!("equipe: {squad}: T-{}'s checklist wasn't kept: {err}", ticket.id);
            return None;
        }
        if let Some(made) = journal.ticket(ticket.id).ok().flatten() {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: made });
        }
        let driver = driver.unwrap_or_else(|| "its owner".to_owned());
        let slice_note = if slice { " It is one UI slice: the prototype and the build go on this ticket's branch, checked by equipe's precheck — no design QA ticket." } else { "" };
        Some(format!(
            " {driver} drives it to production. Its checklist:\n{}\nSpec, build and checks are items of it, on this one ticket: `check_steps` ticks and adds them, `pass_baton` hands it to the next role in the same worktree.{slice_note}",
            rules::checklist(&flow)
        ))
    }

    /// `pass_baton`: the ticket member `id` holds goes to `to` when the turn
    /// ends, on the same branch and worktree (`turn_end::Reason::Baton`).
    pub(super) fn pass_baton(&mut self, squad: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let note = board::arg_str(args, "note").map(str::trim).filter(|n| !n.is_empty()).ok_or("Say in `note` what they need to carry on: what is done, where, what is next.")?.to_owned();
        let back = args.get("back").and_then(Value::as_bool).unwrap_or(false);
        let (on, holder) = {
            let s = self.sessions.get(&id).ok_or("You're not on the squad.")?;
            let work = s.work.as_ref().ok_or("You're not on a ticket — there is no baton to pass.")?;
            // Not passed for a relay on missing work, it may still name the missing piece.
            if work.handed_in.is_some() || work.waiting.is_some() || work.end.handed_back.as_ref().is_some_and(|h| h.reason != Reason::Relay) {
                return Err(format!("T-{} is already handed in, waiting or handed back.", work.ticket));
            }
            (work.ticket, s.name().unwrap_or_default().to_owned())
        };
        // The next part needs work that doesn't exist yet: not a baton — that ticket made, and this one waits on it.
        if let Some(missing) = crate::blocked_on::parse(args)? {
            self.check_blocked_on(squad, &missing)?;
            let line = format!("{} ({})", missing.what, missing.role);
            let work = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()).ok_or("You're not on a ticket.")?;
            work.end.handed_back = Some(HandBack { reason: Reason::BlockedOn, tickets: Vec::new(), suggest: None, note, blocked_on: Some(missing) });
            return Ok(format!(
                "T-{on} isn't passed on: it waits on what is missing when this turn ends — {line}. equipe makes that ticket for its role, or \
finds the one on the board, under the same request; T-{on} comes back to you, in the same worktree, once it lands. End your turn now with a short line."
            ));
        }
        // Passed on before on the same missing piece: not again — the PM decides.
        if self.relay_caught(squad, on, &note).is_some() {
            let work = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()).ok_or("You're not on a ticket.")?;
            work.end.handed_back = Some(HandBack { reason: Reason::Relay, tickets: Vec::new(), suggest: board::arg_str(args, "to").map(str::to_owned), note, blocked_on: None });
            return Ok(format!(
                "T-{on} isn't passed on: it changed hands before with a note that something it needs doesn't exist yet. It goes to the PM \
when this turn ends, with one card asking whether to make the missing ticket. If you know what is missing and whose work it is, \
call `pass_baton` again with `blocked_on` {{ role, what }} instead — equipe then makes that ticket and T-{on} waits on it. Otherwise \
end your turn now with a short line."
            ));
        }
        let ticket = self.find_ticket(squad, on).ok_or_else(|| format!("There is no T-{on}."))?;
        let mut flow = ticket.flow.as_deref().cloned().unwrap_or_default();
        // A helper gives it back where it came from, when it names nobody.
        let to = board::arg_str(args, "to").map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned).or_else(|| flow.back_to.clone().filter(|b| *b != holder)).unwrap_or_default();
        if let Some(why) = rules::baton_refusal(&ticket, &holder, &to) {
            return Err(why);
        }
        let passes = self.journal(squad).and_then(|j| j.ticket_entries(on).ok()).map(|entries| rules::batons_since_hand_in(entries.iter().map(|e| e.kind.as_str()))).unwrap_or(0);
        if let Some(why) = rules::baton_loop(on, passes) {
            crate::daemon_log!("equipe: {squad}: T-{on}'s baton stopped after {passes} hand-overs since its last hand-in: its holder hands it in");
            return Err(why);
        }
        let staffed = self.member_id(squad, &to).is_some() || self.team(squad).iter().any(|(_, _, role)| *role == to);
        if !staffed {
            return Err(format!("Nobody on the squad is {to}. Pass it to a role the squad has, or hand it in."));
        }
        let mut done = rules::item_numbers(&flow, on, rules::PASS_BATON, "done", args.get("done"))?;
        // Passed with nothing named done: its holder's part is over — its items are ticked for it.
        if done.is_empty() && args.get("done").is_none() {
            done = rules::open_for(&flow, &holder);
        }
        if !done.is_empty() {
            rules::tick(&mut flow, &done, &[], Vec::new(), &holder)?;
        }
        flow.from = Some(holder.clone());
        if back {
            flow.back_to = Some(holder.clone());
        } else if flow.back_to.as_deref() == Some(to.as_str()) || flow.back_to.as_deref().is_some_and(|b| board::role_of(b) == to) {
            // Back with the one who lent it out.
            flow.back_to = None;
        }
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        journal.set_ticket_flow(on, Some(&flow)).map_err(|e| e.to_string())?;
        let work = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()).ok_or("You're not on a ticket.")?;
        work.end.handed_back = Some(HandBack { reason: Reason::Baton, tickets: Vec::new(), suggest: Some(to.clone()), note, blocked_on: None });
        let back_line = if back { " It comes back to you when they are done with their part." } else { "" };
        Ok(format!(
            "T-{on} goes to {to} when this turn ends: your changes are committed on its branch and they carry on in the same worktree, \
with your note — no new ticket, no wait for the PM.{back_line} End your turn now with a short line."
        ))
    }

    /// `check_steps`: tick off and add items of the checklist of the ticket
    /// member `id` holds.
    pub(super) fn check_steps(&mut self, squad: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let (on, holder) = {
            let s = self.sessions.get(&id).ok_or("You're not on the squad.")?;
            let work = s.work.as_ref().ok_or("You're not on a ticket — its holder ticks its checklist.")?;
            (work.ticket, s.name().unwrap_or_default().to_owned())
        };
        // Items by number or by their words, however the list is written; a call
        // that changes nothing is shown the checklist and the calls that would.
        let flow = self.find_ticket(squad, on).and_then(|t| t.flow.as_deref().cloned()).unwrap_or_default();
        let done = rules::item_numbers(&flow, on, rules::CHECK_STEPS, "done", args.get("done"))?;
        let replace = rules::item_numbers(&flow, on, rules::CHECK_STEPS, "replace", args.get("replace"))?;
        let added = rules::steps_arg(args, "add")?;
        if done.is_empty() && added.is_empty() && replace.is_empty() {
            return Err(rules::check_steps_help(on, &flow, board::unknown_args(rules::CHECK_STEPS, args)));
        }
        // What the person can now see or try: one line for them, in their language, once per item.
        let note = board::arg_str(args, "note").map(str::trim).filter(|n| !n.is_empty()).map(|n| n.chars().take(240).collect::<String>());
        if let (Some(note), Some(want)) = (&note, self.squad_language(squad))
            && super::reply_guard::wrong_language(note, want)
        {
            return Err(format!("Nothing ticked: `note` is read by the person, in {lang}. Write it in {lang} and call again.", lang = want.english_name()));
        }
        let said = self.change_steps(squad, on, &done, &replace, added, &holder)?;
        if let Some(note) = note
            && let Some(first) = done.first().copied()
        {
            self.step_note(squad, id, on, first, &note);
        }
        Ok(said)
    }

    /// A member's line to the person on what item `n` of `ticket` gave them
    /// to see or try: kept on the item and shown once, as a note of the work.
    fn step_note(&mut self, squad: &str, id: SessionId, ticket: TicketId, n: usize, note: &str) {
        let Some(found) = self.find_ticket(squad, ticket) else { return };
        let mut flow = found.flow.as_deref().cloned().unwrap_or_default();
        let Some(step) = flow.steps.get_mut(n - 1).filter(|s| s.note.is_none()) else { return };
        step.note = Some(note.to_owned());
        let Some(journal) = self.journal(squad) else { return };
        if journal.set_ticket_flow(ticket, Some(&flow)).is_err() {
            return;
        }
        if let Some(changed) = journal.ticket(ticket).ok().flatten() {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: changed });
        }
        let (Some(from), Some(pm)) = (self.sessions.get(&id).and_then(|s| s.member.clone()), self.pm_of(squad)) else { return };
        crate::daemon_log!("equipe: {squad}: T-{ticket} item {n} is ready for the person to see");
        self.publish(Event::MemberNote { session: pm, from, text: note.to_owned(), choices: Vec::new(), ticket: Some(ticket), at: now_secs(), source: id });
    }

    /// A run on a ticket whose turn just ended without ticking anything while
    /// items are still open for it: its next turn opens with a reminder.
    pub(super) fn remind_ticks(&mut self, id: SessionId) {
        let Some(s) = self.sessions.get(&id) else { return };
        let (Some(squad), Some(work), Some(began)) = (s.squad.clone(), s.work.as_ref(), s.turn_began) else { return };
        if work.handed_in.is_some() || work.end.handed_back.is_some() || work.waiting.is_some() {
            return;
        }
        let (ticket, holder) = (work.ticket, s.name().unwrap_or_default().to_owned());
        let Some(flow) = self.find_ticket(&squad, ticket).and_then(|t| t.flow.as_deref().cloned()) else { return };
        if !rules::ticked_nothing(&flow, &holder, began) {
            return;
        }
        let line = rules::tick_reminder(ticket, &flow, &holder);
        if let Some(session) = self.sessions.get_mut(&id)
            && !session.briefing.iter().any(|b| b.starts_with(&format!("[equipe] T-{ticket}: you ticked nothing")))
        {
            session.briefing.push(line);
            self.save_waiting(id);
        }
    }

    /// A finished tool call of member `id` showed its work: kept for its
    /// hand-in, and its checklist ticked by it now ([`rules::ticked_by`]).
    pub(super) fn evidence_shown(&mut self, id: SessionId, evidence: rules::Evidence) {
        let Some(s) = self.sessions.get_mut(&id) else { return };
        let (Some(squad), Some(holder)) = (s.squad.clone(), s.member.as_ref().map(|m| m.name.clone())) else { return };
        let Some(work) = s.work.as_mut() else { return };
        if !work.evidence.contains(&evidence) {
            work.evidence.push(evidence);
        }
        let ticket = work.ticket;
        let Some(flow) = self.find_ticket(&squad, ticket).and_then(|t| t.flow.as_deref().cloned()) else { return };
        let ticks = rules::ticked_by(&flow, &holder, evidence);
        if ticks.is_empty() {
            return;
        }
        match self.change_steps(&squad, ticket, &ticks, &[], Vec::new(), &holder) {
            Ok(_) => crate::daemon_log!("equipe: {squad}: T-{ticket}: {evidence:?} ticked item(s) {ticks:?} for {holder}"),
            Err(err) => crate::daemon_log!("equipe: {squad}: T-{ticket}: {evidence:?} could not tick {ticks:?}: {err}"),
        }
    }

    /// `holder` handed `ticket` in, its run having shown `shown`: its items
    /// still open ticked by [`rules::hand_in_ticks`], those on its word alone
    /// marked unconfirmed.
    pub(super) fn hand_in_steps(&mut self, squad: &str, ticket: TicketId, holder: &str, shown: &[rules::Evidence]) {
        let Some(found) = self.find_ticket(squad, ticket) else { return };
        let mut flow = found.flow.as_deref().cloned().unwrap_or_default();
        let (confirmed, unconfirmed) = rules::hand_in_ticks(&flow, holder, shown);
        if confirmed.is_empty() && unconfirmed.is_empty() {
            return;
        }
        let all: Vec<usize> = confirmed.iter().chain(&unconfirmed).copied().collect();
        if rules::tick(&mut flow, &all, &[], Vec::new(), holder).is_err() {
            return;
        }
        for n in &unconfirmed {
            flow.steps[n - 1].unconfirmed = true;
        }
        if !unconfirmed.is_empty() {
            crate::daemon_log!("equipe: {squad}: T-{ticket}: {holder} handed in with item(s) {unconfirmed:?} shown by no check or deploy: marked unconfirmed");
        }
        self.save_flow(squad, ticket, &flow);
    }

    /// `tickets` went live (equipe deployed main after they landed): their
    /// go-live items are ticked, closed or not.
    pub(super) fn went_live(&mut self, squad: &str, tickets: &[TicketId]) {
        for &ticket in tickets {
            let Some(found) = self.find_ticket(squad, ticket) else { continue };
            let Some(mut flow) = found.flow.as_deref().cloned() else { continue };
            let live = rules::ticked_by(&flow, "equipe", rules::Evidence::Live);
            if live.is_empty() || rules::tick(&mut flow, &live, &[], Vec::new(), "equipe").is_err() {
                continue;
            }
            crate::daemon_log!("equipe: {squad}: T-{ticket} is live: item(s) {live:?} ticked");
            self.save_flow(squad, ticket, &flow);
        }
    }

    /// `ticket`'s checklist kept as `flow`, and told to the screens.
    fn save_flow(&mut self, squad: &str, ticket: TicketId, flow: &TicketFlow) {
        let Some(journal) = self.journal(squad) else { return };
        if journal.set_ticket_flow(ticket, Some(flow)).is_err() {
            return;
        }
        if let Some(changed) = journal.ticket(ticket).ok().flatten() {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: changed });
        }
    }

    /// Tick `done`, take out the open items `replace` and add `added` (those
    /// it hasn't already) on `ticket`'s checklist, by `by`: what it says back.
    pub(super) fn change_steps(&mut self, squad: &str, ticket: TicketId, done: &[usize], replace: &[usize], added: Vec<TicketStep>, by: &str) -> Result<String, String> {
        let found = self.find_ticket(squad, ticket).ok_or_else(|| format!("There is no T-{ticket}."))?;
        if found.status.is_closed() {
            return Err(format!("T-{ticket} is {} — its checklist is closed.", found.status.label()));
        }
        let mut flow = found.flow.as_deref().cloned().unwrap_or_default();
        let had = flow.steps.len();
        let said = rules::tick(&mut flow, done, replace, added, by)?;
        // Only what it grew by counts against the cap: repeats and replaced items don't.
        if let Some(why) = rules::steps_overflow(ticket, had, flow.steps.len().saturating_sub(had)) {
            return Err(why);
        }
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        journal.set_ticket_flow(ticket, Some(&flow)).map_err(|e| e.to_string())?;
        if let Some(changed) = journal.ticket(ticket).ok().flatten() {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: changed });
        }
        Ok(format!("T-{ticket}: {said}\n{}", rules::checklist(&flow)))
    }

    /// `ticket` just became ready to land — handed in with nothing more to
    /// check, or its design check passed. Where the PM accepts work, its
    /// member lands it now when nothing in it needs a person; otherwise the
    /// PM is woken with `prompt`, as before.
    /// Who lands `found`, ready, by the squad's settings and what its branch
    /// changed (`crate::flow::landing_with`); with whether the PM accepts and
    /// whether it is the PM's own. `None` when the squad isn't open.
    fn landing_of(&mut self, squad: &str, found: &Ticket) -> Option<(rules::Landing, bool, bool)> {
        // The squad's autonomy policy keeping every merge for the person: no member lands its own.
        let merges_ask = self.autonomy_state(squad).policy.merge == equipe_protocol::autonomy::Level::Ask;
        let journal = self.journals.get(squad)?;
        let (self_land, pm_accepts) = (journal.self_land() && !merges_ask, journal.pm_accepts());
        let pm_name = self.pm_of(squad).and_then(|pm| self.sessions.get(&pm)).and_then(|s| s.name().map(str::to_owned)).unwrap_or_default();
        let pms_own = crate::accept::own_code(found, &pm_name).is_some();
        let changed = self.handed_in_files.get(&(squad.to_owned(), found.id)).cloned().unwrap_or_default();
        let sql = self.handed_in_sql.get(&(squad.to_owned(), found.id)).cloned().unwrap_or_default();
        Some((rules::landing_with(self_land, pm_accepts, found, &changed, &sql, pms_own), pm_accepts, pms_own))
    }

    /// Whether `ticket`, handed in, is its member's to land — nothing in it
    /// needs a person — so the PM puts nothing about it to anyone.
    pub(super) fn lands_itself(&mut self, squad: &str, ticket: TicketId) -> bool {
        let Some(found) = self.find_ticket(squad, ticket) else { return false };
        found.status == TicketStatus::Review && found.ready_for_review() && matches!(self.landing_of(squad, &found), Some((rules::Landing::Member, _, _)))
    }

    pub(super) fn work_ready(&mut self, squad: &str, ticket: TicketId, prompt: String) {
        let Some(found) = self.find_ticket(squad, ticket) else { return };
        let Some((landing, pm_accepts, pms_own)) = self.landing_of(squad, &found) else { return };
        match landing {
            rules::Landing::Member => {
                crate::daemon_log!("equipe: {squad}: T-{ticket} is ready and needs no person: its member lands it");
                let _ = self.internal.send(Internal::SelfLand { squad: squad.to_owned(), ticket });
            }
            rules::Landing::Pm(held) => {
                if !pm_accepts || (pms_own && held != rules::Held::PersonConfirms) {
                    return;
                }
                let Some(pm) = self.pm_of(squad) else { return };
                let why = match held {
                    rules::Held::PersonConfirms => " It isn't landed: you told the person they would see it and say yes first. Show them now \
its preview link and its pictures (phone and computer), in plain words, and ask; it stays on its branch and preview — nothing of it \
lands or goes live — until they say yes. Then `accept_work` it with their words in `confirmed`."
                        .to_owned(),
                    rules::Held::Gated(why) => format!(" It isn't landed by its member: it touches what needs a yes ({}).", crate::i18n::core(crate::i18n::Lang::En).get(why.key())),
                    _ => String::new(),
                };
                let _ = self.internal.send(Internal::Wake { session: pm, prompt: format!("{prompt}{why}") });
            }
        }
    }

    /// Land `ticket` for its member: the PM's acceptance, by the member —
    /// the same look for conflicts, the project's checks on the merge, the
    /// landing queue — and its record says who landed it.
    pub(super) async fn self_land(&mut self, squad: &str, ticket: TicketId) {
        let Some(found) = self.find_ticket(squad, ticket) else { return };
        if found.status != TicketStatus::Review || !found.ready_for_review() {
            return;
        }
        let Some(by) = found.assignee.clone().or_else(|| found.owner.clone()) else { return };
        let Some(session) = self.member_id(squad, &by).or_else(|| self.pm_of(squad)) else { return };
        let checked = "Landed by its member: equipe found nothing in it that needs a person, and its gates passed — a clean merge with main, \
the project's checks on that merge where it declares them, the design precheck where screens changed.";
        // Landed work goes live: main is pushed, as the PM's acceptance did.
        // Without it a member's landing stayed on this Mac's main and never
        // reached origin, where the project deploys from.
        let args = json!({ "tickets": [ticket], "checked": checked, "push": true });
        let (reply, answer) = oneshot::channel::<ToolResult>();
        let squad_name = squad.to_owned();
        tokio::spawn(async move {
            if let Ok(said) = answer.await {
                crate::daemon_log!("equipe: {squad_name}: T-{ticket} landed by its member: {}", said.text.lines().next().unwrap_or_default());
            }
        });
        Box::pin(self.accept_work_by(squad, &by, session, &args, reply, true)).await;
    }

    /// Each minute: a squad whose members landed work since the PM's last
    /// digest tells the PM in one message, once the first has waited
    /// [`DIGEST_AFTER`] or [`DIGEST_AT`] are in.
    pub(super) fn digest_landed(&mut self, squad: &str) {
        let Some(journal) = self.journals.get(squad) else { return };
        if !journal.pm_accepts() || !journal.self_land() {
            return;
        }
        let since = journal.landed_digest_at();
        let landed = journal.landed_since(since).unwrap_or_default();
        let now = now_secs() as i64;
        let Some(first) = landed.first().map(|(_, at, _)| *at) else { return };
        if landed.len() < DIGEST_AT && now - first < DIGEST_AFTER {
            return;
        }
        // An additive schema migration lands like code, and is told here.
        let migrated = |t: TicketId| self.handed_in_sql.get(&(squad.to_owned(), t)).is_some_and(|sql| !sql.is_empty() && sql.values().all(|text| crate::migrations::classify(text).is_additive()));
        let lines: Vec<(TicketId, String, String, String)> = landed
            .iter()
            .filter_map(|(t, _, by)| {
                journal.ticket(*t).ok().flatten().map(|found| {
                    let what = crate::person_gate::what_changes(&found);
                    let what = if migrated(*t) { [what.as_str(), rules::ADDITIVE_MIGRATION].into_iter().filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" · ") } else { what };
                    (*t, found.title.clone(), by.clone(), what)
                })
            })
            .collect();
        let _ = journal.set_landed_digest_at(now);
        // With its next message, not a turn of its own: the digest itself says there is nothing to do if they're fine.
        self.brief_pm(squad, rules::digest(&lines), None);
    }

    /// `revert_landed`: work a member landed is undone by a revert ticket at
    /// the front of its member's queue, serving the same request.
    pub(super) fn revert_landed(&mut self, squad: &str, member: &str, args: &Value) -> Result<String, String> {
        let ticket = board::arg_id(args, "id").ok_or("Which ticket? Give its `id`.")?;
        let why = board::arg_str(args, "why").map(str::trim).filter(|w| !w.is_empty()).ok_or("Say `why`: what is wrong with it, for its member.")?.to_owned();
        let found = self.find_ticket(squad, ticket).ok_or_else(|| format!("There is no T-{ticket}."))?;
        let landed_by = self.journal(squad).ok_or("The board can't be opened.")?.landed_since(0).unwrap_or_default().into_iter().find(|(t, _, _)| *t == ticket).map(|(_, _, by)| by);
        let Some(by) = landed_by else {
            return Err(format!("T-{ticket} wasn't landed by its member. Work you or the person accepted is reverted by a ticket of its own, as any change."));
        };
        let title = say!(self, "ticket.revert_title", id = ticket, title = found.title);
        let body = format!(
            "{}\n\n{why}\n\nRevert what T-{ticket} merged into main (`git log --grep 'T-{ticket}'` finds its merge; `git revert -m 1 <merge>`), \
run the tests, and hand it in. Then, if the change is still wanted, fix it on a new branch.",
            say!(self, "ticket.revert_body", id = ticket, name = member)
        );
        let revert = Ticket { summary: None,
            id: 0,
            title: title.clone(),
            body,
            status: TicketStatus::Todo,
            kind: found.kind,
            label: Some(equipe_protocol::title::TicketLabel::Fix),
            owner: Some(by.clone()),
            assignee: None,
            project: found.project.clone(),
            depends_on: Vec::new(),
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            landing: false,
            created_by: member.to_owned(),
            note: None,
            asks: Vec::new(),
            remaining: Vec::new(),
            images: Vec::new(),
            priority: equipe_protocol::Priority::High,
            rank: 0,
            updated_at: None,
            started_at: None,
            app: None,
            design_qa: None,
            plan: None,
            model: None,
            flow: None,
        };
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        let id = journal.create_ticket(&revert).map_err(|e| e.to_string())?;
        let _ = journal.ticket_event(id, member, "created", &title);
        let _ = journal.ticket_event(ticket, member, "revert asked", &why);
        if let Some(request) = journal.requests().of_ticket(ticket) {
            let _ = journal.requests().link(request, id, Some(ticket), Link::Linked, now_secs());
        }
        let made = journal.ticket(id).ok().flatten();
        if let Some(made) = made {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: made });
        }
        Ok(format!("Created T-{id}, the revert of T-{ticket}, at the front of {by}'s queue. Tell the person in one line what was undone and why."))
    }
}
