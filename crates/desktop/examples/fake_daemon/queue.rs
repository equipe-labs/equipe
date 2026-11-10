//! `FAKE_DECIDE=queue`: a long, mixed queue of questions, as a squad that has
//! been asking for nine days builds one — two from backend at the front, a
//! run of the PM's going back days, the designer's, and two decisions the PM
//! asked twice (the second time worded a little differently). The daemon
//! keeps one card per decision ([`decide::collapse`]), and so does this one.
//! `FAKE_DECIDE_AFTER=<ms>`: then the PM asks one more in its conversation,
//! as if answering what the person just said.

use equipe_core::protocol::decide::{self, Choice, Question, QuestionSet};

pub fn on() -> bool {
    std::env::var("FAKE_DECIDE").is_ok_and(|v| v == "queue")
}

const HOUR: u64 = 3600;
const DAY: u64 = 24 * HOUR;

#[allow(clippy::too_many_arguments)]
fn set(id: u64, session: u64, member: &str, ticket: Option<u32>, header: &str, question: &str, options: &[&str], ago: u64, now: u64) -> QuestionSet {
    QuestionSet {
        id,
        session,
        member: member.into(),
        ticket,
        questions: vec![Question { question: question.into(), header: header.into(), options: options.iter().map(|l| Choice { label: (*l).into(), ..Choice::default() }).collect(), ..Question::default() }],
        asked_at: now.saturating_sub(ago),
        review: false,
        retro: None,
    }
}

/// The queue, oldest first, before the duplicates are collapsed.
pub fn asked(now: u64) -> Vec<QuestionSet> {
    let pm = |id, header, question, options: &[&str], ago| set(id, 1, "pm", None, header, question, options, ago, now);
    let mut sets = vec![
        set(9301, 3, "backend", Some(318), "Sign-up", "The browser you handed back still shows the code screen. How did the AWS sign-up go?", &["Finished elsewhere", "No code email", "Not yet"], 5 * DAY + 2 * HOUR, now),
        set(9302, 3, "backend", Some(70), "Page", "What page size should the order list use?", &["20 (Recommended)", "50"], 4 * DAY + 22 * HOUR, now),
        pm(9303, "Secrets", "Saved site passwords won't be readable after the update. What should we do?", &["File + no backup", "Sign first", "Keep the keychain"], 4 * DAY + 16 * HOUR),
        pm(9304, "First run", "Where should people first meet the design kit?", &["Inside the designer", "On the website", "CLI first"], 4 * DAY + 5 * HOUR),
        pm(9305, "Quality", "What should 'really good' mean for the design kit first?", &["Looks right in my app", "Extraction accuracy", "First impression"], 4 * DAY + 2 * HOUR),
        pm(9306, "Sign-in", "How far may a member sign in and browse inside a service?", &["My services only", "Anywhere", "Public pages only"], 4 * DAY),
        pm(9307, "T-338", "T-338 has waited for days and holds frontend up. What should happen to it?", &["Close it, go on", "Give it back"], 3 * DAY + 22 * HOUR),
        pm(9308, "Voice 1/3", "When you speak on the move, which situation do you picture most?", &["Walking with earbuds", "Driving", "From the lock screen"], 3 * DAY + 5 * HOUR),
        pm(9309, "Voice 2/3", "Who answers the call when you talk to the squad live?", &["A realtime voice", "On-device voice"], 2 * DAY + 22 * HOUR),
        pm(9310, "Voice 3/3", "How much may you do by voice during a call?", &["Hand out work only", "Small approvals too", "Every approval"], 2 * DAY + 21 * HOUR),
        set(9311, 3, "backend", Some(411), "Commands", "`scale` has done nothing since 0.19. Which commands should we open?", &["start only", "start + scale", "Bring back scale"], 2 * DAY + 20 * HOUR, now),
        pm(9312, "Face ID", "Should Face ID come off the approvals that have it today?", &["All off", "A setting", "New ones only"], 2 * DAY + 19 * HOUR),
        pm(9313, "Play", "Shipping Android often needs a new Play developer account. Which one?", &["Organisation", "Personal", "Later"], 2 * DAY + 17 * HOUR),
        pm(9314, "T-163", "T-163 has stood still for nine days. What should happen to it?", &["Fold into T-430", "Leave it", "Close it"], 2 * DAY + 16 * HOUR),
        set(9315, 3, "backend", Some(416), "deepmerge", "Three Low advisories remain after the update. How should we handle them?", &["Allow the three", "Vendor a safe copy"], 2 * DAY + 2 * HOUR, now),
        pm(9316, "Latency", "Call mode on the subscription answers in 2.6 s, not 1 s. What should we do?", &["Keep it + a chime", "Try an API key", "Hold call mode"], DAY + 19 * HOUR),
        set(9317, 4, "designer", Some(434), "Call screen", "Which direction should the call screen take?", &["B: captions first", "A: status first", "C: pocket mode"], DAY + 17 * HOUR, now),
        pm(9318, "Self-QA", "Should engineers attach their own screen checks before handing in?", &["Required", "Checklist only", "Not this week"], DAY + 14 * HOUR),
        pm(9319, "Engines", "Which members should move to Codex for the remaining 3 days?", &["designer and mobile", "backend and marketing too", "Keep as is"], DAY + 5 * HOUR),
        // Asked again a few hours later, one option reworded: one card, the newer.
        pm(9320, "Engines", "Which members should move to Codex for the remaining 3 days?", &["designer and mobile", "backend and marketing as well", "Keep as is"], DAY + 2 * HOUR),
        pm(9321, "Feel 1/3", "What should a first-time maker feel in 3 seconds?", &["My team is already working", "A precise tool", "A warm team"], 2 * HOUR + 20 * 60),
    ];
    // The office-hours question asked again just now, reworded, its options new.
    sets.push(pm(9322, "Direction 1/3", "What should a first-time maker feel in the first 3 seconds?", &["My team is already at work", "Calm and dependable", "Something new"], 3 * 60));
    sets
}

/// The queue as the daemon keeps it: each decision once.
pub fn sets(now: u64) -> Vec<QuestionSet> {
    decide::collapse(asked(now))
}

/// What the PM asks after `FAKE_DECIDE_AFTER`: a follow-up to the office hours.
pub fn later(now: u64) -> QuestionSet {
    set(9330, 1, "pm", None, "Direction 2/3", "Which word should the first screen lead with?", &["Team", "Calm", "Craft"], 0, now)
}
