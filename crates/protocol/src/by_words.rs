//! Deciding in the person's own words: what is safe to say instead of
//! clicking, how a yes and a no are told apart, and what is kept of it.
//!
//! Only the person's own words count — what they typed or dictated into a
//! composer of equipe's on their own device ([`crate::Command::Say`]). Never
//! a member's message, a model's output, a tool's result, a page, a file, or
//! words quoted or pasted into the person's message ([`own_words`]). A model
//! may suggest which waiting request the person means; equipe decides, here,
//! from fixed word lists (`locales/<code>.json`, every language at once).
//!
//! Every request that waits on the person has a [`Kind`], and every kind one
//! [`Tier`] — defined in one place, [`Kind::tier`]:
//!
//! - [`Tier::Low`]: the person's words that name it act at once, after a few
//!   seconds' undo window ([`UNDO_MS`]) — in every mode.
//! - [`Tier::High`]: a card's click — or, in "all by words", equipe reads
//!   back exactly what will happen and the person's next own message must be
//!   a clear yes ([`reply`]).
//! - [`Tier::Outward`]: irreversible or leaving the squad — as high, then a
//!   cancel window ([`CANCEL_MS`]) before it runs.
//!
//! Some kinds never go by words, in any mode ([`Reach::Card`]), and what
//! equipe refuses stays refused ([`Reach::Never`]).

use serde::{Deserialize, Serialize};

use crate::SessionId;

/// How long the undo of an action taken at once on words lasts.
pub const UNDO_MS: u64 = 5_000;
/// How long an irreversible or outward action waits, once confirmed, for a
/// "cancel".
pub const CANCEL_MS: u64 = 10_000;
/// How long a read-back waits for the person's answer before it lapses.
pub const READ_BACK_MS: u64 = 120_000;
/// How many unclear answers a read-back asks again after, before it holds.
pub const ASK_AGAIN: u32 = 2;
/// The most of a message read for a decision: past it, it says more than a
/// decision, and goes to the member as it is.
pub const MOST_CHARS: usize = 80;

/// How the person's words came in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// Typed into a composer.
    #[default]
    Typed,
    /// Dictated into a composer, the transcript sent as the person left it.
    Dictated,
    /// Said aloud in a phone's call mode.
    Voice,
}

impl Via {
    pub fn label(self) -> &'static str {
        match self {
            Via::Typed => "typed",
            Via::Dictated => "dictated",
            Via::Voice => "voice",
        }
    }
}

/// How a decision a device made sure of on its own — a phone's call mode
/// reading the request back and checking the confirming words on the device —
/// was said: kept in the log with the decision ([`crate::Command::Decide`],
/// [`crate::Command::ConfirmAccept`]). Never what lets it through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Said {
    pub via: Via,
    /// The confirming words, as the device heard them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub utterance: String,
}

/// How much is at stake in saying yes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Undone in a moment, nothing leaves the squad: words act at once.
    Low,
    /// A card, or — all by words — read back and a clear yes.
    High,
    /// Irreversible, or out of the squad's hands — deploying, paying, sending
    /// outside, widening permissions, secrets, team changes: as high, then a
    /// cancel window.
    Outward,
}

/// Whether a kind can go by words at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// By words, as its tier says.
    Words,
    /// Only ever its card, in every mode.
    Card,
    /// Refused outright, in every mode.
    Never,
}

