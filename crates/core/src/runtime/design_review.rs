//! Risk-based design QA's runtime half (`crate::design_qa::route` has the
//! rules, docs/DESIGN-QA.md the whole): the light session a designer judges
//! a change in — the precheck's report, its pictures and the spec's excerpts,
//! no repository — the daily design review that collects every change let
//! through without a designer turn, the audit of a random sample of them on
//! the top tier, and the routing tightened when audits find misses.

use super::*;
use crate::design::precheck;
use crate::design_qa::{self as rules, Mode, Review};
use crate::protocol::routines::Schedule;
use journal::{DesignQaBatch, DesignQaRun};

/// How much of a ticket's body a light brief carries.
const BODY_CLIP: usize = 1_500;
/// How much of a hand-in's note.
const NOTE_CLIP: usize = 800;
/// How much of a precheck's summary.
const SUMMARY_CLIP: usize = 4_000;
/// How much of each spec file named in the ticket, and how many.
const SPEC_CLIP: usize = 1_500;
const SPEC_FILES: usize = 3;
/// Pictures a light per-ticket brief names first.
const FIRST_PICTURES: usize = 6;
/// Pictures each change of a review brings; and of an audit.
const REVIEW_PICTURES: usize = 2;
const AUDIT_PICTURES: usize = 4;
/// Changes one review covers at most: more wait for the next.
const REVIEW_MAX: usize = 20;

/// The kinds of design QA batch tickets.
pub(super) const REVIEW: &str = "review";
pub(super) const AUDIT: &str = "audit";

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max { text.to_owned() } else { format!("{}…", text.chars().take(max).collect::<String>()) }
}

/// Where daily review or audit `ticket` keeps its pictures and main's precheck.
pub(super) fn batch_dir(home: &Path, squad: &str, ticket: TicketId) -> PathBuf {
    journal::squad_dir(home, squad).join("design-qa").join(format!("B-{ticket}"))
}

/// A task equipe puts on the board for `owner`.
#[allow(clippy::too_many_arguments)]
fn equipe_ticket(title: String, body: String, kind: TicketKind, owner: Option<String>, project: Option<PathBuf>, status: TicketStatus, priority: crate::protocol::Priority, by: &str) -> Ticket {
    Ticket {
        id: 0,
        title,
        body,
        status,
        kind,
        owner,
        assignee: None,
        project,
        depends_on: Vec::new(),
        branch: None,
        worktree: None,
        execution: None,
        merged: None,
        off_main: None,
        design_qa: None,
        plan: None,
        model: None,
        created_by: by.to_owned(),
        note: None,
        asks: Vec::new(),
        label: None, remaining: Vec::new(),
        images: Vec::new(),
        updated_at: None,
        priority,
        rank: 0,
    }
}

impl Runtime {
    /// The folder a design QA, review or audit ticket is judged in when it
    /// runs as a light session — the precheck's report and pictures, no
    /// repository — or `None` for a full session.
    pub(super) fn lite_dir(&mut self, squad: &str, ticket: TicketId) -> Option<PathBuf> {
        let home = self.config.home.clone();
        let journal = self.journal(squad)?;
        if let Some(batch) = journal.design_qa_batch(ticket) {
            return Some(batch.dir.map(PathBuf::from).unwrap_or_else(|| batch_dir(&home, squad, ticket)));
        }
        let run = journal.design_qa_run(ticket)?;
        let lite = run.lite && run.route.as_deref() == Some(rules::Route::PerTicket.code()) && run.report.is_some();
        lite.then(|| design_qa::precheck_dir(&home, squad, ticket))
    }

    /// What a light session reads besides its own folder: for a review or
    /// an audit, each change's own precheck.
    pub(super) fn lite_reads(&mut self, squad: &str, ticket: TicketId) -> Vec<PathBuf> {
        let home = self.config.home.clone();
        let Some(journal) = self.journal(squad) else { return Vec::new() };
        if journal.design_qa_batch(ticket).is_none() {
            return Vec::new();
        }
        journal.design_qa_batch_items(ticket).iter().map(|r| design_qa::precheck_dir(&home, squad, r.qa)).filter(|d| d.is_dir()).collect()
    }

