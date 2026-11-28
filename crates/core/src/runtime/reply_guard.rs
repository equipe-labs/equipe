//! The PM's reply, checked before the person reads it. A review of a cloud
//! squad (o1path, 2026-11-26) found what the PM said unreliable:
//!
//! - **Narration between tool calls** reached the person as messages:
//!   "Render a side-by-side sheet.", "Now check live pages with screenshots."
//! - **Replies in the wrong language**: English to a Korean founder after a
//!   fresh conversation or a monitor's wake.
//! - **Claims the state didn't back**: "nothing is waiting on you" while the
//!   goal and a weekly plan waited on them; credits said to be out while they
//!   weren't; "it is live" while its deploy still ran.
//! - **The same paragraph twice**: a nudge made the PM repeat an answer it had
//!   given seconds before.
//! - **Promises nobody tracked**: "I'll fix it with the next change" never
//!   became a ticket.
//!
//! So the PM's conversation's text is held while its turn runs. At each tool
//! call the text before it is either narration (short, one line: it goes on
//! the tool's line, in the work view) or a message, checked; at the turn's
//! end its last message is checked. A message that passes is told as it was;
//! repeated paragraphs are dropped; one in the wrong language or with a claim
//! the state contradicts is not shown, and the PM is asked once to write it
//! again with the facts (its second answer is shown whatever it says). A
//! promise with no board change in the same turn has the PM put it on the
//! board. All of it is plain code: the phrases are each language's in its
//! locale (`guard.*`), the facts the runtime's and the platform's own.
//!
//! A health sweep of the customer teams (2026-11-28) found more of it:
//! replies to equipe's own notes ("The S-2 target is a known loop… I'm leaving
//! this wake alone.", "I didn't plan a sprint: nothing has been built yet"),
//! the team's work codes (T-12, S-1) in what the person reads, and English
//! role names inside Korean ("Designer가 조사 중이에요"). So a turn equipe
//! started — a wake, a monitor's note, a sprint's planning — tells the person
//! nothing unless it put something to them (a card, a question, a sign-in);
//! a message naming a work code is asked again, and a role's English name in
//! a Chinese, Japanese or Korean reply is said in that language.

use std::collections::{HashMap, HashSet, VecDeque};

use super::*;
use crate::i18n::Lang;

/// Text up to this long, on one line, before a tool call is narration.
const NARRATION_MAX: usize = 180;
/// A paragraph shorter than this (letters and digits) is never a repeat.
const REPEAT_MIN: usize = 40;
/// How similar two paragraphs are (character pairs in common) to be one.
const REPEAT_SIMILAR: f64 = 0.85;
/// Paragraphs told to the person that a new one is compared with.
const TOLD_KEPT: usize = 24;
/// How long a told paragraph is compared with.
const TOLD_FOR: u64 = 30 * 60;

#[derive(Default)]
pub(super) struct Guard {
    held: HashMap<SessionId, Held>,
    /// Conversations whose turn now is equipe's, not the person's: a wake, a note, a sprint's planning.
    equipes: HashSet<SessionId>,
    /// Sessions whose last reply was held back and asked again: their next is told whatever it says.
    asked_again: HashSet<SessionId>,
    /// Paragraphs told lately, per conversation: when, and their plain form.
    told: HashMap<SessionId, VecDeque<(u64, String)>>,
}

#[derive(Default)]
struct Held {
    text: String,
    /// The turn changed the board (a ticket made or changed): a promise in it is tracked.
    board: bool,
    /// Why a message of this turn was held back.
    failed: Vec<String>,
    /// The tickets the turn made or changed: held for the person's yes when
    /// its message promises to wait for it (`guard.promise_confirm`).
    tickets: Vec<TicketId>,
    /// The last short line taken for narration: the turn's message after all
    /// when nothing follows it ("Starting on it now." and then a ticket).
    narrated: Option<String>,
    /// The turn put something to the person: a card, a question, a sign-in, a word.
    to_person: bool,
    /// What an equipe turn said before its tool calls, kept for its end.
    kept: String,
}

/// Board tools that put something to the person, by name or by prefix (`propose_`).
const TO_PERSON: [&str; 12] = [
    "ask_decision",
    "ask_person",
    "block_on_person",
    "tell_person",
    "sign_in",
    "show_my_browser",
    "show_references",
    "request_secret",
    "enable_payments",
    "plan_sprint",
    "propose_",
    "connect_",
];

/// Whether `tool` (`mcp__equipe__ask_decision`, or its bare name) puts something to the person.
pub(super) fn puts_to_person(tool: &str) -> bool {
    let name = tool.rsplit("__").next().unwrap_or(tool);
    TO_PERSON.iter().any(|t| if t.ends_with('_') { name.starts_with(t) } else { name == *t })
}

/// Whether the turn about to run in a conversation with `entries` is
/// equipe's: a note of equipe's there (a monitor, a wake, a sprint's
/// planning) and nothing the person said since the last reply. A correction
/// equipe asks of a reply ("show that picture again") leaves no note and
/// follows what the person asked; the person's own routine (`[Routine · …]`,
/// the weekly recap among them) is theirs.
pub(super) fn equipes_turn(entries: &[Entry]) -> bool {
    let mut noted = false;
    for entry in entries.iter().rev() {
        match entry {
            Entry::Reply { .. } => break,
            Entry::User { .. } => return false,
            Entry::System { text } if text.trim_start().starts_with("[Routine ·") => return false,
            Entry::System { .. } => noted = true,
            _ => {}
        }
    }
    noted
}

/// Where the team's work codes start: tickets, sprints, design QA batches.
const CODE_LETTERS: [char; 3] = ['T', 'S', 'B'];