/// Every kind of thing that waits on the person, by what a yes to it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Handed-in work done in the squad's folder (research, writing,
    /// deciding) or in the world (an action), accepted.
    AcceptTask,
    /// A member's proposal kept as an idea.
    KeepIdea,
    /// A member's proposal made real: on the board, ready.
    TakeOn,
    /// A saved routine, run once now.
    RunRoutine,
    /// Handed-in work sent back with what to change.
    SendBack,
    /// Accepted code left on its branch, no longer asking to be merged.
    KeepOnBranch,
    /// A member's questions answered: the person's words go to it as they are.
    Answer,
    /// A waiting ticket given what it needed: the words go to its member.
    Resume,
    /// Handed-in code accepted, left on its branch.
    AcceptCode,
    /// Code merged into the project's main branch, on this Mac.
    Merge,
    /// A member's request for permission, once, within the squad's reach.
    AllowOnce,
    /// A routine the PM proposed, turned on to run on its schedule.
    ApproveRoutine,
    /// Members moved to another engine (another plan, another bill).
    MoveEngine,
    /// Merged and pushed to `origin`.
    Push,
    /// A request for permission that pushes, publishes or deploys.
    AllowOutward,
    /// "Always allow": a rule, from now on, for the whole squad.
    AllowAlways,
    /// A member acting in the person's name: signing up, accepting terms, paying.
    ActInName,
    /// A message sent from the person's own apps on this Mac.
    SendAsPerson,
    /// A kept login handed to a member to sign in with.
    SignInWithLogin,
    /// A request for permission that reads or writes where keys live.
    TouchSecrets,
    /// A setting the PM asked for that widens what members do alone.
    WidenSetting,
    /// A member joining or leaving the team.
    TeamChange,
    /// A secret's value: typed into its card, never into a conversation.
    GiveSecret,
    /// A new phone paired with this Mac: the root of what may decide.
    PairDevice,
    /// What equipe never allows, in any mode, whoever asks.
    Blocked,
}

impl Kind {
    /// Every kind, for the table and its tests.
    pub const ALL: [Kind; 25] = [
        Kind::AcceptTask,
        Kind::KeepIdea,
        Kind::TakeOn,
        Kind::RunRoutine,
        Kind::SendBack,
        Kind::KeepOnBranch,
        Kind::Answer,
        Kind::Resume,
        Kind::AcceptCode,
        Kind::Merge,
        Kind::AllowOnce,
        Kind::ApproveRoutine,
        Kind::MoveEngine,
        Kind::Push,
        Kind::AllowOutward,
        Kind::AllowAlways,
        Kind::ActInName,
        Kind::SendAsPerson,
        Kind::SignInWithLogin,
        Kind::TouchSecrets,
        Kind::WidenSetting,
        Kind::TeamChange,
        Kind::GiveSecret,
        Kind::PairDevice,
        Kind::Blocked,
    ];

    /// The one place a kind's stake is set.
    pub fn tier(self) -> Tier {
        match self {
            Kind::AcceptTask | Kind::KeepIdea | Kind::TakeOn | Kind::RunRoutine | Kind::SendBack | Kind::KeepOnBranch | Kind::Answer | Kind::Resume => Tier::Low,
            Kind::AcceptCode | Kind::Merge | Kind::AllowOnce | Kind::ApproveRoutine | Kind::MoveEngine => Tier::High,
            Kind::Push
            | Kind::AllowOutward
            | Kind::AllowAlways
            | Kind::ActInName
            | Kind::SendAsPerson
            | Kind::SignInWithLogin
            | Kind::TouchSecrets
            | Kind::WidenSetting
            | Kind::TeamChange
            | Kind::GiveSecret
            | Kind::PairDevice
            | Kind::Blocked => Tier::Outward,
        }
    }

    /// Whether it can go by words at all.
    pub fn reach(self) -> Reach {
        match self {
            // A secret said in a conversation is in its transcript; a phone
            // paired by words would let anyone's phone decide from then on;
            // a rule or a setting that widens what members do alone lasts.
            Kind::GiveSecret | Kind::PairDevice | Kind::AllowAlways | Kind::WidenSetting => Reach::Card,
            Kind::Blocked => Reach::Never,
            _ => Reach::Words,
        }
    }

    /// Whether the person's words go on to the member as they are, as the
    /// way to do it: nothing for equipe to decide.
    pub fn in_conversation(self) -> bool {
        matches!(self, Kind::Answer | Kind::Resume)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::AcceptTask => "accept_task",
            Kind::KeepIdea => "keep_idea",
            Kind::TakeOn => "take_on",
            Kind::RunRoutine => "run_routine",
            Kind::SendBack => "send_back",
            Kind::KeepOnBranch => "keep_on_branch",
            Kind::Answer => "answer",
            Kind::Resume => "resume",
            Kind::AcceptCode => "accept_code",
            Kind::Merge => "merge",
            Kind::AllowOnce => "allow_once",
            Kind::ApproveRoutine => "approve_routine",
            Kind::MoveEngine => "move_engine",
            Kind::Push => "push",
            Kind::AllowOutward => "allow_outward",
            Kind::AllowAlways => "allow_always",
            Kind::ActInName => "act_in_name",
            Kind::SendAsPerson => "send_as_person",
            Kind::SignInWithLogin => "sign_in_with_login",
            Kind::TouchSecrets => "touch_secrets",
            Kind::WidenSetting => "widen_setting",
            Kind::TeamChange => "team_change",
            Kind::GiveSecret => "give_secret",
            Kind::PairDevice => "pair_device",
            Kind::Blocked => "blocked",
        }
    }
}

