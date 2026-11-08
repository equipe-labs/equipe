//! What a member's command is really doing on its machine, from the
//! processes under the member's engine ([`equipe_protocol::watch`]).
//!
//! The pure part, tested without a process: reading `ps` (the same columns
//! on macOS and Linux), finding the engine's descendants, telling the
//! command's own processes from the engine's long-lived ones (its MCP
//! servers started with it; a command's processes start after its step
//! did), which one a person sees, and whether it looks stuck — never while
//! it uses the CPU. The runtime runs `ps` off its loop and acts on this
//! ([`crate::runtime`]).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use equipe_protocol::Engine;
use equipe_protocol::watch::{CommandWatch, WatchState, WatchedProcess};

/// How `ps` is asked: every process, whole command lines, no header —
/// columns `ps` has on macOS and Linux alike.
pub const PS_ARGS: [&str; 4] = ["-A", "-ww", "-o", "pid=,ppid=,stat=,etime=,time=,command="];

/// A command quiet this long, with its engine quiet as long, looks stuck.
pub const STUCK_AFTER: Duration = Duration::from_secs(3 * 60);
/// Under this share of one core since it went quiet, a command is idle.
const IDLE_SHARE: f64 = 0.01;
/// From this share of one core, it reads as busy.
const BUSY_PERCENT: u32 = 50;
/// A step this young with nothing under the engine is still starting.
const STARTING: Duration = Duration::from_secs(10);
/// A command's processes started after its step did, give or take this
/// (the step is seen when the engine says it, a moment before it runs).
const SLACK_SECS: u64 = 3;

/// One process, as `ps` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    /// `R` running, `S` sleeping, `T` stopped, `Z` a zombie…
    pub stat: String,
    /// How long it has run, in seconds.
    pub secs: u64,
    /// The CPU it used, user and system, in milliseconds.
    pub cpu_ms: u64,
    /// Its command line.
    pub command: String,
}

impl Proc {
    /// The program's name, as a person reads it: `sqlite3` for
    /// `/usr/bin/sqlite3 squad.db …`, `zsh` for a login shell's `-zsh`.
    pub fn name(&self) -> String {
        let line = self.command.trim();
        // An app's binary: its path has spaces, its name is after `MacOS/`.
        let program = match line.split_once(".app/Contents/MacOS/") {
            Some((_, rest)) => rest.split(" -").next().unwrap_or(rest),
            None => line.split_whitespace().next().unwrap_or(""),
        };
        let name = program.rsplit('/').next().unwrap_or(program).trim_start_matches('-');
        if name.is_empty() { "?".to_owned() } else { name.chars().take(40).collect() }
    }

    /// A shell or a wrapper around the command, not what it does.
    fn wraps(&self) -> bool {
        matches!(self.name().as_str(), "sh" | "bash" | "zsh" | "dash" | "fish" | "env" | "sandbox-exec" | "timeout" | "gtimeout" | "nice" | "nohup" | "script")
    }
}

/// `ps`'s lines, as processes; a line that doesn't read is left out.
pub fn parse_ps(text: &str) -> Vec<Proc> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Proc> {
    let mut rest = line.trim_start();
    let mut field = || {
        let end = rest.find(char::is_whitespace)?;
        let (word, after) = rest.split_at(end);
        rest = after.trim_start();
        Some(word)
    };
    let pid = field()?.parse().ok()?;
    let ppid = field()?.parse().ok()?;
    let stat = field()?.to_owned();
    let secs = clock_secs(field()?)? as u64;
    let cpu_ms = (clock_secs(field()?)? * 1000.0) as u64;
    Some(Proc { pid, ppid, stat, secs, cpu_ms, command: rest.trim_end().to_owned() })
}