    /// Whether `ticket` is a daily review or audit: its kind.
    pub(super) fn design_batch_kind(&mut self, squad: &str, ticket: TicketId) -> Option<String> {
        self.journal(squad)?.design_qa_batch(ticket).map(|b| b.kind)
    }

    /// Whether `ticket`, by its plan and the paths its body names, carries
    /// a designer's spec ([`rules::spec_attached`]).
    pub(super) fn spec_of(&self, ticket: &Ticket) -> bool {
        rules::spec_attached(&ticket.body, ticket.plan.as_deref())
    }

    /// The first turn's brief of a light per-ticket design QA session.
    pub(super) fn lite_prompt(&mut self, squad: &str, qa: TicketId, checked: &Ticket) -> String {
        let dir = design_qa::precheck_dir(&self.config.home, squad, qa);
        let run = self.journal(squad).and_then(|j| j.design_qa_run(qa)).unwrap_or_default();
        let engineer = checked.assignee.as_deref().or(checked.owner.as_deref()).unwrap_or("its engineer");
        let risks: Vec<&str> = run.risk.as_deref().unwrap_or_default().split(',').filter_map(|c| [rules::Risk::Waived, rules::Risk::NewScreens, rules::Risk::Broad, rules::Risk::Spec, rules::Risk::Recheck, rules::Risk::Tight, rules::Risk::Unverified, rules::Risk::UnmappedLarge].into_iter().find(|r| r.code() == c)).map(rules::Risk::said).collect();
        let why = if risks.is_empty() { String::new() } else { format!(" A designer looks at it now because {}.", risks.join("; ")) };
        let mut out = format!(
            "[equipe] You're on T-{qa}: design QA of T-{} ({}), handed in by {engineer} with changed screens. It waits in review until you finish.{why}\n\nYou work in {}, which holds equipe's precheck of the change: report.md, report.json, shots/, base/ and crops/.\n",
            checked.id,
            checked.title,
            dir.display()
        );
        out.push_str(&format!("\nWhat T-{} asked for:\n{}\n", checked.id, clip(&checked.body, BODY_CLIP)));
        if let Some(note) = &checked.note {
            out.push_str(&format!("\n{engineer}'s hand-in:\n{}\n", clip(note, NOTE_CLIP)));
        }
        if let Some(reason) = &run.waived {
            out.push_str(&format!(
                "\n{engineer} waived the precheck's S1/S2 findings: “{}”. Decide for each whether it truly costs the person something: a real one is a blocker.\n",
                clip(reason, 400)
            ));
        }
        let spec = spec_excerpts(checked, &dir);
        if !spec.is_empty() {
            out.push_str(&format!("\nThe spec it was built from:\n{spec}\n"));
        }
        let summary = std::fs::read_to_string(dir.join(precheck::REPORT_MD)).unwrap_or_default();
        out.push_str(&format!("\nThe precheck's report:\n{}\n", clip(&summary, SUMMARY_CLIP)));
        let first: Vec<String> = precheck::read(&dir).map(|r| r.flagged()).unwrap_or_default().into_iter().take(FIRST_PICTURES).collect();
        if !first.is_empty() {
            out.push_str(&format!("\nLook at these first (Read each once):\n- {}\n", first.join("\n- ")));
        }
        out.push_str(
            "\nFinish with design_qa: `pass` needs at least two of these pictures (or ones `qa_pictures` gave you) in `images`, \
`compared: true` once you have put them beside the spec, at least three conditions in `checked`, and no blocker; `changes` names \
each blocker — where, what the spec says, what was built. Polish never sends it back but always goes in `polish`. Then end your turn.",
        );
        out
    }

