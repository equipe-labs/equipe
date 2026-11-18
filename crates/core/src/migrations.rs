//! Whether a schema migration only adds to the database, read from its SQL.
//!
//! A migration that only adds — a new table, a new index, a nullable column
//! or one with a default — changes nothing already there: every row, column
//! and query that worked before works after, and it is undone by dropping
//! what it added. It lands like code (just-do: told in the landed digest).
//! One that drops, deletes, updates existing rows, changes a column's type,
//! renames, or tightens what existing rows must satisfy can lose or break
//! data that is already there: that stays a person's yes
//! (`crate::person_gate`).
//!
//! Read conservatively: only statements known to add are additive; anything
//! not understood — a `DO` block, a function, a statement this doesn't know —
//! makes the whole file need the person. Pure functions, tested here.

/// What a migration file does to the data already there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Migration {
    /// It only adds: lands like code.
    Additive,
    /// It can change or lose what is there — the first statement that does, shortened.
    Destructive(String),
}

impl Migration {
    pub fn is_additive(&self) -> bool {
        matches!(self, Migration::Additive)
    }
}

/// Whether `path` is a SQL migration file run forward — not a down migration,
/// which only runs when one is rolled back.
pub fn is_sql(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".sql") && !is_down(&lower)
}

/// A down (rollback) migration: `0001_x.down.sql`, `down.sql`, `…/down/…`.
pub fn is_down(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let file = lower.rsplit('/').next().unwrap_or(&lower);
    file == "down.sql" || file.ends_with(".down.sql") || file.ends_with("_down.sql") || lower.contains("/down/")
}

/// What the migration `sql` does to the data already there.
pub fn classify(sql: &str) -> Migration {
    // Dollar-quoted bodies (functions, DO blocks) can't be read statement by statement.
    if sql.contains("$$") {
        return Migration::Destructive("a function or DO block".into());
    }
    let statements = split(&strip_comments(sql));
    let mut created: Vec<String> = Vec::new();
    for statement in &statements {
        let words: Vec<String> = statement.split_whitespace().map(str::to_ascii_uppercase).collect();
        if words.is_empty() {
            continue;
        }
        if let Some(table) = created_table(&words, statement) {
            created.push(table);
            continue;
        }
        if !additive(&words, statement, &created) {
            return Migration::Destructive(short(statement));
        }
    }
    Migration::Additive
}

/// The table a `CREATE TABLE` statement makes, by its name as written (lowercase, unquoted).
fn created_table(words: &[String], statement: &str) -> Option<String> {
    let w: Vec<&str> = words.iter().map(String::as_str).collect();
    let rest = match w.as_slice() {
        ["CREATE", "TABLE", "IF", "NOT", "EXISTS", ..] => 5,
        ["CREATE", "TABLE", ..] => 2,
        ["CREATE", "UNLOGGED" | "TEMP" | "TEMPORARY", "TABLE", "IF", "NOT", "EXISTS", ..] => 6,
        ["CREATE", "UNLOGGED" | "TEMP" | "TEMPORARY", "TABLE", ..] => 3,
        _ => return None,
    };
    let name = statement.split_whitespace().nth(rest)?;
    Some(bare_name(name.split('(').next().unwrap_or(name)))
}

/// A table's name without quotes, brackets or a trailing `(`, lowercased.
fn bare_name(name: &str) -> String {
    name.trim_matches(|c: char| matches!(c, '"' | '`' | '[' | ']' | '(' | ';')).replace(['"', '`', '[', ']'], "").to_ascii_lowercase()
}

/// Whether one statement (its words uppercased) only adds.
fn additive(w: &[String], statement: &str, created: &[String]) -> bool {
    let w: Vec<&str> = w.iter().map(String::as_str).collect();
    match w.as_slice() {
        // Transactions and session settings around the rest.
        ["BEGIN" | "COMMIT" | "END", ..] | ["START", "TRANSACTION", ..] | ["SET", ..] => true,
        ["PRAGMA", rest @ ..] => !rest.iter().any(|r| r.contains("WRITABLE_SCHEMA")),
        ["COMMENT", "ON", ..] => true,
        // New indexes, of any kind.
        ["CREATE", "INDEX", ..] | ["CREATE", "UNIQUE", "INDEX", ..] => true,
        // New objects, never one replaced.
        ["CREATE", "SEQUENCE" | "SCHEMA" | "TYPE" | "DOMAIN" | "EXTENSION" | "VIEW" | "MATERIALIZED", ..] => true,
        // Rows put in a table this same migration made.
        ["INSERT", "INTO", table, ..] => created.contains(&bare_name(table.split('(').next().unwrap_or(table))),
        ["ALTER", "TABLE", ..] => alter_adds(&w, statement),
        _ => false,
    }
}