/// `[[dd-]hh:]mm:ss[.cc]` in seconds: `ps`'s elapsed time on both systems,
/// and its CPU time — macOS writes `1:23.45`, Linux `00:01:23`.
fn clock_secs(text: &str) -> Option<f64> {
    let (days, clock) = match text.split_once('-') {
        Some((d, c)) => (d.parse::<f64>().ok()?, c),
        None => (0.0, text),
    };
    let mut secs = 0.0;
    for (i, part) in clock.rsplit(':').enumerate() {
        let n: f64 = part.parse().ok()?;
        secs += n * [1.0, 60.0, 3600.0].get(i)?;
    }
    Some(days * 86_400.0 + secs)
}

/// Every process under `root`, not `root` itself.
pub fn descendants(procs: &[Proc], root: u32) -> Vec<&Proc> {
    let mut children: HashMap<u32, Vec<&Proc>> = HashMap::new();
    for p in procs.iter().filter(|p| p.pid != p.ppid) {
        children.entry(p.ppid).or_default().push(p);
    }
    let mut out = Vec::new();
    let mut next = vec![root];
    let mut seen = HashSet::from([root]);
    while let Some(pid) = next.pop() {
        for child in children.get(&pid).into_iter().flatten() {
            if seen.insert(child.pid) {
                out.push(*child);
                next.push(child.pid);
            }
        }
    }
    out
}

/// The processes of the command under way: those under the engine that
/// started since its step did, with all under them — the engine's own
/// servers, started with it, are not. Where some of them name one of the
/// commands (`targets`, a command's first line), only those: what else
/// started meanwhile (a subagent's command, a hook) isn't this one.
pub fn command_procs<'a>(procs: &'a [Proc], engine: u32, step_age: Duration, targets: &[String]) -> Vec<&'a Proc> {
    let under = descendants(procs, engine);
    let since = step_age.as_secs() + SLACK_SECS;
    let fresh: HashSet<u32> = under.iter().filter(|p| p.secs <= since).map(|p| p.pid).collect();
    let roots: Vec<&Proc> = under.iter().copied().filter(|p| fresh.contains(&p.pid) && !fresh.contains(&p.ppid)).collect();
    let named: Vec<&Proc> = roots.iter().copied().filter(|p| targets.iter().any(|t| mentions(&p.command, t))).collect();
    let roots = if named.is_empty() { roots } else { named };
    let mut out = Vec::new();
    for root in roots {
        out.push(root);
        out.extend(descendants(procs, root.pid));
    }
    out
}

/// Whether a command line runs `target`: its start, up to the first quote
/// or `$` (an engine may quote it otherwise), is in the line.
fn mentions(line: &str, target: &str) -> bool {
    let plain: String = target.chars().take_while(|c| !matches!(c, '\'' | '"' | '`' | '$' | '\\')).take(40).collect();
    let plain = plain.trim();
    plain.chars().count() >= 4 && line.contains(plain)
}

/// The process a person sees for the command: the one that used the most
/// CPU since the last look, when one used any to speak of; else the deepest
/// that isn't a shell around it (`sqlite3`, not the `zsh -c` running it).
pub fn shown<'a>(command: &[&'a Proc], used: &HashMap<u32, u64>, window: Duration) -> Option<&'a Proc> {
    let floor = (window.as_millis() as f64 * IDLE_SHARE) as u64;
    let busiest = command.iter().copied().filter(|p| used.get(&p.pid).copied().unwrap_or(0) > floor).max_by_key(|p| used.get(&p.pid).copied().unwrap_or(0));
    if busiest.is_some() {
        return busiest;
    }
    let pids: HashSet<u32> = command.iter().map(|p| p.pid).collect();
    let depth = |p: &Proc| {
        let mut depth = 0;
        let mut at = p.ppid;
        while let Some(parent) = command.iter().find(|q| q.pid == at && pids.contains(&q.pid)) {
            depth += 1;
            at = parent.ppid;
        }
        depth
    };
    let deepest = |list: Vec<&'a Proc>| list.into_iter().max_by_key(|p| (depth(p), std::cmp::Reverse(p.secs)));
    deepest(command.iter().copied().filter(|p| !p.wraps()).collect()).or_else(|| deepest(command.to_vec()))
}

