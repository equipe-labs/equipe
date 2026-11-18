//! What is worth the person's yes. Small things — a UI tweak its design QA
//! passed, accepted work merged again, internal docs — the PM decides itself
//! where it accepts the team's work, and they go into the person's daily
//! digest ("Decided for you" under Your requests); only what touches
//! production data, permissions or security, money, legal matters, a big
//! visible change, or what can't be taken back reaches them as a card — with
//! why. And whatever reaches them names each ticket by its title and one
//! plain line on what changes for people ([`what_changes`]), never by number
//! alone. Pure rules, tested here; `runtime::person_gate` applies them.

use equipe_protocol::autonomy::{Kind, Level, Policy};
use equipe_protocol::title::TicketLabel;
use equipe_protocol::{Ticket, TicketKind};

/// Why something needs the person, the weightiest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Why {
    /// It acts in the world — an app, a site, a message sent.
    Outward,
    /// It can't be taken back: a deploy, a deletion, infrastructure.
    Irreversible,
    /// Production data: migrations, schemas, the database.
    Data,
    /// Who may do what: sign-in, permissions, secrets.
    Security,
    /// Money: payments, prices, billing.
    Money,
    /// Licences, terms, privacy.
    Legal,
    /// A change many screens show.
    Visible,
}

impl Why {
    /// Its kind in the squad's autonomy policy (`protocol::autonomy`), the
    /// one source of what may go without the person: outward, irreversible,
    /// security, money and legal are on its floor; production data follows
    /// the policy; a big visible change has no kind there and always asks.
    pub fn kind(self) -> Option<Kind> {
        match self {
            Why::Outward => Some(Kind::Outside),
            Why::Irreversible => Some(Kind::Deletion),
            Why::Data => Some(Kind::ProductionData),
            Why::Security => Some(Kind::Security),
            Why::Money => Some(Kind::Money),
            Why::Legal => Some(Kind::Legal),
            Why::Visible => None,
        }
    }

    /// Whether the squad's policy lets the PM settle work weighty for this
    /// reason on its own: only a kind off the floor the person set to "just
    /// do". Decide-and-tell has no window on an accept card, so it asks.
    pub fn pm_may(self, policy: &Policy) -> bool {
        self.kind().is_some_and(|k| !k.is_floor() && policy.level(k) == Level::JustDo)
    }

    /// Its locale key (`gate.why.*`).
    pub fn key(self) -> &'static str {
        match self {
            Why::Outward => "gate.why.outward",
            Why::Irreversible => "gate.why.irreversible",
            Why::Data => "gate.why.data",
            Why::Security => "gate.why.security",
            Why::Money => "gate.why.money",
            Why::Legal => "gate.why.legal",
            Why::Visible => "gate.why.visible",
        }
    }
}

/// Signs in a changed file's path (or an English title) of what it touches.
/// A sign with a `/` is looked for anywhere in the path; one starting with a
/// `.` is the file's kind (or a dotfile); any other is a word of the path —
/// the whole word, a plural, or, from six letters on, its start (`migration`
/// is in `migrations`, but `auth` isn't in `AuthorCard`).
const SIGNS: &[(Why, &[&str])] = &[
    (Why::Irreversible, &["deploy", "deployment", ".github/workflows", "terraform", "infra/", "dockerfile", "k8s", "helm", "vercel.json", "fly.toml", "purge"]),
    (Why::Data, &["migration", "migrate", "schema", ".sql", "seed", "database", "db/", "supabase/", "production", "prod"]),
    (Why::Security, &["auth", "oauth", "authz", "authn", "permission", "rbac", "policy", "policies", "secret", "credential", "apikey", ".env", "security", "crypto", "password"]),
    (Why::Money, &["payment", "billing", "invoice", "pricing", "price", "checkout", "subscription", "stripe", "refund", "payout"]),
    (Why::Legal, &["license", "licence", "terms", "privacy", "legal", "gdpr", "consent"]),
];

/// More screens' files changed than this is a big visible change.
const VISIBLE_FILES: usize = 12;

