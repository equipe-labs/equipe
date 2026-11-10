//! Running the programs that drive a simulator or an emulator (`xcrun simctl`,
//! `axe`, `adb`, `emulator`): behind a trait, so the rules around them are
//! tested against a fake that answers as the real ones do.

use std::io::{Read as _, Write as _};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// What a program said.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Out {
    pub ok: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Out {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Why it failed, in its own words, in a line or two.
    pub fn why(&self) -> String {
        let said = if self.stderr.trim().is_empty() { self.text() } else { self.stderr.clone() };
        let lines: Vec<&str> = said.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let tail = lines[lines.len().saturating_sub(3)..].join(" ");
        if tail.is_empty() { "it failed without saying why".into() } else { tail }
    }
}

/// A program left running (a recording): stopped by an interrupt, as Ctrl-C would.
pub trait Running: Send {
    /// Ask it to finish (SIGINT) and wait up to `wait` for it to; whether it did.
    fn finish(&mut self, wait: Duration) -> bool;
    fn alive(&mut self) -> bool;
}

pub trait Exec: Send + Sync {
    /// Run `program` with `args` (and `stdin`) to its end, at most `timeout`.
    fn run(&self, program: &str, args: &[String], stdin: Option<&[u8]>, timeout: Duration) -> Out;
    /// Start `program` and leave it running.
    fn spawn(&self, program: &str, args: &[String]) -> Result<Box<dyn Running>, String>;
    /// Start `program` on its own, outliving this call (an emulator).
    fn detach(&self, program: &str, args: &[String]) -> Result<(), String>;
    /// Wait, between polls.
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// The real programs.
pub struct Real;

struct Proc(Child);

impl Running for Proc {
    fn finish(&mut self, wait: Duration) -> bool {
        #[cfg(unix)]
        {
            let pid = self.0.id() as libc::pid_t;
            // SAFETY: a signal to our own child, by its pid.
            unsafe { libc::kill(pid, libc::SIGINT) };
        }
        #[cfg(windows)]
        let _ = self.0.kill();
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if matches!(self.0.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
        false
    }

    fn alive(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        if self.alive() {
            self.finish(Duration::from_secs(10));
        }
    }
}

impl Exec for Real {
    fn run(&self, program: &str, args: &[String], stdin: Option<&[u8]>, timeout: Duration) -> Out {
        let mut command = Command::new(program);
        command.args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => return Out { ok: false, stdout: Vec::new(), stderr: format!("{program} couldn't be started: {err}") },
        };
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            let _ = pipe.write_all(input);
        }
        // Read both pipes on their own threads, so a full one never stalls the program.
        let mut out_pipe = child.stdout.take();
        let mut err_pipe = child.stderr.take();
        let out_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(p) = out_pipe.as_mut() {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        });
        let err_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(p) = err_pipe.as_mut() {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        });
        let until = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let stdout = out_reader.join().unwrap_or_default();
        let mut stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
        if status.is_none() {
            stderr.push_str(&format!("\n{program} took longer than {}s and was stopped", timeout.as_secs()));
        }
        Out { ok: status.is_some_and(|s| s.success()), stdout, stderr }
    }

    fn spawn(&self, program: &str, args: &[String]) -> Result<Box<dyn Running>, String> {
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{program} couldn't be started: {e}"))?;
        Ok(Box::new(Proc(child)))
    }

    fn detach(&self, program: &str, args: &[String]) -> Result<(), String> {
        let mut command = Command::new(program);
        command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        // Its own process group: a member's turn ending doesn't take it down.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|e| format!("{program} couldn't be started: {e}"))?;
        // Reaped on its own thread whenever it ends, never left a zombie.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

/// Owned strings from string slices, for `Exec`'s arguments.
pub fn args<const N: usize>(a: [&str; N]) -> Vec<String> {
    a.iter().map(|s| (*s).to_owned()).collect()
}

#[cfg(test)]
pub mod fake {
    //! A stand-in for the programs: each call is recorded, and answered by the
    //! first rule whose words all appear in it.

    use super::*;
    use std::sync::{Arc, Mutex};

    /// What a command's words start with, and what it answers.
    type Rules = Arc<Mutex<Vec<(Vec<String>, Out)>>>;

    #[derive(Default, Clone)]
    pub struct Fake {
        pub calls: Arc<Mutex<Vec<String>>>,
        rules: Rules,
    }

    pub struct Still(pub Arc<Mutex<Vec<String>>>, pub String);

    impl Running for Still {
        fn finish(&mut self, _: Duration) -> bool {
            self.0.lock().unwrap().push(format!("SIGINT {}", self.1));
            true
        }
        fn alive(&mut self) -> bool {
            true
        }
    }

    impl Fake {
        /// Answer calls containing every one of `words` with `stdout` (success).
        pub fn on(&self, words: &str, stdout: &str) -> &Self {
            self.answer(words, Out { ok: true, stdout: stdout.as_bytes().to_vec(), stderr: String::new() })
        }

        pub fn fail(&self, words: &str, stderr: &str) -> &Self {
            self.answer(words, Out { ok: false, stdout: Vec::new(), stderr: stderr.into() })
        }

        pub fn answer(&self, words: &str, out: Out) -> &Self {
            self.rules.lock().unwrap().insert(0, (words.split_whitespace().map(str::to_owned).collect(), out));
            self
        }

        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        pub fn called(&self, words: &str) -> bool {
            let words: Vec<&str> = words.split_whitespace().collect();
            self.calls().iter().any(|c| words.iter().all(|w| c.split(' ').any(|p| p == *w)))
        }
    }

    impl Exec for Fake {
        fn run(&self, program: &str, args: &[String], stdin: Option<&[u8]>, _: Duration) -> Out {
            let line = std::iter::once(program.to_owned()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" ");
            let line = match stdin {
                Some(input) => format!("{line} <<< {}", String::from_utf8_lossy(input)),
                None => line,
            };
            self.calls.lock().unwrap().push(line.clone());
            let parts: Vec<&str> = line.split(' ').collect();
            self.rules
                .lock()
                .unwrap()
                .iter()
                .find(|(words, _)| words.iter().all(|w| parts.contains(&w.as_str())))
                .map(|(_, out)| out.clone())
                .unwrap_or(Out { ok: true, ..Out::default() })
        }

        fn spawn(&self, program: &str, args: &[String]) -> Result<Box<dyn Running>, String> {
            let line = std::iter::once(program.to_owned()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" ");
            self.calls.lock().unwrap().push(format!("spawn {line}"));
            Ok(Box::new(Still(self.calls.clone(), line)))
        }

        fn detach(&self, program: &str, args: &[String]) -> Result<(), String> {
            let line = std::iter::once(program.to_owned()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" ");
            self.calls.lock().unwrap().push(format!("detach {line}"));
            Ok(())
        }

        fn sleep(&self, _: Duration) {}
    }
}
