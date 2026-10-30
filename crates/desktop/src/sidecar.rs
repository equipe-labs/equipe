//! The commands that change or read the daemon's machine's own files — the
//! squad's folder and memory, its squads and projects, the apps a send waits
//! on, the browsers' limit, the sites signed in to. The terminal console does
//! them with `equipe_core` code on its disk; the app never reads that disk
//! (it may be another Mac's), so it asks `equipe` itself, the same code:
//! `equipe home|memory|squad|send-apps|browser …` run here, or over SSH on the
//! daemon's machine (T-82's "side command", over the same shared connection).
//! `EQUIPE_NO_OPEN=1` keeps them from opening Finder windows on a far screen.

use crate::install;
use crate::ssh::{self, quote};
use crate::target::Endpoint;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    /// `equipe home changes|undo [n]`.
    Home { squad: String, words: Vec<String> },
    /// `equipe memory [changes]`.
    Memory { squad: String, words: Vec<String> },
    /// `equipe squad new|add|remove …`; `dry_run` only checks and describes.
    Squad { squad: String, words: Vec<String>, dry_run: bool },
    /// `equipe send-apps [add|remove <app> · reset]`.
    SendApps { squad: String, words: Vec<String> },
    /// `equipe browser --max [n]`.
    BrowserMax(Option<String>),
    /// `equipe browser [url]`: the window to sign in to sites in. Ends when it is closed.
    BrowserSignin,
}

impl Job {
    pub fn args(&self) -> Vec<String> {
        let words = |head: &[&str], squad: &str, words: &[String]| -> Vec<String> {
            let named = (!squad.is_empty()).then(|| ["--squad".to_owned(), squad.to_owned()]).into_iter().flatten();
            head.iter().map(|w| (*w).to_owned()).chain(words.iter().cloned()).chain(named).collect()
        };
        match self {
            Self::Home { squad, words: w } => words(&["home"], squad, w),
            Self::Memory { squad, words: w } => words(&["memory"], squad, w),
            Self::SendApps { squad, words: w } => words(&["send-apps"], squad, w),
            Self::Squad { squad, words: w, dry_run } => {
                let mut args = words(&["squad"], squad, w);
                if *dry_run {
                    args.push("--dry-run".into());
                }
                args
            }
            Self::BrowserMax(n) => ["browser", "--max"].iter().map(|w| (*w).to_owned()).chain(n.clone()).collect(),
            Self::BrowserSignin => vec!["browser".into()],
        }
    }

    /// What it is called in a message about it failing.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Home { .. } => "equipe home",
            Self::Memory { .. } => "equipe memory",
            Self::Squad { .. } => "equipe squad",
            Self::SendApps { .. } => "equipe send-apps",
            Self::BrowserMax(_) | Self::BrowserSignin => "equipe browser",
        }
    }
}

/// The script an SSH side command runs: `equipe …` with its output and exit
/// status told apart in the one stream `ssh` gives.
pub fn script(args: &[String]) -> String {
    let line = args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
    format!("out=$(EQUIPE_NO_OPEN=1 equipe {line} 2>&1); code=$?; printf '%s\\n' \"$out\"; echo \"{EXIT}$code\"")
}

const EXIT: &str = "__equipe_exit:";

/// The text before the exit line, and the status it gives (127: no `equipe`).
pub fn split_exit(output: &str) -> (String, i32) {
    match output.rfind(EXIT) {
        Some(at) => (output[..at].trim_end().to_owned(), output[at + EXIT.len()..].trim().parse().unwrap_or(1)),
        None => (output.trim_end().to_owned(), 1),
    }
}

