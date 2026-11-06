//! equipe's guard on a member's tool calls: what a charter once asked for in
//! words, refused where it happens instead. Claude Code puts each of a
//! member's calls to it before running one (a `PreToolUse` hook that runs
//! `equipe hook`), and a refusal comes back to the member as the call's
//! answer, saying what to do instead:
//!
//! - a long `sleep` in the foreground: nothing reaches the member while it
//!   waits, so what it waits for runs in the background and its engine's own
//!   tool waits for it;
//! - `equipe browser`, which opens a window for the person and holds the turn
//!   until it is closed: a member hands its browser over with `show_my_browser`;
//! - a write into a folder the member only reads here — a file tool's, or a
//!   command's (`cp` into it, `rm` in it, `>` onto it). Copying out of one is
//!   reading it, and is never refused.
//!
//! The guard only refuses: what it lets through goes on to the engine's own
//! permission rules and the operating system's fence, as before.

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

/// A pause shorter than this many seconds is fine in the foreground — a
/// server binding its port, a file settling, a retry backing off: waiting
/// for it costs less than starting it in the background and waiting there.
/// From here on a `sleep` is waiting for something, which is what the
/// background and the engine's own wait are for. Claude Code's own check
/// refuses only a `sleep` of 25 seconds or more that starts a command.
pub const SHORT_PAUSE_SECS: f64 = 10.0;

/// The tools whose calls the guard is asked about: commands, and the file tools.
pub const MATCHER: &str = "Bash|Edit|Write|MultiEdit|NotebookEdit";

/// How long Claude Code waits for the guard's answer, in seconds.
const HOOK_TIMEOUT_SECS: u64 = 30;

/// What a member is told when it runs `equipe browser` — by the guard, and by
/// `equipe browser` itself when a member's command runs it.
pub const BROWSER_REFUSED: &str = "Blocked: `equipe browser` opens a window for the person and waits until they close \
it, which would stop your turn. To hand your browser to the person — to sign in, or for a step only they can do — \
call `show_my_browser`; to work in a site yourself, use your `browser` tools.";

/// Set in a member's commands' environment: `equipe browser` refuses there.
pub const MEMBER_ENV: &str = "EQUIPE_MEMBER";

/// Whether a command runs for a member, by [`MEMBER_ENV`] as it was found:
/// on every engine, `equipe browser` refuses then, with [`BROWSER_REFUSED`].
pub fn run_by_member(env: Option<std::ffi::OsString>) -> bool {
    env.is_some_and(|name| !name.is_empty())
}

/// Why a tool call is refused, or `None` to let it go on. `cwd` is where the
/// member's command runs; `work`, its working directory, never refused;
/// `read_only`, the folders it only reads in this work.
pub fn refusal(tool: &str, input: &Value, cwd: &Path, work: &Path, read_only: &[PathBuf]) -> Option<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let shut = |path: &str, cwd: &Path| -> Option<String> {
        let target = resolve(path, cwd, home.as_deref())?;
        if target.starts_with(work) {
            return None;
        }
        let dir = read_only.iter().find(|dir| target.starts_with(dir))?;
        Some(read_only_refused(&target, dir))
    };
    match tool {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let path = ["file_path", "notebook_path"].iter().find_map(|key| input.get(*key).and_then(Value::as_str))?;
            shut(path, cwd)
        }
        "Bash" => {
            let command = input.get("command").and_then(Value::as_str)?;
            let background = input.get("run_in_background").and_then(Value::as_bool) == Some(true);
            let commands = parse(command);
            if commands.iter().any(|c| runs_equipe_browser(&c.words)) {
                return Some(BROWSER_REFUSED.to_owned());
            }
            // Where each command runs: a `cd` moves the rest of the chain.
            let mut here = cwd.to_path_buf();
            for c in &commands {
                if let [cd, dir] = c.words.as_slice()
                    && cd == "cd"
                    && let Some(dir) = resolve(dir, &here, home.as_deref())
                {
                    here = dir;
                    continue;
                }
                if let Some(why) = writes(c).iter().find_map(|target| shut(target, &here)) {
                    return Some(why);
                }
            }
            if !background && let Some(slept) = long_sleep(&commands) {
                return Some(sleep_refused(&slept));
            }
            None
        }
        _ => None,
    }
}