/// Whether an `ALTER TABLE` only adds columns that need nothing of the rows
/// already there: each action `ADD [COLUMN]`, nullable or with a default.
fn alter_adds(w: &[&str], statement: &str) -> bool {
    // ALTER TABLE [IF EXISTS] [ONLY] name action, action…
    let mut at = 2;
    if w.get(at) == Some(&"IF") && w.get(at + 1) == Some(&"EXISTS") {
        at += 2;
    }
    if w.get(at) == Some(&"ONLY") {
        at += 1;
    }
    // Past the table's name: the actions, as written.
    let actions = statement.split_whitespace().skip(at + 1).collect::<Vec<_>>().join(" ");
    if actions.trim().is_empty() {
        return false;
    }
    top_level_commas(&actions).iter().all(|action| add_column(action))
}

/// One `ALTER TABLE` action that adds a column existing rows can take.
fn add_column(action: &str) -> bool {
    let upper = action.to_ascii_uppercase();
    let words: Vec<&str> = upper.split_whitespace().collect();
    let mut at = match words.as_slice() {
        ["ADD", "COLUMN", ..] => 2,
        // `ADD CONSTRAINT`, `ADD PRIMARY KEY`, `ADD FOREIGN KEY`, `ADD UNIQUE`, `ADD CHECK` restrict existing rows.
        ["ADD", "CONSTRAINT" | "PRIMARY" | "FOREIGN" | "UNIQUE" | "CHECK" | "INDEX" | "KEY", ..] => return false,
        ["ADD", ..] => 1,
        _ => return false,
    };
    if words.get(at) == Some(&"IF") && words.get(at + 1) == Some(&"NOT") && words.get(at + 2) == Some(&"EXISTS") {
        at += 3;
    }
    if words.len() <= at + 1 {
        return false;
    }
    let definition = words[at + 1..].join(" ");
    // A primary key or a unique column fills or checks every existing row.
    if definition.contains("PRIMARY KEY") || definition.split_whitespace().any(|w| w == "UNIQUE") {
        return false;
    }
    let not_null = definition.contains("NOT NULL");
    let defaulted = definition.split_whitespace().any(|w| w == "DEFAULT") || definition.contains("GENERATED") || definition.contains("SERIAL");
    !not_null || defaulted
}

/// `text` split at commas outside brackets and quotes.
fn top_level_commas(text: &str) -> Vec<String> {
    let (mut parts, mut current, mut depth, mut quote) = (Vec::new(), String::new(), 0i32, None::<char>);
    for c in text.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"' | '`') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ',') if depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    parts.push(current);
    parts.into_iter().map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect()
}