    /// The first turn's brief of a daily review or an audit.
    pub(super) fn batch_prompt(&mut self, squad: &str, batch: &DesignQaBatch) -> String {
        let home = self.config.home.clone();
        let Some(journal) = self.journal(squad) else { return String::new() };
        let items: Vec<DesignQaRun> = journal.design_qa_batch_items(batch.ticket).into_iter().filter(|r| if batch.kind == AUDIT { r.audit == Some(batch.ticket) } else { r.batch == Some(batch.ticket) }).collect();
        let tickets = self.tickets(squad);
        let dir = batch.dir.clone().map(PathBuf::from).unwrap_or_else(|| batch_dir(&home, squad, batch.ticket));
        let audit = batch.kind == AUDIT;
        let mut out = if audit {
            format!(
                "[equipe] You're on T-{}: an audit of {} change(s) picked at random from those equipe let through without a designer turn because they looked low risk. \
Audit each as its own design QA would, on the top tier: every S1/S2 you find is a miss of the routing — it is counted, files a fix ticket for its engineer, and enough misses send more changes to a designer. \
You work in {}.\n",
                batch.ticket,
                items.len(),
                dir.display()
            )
        } else {
            format!(
                "[equipe] You're on T-{}: the daily design review — {} change(s) equipe's precheck let through without a designer turn because they looked low risk. \
Look at them together, in one pass, and file only what is real: an S1/S2 becomes a fix ticket for its engineer; S3/S4 go into one polish ticket. You work in {}.\n",
                batch.ticket,
                items.len(),
                dir.display()
            )
        };
        if !audit {
            // Main's precheck since the last review, per project.
            let mains: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.join(precheck::REPORT_MD).is_file()).collect()).unwrap_or_default();
            for main in &mains {
                let summary = std::fs::read_to_string(main.join(precheck::REPORT_MD)).unwrap_or_default();
                out.push_str(&format!("\nMain since the last review ({}):\n{}\n", main.display(), clip(&summary, SUMMARY_CLIP / 2)));
            }
            if mains.is_empty() {
                out.push_str("\nMain's own precheck couldn't run this time: judge from each change's report below.\n");
            }
        }
        out.push_str("\nThe changes:\n");
        for r in &items {
            let checked = tickets.iter().find(|t| t.id == r.checked);
            let title = checked.map(|t| t.title.as_str()).unwrap_or("");
            let engineer = checked.and_then(|t| t.assignee.as_deref().or(t.owner.as_deref())).unwrap_or("its engineer");
            let report_dir = design_qa::precheck_dir(&home, squad, r.qa);
            let report = precheck::read(&report_dir);
            out.push_str(&format!("\n- T-{} {title} ({engineer}; its precheck: {})", r.checked, report_dir.display()));
            match &report {
                Some(rep) => {
                    let changed = rep.changed();
                    let polish = rep.findings.iter().filter(|f| f.new).count();
                    out.push_str(&format!("\n  Changed states: {}. New findings (polish): {polish}.", if changed.is_empty() { "none".to_owned() } else { clip(&changed.join(", "), 300) }));
                    if !rep.unmapped.is_empty() {
                        out.push_str(&format!(" Changed UI files no screen lists: {}.", clip(&rep.unmapped.join(", "), 200)));
                    }
                    if audit {
                        let summary = std::fs::read_to_string(report_dir.join(precheck::REPORT_MD)).unwrap_or_default();
                        out.push_str(&format!("\n  Its report:\n{}", clip(&summary, SUMMARY_CLIP / 3).lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")));
                    }
                    let pictures: Vec<String> = rep.flagged().into_iter().take(if audit { AUDIT_PICTURES } else { REVIEW_PICTURES }).collect();
                    if !pictures.is_empty() {
                        out.push_str(&format!("\n  Look at: {}", pictures.join(", ")));
                    }
                }
                None => out.push_str("\n  Its precheck report is gone: judge it from main's report."),
            }
            if let Some(c) = checked
                && let Some(note) = &c.note
            {
                out.push_str(&format!("\n  Hand-in: {}", clip(note.lines().next().unwrap_or_default(), 200)));
            }
        }
        out.push_str(
            "\n\nAsk `qa_pictures` for a state or condition you need and weren't given. Finish with design_review: `findings` in a few \
lines, and one entry in `items` for every change above — `ticket` (its number), `blockers` (S1/S2 only: where, what is wrong, \
what it should be) and `polish` (S3/S4). Then end your turn.",
        );
        out
    }

    /// The minute's look at design QA: the one-time word on a squad moved to
    /// `risk_batch`, and each daily review that is due.
    pub(super) async fn design_qa_tick(&mut self, now: u64) {
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            if !self.has_designer(&squad) {
                continue;
            }
            let Some(journal) = self.journal(&squad) else { continue };
            // A squad that ran design QA before and never chose a mode moves to risk_batch: told once.
            let told = !journal.design_qa_mode_set() && journal.first_time("design_qa_risk_batch") && journal.has_design_qa_runs();
            let settings = journal.design_qa_settings();
            if told {
                crate::daemon_log!("equipe: {squad}: design QA moved to risk_batch (revert: equipe design qa-config --mode per_ticket)");
                let line = "equipe: design QA now runs risk-based — only high-risk screen changes get a designer turn of their own; the rest go into one daily design review, with a random sample audited. The person can go back with `equipe design qa-config --mode per_ticket`.".to_owned();
                let shown = say!(self, "news.design_qa_mode_switched");
                self.brief_pm(&squad, line, Some(shown));
            }
            if settings.mode != Mode::RiskBatch {
                continue;
            }
            let Some(journal) = self.journal(&squad) else { continue };
            // A review or audit the person closed without a verdict doesn't hold the next one back.
            for open in journal.design_qa_open_batches() {
                if journal.ticket(open).ok().flatten().is_none_or(|t| t.status.is_closed()) {
                    let _ = journal.finish_design_qa_batch(open, 0, 0, 0);
                }
            }
            if journal.design_qa_batch_open() {
                continue;
            }
            let waiting = journal.design_qa_waiting_for_batch();
            let Some(first) = waiting.first() else { continue };
            let last = journal.design_qa_last_batch();
            let from = if last > 0 { last } else { first.created_at }.max(0) as u64;
            let (hour, minute) = settings.batch_at;
            let at = routines::next_run(Schedule::Daily { hour, minute }, from, &routines::Local);
            if waiting.len() < settings.batch_size && now < at {
                continue;
            }
            self.start_design_review(&squad, waiting, &settings);
            Box::pin(self.dispatch(&squad)).await;
        }
    }

    /// One daily review of `waiting`, and the audit of its sample: put on
    /// the board, held while main's precheck runs off the loop.
    fn start_design_review(&mut self, squad: &str, mut waiting: Vec<DesignQaRun>, settings: &rules::Settings) {
        waiting.truncate(REVIEW_MAX);
        let home = self.config.home.clone();
        let tickets = self.tickets(squad);
        let checked: Vec<Ticket> = waiting.iter().filter_map(|r| tickets.iter().find(|t| t.id == r.checked).cloned()).collect();
        let listed = |runs: &[&DesignQaRun]| runs.iter().map(|r| format!("- T-{} {}", r.checked, checked.iter().find(|t| t.id == r.checked).map(|t| t.title.as_str()).unwrap_or(""))).collect::<Vec<_>>().join("\n");
        let all: Vec<&DesignQaRun> = waiting.iter().collect();
        let review = equipe_ticket(
            say!(self, "ticket.design_review_title", count = waiting.len()),
            format!("{}\n\n{}", say!(self, "ticket.design_review_body", count = waiting.len()), listed(&all)),
            TicketKind::Task,
            Some(design_qa::DESIGNER.to_owned()),
            None,
            TicketStatus::Todo,
            crate::protocol::Priority::Normal,
            "equipe",
        );
        let Some(journal) = self.journal(squad) else { return };
        let Ok(review_id) = journal.create_ticket(&review) else { return };
        let _ = journal.ticket_event(review_id, "equipe", "created", &format!("daily design review of {} change(s)", waiting.len()));
        let dir = batch_dir(&home, squad, review_id);
        let _ = std::fs::create_dir_all(&dir);
        let covers: Vec<TicketId> = waiting.iter().map(|r| r.qa).collect();
        let row = DesignQaBatch { ticket: review_id, kind: REVIEW.into(), dir: Some(dir.to_string_lossy().into_owned()), ..DesignQaBatch::default() };
        if let Err(err) = journal.start_design_qa_batch(&row, &covers) {
            crate::daemon_log!("equipe: {squad}: daily design review T-{review_id} not kept: {err}");
        }
        // The audit: a random share of them, on the top tier.
        let keys: Vec<u64> = covers.iter().map(|&q| u64::from(q)).collect();
        let sampled: Vec<TicketId> = rules::sample(&keys, settings.sample_now(), u64::from(review_id)).into_iter().filter_map(|q| TicketId::try_from(q).ok()).collect();
        let mut audit_id = None;
        if !sampled.is_empty() {
            let picked: Vec<&DesignQaRun> = waiting.iter().filter(|r| sampled.contains(&r.qa)).collect();
            let audit = equipe_ticket(
                say!(self, "ticket.design_audit_title", count = picked.len()),
                format!("{}\n\n{}", say!(self, "ticket.design_audit_body", count = picked.len()), listed(&picked)),
                TicketKind::Task,
                Some(design_qa::DESIGNER.to_owned()),
                None,
                TicketStatus::Todo,
                crate::protocol::Priority::Normal,
                "equipe",
            );
            if let Some(journal) = self.journal(squad)
                && let Ok(id) = journal.create_ticket(&audit)
            {
                let _ = journal.ticket_event(id, "equipe", "created", &format!("design QA audit of {} change(s)", picked.len()));
                let adir = batch_dir(&home, squad, id);
                let _ = std::fs::create_dir_all(&adir);
                let row = DesignQaBatch { ticket: id, kind: AUDIT.into(), dir: Some(adir.to_string_lossy().into_owned()), ..DesignQaBatch::default() };
                let _ = journal.start_design_qa_batch(&row, &sampled);
                audit_id = Some(id);
            }
        }
        crate::daemon_log!("equipe: {squad}: daily design review T-{review_id} of {} change(s), {} audited", covers.len(), sampled.len());
        for id in std::iter::once(review_id).chain(audit_id) {
            if let Some(made) = self.journal(squad).and_then(|j| j.ticket(id).ok().flatten()) {
                self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: made });
            }
        }
        // Main's precheck since the last review, one per project, off the loop.
        let mut projects: Vec<PathBuf> = Vec::new();
        for t in &checked {
            if let Some(p) = &t.project
                && !projects.contains(p)
            {
                projects.push(p.clone());
            }
        }
        let Some(bin) = self.config.equipe_binary.clone().filter(|_| !projects.is_empty()) else { return };
        let bases: Vec<(PathBuf, Option<String>)> = projects
            .iter()
            .map(|p| {
                let kept = self.journal(squad).and_then(|j| j.design_qa_batch_base(p));
                let first = waiting.iter().filter(|r| checked.iter().any(|t| t.id == r.checked && t.project.as_ref() == Some(p))).find_map(|r| r.base.clone());
                (p.clone(), kept.or(first))
            })
            .collect();
        self.prechecks.insert((squad.to_owned(), review_id), std::time::Instant::now());
        let (internal, squad) = (self.internal.clone(), squad.to_owned());
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            let mut heads = Vec::new();
            for (project, base) in bases {
                let out = dir.join(format!("main-{}", board::project_name(&project)));
                heads.push((project.clone(), main_precheck(&bin, &project, &out, base).await));
            }
            let _ = internal.send(Internal::ReviewPrechecked { squad, review: review_id, heads, secs: started.elapsed().as_secs() });
        });
    }

    /// Main's precheck for daily review `review` ended: where main was kept
    /// as the next review's base, and the review goes on the board.
    pub(super) async fn on_review_prechecked(&mut self, squad: &str, review: TicketId, heads: Vec<(PathBuf, Result<String, String>)>, secs: u64) {
        self.prechecks.remove(&(squad.to_owned(), review));
        let mut kept = Vec::new();
        for (project, head) in &heads {
            match head {
                Ok(head) => {
                    if let Some(journal) = self.journal(squad) {
                        let _ = journal.set_design_qa_batch_base(project, head);
                    }
                    kept.push(format!("{}={head}", project.display()));
                }
                Err(why) => crate::daemon_log!("equipe: {squad}: daily design review T-{review}: main's precheck of {} failed: {why}", project.display()),
            }
        }
        if let Some(journal) = self.journal(squad) {
            let _ = journal.set_design_qa_batch_heads(review, &kept.join("\n"));
        }
        crate::daemon_log!("equipe: {squad}: daily design review T-{review}: main prechecked ({secs}s)");
        self.dispatch(squad).await;
    }

    /// `design_review`: a designer's word on a daily review or audit — fix
    /// tickets for each S1/S2, one polish ticket for the rest, an audit's
    /// misses counted — handed in when the turn ends.
    pub(super) fn finish_design_review(&mut self, squad: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let (on, designer) = {
            let s = self.sessions.get(&id).ok_or("You're not on the squad.")?;
            let work = s.work.as_ref().ok_or("You're not on a ticket — there is no design review to finish.")?;
            if work.verdict.is_some() {
                return Err("This review is finished already: end your turn now with a short summary.".into());
            }
            (work.ticket, s.name().unwrap_or_default().to_owned())
        };
        let journal = self.journal(squad).ok_or("The board can't be opened.")?;
        let batch = journal.design_qa_batch(on).ok_or_else(|| format!("T-{on} isn't a daily design review or audit: finish a design QA ticket with design_qa, other work with submit_for_review."))?;
        let audit = batch.kind == AUDIT;
        let items: Vec<DesignQaRun> = journal.design_qa_batch_items(on).into_iter().filter(|r| if audit { r.audit == Some(on) } else { r.batch == Some(on) }).collect();
        let expected: Vec<u64> = items.iter().map(|r| u64::from(r.checked)).collect();
        let review = Review::read(args, &expected)?;
        let tickets = self.tickets(squad);
        let mut fixes = 0u32;
        let mut polish_lines = Vec::new();
        let mut polish_owner: Option<String> = None;
        for item in &review.items {
            let checked = tickets.iter().find(|t| u64::from(t.id) == item.ticket);
            let title = checked.map(|t| t.title.clone()).unwrap_or_default();
            let owner = checked.and_then(|t| t.assignee.clone().or_else(|| t.owner.clone()));
            if !item.blockers.is_empty() {
                let body = format!(
                    "{}\n\n{}",
                    say!(self, "ticket.design_fix_body", id = item.ticket, review = on),
                    item.blockers.iter().map(|b| format!("- {b}")).collect::<Vec<_>>().join("\n")
                );
                let fix = equipe_ticket(say!(self, "ticket.design_fix_title", id = item.ticket, title = title), body, TicketKind::Code, owner.clone(), checked.and_then(|t| t.project.clone()), TicketStatus::Todo, crate::protocol::Priority::High, &designer);
                if let Some(journal) = self.journal(squad)
                    && let Ok(fid) = journal.create_ticket(&fix)
                {
                    let _ = journal.ticket_event(fid, &designer, "created", &format!("design fix of T-{} from T-{on}", item.ticket));
                    fixes += 1;
                    if let Some(made) = self.journal(squad).and_then(|j| j.ticket(fid).ok().flatten()) {
                        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: made });
                    }
                }
            }
            polish_lines.extend(item.polish.iter().map(|p| format!("- T-{}: {p}", item.ticket)));
            if !item.polish.is_empty() && polish_owner.is_none() {
                polish_owner = owner;
            }
            if audit && let Some(run) = items.iter().find(|r| u64::from(r.checked) == item.ticket) {
                let verdict = if item.blockers.is_empty() { "ok" } else { "miss" };
                if let Some(journal) = self.journal(squad) {
                    let _ = journal.set_design_qa_audit_verdict(run.qa, verdict);
                }
            }
        }
        // S3/S4 go together into one polish ticket, for the PM to schedule.
        if !polish_lines.is_empty() {
            let polish = equipe_ticket(
                say!(self, "ticket.design_polish_title", review = on),
                format!("{}\n\n{}", say!(self, "ticket.design_polish_body", review = on), polish_lines.join("\n")),
                TicketKind::Code,
                polish_owner,
                None,
                TicketStatus::Proposed,
                crate::protocol::Priority::Low,
                &designer,
            );
            if let Some(journal) = self.journal(squad)
                && let Ok(pid) = journal.create_ticket(&polish)
            {
                let _ = journal.ticket_event(pid, &designer, "created", &format!("design polish from T-{on}"));
                if let Some(made) = self.journal(squad).and_then(|j| j.ticket(pid).ok().flatten()) {
                    self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: made });
                }
            }
        }
        let (blockers, polish) = (review.blockers() as u32, review.polish() as u32);
        if let Some(journal) = self.journal(squad) {
            let _ = journal.finish_design_qa_batch(on, blockers, fixes, polish);
        }
        if audit {
            self.after_audit(squad);
        }
        let report = format!(
            "{}\n\n{} change(s) looked at: {blockers} S1/S2 finding(s), {fixes} fix ticket(s); {polish} polish point(s){}.",
            review.findings,
            review.items.len(),
            if polish > 0 { " gathered into one polish ticket" } else { "" }
        );
        let work = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()).ok_or("You're not on a ticket.")?;
        work.handed_in = Some(report.clone());
        work.waiting = None;
        work.verdict = Some(design_qa::Verdict { passed: true, findings: report });
        Ok(format!("T-{on} is finished when this turn ends: {fixes} fix ticket(s) filed. End your turn now with a short summary."))
    }

    /// An audit finished: the routing tightens when audits miss too much,
    /// and relaxes after a clean run — the person told either way.
    fn after_audit(&mut self, squad: &str) {
        let Some(journal) = self.journal(squad) else { return };
        let settings = journal.design_qa_settings();
        let latest = journal.design_qa_audits(0, rules::AUDIT_WINDOW);
        if !settings.tight && rules::should_tighten(&latest, settings.audit_miss) {
            let _ = journal.set_design_qa_tight(true);
            let window = latest.len().min(rules::AUDIT_WINDOW);
            let misses = latest.iter().take(window).filter(|m| **m).count();
            crate::daemon_log!("equipe: {squad}: design QA audits missed {misses} of {window}: routing tightened");
            let line = format!(
                "equipe: design QA audits found S1/S2 misses in {misses} of the last {window} changes let through without a designer, over the squad's {}% threshold: more changes now get a designer turn of their own, and more are audited, until audits come back clean.",
                settings.audit_miss
            );
            let shown = say!(self, "news.design_qa_tightened", misses = misses, of = window);
            self.brief_pm(squad, line, Some(shown));
        } else if settings.tight {
            let since = journal.design_qa_audits(journal.design_qa_tight_at(), rules::RELAX_CLEAN);
            if rules::may_relax(&since) {
                let _ = journal.set_design_qa_tight(false);
                crate::daemon_log!("equipe: {squad}: design QA audits clean: routing relaxed");
                let shown = say!(self, "news.design_qa_relaxed", clean = rules::RELAX_CLEAN);
                self.brief_pm(squad, format!("equipe: the last {} audited design changes were clean: design QA routing is back to normal.", rules::RELAX_CLEAN), Some(shown));
            }
        }
    }

    /// `qa_pictures`: the pictures the precheck took of the states asked
    /// for, at the conditions asked for. For a board server (`_server`),
    /// as JSON, with where to render what it didn't take.
    pub(super) fn qa_pictures(&mut self, squad: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let on = self.sessions.get(&id).and_then(|s| s.work.as_ref()).map(|w| w.ticket).ok_or("You're not on a ticket.")?;
        let home = self.config.home.clone();
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut render: Option<(PathBuf, PathBuf)> = None;
        if let Some(batch) = self.journal(squad).and_then(|j| j.design_qa_batch(on)) {
            let dir = batch.dir.map(PathBuf::from).unwrap_or_else(|| batch_dir(&home, squad, on));
            dirs.extend(std::fs::read_dir(&dir).map(|d| d.filter_map(Result::ok).map(|e| e.path()).collect::<Vec<_>>()).unwrap_or_default());
            dirs.extend(self.lite_reads(squad, on));
        } else {
            let dir = design_qa::precheck_dir(&home, squad, on);
            if let Some(worktree) = self.checked_by(squad, on).and_then(|c| c.worktree).filter(|w| w.exists()) {
                let n = std::fs::read_dir(dir.join("extra")).map(|d| d.count()).unwrap_or(0);
                render = Some((worktree, dir.join("extra").join(format!("{}", n + 1))));
            }
            dirs.push(dir.join("extra"));
            dirs.extend(std::fs::read_dir(dir.join("extra")).map(|d| d.filter_map(Result::ok).map(|e| e.path()).collect::<Vec<_>>()).unwrap_or_default());
            dirs.push(dir);
        }
        let states = board::arg_strs(args, "states");
        if states.is_empty() {
            return Err("Name the states you want in `states`: `flow/screen/state`, as the report writes them.".into());
        }
        let mut conditions = board::arg_strs(args, "conditions");
        if conditions.is_empty() {
            conditions = vec!["390.light.normal".into(), "1280.light.normal".into()];
        }
        let (found, missing) = rules::find_pictures(&dirs, &states, &conditions);
        if args.get("_server").and_then(Value::as_bool).unwrap_or(false) {
            let render = render.filter(|_| !missing.is_empty()).map(|(w, o)| json!({ "worktree": w, "out": o }));
            return Ok(json!({ "found": found, "missing": missing, "conditions": conditions, "render": render }).to_string());
        }
        Ok(rules::pictures_said(&found, &missing))
    }
}