fn read_only_refused(target: &Path, dir: &Path) -> String {
    format!(
        "Blocked: {} is in {}, a folder you only read here. Copying out of it is fine — write your copy where you \
work. If the work needs a change in that folder itself, say what and why and ask the person: a project changes only \
on a `code` ticket, in its own worktree.",
        target.display(),
        dir.display()
    )
}

fn sleep_refused(slept: &str) -> String {
    format!(
        "Blocked: `{slept}` would hold your turn in the foreground, and nothing reaches you while it waits. Start what \
you are waiting for in the background (run_in_background: true): you are told when it ends. To wait for something \
else to happen, use your engine's own wait — Monitor, with an until-loop (`until <check>; do sleep 2; done`). A pause \
under {} seconds is fine.",
        SHORT_PAUSE_SECS as u64
    )
}

/// The engine's own rules against `equipe browser`, by name and by where
/// equipe is: refused even where the guard can't be asked.
pub fn deny_rules(binary: Option<&Path>) -> Vec<String> {
    let mut rules = vec!["Bash(equipe browser:*)".to_owned()];
    if let Some(binary) = binary {
        rules.push(format!("Bash({} browser:*)", binary.display()));
    }
    rules
}

/// The guard as Claude Code's settings take it (`hooks`): `equipe hook`,
/// told the member's working directory and the folders it only reads.
pub fn hooks(binary: &Path, work: &Path, read_only: &[PathBuf]) -> Value {
    let mut command = format!("{} hook --work {}", quote(&binary.to_string_lossy()), quote(&work.to_string_lossy()));
    for dir in read_only {
        command.push_str(&format!(" --read-only {}", quote(&dir.to_string_lossy())));
    }
    json!({ "PreToolUse": [{ "matcher": MATCHER, "hooks": [{ "type": "command", "command": command, "timeout": HOOK_TIMEOUT_SECS }] }] })
}

