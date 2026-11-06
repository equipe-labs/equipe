//! Model providers the person connects — OpenRouter, Groq, Mistral, a local
//! Ollama… — for members to run on, beside Claude Code and Codex: each an
//! OpenAI-compatible API, its key kept on the daemon's machine as the Nebius
//! key is (`keys/<id>` under equipe's home, readable by the person alone),
//! and its members run on Codex's agent in plain mode, as Nebius's do
//! ([`crate::nemotron`], `codex_app::provider_args`).
//!
//! - [`PRESETS`]: the providers equipe knows, with their API's root as their
//!   own documentation gives it; and `other`, any OpenAI-compatible server.
//! - [`entries`]: the providers connected, as kept in `providers.json` under
//!   equipe's home (no key in it), Nebius's key from before
//!   ([`crate::nemotron::key_path`]) as the Nebius entry, and the
//!   environment's `EQUIPE_MODEL_BASE_URL` as one more (`custom`).
//! - [`add`], [`remove`]: from a screen ([`equipe_protocol::Command::AddProvider`]).
//! - [`config`]: a provider as a [`crate::nemotron::Config`], what a member's
//!   turns and the listing of its models ask with.
//! - A member's model on one reads `@<provider>/<model>` ([`model_ref`],
//!   [`split_ref`]); an open model without one (`nvidia/…`) runs where it
//!   always has: the environment's server, else Nebius.
//!
//! A key is never logged, shown, put in a URL or on a command line: it goes
//! to the provider as a bearer and to Codex by its environment only.

use std::path::{Path, PathBuf};

use equipe_protocol::ProviderKey;
use equipe_providers::relay::Wire;
use serde::{Deserialize, Serialize};

use crate::nemotron::{self, ReviewError};

/// A provider equipe knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    /// Its id: in `keys/<id>`, in a member's model (`@<id>/…`).
    pub id: &'static str,
    /// Its own name, the same in every language.
    pub name: &'static str,
    /// Its OpenAI-compatible API's root, before `chat/completions` and
    /// `models`; empty for `other`, whose root the person gives.
    pub base_url: &'static str,
    /// Whether it needs a key: a server on the person's machine doesn't.
    pub needs_key: bool,
    /// The variable its own documentation keeps the key in: set in the
    /// daemon's environment, it wins over the saved key.
    pub env_var: Option<&'static str>,
    /// Where the person makes a key.
    pub key_page: Option<&'static str>,
    /// Whether it runs on the person's machine.
    pub local: bool,
}

/// The id of any other OpenAI-compatible server.
pub const OTHER: &str = "other";
/// The id of `EQUIPE_MODEL_BASE_URL`'s server ([`nemotron::CUSTOM_PROVIDER`]).
pub const CUSTOM: &str = nemotron::CUSTOM_PROVIDER;
/// Nebius Token Factory's id, the key's file as before.
pub const NEBIUS: &str = "nebius";

/// The providers a screen offers, in its order. Each base URL is the one
/// the provider's own documentation gives for its OpenAI-compatible API;
/// all are asked in Chat Completions, which every one documents.
pub const PRESETS: [Preset; 12] = [
    Preset { id: "openrouter", name: "OpenRouter", base_url: "https://openrouter.ai/api/v1", needs_key: true, env_var: Some("OPENROUTER_API_KEY"), key_page: Some("https://openrouter.ai/keys"), local: false },
    Preset { id: NEBIUS, name: nemotron::PROVIDER_NAME, base_url: nemotron::DEFAULT_BASE_URL, needs_key: true, env_var: Some("NEBIUS_API_KEY"), key_page: Some("https://tokenfactory.nebius.com"), local: false },
    Preset { id: "together", name: "Together AI", base_url: "https://api.together.ai/v1", needs_key: true, env_var: Some("TOGETHER_API_KEY"), key_page: Some("https://api.together.ai/settings/api-keys"), local: false },
    Preset { id: "fireworks", name: "Fireworks AI", base_url: "https://api.fireworks.ai/inference/v1", needs_key: true, env_var: Some("FIREWORKS_API_KEY"), key_page: Some("https://app.fireworks.ai/settings/users/api-keys"), local: false },
    Preset { id: "groq", name: "Groq", base_url: "https://api.groq.com/openai/v1", needs_key: true, env_var: Some("GROQ_API_KEY"), key_page: Some("https://console.groq.com/keys"), local: false },
    Preset { id: "deepseek", name: "DeepSeek", base_url: "https://api.deepseek.com", needs_key: true, env_var: Some("DEEPSEEK_API_KEY"), key_page: Some("https://platform.deepseek.com/api_keys"), local: false },
    Preset { id: "mistral", name: "Mistral AI", base_url: "https://api.mistral.ai/v1", needs_key: true, env_var: Some("MISTRAL_API_KEY"), key_page: Some("https://console.mistral.ai/api-keys"), local: false },
    Preset { id: "xai", name: "xAI", base_url: "https://api.x.ai/v1", needs_key: true, env_var: Some("XAI_API_KEY"), key_page: Some("https://console.x.ai"), local: false },
    Preset { id: "gemini", name: "Google Gemini", base_url: "https://generativelanguage.googleapis.com/v1beta/openai/", needs_key: true, env_var: Some("GEMINI_API_KEY"), key_page: Some("https://aistudio.google.com/apikey"), local: false },
    Preset { id: "ollama", name: "Ollama", base_url: "http://localhost:11434/v1", needs_key: false, env_var: None, key_page: None, local: true },
    Preset { id: "lmstudio", name: "LM Studio", base_url: "http://localhost:1234/v1", needs_key: false, env_var: None, key_page: None, local: true },
    Preset { id: OTHER, name: "Other (OpenAI-compatible)", base_url: "", needs_key: true, env_var: None, key_page: None, local: false },
];

