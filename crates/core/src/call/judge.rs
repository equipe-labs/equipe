//! The call's judge: who an utterance is for. A fast model first (2.5 s),
//! then word rules, then the PM — it never fails.

use std::time::Duration;

use equipe_protocol::SessionId;
use equipe_protocol::call::{CallRoute, Dispatched, RouteBy};

use super::Turn;

/// A member the judge may route to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterMember {
    pub session: SessionId,
    /// `designer`, `backend-2`…
    pub name: String,
    /// `designer`, `backend`, `pm`…
    pub role: String,
    /// Titles of the tickets it is on.
    pub tickets: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Instruct,
    Question,
    End,
    Unclear,
}

/// What the judge reads: the call so far (bounded: 8 turns + summary) and the squad.
#[derive(Debug, Clone)]
pub struct JudgeInput {
    pub summary: String,
    pub turns: Vec<Turn>,
    pub dispatched: Vec<Dispatched>,
    pub pending: Option<String>,
    pub utterance: String,
    pub roster: Vec<RosterMember>,
}

/// The judge's answer, routes resolved to the roster.
#[derive(Debug, Clone, PartialEq)]
pub struct Judged {
    pub intent: Intent,
    pub routes: Vec<CallRoute>,
    pub reply: Option<String>,
    pub by: RouteBy,
    /// How long judging took.
    pub millis: u64,
    /// Bytes of the prompt the model was given (0 without the model): stays flat however long the call.
    pub input_bytes: usize,
}

/// Where and how the model is asked.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// `None`: no model, straight to the rules.
    pub key: Option<String>,
    pub model: String,
    pub timeout: Duration,
    /// `https://api.anthropic.com`
    pub base: String,
}

pub const MODEL: &str = "claude-haiku-5-5";
pub const TIMEOUT: Duration = Duration::from_millis(2500);

use equipe_protocol::call::{CallTarget, TargetKind};

use super::Who;

const DEFAULT_BASE: &str = "https://api.anthropic.com";
const QUESTION_REPLY: &str = "I'll ask the PM.";
const FALLBACK_REPLY: &str = "I'll pass that to the PM. Anything else?";

impl LlmConfig {
    /// `ANTHROPIC_API_KEY`, `EQUIPE_CALL_MODEL`, `EQUIPE_CALL_LLM_BASE`.
    pub fn from_env() -> Self {
        let var = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        LlmConfig {
            key: var("ANTHROPIC_API_KEY"),
            model: var("EQUIPE_CALL_MODEL").unwrap_or_else(|| MODEL.to_string()),
            timeout: TIMEOUT,
            base: var("EQUIPE_CALL_LLM_BASE").unwrap_or_else(|| DEFAULT_BASE.to_string()),
        }
    }

    /// No model: straight to the rules.
    pub fn off() -> Self {
        LlmConfig {
            key: None,
            model: MODEL.to_string(),
            timeout: TIMEOUT,
            base: DEFAULT_BASE.to_string(),
        }
    }
}

fn by_str(by: RouteBy) -> &'static str {
    match by {
        RouteBy::Llm => "llm",
        RouteBy::Rules => "rules",
        RouteBy::Fallback => "fallback",
    }
}

fn intent_str(intent: Intent) -> &'static str {
    match intent {
        Intent::Instruct => "instruct",
        Intent::Question => "question",
        Intent::End => "end",
        Intent::Unclear => "unclear",
    }
}