/// The hook's answer to Claude Code for one call (its JSON on stdin): a
/// refusal, or nothing — never an approval, which would skip the engine's
/// own rules.
pub fn answer(call: &Value, work: &Path, read_only: &[PathBuf]) -> Option<Value> {
    let tool = call.get("tool_name").and_then(Value::as_str)?;
    let input = call.get("tool_input")?;
    let cwd = call.get("cwd").and_then(Value::as_str).map_or_else(|| work.to_path_buf(), PathBuf::from);
    let why = refusal(tool, input, &cwd, work, read_only)?;
    Some(json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": why } }))
}

/// Why a side chat's call is refused ([`crate::side_chat`]): every tool but
/// reading and searching files and the web — whatever its engine's flags let
/// through by mistake, the guard doesn't.
pub fn side_chat_refusal(tool: &str) -> Option<String> {
    if crate::side_chat::CLAUDE_TOOLS.contains(&tool) {
        return None;
    }
    Some(format!(
        "Blocked: `{tool}` isn't available in a side chat. A side chat only reads files and the web, and changes \
nothing; say what you'd do instead, and the person can take it to the main conversation."
    ))
}

/// The hook's answer for a side chat's call ([`side_chat_refusal`]).
pub fn side_chat_answer(call: &Value) -> Option<Value> {
    let tool = call.get("tool_name").and_then(Value::as_str)?;
    let why = side_chat_refusal(tool)?;
    Some(json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": why } }))
}

/// The guard for a side chat, as Claude Code's settings take it: asked
/// before every call (`equipe hook --side-chat`).
pub fn side_chat_hooks(binary: &Path) -> Value {
    let command = format!("{} hook --side-chat", quote(&binary.to_string_lossy()));
    json!({ "PreToolUse": [{ "matcher": "*", "hooks": [{ "type": "command", "command": command, "timeout": HOOK_TIMEOUT_SECS }] }] })
}

/// A word for a shell, in single quotes when it needs them.
fn quote(word: &str) -> String {
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+:@%=".contains(c)) {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// `path` as an absolute path, `.` and `..` worked out: relative to `cwd`,
/// `~` at the person's home. `None` for what the shell would still expand.
fn resolve(path: &str, cwd: &Path, home: Option<&Path>) -> Option<PathBuf> {
    if path.is_empty() || path.contains(['$', '`']) {
        return None;
    }
    let joined = match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => home?.join(rest.trim_start_matches('/')),
        Some(_) => return None,
        None => cwd.join(path),
    };
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    Some(out)
}

/// One simple command of a shell line: its words, quotes taken off, and
/// where its output goes.
#[derive(Debug, Default, Clone, PartialEq)]
struct Simple {
    words: Vec<String>,
    /// Files its output is written to (`> out`, `2>> log`).
    redirects: Vec<String>,
    /// Left running with `&`: the line doesn't wait for it.
    background: bool,
    /// Inside a loop's `do … done`.
    looped: bool,
}

/// The simple commands of a shell line, in order: `a && b; c | d &`, `if`,
/// loops and subshells opened up, a `sh -c '…'` read too. A heredoc's lines
/// are its text, never commands.
fn parse(line: &str) -> Vec<Simple> {
    parsed(line)
}

/// The words of each simple command a shell line runs, as the guard reads
/// it, a wrapper (`env`, `timeout`…) taken off — for measuring what members
/// ran ([`crate::quality`]).
pub fn commands(line: &str) -> Vec<Vec<String>> {
    parsed(line).into_iter().map(|c| unwrapped(c.words)).collect()
}

fn parsed(line: &str) -> Vec<Simple> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    parse_into(&without_heredocs(line), &mut depth, &mut out);
    out
}

fn parse_into(line: &str, loops: &mut usize, out: &mut Vec<Simple>) {
    let mut current = Simple::default();
    let mut word = String::new();
    let mut in_word = false;
    let mut redirect_next = false;
    let mut chars = line.chars().peekable();
    let push_word = |current: &mut Simple, word: &mut String, in_word: &mut bool, redirect_next: &mut bool| {
        if *in_word {
            if *redirect_next {
                current.redirects.push(std::mem::take(word));
                *redirect_next = false;
            } else {
                current.words.push(std::mem::take(word));
            }
            *in_word = false;
        }
    };
    let end = |current: &mut Simple, background: bool, loops: &mut usize, out: &mut Vec<Simple>| {
        let mut simple = std::mem::take(current);
        // Keywords open and close what follows; what is left is the command.
        loop {
            match simple.words.first().map(String::as_str) {
                Some("do") => {
                    *loops += 1;
                    simple.words.remove(0);
                }
                Some("done") => {
                    *loops = loops.saturating_sub(1);
                    simple.words.remove(0);
                }
                Some("if" | "then" | "else" | "elif" | "fi" | "while" | "until" | "!" | "{" | "}" | "(" | ")" | "time" | "exec") => {
                    simple.words.remove(0);
                }
                _ => break,
            }
        }
        if simple.words.first().is_some_and(|w| w == "for") {
            return; // `for x in …`: a list, run as nothing.
        }
        simple.words = unwrapped(simple.words);
        simple.background = background;
        simple.looped = *loops > 0;
        if let [shell, flag, script] = simple.words.as_slice()
            && matches!(shell.rsplit('/').next(), Some("sh" | "bash" | "zsh"))
            && flag == "-c"
        {
            let script = script.clone();
            parse_into(&script, loops, out);
            return;
        }
        if !simple.words.is_empty() || !simple.redirects.is_empty() {
            out.push(simple);
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    word.push(q);
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => {
                            if let Some(next) = chars.next() {
                                word.push(next);
                            }
                        }
                        _ => word.push(q),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(next) = chars.next()
                    && next != '\n'
                {
                    word.push(next);
                }
            }
            ' ' | '\t' => push_word(&mut current, &mut word, &mut in_word, &mut redirect_next),
            '>' => {
                // `2>`, `&>`: the number or `&` before is part of the redirect.
                if in_word && (word.chars().all(|d| d.is_ascii_digit()) || word == "&") {
                    word.clear();
                    in_word = false;
                }
                push_word(&mut current, &mut word, &mut in_word, &mut redirect_next);
                if chars.peek() == Some(&'>') || chars.peek() == Some(&'|') {
                    chars.next();
                }
                if chars.peek() == Some(&'&') {
                    // `>&2`: onto another output, not a file.
                    chars.next();
                    while chars.peek().is_some_and(|d| d.is_ascii_digit() || *d == '-') {
                        chars.next();
                    }
                    continue;
                }
                redirect_next = true;
            }
            '<' => {
                push_word(&mut current, &mut word, &mut in_word, &mut redirect_next);
                while chars.peek() == Some(&'<') {
                    chars.next();
                }
                // What is read from is no command's word.
                while chars.next_if_eq(&' ').is_some() {}
                while chars.next_if(|d| !d.is_whitespace() && !";&|()".contains(*d)).is_some() {}
            }
            ';' | '\n' | '(' | ')' | '`' => {
                push_word(&mut current, &mut word, &mut in_word, &mut redirect_next);
                end(&mut current, false, loops, out);
            }
            '$' if chars.peek() == Some(&'(') => {
                chars.next();
                push_word(&mut current, &mut word, &mut in_word, &mut redirect_next);
                end(&mut current, false, loops, out);
            }
            '|' => {
                chars.next_if_eq(&'|');
                push_word(&mut current, &mut word, &mut in_word, &mut redirect_next);
                end(&mut current, false, loops, out);
            }
            '&' => {
                let and = chars.next_if_eq(&'&').is_some();
                push_word(&mut current, &mut word, &mut in_word, &mut redirect_next);
                end(&mut current, !and, loops, out);
            }
            _ => {
                in_word = true;
                word.push(c);
            }
        }
    }
    push_word(&mut current, &mut word, &mut in_word, &mut redirect_next);
    end(&mut current, false, loops, out);
}

