//! `ask_secret`: a member needs a password, a key or a token for a command or
//! a config, and the person types it into a card instead of a conversation
//! (see `equipe_protocol::secrets`). Checked here — the keys, how many, and,
//! only when the member names one, the file it is also saved to: under the
//! person's home, never one of the places keys and logins live (`~/.ssh`,
//! equipe's own) — as `KEY="value"` lines, readable by the person alone. The
//! values are kept in the system's secret store (login autopilot's
//! command-line values, `crate::vault`) and given to that squad's members'
//! commands as environment variables at run time: not the member's context,
//! not an event, not a log.

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use crate::protocol::secrets::{SecretField, SecretValue};

pub const ASK_SECRET: &str = "ask_secret";

/// At most this many values on one card.
pub const MOST: usize = 6;

/// The tool, as an MCP tool definition.
pub fn tool() -> Value {
    json!({
        "name": ASK_SECRET,
        "description": "Ask the person for a password, key or token a command or a config needs — a NAS login, an API key, \
a database password. They type it into a card in equipe, and equipe keeps it in this machine's secret store: you never \
see the values, and they never pass through the conversation. Once they save it, your commands get each value as an \
environment variable named by its key (`$NAS_PASSWORD`) from your next turn on — use it from there, and never print, \
echo or copy a value into chat, tickets, commits or logs. Only for a program equipe doesn't run (a scheduled job) that \
must read a file, give `save_to`: equipe saves it there too (`KEY=\"value\"` lines, readable only by them). Never ask \
for one in words, and never have the person run a command to type it. Every request carries `steps` (numbered on the card) and, \
for anything from a website or console, the exact page as `url` (one button opens it): a card without them is refused. \
A website sign-in is `sign_in`, not this.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "why": { "type": "string", "description": "What it is for, in a line in the person's language: \"NAS login for the daily automatic import from the NAS\"." },
                "fields": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "key": { "type": "string", "description": "The key it is saved under: NAS_USER (uppercase, digits, _)." },
                            "label": { "type": "string", "description": "How the card names it, in the person's language: \"NAS username\"." },
                            "secret": { "type": "boolean", "description": "Typed hidden: a password, key or token. False for a username." },
                        },
                        "required": ["key", "label"],
                    },
                    "description": "What to ask for, 1 to 6.",
                },
                "steps": crate::connections::steps_schema("what the person does to find or create the value"),
                "url": { "type": "string", "description": "The exact page the value comes from (https, a real site: no credentials, port, IP or localhost). Required for anything from a website or console; left out only with `no_page`, or for a catalog service named in `service` (its key page is used)." },
                "no_page": crate::connections::no_page_schema(),
                "service": { "type": "string", "description": "The service or site it comes from, when it does (`anthropic`, `github`, `app.example.com`): for a service in equipe's catalog the key page is filled in when `url` is left out." },
                "save_to": { "type": "string", "description": "Only for a program equipe doesn't run that must read a file: the file to save it to as well, under the person's home: `~/.config/atinum/nas.env`. Keys already in it are replaced, the rest kept." },
            },
            "required": ["why", "fields", "steps"],
        },
    })
}

/// A call, checked: what the card asks and where it saves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub why: String,
    pub fields: Vec<SecretField>,
    /// The file it is saved to as well, whole — only when the member named one.
    pub path: Option<PathBuf>,
    /// The file as the person reads it: `~/.config/atinum/nas.env`; empty: none.
    pub shown: String,
    /// What the person does, in order; the page it comes from; the sign-in page.
    pub guide: crate::connections::Guide,
}

/// Check an `ask_secret` call against the person's `home` directory.
pub fn check(args: &Value, home: &Path) -> Result<Asked, String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
    let why = text("why");
    if why.is_empty() {
        return Err("Say in `why` what it is for.".into());
    }
    let given = args.get("fields").and_then(Value::as_array).cloned().unwrap_or_default();
    if given.is_empty() || given.len() > MOST {
        return Err(format!("Ask for 1 to {MOST} values in `fields`."));
    }
    let mut fields: Vec<SecretField> = Vec::new();
    for f in &given {
        let key = f.get("key").and_then(Value::as_str).unwrap_or_default().trim().to_owned();
        if !valid_key(&key) {
            return Err(format!("{key:?}: a key is uppercase letters, digits and _, starting with a letter (NAS_PASSWORD)."));
        }
        if fields.iter().any(|g| g.key == key) {
            return Err(format!("{key} is asked for twice."));
        }
        let label = f.get("label").and_then(Value::as_str).map(str::trim).filter(|l| !l.is_empty()).unwrap_or(&key).to_owned();
        fields.push(SecretField { key, label, secret: f.get("secret").and_then(Value::as_bool).unwrap_or(false) });
    }
    let guide = crate::connections::check_guide(args, &text("service"), "url")?;
    let save_to = text("save_to");
    let (path, shown) = if save_to.is_empty() { (None, String::new()) } else { place(&save_to, home).map(|(p, s)| (Some(p), s))? };
    Ok(Asked { why, fields, path, shown, guide })
}

