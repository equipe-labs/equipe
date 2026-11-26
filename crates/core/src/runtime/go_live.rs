//! Going live after a landing (`crate::go_live`): once work lands and main is
//! pushed — a member's own landing, the PM's acceptance, the person's merge —
//! the project's go-live command runs in its main checkout, off the loop,
//! once for landings close together and one run at a time per project.
//! What it deployed is noted as a deploy (`crate::deploys`), told like any
//! other; a run that failed, or couldn't start, is told to the PM with the
//! reason and the last lines it printed, and in the person's news. A project
//! with no command declared but an obvious one in its files gets the PM
//! asked, once, to confirm it (`set_deploy`); nothing undeclared ever runs.

use std::collections::BTreeMap;

use super::deploys::key_of;
use super::*;
use crate::deploys::{Channel, Live, Looked};
use crate::go_live::{self as rules, Blocked, Found, Lane, Outcome};
use crate::protocol::autonomy::{Kind, Level};
use equipe_protocol::deploys::DeploySource;

impl Runtime {
    /// `squad`'s lanes, read from its journal the first time.
    pub(super) fn lanes(&mut self, squad: &str) -> &mut BTreeMap<String, Lane> {
        if !self.go_live.contains_key(squad) {
            let kept = self.journals.get(squad).map(Journal::go_live).unwrap_or_default();
            self.go_live.insert(squad.to_owned(), kept);
        }
        self.go_live.entry(squad.to_owned()).or_default()
    }

    pub(super) fn keep_lanes(&mut self, squad: &str) {
        let lanes = self.lanes(squad).clone();
        if let Some(journal) = self.journals.get(squad)
            && let Err(err) = journal.set_go_live(&lanes)
        {
            crate::daemon_log!("equipe: {squad}: how its projects go live wasn't kept: {err}");
        }
    }

    /// Work of `ticket` (none: the PM asked) landed in `project` and main was
    /// pushed: noted, and what the project goes live with read off the loop.
    pub(super) fn go_live_landed(&mut self, squad: &str, project: PathBuf, ticket: Option<TicketId>) {
        let key = key_of(&project);
        let now = now_secs();
        let lane = self.lanes(squad).entry(key.clone()).or_default();
        lane.landed(ticket, now);
        let confirmed = lane.confirmed.clone();
        self.keep_lanes(squad);
        let watch = self.journals.get(squad).map(Journal::deploy_watch).unwrap_or_default().remove(&key).unwrap_or_default();
        // GitHub said how it goes live from a push (Vercel, a deploy workflow): nothing to suggest.
        let channel_known = !watch.channel_failed && matches!(watch.channel, Some(Channel::Deployments | Channel::Workflows { .. }));
        let (internal, squad) = (self.internal.clone(), squad.to_owned());
        tokio::task::spawn_blocking(move || {
            let found = match worktree::repo_root(&project) {
                Ok(repo) => rules::find(&repo, confirmed.as_deref()),
                Err(_) => Found::Undeclared { suggestion: None },
            };
            let _ = internal.send(Internal::GoLiveRead { squad, project, found, channel_known });
        });
    }