/// Who an utterance is for. Never fails: the model, then the rules, then the PM.
pub async fn judge(input: &JudgeInput, llm: &LlmConfig) -> Judged {
    let started = std::time::Instant::now();
    let (system, user) = prompt(input);
    let mut input_bytes = 0;

    let mut judged = None;
    if let Some(key) = &llm.key {
        input_bytes = system.len() + user.len();
        match tokio::time::timeout(llm.timeout, ask_model(llm, key, &system, &user)).await {
            Ok(Ok(text)) => match parse(&text, &input.roster) {
                Ok((intent, routes, reply)) => {
                    judged = Some(match intent {
                        Intent::Question => Judged {
                            intent,
                            routes: vec![CallRoute {
                                to: pm_target(&input.roster),
                                text: input.utterance.clone(),
                            }],
                            reply: Some(QUESTION_REPLY.to_string()),
                            by: RouteBy::Llm,
                            millis: 0,
                            input_bytes,
                        },
                        _ => Judged {
                            intent,
                            routes,
                            reply,
                            by: RouteBy::Llm,
                            millis: 0,
                            input_bytes,
                        },
                    });
                }
                Err(e) => eprintln!("call judge: model answer unusable: {e}"),
            },
            Ok(Err(e)) => eprintln!("call judge: model failed: {e}"),
            Err(_) => eprintln!("call judge: model timed out after {:?}", llm.timeout),
        }
    }

    let mut judged = judged
        .or_else(|| rules(&input.utterance, &input.roster))
        .unwrap_or_else(|| fallback(&input.utterance, &input.roster));
    judged.input_bytes = input_bytes;
    judged.millis = started.elapsed().as_millis() as u64;
    eprintln!(
        "call judge: by={} intent={} ms={} input_bytes={} turns={} summary_bytes={}",
        by_str(judged.by),
        intent_str(judged.intent),
        judged.millis,
        judged.input_bytes,
        input.turns.len(),
        input.summary.len()
    );
    judged
}

fn target_label(to: &CallTarget) -> String {
    match to.kind {
        TargetKind::Squad => "everyone".to_string(),
        TargetKind::Pm => to.name.clone().unwrap_or_else(|| "pm".to_string()),
        TargetKind::Member => to
            .name
            .clone()
            .or_else(|| to.role.clone())
            .unwrap_or_else(|| "?".to_string()),
    }
}

/// The system prompt and the user message.
fn prompt(input: &JudgeInput) -> (String, String) {
    let system = "You route what a person says on a voice call with their software squad. \
Decide who each part of the latest utterance is for, using the squad roster. \
Answer with JSON only, no other text, in exactly this shape:\n\
{\"intent\":\"instruct\"|\"question\"|\"end\"|\"unclear\",\"routes\":[{\"to\":\"<member name>|everyone|pm\",\"text\":\"<the part of the utterance for them, rephrased as an instruction to them>\"}],\"reply\":\"<short spoken reply>\"}\n\
Rules: intent \"end\" when they are done talking (e.g. \"that's all\", \"bye\"), with no routes. \
\"instruct\" when they give work; every instruct has at least one route. \
Sentences that give context but name no one belong in the text of each route they explain. \
A role the roster does not have goes to \"pm\". If no one is named, send it to \"pm\". \
\"question\" when they ask something. \"unclear\" only if you cannot tell at all; then reply with one short question. \
For instruct, reply may be omitted."
        .to_string();

    let mut user = String::new();
    user.push_str("Squad roster:\n");
    for m in &input.roster {
        user.push_str(&format!("- name: {}, role: {}", m.name, m.role));
        if !m.tickets.is_empty() {
            user.push_str(&format!(", tickets: {}", m.tickets.join("; ")));
        }
        user.push('\n');
    }
    if !input.summary.trim().is_empty() {
        user.push_str(&format!("\nEarlier in the call (summary):\n{}\n", input.summary.trim()));
    }
    if !input.turns.is_empty() {
        user.push_str("\nRecent turns:\n");
        for t in &input.turns {
            let who = match t.who {
                Who::Person => "Person",
                Who::Equipe => "Equipe",
            };
            user.push_str(&format!("{who}: {}\n", t.text));
        }
    }
    if !input.dispatched.is_empty() {
        user.push_str("\nAlready sent this call:\n");
        for d in &input.dispatched {
            user.push_str(&format!("- to {}: {}\n", target_label(&d.to), d.text));
        }
    }
    if let Some(p) = &input.pending {
        user.push_str(&format!("\nPending (asked them to clarify): {p}\n"));
    }
    user.push_str(&format!("\nLatest utterance:\n{}\n", input.utterance));
    (system, user)
}

async fn ask_model(llm: &LlmConfig, key: &str, system: &str, user: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": llm.model,
        "max_tokens": 400,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
    });
    let body = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let url = format!("{}/v1/messages", llm.base.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(url)
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        let text = String::from_utf8_lossy(&bytes);
        return Err(format!("HTTP {status}: {}", text.chars().take(200).collect::<String>()));
    }
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let text: String = v
        .get("content")
        .and_then(|c| c.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    if text.trim().is_empty() {
        return Err("no text in the model's answer".to_string());
    }
    Ok(text)
}

