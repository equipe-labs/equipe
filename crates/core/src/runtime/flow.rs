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
        let done: Vec<usize> = board::arg_ids(args, "done").unwrap_or_default().into_iter().map(|n| n as usize).collect();
        let back = args.get("back").and_then(Value::as_bool).unwrap_or(false);
        let (on, holder) = {
            let s = self.sessions.get(&id).ok_or("You're not on the squad.")?;
            let work = s.work.as_ref().ok_or("You're not on a ticket — there is no baton to pass.")?;
            if work.handed_in.is_some() || work.waiting.is_some() || work.end.handed_back.is_some() {
                return Err(format!("T-{} is already handed in, waiting or handed back.", work.ticket));
            }
            (work.ticket, s.name().unwrap_or_default().to_owned())
        };
        let ticket = self.find_ticket(squad, on).ok_or_else(|| format!("There is no T-{on}."))?;
        let mut flow = ticket.flow.as_deref().cloned().unwrap_or_default();
        // A helper gives it back where it came from, when it names nobody.
        let to = board::arg_str(args, "to").map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned).or_else(|| flow.back_to.clone().filter(|b| *b != holder)).unwrap_or_default();
        if let Some(why) = rules::baton_refusal(&ticket, &holder, &to) {
            return Err(why);
        }
        let staffed = self.member_id(squad, &to).is_some() || self.team(squad).iter().any(|(_, _, role)| *role == to);
        if !staffed {
            return Err(format!("Nobody on the squad is {to}. Pass it to a role the squad has, or hand it in."));
        }
        if !done.is_empty() {
            rules::tick(&mut flow, &done, Vec::new(), &holder)?;
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
        work.end.handed_back = Some(HandBack { reason: Reason::Baton, tickets: Vec::new(), suggest: Some(to.clone()), note });
        let back_line = if back { " It comes back to you when they are done with their part." } else { "" };
        Ok(format!(
            "T-{on} goes to {to} when this turn ends: your changes are committed on its branch and they carry on in the same worktree, \
with your note — no new ticket, no wait for the PM.{back_line} End your turn now with a short line."
        ))
    }

    /// `check_steps`: tick off and add items of the checklist of the ticket
    /// member `id` holds.
    pub(super) fn check_steps(&mut self, squad: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let done: Vec<usize> = board::arg_ids(args, "done").unwrap_or_default().into_iter().map(|n| n as usize).collect();
        let added = rules::steps_arg(args, "add")?;
        if done.is_empty() && added.is_empty() {
            return Err("Say which items are `done` (by number), or `add` some.".into());
        }
        let (on, holder) = {
            let s = self.sessions.get(&id).ok_or("You're not on the squad.")?;
            let work = s.work.as_ref().ok_or("You're not on a ticket — its holder ticks its checklist.")?;
            (work.ticket, s.name().unwrap_or_default().to_owned())
        };
        self.change_steps(squad, on, &done, added, &holder)
    }

    /// Tick `done` and add `added` on `ticket`'s checklist, by `by`: what it says back.
    pub(super) fn change_steps(&mut self, squad: &str, ticket: TicketId, done: &[usize], added: Vec<TicketStep>, by: &str) -> Result<String, String> {
        let found = self.find_ticket(squad, ticket).ok_or_else(|| format!("There is no T-{ticket}."))?;
        if found.status.is_closed() {
            return Err(format!("T-{ticket} is {} — its checklist is closed.", found.status.label()));
        }
        let mut flow = found.flow.as_deref().cloned().unwrap_or_default();
        let said = rules::tick(&mut flow, done, added, by)?;
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
    pub(super) fn work_ready(&mut self, squad: &str, ticket: TicketId, prompt: String) {
        let Some(found) = self.find_ticket(squad, ticket) else { return };
        // The squad's autonomy policy keeping every merge for the person: no member lands its own.
        let merges_ask = self.autonomy_state(squad).policy.merge == equipe_protocol::autonomy::Level::Ask;
        let Some(journal) = self.journals.get(squad) else { return };
        let (self_land, pm_accepts) = (journal.self_land() && !merges_ask, journal.pm_accepts());
        let pm_name = self.pm_of(squad).and_then(|pm| self.sessions.get(&pm)).and_then(|s| s.name().map(str::to_owned)).unwrap_or_default();
        let pms_own = crate::accept::own_code(&found, &pm_name).is_some();
        let changed = self.handed_in_files.get(&(squad.to_owned(), ticket)).cloned().unwrap_or_default();
        match rules::landing(self_land, pm_accepts, &found, &changed, pms_own) {
            rules::Landing::Member => {
                crate::daemon_log!("equipe: {squad}: T-{ticket} is ready and needs no person: its member lands it");
                let _ = self.internal.send(Internal::SelfLand { squad: squad.to_owned(), ticket });
            }
            rules::Landing::Pm(held) => {
                if !pm_accepts || pms_own {
                    return;
                }
                let Some(pm) = self.pm_of(squad) else { return };
                let why = match held {
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
        let args = json!({ "tickets": [ticket], "checked": checked });
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
        let lines: Vec<(TicketId, String, String, String)> = landed
            .iter()
            .filter_map(|(t, _, by)| journal.ticket(*t).ok().flatten().map(|found| (*t, found.title.clone(), by.clone(), crate::person_gate::what_changes(&found))))
            .collect();
        let _ = journal.set_landed_digest_at(now);
        let Some(pm) = self.pm_of(squad) else { return };
        let _ = self.internal.send(Internal::Wake { session: pm, prompt: rules::digest(&lines) });
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
        let revert = Ticket {
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
            landing: false, note_full: None,
            created_by: member.to_owned(),
            note: None,
            asks: Vec::new(),
            remaining: Vec::new(),
            images: Vec::new(),
            priority: equipe_protocol::Priority::High,
            rank: 0,
            updated_at: None,
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