/// What the person and the loop know of one member's command, between looks.
#[derive(Debug, Clone)]
pub struct Watcher {
    pub engine: Engine,
    /// The last look: when, and the CPU the command had used by then, all of
    /// it and by process.
    last: Option<(Instant, u64, HashMap<u32, u64>)>,
    /// Since when the command has used next to no CPU, and what it had used then.
    idle: Option<(Instant, u64)>,
    /// Since when nothing has run under the engine for it.
    nothing: Option<Instant>,
}

/// What the loop knows at a look.
pub struct Seen<'a> {
    pub procs: &'a [Proc],
    /// The engine's process.
    pub engine_pid: u32,
    /// How long the oldest command still open has run, and the commands.
    pub step_age: Duration,
    pub targets: &'a [String],
    /// How long the engine has said nothing.
    pub quiet: Duration,
    /// The member waits for the person's answer to a request.
    pub asking: bool,
}

impl Watcher {
    pub fn new(engine: Engine) -> Self {
        Self { engine, last: None, idle: None, nothing: None }
    }

    /// A look: what runs for the command, and how it looks.
    pub fn look(&mut self, seen: &Seen<'_>, now: Instant) -> CommandWatch {
        let command = command_procs(seen.procs, seen.engine_pid, seen.step_age, seen.targets);
        let used: HashMap<u32, u64> = command.iter().map(|p| (p.pid, p.cpu_ms)).collect();
        let total: u64 = used.values().sum();
        // Used since the last look, by process — a process new since then, all of it.
        let (window, since_last, by_pid) = match &self.last {
            Some((at, before, by)) => {
                let fresh: HashMap<u32, u64> = used.iter().map(|(pid, ms)| (*pid, ms.saturating_sub(by.get(pid).copied().unwrap_or(0)))).collect();
                (now.saturating_duration_since(*at), total.saturating_sub(*before), fresh)
            }
            // A first look: each process's whole life, averaged.
            None => (Duration::from_secs(command.iter().map(|p| p.secs).max().unwrap_or(0).max(1)), total, used.clone()),
        };
        let cpu_percent = if window.is_zero() { 0 } else { (since_last as f64 * 100.0 / window.as_millis() as f64).round() as u32 };
        self.last = Some((now, total, used));
        let quiet_secs = seen.quiet.as_secs();
        let quiet = seen.quiet >= STUCK_AFTER;
        let quiet_secs_of = |since: Instant| now.saturating_duration_since(since).as_secs();
        let state = if command.is_empty() {
            self.idle = None;
            let since = *self.nothing.get_or_insert(now);
            let nothing_for = quiet_secs_of(since).max(seen.step_age.as_secs().saturating_sub(SLACK_SECS).min(quiet_secs));
            if seen.asking {
                WatchState::AwaitingYou
            } else if seen.step_age < STARTING {
                WatchState::Working
            } else if quiet && nothing_for >= STUCK_AFTER.as_secs() {
                WatchState::Stuck { idle_secs: nothing_for }
            } else {
                WatchState::Nothing
            }
        } else {
            self.nothing = None;
            let (since, before) = *self.idle.get_or_insert((now, total));
            let allowed = now.saturating_duration_since(since).as_millis() as f64 * IDLE_SHARE;
            if total < before {
                // A process of it ended: what it used goes with it.
                self.idle = Some((since, total));
            } else if (total - before) as f64 > allowed.max(IDLE_SHARE * 5000.0) {
                self.idle = Some((now, total));
            }
            let idle_for = self.idle.map_or(0, |(since, _)| quiet_secs_of(since));
            if seen.asking {
                WatchState::AwaitingYou
            } else if cpu_percent >= BUSY_PERCENT {
                WatchState::Busy
            } else if quiet && idle_for >= STUCK_AFTER.as_secs() {
                WatchState::Stuck { idle_secs: idle_for }
            } else {
                WatchState::Working
            }
        };
        let process = shown(&command, &by_pid, window).map(|p| WatchedProcess { name: p.name(), cpu_percent, secs: p.secs });
        CommandWatch { engine: self.engine, process, quiet_secs, state }
    }
}