    /// What `project` goes live with is read: its landings wait for the quiet
    /// time — or, where deploys wait for the person, for the PM to ask them;
    /// a project with none declared but an obvious one has the PM asked once.
    pub(super) fn go_live_read(&mut self, squad: &str, project: PathBuf, found: Found, channel_known: bool) {
        let key = key_of(&project);
        let name = board::project_name(&project);
        let ask = self.autonomy_store(squad).policy.level(Kind::Deploy) == Level::Ask;
        let lane = self.lanes(squad).entry(key).or_default();
        match found {
            Found::Command(go) if ask => {
                let tickets = lane.pending.take().map(|p| p.tickets).unwrap_or_default();
                lane.declared = true;
                lane.stuck = Some("deploy_waits".into());
                self.keep_lanes(squad);
                self.requests_of_moved(squad, &tickets);
                crate::daemon_log!("equipe: {squad}: {name} is on main, not deployed: the squad's policy asks before deploys");
                let line = format!(
                    "Work landed in {name} and main is pushed ({}), but {name} is NOT live: the squad's policy asks the person before a \
deploy. Ask them whether to deploy now (`{}`); on their yes, call `set_deploy` with `project: \"{name}\"` and `deploy_now: true`.",
                    tickets.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", "),
                    go.command
                );
                let shown = say!(self, "news.deploy_waits", project = name);
                self.brief_pm(squad, line, Some(shown));
            }
            Found::Command(go) => {
                lane.quiet = go.quiet;
                lane.declared = true;
                self.keep_lanes(squad);
                self.go_live_schedule(squad, &project);
            }
            Found::Invalid(err) => {
                lane.pending = None;
                lane.declared = true;
                lane.stuck = Some("deploy_invalid".into());
                let again = lane.failed.as_deref() == Some(err.as_str());
                lane.failed = Some(err.clone());
                self.keep_lanes(squad);
                if !again {
                    self.tell_invalid(squad, &name, &err);
                }
            }
            Found::Undeclared { suggestion } => {
                lane.pending = None;
                lane.declared = false;
                lane.stuck = None;
                let ask_pm = suggestion.filter(|s| !channel_known && lane.asked.as_deref() != Some(s.as_str()));
                if let Some(command) = &ask_pm {
                    lane.asked = Some(command.clone());
                }
                self.keep_lanes(squad);
                if let Some(command) = ask_pm {
                    crate::daemon_log!("equipe: {squad}: {name} declares no go-live command; the PM is asked to confirm `{command}`");
                    self.brief_pm(
                        squad,
                        format!(
                            "Work landed in {name} and main is pushed, but equipe can't tell it went live: {name} has no {}, and \
GitHub shows no production deployment or deploy workflow equipe can confirm. Its own files suggest it deploys with `{command}`. \
If that is how it goes to production, confirm it with `set_deploy` (`project: \"{name}\"`, `command`) — equipe then runs it after \
each landing — else ask the person how it goes live. Until then its landed work is on main, not live: never tell anyone it is.",
                            rules::FILE
                        ),
                        None,
                    );
                }
            }
        }
    }

    /// `project`'s landings come due at their time: a wake then.
    fn go_live_schedule(&mut self, squad: &str, project: &Path) {
        let Some(at) = self.lanes(squad).get(&key_of(project)).and_then(Lane::due_at) else { return };
        let wait = Duration::from_secs(at.saturating_sub(now_secs()));
        let (internal, squad, project) = (self.internal.clone(), squad.to_owned(), project.to_owned());
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let _ = internal.send(Internal::GoLiveDue { squad, project });
        });
    }

    /// Each minute: landings a restart left waiting go live once due.
    pub(super) fn go_live_tick(&mut self) {
        let now = now_secs();
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            let due: Vec<String> = self.lanes(&squad).iter().filter(|(_, l)| l.is_due(now)).map(|(k, _)| k.clone()).collect();
            for key in due {
                self.go_live_due(&squad, PathBuf::from(key));
            }
        }
    }

    /// `project`'s landings may be due: its command runs off the loop if they
    /// are and none runs now; else they wait on.
    pub(super) fn go_live_due(&mut self, squad: &str, project: PathBuf) {
        let key = key_of(&project);
        let now = now_secs();
        let Some(lane) = self.lanes(squad).get_mut(&key) else { return };
        if lane.running || lane.pending.is_none() {
            return;
        }
        if !lane.is_due(now) {
            return self.go_live_schedule(squad, &project);
        }
        let tickets = lane.start();
        let confirmed = lane.confirmed.clone();
        let deployed = lane.deployed.clone();
        self.keep_lanes(squad);
        let from = self.journals.get(squad).and_then(|j| j.deploy_watch().remove(&key)).and_then(|w| w.live).or(deployed);
        let log = self.config.home.join("deploys").join(format!("{squad}-{}.log", board::project_name(&project)));
        let (internal, squad) = (self.internal.clone(), squad.to_owned());
        tokio::task::spawn_blocking(move || {
            let outcome = rules::go(&project, confirmed.as_deref(), from.as_deref(), &log);
            let _ = internal.send(Internal::GoLiveRan { squad, project, tickets, outcome });
        });
    }

    /// One go at deploying `project` ended: what went live noted and told;
    /// why not told to the PM and in the person's news. Landings that came
    /// meanwhile go next.
    pub(super) fn go_live_ran(&mut self, squad: &str, project: PathBuf, tickets: Vec<TicketId>, outcome: Outcome) {
        let key = key_of(&project);
        let path = project.clone();
        let name = board::project_name(&project);
        let now = now_secs();
        let after = after_words(&tickets);
        let again = "equipe deploys again after the next landing, or at once with `set_deploy` (`deploy_now: true`)";
        match outcome {
            Outcome::Went { command, sha, took, from, merges } => {
                let short = rules::short(&sha).to_owned();
                crate::daemon_log!("equipe: {squad}: {name} is live at {short}: `{command}` succeeded in {}s", took.as_secs());
                let back = self.lanes(squad).entry(key.clone()).or_default().finished(Ok(&sha), now);
                // Noted as a deploy, told like any other; watching main goes on from it.
                let mut watch = self.journals.get(squad).map(Journal::deploy_watch).unwrap_or_default().remove(&key).unwrap_or_default();
                let url = watch.site.clone();
                watch.live = Some(sha.clone());
                let live = Live { sha, at: now, url, environment: None, source: DeploySource::Command };
                self.note_looks(squad, vec![(project, watch, Looked::Live { from, live, merges })]);
                let line = if back {
                    format!("{name} is live again at {short}: `{command}` succeeded, run {after}.")
                } else {
                    format!("{name} is live at {short}: equipe ran `{command}` {after}, and it succeeded.")
                };
                self.brief_pm(squad, line, None);
            }
            Outcome::Blocked { command, blocked } => {
                let why = blocked.why();
                crate::daemon_log!("equipe: {squad}: {name} wasn't deployed: {why}");
                let lane = self.lanes(squad).entry(key).or_default();
                lane.finished(Err(&why), now);
                lane.stuck = Some("deploy_blocked".into());
                let fix = match &blocked {
                    Blocked::Dirty { .. } => "Get those files committed through a ticket, or cleared if they are leftovers, ",
                    Blocked::NotAtOrigin { .. } => "Get main and origin's main together again (push main, or merge origin's in), ",
                    _ => "Get that sorted, ",
                };
                let shown = self.blocked_words(&name, &blocked);
                self.brief_pm(squad, format!("{name} is NOT live: equipe didn't run `{command}` {after}, because {why}. {fix}then {again}."), Some(shown));
            }
            Outcome::Failed { command, sha, ran, timeout } => {
                let short = rules::short(&sha);
                let why = if ran.timed_out { format!("it ran longer than {} min and was stopped", timeout.as_secs() / 60) } else { format!("it exited with {}", ran.code.map_or("no code".into(), |c| c.to_string())) };
                crate::daemon_log!("equipe: {squad}: deploying {name} at {short} failed: {why}: {}", ran.tail.lines().last().unwrap_or_default());
                let lane = self.lanes(squad).entry(key).or_default();
                lane.finished(Err(&why), now);
                lane.stuck = Some("deploy_failed".into());
                let shown = if ran.timed_out {
                    say!(self, "news.deploy_timed_out", project = name, command = command, minutes = timeout.as_secs() / 60)
                } else {
                    say!(self, "news.deploy_failed", project = name, command = command)
                };
                let tail = if ran.tail.is_empty() { "(it printed nothing)".to_owned() } else { ran.tail.clone() };
                self.brief_pm(
                    squad,
                    format!(
                        "{name} is NOT live: equipe ran `{command}` on {short} {after}, and {why}. Its last lines:\n```\n{tail}\n```\n\
Find out why and get it fixed (a ticket, if it is the project's code); then {again}. Until it succeeds, that work is on main, not live."
                    ),
                    Some(shown),
                );
            }
            Outcome::Invalid(err) => {
                crate::daemon_log!("equipe: {squad}: {name} wasn't deployed: {err}");
                let lane = self.lanes(squad).entry(key).or_default();
                lane.finished(Err(&err), now);
                lane.stuck = Some("deploy_invalid".into());
                self.tell_invalid(squad, &name, &err);
            }
            Outcome::Gone => {
                crate::daemon_log!("equipe: {squad}: {name} has no go-live command any more: nothing deployed");
                if let Some(lane) = self.lanes(squad).get_mut(&key) {
                    lane.running = false;
                    lane.declared = false;
                    lane.stuck = None;
                }
                self.brief_pm(squad, format!("Nothing was deployed for {name}: it declares no {} and no command is confirmed with `set_deploy`.", rules::FILE), None);
            }
        }
        self.keep_lanes(squad);
        // The requests its work serves move with it: live, or on main and why not.
        self.requests_of_moved(squad, &tickets);
        self.go_live_schedule(squad, &path);
    }

    /// Whether work on main is live, as the squad's deploys say
    /// (`crate::requests::Shipped`): the tickets a deploy equipe saw succeed
    /// carried, and the projects that go live only by a deploy — a command
    /// of their own, or a production deployment or deploy workflow GitHub
    /// confirmed — with why theirs can't now.
    pub(super) fn shipped(&self, squad: &str) -> crate::requests::Shipped {
        let Some(journal) = self.journals.get(squad) else { return crate::requests::Shipped::default() };
        let mut live: std::collections::HashSet<TicketId> = journal.deploys().iter().filter(|d| d.source.is_live()).flat_map(|d| d.tickets.iter().copied()).collect();
        let lanes = self.go_live.get(squad).cloned().unwrap_or_else(|| journal.go_live());
        let mut by_deploy: std::collections::HashMap<String, Option<String>> =
            lanes.into_iter().filter(|(_, l)| l.by_deploy()).map(|(k, l)| (k, l.stuck)).collect();
        for (key, watch) in journal.deploy_watch() {
            if !watch.channel_failed && matches!(watch.channel, Some(Channel::Deployments | Channel::Workflows { .. })) {
                by_deploy.entry(key).or_insert(None);
            }
        }
        // A cloud team: the platform's live commits say what is live (`crate::cloud_live`).
        if let Some(carried) = self.cloud_carried(squad) {
            live.extend(carried.live);
            for key in carried.by_deploy {
                by_deploy.entry(key).or_insert(None);
            }
        }
        crate::requests::Shipped { live, by_deploy }
    }

    /// For a cloud team (a platform key on this machine) whose apps' live
    /// commits are known: what they carry of its landings on main. `None`
    /// on a desktop squad, or before the platform answered once.
    pub(super) fn cloud_carried(&self, squad: &str) -> Option<crate::cloud_live::Carried> {
        let commits = crate::sprint::platform::Platform::discover().and_then(|p| crate::sprint::platform::live_commits_cached(&p))?;
        if commits.is_empty() {
            return None;
        }
        let journal = self.journals.get(squad)?;
        Some(crate::cloud_live::carried(&journal.deploys(), &commits))
    }

    /// Each minute, for a cloud team (`crate::cloud_live`): work the
    /// platform's live version now carries moves its requests (live, the
    /// person told as usual); work on main it still lacks past
    /// [`crate::cloud_live::NOT_LIVE_AFTER`] is brought to the PM, once.
    pub(super) async fn watch_cloud_live(&mut self, squad: &str) {
        let Some(carried) = self.cloud_carried(squad) else { return };
        let before = self.cloud_live_seen.insert(squad.to_owned(), carried.live.clone());
        let newly: Vec<TicketId> = match &before {
            Some(b) => carried.live.difference(b).copied().collect(),
            None => carried.live.iter().copied().collect(),
        };
        if !newly.is_empty() {
            self.requests_of_moved(squad, &newly);
        }
        let Some(journal) = self.journals.get(squad) else { return };
        let mut told = journal.not_live_told();
        let still: std::collections::HashSet<TicketId> = carried.waiting.iter().map(|(t, _)| *t).collect();
        let was = told.len();
        told.retain(|t| still.contains(t));
        let now = now_secs();
        let due = crate::cloud_live::due(&carried.waiting, &told, now);
        if due.is_empty() {
            if told.len() != was {
                let _ = journal.set_not_live_told(&told);
            }
            return;
        }
        let tickets = self.tickets(squad);
        let due: Vec<(TicketId, String, u64)> = due.into_iter().filter_map(|(t, at)| tickets.iter().find(|x| x.id == t).map(|x| (t, x.title.clone(), at))).collect();
        told.extend(due.iter().map(|(t, _, _)| *t));
        if let Some(Err(err)) = self.journals.get(squad).map(|j| j.set_not_live_told(&told)) {
            crate::daemon_log!("equipe: {squad}: which work the PM was told isn't live wasn't kept: {err}");
            return;
        }
        if due.is_empty() {
            return;
        }
        let Some(pm) = self.pm_of(squad) else { return };
        // Main goes live by itself after each landing here: its deploy tells the PM how it went (`go_live_ran`).
        // A nudge to deploy by hand made the PM file tickets only to deploy, and came again after deploys.
        if crate::go_live::cloud() {
            crate::daemon_log!("equipe: {squad}: {} landed ticket(s) not on the live site yet; main deploys by itself, no nudge", due.len());
            return;
        }
        let ask = self.autonomy_store(squad).policy.level(Kind::Deploy) == Level::Ask;
        crate::daemon_log!("equipe: {squad}: {} landed ticket(s) are on main but not on the live site; the PM is told", due.len());
        self.hand_to_pm(squad, pm, crate::cloud_live::nudge_message(&due, ask, now)).await;
    }

    /// The requests `tickets` serve moved: told where they stand now.
    pub(super) fn requests_of_moved(&mut self, squad: &str, tickets: &[TicketId]) {
        for &t in tickets {
            self.request_question_changed(squad, t);
        }
    }

    /// The person's news for a deploy that couldn't start.
    fn blocked_words(&self, name: &str, blocked: &Blocked) -> String {
        match blocked {
            Blocked::Dirty { files, .. } => say!(self, "news.deploy_dirty", project = name, files = files.join(", ")),
            Blocked::NotAtOrigin { main, .. } => say!(self, "news.deploy_not_at_origin", project = name, branch = main),
            Blocked::From { from, main } => say!(self, "news.deploy_other_branch", project = name, from = from, branch = main),
            Blocked::NotCheckedOut { main } => say!(self, "news.deploy_no_checkout", project = name, branch = main),
            Blocked::NoMain(_) => say!(self, "news.deploy_no_checkout", project = name, branch = "main"),
        }
    }

    /// A deploy file that can't be read: the PM hears what is wrong, the person that nothing deployed.
    fn tell_invalid(&mut self, squad: &str, name: &str, err: &str) {
        let shown = say!(self, "news.deploy_invalid", project = name, file = rules::FILE);
        self.brief_pm(squad, format!("{name} is NOT live: {err}. Nothing runs until the file is fixed on main (a ticket); landed work stays on main, not live."), Some(shown));
    }

    /// `set_deploy`: the PM confirms (or stops) how a project goes live, or deploys it now.
    pub(super) fn set_deploy(&mut self, squad: &str, pm: SessionId, args: &Value) -> Result<String, String> {
        let projects = self.projects(squad);
        let project = match board::arg_str(args, "project") {
            Some(arg) => board::resolve_project(arg, &projects).ok_or_else(|| format!("There is no project {arg:?} in the squad."))?,
            None => match projects.as_slice() {
                [one] => one.clone(),
                _ => return Err("Which project? Give `project`, by name.".into()),
            },
        };
        let command = args.get("command").and_then(Value::as_str).map(str::trim);
        let now = args.get("deploy_now").and_then(Value::as_bool).unwrap_or(false);
        if command.is_none() && !now {
            return Err("Give `command` (the shell command the project deploys to production with; \"\" stops it), or `deploy_now: true`.".into());
        }
        if command.is_some_and(|c| c.contains('\n')) {
            return Err("`command` is one line; put several steps in the project's own deploy script.".into());
        }
        let name = board::project_name(&project);
        let key = key_of(&project);
        let mut said = Vec::new();
        if let Some(command) = command {
            let lane = self.lanes(squad).entry(key.clone()).or_default();
            if command.is_empty() {
                lane.confirmed = None;
                crate::daemon_log!("equipe: {squad}: the PM stopped {name}'s go-live command");
                said.push(format!("{name} runs no go-live command after landings now (a {} on main still would).", rules::FILE));
                let shown = say!(self, "news.deploy_cleared", project = name);
                self.show_news_about(pm, shown, Some(name.clone()));
            } else {
                lane.confirmed = Some(command.to_owned());
                lane.failed = None;
                crate::daemon_log!("equipe: {squad}: the PM confirmed {name}'s go-live command: `{command}`");
                said.push(format!(
                    "Saved: after each landing that pushes main, equipe runs `{command}` in {name}'s main checkout (once for landings close \
together) and tells you and the person how it went. A {} on main wins over it.",
                    rules::FILE
                ));
                let shown = say!(self, "news.deploy_set", project = name, command = command);
                self.show_news_about(pm, shown, Some(name.clone()));
            }
        }
        if now {
            let lane = self.lanes(squad).entry(key).or_default();
            if lane.running {
                said.push(format!("A deploy of {name} is running now; another follows it."));
            } else {
                said.push(format!("Deploying {name} now: you hear how it went."));
            }
            lane.landed(None, now_secs());
            lane.now(now_secs());
            self.keep_lanes(squad);
            self.go_live_due(squad, project);
        } else {
            self.keep_lanes(squad);
        }
        Ok(said.join(" "))
    }
}

/// What a go was run for, for a line: `after T-3, T-4 landed`, or the PM's word.
fn after_words(tickets: &[TicketId]) -> String {
    match tickets {
        [] => "on your word".into(),
        _ => format!("after {} landed", tickets.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ")),
    }
}