/// Where a words request stands while it waits on the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Low: it runs when the undo window closes, unless undone.
    Undo,
    /// Several things fit the words: which one, by its number.
    Which,
    /// Read back: the person's next own message must be a clear yes.
    ReadBack,
    /// Confirmed, outward: it runs when the cancel window closes.
    Countdown,
}

/// A request made in words, waiting on the person: shown on every screen
/// above the composer, and answered by their next own message there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Live {
    pub id: u64,
    /// The conversation the words were said in.
    pub session: SessionId,
    pub stage: Stage,
    pub tier: Tier,
    pub kind: Kind,
    /// What will happen, in equipe's own words — never a member's summary:
    /// the action and its target, amounts and recipients as they were asked.
    pub read_back: String,
    /// Each thing it does, or (choosing) each that fits, in order: the
    /// person names one by its number.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    /// What a member wrote about it — why, what it asks to do in the person's
    /// name — shown as the member's, apart from equipe's own lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quotes: Vec<Quote>,
    /// Until when (unix milliseconds) the undo or cancel window is open, or
    /// the read-back waits.
    pub until: u64,
    /// The person's words it came from.
    pub said: String,
    pub via: Via,
    /// Answers taken as unclear so far.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub asked_again: u32,
    /// The paired phone it was said on; `None`: a screen of this Mac's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// The most of a member's words a read-back quotes.
pub const QUOTE_CHARS: usize = 240;

/// A member's own words beside a read-back: never equipe's voice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    /// The member who wrote it.
    pub by: String,
    pub text: String,
}

impl Quote {
    /// `text` as quoted: its line breaks and runs of space made one space,
    /// clipped to [`QUOTE_CHARS`].
    pub fn of(by: &str, text: &str) -> Self {
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let text = if flat.chars().count() > QUOTE_CHARS { format!("{}…", flat.chars().take(QUOTE_CHARS).collect::<String>()) } else { flat };
        Self { by: by.to_owned(), text }
    }
}

/// How a words request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Done, as its card's button would have.
    Done,
    /// Undone in its undo window: nothing happened.
    Undone,
    /// Cancelled in its cancel window: nothing happened.
    Cancelled,
    /// Said no to (or unclear too often): it waits on its card.
    Held,
    /// Nobody answered the read-back in time: it waits on its card.
    Lapsed,
    /// Decided otherwise meanwhile (a card, another screen): nothing to do.
    Moot,
    /// Not done: high in a mode that takes cards for it — its card is up.
    NeedsCard,
    /// Still waiting when the daemon stopped: nothing happened, its card is up.
    Dropped,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::Undone => "undone",
            Outcome::Cancelled => "cancelled",
            Outcome::Held => "held",
            Outcome::Lapsed => "lapsed",
            Outcome::Moot => "moot",
            Outcome::NeedsCard => "needs_card",
            Outcome::Dropped => "dropped",
        }
    }
}

/// How equipe read one of the person's answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Yes,
    No,
    /// A yes with more said, or not plainly enough: asked again.
    Unclear,
    Cancel,
    Undo,
    /// A number, choosing.
    Pick,
    /// Something else: on to the member, the request left as it was.
    Other,
}

/// One answer, as kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heard {
    pub words: String,
    pub read: Reply,
    /// Unix milliseconds.
    pub at: u64,
    pub via: Via,
}

/// Where a words decision may have gone wrong, for the person to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flag {
    Undone,
    Cancelled,
    AskedAgain,
    /// What was done by words was taken back later: reopened, sent back,
    /// cancelled, or its merge reverted.
    RevertedLater,
    /// The words fit several things, or an answer was unclear.
    Ambiguous,
    Held,
    Lapsed,
    /// Still waiting when the daemon stopped.
    Dropped,
}