#[derive(serde::Deserialize)]
struct RawAnswer {
    intent: String,
    #[serde(default)]
    routes: Vec<RawRoute>,
    #[serde(default)]
    reply: Option<String>,
}

#[derive(serde::Deserialize)]
struct RawRoute {
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

fn member_target(m: &RosterMember) -> CallTarget {
    let kind = if m.role.eq_ignore_ascii_case("pm") {
        TargetKind::Pm
    } else {
        TargetKind::Member
    };
    CallTarget {
        kind,
        session: Some(m.session),
        name: Some(m.name.clone()),
        role: Some(m.role.clone()),
    }
}

/// The PM: the roster's pm member, or a bare `pm` target.
fn pm_target(roster: &[RosterMember]) -> CallTarget {
    roster
        .iter()
        .find(|m| m.role.eq_ignore_ascii_case("pm"))
        .map(member_target)
        .unwrap_or(CallTarget {
            kind: TargetKind::Pm,
            session: None,
            name: None,
            role: None,
        })
}

fn squad_target() -> CallTarget {
    CallTarget {
        kind: TargetKind::Squad,
        session: None,
        name: None,
        role: None,
    }
}

/// A `to` from the model, resolved to the roster. Unknown names go to the PM.
fn resolve(to: &str, roster: &[RosterMember]) -> CallTarget {
    let to = to.trim().trim_start_matches('@');
    let lower = to.to_lowercase();
    if matches!(lower.as_str(), "everyone" | "all" | "squad" | "everybody") {
        return squad_target();
    }
    if lower == "pm" {
        return pm_target(roster);
    }
    if let Some(m) = roster.iter().find(|m| m.name.eq_ignore_ascii_case(to)) {
        return member_target(m);
    }
    if let Some(m) = roster.iter().find(|m| m.role.eq_ignore_ascii_case(to)) {
        return member_target(m);
    }
    pm_target(roster)
}

/// Adds a route, merging text into an existing route to the same target.
fn push_route(routes: &mut Vec<CallRoute>, to: CallTarget, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if let Some(r) = routes.iter_mut().find(|r| r.to == to) {
        r.text.push(' ');
        r.text.push_str(text);
    } else {
        routes.push(CallRoute {
            to,
            text: text.to_string(),
        });
    }
}

/// Reads the model's JSON answer (fences and text around it tolerated).
pub fn parse(
    json_text: &str,
    roster: &[RosterMember],
) -> Result<(Intent, Vec<CallRoute>, Option<String>), String> {
    let start = json_text.find('{').ok_or("no JSON object")?;
    let end = json_text.rfind('}').ok_or("no JSON object")?;
    if end < start {
        return Err("no JSON object".to_string());
    }
    let raw: RawAnswer =
        serde_json::from_str(&json_text[start..=end]).map_err(|e| format!("bad JSON: {e}"))?;
    let intent = match raw.intent.trim().to_lowercase().as_str() {
        "instruct" => Intent::Instruct,
        "question" => Intent::Question,
        "end" => Intent::End,
        "unclear" => Intent::Unclear,
        other => return Err(format!("unknown intent {other:?}")),
    };
    let mut routes = Vec::new();
    if intent == Intent::Instruct {
        for r in &raw.routes {
            let (Some(to), Some(text)) = (&r.to, &r.text) else {
                continue;
            };
            if to.trim().is_empty() {
                continue;
            }
            push_route(&mut routes, resolve(to, roster), text);
        }
        if routes.is_empty() {
            return Err("instruct without routes".to_string());
        }
    }
    let reply = raw
        .reply
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    Ok((intent, routes, reply))
}

/// Normalizes for matching: lowercase, curly apostrophes straightened.
fn normalize(text: &str) -> String {
    text.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'")
}

/// Whether `c` is from a script written without spaces between words, or
/// with particles joined on (Hangul, kana, Chinese characters).
fn unspaced(c: char) -> bool {
    matches!(c, '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{3040}'..='\u{30FF}' | '\u{4E00}'..='\u{9FFF}')
}

/// `phrase` in `hay` on word boundaries; a phrase in an unspaced script
/// ([`unspaced`]) matches anywhere.
fn has_phrase(hay: &str, phrase: &str) -> bool {
    if phrase.chars().any(unspaced) {
        return hay.contains(phrase);
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '\'' || c == '-';
    let mut from = 0;
    while let Some(i) = hay[from..].find(phrase) {
        let at = from + i;
        let before = hay[..at].chars().next_back();
        let after = hay[at + phrase.len()..].chars().next();
        let ok_before = before.is_none_or(|c| !is_word(c));
        let ok_after = after.is_none_or(|c| !is_word(c));
        if ok_before && ok_after {
            return true;
        }
        from = at + phrase.len();
    }
    false
}

/// Words that end a call, in every language (`locales/`).
fn end_phrases() -> &'static [&'static str] {
    crate::i18n::every("match.call_end")
}

/// Role words → role (`squad` for everyone), in every language (`locales/`).
fn role_words() -> impl Iterator<Item = (&'static str, &'static str)> {
    [
        ("match.call_designer", "designer"),
        ("match.call_backend", "backend"),
        ("match.call_frontend", "frontend"),
        ("match.call_qa", "qa"),
        ("match.call_everyone", "squad"),
    ]
    .into_iter()
    .flat_map(|(key, role)| crate::i18n::every(key).iter().map(move |w| (*w, role)))
}

fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c == '\n' || c == '\r' {
            let s = cur.trim();
            if !s.is_empty() {
                out.push(s.to_string());
            }
            cur.clear();
            continue;
        }
        cur.push(c);
        if matches!(c, '.' | '!' | '?' | '。') {
            let s = cur.trim();
            if !s.is_empty() {
                out.push(s.to_string());
            }
            cur.clear();
        }
    }
    let s = cur.trim();
    if !s.is_empty() {
        out.push(s.to_string());
    }
    out
}