/// The command a wrapper runs: `env A=1 nice -n 5 timeout 60 sleep 30` is `sleep 30`.
fn unwrapped(mut words: Vec<String>) -> Vec<String> {
    loop {
        let Some(first) = words.first() else { return words };
        let program = first.rsplit('/').next().unwrap_or(first);
        let skip = if first.contains('=') && !first.starts_with('=') && !first.starts_with('-') {
            1 // an assignment before the command
        } else {
            match program {
                "env" | "command" | "nohup" | "caffeinate" | "builtin" => 1 + words[1..].iter().take_while(|w| w.starts_with('-') || w.contains('=')).count(),
                "nice" => 1 + if words.get(1).is_some_and(|w| w == "-n") { 2 } else { usize::from(words.get(1).is_some_and(|w| w.starts_with('-'))) },
                "timeout" | "gtimeout" => 1 + words[1..].iter().take_while(|w| w.starts_with('-')).count() + 1,
                _ => return words,
            }
        };
        words.drain(..skip.min(words.len()));
    }
}

/// A heredoc's lines dropped: the text it feeds, never commands of the line.
fn without_heredocs(line: &str) -> String {
    let mut out = String::new();
    let mut until: Option<String> = None;
    for text in line.split_inclusive('\n') {
        if let Some(end) = &until {
            if text.trim() == end {
                until = None;
            }
            continue;
        }
        out.push_str(text);
        if let Some(at) = text.find("<<") {
            let rest = text[at + 2..].trim_start_matches(['-', '~']).trim_start();
            if rest.starts_with('<') {
                continue; // `<<<`: a here-string, on its own line
            }
            let end: String = rest.trim_start_matches(['\'', '"']).chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !end.is_empty() {
                until = Some(end);
            }
        }
    }
    out
}

/// Whether these words run `equipe browser`.
fn runs_equipe_browser(words: &[String]) -> bool {
    match words {
        [program, sub, ..] => program.rsplit('/').next() == Some("equipe") && sub == "browser",
        _ => false,
    }
}

/// What a simple command writes: where its output goes, and the files
/// programs that change files name — a copy's destination, never its source.
fn writes(c: &Simple) -> Vec<String> {
    let mut out = c.redirects.clone();
    let Some(first) = c.words.first() else { return out };
    let program = first.rsplit('/').next().unwrap_or(first);
    let args: Vec<&String> = c.words[1..].iter().collect();
    let paths = || args.iter().filter(|a| !a.starts_with('-')).map(|a| (*a).clone()).collect::<Vec<_>>();
    match program {
        // A copy writes its destination: the last path, or the one `-t` names.
        "cp" | "install" | "ln" | "rsync" | "ditto" | "scp" | "gcp" => {
            let target = args.iter().position(|a| *a == "-t" || *a == "--target-directory").and_then(|at| args.get(at + 1)).map(|a| (*a).clone());
            let target = target.or_else(|| args.iter().find_map(|a| a.strip_prefix("--target-directory=").map(str::to_owned)));
            out.extend(target.or_else(|| paths().pop()));
        }
        // These change every file they name — a move, its source too.
        "mv" | "rm" | "rmdir" | "mkdir" | "touch" | "tee" | "truncate" | "unlink" | "shred" | "chmod" | "chown" | "chgrp" | "chflags" | "xattr" => out.extend(paths()),
        // Edited in place.
        "sed" | "gsed" | "perl" if args.iter().any(|a| a.starts_with("-i") || *a == "--in-place" || a.starts_with("-pi")) => out.extend(paths()),
        "dd" => out.extend(args.iter().filter_map(|a| a.strip_prefix("of=").map(str::to_owned))),
        _ => {}
    }
    out
}

