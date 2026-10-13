//! Watch what the screen's gate sees on this Mac: the last input's age, the
//! lock state, and whether a member's input would go in now. Posts nothing.
//!
//! `cargo run -p equipe-core --example desktop_watch -- [seconds]` — move the
//! mouse while it runs and see members' input held back.

use std::time::Duration;

use equipe_core::desktop::{Desktop, Probe, ThisMac, Use};
use equipe_core::protocol::MemberRef;

fn main() {
    let seconds: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let mut desktop = Desktop::new(Box::new(ThisMac));
    let who = MemberRef { squad: "probe".into(), member: "frontend".into() };
    for _ in 0..seconds * 2 {
        let idle = ThisMac.idle().map_or_else(|| "?".into(), |s| format!("{s:6.1}s"));
        let screen = ThisMac.screen();
        let input = match desktop.ask(&who, Use::Input) {
            Ok(()) => "member input: GO".to_owned(),
            Err(why) => format!("member input: HELD — {why}"),
        };
        println!("t={:5.1}  idle {idle}  {screen:?}  {input}", desktop.now());
        std::thread::sleep(Duration::from_millis(500));
    }
}