/// `sql` without its `--` and `/* */` comments, quoted text kept as it is.
fn strip_comments(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            out.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => {
                quote = Some(c);
                out.push(c);
            }
            '-' if chars.peek() == Some(&'-') => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = ' ';
                for n in chars.by_ref() {
                    if last == '*' && n == '/' {
                        break;
                    }
                    last = n;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// `sql` split into statements at semicolons outside quotes.
fn split(sql: &str) -> Vec<String> {
    let (mut statements, mut current, mut quote) = (Vec::new(), String::new(), None::<char>);
    for c in sql.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"' | '`') => quote = Some(c),
            (None, ';') => {
                statements.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    statements.push(current);
    statements.into_iter().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()
}

/// A statement in one short line, for saying why.
fn short(statement: &str) -> String {
    let line = statement.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= 80 {
        return line;
    }
    format!("{}…", line.chars().take(79).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tables_indexes_and_columns_existing_rows_can_take_are_additive() {
        let sql = "-- 225: usage event timing\n\
            BEGIN;\n\
            CREATE TABLE IF NOT EXISTS usage_event_timing (\n  id bigserial PRIMARY KEY,\n  event_id bigint NOT NULL REFERENCES usage_events(id),\n  took_ms integer NOT NULL,\n  note text DEFAULT 'a; b'\n);\n\
            CREATE INDEX CONCURRENTLY IF NOT EXISTS usage_event_timing_event ON usage_event_timing (event_id);\n\
            ALTER TABLE usage_events ADD COLUMN IF NOT EXISTS started_at timestamptz;\n\
            ALTER TABLE usage_events ADD COLUMN source text NOT NULL DEFAULT 'web', ADD COLUMN retries int DEFAULT 0;\n\
            /* a note; with a semicolon */\n\
            COMMENT ON COLUMN usage_events.started_at IS 'when it began; UTC';\n\
            INSERT INTO usage_event_timing (event_id, took_ms) VALUES (1, 2);\n\
            COMMIT;";
        assert_eq!(classify(sql), Migration::Additive);
        assert_eq!(classify("CREATE UNIQUE INDEX one_email ON users (email);"), Migration::Additive);
        assert_eq!(classify("alter table `orders` add `shipped_at` datetime null;"), Migration::Additive, "MySQL, lowercase");
        assert_eq!(classify("CREATE TYPE mood AS ENUM ('ok', 'sad'); CREATE VIEW recent AS SELECT * FROM orders;"), Migration::Additive);
        assert_eq!(classify(""), Migration::Additive, "nothing in it");
    }

    #[test]
    fn what_drops_deletes_updates_retypes_renames_or_restricts_existing_rows_needs_the_person() {
        for sql in [
            "DROP TABLE usage_events;",
            "ALTER TABLE usage_events DROP COLUMN note;",
            "DELETE FROM usage_events WHERE id < 10;",
            "UPDATE usage_events SET source = 'web';",
            "TRUNCATE usage_events;",
            "ALTER TABLE usage_events ALTER COLUMN took_ms TYPE bigint;",
            "ALTER TABLE usage_events RENAME COLUMN note TO memo;",
            "ALTER TABLE usage_events RENAME TO events;",
            "ALTER TABLE usage_events ALTER COLUMN note SET NOT NULL;",
            "ALTER TABLE usage_events ADD COLUMN owner_id bigint NOT NULL;",
            "ALTER TABLE usage_events ADD CONSTRAINT one_per_day UNIQUE (day);",
            "ALTER TABLE usage_events ADD COLUMN code text UNIQUE;",
            "ALTER TABLE usage_events ADD COLUMN a int, DROP COLUMN b;",
            "INSERT INTO usage_events (id) VALUES (1);",
            "GRANT SELECT ON usage_events TO reporting;",
            "CREATE OR REPLACE VIEW recent AS SELECT 1;",
            "CREATE OR REPLACE FUNCTION f() RETURNS int AS $$ SELECT 1 $$ LANGUAGE sql;",
            "DO $$ BEGIN DELETE FROM t; END $$;",
            "CREATE TABLE a (id int); DROP TABLE b;",
        ] {
            assert!(!classify(sql).is_additive(), "{sql}");
        }
        assert_eq!(classify("CREATE TABLE a (id int);\nDROP TABLE b;"), Migration::Destructive("DROP TABLE b".into()), "says which");
        // A drop hidden in a comment is no drop; one in quoted text isn't either.
        assert!(classify("-- DROP TABLE old;\nCREATE TABLE new_one (id int);").is_additive());
        assert!(classify("COMMENT ON TABLE t IS 'we DROP nothing; ok';").is_additive());
    }

    #[test]
    fn down_migrations_are_not_run_forward() {
        assert!(is_sql("db/migrations/225_usage_event_timing.sql"));
        assert!(is_sql("prisma/migrations/2026_add/migration.sql"));
        assert!(!is_sql("migrations/0002_x.down.sql"));
        assert!(!is_sql("migrations/0002_x/down.sql"));
        assert!(!is_sql("src/db/queries.ts"));
        assert!(is_down("migrations/down/0002.sql"));
    }
}