/// Word rules: end phrases, then role words per sentence. `None` if nothing matched.
pub fn rules(text: &str, roster: &[RosterMember]) -> Option<Judged> {
    let norm = normalize(text);
    if end_phrases().iter().any(|p| has_phrase(&norm, p)) {
        return Some(Judged {
            intent: Intent::End,
            routes: Vec::new(),
            reply: None,
            by: RouteBy::Rules,
            millis: 0,
            input_bytes: 0,
        });
    }

    let mut context: Vec<String> = Vec::new();
    let mut routes: Vec<CallRoute> = Vec::new();
    for sentence in sentences(text) {
        let s = normalize(&sentence);
        let mut targets: Vec<CallTarget> = Vec::new();
        let mut named = false;
        for (word, role) in role_words() {
            if !has_phrase(&s, word) {
                continue;
            }
            named = true;
            let target = if role == "squad" {
                Some(squad_target())
            } else {
                roster
                    .iter()
                    .find(|m| m.role.eq_ignore_ascii_case(role))
                    .map(member_target)
            };
            if let Some(t) = target
                && !targets.contains(&t)
            {
                targets.push(t);
            }
        }
        if !named {
            context.push(sentence);
            continue;
        }
        for t in targets {
            push_route(&mut routes, t, &sentence);
        }
    }
    if routes.is_empty() {
        return None;
    }
    if !context.is_empty() {
        let context = context.join(" ");
        for r in &mut routes {
            r.text = format!("{context} {}", r.text);
        }
    }
    Some(Judged {
        intent: Intent::Instruct,
        routes,
        reply: None,
        by: RouteBy::Rules,
        millis: 0,
        input_bytes: 0,
    })
}

