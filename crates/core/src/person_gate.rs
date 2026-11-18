//! What is worth the person's yes. Small things — a UI tweak its design QA
//! passed, accepted work merged again, internal docs — the PM decides itself
//! where it accepts the team's work, and they go into the person's daily
//! digest ("Decided for you" under Your requests); only what touches
//! production data, permissions or security, money, legal matters, a big
//! visible change, or what can't be taken back reaches them as a card — with
//! why. And whatever reaches them names each ticket by its title and one
//! plain line on what changes for people ([`what_changes`]), never by number
//! alone. Pure rules, tested here; `runtime::person_gate` applies them.

use std::collections::BTreeMap;

use equipe_protocol::autonomy::{Kind, Level, Policy};
use equipe_protocol::title::TicketLabel;
use equipe_protocol::{Ticket, TicketKind};

use crate::migrations;

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
    weigh_change(ticket, changed, &BTreeMap::new())
}

/// Files that go with a schema migration: the schema it describes
/// (`schema.prisma`, `schema.rb`, `db/schema.ts`), a migrations folder's
/// lock or journal, the database code using it. Not a migration in another
/// language (`db/migrate/…rb`, `migrations/…py`), which can't be read, nor seeds.
fn goes_with_migration(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let code_migration = lower.contains("migrat") && lower.rsplit_once('.').is_some_and(|(_, kind)| MIGRATION_CODE.contains(&kind));
    !code_migration && !lower.contains("seed") && ["migrat", "schema", "db/", "database", "supabase/"].iter().any(|s| lower.contains(s))
}

/// Kinds of file a migration written as code comes in.
const MIGRATION_CODE: &[&str] = &["rb", "py", "js", "ts", "mjs", "cjs", "go", "java", "kt", "php", "ex", "exs", "cs", "rs", "scala"];

/// [`weigh`], with `sql` the contents of the SQL migrations its branch
/// changed, by path. When every SQL migration it runs forward only adds
/// (`crate::migrations`) — new tables, indexes, columns existing rows can
/// take — production data is no reason to hold it: neither those files, the
/// files that go with them, nor its title's words say so. A migration whose
/// contents aren't given, or that drops, deletes, updates, retypes or
/// renames, weighs as before. Down migrations aren't run forward.
pub fn weigh_change(ticket: &Ticket, changed: &[String], sql: &BTreeMap<String, String>) -> Option<Why> {
    let mut found: Vec<Why> = Vec::new();
    if ticket.kind == TicketKind::Action {
        found.push(Why::Outward);
    }
    if ticket.label == Some(TicketLabel::Deploy) {
        found.push(Why::Irreversible);
    }
    // Docs and notes only: never weighty, whatever they mention.
    let docs = |p: &str| p.ends_with(".md") || p.ends_with(".txt") || p.starts_with("docs/") || p.contains("/docs/") || p.starts_with("design/");
    let forward: Vec<&String> = changed.iter().filter(|p| !docs(p) && migrations::is_sql(p)).collect();
    let additive = !forward.is_empty() && forward.iter().all(|p| sql.get(p.as_str()).is_some_and(|text| migrations::classify(text).is_additive()));
    for path in changed.iter().filter(|p| !docs(p)) {
        // A rollback isn't run when it lands; an additive migration lands like code.
        if (path.to_ascii_lowercase().ends_with(".sql") && migrations::is_down(path)) || (additive && migrations::is_sql(path)) {
            continue;
        }
        match sign_in(path) {
            Some(Why::Data) if additive && goes_with_migration(path) => {}
            sign => found.extend(sign),
        }
    }
    found.extend(sign_in(&ticket.title).filter(|why| !(additive && *why == Why::Data)));
    let screens = changed.iter().filter(|p| p.rsplit_once('.').is_some_and(|(_, kind)| SCREEN_KINDS.contains(&kind.to_ascii_lowercase().as_str()))).count();
    if screens > VISIBLE_FILES {
        found.push(Why::Visible);
    }
    found.into_iter().min()
}

/// The largest migration file read.
const MOST_SQL: u64 = 512 * 1024;

/// The contents of the SQL migrations among `changed`, read from the
/// worktree at `root`, by path: a file gone (deleted) or too big to read
/// is left out, and so weighs as before.
pub fn read_sql(root: &std::path::Path, changed: &[String]) -> BTreeMap<String, String> {
    changed
        .iter()
        .filter(|p| migrations::is_sql(p))
        .filter_map(|p| {
            let path = root.join(p);
            let size = std::fs::metadata(&path).ok()?.len();
            (size <= MOST_SQL).then(|| std::fs::read_to_string(&path).ok()).flatten().map(|text| (p.clone(), text))
        })
        .collect()
}

/// The SQL migration files `words` (a member's request) names, found in
/// the worktree at `root` — by their path, or by file name under it — when
/// it names at least one and every one only adds (`crate::migrations`).
/// `None` when it names none, one can't be found or read, or one doesn't only add.
pub fn additive_migrations_named(root: &std::path::Path, words: &str) -> Option<Vec<String>> {
    let named: Vec<String> = words
        .split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']' | '“' | '”' | '「' | '」' | '、' | '，'))
        .map(|t| t.trim_end_matches(['.', ':', '!', '?']).trim_start_matches("./"))
        .filter(|t| t.to_ascii_lowercase().ends_with(".sql"))
        .map(str::to_owned)
        .collect();
    if named.is_empty() {
        return None;
    }
    let mut found = Vec::new();
    for name in &named {
        let path = Some(root.join(name)).filter(|p| p.is_file()).or_else(|| find_file(root, name.rsplit('/').next().unwrap_or(name)))?;
        if std::fs::metadata(&path).ok()?.len() > MOST_SQL || migrations::is_down(name) {
            return None;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        if !migrations::classify(&text).is_additive() {
            return None;
        }
        if !found.contains(name) {
            found.push(name.clone());
        }
    }
    Some(found)
}

