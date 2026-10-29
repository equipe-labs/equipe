//! `ask_secret`: a member needs a password, a key or a token for a command or
//! a config, and the person types it into a card instead of a conversation
//! (see `equipe_protocol::secrets`). Checked here — the keys, how many, and
//! where it may be saved: a file under the person's home, never one of the
//! places keys and logins live (`~/.ssh`, equipe's own) — and written there as
//! `KEY="value"` lines, readable by the person alone. The values go to the
//! file and nowhere else: not the member, not an event, not a log.

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
a database password. They type it into a card in equipe, and equipe saves it straight to a file for you (`save_to`, \
`KEY=\"value\"` lines, readable only by them): you never see the values, and they never pass through the conversation. \
You hear where it is once they save it — then use it from there (read the file in your code, or `source` it in a \
command), and never print, echo or copy a value into chat, tickets, commits or logs. Never ask for one in words, and \
never have the person run a command to type it. A website sign-in is `sign_in`, not this.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "why": { "type": "string", "description": "What it is for, in a line in the person's language: \"NAS 매일 자동 수집에 쓸 NAS 로그인\"." },
                "fields": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "key": { "type": "string", "description": "The key it is saved under: NAS_USER (uppercase, digits, _)." },
                            "label": { "type": "string", "description": "How the card names it, in the person's language: \"NAS 아이디\"." },
                            "secret": { "type": "boolean", "description": "Typed hidden: a password, key or token. False for a username." },
                        },
                        "required": ["key", "label"],
                    },
                    "description": "What to ask for, 1 to 6.",
                },
                "save_to": { "type": "string", "description": "The file to save it to, under the person's home: `~/.config/atinum/nas.env`. Keys already in it are replaced, the rest kept." },
            },
            "required": ["why", "fields", "save_to"],
        },
    })
}

/// A call, checked: what the card asks and where it saves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub why: String,
    pub fields: Vec<SecretField>,
    /// The file, whole.
    pub path: PathBuf,
    /// The file as the person reads it: `~/.config/atinum/nas.env`.
    pub shown: String,
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
    let (path, shown) = place(&text("save_to"), home)?;
    Ok(Asked { why, fields, path, shown })
}

fn valid_key(key: &str) -> bool {
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
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
    if values.len() != fields.len() {
        return Err(format!("{} values for {} fields", values.len(), fields.len()));
    }
    if values.iter().any(|v| v.0.contains(['\n', '\r', '\0'])) {
        return Err("a value can't hold a line break".into());
    }
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| e.to_string())?;
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
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&draft).map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    std::fs::set_permissions(&draft, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
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

    #[test]
    fn a_request_names_its_keys_and_a_file_of_its_own_under_home() {
        let home = home("ask");
        let args = json!({ "why": "NAS 자동 수집", "save_to": "~/.config/atinum/nas.env", "fields": [
            { "key": "NAS_USER", "label": "NAS 아이디" }, { "key": "NAS_PASSWORD", "label": "비밀번호", "secret": true } ] });
        let asked = check(&args, &home).unwrap();
        assert_eq!(asked.shown, "~/.config/atinum/nas.env");
        assert_eq!(asked.path, home.join(".config/atinum/nas.env"));
        assert_eq!(asked.fields.iter().map(|f| (f.key.as_str(), f.secret)).collect::<Vec<_>>(), [("NAS_USER", false), ("NAS_PASSWORD", true)]);
        let with = |save_to: &str| check(&json!({ "why": "x", "save_to": save_to, "fields": [{ "key": "K", "label": "k" }] }), &home);
        for bad in ["~/.ssh/config", "~/.equipe/vault.json", "/etc/passwd", "~/../x.env", "relative.env", "~/notes.env", ""] {
            assert!(with(bad).is_err(), "{bad:?}");
        }
        assert!(with("~/.atinum.env").is_ok(), "a dotfile of its own in home");
        assert!(check(&json!({ "why": "x", "save_to": "~/.config/a.env", "fields": [{ "key": "lower", "label": "k" }] }), &home).is_err());
        assert!(check(&json!({ "why": "", "save_to": "~/.config/a.env", "fields": [{ "key": "K" }] }), &home).is_err());
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