/// The byte ranges of the team's work codes in `text` (T-12, S-1, B-43): a
/// letter, a dash and digits, standing alone ("S-1을" too; "TP-3", "T-shirt"
/// and "COVID-19" are not).
fn code_ranges(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 < bytes.len() {
        let before_ok = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || matches!(bytes[i - 1], b'-' | b'/' | b'_' | b'.'));
        if before_ok && CODE_LETTERS.contains(&(bytes[i] as char)) && bytes[i + 1] == b'-' && bytes[i + 2].is_ascii_digit() {
            let mut end = i + 2;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end == bytes.len() || !(bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'-' | b'_')) {
                out.push((i, end));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The team's work codes `text` names in its prose (not in links, paths or code).
pub(super) fn work_codes(text: &str) -> Vec<String> {
    let prose = prose(text);
    code_ranges(&prose).into_iter().map(|(a, b)| prose[a..b].to_owned()).collect()
}

/// `text` with the work codes taken out: the last resort when a message asked
/// again still names them. "(T-12)" goes whole; "T-12 Pricing page" keeps
/// "Pricing page".
pub(super) fn without_work_codes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (a, b) in code_ranges(text) {
        let (mut a, mut b) = (a, b);
        if text[..a].ends_with('(') && text[b..].starts_with(')') {
            a -= 1;
            b += 1;
        }
        if a < last {
            continue;
        }
        out.push_str(&text[last..a]);
        if text[b..].starts_with(' ') {
            b += 1;
        } else if out.ends_with(' ') && text[b..].chars().next().is_none_or(|c| c.is_ascii_punctuation()) {
            out.pop();
        }
        last = b;
    }
    out.push_str(&text[last..]);
    out
}

/// The roles' English names a reply may use, longest first where one holds another, with the role each is.
const ROLE_WORDS: [(&str, &str); 19] = [
    ("product designer", "designer"),
    ("frontend engineer", "frontend"),
    ("frontend developer", "frontend"),
    ("backend engineer", "backend"),
    ("backend developer", "backend"),
    ("mobile engineer", "mobile"),
    ("software engineer", "engineer"),
    ("security engineer", "security"),
    ("product marketer", "marketing"),
    ("product manager", "pm"),
    ("data analyst", "data"),
    ("qa engineer", "qa"),
    ("designer", "designer"),
    ("frontend", "frontend"),
    ("backend", "backend"),
    ("engineer", "engineer"),
    ("marketer", "marketing"),
    ("developer", "engineer"),
    ("analyst", "data"),
];

/// `text` with the roles' English names said in `lang`, for a language
/// written in its own script — "Designer가" to a Korean founder is
/// "디자이너가". Never inside code, a link's address or a path. In a
/// Latin-script language the English word may be that language's too
/// ("le designer"): left as it is.
pub(super) fn with_roles_in(text: &str, lang: Lang) -> String {
    if !matches!(lang, Lang::Ko | Lang::Ja | Lang::ZhHans | Lang::ZhHant) {
        return text.to_owned();
    }
    let words = crate::i18n::core(lang);
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    // What is never rewritten: code in backticks, a link's address, a word with a path or a scheme in it.
    let mut fixed = vec![false; bytes.len()];
    let mut code = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            code = !code;
        }
        if bytes[i..].starts_with(b"](") {
            let end = text[i..].find(')').map_or(bytes.len(), |e| i + e + 1);
            fixed[i..end].iter_mut().for_each(|f| *f = true);
            i = end;
            continue;
        }
        fixed[i] = code;
        i += 1;
    }
    let mut start = 0;
    for (at, c) in text.char_indices().chain([(text.len(), ' ')]) {
        if c.is_whitespace() {
            if text[start..at].contains("://") || text[start..at].contains('/') {
                fixed[start..at].iter_mut().for_each(|f| *f = true);
            }
            start = at + c.len_utf8();
        }
    }
    let free = |at: usize| at >= bytes.len() || !(bytes[at].is_ascii_alphanumeric() || bytes[at] == b'-' || bytes[at] == b'_');
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'scan: while i < bytes.len() {
        if !fixed[i] && (i == 0 || free(i - 1)) {
            for (english, role) in ROLE_WORDS {
                let end = i + english.len();
                if lower[i..].starts_with(english) && free(end) && !fixed[i..end].iter().any(|f| *f) {
                    let key = format!("role_name.{role}");
                    if let Some(said) = words.own(&key) {
                        out.push_str(said);
                        i = end;
                        continue 'scan;
                    }
                }
            }
        }
        let c = text[i..].chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// What a message's checks found.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Checked {
    /// The message to tell, repeated paragraphs left out (empty: nothing to tell).
    pub text: String,
    /// Why it must not be told as it is.
    pub failed: Vec<String>,
}

/// The facts a message is checked against.
#[derive(Debug, Default, Clone)]
pub(super) struct Facts {
    /// The person's language, as equipe last read it from their own words.
    pub language: Option<Lang>,
    /// What waits on the person now, in a few words each.
    pub waiting: Vec<String>,
    /// Whether the team's credits pause its work; `None`: not known.
    pub paused: Option<bool>,
    /// Projects whose deploy is under way or failed: name and how it stands.
    pub not_live: Vec<(String, String)>,
    /// The person never sees the board (a hosted team, a non-developer): the team's work codes mean nothing to them.
    pub codes_hidden: bool,
}

/// Whether text before a tool call is narration on the way: short, one line.
pub(super) fn narration(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && t.chars().count() <= NARRATION_MAX && !t.contains('\n') && !t.to_lowercase().starts_with("choices:")
}

/// `text` in a plain form for comparing: its words without pictures, links
/// and paths, lower case, letters and digits only. A paragraph that shows a
/// picture is never a repeat (two logos side by side differ by their file).
fn plain(text: &str) -> String {
    if text.contains("![") {
        return String::new();
    }
    let words: Vec<&str> = text.split_whitespace().filter(|w| !w.contains("://") && !w.contains('/')).collect();
    words.join(" ").chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// How alike two plain paragraphs are: the share of their character pairs in common.
fn similarity(a: &str, b: &str) -> f64 {
    let pairs = |s: &str| -> HashSet<(char, char)> {
        let c: Vec<char> = s.chars().collect();
        c.windows(2).map(|w| (w[0], w[1])).collect()
    };
    let (x, y) = (pairs(a), pairs(b));
    if x.is_empty() || y.is_empty() {
        return 0.0;
    }
    x.intersection(&y).count() as f64 / x.union(&y).count() as f64
}

/// Whether a plain paragraph says again one of `seen`.
fn repeats(p: &str, seen: &[&str]) -> bool {
    p.chars().count() >= REPEAT_MIN && seen.iter().any(|s| *s == p || similarity(p, s) >= REPEAT_SIMILAR)
}

/// `text` without paragraphs that say again what it said above or what was told lately (`told`, plain).
pub(super) fn without_repeats(text: &str, told: &[String]) -> String {
    let mut seen: Vec<String> = told.to_vec();
    let mut kept: Vec<&str> = Vec::new();
    for para in text.split("\n\n") {
        let p = plain(para);
        let refs: Vec<&str> = seen.iter().map(String::as_str).collect();
        if repeats(&p, &refs) {
            continue;
        }
        if p.chars().count() >= REPEAT_MIN {
            seen.push(p);
        }
        kept.push(para);
    }
    kept.join("\n\n").trim().to_owned()
}

/// `text` without code, links and paths: what is read for its language.
fn prose(text: &str) -> String {
    let mut out = String::new();
    let mut code = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            code = !code;
            continue;
        }
        if code {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().filter(|w| !w.contains("://") && !w.contains('/') && !w.starts_with('`')).collect();
        out.push_str(&words.join(" "));
        out.push('\n');
    }
    out
}

/// Whether `text` is written in another language than `want`, as far as plain
/// code can tell: as a whole, or by one line of it in another language. A
/// Korean reply that opened on an English line ("I made one picture-saving
/// mistake and two new requests have come in since. Here's where things
/// stand.", inhwadoen, 2026-11-28) read as Korean by its share of Hangul.
pub(super) fn wrong_language(text: &str, want: Lang) -> bool {
    let text = prose(text);
    whole_in_another(&text, want) || text.lines().any(|line| line_in_another(line, want))
}

/// Whether one line of prose is a sentence of another language than `want`.
/// Only what can't be a name or a list: a line in another script, or a line
/// the detector reads as another Latin language with its little words
/// ("Pretendard, Noto Sans KR, Gothic A1" is no English sentence). Between two
/// Latin languages a line alone never decides: a quote is not a reply.
fn line_in_another(line: &str, want: Lang) -> bool {
    // A quote is someone else's words; a link's text is a page's title.
    if line.trim_start().starts_with('>') {
        return false;
    }
    let line = &unquoted(line);
    let letters = line.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 20 {
        return false;
    }
    let cjk = line.chars().filter(|c| ('\u{AC00}'..='\u{D7A3}').contains(c) || ('\u{3040}'..='\u{30FF}').contains(c) || ('\u{4E00}'..='\u{9FFF}').contains(c)).count();
    match want {
        Lang::Ko | Lang::Ja | Lang::ZhHans | Lang::ZhHant => {
            cjk == 0 && crate::language::sentence_language(line).is_some()
        }
        _ => cjk >= 10 && cjk * 10 >= letters * 3,
    }
}