/// Kinds of file a screen is made of.
const SCREEN_KINDS: &[&str] = &["tsx", "jsx", "vue", "svelte", "css", "scss", "html", "swift", "kt", "dart", "astro"];

fn sign_in(text: &str) -> Option<Why> {
    let text = text.to_lowercase();
    let words: Vec<&str> = text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |sign: &str| {
        if sign.contains('/') {
            text.contains(sign)
        } else if let Some(kind) = sign.strip_prefix('.') {
            text.ends_with(sign) || text.split('/').any(|part| part == sign || part.starts_with(&format!(".{kind}.")))
        } else {
            words.iter().any(|w| *w == sign || w.strip_suffix('s') == Some(sign) || (sign.len() >= 6 && w.starts_with(sign)))
        }
    };
    SIGNS.iter().find(|(_, signs)| signs.iter().any(|s| has(s))).map(|(why, _)| *why)
}

/// Whether a ticket needs the person's yes, and why — from what it is
/// (acting in the world, a deploy), the files its branch changed, and its
/// title's words. `None`: small, the PM's to decide.
pub fn weigh(ticket: &Ticket, changed: &[String]) -> Option<Why> {
    let mut found: Vec<Why> = Vec::new();
    if ticket.kind == TicketKind::Action {
        found.push(Why::Outward);
    }
    if ticket.label == Some(TicketLabel::Deploy) {
        found.push(Why::Irreversible);
    }
    // Docs and notes only: never weighty, whatever they mention.
    let docs = |p: &str| p.ends_with(".md") || p.ends_with(".txt") || p.starts_with("docs/") || p.contains("/docs/") || p.starts_with("design/");
    found.extend(changed.iter().filter(|p| !docs(p)).filter_map(|p| sign_in(p)));
    found.extend(sign_in(&ticket.title));
    let screens = changed.iter().filter(|p| p.rsplit_once('.').is_some_and(|(_, kind)| SCREEN_KINDS.contains(&kind.to_ascii_lowercase().as_str()))).count();
    if screens > VISIBLE_FILES {
        found.push(Why::Visible);
    }
    found.into_iter().min()
}

/// What changes for people, in a plain line: the first sentence of its
/// hand-in (written for the person, in their language), else of its body —
/// without markdown's marks — cut short. Empty when it says nothing.
pub fn what_changes(ticket: &Ticket) -> String {
    let source = ticket.note.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(&ticket.body);
    let line = source
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.trim_start_matches(['-', '*', '>', ' ']).trim())
        .find(|l| !l.is_empty() && !l.starts_with("```") && !l.starts_with('|'))
        .unwrap_or_default()
        .replace(['`', '*'], "");
    let mut end = line.len();
    for (i, c) in line.char_indices() {
        let next = line[i + c.len_utf8()..].chars().next();
        if matches!(c, '。' | '！' | '？') || (matches!(c, '.' | '!' | '?') && next.is_none_or(char::is_whitespace)) {
            end = i + c.len_utf8();
            break;
        }
    }
    let sentence = line[..end].trim();
    if sentence.chars().count() <= 110 {
        return sentence.to_owned();
    }
    let kept: String = sentence.chars().take(109).collect();
    format!("{}…", kept.trim_end())
}

