//! The daemon's Mac kept awake while it is wanted: members at work, or a
//! screen attached (from another machine over SSH above all — a Mac mini
//! asleep drops the person's messages into a connection that went quiet).
//! The display may sleep; the Mac doesn't. `caffeinate -i`, tied to the
//! daemon's own life (`-w`): it ends when the daemon does, however it ends.

/// The hold on sleep, while there is one.
#[derive(Default)]
pub struct Awake {
    #[cfg(target_os = "macos")]
    hold: Option<std::process::Child>,
}

impl Awake {
    /// Hold the Mac awake, or let it sleep again. True when that changed.
    pub fn want(&mut self, on: bool) -> bool {
        #[cfg(target_os = "macos")]
        {
            // Ended on its own (killed by someone): taken again below if wanted.
            if self.hold.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(Some(_)))) {
                self.hold = None;
            }
            match (on, self.hold.take()) {
                (true, Some(child)) => {
                    self.hold = Some(child);
                    false
                }
                (true, None) => {
                    self.hold = std::process::Command::new("caffeinate")
                        .args(["-i", "-w", &std::process::id().to_string()])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                        .ok();
                    self.hold.is_some()
                }
                (false, Some(mut child)) => {
                    let _ = child.kill();
                    // Reaped off the caller's thread: the core loop never waits on it.
                    std::thread::spawn(move || child.wait());
                    true
                }
                (false, None) => false,
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = on;
            false
        }
    }

    pub fn held(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.hold.is_some()
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        self.want(false);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn the_hold_is_taken_once_and_let_go() {
        let mut awake = Awake::default();
        assert!(awake.want(true) && awake.held());
        assert!(!awake.want(true), "already held");
        assert!(awake.want(false) && !awake.held());
        assert!(!awake.want(false));
    }
}