/// The preset `id`.
pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// A provider connected on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    /// The preset it comes from; [`CUSTOM`] for the environment's server.
    pub preset: String,
    pub name: String,
    pub base_url: String,
    pub key: ProviderKey,
    /// Whether a screen can remove it: not the environment's.
    pub removable: bool,
}

/// What `providers.json` keeps of a provider: never its key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    id: String,
    preset: String,
    /// Another root than the preset's (a local server on another port), or `other`'s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    providers: Vec<Kept>,
}

/// Where the providers are kept: their ids and roots, no key.
pub fn list_path(home: &Path) -> PathBuf {
    home.join("providers.json")
}

/// Where a provider's key is kept: `keys/<id>`, Nebius's where it always was.
pub fn key_path(home: &Path, id: &str) -> PathBuf {
    home.join("keys").join(id)
}

/// The saved key of provider `id`, if there is one.
pub fn saved_key(home: &Path, id: &str) -> Option<String> {
    std::fs::read_to_string(key_path(home, id)).ok().map(|k| k.trim().to_owned()).filter(|k| !k.is_empty())
}

/// Save `key` for provider `id`, readable and writable by the person alone (the folder too).
pub fn save_key(home: &Path, id: &str, key: &str) -> std::io::Result<()> {
    let path = key_path(home, id);
    let dir = path.parent().unwrap_or(home);
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{id}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        std::io::Write::write_all(&mut file, key.trim().as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, key.trim())?;
    std::fs::rename(&tmp, &path)
}