/// A file called `name` under `root`, looked for a few levels down past
/// what is never a project's own (`.git`, `node_modules`, build folders).
fn find_file(root: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    const SKIP: &[&str] = &[".git", "node_modules", "target", "dist", "build", ".next", "vendor", ".venv", "venv"];
    let mut dirs = vec![(root.to_path_buf(), 0usize)];
    let mut looked = 0usize;
    while let Some((dir, depth)) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            looked += 1;
            if looked > 20_000 {
                return None;
            }
            let path = entry.path();
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                if depth < 6 && !SKIP.contains(&file_name.as_ref()) {
                    dirs.push((path, depth + 1));
                }
            } else if file_name == name {
                return Some(path);
            }
        }
    }
    None
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
    fn an_additive_migration_lands_like_code_and_a_destructive_one_needs_the_person() {
        let t = ticket("Add usage event timing migration", "");
        let files = paths(&["db/migrations/225_usage_event_timing.sql", "db/migrations/226_usage_event_index.sql", "src/db/usage.ts", "prisma/schema.prisma"]);
        let sql = |a: &str, b: &str| BTreeMap::from([(files[0].clone(), a.to_owned()), (files[1].clone(), b.to_owned())]);
        let additive = sql("CREATE TABLE usage_event_timing (id bigserial PRIMARY KEY, took_ms int);", "CREATE INDEX usage_events_at ON usage_events (at); ALTER TABLE usage_events ADD COLUMN source text DEFAULT 'web';");
        assert_eq!(weigh_change(&t, &files, &additive), None, "new tables, an index, a defaulted column: the title's 'migration' doesn't hold it either");
        assert_eq!(weigh_change(&t, &files, &sql("CREATE TABLE a (id int);", "ALTER TABLE usage_events DROP COLUMN note;")), Some(Why::Data), "one destructive file is enough");
        assert_eq!(weigh_change(&t, &files, &sql("UPDATE usage_events SET source = 'web';", "CREATE INDEX i ON t (a);")), Some(Why::Data));
        assert_eq!(weigh_change(&t, &files, &BTreeMap::from([(files[0].clone(), "CREATE TABLE a (id int);".to_owned())])), Some(Why::Data), "a migration whose SQL wasn't read weighs as before");
        assert_eq!(weigh(&t, &files), Some(Why::Data), "no contents: as before");
        // Other reasons still hold it: an additive migration doesn't excuse a change to sign-in code.
        let mut with_auth = files.clone();
        with_auth.push("src/lib/auth/session.ts".into());
        assert_eq!(weigh_change(&t, &with_auth, &additive), Some(Why::Security));
        // A migration in code can't be read: it still asks.
        let mut with_rails = files.clone();
        with_rails.push("db/migrate/20261118_add_timing.rb".into());
        assert_eq!(weigh_change(&t, &with_rails, &additive), Some(Why::Data));
        // A rollback beside it isn't run when it lands.
        let mut with_down = files.clone();
        with_down.push("db/migrations/225_usage_event_timing.down.sql".into());
        assert_eq!(weigh_change(&t, &with_down, &additive), None);
    }

    #[test]
    fn a_request_naming_only_additive_migrations_needs_no_one() {
        let root = std::env::temp_dir().join(format!("equipe-person-gate-sql-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("db/migrations")).unwrap();
        std::fs::write(root.join("db/migrations/225_usage_event_timing.sql"), "CREATE TABLE usage_event_timing (id bigserial PRIMARY KEY);").unwrap();
        std::fs::write(root.join("db/migrations/226_usage_event_source.sql"), "ALTER TABLE usage_events ADD COLUMN source text;").unwrap();
        std::fs::write(root.join("db/migrations/227_drop_old.sql"), "DROP TABLE old_usage;").unwrap();
        let named = additive_migrations_named(&root, "Apply `225_usage_event_timing.sql` and db/migrations/226_usage_event_source.sql to production.");
        assert_eq!(named, Some(vec!["225_usage_event_timing.sql".to_owned(), "db/migrations/226_usage_event_source.sql".to_owned()]));
        assert_eq!(additive_migrations_named(&root, "Apply 225_usage_event_timing.sql and 227_drop_old.sql"), None, "one drops");
        assert_eq!(additive_migrations_named(&root, "Apply 999_missing.sql"), None, "not found");
        assert_eq!(additive_migrations_named(&root, "Create a test draft in the ERP"), None, "names no migration");
        let read = read_sql(&root, &paths(&["db/migrations/225_usage_event_timing.sql", "src/app.ts", "db/migrations/gone.sql"]));
        assert_eq!(read.keys().cloned().collect::<Vec<_>>(), vec!["db/migrations/225_usage_event_timing.sql".to_owned()]);
        let _ = std::fs::remove_dir_all(&root);
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