/// Plain lines for the person: no colour codes, no indent, none empty.
pub fn lines(output: &str) -> Vec<String> {
    output.lines().map(|l| strip_ansi(l).trim().to_owned()).filter(|l| !l.is_empty()).collect()
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Why it failed, as the last thing it said.
fn reason(job: &Job, output: &str, code: i32) -> String {
    if code == 127 {
        return equipe_core::t!("sidecar.no_equipe_there").into();
    }
    let said = lines(output);
    // An `equipe` that predates the command.
    if said.iter().any(|l| l.contains("unrecognized subcommand") || l.contains("unexpected argument")) {
        return equipe_core::t!("sidecar.too_old", command = job.name());
    }
    said.iter()
        .rev()
        .find(|l| !l.starts_with("Usage:") && !l.starts_with("For more information") && !l.starts_with("tip:"))
        .map(|l| l.trim_start_matches('✗').trim().to_owned())
        .unwrap_or_else(|| equipe_core::t!("sidecar.exit_code", command = job.name(), code = code))
}

/// Run `job` on the daemon's machine: its lines, or why not. Blocks — run it off the UI thread.
pub fn run(endpoint: &Endpoint, job: &Job) -> Result<Vec<String>, String> {
    let args = job.args();
    let (output, code) = match endpoint {
        Endpoint::Local { .. } => {
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
            let cli = install::find_cli(&home, std::env::var_os("PATH").as_deref()).ok_or_else(|| equipe_core::t!("sidecar.no_equipe_here").to_owned())?;
            let out = std::process::Command::new(cli)
                .args(&args)
                .env("EQUIPE_NO_OPEN", "1")
                .stdin(std::process::Stdio::null())
                .output()
                .map_err(|e| equipe_core::t!("sidecar.couldnt_run", command = job.name(), err = e))?;
            let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            (text, out.status.code().unwrap_or(1))
        }
        Endpoint::Ssh(host) => {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
            let bytes = runtime
                .block_on(ssh::Launcher::system(None).run(host, &script(&args)))
                .map_err(|f| f.guide(endpoint).short)?;
            split_exit(&String::from_utf8_lossy(&bytes))
        }
    };
    match code {
        0 => Ok(lines(&output)),
        code => Err(reason(job, &output, code)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(w: &[&str]) -> Vec<String> {
        w.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn each_job_is_the_equipe_command_the_console_shares() {
        assert_eq!(Job::Home { squad: "acme".into(), words: s(&["undo", "2"]) }.args(), s(&["home", "undo", "2", "--squad", "acme"]));
        assert_eq!(Job::Memory { squad: "acme".into(), words: s(&["changes"]) }.args(), s(&["memory", "changes", "--squad", "acme"]));
        assert_eq!(Job::Squad { squad: "acme".into(), words: s(&["add", "~/Projects/shop"]), dry_run: true }.args(), s(&["squad", "add", "~/Projects/shop", "--squad", "acme", "--dry-run"]));
        assert_eq!(Job::SendApps { squad: "acme".into(), words: s(&["add", "Slack"]) }.args(), s(&["send-apps", "add", "Slack", "--squad", "acme"]));
        assert_eq!(Job::BrowserMax(Some("6".into())).args(), s(&["browser", "--max", "6"]));
        assert_eq!(Job::BrowserMax(None).args(), s(&["browser", "--max"]));
        assert_eq!(Job::BrowserSignin.args(), s(&["browser"]));
    }

    #[test]
    fn over_ssh_every_word_is_quoted_and_the_status_travels_with_the_output() {
        let script = script(&s(&["squad", "add", "it's a folder; rm -rf /", "--squad", "a b"]));
        assert!(script.contains("equipe 'squad' 'add' 'it'\\''s a folder; rm -rf /' '--squad' 'a b'"), "{script}");
        assert!(script.starts_with("out=$(EQUIPE_NO_OPEN=1 equipe "));
        assert_eq!(split_exit("  one\n  two\n__equipe_exit:0\n"), ("  one\n  two".to_owned(), 0));
        assert_eq!(split_exit("boom\n__equipe_exit:127\n").1, 127);
        assert_eq!(split_exit("no status").1, 1);
    }

    #[test]
    fn output_comes_back_as_plain_lines_and_a_failure_as_its_last_word() {
        assert_eq!(lines("\u{1b}[1m  1 · just now · pm saved a.md\u{1b}[0m\n\n  2 · 5m ago\n"), ["1 · just now · pm saved a.md", "2 · 5m ago"]);
        let job = Job::Home { squad: "a".into(), words: vec![] };
        assert_eq!(reason(&job, "  The squad's folder isn't made yet\n", 1), "The squad's folder isn't made yet");
        assert_eq!(reason(&job, "\n✗ shop doesn't work on nope\n", 1), "shop doesn't work on nope");
        assert!(reason(&job, "", 127).contains("찾지 못했어요"));
        assert!(reason(&job, "error: unrecognized subcommand 'squad'\n\nUsage: equipe\n", 2).contains("equipe update"));
        assert_eq!(reason(&job, "", 3), "equipe home 종료 코드 3");
    }
}