/// What stopping the command ends, from a fresh look: its processes, deepest
/// first — never the engine, this process or anything not under the engine.
pub fn to_stop(procs: &[Proc], engine_pid: u32, step_age: Duration, targets: &[String]) -> Vec<Proc> {
    let own = std::process::id();
    let mut out: Vec<Proc> = command_procs(procs, engine_pid, step_age, targets).into_iter().filter(|p| p.pid > 1 && p.pid != engine_pid && p.pid != own).cloned().collect();
    // Children before their parents: a shell gone first would leave them to run on.
    out.reverse();
    out
}

/// Of the processes stopped, those still there a moment later: the same pid
/// running the same command, at least as old (a pid taken again is younger).
pub fn survivors(stopped: &[Proc], now: &[Proc]) -> Vec<u32> {
    stopped.iter().filter(|s| now.iter().any(|p| p.pid == s.pid && p.command == s.command && p.secs >= s.secs && !p.stat.starts_with('Z'))).map(|s| s.pid).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: &str = "    1     0 Ss   10-02:03:04  12:01.50 /sbin/launchd
  500     1 S       30:00   0:01.20 /Users/a/.local/bin/claude -p --output-format stream-json
  501   500 S       30:00   0:00.40 /Users/a/.local/bin/equipe mcp --squad atinum --member pm
  610   500 S       28:06   0:00.01 /bin/zsh -c -l source /Users/a/.claude/shell-snapshots/snapshot-zsh.sh && eval 'cd ~/.equipe/squads/atinum-squad; sqlite3 squad.db \"select count(*)\"' < /dev/null
  611   610 S       28:05   0:00.02 sqlite3 squad.db select count(*)
  700     1 S       01:00   0:00.00 /usr/bin/other
";

    fn p(pid: u32, ppid: u32, secs: u64, cpu_ms: u64, command: &str) -> Proc {
        Proc { pid, ppid, stat: "S".into(), secs, cpu_ms, command: command.into() }
    }

    #[test]
    fn ps_reads_alike_on_macos_and_linux() {
        let procs = parse_ps(MAC);
        assert_eq!(procs.len(), 6);
        assert_eq!(procs[0].secs, 10 * 86_400 + 2 * 3600 + 3 * 60 + 4);
        assert_eq!(procs[0].cpu_ms, 721_500);
        assert_eq!(procs[4], p(611, 610, 28 * 60 + 5, 20, "sqlite3 squad.db select count(*)"));
        let linux = parse_ps(" 4242  4200 R    1-01:00:00 00:02:03 cargo build --release\nnot a process line\n");
        assert_eq!(linux.len(), 1, "a line that doesn't read is left out");
        assert_eq!((linux[0].secs, linux[0].cpu_ms, linux[0].stat.as_str()), (90_000, 123_000, "R"));
    }

    #[test]
    fn a_program_reads_as_its_name() {
        assert_eq!(p(1, 0, 0, 0, "/usr/bin/sqlite3 squad.db").name(), "sqlite3");
        assert_eq!(p(1, 0, 0, 0, "-zsh").name(), "zsh");
        assert_eq!(p(1, 0, 0, 0, "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome --type=renderer").name(), "Google Chrome");
    }

    #[test]
    fn the_command_is_what_started_under_the_engine_since_its_step_not_the_engines_servers() {
        let procs = parse_ps(MAC);
        let pids = |list: Vec<&Proc>| list.iter().map(|p| p.pid).collect::<Vec<_>>();
        let mut all = pids(descendants(&procs, 500));
        all.sort();
        assert_eq!(all, [501, 610, 611], "under the engine, the engine not among them");
        let step = Duration::from_secs(28 * 60 + 6);
        let mut command = pids(command_procs(&procs, 500, step, &[]));
        command.sort();
        assert_eq!(command, [610, 611], "its MCP server started with it, long before");
        assert!(command_procs(&procs, 500, Duration::from_secs(60), &[]).is_empty(), "a step a minute old has none of those");
    }

    #[test]
    fn of_several_started_since_the_one_naming_the_command_is_it() {
        let procs = vec![
            p(500, 1, 900, 0, "claude"),
            p(610, 500, 100, 0, "/bin/zsh -c eval 'cargo test -p equipe-core'"),
            p(620, 500, 50, 0, "/bin/zsh -c eval 'npm run dev'"),
        ];
        let only = command_procs(&procs, 500, Duration::from_secs(100), &["cargo test -p equipe-core".into()]);
        assert_eq!(only.iter().map(|p| p.pid).collect::<Vec<_>>(), [610]);
        assert_eq!(command_procs(&procs, 500, Duration::from_secs(100), &["\"quoted\" all along".into()]).len(), 2, "naming none, all of them");
    }

    #[test]
    fn the_process_shown_is_the_busiest_else_the_deepest_that_isnt_a_shell() {
        let procs = parse_ps(MAC);
        let command = command_procs(&procs, 500, Duration::from_secs(1700), &[]);
        let idle: HashMap<u32, u64> = HashMap::from([(610, 0), (611, 0)]);
        assert_eq!(shown(&command, &idle, Duration::from_secs(5)).map(|p| p.pid), Some(611), "sqlite3, not the zsh running it");
        let busy_shell: HashMap<u32, u64> = HashMap::from([(610, 4000), (611, 0)]);
        assert_eq!(shown(&command, &busy_shell, Duration::from_secs(5)).map(|p| p.pid), Some(610), "the one at work");
    }

    fn engine_and(cpu_ms: u64, secs: u64) -> Vec<Proc> {
        vec![p(500, 1, 4000, 1000, "claude"), p(610, 500, secs, 0, "/bin/zsh -c eval 'x'"), p(611, 610, secs, cpu_ms, "cargo build")]
    }

    fn seen(procs: &[Proc], step: u64, quiet: u64) -> Seen<'_> {
        Seen { procs, engine_pid: 500, step_age: Duration::from_secs(step), targets: &[], quiet: Duration::from_secs(quiet), asking: false }
    }

    #[test]
    fn a_long_build_that_uses_the_cpu_is_busy_never_stuck() {
        let t0 = Instant::now();
        let mut w = Watcher::new(Engine::Claude);
        let mut cpu = 0;
        for minute in 0..30u64 {
            cpu += 60_000 * 4; // four cores, all minute
            let procs = engine_and(cpu, minute * 60 + 30);
            let look = w.look(&seen(&procs, minute * 60 + 31, minute * 60 + 31), t0 + Duration::from_secs(minute * 60));
            if minute > 0 {
                assert_eq!(look.state, WatchState::Busy, "minute {minute}");
                assert_eq!(look.process.as_ref().map(|p| (p.name.as_str(), p.cpu_percent)), Some(("cargo", 400)));
            }
        }
    }

    #[test]
    fn a_command_idle_for_minutes_with_its_engine_quiet_looks_stuck() {
        let t0 = Instant::now();
        let mut w = Watcher::new(Engine::Claude);
        let mut state = None;
        for look in 0..=40u64 {
            let at = look * 5;
            let procs = engine_and(20, at + 2);
            state = Some(w.look(&seen(&procs, at + 3, at + 3), t0 + Duration::from_secs(at)).state);
            if at < STUCK_AFTER.as_secs() {
                assert_eq!(state, Some(WatchState::Working), "{at}s in, not yet");
            }
        }
        assert_eq!(state, Some(WatchState::Stuck { idle_secs: 200 }));
    }

    #[test]
    fn a_command_at_work_now_and_then_is_not_stuck() {
        let t0 = Instant::now();
        let mut w = Watcher::new(Engine::Codex);
        for look in 0..=60u64 {
            let at = look * 5;
            // A second of CPU every minute: a poll, a page fetched.
            let procs = engine_and(1000 * (at / 60), at + 2);
            let state = w.look(&seen(&procs, at + 3, at + 3), t0 + Duration::from_secs(at)).state;
            assert!(!state.is_stuck(), "{at}s: {state:?}");
        }
    }

    #[test]
    fn nothing_under_the_engine_says_so_then_looks_stuck_once_the_engine_is_quiet_long() {
        let t0 = Instant::now();
        let procs = vec![p(500, 1, 4000, 1000, "claude"), p(501, 500, 4000, 10, "equipe mcp")];
        let mut w = Watcher::new(Engine::Claude);
        assert_eq!(w.look(&seen(&procs, 4, 4), t0).state, WatchState::Working, "just started");
        assert_eq!(w.look(&seen(&procs, 30, 30), t0 + Duration::from_secs(26)).state, WatchState::Nothing);
        let late = w.look(&seen(&procs, 1686, 1686), t0 + Duration::from_secs(1682));
        assert!(late.state.is_stuck(), "{late:?}");
        assert_eq!(late.process, None);
        // A watch begun late (the daemon just learned the engine) counts from the step.
        let mut fresh = Watcher::new(Engine::Claude);
        assert!(fresh.look(&seen(&procs, 1686, 1686), t0).state.is_stuck());
        assert_eq!(fresh.look(&seen(&procs, 1686, 20), t0).state, WatchState::Nothing, "the engine spoke lately");
    }

    #[test]
    fn waiting_for_the_persons_answer_is_never_stuck() {
        let procs = vec![p(500, 1, 4000, 1000, "claude")];
        let mut w = Watcher::new(Engine::Claude);
        let asking = Seen { asking: true, ..seen(&procs, 1500, 1500) };
        assert_eq!(w.look(&asking, Instant::now()).state, WatchState::AwaitingYou);
    }

    #[test]
    fn the_systems_own_ps_lists_a_command_under_its_parent_and_stopping_ends_it() {
        let marker = "sleep 30; echo command-watch-marker";
        let mut command = std::process::Command::new("sh").arg("-c").arg(marker).spawn().unwrap();
        // A bystander of this process's too, started as long ago: never stopped.
        let mut bystander = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let listed = || parse_ps(&String::from_utf8_lossy(&std::process::Command::new("ps").args(PS_ARGS).output().unwrap().stdout));
        let procs = listed();
        let me = procs.iter().find(|p| p.pid == std::process::id()).expect("this process is listed");
        assert_eq!(me.ppid, std::os::unix::process::parent_id());
        let stop = to_stop(&procs, std::process::id(), Duration::from_secs(5), &[marker.to_owned()]);
        let names: Vec<String> = stop.iter().map(Proc::name).collect();
        assert!(stop.iter().any(|p| p.pid == command.id()), "{names:?}");
        assert!(names.iter().any(|n| n == "sleep"), "what the shell runs, with it: {names:?}");
        assert!(!stop.iter().any(|p| p.pid == bystander.id()), "{names:?}");
        for p in &stop {
            crate::group::end(p.pid, crate::group::Signal::Terminate);
        }
        assert!(!command.wait().unwrap().success(), "stopped");
        assert!(bystander.try_wait().unwrap().is_none(), "the bystander runs on");
        let _ = bystander.kill();
        let _ = bystander.wait();
    }

    #[test]
    fn stopping_ends_the_commands_processes_children_first_and_nothing_else() {
        let procs = parse_ps(MAC);
        let stop = to_stop(&procs, 500, Duration::from_secs(1700), &[]);
        assert_eq!(stop.iter().map(|p| p.pid).collect::<Vec<_>>(), [611, 610]);
        assert!(to_stop(&procs, 999, Duration::from_secs(1700), &[]).is_empty(), "under no engine of ours, nothing");
        let later = vec![p(610, 1, 1690, 10, &stop[1].command), p(611, 1, 2, 0, &stop[0].command)];
        assert_eq!(survivors(&stop, &later), [610], "a pid taken again is younger: not ours");
    }
}