/// Forget provider `id`'s saved key. Whether there was one.
pub fn forget_key(home: &Path, id: &str) -> std::io::Result<bool> {
    match std::fs::remove_file(key_path(home, id)) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

fn read(home: &Path) -> File {
    std::fs::read_to_string(list_path(home)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn write(home: &Path, file: &File) -> std::io::Result<()> {
    std::fs::create_dir_all(home)?;
    let text = serde_json::to_string_pretty(file).map_err(std::io::Error::other)?;
    let tmp = home.join(".providers.json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, list_path(home))
}

/// Whether a base URL is a server on this machine.
pub fn is_loopback(base_url: &str) -> bool {
    reqwest::Url::parse(base_url.trim()).ok().and_then(|u| u.host_str().map(str::to_owned)).is_some_and(|h| h == "localhost" || h == "[::1]" || h == "::1" || h.starts_with("127."))
}

/// The variable a provider's key reaches Codex by: its own documentation's,
/// else one named for it.
fn key_var(id: &str, preset: Option<&Preset>) -> String {
    match preset.and_then(|p| p.env_var) {
        Some(var) => var.to_owned(),
        None => format!("EQUIPE_{}_API_KEY", id.to_ascii_uppercase().replace(|c: char| !c.is_ascii_alphanumeric(), "_")),
    }
}

/// Where `kept`'s key comes from, `var` reading the environment.
fn key_source(home: &Path, kept: &Kept, var: &impl Fn(&str) -> Option<String>) -> ProviderKey {
    let preset = preset(&kept.preset);
    let from_env = preset.and_then(|p| p.env_var).and_then(var).is_some_and(|k| !k.trim().is_empty());
    let base = kept.base_url.as_deref().or(preset.map(|p| p.base_url)).unwrap_or_default();
    if from_env {
        ProviderKey::Environment
    } else if saved_key(home, &kept.id).is_some() {
        ProviderKey::Saved
    } else if preset.is_some_and(|p| !p.needs_key) || is_loopback(base) {
        ProviderKey::NotNeeded
    } else {
        ProviderKey::None
    }
}

/// The providers connected on this machine, `var` reading the environment:
/// the environment's server first (`EQUIPE_MODEL_BASE_URL`), then those
/// kept, in the order they were added — Nebius among them when its key is
/// saved or in the environment, as it was before there was a list.
pub fn entries(home: &Path, var: impl Fn(&str) -> Option<String>) -> Vec<Entry> {
    let set = |name: &str| var(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let mut out = Vec::new();
    if let Some(base_url) = set("EQUIPE_MODEL_BASE_URL") {
        let name = reqwest::Url::parse(&base_url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_else(|| base_url.clone());
        let key = if set("EQUIPE_MODEL_API_KEY").is_some() { ProviderKey::Environment } else if is_loopback(&base_url) { ProviderKey::NotNeeded } else { ProviderKey::None };
        out.push(Entry { id: CUSTOM.to_owned(), preset: CUSTOM.to_owned(), name, base_url, key, removable: false });
    }
    let mut kept = read(home).providers;
    kept.retain(|k| preset(&k.preset).is_some() && k.id != CUSTOM);
    // Nebius's key from before the list: its entry.
    if !kept.iter().any(|k| k.id == NEBIUS) && (saved_key(home, NEBIUS).is_some() || set("NEBIUS_API_KEY").is_some()) {
        kept.insert(0, Kept { id: NEBIUS.to_owned(), preset: NEBIUS.to_owned(), base_url: None });
    }
    for k in kept {
        let Some(preset) = preset(&k.preset) else { continue };
        let base_url = k.base_url.clone().filter(|b| !b.trim().is_empty()).unwrap_or_else(|| preset.base_url.to_owned());
        let name = if preset.id == OTHER { host(&base_url) } else { preset.name.to_owned() };
        let key = key_source(home, &k, &var);
        out.push(Entry { id: k.id, preset: preset.id.to_owned(), name, base_url, key, removable: true });
    }
    out
}

/// A URL's host (and port), as a provider's name: `vllm.example.com`.
fn host(base_url: &str) -> String {
    match reqwest::Url::parse(base_url) {
        Ok(url) => match (url.host_str(), url.port()) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            (Some(host), None) => host.to_owned(),
            _ => base_url.to_owned(),
        },
        Err(_) => base_url.to_owned(),
    }
}

/// Why a provider couldn't be added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddError {
    /// No such preset.
    Unknown,
    /// The base URL isn't one: `http(s)://…`.
    BadUrl,
    /// A key would go to a server elsewhere unencrypted: `http://` off this machine.
    Insecure,
    /// It needs a key and none was given.
    NoKey,
    /// What was pasted doesn't look like a key.
    BadKey,
    /// The file couldn't be written.
    Io(String),
}

impl AddError {
    /// Why, in the person's language.
    pub fn why(&self, lang: crate::i18n::Lang, provider: &str) -> String {
        let words = crate::i18n::core(lang);
        let key = match self {
            AddError::Unknown => "provider.add.unknown",
            AddError::BadUrl => "provider.add.bad_url",
            AddError::Insecure => "provider.add.insecure",
            AddError::NoKey => "provider.add.no_key",
            AddError::BadKey => "provider.add.bad_key",
            AddError::Io(why) => return words.fill("provider.add.io", &[("provider", provider.to_owned()), ("why", why.clone())]),
        };
        words.fill(key, &[("provider", provider.to_owned())])
    }
}

/// The id `other`'s server at `base_url` gets: `other-<host>`, made unique.
fn other_id(base_url: &str, taken: &[Kept]) -> String {
    let slug: String = host(base_url).to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let slug = slug.trim_matches('-').chars().take(40).collect::<String>();
    let base = if slug.is_empty() { OTHER.to_owned() } else { format!("{OTHER}-{slug}") };
    let mut id = base.clone();
    let mut n = 2;
    while taken.iter().any(|k| k.id == id && k.base_url.as_deref() != Some(base_url)) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Whether `preset` at `base_url` needs a key: one elsewhere does, a server on this machine doesn't.
pub fn needs_key(preset: &Preset, base_url: &str) -> bool {
    preset.needs_key && !is_loopback(base_url)
}

/// What a screen's form gives, looked at before anything is kept: the
/// preset, its root (`other`'s or another port's), and its key — required
/// when it [`needs_key`]. The preset and the root used.
pub fn check(preset_id: &str, base_url: Option<&str>, key: Option<&str>) -> Result<(&'static Preset, String), AddError> {
    let (preset, base) = checked(preset_id, base_url, key)?;
    if needs_key(preset, &base) && key.is_none_or(|k| k.trim().is_empty()) {
        return Err(AddError::NoKey);
    }
    Ok((preset, base))
}

fn checked(preset_id: &str, base_url: Option<&str>, key: Option<&str>) -> Result<(&'static Preset, String), AddError> {
    let preset = preset(preset_id).ok_or(AddError::Unknown)?;
    let given = base_url.map(str::trim).filter(|b| !b.is_empty());
    let base = match (preset.id, given) {
        (OTHER, None) => return Err(AddError::BadUrl),
        (_, Some(url)) => url.to_owned(),
        (_, None) => preset.base_url.to_owned(),
    };
    let parsed = reqwest::Url::parse(&base).map_err(|_| AddError::BadUrl)?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() || !parsed.username().is_empty() || parsed.password().is_some() || parsed.query().is_some() {
        return Err(AddError::BadUrl);
    }
    if parsed.scheme() == "http" && !is_loopback(&base) {
        return Err(AddError::Insecure);
    }
    if let Some(key) = key.map(str::trim).filter(|k| !k.is_empty())
        && !nemotron::key_is_plausible(key)
    {
        return Err(AddError::BadKey);
    }
    Ok((preset, base))
}

/// Connect a provider: `preset`, at `base_url` (`other`'s; a local server's
/// other port), with `key` when it needs one — saved as [`save_key`] does.
/// Added again, its key and root are replaced. Its id.
pub fn add(home: &Path, preset_id: &str, base_url: Option<&str>, key: Option<&str>) -> Result<String, AddError> {
    let (preset, base) = checked(preset_id, base_url, key)?;
    let given = base_url.map(str::trim).filter(|b| !b.is_empty());
    let key = key.map(str::trim).filter(|k| !k.is_empty());
    let mut file = read(home);
    let id = if preset.id == OTHER { other_id(&base, &file.providers) } else { preset.id.to_owned() };
    if needs_key(preset, &base) && key.is_none() && saved_key(home, &id).is_none() {
        return Err(AddError::NoKey);
    }
    let io = |err: std::io::Error| AddError::Io(err.to_string());
    if let Some(key) = key {
        save_key(home, &id, key).map_err(io)?;
    }
    let kept_url = (given.is_some() && base != preset.base_url).then(|| base.clone());
    file.providers.retain(|k| k.id != id);
    file.providers.push(Kept { id: id.clone(), preset: preset.id.to_owned(), base_url: kept_url });
    write(home, &file).map_err(io)?;
    Ok(id)
}

/// Forget provider `id` and its saved key. Whether it was there.
pub fn remove(home: &Path, id: &str) -> std::io::Result<bool> {
    let mut file = read(home);
    let before = file.providers.len();
    file.providers.retain(|k| k.id != id);
    let listed = file.providers.len() != before;
    if listed {
        write(home, &file)?;
    }
    let had_key = forget_key(home, id)?;
    Ok(listed || had_key)
}

/// The id Codex knows a provider by in a member's turns: Nebius's as
/// always; another's with equipe's mark, so it never stands for one of
/// Codex's own (`ollama`, `lmstudio`), whose settings it would take instead.
pub fn codex_id(id: &str) -> String {
    match id {
        NEBIUS | CUSTOM => id.to_owned(),
        _ => format!("equipe_{}", id.replace(|c: char| !c.is_ascii_alphanumeric(), "_")),
    }
}

/// Provider `id` as a member's turns and its listing ask it, `var` reading
/// the environment: its key from there first, else saved; none for a
/// server on this machine. Nebius and the environment's server as
/// [`nemotron::Config::from_vars`] reads them, so they behave as they did.
pub fn config(home: &Path, id: &str, var: impl Fn(&str) -> Option<String>) -> Result<nemotron::Config, ReviewError> {
    let set = |name: &str| var(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    match id {
        CUSTOM => {
            set("EQUIPE_MODEL_BASE_URL").ok_or(ReviewError::MissingKey)?;
            return nemotron::Config::from_vars(&var);
        }
        NEBIUS => {
            let saved = saved_key(home, NEBIUS);
            return nemotron::Config::from_vars(|name| match name {
                // Nebius itself, whatever other server the environment names.
                "EQUIPE_MODEL_BASE_URL" => None,
                "NEBIUS_API_KEY" => set(name).or_else(|| saved.clone()),
                _ => var(name),
            });
        }
        _ => {}
    }
    let entry = entries(home, &var).into_iter().find(|e| e.id == id).ok_or(ReviewError::MissingKey)?;
    let preset = preset(&entry.preset);
    let key = match entry.key {
        ProviderKey::Environment => preset.and_then(|p| p.env_var).and_then(set),
        ProviderKey::Saved => saved_key(home, id),
        // Any key does for a server on this machine; one is sent all the same.
        ProviderKey::NotNeeded => saved_key(home, id).or_else(|| Some("local".to_owned())),
        ProviderKey::None => None,
    }
    .ok_or(ReviewError::MissingKey)?;
    Ok(nemotron::Config::connected(key, &key_var(id, preset), &codex_id(id), &entry.name, &entry.base_url, Wire::Chat))
}

// ── a member's model on a provider ──

/// A model on provider `id`, as a member is set to it: `@openrouter/qwen/qwen3-coder`.
pub fn model_ref(id: &str, model: &str) -> String {
    format!("@{id}/{}", model.trim())
}

/// A model set as [`model_ref`] makes it: its provider and the model's own id.
pub fn split_ref(model: &str) -> Option<(&str, &str)> {
    let (id, model) = model.trim().strip_prefix('@')?.split_once('/')?;
    (!id.is_empty() && !model.trim().is_empty()).then_some((id, model))
}

/// The model's own id, without the provider it is on: what the provider is asked for.
pub fn bare_model(model: &str) -> &str {
    split_ref(model).map_or(model.trim(), |(_, m)| m)
}

/// A model's id as a person reads it: without the provider, nor Gemini's `models/`.
pub fn display(model: &str) -> &str {
    let bare = bare_model(model);
    bare.strip_prefix("models/").unwrap_or(bare)
}

/// Of a provider's listed ids, those members can work with: no embeddings,
/// speech, images, rerankers or safety filters — models that only answer
/// in other forms than text a member can act on.
pub fn chat_models(ids: &[String]) -> Vec<String> {
    const NOT_CHAT: [&str; 22] = [
        "embed", "rerank", "whisper", "tts", "transcri", "speech", "audio", "moderation", "guard", "safety", "reward", "dall-e", "stable-diffusion", "sdxl", "flux", "imagen", "image", "veo", "lyria", "bge-", "-aqa", "retriever",
    ];
    let mut out: Vec<String> = Vec::new();
    for id in ids.iter().map(|id| id.trim()).filter(|id| !id.is_empty()) {
        let low = id.to_ascii_lowercase();
        if NOT_CHAT.iter().any(|w| low.contains(w)) || out.iter().any(|o| o == id) {
            continue;
        }
        out.push(id.to_owned());
    }
    out
}

/// Provider `id`'s models, best first, as `GET /models` lists them, for
/// members: on Nebius its NVIDIA Nemotron chat models first.
pub async fn list(cfg: &nemotron::Config, id: &str) -> Result<Vec<String>, ReviewError> {
    let listed = chat_models(&nemotron::models(cfg).await?);
    if listed.is_empty() {
        return Err(ReviewError::NoModel);
    }
    if id != NEBIUS {
        return Ok(listed);
    }
    let mut best = nemotron::nemotron_models(&listed);
    best.extend(listed.into_iter().filter(|m| !best.contains(m)).collect::<Vec<_>>());
    Ok(best)
}

/// Why a provider's models couldn't be listed, in plain words, naming it.
pub fn why(err: &ReviewError, provider: &str, base_url: &str, lang: crate::i18n::Lang) -> String {
    let words = crate::i18n::core(lang);
    let args = |extra: Option<(&'static str, String)>| {
        let mut args = vec![("provider", provider.to_owned()), ("url", base_url.to_owned())];
        args.extend(extra);
        args
    };
    match err {
        ReviewError::MissingKey => words.fill("provider.why.missing_key", &args(None)),
        ReviewError::Unauthorized => words.fill("provider.why.unauthorized", &args(None)),
        ReviewError::RateLimited => words.fill("provider.why.rate_limited", &args(None)),
        ReviewError::Timeout => words.fill("provider.why.timeout", &args(None)),
        ReviewError::NoModel => words.fill("provider.why.no_models", &args(None)),
        ReviewError::Http(404, _) => words.fill("provider.why.not_found", &args(None)),
        ReviewError::Http(status, _) => words.fill("provider.why.http", &args(Some(("status", status.to_string())))),
        ReviewError::BadOutput => words.fill("provider.why.bad_output", &args(None)),
        ReviewError::Network(_) | ReviewError::Diff(_) | ReviewError::EmptyDiff => words.fill("provider.why.network", &args(None)),
    }
}

#[cfg(test)]
mod tests {
    //! On a home of the test's own, against a stand-in server on localhost:
    //! nothing here reaches a provider, and no key is real.

    use super::*;

    fn home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("equipe-providers-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    fn none(_: &str) -> Option<String> {
        None
    }

    fn vars(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |n: &str| pairs.iter().find(|(k, _)| *k == n).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn every_preset_has_its_documented_root_and_says_whether_it_needs_a_key() {
        let ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        for wanted in ["openrouter", "nebius", "together", "fireworks", "groq", "deepseek", "mistral", "xai", "gemini", "ollama", "lmstudio", "other"] {
            assert!(ids.contains(&wanted), "{wanted}");
        }
        let mut seen = ids.clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), ids.len(), "ids are unique");
        for p in PRESETS {
            assert!(!p.id.contains('/') && !p.id.starts_with('@'), "{}", p.id);
            if p.id == OTHER {
                assert!(p.base_url.is_empty());
                continue;
            }
            let url = reqwest::Url::parse(p.base_url).unwrap();
            // A key never goes unencrypted to a server elsewhere.
            assert_eq!(url.scheme() == "https", !p.local, "{}", p.id);
            assert_eq!(p.needs_key, !p.local, "{}", p.id);
            assert_eq!(p.key_page.is_some(), p.needs_key, "{}", p.id);
        }
        assert_eq!(preset(NEBIUS).unwrap().base_url, nemotron::DEFAULT_BASE_URL);
        assert_eq!(preset("groq").unwrap().base_url, "https://api.groq.com/openai/v1");
        assert_eq!(preset("gemini").unwrap().base_url, "https://generativelanguage.googleapis.com/v1beta/openai/");
        assert!(preset("nope").is_none());
    }

    #[test]
    fn a_model_on_a_provider_names_it_and_keeps_its_own_id_whole() {
        assert_eq!(model_ref("openrouter", "qwen/qwen3-coder"), "@openrouter/qwen/qwen3-coder");
        assert_eq!(split_ref("@openrouter/qwen/qwen3-coder"), Some(("openrouter", "qwen/qwen3-coder")));
        assert_eq!(split_ref("@ollama/qwen3-coder:30b"), Some(("ollama", "qwen3-coder:30b")));
        assert_eq!(split_ref("@gemini/models/gemini-2.5-pro"), Some(("gemini", "models/gemini-2.5-pro")));
        for not in ["qwen/qwen3-coder", "nvidia/x", "opus", "@", "@x", "@x/", "@/y"] {
            assert_eq!(split_ref(not), None, "{not}");
        }
        assert_eq!(bare_model("@groq/llama-3.3-70b-versatile"), "llama-3.3-70b-versatile");
        assert_eq!(bare_model("nvidia/x"), "nvidia/x");
        assert_eq!(display("@gemini/models/gemini-2.5-pro"), "gemini-2.5-pro");
        // An open model either way: it runs on Codex's agent.
        assert!(nemotron::is_agent_model(&model_ref("deepseek", "deepseek-chat")));
        assert!(nemotron::is_agent_model(&model_ref("mistral", "devstral-medium-latest")));
    }

    #[test]
    fn an_added_provider_keeps_its_key_to_the_person_alone_and_none_in_its_list() {
        let home = home("add");
        assert!(entries(&home, none).is_empty());
        let id = add(&home, "openrouter", None, Some("  sk-or-v1-secret-123 ")).unwrap();
        assert_eq!(id, "openrouter");
        assert_eq!(saved_key(&home, "openrouter").as_deref(), Some("sk-or-v1-secret-123"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(key_path(&home, "openrouter")).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(home.join("keys")).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let list = std::fs::read_to_string(list_path(&home)).unwrap();
        assert!(!list.contains("secret"), "the list keeps no key: {list}");
        let all = entries(&home, none);
        assert_eq!(all, vec![Entry { id: "openrouter".into(), preset: "openrouter".into(), name: "OpenRouter".into(), base_url: "https://openrouter.ai/api/v1".into(), key: ProviderKey::Saved, removable: true }]);
        // Its environment variable wins over the saved key.
        assert_eq!(entries(&home, vars(&[("OPENROUTER_API_KEY", "sk-or-env-456")]))[0].key, ProviderKey::Environment);
        let cfg = config(&home, "openrouter", vars(&[("OPENROUTER_API_KEY", "sk-or-env-456")])).unwrap();
        assert_eq!(cfg.codex_env(), ("OPENROUTER_API_KEY".to_owned(), "sk-or-env-456".to_owned()));
        // Added again: the key replaced, listed once.
        add(&home, "openrouter", None, Some("sk-or-v1-other-789")).unwrap();
        assert_eq!(entries(&home, none).len(), 1);
        assert_eq!(saved_key(&home, "openrouter").as_deref(), Some("sk-or-v1-other-789"));
        // Added again with no key: the saved one stays.
        add(&home, "openrouter", None, None).unwrap();
        assert_eq!(saved_key(&home, "openrouter").as_deref(), Some("sk-or-v1-other-789"));
        assert!(remove(&home, "openrouter").unwrap());
        assert!(entries(&home, none).is_empty() && saved_key(&home, "openrouter").is_none());
        assert!(!remove(&home, "openrouter").unwrap());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn what_cant_be_added_says_why() {
        let home = home("refuse");
        assert_eq!(add(&home, "nope", None, Some("sk-123456789")), Err(AddError::Unknown));
        assert_eq!(add(&home, "groq", None, None), Err(AddError::NoKey));
        assert_eq!(add(&home, "groq", None, Some("two words")), Err(AddError::BadKey));
        assert_eq!(add(&home, OTHER, None, Some("sk-123456789")), Err(AddError::BadUrl));
        assert_eq!(add(&home, OTHER, Some("not a url"), Some("sk-123456789")), Err(AddError::BadUrl));
        assert_eq!(add(&home, OTHER, Some("ftp://example.com/v1"), Some("sk-123456789")), Err(AddError::BadUrl));
        // A key is never put in a URL.
        assert_eq!(add(&home, OTHER, Some("https://me:sk-123456789@example.com/v1"), None), Err(AddError::BadUrl));
        assert_eq!(add(&home, OTHER, Some("https://example.com/v1?key=sk-123456789"), None), Err(AddError::BadUrl));
        assert_eq!(add(&home, OTHER, Some("http://vllm.example.com/v1"), Some("sk-123456789")), Err(AddError::Insecure));
        assert!(entries(&home, none).is_empty(), "nothing kept");
        let words = AddError::Insecure.why(crate::i18n::Lang::En, "vllm.example.com");
        assert!(words.contains("https"), "{words}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_server_on_this_machine_needs_no_key_and_another_port_is_kept() {
        let home = home("local");
        assert_eq!(add(&home, "ollama", None, None).unwrap(), "ollama");
        assert_eq!(add(&home, "lmstudio", Some("http://127.0.0.1:4321/v1"), None).unwrap(), "lmstudio");
        let id = add(&home, OTHER, Some("http://localhost:8000/v1"), None).unwrap();
        assert_eq!(id, "other-localhost-8000");
        let all = entries(&home, none);
        let rows: Vec<(&str, &str, ProviderKey)> = all.iter().map(|e| (e.id.as_str(), e.base_url.as_str(), e.key)).collect();
        assert_eq!(
            rows,
            [("ollama", "http://localhost:11434/v1", ProviderKey::NotNeeded), ("lmstudio", "http://127.0.0.1:4321/v1", ProviderKey::NotNeeded), ("other-localhost-8000", "http://localhost:8000/v1", ProviderKey::NotNeeded)]
        );
        assert_eq!(all[2].name, "localhost:8000");
        let cfg = config(&home, "ollama", none).unwrap();
        assert_eq!(cfg.provider, "equipe_ollama", "never Codex's own ollama");
        assert_eq!(cfg.codex_env().0, "EQUIPE_OLLAMA_API_KEY");
        let other = config(&home, &id, none).unwrap();
        assert_eq!((other.provider.as_str(), other.codex_env().0.as_str()), ("equipe_other_localhost_8000", "EQUIPE_OTHER_LOCALHOST_8000_API_KEY"));
        // Another server elsewhere, by its host; one more at the same host gets an id of its own.
        let far = add(&home, OTHER, Some("https://llm.example.com/v1"), Some("sk-far-123456")).unwrap();
        assert_eq!(far, "other-llm-example-com");
        assert_eq!(add(&home, OTHER, Some("https://llm.example.com/v1"), Some("sk-far-654321")).unwrap(), far, "the same server again is the same one");
        assert_eq!(add(&home, OTHER, Some("https://llm.example.com/v2"), Some("sk-far-999999")).unwrap(), "other-llm-example-com-2");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn nebius_s_key_from_before_is_its_entry_and_runs_exactly_as_it_did() {
        let home = home("nebius");
        nemotron::save_key(&home, "sk-nebius-saved-123").unwrap();
        let all = entries(&home, none);
        assert_eq!(all.len(), 1);
        assert_eq!((all[0].id.as_str(), all[0].name.as_str(), all[0].key, all[0].base_url.as_str()), (NEBIUS, "Nebius Token Factory", ProviderKey::Saved, nemotron::DEFAULT_BASE_URL));
        let cfg = config(&home, NEBIUS, none).unwrap();
        let before = nemotron::Config::from_env_or_saved_with(&home, none).unwrap();
        assert_eq!(cfg.codex_provider(), before.codex_provider(), "the same provider Codex had");
        assert_eq!(cfg.codex_env(), before.codex_env());
        // Its key in the environment: an entry too, whether saved or not.
        let fresh = home.join("fresh");
        assert_eq!(entries(&fresh, vars(&[("NEBIUS_API_KEY", "sk-nebius-env")]))[0].key, ProviderKey::Environment);
        // Added from the new list: the same file.
        add(&fresh, NEBIUS, None, Some("sk-nebius-new-456")).unwrap();
        assert_eq!(nemotron::saved_key(&fresh).as_deref(), Some("sk-nebius-new-456"));
        // Removed: the key with it, as forgetting it did.
        assert!(remove(&home, NEBIUS).unwrap());
        assert!(nemotron::saved_key(&home).is_none() && entries(&home, none).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_environments_server_is_one_more_that_a_screen_cant_remove() {
        let home = home("env");
        let env = vars(&[("EQUIPE_MODEL_BASE_URL", "https://openrouter.ai/api/v1"), ("EQUIPE_MODEL_API_KEY", "sk-or-env"), ("NEBIUS_API_KEY", "sk-nebius")]);
        let all = entries(&home, &env);
        assert_eq!(all.iter().map(|e| (e.id.as_str(), e.removable)).collect::<Vec<_>>(), [(CUSTOM, false), (NEBIUS, true)]);
        assert_eq!((all[0].name.as_str(), all[0].key), ("openrouter.ai", ProviderKey::Environment));
        let custom = config(&home, CUSTOM, &env).unwrap();
        assert_eq!(custom.codex_env(), ("EQUIPE_MODEL_API_KEY".to_owned(), "sk-or-env".to_owned()));
        // Nebius by its id is Nebius, whatever server the environment names.
        let nebius = config(&home, NEBIUS, &env).unwrap();
        assert_eq!((nebius.provider.as_str(), nebius.base_url.as_str()), (NEBIUS, nemotron::DEFAULT_BASE_URL));
        assert_eq!(config(&home, CUSTOM, none).unwrap_err(), ReviewError::MissingKey);
        assert_eq!(config(&home, "groq", none).unwrap_err(), ReviewError::MissingKey, "not connected");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_providers_list_keeps_the_models_a_member_can_work_with() {
        let ids = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let kept = chat_models(&ids(&[
            "qwen/qwen3-coder",
            "text-embedding-3-small",
            "whisper-large-v3",
            "meta-llama/llama-guard-4-12b",
            "playai-tts",
            "black-forest-labs/flux-schnell",
            "models/gemini-2.5-pro",
            "models/gemini-2.5-flash-image",
            "models/imagen-4.0-generate-001",
            "BAAI/bge-m3",
            "deepseek-chat",
            "deepseek-chat",
            " ",
            "mistral-moderation-latest",
        ]));
        assert_eq!(kept, ["qwen/qwen3-coder", "models/gemini-2.5-pro", "deepseek-chat"]);
    }

    #[test]
    fn why_a_list_failed_names_the_provider_in_plain_words() {
        let en = crate::i18n::Lang::En;
        let said = |err: ReviewError| why(&err, "Groq", "https://api.groq.com/openai/v1", en);
        assert!(said(ReviewError::Unauthorized).contains("Groq") && said(ReviewError::Unauthorized).contains("key"));
        assert!(said(ReviewError::Network("dns".into())).contains("https://api.groq.com/openai/v1"));
        assert!(said(ReviewError::Http(404, String::new())).contains("/v1"));
        assert!(said(ReviewError::Http(500, String::new())).contains("500"));
        assert!(!said(ReviewError::NoModel).contains("Nemotron"), "not Nebius's words");
        for err in [ReviewError::MissingKey, ReviewError::RateLimited, ReviewError::Timeout, ReviewError::BadOutput] {
            let text = said(err.clone());
            assert!(text.contains("Groq") && !text.contains('{'), "{err:?}: {text}");
        }
    }

    #[tokio::test]
    async fn testing_a_provider_lists_its_models_with_its_key_as_a_bearer_only() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let kept = seen.clone();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            *kept.lock().unwrap() = String::from_utf8_lossy(&buf[..n]).into_owned();
            let body = r#"{"object":"list","data":[{"id":"text-embedding-x"},{"id":"qwen3-coder:30b"},{"id":"gpt-oss:20b"}]}"#;
            let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(body.as_bytes()).await;
        });
        let home = home("list");
        let id = add(&home, OTHER, Some(&base), Some("sk-local-secret-1")).unwrap();
        let cfg = config(&home, &id, none).unwrap();
        assert_eq!(list(&cfg, &id).await.unwrap(), ["qwen3-coder:30b", "gpt-oss:20b"]);
        let request = seen.lock().unwrap().clone();
        assert!(request.starts_with("GET /v1/models "), "{request}");
        assert!(request.to_ascii_lowercase().contains("authorization: bearer sk-local-secret-1"));
        assert!(!request.lines().next().unwrap().contains("sk-local"), "never in the URL");
        assert!(!format!("{cfg:?}").contains("sk-local"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