pub(crate) fn valid_key(key: &str) -> bool {
    (1..=64).contains(&key.len()) && key.starts_with(|c: char| c.is_ascii_uppercase()) && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Where `given` may be saved: a file under `home`, by a path without `..`,
/// outside where keys and logins live. The file and how the person reads it.
pub fn place(given: &str, home: &Path) -> Result<(PathBuf, String), String> {
    let given = given.trim();
    if given.is_empty() {
        return Err("Say in `save_to` which file to save it to, under the person's home: ~/.config/<project>/<name>.env.".into());
    }
    let path = match given.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(given),
    };
    let refuse = |why: &str| Err(format!("{given}: {why}"));
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return refuse("give the full path (or one starting with ~/), without `..`");
    }
    let Ok(rest) = path.strip_prefix(home) else { return refuse("save it under the person's home (~/)") };
    let first = rest.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
    if rest.components().count() < 2 && !first.starts_with('.') || rest.as_os_str().is_empty() {
        return refuse("save it in a folder of its own under ~/, such as ~/.config/<project>/");
    }
    if [".ssh", ".gnupg", ".equipe", ".aws", "Library"].contains(&first.as_str()) {
        return refuse("that's where keys and logins already live: pick a file of its own, such as ~/.config/<project>/<name>.env");
    }
    if path.is_dir() {
        return refuse("that's a folder: name the file");
    }
    let shown = format!("~/{}", rest.to_string_lossy());
    Ok((path, shown))
}