/// A ticket as a card names it: its title, and what changes for people.
pub fn named(ticket: &Ticket) -> String {
    let what = what_changes(ticket);
    if what.is_empty() { format!("“{}”", ticket.title) } else { format!("“{}” — {what}", ticket.title) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::TicketStatus;

    fn ticket(title: &str, note: &str) -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": 1, "title": title, "body": "", "status": "review", "owner": "frontend", "assignee": "frontend",
            "project": null, "depends_on": [], "branch": "equipe/t1", "created_by": "pm", "note": note
        }))
        .unwrap()
    }

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn small_things_are_the_pms_and_weighty_ones_the_persons_with_why() {
        let tweak = ticket("Tighten the composer's padding", "");
        assert_eq!(weigh(&tweak, &paths(&["src/components/Composer.tsx", "src/styles/composer.css"])), None, "a UI tweak");
        assert_eq!(weigh(&tweak, &paths(&["docs/composer.md", "design/specs/side-nav.md"])), None, "internal docs, whatever they say");
        assert_eq!(weigh(&tweak, &paths(&["prisma/migrations/2026_add_role/migration.sql"])), Some(Why::Data));
        assert_eq!(weigh(&tweak, &paths(&["src/lib/auth/session.ts"])), Some(Why::Security));
        assert_eq!(weigh(&tweak, &paths(&["src/app/checkout/page.tsx"])), Some(Why::Money));
        assert_eq!(weigh(&tweak, &paths(&["LICENSE"])), Some(Why::Legal));
        assert_eq!(weigh(&tweak, &paths(&[".github/workflows/deploy.yml", "src/lib/auth/x.ts"])), Some(Why::Irreversible), "the weightiest first");
        let many: Vec<String> = (0..13).map(|i| format!("src/screens/S{i}.tsx")).collect();
        assert_eq!(weigh(&tweak, &many), Some(Why::Visible));
        let mut action = ticket("Post the launch note", "");
        action.kind = TicketKind::Action;
        assert_eq!(weigh(&action, &[]), Some(Why::Outward));
        let mut deploy = ticket("Ship it", "");
        deploy.label = Some(TicketLabel::Deploy);
        assert_eq!(weigh(&deploy, &[]), Some(Why::Irreversible));
        assert_eq!(weigh(&ticket("Rotate the password policy", ""), &[]), Some(Why::Security), "an English title says it too");
        assert_eq!(weigh(&tweak, &paths(&["design/tokens.json", "src/components/AuthorCard.tsx", "src/components/Notice.tsx"])), None, "design tokens, an author, a notice: none of it weighty");
        assert_eq!(tweak.status, TicketStatus::Review);
    }

    #[test]
    fn the_autonomy_policy_decides_what_weighty_work_the_pm_may_settle_and_never_the_floor() {
        let mut policy = Policy::default();
        let all = [Why::Outward, Why::Irreversible, Why::Data, Why::Security, Why::Money, Why::Legal, Why::Visible];
        assert!(all.iter().all(|w| !w.pm_may(&policy)), "by default every weighty reason asks");
        policy.set(Kind::ProductionData, Level::Tell).unwrap();
        assert!(!Why::Data.pm_may(&policy), "decide-and-tell has no window on an accept card");
        policy.set(Kind::ProductionData, Level::JustDo).unwrap();
        assert!(Why::Data.pm_may(&policy));
        for why in all.into_iter().filter(|w| *w != Why::Data) {
            assert!(!why.pm_may(&policy), "{why:?} stays the person's");
            assert!(why.kind().is_none_or(Kind::is_floor), "{why:?} is on the floor or has no kind");
        }
    }

    #[test]
    fn a_card_names_each_ticket_by_title_and_what_changes_for_people() {
        let t = ticket("Apply the composer to Tinumi chat", "## Done\n**The chat input** now grows with the text and keeps the model chip on the left. Tests pass.");
        assert_eq!(what_changes(&t), "The chat input now grows with the text and keeps the model chip on the left.");
        assert_eq!(named(&t), "“Apply the composer to Tinumi chat” — The chat input now grows with the text and keeps the model chip on the left.");
        let ko = ticket("입력창 적용", "입력창이 글 길이에 맞춰 늘어나요. 테스트도 통과했어요.");
        assert_eq!(what_changes(&ko), "입력창이 글 길이에 맞춰 늘어나요.");
        let ja = ticket("入力欄", "入力欄が文の長さに合わせて広がります。テストも通りました。");
        assert_eq!(what_changes(&ja), "入力欄が文の長さに合わせて広がります。");
        assert_eq!(what_changes(&ticket("v1.2 bump", "Bumped to v1.2.3 for the fix")), "Bumped to v1.2.3 for the fix", "a dot inside a word isn't a sentence's end");
        assert_eq!(named(&ticket("Bare", "")), "“Bare”");
        assert!(what_changes(&ticket("Long", &"word ".repeat(60))).ends_with('…'));
    }
}