/// The learning log's line: one words decision, everything about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub session: SessionId,
    pub kind: Kind,
    pub tier: Tier,
    pub via: Via,
    /// "All by words" was on where it was said.
    #[serde(default)]
    pub all_by_words: bool,
    /// The person's words that asked for it.
    pub said: String,
    /// What equipe read back (empty: low, acted at once; or a device's own).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub read_back: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quotes: Vec<Quote>,
    /// Every answer after, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<Heard>,
    /// The tickets it was about, to find later reversals by.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tickets: Vec<crate::TicketId>,
    /// Unix milliseconds: said, read back, decided (yes, undone, no…), closed.
    pub said_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_back_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<u64>,
    pub closed_at: u64,
    pub outcome: Outcome,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub asked_again: u32,
    /// The words fit more than one thing, and the person chose.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub chose: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Worked out when the log is read: where it may have gone wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<Flag>,
}

impl Entry {
    /// The flags its own record shows; [`Flag::RevertedLater`] is the
    /// reader's to add, from what happened after.
    pub fn own_flags(&self) -> Vec<Flag> {
        let mut flags = Vec::new();
        match self.outcome {
            Outcome::Undone => flags.push(Flag::Undone),
            Outcome::Cancelled => flags.push(Flag::Cancelled),
            Outcome::Held => flags.push(Flag::Held),
            Outcome::Lapsed => flags.push(Flag::Lapsed),
            Outcome::Dropped => flags.push(Flag::Dropped),
            _ => {}
        }
        if self.asked_again > 0 {
            flags.push(Flag::AskedAgain);
        }
        if self.chose || self.answers.iter().any(|a| a.read == Reply::Unclear) {
            flags.push(Flag::Ambiguous);
        }
        flags
    }
}

// ── the person's own words ──

/// The person's own words in a message: without what they quoted — a
/// Markdown blockquote (`> …`, a quoted reply's passage), a fenced block —
/// or the marks a reply carries (`<!-- reply 1 -->`), trimmed. Pasted text
/// is the screen's to leave out (`Say`'s `pasted`).
pub fn own_words(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced || trimmed.starts_with('>') || (trimmed.starts_with("<!--") && trimmed.ends_with("-->")) {
            continue;
        }
        if !trimmed.is_empty() {
            out.push(trimmed);
        }
    }
    out.join("\n")
}

/// Whether `text` — lower case — holds `word`: as a whole word when it is
/// written in Latin letters (so "no" is not in "now"), anywhere otherwise.
pub fn holds(text: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let latin = word.chars().all(|c| c.is_ascii() || c.is_alphabetic() && !is_cjk(c));
    if !latin {
        return text.contains(word);
    }
    let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    let mut from = 0;
    while let Some(at) = text[from..].find(word) {
        let start = from + at;
        let end = start + word.len();
        let starts = edge(text[..start].chars().next_back()) || !word.starts_with(|c: char| c.is_alphanumeric());
        let ends = edge(text[end..].chars().next()) || !word.ends_with(|c: char| c.is_alphanumeric());
        if starts && ends {
            return true;
        }
        from = start + word.chars().next().map_or(1, char::len_utf8);
    }
    false
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0x3040..=0x30FF | 0x3130..=0x318F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// The languages the person's words are read in: theirs — the screen's,
/// and the one they write in when it is known — and English, always. Never
/// every language at once: one language's no is another's yes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tongues(Vec<&'static str>);

impl Tongues {
    /// From language codes (`ko`, `zh-Hant`); ones not known are left out.
    pub fn of(codes: &[&str]) -> Self {
        let mut out = vec!["en"];
        for code in codes {
            if let Some(known) = crate::words::CODES.iter().find(|c| **c == *code)
                && !out.contains(known)
            {
                out.push(known);
            }
        }
        Self(out)
    }

    pub fn codes(&self) -> &[&'static str] {
        &self.0
    }

    /// `key`'s words in these languages.
    pub fn words(&self, key: &str) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for code in &self.0 {
            for word in crate::words::of(code, key) {
                if !out.contains(word) {
                    out.push(word);
                }
            }
        }
        out
    }

    /// Whether any of `key`'s words is in `text` (lower case), each as [`holds`] finds it.
    pub fn any(&self, text: &str, key: &str) -> bool {
        self.words(key).iter().any(|w| holds(text, w))
    }

    /// Whether any of `key`'s words is written anywhere in `text`, even inside a word.
    pub fn written(&self, text: &str, key: &str) -> bool {
        self.words(key).iter().any(|w| text.contains(w))
    }
}