/// Save `values` (one for each of `fields`) to `path`: each `KEY="value"`
/// line replaced or added, the file's other lines kept; the file readable by
/// its owner alone, its new folders too.
pub fn write(path: &Path, fields: &[SecretField], values: &[SecretValue]) -> Result<(), String> {
    if values.len() != fields.len() {
        return Err(format!("{} values for {} fields", values.len(), fields.len()));
    }
    if values.iter().any(|v| v.0.contains(['\n', '\r', '\0'])) {
        return Err("a value can't hold a line break".into());
    }
    if let Some(dir) = path.parent() {
        let mut builder = std::fs::DirBuilder::new(); builder.recursive(true);
        #[cfg(unix)]
        { use std::os::unix::fs::DirBuilderExt as _; builder.mode(0o700); }
        builder.create(dir).map_err(|e| e.to_string())?;
    }
    let before = std::fs::read_to_string(path).unwrap_or_default();
    let ours = |line: &str| fields.iter().any(|f| line.trim_start().strip_prefix("export ").unwrap_or(line.trim_start()).starts_with(&format!("{}=", f.key)));
    let mut lines: Vec<String> = before.lines().filter(|l| !ours(l)).map(str::to_owned).collect();
    for (field, value) in fields.iter().zip(values) {
        let quoted = value.0.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`");
        lines.push(format!("{}=\"{quoted}\"", field.key));
    }
    let text = lines.join("\n") + "\n";
    let draft = path.with_extension("equipe-draft");
    {
        use std::io::Write as _;
        let mut options = std::fs::OpenOptions::new(); options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        { use std::os::unix::fs::OpenOptionsExt as _; options.mode(0o600); }
        let mut file = options.open(&draft).map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt as _; std::fs::set_permissions(&draft, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?; }
    std::fs::rename(&draft, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-secrets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    /// What the charters stopped saying because the tool says it (crates/core/charter-moved.txt).
    #[test]
    fn ask_secrets_description_says_the_person_types_it_into_a_card_never_in_words() {
        let said = tool()["description"].as_str().unwrap().to_owned();
        for part in ["They type it into a card in equipe", "keeps it in this machine's secret store", "Never ask for one in words", "never have the person run a command to type it", "never print, echo or copy a value"] {
            assert!(said.contains(part), "{part}: {said}");
        }
    }

    #[test]
    fn a_request_names_its_keys_and_a_file_of_its_own_under_home() {
        let home = home("ask");
        let args = json!({ "why": "NAS 자동 수집", "save_to": "~/.config/atinum/nas.env", "no_page": true, "steps": ["Look on the NAS label"], "fields": [
            { "key": "NAS_USER", "label": "NAS 아이디" }, { "key": "NAS_PASSWORD", "label": "비밀번호", "secret": true } ] });
        let asked = check(&args, &home).unwrap();
        assert_eq!(asked.shown, "~/.config/atinum/nas.env");
        assert_eq!(asked.path, Some(home.join(".config/atinum/nas.env")));
        assert_eq!(asked.fields.iter().map(|f| (f.key.as_str(), f.secret)).collect::<Vec<_>>(), [("NAS_USER", false), ("NAS_PASSWORD", true)]);
        let with = |save_to: &str| check(&json!({ "why": "x", "save_to": save_to, "no_page": true, "steps": ["Find it"], "fields": [{ "key": "K", "label": "k" }] }), &home);
        for bad in ["~/.ssh/config", "~/.equipe/vault.json", "/etc/passwd", "~/../x.env", "relative.env", "~/notes.env"] {
            assert!(with(bad).is_err(), "{bad:?}");
        }
        // No file named: the secret store alone.
        let kept = check(&json!({ "why": "x", "no_page": true, "steps": ["Find it"], "fields": [{ "key": "K", "label": "k" }] }), &home).unwrap();
        assert_eq!((kept.path, kept.shown.as_str()), (None, ""));
        assert!(with("~/.atinum.env").is_ok(), "a dotfile of its own in home");
        assert!(check(&json!({ "why": "x", "save_to": "~/.config/a.env", "no_page": true, "steps": ["Find it"], "fields": [{ "key": "lower", "label": "k" }] }), &home).is_err());
        assert!(check(&json!({ "why": "", "save_to": "~/.config/a.env", "fields": [{ "key": "K" }] }), &home).is_err());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_card_never_comes_bare_steps_and_a_page_are_required() {
        let home = home("guide");
        let fields = json!([{ "key": "ANTHROPIC_ADMIN_KEY", "label": "Admin key", "secret": true }]);
        let ask = |extra: Value| {
            let mut args = json!({ "why": "usage reports", "fields": fields });
            args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            check(&args, &home)
        };
        // No steps: refused, and the error says what to add.
        let none = ask(json!({ "url": "https://platform.claude.com/settings/admin-keys" })).unwrap_err();
        assert!(none.contains("`steps`") && none.contains("numbered"), "{none}");
        assert!(ask(json!({ "steps": [], "url": "https://platform.claude.com/x" })).is_err());
        assert!(ask(json!({ "steps": ["  "], "url": "https://platform.claude.com/x" })).is_err(), "blank lines are not steps");
        assert!(ask(json!({ "steps": (0..9).map(|n| format!("step {n}")).collect::<Vec<_>>(), "url": "https://platform.claude.com/x" })).is_err(), "at most 8");
        // A website's secret without its page: refused, naming `url` and `no_page`.
        let bare = ask(json!({ "steps": ["Open the console"] })).unwrap_err();
        assert!(bare.contains("`url`") && bare.contains("no_page"), "{bare}");
        // The page is checked like request_secret's.
        for bad in ["http://acme.example.com", "javascript:alert(1)", "file:///etc/passwd", "https://u:p@acme.example.com", "https://acme.example.com:8443/", "https://localhost/", "https://127.0.0.1/"] {
            assert!(ask(json!({ "steps": ["Open it"], "url": bad })).is_err(), "{bad}");
        }
        // The member's page and steps are kept as given.
        let own = ask(json!({ "steps": ["Open  the   page", "Click Create"], "url": "https://acme.example.com/keys" })).unwrap();
        assert_eq!((own.guide.steps.as_slice(), own.guide.url.as_deref()), (&["Open the page".to_owned(), "Click Create".to_owned()][..], Some("https://acme.example.com/keys")));
        // A catalog service with no url: its key page, and its sign-in page as a second link.
        let filled = ask(json!({ "steps": ["Open Admin keys"], "service": "anthropic" })).unwrap();
        assert_eq!(filled.guide.url.as_deref(), Some("https://platform.claude.com/settings/admin-keys"));
        assert_eq!(filled.guide.sign_in_url.as_deref(), Some("https://platform.claude.com/"));
        // No web page at all: allowed, with steps.
        assert!(ask(json!({ "steps": ["Read it off the NAS label"], "no_page": true })).unwrap().guide.url.is_none());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn values_land_in_their_file_for_its_owner_alone_and_other_lines_stay() {
        use std::os::unix::fs::PermissionsExt as _;
        let home = home("write");
        let path = home.join(".config/atinum/nas.env");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "NAS_HOST=\"10.0.0.2\"\nNAS_PASSWORD=\"old\"\n").unwrap();
        let fields = vec![SecretField { key: "NAS_USER".into(), label: "id".into(), secret: false }, SecretField { key: "NAS_PASSWORD".into(), label: "pw".into(), secret: true }];
        write(&path, &fields, &[SecretValue("ai".into()), SecretValue("p\"a$s`s".into())]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "NAS_HOST=\"10.0.0.2\"\nNAS_USER=\"ai\"\nNAS_PASSWORD=\"p\\\"a\\$s\\`s\"\n");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(write(&path, &fields, &[SecretValue("a\nb".into()), SecretValue("x".into())]).is_err());
        assert!(write(&path, &fields, &[SecretValue("a".into())]).is_err(), "one for each");
        std::fs::remove_dir_all(home).unwrap();
    }
}
