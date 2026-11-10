//! The plan step on the board (`crate::plan`): the PM writes a plan with
//! `write_plan` and asks for its reviews in the same call — one review
//! ticket for each reviewer, all ready at once; a reviewer writes its own
//! notes into the plan with the same tool; the tickets cut from it with
//! `create_ticket` (`plan`, `tests`) carry its path and their slice of its
//! test plan, and are listed in its front matter. The file is kept in the
//! squad's own folder, never a project's repository, and saved there as a
//! change ([`briefs::save`]): its git work runs off the loop, one write at a
//! time, and the call is answered once it is in. Tickets name it by its
//! absolute path, which a member reads from its worktree.

use std::sync::Mutex;

use super::*;
use crate::briefs::{self, Kind, Target};
use crate::plan::{self, Change, Status};

/// One plan written at a time: each write reads the file, changes it and
/// saves it, and two at once would lose one's change.
static WRITING: Mutex<()> = Mutex::new(());

/// What a `write_plan` call did off the loop, back on it.
pub(super) struct Written {
    squad: String,
    member: String,
    reply: oneshot::Sender<ToolResult>,
    result: Result<Wrote, String>,
    /// Who the PM asked to review it.
    reviewers: Vec<String>,
    /// A reviewer wrote its notes.
    reviewed: bool,
}

pub(super) struct Wrote {
    /// What a ticket names it by: its absolute path in the squad's folder.
    plan: String,
    /// Where it is on this machine.
    path: PathBuf,
    title: String,
    project: Option<PathBuf>,
    status: Status,
}

/// A plan read off the loop for a ticket to be cut from it, back on the loop.
pub(super) struct Cut {
    call: ToolCall,
    reply: oneshot::Sender<ToolResult>,
    result: Result<Slice, String>,
}

pub(super) struct Slice {
    plan: String,
    /// What the ticket's body ends with: the plan, its test cases.
    block: String,
    project: Option<PathBuf>,
    target: Target,
    slug: String,
}

impl Runtime {
    /// Where a plan is kept, in the squad's own folder, and the project it is
    /// about: the one its absolute path names, else the project named, else
    /// the squad's only one, else — with none — the squad's own work.
    fn plan_target(&mut self, squad: &str, project: Option<&str>, named: Option<&str>) -> Result<(Target, Option<PathBuf>), String> {
        let projects = self.projects(squad);
        let folder = squad_home::path(&self.config.home, squad);
        let names = || projects.iter().map(|p| board::project_name(p)).collect::<Vec<_>>().join(", ");
        let of = |repo: &PathBuf| Target::project(&folder, &board::project_name(repo)).map(|t| (t, Some(repo.clone())));
        if let Some((target, Kind::Plan)) = named.map(Path::new).filter(|p| p.is_absolute()).and_then(|p| Target::of_path(&folder, p)) {
            let repo = target.project_name().and_then(|name| board::resolve_project(name, &projects));
            return Ok((target, repo));
        }
        match (project, projects.as_slice()) {
            (Some(arg), _) => board::resolve_project(arg, &projects).ok_or_else(|| format!("No project called {arg}. The squad's projects: {}.", names())).and_then(|repo| of(&repo)),
            (None, [one]) => of(one),
            (None, []) => Ok((Target::squad(&folder), None)),
            (None, _) => Err(format!("Which project is the plan for? Name it in `project`: {}.", names())),
        }
    }

