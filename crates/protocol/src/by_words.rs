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
            // paired by words would let anyone's phone decide from then on.
            Kind::GiveSecret | Kind::PairDevice => Reach::Card,
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
        }
    }
}

/// How equipe read one of the person's answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Yes,
    No,
    /// Neither, plainly: asked again.
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

/// Whether any of `key`'s words, in any language, is in `text` (lower case).
pub fn any(text: &str, key: &str) -> bool {
    crate::words::every(key).iter().any(|w| holds(text, w))
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
/// what is a yes word on its own ("没问题" is a yes, though it holds "没").
pub fn says_no(text: &str) -> bool {
    let yes = crate::words::every("match.yes");
    let with = crate::words::every("match.yes_with");
    let rest: Vec<String> = tokens(text).into_iter().filter(|t| !yes.contains(&t.as_str()) && !with.contains(&t.as_str())).collect();
    let rest = rest.join(" ");
    any(&rest, "match.no")
}

/// How a read-back's answer reads: a clear yes — only yes words, and the
/// few that go with one ("yes please", "응 해", "ok go") — a no (any
/// negation), or neither. A question is never a yes.
pub fn reply(text: &str) -> Reply {
    let words = own_words(text);
    if words.is_empty() {
        return Reply::Unclear;
    }
    if says_no(&words) {
        return Reply::No;
    }
    if words.contains(['?', '？']) {
        return Reply::Unclear;
    }
    let yes = crate::words::every("match.yes");
    let with = crate::words::every("match.yes_with");
    let tokens = tokens(&words);
    let plain = tokens.iter().all(|t| yes.contains(&t.as_str()) || with.contains(&t.as_str()));
    if plain && tokens.iter().any(|t| yes.contains(&t.as_str())) {
        Reply::Yes
    } else {
        Reply::Unclear
    }
}

/// Whether the words stop what waits to run: a cancel, an undo, or any no.
pub fn stops(text: &str) -> bool {
    let words = own_words(text).to_lowercase();
    !words.is_empty() && (says_no(&words) || any(&words, "match.cancel") || any(&words, "match.undo"))
}

/// The number the person picked, choosing among `count`: `2`, `#2`, `2번`,
/// `2番`, `2号` — alone, or with a yes word.
pub fn picked(text: &str, count: usize) -> Option<usize> {
    let words = own_words(text);
    let yes = crate::words::every("match.yes");
    let with = crate::words::every("match.yes_with");
    let mut found = None;
    for token in tokens(&words) {
        let digits: String = token.trim_start_matches('#').chars().take_while(char::is_ascii_digit).collect();
        let rest = &token.trim_start_matches('#')[digits.len()..];
        if !digits.is_empty() && (rest.is_empty() || crate::words::every("match.number").contains(&rest)) {
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
        // Only a secret and a new phone stay cards everywhere; what equipe refuses, everywhere.
        let cards: Vec<Kind> = Kind::ALL.into_iter().filter(|k| k.reach() == Reach::Card).collect();
        assert_eq!(cards, [Kind::GiveSecret, Kind::PairDevice]);
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

    #[test]
    fn a_yes_is_only_a_yes_in_every_language() {
        for yes in [
            "yes", "Yes!", "ok", "OK.", "okay", "go", "go ahead", "yes please", "sure", "do it", "ok go", "yep",
            "응", "네", "넹", "예", "어", "그래", "좋아", "ㅇㅇ", "ㅇㅋ", "ㄱㄱ", "고고", "가자", "응 해", "해 줘", "진행해", "네 진행해 주세요", "승인", "오케이", "콜",
            "はい", "うん", "お願いします", "いいよ", "オッケー", "了解", "進めて",
            "是", "好的", "可以", "行", "确认", "没问题", "好的，没问题", "確認", "沒問題",
            "ja", "jawohl", "mach", "los", "passt", "ok, los",
            "oui", "vas-y", "d'accord", "allez", "ok vas-y",
            "sí", "si", "vale", "dale", "adelante", "claro",
        ] {
            assert_eq!(reply(yes), Reply::Yes, "{yes}");
        }
        for no in [
            "no", "No.", "not yet", "don't", "dont", "don't do it", "wait", "hold on", "stop", "cancel", "nope", "yes, but not now", "ok wait", "never",
            "아니", "아니요", "아냐", "아직", "하지 마", "하지마", "응 하지 마", "안 돼", "잠깐", "ㄴㄴ", "취소", "멈춰", "보류", "그만",
            "いいえ", "だめ", "まだ", "やめて", "待って", "しないで",
            "不", "不要", "别", "等等", "取消", "先别", "不行", "別", "不用",
            "nein", "nicht jetzt", "noch nicht", "warte", "stopp", "abbrechen",
            "non", "pas encore", "attends", "annule", "arrête",
            "no lo hagas", "todavía no", "espera", "cancela", "para",
        ] {
            assert_eq!(reply(no), Reply::No, "{no}");
        }
        for unclear in ["ok?", "yes?", "merge it?", "hmm", "maybe", "what does it do", "the other one", "fix the header", "글쎄", "음", "어떻게 되는데", "そうかな", "也许", "vielleicht", "peut-être", "quizás", "", "> yes"] {
            assert_eq!(reply(unclear), Reply::Unclear, "{unclear}");
        }
        // Quoted words are not the person's: a quoted yes, their own no.
        assert_eq!(reply("> yes, merge it\nno"), Reply::No);
        assert_eq!(reply("<!-- reply 1 -->\n> go ahead and merge\n<!-- reply -->\nok"), Reply::Yes);
    }

    #[test]
    fn a_no_is_a_whole_word_and_never_inside_another() {
        assert!(holds("no", "no") && holds("no way", "no") && !holds("now", "no") && !holds("know", "no") && !holds("nothing", "no"));
        assert!(holds("don't", "don't") && holds("ok, stop!", "stop"));
        assert!(holds("하지 마세요", "하지 마"));
        assert!(!says_no("now") && !says_no("ok now"));
    }

    #[test]
    fn a_cancel_an_undo_or_any_no_stops_what_waits_to_run() {
        for stop in ["cancel", "stop", "undo", "no", "wait", "취소", "멈춰", "되돌려", "그만", "キャンセル", "取り消して", "取消", "撤销", "abbrechen", "rückgängig", "annule", "cancela", "deshacer"] {
            assert!(stops(stop), "{stop}");
        }
        for go_on in ["ok", "and then fix the header", "", "> cancel"] {
            assert!(!stops(go_on), "{go_on}");
        }
    }

    #[test]
    fn a_pick_is_one_number_in_range() {
        assert_eq!(picked("2", 3), Some(2));
        assert_eq!(picked("#1", 3), Some(1));
        assert_eq!(picked("2번", 3), Some(2));
        assert_eq!(picked("2番", 3), Some(2));
        assert_eq!(picked("2号", 3), Some(2));
        assert_eq!(picked("ok 3", 3), Some(3));
        assert_eq!(picked("4", 3), None, "out of range");
        assert_eq!(picked("1 and 2", 3), None);
        assert_eq!(picked("1 2", 3), None);
        assert_eq!(picked("the second", 3), None);
        assert_eq!(picked("> 1", 3), None);
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