/// `line` without what it quotes ("…", “…”) and without the text of its links (`[…](…)`).
fn unquoted(line: &str) -> String {
    let mut out = String::new();
    let mut inside: Option<char> = None;
    for c in line.chars() {
        match (inside, c) {
            (None, '"') => inside = Some('"'),
            (None, '“') => inside = Some('”'),
            (None, '[') => inside = Some(']'),
            (Some(end), c) if c == end => inside = None,
            (Some(_), _) => {}
            (None, c) => out.push(c),
        }
    }
    out
}

fn whole_in_another(text: &str, want: Lang) -> bool {
    let text = text.to_owned();
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 20 {
        return false;
    }
    let share = |range: &dyn Fn(char) -> bool| text.chars().filter(|c| range(*c)).count() as f64 / letters as f64;
    let hangul = share(&|c| ('\u{AC00}'..='\u{D7A3}').contains(&c) || ('\u{1100}'..='\u{11FF}').contains(&c));
    let kana = share(&|c| ('\u{3040}'..='\u{30FF}').contains(&c));
    let han = share(&|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
    match want {
        Lang::Ko => hangul < 0.25,
        Lang::Ja => kana + han < 0.25,
        Lang::ZhHans | Lang::ZhHant => han < 0.25 || kana > 0.1,
        latin => {
            if hangul + kana + han > 0.3 {
                return true;
            }
            crate::language::detect(&text).is_some_and(|name| name != latin.english_name())
        }
    }
}

/// Whether `text` says one of the phrases a locale lists under `key`, in any language.
fn says(text: &str, key: &str) -> Option<String> {
    let lower = text.to_lowercase();
    Lang::ALL.iter().flat_map(|lang| crate::i18n::core(*lang).get(key).split('|')).map(str::trim).find(|p| !p.is_empty() && lower.contains(&p.to_lowercase())).map(str::to_owned)
}

/// The sentence of `text` that holds `phrase`.
fn sentence_with(text: &str, phrase: &str) -> String {
    let lower = text.to_lowercase();
    let Some(at) = lower.find(&phrase.to_lowercase()) else { return phrase.to_owned() };
    let ends = |c: char| matches!(c, '.' | '!' | '?' | '\n' | '。' | '！' | '？');
    let start = text[..at].rfind(ends).map_or(0, |i| i + text[i..].chars().next().map_or(1, char::len_utf8));
    let end = text[at..].find(ends).map_or(text.len(), |i| at + i + 1);
    text[start..end].trim().chars().take(240).collect()
}

/// A message checked: repeated paragraphs dropped (against `told`, plain),
/// and why it must not be told if its language or a claim is wrong.
pub(super) fn check(text: &str, told: &[String], facts: &Facts) -> Checked {
    let text = without_repeats(text, told);
    let mut failed = Vec::new();
    if text.is_empty() {
        return Checked { text, failed };
    }
    if let Some(want) = facts.language
        && wrong_language(&text, want)
    {
        failed.push(format!("it isn't in {} — the language the person writes in", want.english_name()));
    }
    if !facts.waiting.is_empty() && says(&text, "guard.claim_nothing_waiting").is_some() {
        failed.push(format!("it says nothing waits on the person, but these do now: {}", facts.waiting.join("; ")));
    }
    if facts.paused == Some(false) && says(&text, "guard.claim_credits_out").is_some() {
        failed.push("it says credits ran out or work is paused for them, but the team's credits are fine now (not paused, the `credits` tool says the same)".to_owned());
    }
    if !facts.not_live.is_empty() && says(&text, "guard.claim_live").is_some() {
        let how: Vec<String> = facts.not_live.iter().map(|(p, how)| format!("{p}: {how}")).collect();
        failed.push(format!("it says it is live, but not all of it is yet ({})", how.join("; ")));
    }
    let codes = if facts.codes_hidden { work_codes(&text) } else { Vec::new() };
    if !codes.is_empty() {
        failed.push(format!(
            "it names the team's own work codes ({}): the person never sees those — name the work by what it does for them",
            codes.join(", ")
        ));
    }
    let text = match facts.language {
        Some(lang) => with_roles_in(&text, lang),
        None => text,
    };
    if let Some(said) = quotes_cost(&text) {
        failed.push(format!(
            "it tells the person what the team's work will cost (“{said}”): say what will be done and ask for their OK, with no amount \
in money or credits — only a real purchase from another company, like a domain, keeps its real price"
        ));
    }
    Checked { text, failed }
}

/// The sentence of `text` that tells the person what the team's work will
/// cost — an amount in money or credits beside a word of cost or estimate
/// ("about $3", "roughly 300 credits"), if any. The owner, 2026-11-28: no
/// estimates up front. A real purchase from another company (a domain) says
/// its real price, and a price of the person's own product is no estimate.
pub(super) fn quotes_cost(text: &str) -> Option<String> {
    let units = crate::i18n::every("guard.money_units");
    let costs = crate::i18n::every("guard.cost_words");
    let purchases = crate::i18n::every("guard.purchase_words");
    let ends = |c: char| matches!(c, '.' | '!' | '?' | '\n' | '。' | '！' | '？');
    for sentence in text.split(ends) {
        let lower = sentence.to_lowercase();
        let chars: Vec<char> = lower.chars().collect();
        let dollar = chars.windows(2).any(|w| w[0] == '$' && w[1].is_ascii_digit()) || chars.windows(3).any(|w| w[0] == '$' && w[1] == ' ' && w[2].is_ascii_digit());
        let in_units = units.iter().any(|u| {
            let u = u.to_lowercase();
            lower.match_indices(&u).any(|(at, _)| lower[..at].trim_end().chars().last().is_some_and(|c| c.is_ascii_digit()))
        });
        if (dollar || in_units) && costs.iter().any(|w| lower.contains(&w.to_lowercase())) && !purchases.iter().any(|w| lower.contains(&w.to_lowercase())) {
            return Some(sentence.trim().chars().take(240).collect());
        }
    }
    None
}

/// The promise in `text` to wait for the person's pick or yes before
/// applying something ("I'll apply it once you confirm"), if any: its sentence.
/// Only a promise about applying or putting live counts ("once you pick, it starts",
/// once you pick it starts, is no promise to hold anything).
pub(super) fn waits_for_yes(text: &str) -> Option<String> {
    let applies = crate::i18n::every("guard.apply_words");
    let ends = |c: char| matches!(c, '.' | '!' | '?' | '\n' | '。' | '！' | '？');
    text.split(ends).map(str::trim).find(|s| says(s, "guard.promise_confirm").is_some() && applies.iter().any(|w| s.to_lowercase().contains(&w.to_lowercase()))).map(str::to_owned)
}

/// What the PM is told of the tickets equipe held for the person's yes after its promise.
fn held_for_yes(said: &str, tickets: &[TicketId]) -> String {
    let list = tickets.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
    format!(
        "[equipe: you told the person: “{said}”. So {list}, made or changed in that turn, wait for their yes: they stay on their \
branch and preview, and nothing of them lands or goes live until you accept them with the person's words in `confirmed`. One that \
isn't what you promised to wait on: `update_ticket` with `wait_for_person: false`.]"
    )
}

/// The promise of later work in `text`, if any: its sentence.
pub(super) fn promise(text: &str) -> Option<String> {
    says(text, "guard.promise").map(|p| sentence_with(text, &p))
}

/// What the PM is asked when its reply was held back.
fn ask_again(failed: &[String], language: Option<Lang>) -> String {
    let lang = language.map_or(String::new(), |l| format!(", in {}", l.english_name()));
    format!(
        "[equipe: your last message was NOT shown to the person: {}. Write it to them again now{lang}, with the same substance and the facts \
right; say nothing about this note.]",
        failed.join("; and ")
    )
}

/// What the PM is told of a promise it made and didn't put on the board.
fn track_promise(said: &str) -> String {
    format!(
        "[equipe: you told the person: “{said}”. If that promises work for later, put it on the board now so it isn't lost: a line in the \
open ticket it belongs to (`update_ticket`), or a small `create_ticket` for its project that serves their request. Don't message the \
person about this; if it promised nothing, do nothing.]"
    )
}

impl Runtime {
    /// A turn of `id` starts. Whose turn it is — the person's or equipe's —
    /// decides whether its words reach the person. A message asked again
    /// keeps its first turn's say.
    pub(super) fn reply_turn_starts(&mut self, id: SessionId) {
        if !self.reply_guarded(id) || self.reply_guard.asked_again.contains(&id) {
            return;
        }
        let equipes = self.sessions.get(&id).is_some_and(|s| equipes_turn(s.transcript.entries()));
        if equipes {
            self.reply_guard.equipes.insert(id);
        } else {
            self.reply_guard.equipes.remove(&id);
        }
    }

    /// The PM's turn in `id` made or changed `ticket` on the board.
    pub(super) fn board_touched(&mut self, id: SessionId, ticket: TicketId) {
        if self.reply_guarded(id) {
            let held = self.reply_guard.held.entry(id).or_default();
            if !held.tickets.contains(&ticket) {
                held.tickets.push(ticket);
            }
        }
    }

    /// Whether `id` is the PM's own conversation with the person.
    fn reply_guarded(&self, id: SessionId) -> bool {
        self.sessions.get(&id).is_some_and(|s| s.is_pm() && s.work.is_none() && s.run_of.is_none())
    }

    /// The facts a reply in `squad` is checked against, from what is known now (nothing waits on the network).
    fn reply_facts(&mut self, id: SessionId, squad: &str) -> Facts {
        // The language of the person's latest message, their own words only; when it can't
        // tell (a few words, a tapped suggestion), the language the squad keeps for them. An
        // older message never decides: the language changes only when the person changes it.
        let said = self.sessions.get(&id).and_then(|s| {
            s.transcript.entries().iter().rev().find_map(|e| if let Entry::User { text, .. } = e { Some(text) } else { None }).and_then(|text| crate::language::detect(&crate::language::persons_words(text)))
        });
        // A squad that kept the platform's language (English notes read as the person's, before 0.103.194) heals here.
        if let Some(name) = said
            && let Some(journal) = self.journal(squad)
            && journal.language().as_deref() != Some(name)
        {
            let _ = journal.set_language(name);
        }
        let language = said.and_then(Lang::from_english_name).or_else(|| self.squad_language(squad));
        let mut waiting: Vec<String> = self.waiting_now(squad).iter().map(|w| w.title.chars().take(80).collect()).collect();
        let cloud = crate::sprint::platform::Platform::discover().and_then(|p| crate::sprint::platform::cloud_facts_cached(&p));
        let paused = cloud.as_ref().and_then(|c| c.paused);
        // A product never published has nothing live: "the live site already shows it" is false of it (Team Aman, 2026-11-28).
        let never_live: Vec<(String, String)> =
            cloud.as_ref().map(|c| c.never_live.iter().map(|app| (app.clone(), "it was never published — only its private preview address exists, so say \"on the preview\"".to_owned())).collect()).unwrap_or_default();
        waiting.extend(cloud.map(|c| c.waiting).unwrap_or_default());
        let mut not_live: Vec<(String, String)> = self
            .lanes(squad)
            .iter()
            .filter(|(_, l)| l.declared && (l.running || l.pending.is_some() || l.stuck.is_some()))
            .map(|(key, l)| {
                let how = if l.running || l.pending.is_some() { "its deploy is under way" } else { "its last deploy didn't go through" };
                (board::project_name(Path::new(key)), how.to_owned())
            })
            .collect();
        not_live.extend(never_live);
        let codes_hidden = crate::autonomy::sandbox_machine() || self.autonomy_mode(squad) == equipe_protocol::autonomy::Autonomy::Full;
        Facts { language, waiting, paused, not_live, codes_hidden }
    }

    /// The plain paragraphs told lately in `id`.
    fn told_lately(&mut self, id: SessionId) -> Vec<String> {
        let now = now_secs();
        let told = self.reply_guard.told.entry(id).or_default();
        told.retain(|(at, _)| now.saturating_sub(*at) < TOLD_FOR);
        told.iter().map(|(_, p)| p.clone()).collect()
    }

    fn keep_told(&mut self, id: SessionId, text: &str) {
        let now = now_secs();
        let told = self.reply_guard.told.entry(id).or_default();
        for para in text.split("\n\n").map(plain).filter(|p| p.chars().count() >= REPEAT_MIN) {
            told.push_back((now, para));
        }
        while told.len() > TOLD_KEPT {
            told.pop_front();
        }
    }

    /// One event on its way to the screens, through the PM's reply guard: the
    /// events to publish now, in order (none while text is held).
    pub(super) fn reply_guard_event(&mut self, event: Event) -> Vec<Event> {
        let Some(id) = event.session().filter(|id| self.reply_guarded(*id)) else { return vec![event] };
        match event {
            Event::TextDelta { text, .. } => {
                self.reply_guard.held.entry(id).or_default().text.push_str(&text);
                Vec::new()
            }
            Event::ToolUse { session, tool, target, mut label, background } => {
                let held = self.reply_guard.held.entry(id).or_default();
                if ["create_ticket", "update_ticket", "revise_ticket"].iter().any(|t| tool.ends_with(t)) {
                    held.board = true;
                }
                if puts_to_person(&tool) {
                    held.to_person = true;
                }
                let text = std::mem::take(&mut held.text);
                let tool_use = |label| Event::ToolUse { session, tool: tool.clone(), target: target.clone(), label, background };
                if text.trim().is_empty() {
                    return vec![tool_use(label)];
                }
                if narration(&text) {
                    // On the way to the work: the tool's line says it, in the work view.
                    if label.is_none() {
                        label = Some(text.trim().to_owned());
                    }
                    held.narrated = Some(text.trim().to_owned());
                    return vec![tool_use(label)];
                }
                held.narrated = None;
                // equipe's turn: whether anything reaches the person is known at its end.
                if self.reply_guard.equipes.contains(&id) {
                    if !held.kept.is_empty() {
                        held.kept.push_str("\n\n");
                    }
                    held.kept.push_str(text.trim());
                    return vec![tool_use(label)];
                }
                let squad = self.sessions.get(&id).and_then(|s| s.squad.clone()).unwrap_or_default();
                let (told, facts) = (self.told_lately(id), self.reply_facts(id, &squad));
                let checked = check(&text, &told, &facts);
                if !checked.failed.is_empty() && !self.reply_guard.asked_again.contains(&id) {
                    crate::daemon_log!("equipe: {squad}: the PM's message held back mid-turn: {}", checked.failed.join("; "));
                    self.reply_guard.held.entry(id).or_default().failed.extend(checked.failed);
                    return vec![tool_use(label)];
                }
                self.keep_told(id, &checked.text);
                let mut out = Vec::new();
                if !checked.text.is_empty() {
                    out.push(Event::TextDelta { session, text: checked.text });
                }
                out.push(tool_use(label));
                out
            }
            Event::TurnEnded { session, outcome, usage } => {
                let held = self.reply_guard.held.remove(&id).unwrap_or_default();
                let ended = Event::TurnEnded { session, outcome, usage };
                let squad = self.sessions.get(&id).and_then(|s| s.squad.clone()).unwrap_or_default();
                // equipe's turn that put nothing to the person: what it said was for the team, not them
                // ("I'm leaving this wake alone", "nothing to plan yet") — kept off the chat.
                if self.reply_guard.equipes.contains(&id) && !held.to_person {
                    self.reply_guard.asked_again.remove(&id);
                    let said = format!("{}{}", held.kept, held.text);
                    if !said.trim().is_empty() {
                        crate::daemon_log!("equipe: {squad}: the PM's reply to equipe's own note kept from the person ({} chars)", said.trim().chars().count());
                    }
                    return vec![ended];
                }
                // A turn that failed or was stopped: what it said goes out as it was.
                if outcome != TurnOutcome::Completed {
                    let mut out = Vec::new();
                    let text = if held.kept.is_empty() { held.text } else { format!("{}\n\n{}", held.kept, held.text) };
                    if !text.trim().is_empty() {
                        out.push(Event::TextDelta { session, text });
                    }
                    out.push(ended);
                    return out;
                }
                let again = self.reply_guard.asked_again.remove(&id);
                let (told, facts) = (self.told_lately(id), self.reply_facts(id, &squad));
                // Nothing said after its last short line: that line was its message — in the person's language only
                // (narration in the engine's own English stays off the chat).
                let mut text = match (held.kept.is_empty(), held.text.trim().is_empty()) {
                    (true, _) => held.text,
                    (false, true) => held.kept,
                    (false, false) => format!("{}\n\n{}", held.kept, held.text),
                };
                if text.trim().is_empty()
                    && let Some(line) = held.narrated
                    && facts.language.is_none_or(|want| !wrong_language(&line, want))
                {
                    text = line;
                }
                let checked = check(&text, &told, &facts);
                let mut failed = held.failed;
                failed.extend(checked.failed.iter().cloned());
                if !failed.is_empty() && !again {
                    crate::daemon_log!("equipe: {squad}: the PM's reply held back and asked again: {}", failed.join("; "));
                    self.reply_guard.asked_again.insert(id);
                    // What this turn put on the board, and to the person, goes with the message written again.
                    if !held.tickets.is_empty() || held.to_person {
                        self.reply_guard.held.insert(id, Held { tickets: held.tickets, to_person: held.to_person, ..Held::default() });
                    }
                    let _ = self.internal.send(Internal::Wake { session: id, prompt: ask_again(&failed, facts.language) });
                    return vec![ended];
                }
                if checked.text.len() < text.trim().len() {
                    crate::daemon_log!("equipe: {squad}: a repeated paragraph left out of the PM's reply");
                }
                // Asked again and still naming work codes: they come out here.
                let mut checked = checked;
                if again && facts.codes_hidden && !work_codes(&checked.text).is_empty() {
                    checked.text = without_work_codes(&checked.text);
                }
                self.keep_told(id, &checked.text);
                // Promised to wait for their pick or yes: what the turn put on the board waits for it too.
                if !held.tickets.is_empty()
                    && let Some(said) = waits_for_yes(&checked.text)
                {
                    let mut kept = Vec::new();
                    for ticket in &held.tickets {
                        if self.hold_for_person(&squad, "equipe", *ticket, true).is_ok() {
                            kept.push(*ticket);
                        }
                    }
                    if !kept.is_empty() {
                        crate::daemon_log!("equipe: {squad}: the PM promised to wait for the person's yes: {kept:?} held until they say it");
                        self.brief_pm(&squad, held_for_yes(&said, &kept), None);
                    }
                }
                if !held.board
                    && let Some(said) = promise(&checked.text)
                {
                    crate::daemon_log!("equipe: {squad}: the PM promised later work with nothing on the board; it is asked to put it there");
                    let _ = self.internal.send(Internal::Wake { session: id, prompt: track_promise(&said) });
                }
                let mut out = Vec::new();
                if !checked.text.is_empty() {
                    out.push(Event::TextDelta { session, text: checked.text });
                }
                out.push(ended);
                out
            }
            // The person speaking into the turn makes it theirs: its answer is for them.
            Event::Steered { .. } | Event::UserMessage { .. } => {
                self.reply_guard.equipes.remove(&id);
                vec![event]
            }
            Event::SessionClosed { session } => {
                self.reply_guard.told.remove(&id);
                self.reply_guard.asked_again.remove(&id);
                self.reply_guard.equipes.remove(&id);
                let held = self.reply_guard.held.remove(&id).unwrap_or_default();
                let mut out = Vec::new();
                if !held.text.trim().is_empty() {
                    out.push(Event::TextDelta { session, text: held.text });
                }
                out.push(Event::SessionClosed { session });
                out
            }
            other => vec![other],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts { language: Some(Lang::Ko), ..Facts::default() }
    }

    /// What customer teams were shown, from the health sweep of 2026-11-28 (squad_issues).
    #[test]
    fn the_teams_work_codes_are_caught_and_taken_out_when_asked_again() {
        let seen = [
            ("고뢰자 님, 다음 스프린트가 이루려는 숫자를 정했어요. 그래서 S-1을 시작하면 전화하기와 문의가 세어져요.", vec!["S-1"]),
            ("The S-2 target is a known loop and was already set four times. Per my note, I'm leaving this wake alone.", vec!["S-2"]),
            ("**Weekly retro** 1. **Shipped:** T-12 Pricing page and T-13 Login (B-43 checked).", vec!["T-12", "T-13", "B-43"]),
        ];
        let hidden = Facts { codes_hidden: true, ..Facts::default() };
        for (text, codes) in seen {
            assert_eq!(work_codes(text), codes, "{text}");
            let checked = check(text, &[], &hidden);
            assert!(checked.failed.iter().any(|f| f.contains("work codes")), "{text}");
            // A developer at their own board talks in its codes.
            assert!(check(text, &[], &Facts::default()).failed.iter().all(|f| !f.contains("work codes")), "{text}");
        }
        // Not codes: a test plan's case, a T-shirt, a date, a model name, a link or a path.
        for text in ["TP-3 passes on phone width.", "The T-shirt page is live.", "COVID-19 rules", "GPT-4 and S-Class", "See https://x.equipe.page/T-12 and `outputs/T-17/report.md`."] {
            assert!(work_codes(text).is_empty(), "{text}");
        }
        assert_eq!(without_work_codes("T-12 Pricing page is live (T-13)."), "Pricing page is live.");
        assert_eq!(without_work_codes("Shipped: T-12."), "Shipped:.");
        assert_eq!(without_work_codes("가격 페이지(T-12)가 올라갔어요."), "가격 페이지가 올라갔어요.");
    }

    #[test]
    fn a_roles_english_name_is_said_in_the_persons_script_and_left_in_a_latin_language() {
        // ironact, 2026-11-27.
        let said = "Designer가 막 회사 조사를 시작했고, 아직 끝난 단계는 없어요. Frontend engineer가 이어서 만들어요.";
        assert_eq!(with_roles_in(said, Lang::Ko), "디자이너가 막 회사 조사를 시작했고, 아직 끝난 단계는 없어요. 프론트엔드 엔지니어가 이어서 만들어요.");
        assert_eq!(with_roles_in("designerの調査が終わりました。", Lang::Ja), "デザイナーの調査が終わりました。");
        // Never in code, a link's address or a path; never part of another word.
        let kept = "`designer` 역할은 [Designer 안내](https://x.com/designer)에, docs/designer/notes.md에 있어요. Designers와 redesigner도.";
        assert_eq!(with_roles_in(kept, Lang::Ko), "`designer` 역할은 [디자이너 안내](https://x.com/designer)에, docs/designer/notes.md에 있어요. Designers와 redesigner도.");
        // ai-based-0k09, 2026-11-27: French to an English founder is held and asked again in English.
        let french = Facts { language: Some(Lang::En), ..Facts::default() };
        let checked = check("La première version de CasePrep AI est en ligne : la page d'accueil présente les trois parcours.", &[], &french);
        assert!(checked.failed.iter().any(|f| f.contains("English")), "{:?}", checked.failed);
        assert_eq!(crate::language::detect("Please set up our brand kit: logo files, colours, type and voice."), Some("English"));
        // In French the word is French too.
        assert_eq!(with_roles_in("Le designer commencera le kit de marque.", Lang::Fr), "Le designer commencera le kit de marque.");
        // The check gives the reply in the person's words.
        let checked = check("Designer가 로고를 다시 그렸어요. 휴대폰에서도 잘 보여요.", &[], &facts());
        assert_eq!(checked.text, "디자이너가 로고를 다시 그렸어요. 휴대폰에서도 잘 보여요.");
        assert!(checked.failed.is_empty(), "{:?}", checked.failed);
    }

    #[test]
    fn a_turn_equipe_started_is_told_only_when_it_puts_something_to_the_person() {
        let user = |t: &str| Entry::User { text: t.into(), kept: Vec::new() };
        let system = |t: &str| Entry::System { text: t.into() };
        let reply = |t: &str| Entry::Reply { text: t.into() };
        // A sprint's planning, a monitor's or a wake's note after the PM's last reply: equipe's.
        assert!(equipes_turn(&[reply("done"), system("[Sprint planning · week 2] Plan the next sprint")]));
        assert!(equipes_turn(&[system("[Idle · 24h]\n\nThe board is empty")]));
        assert!(equipes_turn(&[user("make it blue"), reply("done"), system("[equipe self-watch] S-2 has been set four times")]));
        // A note whose answer was kept from the person left no reply: the next turn is still equipe's.
        assert!(equipes_turn(&[reply("done"), system("[equipe self-watch] a"), system("[Waiting on the person · 3h+] b")]));
        // The person's words since the last reply, their own routine (the weekly recap), or a correction
        // equipe asks of a reply to them (no note): theirs.
        assert!(!equipes_turn(&[reply("done"), system("[equipe self-watch] a"), user("and the logo?")]));
        assert!(!equipes_turn(&[reply("done"), system("[Routine · This week]\n\nWeekly recap")]));
        assert!(!equipes_turn(&[user("Can I see the logo?"), reply("Here it is: ![Logo](/gone.png)")]));
        assert!(!equipes_turn(&[user("hi")]));
        // What reaches the person from such a turn: a card, a question, a sign-in, a word.
        for tool in ["mcp__equipe__ask_decision", "mcp__equipe__block_on_person", "mcp__equipe__propose_goal", "mcp__equipe__plan_sprint", "mcp__equipe-platform__connect_domain", "sign_in"] {
            assert!(puts_to_person(tool), "{tool}");
        }
        for tool in ["mcp__equipe__create_ticket", "mcp__equipe__remember", "mcp__equipe__board", "Bash", "Read"] {
            assert!(!puts_to_person(tool), "{tool}");
        }
    }

    #[test]
    fn short_lines_before_a_tool_call_are_narration_and_messages_are_not() {
        for line in ["Render a side-by-side sheet.", "Now check live pages with screenshots.", "Good. Now Korean on phone."] {
            assert!(narration(line), "{line}");
        }
        assert!(!narration("새 로고가 들어갔어요.\n\n영어권 페이지에는 OnePass가 보여요."));
        assert!(!narration(&"a".repeat(200)));
        assert!(!narration("choices: A | B"));
    }

    #[test]
    fn an_english_reply_to_a_korean_founder_is_caught_and_a_korean_one_passes() {
        let english = "The faster OnePass is still on its way to the real address, and I'll send you the link once its phone speed is confirmed there.";
        assert!(wrong_language(english, Lang::Ko));
        let korean = "빨라진 원패스가 실제 주소에 올라갔어요. 휴대폰 속도 점수는 71에서 98로 올랐어요: https://o1path-equipe-tj9v.equipe.page";
        assert!(!wrong_language(korean, Lang::Ko));
        // An English reply naming a Korean product is still English.
        assert!(wrong_language("I've sent 원패스's plan for this week, and the six languages are live on the site now.", Lang::Ko));
        assert!(!wrong_language("ok 👍", Lang::Ko), "too short to say");
        assert!(wrong_language("새 로고가 원패스 실제 주소에 들어갔어요. 영어권 페이지 맨 위에는 OnePass가 보여요.", Lang::En));
        assert!(!wrong_language("The new logo is live on the site, in light and dark mode, at phone and desktop width.", Lang::En));
    }

    #[test]
    fn one_english_line_in_a_korean_reply_is_caught_and_names_are_not() {
        // inhwadoen, 2026-11-28: the reply opened on an English line, the rest Korean.
        let mixed = "I made one picture-saving mistake and two new requests have come in since. Here's where things stand.\n\n\
예린님, 아까 안 보였던 A안 첫 화면 그림이에요.\n\n말씀하신 글꼴과 카메라 그림은 디자이너가 다시 고르고 있어요. 글꼴은 장식적인 타자기체 대신 고딕 계열로 바꿀 거예요.";
        assert!(wrong_language(mixed, Lang::Ko));
        assert!(wrong_language("녹화 파일을 줄였어요.\nI made the recording smaller so it plays more easily on your phone.", Lang::Ko));
        // Before: the whole message's share of Hangul alone let it through.
        assert!(!whole_in_another(&prose(mixed), Lang::Ko));
        // A quote of someone's English words, or a link's title, is not the reply's language.
        assert!(!wrong_language("민성 님이 저장소에 협업자로 넣어 달라고 하셨어요. 넣어 드릴지 정해 주시면 바로 처리할게요. 보내신 말은 이랬어요.\n> \"Can you add me as a collaborator on the repository? I'll push our feature on a branch.\"", Lang::Ko));
        assert!(!wrong_language("컴퓨터 제어 기능을 켜려면 아래 안내를 따라 설치해 주세요. 설치가 끝나면 바로 이어서 진행할게요.\n- [How to install Claude Desktop computer use on your Mac](https://example.com/a)", Lang::Ko));
        // Names, fonts and links in a Korean reply are no English sentence.
        let names = "글꼴은 이렇게 바꿨어요.\n- Pretendard Variable, Noto Sans KR, Gothic A1, D2Coding\n- 미리보기: https://jungkyung-lab.equipe.page/develop?look=a";
        assert!(!wrong_language(names, Lang::Ko));
        assert!(!wrong_language("A안을 그대로 두고 카메라 그림만 세 가지로 다시 그렸어요. 본문은 SUIT, 제목과 숫자는 D2Coding이에요.", Lang::Ko));
        // An English reply with one Korean line (a quote of the person) is caught for an English founder only when it is a sentence.
        assert!(wrong_language("The new hero is live.\n새 첫 화면이 실제 주소에 올라갔고 휴대폰에서도 잘 보여요.", Lang::En));
    }

    #[test]
    fn a_claim_the_state_contradicts_holds_the_reply_with_the_facts() {
        // A product never published: "the live site already shows it" is false (Team Aman's asap, 2026-11-28).
        let mut never = facts();
        never.not_live = vec![("asap".into(), "it was never published — only its private preview address exists".into())];
        let c = check("The live site already does that: it shows asap everywhere.", &[], &never);
        assert!(c.failed.iter().any(|f| f.contains("never published")), "{c:?}");
        assert!(check("It's renamed to asap now.", &[], &never).failed.iter().any(|f| f.contains("never published")), "a rename claim is checked too");
        assert!(check("미리보기에서 asap으로 보여요. 공개는 말씀하시면 할게요.", &[], &never).failed.is_empty(), "the preview, said so, is true");
        let mut f = facts();
        f.waiting = vec!["confirming the goal for o1path (Goal page)".into(), "this week's plan for o1path: 3 idea(s) to say go to (Goal page)".into()];
        let c = check("이번 주 원패스 계획을 보냈어요. 지금 기다리는 건 없어요.", &[], &f);
        assert_eq!(c.failed.len(), 1, "{c:?}");
        assert!(c.failed[0].contains("confirming the goal"), "{c:?}");
        // Nothing waits: the same words are true.
        assert!(check("이번 주 원패스 계획을 보냈어요. 지금 기다리는 건 없어요.", &[], &facts()).failed.is_empty());
        let mut f = facts();
        f.paused = Some(false);
        assert_eq!(check("크레딧이 떨어져서 디자이너 작업이 멈췄어요.", &[], &f).failed.len(), 1);
        f.paused = Some(true);
        assert!(check("크레딧이 떨어져서 디자이너 작업이 멈췄어요.", &[], &f).failed.is_empty(), "true while paused");
        let mut f = Facts { language: Some(Lang::En), ..Facts::default() };
        f.not_live = vec![("product".into(), "its deploy is under way".into())];
        assert_eq!(check("The six-language OnePass is live now at its real address.", &[], &f).failed.len(), 1);
    }

    #[test]
    fn a_cost_estimate_is_held_and_a_purchase_or_a_product_price_is_not() {
        for said in ["시안 세 개를 만들면 약 $3 정도 들어요. 진행할까요?", "이 작업은 300 크레딧 정도 예상돼요.", "It would cost about $1.50; shall I go ahead?", "That's roughly 250 credits of work."] {
            assert!(quotes_cost(said).is_some(), "{said}");
            assert_eq!(check(said, &[], &Facts::default()).failed.len(), 1, "{said}");
        }
        for said in [
            "jungkyung-lab.com 도메인은 1년에 $12.99예요. 살까요?",
            "Apple Developer 멤버십 연 $99 하나로 직접 배포와 스토어 둘 다 됩니다.",
            "한 명이 숨질 때 사회적 비용은 미국 교통부 기준으로 약 $13M이에요.",
            "2025년 데이터로 계산하면 한 장에 평균 약 $101이에요.",
            "The domain shutter.page is $14 a year from the registrar.",
            "프로 요금제를 월 $29로 가격표에 넣었어요.",
            "I put the Pro plan at $29/month on your pricing page.",
            "크레딧이 충분해서 작업은 그대로 진행돼요.",
        ] {
            assert!(quotes_cost(said).is_none(), "{said}");
        }
    }

    #[test]
    fn only_a_promise_to_apply_after_their_yes_holds_work() {
        for said in ["미리보기 링크와 폰·컴퓨터 화면 사진부터 보여드리고, 확인해 주시면 그때 반영할게요.", "I'll show you the preview first and apply it once you confirm.", "고르신 안으로 사이트에 적용할게요. 고르시면 바로 적용해요."] {
            assert!(waits_for_yes(said).is_some(), "{said}");
        }
        for said in ["고르시면 T-527 디자인 시스템 작업이 시작돼요.", "카드에서 고르시면 그대로 처리하겠습니다.", "확인하시면 보내요.", "Pick one on the card and the work starts."] {
            assert!(waits_for_yes(said).is_none(), "{said}");
        }
        let asked = held_for_yes("확인해 주시면 그때 반영할게요.", &[36]);
        assert!(asked.contains("T-36") && asked.contains("`confirmed`") && asked.contains("wait_for_person: false"), "{asked}");
    }

    #[test]
    fn a_paragraph_said_twice_is_told_once() {
        let answer = "원패스 로고는 산 고갯길 모양으로 정했어요. 두 봉우리 사이로 길 하나가 올라가는 모양이고, 영어 페이지에는 OnePass, 한국어 페이지에는 원패스로 보여요.";
        let twice = format!("{answer}\n\n{answer}");
        assert_eq!(check(&twice, &[], &facts()).text, answer);
        // Said a moment ago in an earlier reply: left out; the new part stays.
        let told: Vec<String> = vec![plain(answer)];
        let again = format!("{answer}\n\n로고는 사이트 맨 위에 들어갔어요. 다크 모드에서도 잘 보이게 따로 만들었고, 휴대폰에서도 한 줄로 보여요.");
        let c = check(&again, &told, &facts());
        assert!(!c.text.contains("산 고갯길") && c.text.contains("맨 위에"), "{c:?}");
        // Nearly the same words (a changed ending) are still a repeat.
        let near = answer.replace("보여요.", "보입니다.");
        assert_eq!(check(&near, &told, &facts()).text, "");
        // Two pictures side by side are two paragraphs, never one repeated.
        let pics = "영어 페이지 로고예요.\n\n![OnePass 새 로고](/home/x/o1path/design/assets/logo/png/lockup.png)\n\n![원패스 새 로고](/home/x/o1path/design/assets/logo/png/lockup-ko.png)";
        assert_eq!(check(pics, &[], &facts()).text, pics);
        // Short lines repeat freely ("네, 좋아요").
        assert_eq!(check("네, 좋아요.\n\n네, 좋아요.", &[], &facts()).text, "네, 좋아요.\n\n네, 좋아요.");
    }

    #[test]
    fn a_promise_of_later_work_is_found_with_its_sentence() {
        let said = "데스크톱과 휴대폰에서 확인했어요. 휴대폰에서 글자를 2배로 키우면 소개 문구의 끝 글자가 조금 잘려요. 다음 원패스 수정 때 함께 고칠게요.";
        assert_eq!(promise(said).as_deref(), Some("다음 원패스 수정 때 함께 고칠게요."));
        assert!(promise("I'll fix the clipped word in the next update.").is_some());
        assert!(promise("The new logo is live on the site.").is_none());
    }

    #[test]
    fn an_english_reply_to_an_english_founder_is_never_held_for_french() {
        // ai-based-0k09, 2026-11-27: the guard read the founder's English request as French and held the PM's
        // English reply until it was rewritten in French.
        let asked = crate::language::detect(&crate::language::persons_words("[@akki · akki shafiqullah · owner] Please set up our brand kit: logo files, colours, type and voice."));
        let want = asked.and_then(Lang::from_english_name);
        assert_eq!(want, Some(Lang::En));
        let reply = "Akki, the designer will start on the brand kit as soon as the first version of the site is set up. The site's first build creates the logo options, and the brand work picks up from there.";
        assert!(check(reply, &[], &Facts { language: want, ..Facts::default() }).failed.is_empty());
        // An English reply is not "not French" on weak evidence either: a French founder's squad holds it, an English one never.
        assert!(wrong_language(reply, Lang::Fr));
        assert!(!wrong_language(reply, Lang::En));
    }

    #[test]
    fn what_the_pm_is_asked_names_every_reason_and_the_language() {
        let asked = ask_again(&["it isn't in Korean".into(), "it says nothing waits".into()], Some(Lang::Ko));
        assert!(asked.contains("NOT shown") && asked.contains("in Korean") && asked.contains("; and "), "{asked}");
    }
}

/// Recorded PM turns replayed through the guard: `EQUIPE_REPLY_REPLAY=<file>
/// cargo test -p equipe-core --lib replay_recorded -- --ignored --nocapture`.
/// The file: `[{at, items: [{reply} | {tool}]}]`, a conversation's turns in order.
#[cfg(test)]
mod replay {
    use super::*;

    #[test]
    #[ignore = "reads a recorded conversation named by EQUIPE_REPLY_REPLAY"]
    fn replay_recorded_pm_turns() {
        let Ok(file) = std::env::var("EQUIPE_REPLY_REPLAY") else { return };
        let turns: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        let mut facts = Facts { language: Some(Lang::Ko), ..Facts::default() };
        let mut told: Vec<String> = Vec::new();
        let (mut shown, mut narration_dropped, mut held, mut repeats, mut promises) = (0, 0, 0, 0, 0);
        for turn in &turns {
            let at = turn["at"].as_u64().unwrap_or(0);
            // o1path's credits: back at 17:47 UTC on 2026-11-26 (paused before, for a while).
            facts.paused = (at >= 1_795_715_257).then_some(false);
            let mut text = String::new();
            let mut board = false;
            let mut narrated: Option<String> = None;
            let mut out: Vec<String> = Vec::new();
            let items = turn["items"].as_array().cloned().unwrap_or_default();
            for item in &items {
                if let Some(tool) = item["tool"].as_str() {
                    // What the platform knew: the PM's proposals wait on the person once made.
                    if tool.ends_with("propose_goal") && facts.waiting.iter().all(|w| !w.contains("goal")) {
                        facts.waiting.push("confirming the goal for o1path (Goal page)".into());
                    }
                    if tool.ends_with("propose_week") && facts.waiting.iter().all(|w| !w.contains("plan")) {
                        facts.waiting.push("this week's plan for o1path: ideas to say go to (Goal page)".into());
                    }
                    board |= ["create_ticket", "update_ticket", "revise_ticket"].iter().any(|t| tool.ends_with(t));
                    if !text.trim().is_empty() {
                        narrated = None;
                        if narration(&text) {
                            narration_dropped += 1;
                            narrated = Some(text.trim().to_owned());
                            out.push(format!("  NARRATION → tool line: {:?}", text.trim()));
                        } else {
                            out.push(format!("  message mid-turn: {:?}", text.chars().take(60).collect::<String>()));
                            shown += 1;
                        }
                        text.clear();
                    }
                } else if let Some(reply) = item["reply"].as_str() {
                    if !text.is_empty() {
                        text.push_str("\n\n");
                    }
                    text.push_str(reply);
                }
            }
            if text.trim().is_empty()
                && let Some(line) = narrated.take().filter(|l| !wrong_language(l, Lang::Ko))
            {
                narration_dropped -= 1;
                out.push(format!("  (its last short line was its message: {line:?})"));
                text = line;
            }
            if text.trim().is_empty() {
                for line in out {
                    println!("{at} {line}");
                }
                continue;
            }
            let checked = check(&text, &told, &facts);
            if checked.text.len() < text.trim().len() {
                repeats += 1;
                out.push(format!("  REPEAT left out of: {:?}", text.chars().take(200).collect::<String>()));
            }
            if !checked.failed.is_empty() {
                held += 1;
                out.push(format!("  HELD, PM asked again: {} :: {:?}", checked.failed.join(" | "), text.chars().take(80).collect::<String>()));
            } else {
                shown += 1;
                told.extend(checked.text.split("\n\n").map(plain).filter(|p| p.chars().count() >= REPEAT_MIN));
                if !board && let Some(p) = promise(&checked.text) {
                    promises += 1;
                    out.push(format!("  PROMISE → PM puts it on the board: {p:?}"));
                }
                out.push(format!("  shown: {:?}", checked.text.chars().take(60).collect::<String>()));
            }
            for line in out {
                println!("{at} {line}");
            }
        }
        println!("\nturns {} · replies shown {shown} · narration kept off the chat {narration_dropped} · held and asked again {held} · repeated paragraphs dropped {repeats} · promises put on the board {promises}", turns.len());
    }
}

/// Real PM replies through the checks, before (the whole message's script
/// share alone) and after (each line too, cost estimates, promises to wait):
/// `EQUIPE_GUARD_CORPUS=<file> cargo test -p equipe-core --lib guard_corpus -- --ignored --nocapture`.
/// The file: `[{squad, lang, text}]`, `lang` the squad's language by its English name.
#[cfg(test)]
mod corpus {
    use super::*;

    #[test]
    #[ignore = "reads recorded PM replies named by EQUIPE_GUARD_CORPUS"]
    fn guard_corpus() {
        let Ok(file) = std::env::var("EQUIPE_GUARD_CORPUS") else { return };
        let replies: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        let (mut before, mut after, mut cost, mut wait) = (0, 0, 0, 0);
        for r in &replies {
            let (Some(text), Some(lang)) = (r["text"].as_str(), r["lang"].as_str().and_then(Lang::from_english_name)) else { continue };
            let old = whole_in_another(&prose(text), lang);
            let new = wrong_language(text, lang);
            before += usize::from(old);
            after += usize::from(new);
            if new && !old {
                let line = prose(text).lines().find(|l| line_in_another(l, lang)).unwrap_or_default().to_owned();
                println!("NEW-LANG [{}] {:?}", r["squad"], line.chars().take(160).collect::<String>());
            }
            if let Some(said) = quotes_cost(text) {
                cost += 1;
                println!("COST [{}] {said:?}", r["squad"]);
            }
            if let Some(said) = waits_for_yes(text) {
                wait += 1;
                println!("WAIT [{}] {said:?}", r["squad"]);
            }
        }
        println!("replies {} · wrong language before {before}, after {after} · cost estimates {cost} · promises to wait {wait}", replies.len());
    }
}