/// A `sleep` the line would wait out in the foreground, as written: one long
/// pause, or short ones that add up outside a loop. A pause in a loop runs
/// once a round, while something is checked; a long one is still waiting.
fn long_sleep(commands: &[Simple]) -> Option<String> {
    let mut chained = 0.0;
    let mut said = Vec::new();
    for c in commands.iter().filter(|c| !c.background) {
        let Some(secs) = sleeps(&c.words) else { continue };
        let written = c.words.join(" ");
        if secs >= SHORT_PAUSE_SECS {
            return Some(written);
        }
        if !c.looped {
            chained += secs;
            said.push(written);
            if chained >= SHORT_PAUSE_SECS {
                return Some(said.join("; "));
            }
        }
    }
    None
}

/// How long `sleep …` pauses, in seconds; `None` for another command, or
/// one whose length the shell would work out (`sleep $n`).
fn sleeps(words: &[String]) -> Option<f64> {
    let (program, args) = words.split_first()?;
    if program.rsplit('/').next() != Some("sleep") || args.is_empty() {
        return None;
    }
    args.iter().map(|a| duration(a.as_str())).sum()
}

fn duration(arg: &str) -> Option<f64> {
    if arg == "infinity" || arg == "inf" {
        return Some(f64::INFINITY);
    }
    let (number, scale) = match arg.char_indices().last()? {
        (at, 's') => (&arg[..at], 1.0),
        (at, 'm') => (&arg[..at], 60.0),
        (at, 'h') => (&arg[..at], 3600.0),
        (at, 'd') => (&arg[..at], 86400.0),
        _ => (arg, 1.0),
    };
    let secs: f64 = number.parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then_some(secs * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: &str = "/w/backend-t3";
    const PROJECT: &str = "/code/site";

    fn bash(command: &str) -> Option<String> {
        refusal("Bash", &json!({ "command": command }), Path::new(WORK), Path::new(WORK), &[PathBuf::from(PROJECT)])
    }

    fn in_background(command: &str) -> Option<String> {
        refusal("Bash", &json!({ "command": command, "run_in_background": true }), Path::new(WORK), Path::new(WORK), &[PathBuf::from(PROJECT)])
    }

    #[test]
    fn a_foreground_sleep_is_refused_with_what_to_do_instead() {
        for command in ["sleep 30", "sleep 30 && ls shots/", "npm run build; sleep 60; curl localhost:3000", "sleep 1m", "sleep 10", "sleep infinity", "env A=1 sleep 45", "timeout 120 sleep 90", "bash -c 'sleep 30 && ls'", "until curl -s localhost:3000; do sleep 30; done", "sleep 5; sleep 5"] {
            let why = bash(command).unwrap_or_else(|| panic!("{command} let through"));
            assert!(why.starts_with("Blocked: `"), "{command}: {why}");
            for part in ["in the background (run_in_background: true)", "Monitor", "until <check>; do sleep 2; done", "under 10 seconds is fine"] {
                assert!(why.contains(part), "{command}: {part}: {why}");
            }
        }
        assert!(bash("sleep 30 && ls").unwrap().contains("`sleep 30`"), "it names the sleep");
    }

    #[test]
    fn a_short_pause_and_a_wait_in_the_background_go_through() {
        for command in [
            "sleep 2 && curl -s localhost:3000",
            "npm run dev & sleep 3; curl localhost:3000",
            "until curl -s localhost:3000 >/dev/null; do sleep 1; done",
            "for i in 1 2 3; do curl -sf localhost:3000 && break; sleep 2; done",
            "sleep 30 & echo started",
            "sleep $DELAY",
            "echo 'sleep 30 is too long'",
            "git commit -m \"Wait: sleep 30 then retry\"",
            "cat > deploy.sh <<'EOF'\nsleep 30\ncurl https://example.com\nEOF",
            "cargo test --workspace",
        ] {
            assert_eq!(bash(command), None, "{command}");
        }
        for command in ["sleep 300", "sleep 60 && ls", "until gh run view 1 --exit-status; do sleep 30; done"] {
            assert_eq!(in_background(command), None, "{command}: waiting in the background is the point");
        }
    }

    #[test]
    fn equipe_browser_is_refused_however_it_is_run() {
        for command in ["equipe browser", "equipe browser https://vercel.com", "/Users/x/.local/bin/equipe browser --forget", "cd /tmp && equipe browser"] {
            let why = bash(command).unwrap_or_else(|| panic!("{command} let through"));
            assert_eq!(why, BROWSER_REFUSED, "{command}");
        }
        assert!(BROWSER_REFUSED.contains("`show_my_browser`") && BROWSER_REFUSED.contains("stop your turn"));
        // The engine's own rules refuse it too, by name and by path.
        assert_eq!(deny_rules(Some(Path::new("/opt/equipe"))), ["Bash(equipe browser:*)", "Bash(/opt/equipe browser:*)"]);
        assert_eq!(deny_rules(None), ["Bash(equipe browser:*)"]);
        // And `equipe browser` itself refuses a member's command, on any engine.
        assert!(run_by_member(Some("frontend".into())));
        assert!(!run_by_member(None) && !run_by_member(Some("".into())), "the person's own terminal");
        assert_eq!(in_background("equipe browser"), Some(BROWSER_REFUSED.to_owned()), "a window held open blocks in the background too");
        for command in ["equipe design check", "echo equipe browser", "equipe --version"] {
            assert_eq!(bash(command), None, "{command}");
        }
    }

    #[test]
    fn copying_out_of_a_folder_kept_for_reading_is_reading_it() {
        for command in [
            "cp /code/site/public/logo.png outputs/logo.png",
            "cp -R /code/site/public/icons outputs/",
            "cp /code/site/a.png /code/site/b.png outputs/shots/",
            "rsync -a /code/site/assets/ outputs/assets/",
            "ditto /code/site/public /w/backend-t3/public-copy",
            "cat /code/site/README.md > notes/readme.md",
            "cd /code/site && cp public/logo.png /w/backend-t3/outputs/",
            "cp -t outputs /code/site/logo.png",
        ] {
            assert_eq!(bash(command), None, "{command}");
        }
    }

    #[test]
    fn a_write_into_a_folder_kept_for_reading_is_refused() {
        for command in [
            "cp outputs/logo.png /code/site/public/logo.png",
            "rsync -a outputs/ /code/site/public/",
            "ditto outputs /code/site/out",
            "mv /code/site/old.png outputs/",
            "rm -rf /code/site/build",
            "echo hi > /code/site/notes.txt",
            "echo hi >> /code/site/notes.txt",
            "npm run build 2> /code/site/err.log",
            "cd /code/site && rm README.md",
            "cd /code/site; touch x",
            "sed -i '' 's/a/b/' /code/site/a.txt",
            "mkdir -p /code/site/new",
            "printf x | tee /code/site/a.txt",
            "cp -t /code/site outputs/logo.png",
            "bash -c 'rm /code/site/a'",
            "cp x ../../code/site/x",
        ] {
            let why = bash(command).unwrap_or_else(|| panic!("{command} let through"));
            assert!(why.starts_with("Blocked: /code/site") && why.contains("a folder you only read here") && why.contains("`code` ticket"), "{command}: {why}");
        }
        // The member's own worktree is always its own.
        assert_eq!(refusal("Bash", &json!({ "command": "touch x" }), Path::new("/code/site/sub"), Path::new("/code/site"), &[PathBuf::from("/code")]), None);
        assert_eq!(bash("echo ok > /dev/null 2>&1"), None);
        assert_eq!(bash("ls /code/site >&2"), None);
    }

    #[test]
    fn the_file_tools_never_write_a_folder_kept_for_reading() {
        let file = |tool: &str, path: &str| refusal(tool, &json!({ "file_path": path, "notebook_path": path }), Path::new(WORK), Path::new(WORK), &[PathBuf::from(PROJECT)]);
        for tool in ["Edit", "Write", "MultiEdit", "NotebookEdit"] {
            assert!(file(tool, "/code/site/src/app.ts").is_some_and(|why| why.starts_with("Blocked: /code/site/src/app.ts")), "{tool}");
            assert_eq!(file(tool, "/w/backend-t3/src/app.ts"), None, "{tool}");
            assert_eq!(file(tool, "src/app.ts"), None, "{tool}: relative to where it works");
        }
        assert!(file("Write", "/code/site/../site/x").is_some(), "no way around it by `..`");
        assert_eq!(file("Read", "/code/site/src/app.ts"), None, "reading is free");
        assert_eq!(file("Write", "/code/sitemap/x"), None, "a folder beside it is another folder");
    }

    #[test]
    fn the_hook_refuses_in_claude_codes_words_and_never_approves() {
        let work = Path::new(WORK);
        let ro = [PathBuf::from(PROJECT)];
        let call = |tool: &str, input: Value| json!({ "hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": input, "cwd": WORK });
        let said = answer(&call("Bash", json!({ "command": "sleep 30 && ls" })), work, &ro).unwrap();
        assert_eq!(said["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(said["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(said["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().starts_with("Blocked: "));
        assert_eq!(answer(&call("Bash", json!({ "command": "cargo build" })), work, &ro), None, "nothing said: the engine's own rules decide");
        assert_eq!(answer(&json!({ "tool_name": "Bash" }), work, &ro), None);
        // Set up for Claude Code: the commands and the file tools, with where it works and what it reads.
        let hooks = hooks(Path::new("/Applications/equipe.app/Contents/MacOS/equipe"), work, &[PathBuf::from("/Users/me/My Site")]);
        let entry = &hooks["PreToolUse"][0];
        assert_eq!(entry["matcher"], MATCHER);
        assert_eq!(entry["hooks"][0]["type"], "command");
        assert_eq!(entry["hooks"][0]["command"], "/Applications/equipe.app/Contents/MacOS/equipe hook --work /w/backend-t3 --read-only '/Users/me/My Site'");
    }

    #[test]
    fn in_a_side_chat_the_guard_lets_only_reading_and_the_web_through() {
        for tool in crate::side_chat::CLAUDE_TOOLS {
            assert_eq!(side_chat_refusal(tool), None, "{tool}");
        }
        for tool in ["Bash", "Edit", "Write", "MultiEdit", "NotebookEdit", "Agent", "Task", "TodoWrite", "mcp__equipe__create_ticket", "mcp__equipe__approve", "mcp__browser__browser_click", "SomethingNew"] {
            assert!(side_chat_refusal(tool).is_some_and(|why| why.starts_with("Blocked: ")), "{tool}");
        }
        let call = json!({ "hook_event_name": "PreToolUse", "tool_name": "Write", "tool_input": { "file_path": "/w/x" } });
        assert_eq!(side_chat_answer(&call).unwrap()["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(side_chat_answer(&json!({ "tool_name": "WebSearch", "tool_input": { "query": "q" } })), None, "the engine's own rules decide the rest");
        // Asked about every call, not only commands and files.
        let hooks = side_chat_hooks(Path::new("/Applications/equipe.app/Contents/MacOS/equipe"));
        assert_eq!(hooks["PreToolUse"][0]["matcher"], "*");
        assert_eq!(hooks["PreToolUse"][0]["hooks"][0]["command"], "/Applications/equipe.app/Contents/MacOS/equipe hook --side-chat");
    }

    #[test]
    fn a_shell_line_reads_as_its_commands() {
        let words = |line: &str| parse(line).into_iter().map(|c| c.words.join(" ")).collect::<Vec<_>>();
        assert_eq!(words("a && b; c | d"), ["a", "b", "c", "d"]);
        assert_eq!(words("if test -f x; then rm x; fi"), ["test -f x", "rm x"]);
        assert_eq!(words("echo \"a; b\" 'c && d'"), ["echo a; b c && d"]);
        assert_eq!(words("x=$(date) ls"), ["date", "ls"]);
        let looped: Vec<bool> = parse("while true; do sleep 1; done; sleep 2").iter().map(|c| c.looped).collect();
        assert_eq!(looped, [false, true, false]);
        assert_eq!(parse("echo a > out.txt 2>&1")[0].redirects, ["out.txt"]);
        assert!(parse("sleep 30 & wait")[0].background);
    }
}