/// Nothing matched: the whole utterance to the PM.
pub fn fallback(text: &str, roster: &[RosterMember]) -> Judged {
    Judged {
        intent: Intent::Instruct,
        routes: vec![CallRoute {
            to: pm_target(roster),
            text: text.trim().to_string(),
        }],
        reply: Some(FALLBACK_REPLY.to_string()),
        by: RouteBy::Fallback,
        millis: 0,
        input_bytes: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &str = "The login screen changed. Designer, redo the mockups. Backend, check if the API needs to change.";

    fn member(session: u64, name: &str, role: &str) -> RosterMember {
        RosterMember {
            session,
            name: name.to_string(),
            role: role.to_string(),
            tickets: vec![format!("{role} ticket")],
        }
    }

    fn roster() -> Vec<RosterMember> {
        vec![
            member(1, "pm", "pm"),
            member(2, "designer", "designer"),
            member(3, "backend", "backend"),
            member(4, "frontend", "frontend"),
        ]
    }

    fn input(text: &str) -> JudgeInput {
        JudgeInput {
            summary: String::new(),
            turns: vec![Turn {
                who: Who::Person,
                text: "hi".to_string(),
            }],
            dispatched: Vec::new(),
            pending: None,
            utterance: text.to_string(),
            roster: roster(),
        }
    }

    fn assert_golden_routes(routes: &[CallRoute]) {
        assert_eq!(routes.len(), 2, "{routes:?}");
        assert_eq!(routes[0].to.role.as_deref(), Some("designer"));
        assert_eq!(routes[0].to.session, Some(2));
        assert_eq!(routes[0].to.kind, TargetKind::Member);
        assert_eq!(routes[1].to.role.as_deref(), Some("backend"));
        assert_eq!(routes[1].to.session, Some(3));
        for r in routes {
            assert!(r.text.contains("The login screen changed."), "{}", r.text);
        }
        assert!(routes[0].text.contains("redo the mockups"));
        assert!(!routes[0].text.contains("API"));
        assert!(routes[1].text.contains("API"));
    }

    #[test]
    fn rules_golden_sentence() {
        let j = rules(GOLDEN, &roster()).expect("routes");
        assert_eq!(j.intent, Intent::Instruct);
        assert_eq!(j.by, RouteBy::Rules);
        assert_golden_routes(&j.routes);
    }

    #[test]
    fn rules_end() {
        for text in ["No, that's all.", "no, that\u{2019}s all", "Okay bye", "끝"] {
            let j = rules(text, &roster()).expect(text);
            assert_eq!(j.intent, Intent::End, "{text}");
            assert!(j.routes.is_empty());
        }
    }

    #[test]
    fn no_target_goes_to_pm() {
        assert!(rules("Make it faster.", &roster()).is_none());
        let j = fallback("Make it faster.", &roster());
        assert_eq!(j.by, RouteBy::Fallback);
        assert_eq!(j.routes.len(), 1);
        assert_eq!(j.routes[0].to.kind, TargetKind::Pm);
        assert_eq!(j.routes[0].to.session, Some(1));
        assert_eq!(j.routes[0].text, "Make it faster.");
        let j = fallback("Make it faster.", &[member(2, "designer", "designer")]);
        assert_eq!(j.routes[0].to.kind, TargetKind::Pm);
        assert_eq!(j.routes[0].to.session, None);
    }

    #[test]
    fn rules_unknown_role_and_word_boundaries() {
        assert!(rules("Marketing, write a post.", &roster()).is_none());
        assert!(rules("Install the package.", &roster()).is_none());
        // QA isn't in the roster: skipped.
        assert!(rules("QA, test the login.", &roster()).is_none());
    }

    #[test]
    fn rules_everyone_and_korean_and_merge() {
        let j = rules("Everyone, ship by five.", &roster()).unwrap();
        assert_eq!(j.routes.len(), 1);
        assert_eq!(j.routes[0].to.kind, TargetKind::Squad);
        assert_eq!(j.routes[0].to.session, None);

        let j = rules("디자이너는 시안을 다시 그려줘. 백엔드는 API 확인해줘.", &roster()).unwrap();
        assert_eq!(j.routes.len(), 2);

        let j = rules("Designer, redo the mockups. Designer, also the icons.", &roster()).unwrap();
        assert_eq!(j.routes.len(), 1);
        assert!(j.routes[0].text.contains("icons"));
    }

    #[test]
    fn parse_fenced_and_unknown_role() {
        let text = "Here you go:\n```json\n{\"intent\":\"instruct\",\"routes\":[{\"to\":\"designer\",\"text\":\"Redo the mockups.\"},{\"to\":\"Backend\",\"text\":\"Check the API.\"},{\"to\":\"marketing\",\"text\":\"Write a post.\"},{\"to\":\"everyone\",\"text\":\"Ship.\"}],\"reply\":\"\"}\n```";
        let (intent, routes, reply) = parse(text, &roster()).unwrap();
        assert_eq!(intent, Intent::Instruct);
        assert_eq!(reply, None);
        assert_eq!(routes.len(), 4);
        assert_eq!(routes[0].to.session, Some(2));
        assert_eq!(routes[1].to.session, Some(3));
        assert_eq!(routes[2].to.kind, TargetKind::Pm);
        assert_eq!(routes[2].to.session, Some(1));
        assert_eq!(routes[3].to.kind, TargetKind::Squad);
    }

    #[test]
    fn parse_errors() {
        assert!(parse("no json here", &roster()).is_err());
        assert!(parse("{\"intent\":\"instruct\",\"routes\":[]}", &roster()).is_err());
        assert!(parse("{\"intent\":\"dance\"}", &roster()).is_err());
        let (intent, routes, _) = parse("{\"intent\":\"end\",\"reply\":\"Bye.\"}", &roster()).unwrap();
        assert_eq!(intent, Intent::End);
        assert!(routes.is_empty());
    }

    #[test]
    fn from_env_defaults() {
        let off = LlmConfig::off();
        assert!(off.key.is_none());
        assert_eq!(off.model, MODEL);
        assert_eq!(off.timeout, TIMEOUT);
    }

    #[tokio::test]
    async fn judge_without_model_uses_rules() {
        let j = judge(&input(GOLDEN), &LlmConfig::off()).await;
        assert_eq!(j.by, RouteBy::Rules);
        assert_eq!(j.input_bytes, 0);
        assert_golden_routes(&j.routes);

        let j = judge(&input("Make it faster."), &LlmConfig::off()).await;
        assert_eq!(j.by, RouteBy::Fallback);
        assert_eq!(j.routes[0].to.kind, TargetKind::Pm);
    }

    #[tokio::test]
    async fn judge_unreachable_model_uses_rules() {
        let llm = LlmConfig {
            key: Some("test".to_string()),
            base: "http://127.0.0.1:9".to_string(),
            ..LlmConfig::off()
        };
        let j = judge(&input(GOLDEN), &llm).await;
        assert_eq!(j.by, RouteBy::Rules);
        assert!(j.input_bytes > 0);
        assert!(j.millis < 2000, "{}", j.millis);
        assert_golden_routes(&j.routes);
    }

    #[tokio::test]
    async fn judge_timeout_uses_rules() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });
        let llm = LlmConfig {
            key: Some("test".to_string()),
            base: format!("http://{addr}"),
            timeout: Duration::from_millis(200),
            ..LlmConfig::off()
        };
        let j = judge(&input(GOLDEN), &llm).await;
        assert_eq!(j.by, RouteBy::Rules);
        assert!(j.millis < 1000, "{}", j.millis);
        assert_golden_routes(&j.routes);
    }

    #[tokio::test]
    async fn judge_model_answer_and_question() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let answers = [
                r#"{"intent":"instruct","routes":[{"to":"designer","text":"Redo the mockups."}]}"#,
                r#"{"intent":"question","routes":[],"reply":"x"}"#,
            ];
            for answer in answers {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 65536];
                let mut got = Vec::new();
                // Read headers and body (content-length).
                loop {
                    let n = s.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    got.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&got).to_string();
                    if let Some(h) = text.find("\r\n\r\n") {
                        let len = text[..h]
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if got.len() >= h + 4 + len {
                            break;
                        }
                    }
                }
                let body = serde_json::json!({"content":[{"type":"text","text": answer}]}).to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                s.write_all(resp.as_bytes()).await.unwrap();
                s.shutdown().await.ok();
            }
        });
        let llm = LlmConfig {
            key: Some("test".to_string()),
            base: format!("http://{addr}"),
            ..LlmConfig::off()
        };
        let j = judge(&input(GOLDEN), &llm).await;
        assert_eq!(j.by, RouteBy::Llm);
        assert_eq!(j.routes.len(), 1);
        assert_eq!(j.routes[0].to.session, Some(2));
        assert_eq!(j.reply, None);

        let j = judge(&input("What is the backend doing?"), &llm).await;
        assert_eq!(j.by, RouteBy::Llm);
        assert_eq!(j.intent, Intent::Question);
        assert_eq!(j.routes[0].to.kind, TargetKind::Pm);
        assert_eq!(j.reply.as_deref(), Some(QUESTION_REPLY));
    }
}