/// The spec files `checked` names, read from its worktree: text excerpts
/// in the brief, pictures copied beside the report for the designer to Read.
fn spec_excerpts(checked: &Ticket, dir: &Path) -> String {
    let Some(root) = checked.worktree.clone().or_else(|| checked.project.clone()) else { return String::new() };
    let mut paths = rules::spec_paths(&checked.body);
    if let Some(plan) = checked.plan.as_deref().filter(|p| !Path::new(p).is_absolute()) {
        paths.insert(0, plan.to_owned());
    }
    let mut out = Vec::new();
    for rel in paths.into_iter().take(SPEC_FILES) {
        let file = root.join(&rel);
        let lower = rel.to_ascii_lowercase();
        if [".md", ".html", ".json"].iter().any(|k| lower.ends_with(k)) {
            if let Ok(text) = std::fs::read_to_string(&file) {
                out.push(format!("{rel}:\n{}", clip(&text, SPEC_CLIP)));
            }
        } else if file.is_file() {
            let to = dir.join("spec").join(rel.replace('/', "_"));
            let _ = std::fs::create_dir_all(dir.join("spec"));
            if std::fs::copy(&file, &to).is_ok() {
                out.push(format!("{rel} (a picture, copied to {} — Read it)", to.display()));
            }
        }
    }
    out.join("\n\n")
}