/// The words of an answer, lower case, the punctuation and emoji between
/// them taken for spaces.
pub fn tokens(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    lower
        .split(|c: char| c.is_whitespace() || (!c.is_alphanumeric() && !matches!(c, '\'' | '’' | '-' | '#')))
        .map(|w| w.trim_matches(|c: char| matches!(c, '\'' | '’' | '-')).to_owned())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Whether the words say no, or hold off: any negation, anywhere, outside
/// what is a yes word on its own (a yes word may hold a negation's letters).
pub fn says_no(t: &Tongues, text: &str) -> bool {
    let yes = t.words("match.yes");
    let with = t.words("match.yes_with");
    let rest: Vec<String> = tokens(text).into_iter().filter(|w| !yes.contains(&w.as_str()) && !with.contains(&w.as_str())).collect();
    t.any(&rest.join(" "), "match.no")
}

/// The most words a yes said aloud or dictated may have, for what is high
/// risk: a short reply on its own, nothing more said with it.
pub const SPOKEN_YES_WORDS: usize = 3;

/// How a read-back's answer reads.
///
/// - [`Reply::Yes`]: a clear yes — only yes words and the few that go with
///   one ("yes please", "ok go"). `strict` (said aloud or dictated, about
///   something high risk): also short, on one line, and no negation's letters
///   anywhere, even inside a yes word.
/// - [`Reply::No`]: any negation.
/// - [`Reply::Unclear`]: a yes word with more said, or a question: asked again.
/// - [`Reply::Other`]: no yes in it at all — something else, for the member.
pub fn reply(t: &Tongues, text: &str, strict: bool) -> Reply {
    let words = own_words(text);
    if words.is_empty() {
        return Reply::Other;
    }
    if says_no(t, &words) {
        return Reply::No;
    }
    let yes = t.words("match.yes");
    let with = t.words("match.yes_with");
    let tokens = tokens(&words);
    let any_yes = tokens.iter().any(|w| yes.contains(&w.as_str()));
    if !any_yes {
        return Reply::Other;
    }
    let plain = tokens.iter().all(|w| yes.contains(&w.as_str()) || with.contains(&w.as_str()));
    let asked = words.contains(['?', '？']);
    let short = tokens.len() <= SPOKEN_YES_WORDS && !words.contains('\n') && !t.any(&words.to_lowercase(), "match.no");
    if plain && !asked && (!strict || short) { Reply::Yes } else { Reply::Unclear }
}

/// Whether the words stop what waits to run: a cancel, an undo, or any no.
pub fn stops(t: &Tongues, text: &str) -> bool {
    let words = own_words(text).to_lowercase();
    !words.is_empty() && (says_no(t, &words) || t.any(&words, "match.cancel") || t.any(&words, "match.undo"))
}

/// The number the person picked, choosing among `count`: `2`, `#2`, or the
/// number with a counter word after it — alone, or with a yes word.
pub fn picked(t: &Tongues, text: &str, count: usize) -> Option<usize> {
    let words = own_words(text);
    let yes = t.words("match.yes");
    let with = t.words("match.yes_with");
    let counters = t.words("match.number");
    let mut found = None;
    for token in tokens(&words) {
        let digits: String = token.trim_start_matches('#').chars().take_while(char::is_ascii_digit).collect();
        let rest = &token.trim_start_matches('#')[digits.len()..];
        if !digits.is_empty() && (rest.is_empty() || counters.contains(&rest)) {
            if found.is_some() {
                return None;
            }
            found = Some(digits.parse::<usize>().ok().filter(|n| (1..=count).contains(n))?);
        } else if !yes.contains(&token.as_str()) && !with.contains(&token.as_str()) {
            return None;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_one_tier_and_the_outward_ones_are_the_irreversible() {
        for kind in Kind::ALL {
            let tier = kind.tier();
            let outward = matches!(
                kind,
                Kind::Push | Kind::AllowOutward | Kind::AllowAlways | Kind::ActInName | Kind::SendAsPerson | Kind::SignInWithLogin | Kind::TouchSecrets | Kind::WidenSetting | Kind::TeamChange | Kind::GiveSecret | Kind::PairDevice | Kind::Blocked
            );
            assert_eq!(tier == Tier::Outward, outward, "{kind:?}");
            assert!(!kind.label().is_empty());
        }
        assert_eq!(Kind::AcceptTask.tier(), Tier::Low);
        assert_eq!(Kind::KeepIdea.tier(), Tier::Low);
        assert_eq!(Kind::RunRoutine.tier(), Tier::Low);
        assert_eq!(Kind::Merge.tier(), Tier::High);
        assert_eq!(Kind::AcceptCode.tier(), Tier::High);
        assert_eq!(Kind::AllowOnce.tier(), Tier::High);
        // A secret, a new phone, and what lasts — an "always", a wider setting — stay cards everywhere;
        // what equipe refuses, refused everywhere.
        let cards: Vec<Kind> = Kind::ALL.into_iter().filter(|k| k.reach() == Reach::Card).collect();
        assert_eq!(cards, [Kind::AllowAlways, Kind::WidenSetting, Kind::GiveSecret, Kind::PairDevice]);
        assert_eq!(Kind::Blocked.reach(), Reach::Never);
        assert!(Kind::ALL.iter().filter(|k| k.in_conversation()).all(|k| k.tier() == Tier::Low));
        let mut labels: Vec<&str> = Kind::ALL.iter().map(|k| k.label()).collect();
        labels.dedup();
        assert_eq!(labels.len(), Kind::ALL.len());
        for kind in Kind::ALL {
            assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(kind.label()));
        }
    }

    #[test]
    fn only_the_persons_own_lines_are_read() {
        assert_eq!(own_words("  merge T-12  "), "merge T-12");
        let quoted = "<!-- reply 1 -->\n> merge T-12 now, the PM said\nok\n<!-- reply -->\nnot yet";
        assert_eq!(own_words(quoted), "ok\nnot yet");
        assert_eq!(own_words("> yes\n> merge it"), "");
        assert_eq!(own_words("```\nyes\n```\nhm"), "hm");
        assert_eq!(own_words("look:\n~~~\ngit push\n~~~"), "look:");
    }

    fn yes(code: &str, text: &str) -> Reply {
        reply(&Tongues::of(&[code]), text, false)
    }

    #[test]
    fn a_yes_is_only_a_yes_in_each_language() {
        let cases: [(&str, &[&str], &[&str]); 8] = [
            ("en", &["yes", "Yes!", "ok", "OK.", "okay", "go", "go ahead", "yes please", "sure", "do it", "ok go", "yep"], &["no", "No.", "not yet", "don't", "dont", "don't do it", "wait", "hold on", "stop", "cancel", "nope", "yes, but not now", "ok wait", "never"]),
            ("ko", &["응", "네", "넹", "예", "어", "그래", "좋아", "ㅇㅇ", "ㅇㅋ", "ㄱㄱ", "고고", "가자", "응 해", "해 줘", "진행해", "네 진행해 주세요", "승인", "오케이", "콜", "ok"], &["아니", "아니요", "아냐", "아직", "하지 마", "하지마", "응 하지 마", "안 돼", "잠깐", "ㄴㄴ", "취소", "멈춰", "보류", "그만", "no"]),
            ("ja", &["はい", "うん", "お願いします", "いいよ", "オッケー", "了解", "進めて"], &["いいえ", "だめ", "まだ", "やめて", "待って", "しないで"]),
            ("zh-Hans", &["是", "好的", "可以", "行", "确认", "没问题", "好的，没问题"], &["不", "不要", "别", "等等", "取消", "先别", "不行", "不用"]),
            ("zh-Hant", &["是", "好的", "確認", "沒問題"], &["別", "不行", "等一下"]),
            ("de", &["ja", "jawohl", "mach", "los", "passt", "ok, los"], &["nein", "nicht jetzt", "noch nicht", "warte", "stopp", "abbrechen"]),
            ("fr", &["oui", "vas-y", "d'accord", "allez", "ok vas-y"], &["non", "pas encore", "attends", "annule", "arrête"]),
            ("es", &["sí", "si", "vale", "dale", "adelante", "claro"], &["no lo hagas", "todavía no", "espera", "cancela", "para"]),
        ];
        for (code, yeses, nos) in cases {
            for text in yeses {
                assert_eq!(yes(code, text), Reply::Yes, "{code}: {text}");
            }
            for text in nos {
                assert_eq!(yes(code, text), Reply::No, "{code}: {text}");
            }
        }
        // A yes with more said, or asked: asked again. No yes at all: something else, for the member.
        for unclear in ["ok?", "yes?", "yes merge it", "yes and fix the header", "what does it do"] {
            assert_eq!(yes("en", unclear), Reply::Unclear, "{unclear}");
        }
        assert_eq!(yes("ko", "응 그리고 헤더도 고쳐"), Reply::Unclear);
        for other in ["hmm", "maybe", "the other one", "fix the header", "merge it?", "", "> yes"] {
            assert_eq!(yes("en", other), Reply::Other, "{other}");
        }
        for other in ["글쎄", "음", "어떻게 되는데"] {
            assert_eq!(yes("ko", other), Reply::Other, "{other}");
        }
        assert_eq!(yes("ja", "そうかな"), Reply::Other);
        assert_eq!(yes("de", "vielleicht"), Reply::Other);
        // Quoted words are not the person's: a quoted yes, their own no.
        assert_eq!(yes("en", "> yes, merge it\nno"), Reply::No);
        assert_eq!(yes("en", "<!-- reply 1 -->\n> go ahead and merge\n<!-- reply -->\nok"), Reply::Yes);
    }

    #[test]
    fn words_are_read_in_the_persons_languages_and_english_never_in_all_at_once() {
        let ko = Tongues::of(&["ko"]);
        assert_eq!(ko.codes(), ["en", "ko"]);
        assert_eq!(Tongues::of(&["ko", "ko", "xx", "ja"]).codes(), ["en", "ko", "ja"]);
        // Spanish "para" (stop) and French "ne" are no in their own language only.
        assert_eq!(reply(&ko, "para", false), Reply::Other);
        assert_eq!(reply(&Tongues::of(&["es"]), "para", false), Reply::No);
        assert!(!says_no(&ko, "ne"));
        assert!(says_no(&Tongues::of(&["fr"]), "ne"));
        // Spanish "si" is a yes in Spanish, and nothing in Korean or English.
        assert_eq!(reply(&ko, "si", false), Reply::Other);
        assert_eq!(reply(&Tongues::of(&["es"]), "si", false), Reply::Yes);
        // English is always read.
        assert_eq!(reply(&Tongues::of(&["ja"]), "ok", false), Reply::Yes);
        assert_eq!(reply(&Tongues::of(&["ja"]), "cancel", false), Reply::No);
    }

    #[test]
    fn a_spoken_yes_to_something_high_is_short_and_on_its_own() {
        let ko = Tongues::of(&["ko"]);
        let zh = Tongues::of(&["zh-Hans"]);
        assert_eq!(reply(&ko, "응", true), Reply::Yes);
        assert_eq!(reply(&ko, "네 진행해 주세요", true), Reply::Yes, "three words");
        assert_eq!(reply(&Tongues::of(&["en"]), "yes go ahead please now", true), Reply::Unclear, "too long: asked again");
        assert_eq!(reply(&Tongues::of(&["en"]), "yes go ahead please now", false), Reply::Yes, "typed, it reads as plain");
        // A yes word holding a negation's letters is a yes typed, asked again said aloud.
        assert_eq!(reply(&zh, "没问题", false), Reply::Yes);
        assert_eq!(reply(&zh, "没问题", true), Reply::Unclear);
        assert_eq!(reply(&ko, "응\n해", true), Reply::Unclear, "on one line");
        assert_eq!(reply(&ko, "아니", true), Reply::No);
    }

    #[test]
    fn a_no_is_a_whole_word_and_never_inside_another() {
        let en = Tongues::of(&["en"]);
        assert!(holds("no", "no") && holds("no way", "no") && !holds("now", "no") && !holds("know", "no") && !holds("nothing", "no"));
        assert!(holds("don't", "don't") && holds("ok, stop!", "stop"));
        assert!(holds("하지 마세요", "하지 마"));
        assert!(!says_no(&en, "now") && !says_no(&en, "ok now"));
    }

    #[test]
    fn a_cancel_an_undo_or_any_no_stops_what_waits_to_run() {
        for (code, stop) in [("en", "cancel"), ("en", "stop"), ("en", "undo"), ("en", "no"), ("en", "wait"), ("ko", "취소"), ("ko", "멈춰"), ("ko", "되돌려"), ("ko", "그만"), ("ja", "キャンセル"), ("ja", "取り消して"), ("zh-Hans", "取消"), ("zh-Hans", "撤销"), ("de", "abbrechen"), ("de", "rückgängig"), ("fr", "annule"), ("es", "cancela"), ("es", "deshacer")] {
            assert!(stops(&Tongues::of(&[code]), stop), "{code}: {stop}");
        }
        for go_on in ["ok", "and then fix the header", "", "> cancel"] {
            assert!(!stops(&Tongues::of(&["en"]), go_on), "{go_on}");
        }
    }

    #[test]
    fn a_pick_is_one_number_in_range() {
        let (en, ko, ja, zh) = (Tongues::of(&["en"]), Tongues::of(&["ko"]), Tongues::of(&["ja"]), Tongues::of(&["zh-Hans"]));
        assert_eq!(picked(&en, "2", 3), Some(2));
        assert_eq!(picked(&en, "#1", 3), Some(1));
        assert_eq!(picked(&ko, "2번", 3), Some(2));
        assert_eq!(picked(&ja, "2番", 3), Some(2));
        assert_eq!(picked(&zh, "2号", 3), Some(2));
        assert_eq!(picked(&en, "ok 3", 3), Some(3));
        assert_eq!(picked(&en, "4", 3), None, "out of range");
        assert_eq!(picked(&en, "1 and 2", 3), None);
        assert_eq!(picked(&en, "1 2", 3), None);
        assert_eq!(picked(&en, "the second", 3), None);
        assert_eq!(picked(&en, "> 1", 3), None);
    }

    #[test]
    fn a_members_words_are_quoted_as_theirs_flat_and_clipped() {
        let q = Quote::of("pm", "Pay $49.00\n\n  a month   to Vercel");
        assert_eq!(q, Quote { by: "pm".into(), text: "Pay $49.00 a month to Vercel".into() });
        let long = Quote::of("pm", &"x".repeat(QUOTE_CHARS + 10));
        assert_eq!(long.text.chars().count(), QUOTE_CHARS + 1);
        assert!(long.text.ends_with('…'));
    }

    #[test]
    fn decisions_said_and_their_log_go_on_the_wire_as_they_are() {
        let said = Said { via: Via::Voice, utterance: "응 해".into() };
        assert_eq!(serde_json::to_value(&said).unwrap(), serde_json::json!({ "via": "voice", "utterance": "응 해" }));
        let entry = Entry {
            id: 1,
            session: 2,
            kind: Kind::Merge,
            tier: Tier::High,
            via: Via::Typed,
            all_by_words: true,
            said: "merge T-12".into(),
            read_back: "Merge T-12".into(),
            items: vec![],
            quotes: vec![],
            answers: vec![Heard { words: "hmm".into(), read: Reply::Unclear, at: 5, via: Via::Typed }, Heard { words: "yes".into(), read: Reply::Yes, at: 9, via: Via::Typed }],
            tickets: vec![12],
            said_at: 1,
            read_back_at: Some(2),
            decided_at: Some(9),
            closed_at: 9,
            outcome: Outcome::Done,
            asked_again: 1,
            chose: false,
            device: None,
            flags: vec![],
        };
        assert_eq!(entry.own_flags(), [Flag::AskedAgain, Flag::Ambiguous]);
        let back: Entry = serde_json::from_value(serde_json::to_value(&entry).unwrap()).unwrap();
        assert_eq!(back, entry);
        assert_eq!(Entry { outcome: Outcome::Undone, asked_again: 0, answers: vec![], ..entry }.own_flags(), [Flag::Undone]);
    }
}