    /// `write_plan`: checked here, written off the loop, answered when it is in.
    pub(super) fn write_plan(&mut self, call: &ToolCall, reply: oneshot::Sender<ToolResult>) {
        let fail = |reply: oneshot::Sender<ToolResult>, text: String| drop(reply.send(ToolResult { text, is_error: true, image: None }));
        let (squad, member, args) = (call.squad.clone(), call.member.clone(), &call.arguments);
        let Some(session) = self.member_id(&squad, &member).and_then(|id| self.sessions.get(&id)) else {
            return fail(reply, format!("{member} isn't on the squad {squad}."));
        };
        let is_pm = session.is_pm();
        let role = session.member.as_ref().map(|m| m.role.clone()).unwrap_or_default();
        let mut change = match Change::read(args, is_pm) {
            Ok(change) => change,
            Err(text) => return fail(reply, text),
        };
        if let Some((who, _)) = change.review.as_mut() {
            *who = plan::reviewer(&member, &role);
        }
        let reviewers: Vec<String> = if is_pm { board::arg_strs(args, "reviewers") } else { Vec::new() };
        if !reviewers.is_empty() && change.status.is_none() {
            change.status = Some(Status::InReview);
        }
        let named = board::arg_str(args, "plan").map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned);
        let slug = match named.as_deref() {
            Some(arg) => plan::slug_of(arg),
            None => board::arg_str(args, "slug").map(briefs::slugify).or_else(|| change.title.as_deref().map(briefs::slugify)),
        };
        let Some(slug) = slug.filter(|s| briefs::is_slug(s)) else {
            return fail(
                reply,
                match named.as_deref() {
                    Some(arg) => format!("`{arg}` doesn't name a plan: give its path (`…/plans/<date>-<slug>.md`) or slug."),
                    None if change.title.is_none() => "A new plan needs a `title`; to change one, name it in `plan`.".into(),
                    None => "Give the plan a `slug` — a few lowercase ASCII words joined by dashes (`magic-link-sign-in`): its title makes none.".into(),
                },
            );
        };
        if named.is_none() && !is_pm {
            return fail(reply, "Which plan? Name it in `plan`, as your review ticket does.".into());
        }
        if change.is_empty() && reviewers.is_empty() {
            return fail(reply, "Nothing to write: give the sections, the status or scope that change, or `reviewers`.".into());
        }
        let (target, project) = match self.plan_target(&squad, board::arg_str(args, "project"), named.as_deref()) {
            Ok(found) => found,
            Err(text) => return fail(reply, text),
        };
        let (lang, internal, reviewed) = (self.config.lang, self.internal.clone(), change.review.is_some());
        tokio::task::spawn_blocking(move || {
            let result = (|| {
                let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
                let today = squad_memory::date(now_secs());
                let existing = briefs::read(&target, Kind::Plan, &slug);
                if existing.is_none() && named.is_some() {
                    return Err(format!("There is no plan `{slug}` in {}.", target.root().display()));
                }
                let text = plan::apply(existing.as_deref(), &change, &today, lang)?;
                let path = briefs::save(&target, Kind::Plan, &slug, &text, &today, &member)?;
                let (front, _) = plan::parse(&text);
                Ok(Wrote { plan: path.to_string_lossy().into_owned(), path, title: front.title, project, status: front.status })
            })();
            let _ = internal.send(Internal::PlanWritten(Box::new(Written { squad, member, reply, result, reviewers, reviewed })));
        });
    }

    /// A plan written: its reviews asked for, the PM told of a review, the
    /// project's page brought up to date, and the call answered.
    pub(super) fn plan_written(&mut self, written: Written) {
        let Written { squad, member, reply, result, reviewers, reviewed } = written;
        let wrote = match result {
            Ok(wrote) => wrote,
            Err(text) => return drop(reply.send(ToolResult { text: format!("The plan wasn't written: {text}"), is_error: true, image: None })),
        };
        self.docs_changed(&squad);
        let mut text = format!("Wrote {} ({}).", wrote.path.display(), wrote.status.label());
        if reviewed {
            let shown = say!(self, "news.plan_reviewed", name = member, path = wrote.plan);
            self.brief_pm(&squad, format!("{member} wrote its review into the plan {}.", wrote.plan), Some(shown));
            text.push_str(" Your notes are in it, under your name; the PM is told. Hand in now with a two-line summary.");
        }
        let mut asked = Vec::new();
        for owner in reviewers {
            match self.ask_plan_review(&squad, &member, &wrote, &owner) {
                Ok(id) => asked.push(format!("{owner} (T-{id})")),
                Err(why) => text.push_str(&format!(" {owner} wasn't asked: {why}.")),
            }
        }
        if !asked.is_empty() {
            text.push_str(&format!(
                " Asked {} to review it, all at once: each writes its notes into the plan and hands in. When they are in, \
take in what they found, put the scope to the person on one `ask_decision` card, then cut the tickets with \
`create_ticket` (`plan`, `tests`).",
                asked.join(", ")
            ));
        } else if !reviewed {
            text.push_str(" Give the person its path in backticks in one line.");
        }
        let _ = reply.send(ToolResult { text, is_error: false, image: None });
    }

    /// A review ticket for `owner`, ready at once: from its role's angle, the plan named.
    fn ask_plan_review(&mut self, squad: &str, pm: &str, wrote: &Wrote, owner: &str) -> Result<TicketId, String> {
        let role = self.team(squad).into_iter().find(|(_, name, role)| name == owner || role == owner).map_or_else(|| owner.to_owned(), |(_, _, role)| role);
        let (title, body) = plan::review_ticket(&wrote.title, &wrote.plan, &role, self.config.lang);
        let ticket = Ticket {
            id: 0,
            title,
            body,
            status: TicketStatus::Todo,
            kind: TicketKind::Task,
            owner: Some(owner.to_owned()),
            assignee: None,
            project: wrote.project.clone(),
            depends_on: Vec::new(),
            branch: None,
            worktree: None,
            merged: None,
            off_main: None,
            design_qa: None,
            plan: Some(wrote.plan.clone()),
            created_by: pm.to_owned(),
            note: None,
            asks: Vec::new(),
            remaining: Vec::new(),
            images: Vec::new(),
            updated_at: None,
            priority: Default::default(),
            rank: 0,
        };
        let journal = self.journal(squad).ok_or("the board can't be opened")?;
        let id = journal.create_ticket(&ticket).map_err(|e| e.to_string())?;
        let _ = journal.ticket_event(id, pm, "created", &ticket.title);
        self.tell(squad, crate::telemetry::Happened::TicketCreated { kind: TicketKind::Task, by: crate::telemetry::Maker::Pm });
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: Ticket { id, ..ticket } });
        Ok(id)
    }

    /// `create_ticket` with `plan`: the plan read off the loop and the
    /// ticket's slice of its test plan taken, then the ticket made.
    pub(super) fn cut_from_plan(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let fail = |reply: oneshot::Sender<ToolResult>, text: String| drop(reply.send(ToolResult { text, is_error: true, image: None }));
        let named = board::arg_str(&call.arguments, "plan").unwrap_or_default().to_owned();
        let Some(slug) = plan::slug_of(&named).filter(|s| briefs::is_slug(s)) else {
            return fail(reply, format!("`{named}` doesn't name a plan: give its path (`…/plans/<date>-<slug>.md`) or slug."));
        };
        let (target, project) = match self.plan_target(&call.squad, board::arg_str(&call.arguments, "project"), Some(&named)) {
            Ok(found) => found,
            Err(text) => return fail(reply, text),
        };
        let tests = board::arg_strs(&call.arguments, "tests");
        let (lang, internal) = (self.config.lang, self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let result = (|| {
                let text = briefs::read(&target, Kind::Plan, &slug).ok_or_else(|| format!("There is no plan `{slug}` in {} — write it first with `write_plan`.", target.root().display()))?;
                let name = briefs::find(&target, Kind::Plan, &slug).unwrap_or_default();
                let plan = target.path(Kind::Plan, &name).to_string_lossy().into_owned();
                let (_, body) = plan::parse(&text);
                let cases = plan::slice(body.section(plan::Section::TestPlan), &tests)
                    .map_err(|id| format!("The plan's test plan has no {id}: name its cases as it numbers them (`TP-1`), or write the case into it first."))?;
                let block = plan::ticket_block(&plan, &cases, lang);
                Ok(Slice { plan, block, project, target, slug })
            })();
            let _ = internal.send(Internal::PlanCut(Box::new(Cut { call, reply, result })));
        });
    }

    /// The plan read: the ticket made with its slice, then listed in the plan.
    pub(super) async fn plan_cut(&mut self, cut: Cut) {
        let Cut { mut call, reply, result } = cut;
        let slice = match result {
            Ok(slice) => slice,
            Err(text) => return drop(reply.send(ToolResult { text, is_error: true, image: None })),
        };
        let squad = call.squad.clone();
        if let Some(args) = call.arguments.as_object_mut() {
            let body = args.get("body").and_then(Value::as_str).unwrap_or_default().trim_end().to_owned();
            let body = if body.is_empty() { slice.block.clone() } else { format!("{body}\n\n{}", slice.block) };
            args.insert("body".into(), Value::String(body));
            args.insert("plan".into(), Value::String(slice.plan.clone()));
            args.remove("tests");
            if !args.contains_key("project")
                && let Some(project) = &slice.project
            {
                args.insert("project".into(), Value::String(board::project_name(project)));
            }
        }
        let (member, plan_ref) = (call.member.clone(), slice.plan.clone());
        let result = self.run_tool(call);
        self.dispatch(&squad).await;
        let made = result.is_ok().then(|| self.tickets(&squad).into_iter().filter(|t| t.plan.as_deref() == Some(plan_ref.as_str()) && t.created_by == member).map(|t| t.id).max()).flatten();
        let _ = reply.send(match result {
            Ok(text) => ToolResult { text: format!("{text} It links to the plan and carries its cases; the plan lists it."), is_error: false, image: None },
            Err(text) => ToolResult { text, is_error: true, image: None },
        });
        if let Some(id) = made {
            self.list_in_plan(&squad, slice, id);
        }
    }

    /// Ticket `id` listed in the plan's `tickets`, off the loop; the plan settled.
    fn list_in_plan(&mut self, squad: &str, slice: Slice, id: TicketId) {
        let (lang, internal, squad) = (self.config.lang, self.internal.clone(), squad.to_owned());
        tokio::task::spawn_blocking(move || {
            let result = (|| {
                let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
                let text = briefs::read(&slice.target, Kind::Plan, &slice.slug).ok_or_else(|| "it is gone".to_owned())?;
                let (mut front, body) = plan::parse(&text);
                front.add_ticket(id);
                briefs::save(&slice.target, Kind::Plan, &slice.slug, &plan::text(&front, &body, lang), &squad_memory::date(now_secs()), "equipe").map(drop)
            })();
            let error = result.err().map(|why| format!("T-{id} isn't listed in the plan {}: {why}.", slice.plan));
            let _ = internal.send(Internal::PlanListed(squad, error));
        });
    }

    pub(super) fn plan_listed(&mut self, squad: &str, error: Option<String>) {
        if let Some(line) = error {
            self.brief_pm(squad, line, None);
        }
        self.docs_changed(squad);
    }
}