/// `equipe design precheck` of main in `project` against `base` (the last
/// review's main, or the oldest change's base), on a detached worktree made
/// for it and removed after; `Ok` with main's commit once its report is in `out`.
async fn main_precheck(bin: &Path, project: &Path, out: &Path, base: Option<String>) -> Result<String, String> {
    let (repo, wt) = (project.to_path_buf(), out.with_extension("wt"));
    let made = wt.clone();
    let head = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let (_, head) = worktree::main_head(&repo)?;
        let _ = std::fs::remove_dir_all(&made);
        if let Some(parent) = made.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        worktree::add_detached(&repo, &made, &head)?;
        worktree::seed_dependencies(&made);
        Ok(head)
    })
    .await
    .map_err(|e| e.to_string())??;
    let base = base.unwrap_or_else(|| format!("{head}~1"));
    let result = design_qa::precheck_process(bin, &wt, out, None, Some(&base)).await;
    let gone = wt.clone();
    let _ = tokio::task::spawn_blocking(move || worktree::remove_at(&gone)).await;
    result.map(|()| head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_light_brief_quotes_the_spec_files_the_ticket_names() {
        let root = std::env::temp_dir().join(format!("equipe-spec-excerpts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("design/specs")).unwrap();
        std::fs::write(root.join("design/specs/billing.md"), "Plans: three cards, 24px gap.").unwrap();
        std::fs::write(root.join("design/mock.png"), b"png").unwrap();
        let ticket = equipe_ticket("Billing".into(), "Per `design/specs/billing.md` and design/mock.png.".into(), TicketKind::Code, None, Some(root.clone()), TicketStatus::Review, Default::default(), "pm");
        let out = root.join("qa");
        let said = spec_excerpts(&ticket, &out);
        assert!(said.contains("three cards, 24px gap") && said.contains("copied to"), "{said}");
        assert!(out.join("spec").join("design_mock.png").is_file());
        let _ = std::fs::remove_dir_all(root);
    }
}
