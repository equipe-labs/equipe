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
    /// The last short line taken for narration: the turn's message after all
    /// when nothing follows it ("Starting on it now." and then a ticket).
    narrated: Option<String>,
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

/// Whether `text` is written in another language than `want`, as far as plain code can tell.
pub(super) fn wrong_language(text: &str, want: Lang) -> bool {
    let text = prose(text);
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
    Checked { text, failed }
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
        waiting.extend(cloud.map(|c| c.waiting).unwrap_or_default());
        let not_live = self
            .lanes(squad)
            .iter()
            .filter(|(_, l)| l.declared && (l.running || l.pending.is_some() || l.stuck.is_some()))
            .map(|(key, l)| {
                let how = if l.running || l.pending.is_some() { "its deploy is under way" } else { "its last deploy didn't go through" };
                (board::project_name(Path::new(key)), how.to_owned())
            })
            .collect();
        Facts { language, waiting, paused, not_live }
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
                // A turn that failed or was stopped: what it said goes out as it was.
                if outcome != TurnOutcome::Completed {
                    let mut out = Vec::new();
                    if !held.text.trim().is_empty() {
                        out.push(Event::TextDelta { session, text: held.text });
                    }
                    out.push(ended);
                    return out;
                }
                let again = self.reply_guard.asked_again.remove(&id);
                let (told, facts) = (self.told_lately(id), self.reply_facts(id, &squad));
                // Nothing said after its last short line: that line was its message — in the person's language only
                // (narration in the engine's own English stays off the chat).
                let mut text = held.text;
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
                    let _ = self.internal.send(Internal::Wake { session: id, prompt: ask_again(&failed, facts.language) });
                    return vec![ended];
                }
                if checked.text.len() < text.trim().len() {
                    crate::daemon_log!("equipe: {squad}: a repeated paragraph left out of the PM's reply");
                }
                self.keep_told(id, &checked.text);
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
            Event::SessionClosed { session } => {
                self.reply_guard.told.remove(&id);
                self.reply_guard.asked_again.remove(&id);
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
    fn a_claim_the_state_contradicts_holds_the_reply_with_the_facts() {
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
