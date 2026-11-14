//! Connecting outside services to a squad: the small catalog of services
//! squads commonly need (where to sign in, where a key is created, what a
//! key is for), what the Logins page's "Connections" shows about each, and
//! checking a member's `request_secret` call.
//!
//! The catalog holds only addresses taken from each provider's public
//! documentation (the source is noted on [`CATALOG`]); where the docs give no
//! address, it is `None` and the member (or the person) supplies one. What a
//! key is for, and what to create, are words, so they live in the locale
//! files under `connections.svc.<id>.use` and `connections.svc.<id>.hint`.
//!
//! Nothing here touches a key's value. A key goes from the person's paste
//! into the squad's secret store (`crate::squad_secrets`) and nowhere else;
//! equipe never creates, rotates or deletes a key at a provider and never
//! touches billing.

use serde_json::{Value, json};

use crate::protocol::secrets::{SecretKind, SecretUse, SquadSecret};
use crate::protocol::vault::{Health, Login};
use crate::squad_secrets as ks;
use crate::vault::{serves, site_of};

pub const REQUEST: &str = "request_secret";

/// One service of the catalog.
#[derive(Debug, Clone, Copy)]
pub struct Service {
    /// What a kept key's `service` says (`anthropic`).
    pub id: &'static str,
    /// How it is written for the person.
    pub label: &'static str,
    /// Hosts that are this service: a saved login or a key's `service` naming
    /// one of them (or a subdomain) is this service's.
    pub hosts: &'static [&'static str],
    /// Where the person signs in (https), when the provider's docs name it.
    pub sign_in_url: Option<&'static str>,
    /// Where the person creates the key (https), when the docs name it.
    pub key_url: Option<&'static str>,
    /// What a key is called by default; `None`: no key is added here.
    pub key_name: Option<&'static str>,
    pub kind: SecretKind,
}

/// The catalog. Sources (public documentation, checked 2026-11-14):
/// - Anthropic: "Create an Admin API key", platform.claude.com/docs/en/manage-claude/admin-api-keys — Console sign-in
///   address `platform.claude.com`, Admin keys page `platform.claude.com/settings/admin-keys` (no scopes to choose).
/// - OpenAI: the docs name only "Organization settings > Admin keys" and no address, so none is given.
/// - GitHub: "Managing your personal access tokens", docs.github.com — `github.com/settings/personal-access-tokens/new`
///   (fine-grained token, minimal repository access and permissions). The docs name no sign-in address.
/// - Vercel: "Access tokens", vercel.com/docs/accounts/access-tokens — Account Tokens page `vercel.com/account/tokens`.
///   The docs name no sign-in address.
/// - PostHog: "Personal API keys", posthog.com/docs/api/personal-api-keys — `us.posthog.com/settings/user-api-keys`
///   (US Cloud; EU Cloud and self-hosted use their own host). The docs name no sign-in address.
/// - AWS: "Sign in to the AWS Management Console", docs.aws.amazon.com/signin — `console.aws.amazon.com`.
///   Console sign-in only: no key is created from here.
pub const CATALOG: [Service; 6] = [
    Service {
        id: "anthropic",
        label: "Anthropic Console",
        hosts: &["platform.claude.com", "console.anthropic.com", "anthropic.com"],
        sign_in_url: Some("https://platform.claude.com/"),
        key_url: Some("https://platform.claude.com/settings/admin-keys"),
        key_name: Some("anthropic-admin-key"),
        kind: SecretKind::ApiKey,
    },
    Service { id: "openai", label: "OpenAI platform", hosts: &["platform.openai.com", "openai.com"], sign_in_url: None, key_url: None, key_name: Some("openai-admin-key"), kind: SecretKind::ApiKey },
    Service {
        id: "github",
        label: "GitHub",
        hosts: &["github.com"],
        sign_in_url: None,
        key_url: Some("https://github.com/settings/personal-access-tokens/new"),
        key_name: Some("github-token"),
        kind: SecretKind::Token,
    },
    Service { id: "vercel", label: "Vercel", hosts: &["vercel.com"], sign_in_url: None, key_url: Some("https://vercel.com/account/tokens"), key_name: Some("vercel-token"), kind: SecretKind::Token },
    Service {
        id: "posthog",
        label: "PostHog",
        hosts: &["posthog.com"],
        sign_in_url: None,
        key_url: Some("https://us.posthog.com/settings/user-api-keys"),
        key_name: Some("posthog-personal-key"),
        kind: SecretKind::ApiKey,
    },
    Service { id: "aws", label: "AWS", hosts: &["aws.amazon.com", "amazon.com"], sign_in_url: Some("https://console.aws.amazon.com/"), key_url: None, key_name: None, kind: SecretKind::Other },
];

impl Service {
    /// Where "Sign in" opens: the sign-in page, else the key page (a page that
    /// asks for a sign-in first). `None`: the docs name neither.
    pub fn sign_in_target(&self) -> Option<&'static str> {
        self.sign_in_url.or(self.key_url)
    }

    /// Whether a key can be added here.
    pub fn takes_key(&self) -> bool {
        self.key_name.is_some()
    }

    pub fn by_id(id: &str) -> Option<&'static Service> {
        CATALOG.iter().find(|s| s.id == id)
    }

    /// The catalog service a name, a site or an address is.
    pub fn of(text: &str) -> Option<&'static Service> {
        let text = text.trim().to_ascii_lowercase();
        if text.is_empty() {
            return None;
        }
        if let Some(found) = CATALOG.iter().find(|s| s.id == text || s.label.to_ascii_lowercase() == text) {
            return Some(found);
        }
        let host = site_of(&text);
        CATALOG.iter().find(|s| !host.is_empty() && s.hosts.iter().any(|h| serves(h, &host)))
    }
}

/// The key's words on the card: `anthropic-admin-key` as "Anthropic admin
/// key" (the catalog's label stands for its id).
pub fn title_of(name: &str, service: &str) -> String {
    let spaced = name.replace(['-', '_', '.'], " ");
    let mut words = spaced.split_whitespace();
    let first = words.next().unwrap_or_default();
    let rest: Vec<&str> = words.collect();
    let head = match Service::of(service).filter(|s| s.id.eq_ignore_ascii_case(first)) {
        Some(s) => s.label.split_whitespace().next().unwrap_or(s.label).to_owned(),
        None => {
            let mut c = first.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    };
    std::iter::once(head.as_str()).chain(rest.iter().copied()).collect::<Vec<_>>().join(" ")
}

// ───────────────────────── status ─────────────────────────

/// Whether the members' browser is signed in to a service, as far as equipe
/// knows from its saved logins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedIn {
    /// A saved login's last sign-in worked.
    Yes,
    /// A saved login's last sign-in failed, or equipe stopped using it.
    NeedsAttention,
    /// A saved login exists and was never used.
    Saved,
    /// Nothing known: no saved login.
    Unknown,
}

/// What Connections shows for one service of one squad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub id: &'static str,
    /// The names of the keys kept for this squad under this service.
    pub keys: Vec<String>,
    pub signed_in: SignedIn,
    /// The latest use of any of the keys.
    pub last_used: Option<SecretUse>,
}

pub fn status(service: &Service, squad: &str, secrets: &[SquadSecret], logins: &[Login]) -> Status {
    let mine = keys_of(service, squad, secrets);
    let last_used = mine.iter().filter_map(|s| s.last_used.clone()).max_by_key(|u| u.at);
    let kept: Vec<&Login> = logins.iter().filter(|l| service.hosts.iter().any(|h| serves(h, &l.site))).collect();
    let signed_in = if kept.is_empty() {
        SignedIn::Unknown
    } else if kept.iter().any(|l| l.health() == Health::Ok) {
        SignedIn::Yes
    } else if kept.iter().any(|l| l.health() == Health::Failed) {
        SignedIn::NeedsAttention
    } else {
        SignedIn::Saved
    };
    Status { id: service.id, keys: mine.iter().map(|s| s.name.clone()).collect(), signed_in, last_used }
}

/// The squad's secrets whose service is `service`: what "Disconnect" removes.
pub fn keys_of<'a>(service: &Service, squad: &str, secrets: &'a [SquadSecret]) -> Vec<&'a SquadSecret> {
    secrets.iter().filter(|s| s.squad == squad && Service::of(&s.service).is_some_and(|f| f.id == service.id)).collect()
}

// ───────────────────────── request_secret ─────────────────────────

/// The tool, as an MCP tool definition.
pub fn tool() -> Value {
    let kinds: Vec<&str> = SecretKind::ALL.iter().map(|k| k.code()).collect();
    json!({
        "name": REQUEST,
        "description": "Ask the person for a key, token or password you need from an outside service — an Anthropic Console admin \
key, a GitHub token, a PostHog personal key. They see ONE card: what it is for, a button that opens the page where they create it, \
your hint on what to create, and a masked box to paste it into; equipe keeps it in this machine's secret store under `name`. \
You never see the value and it never passes through the conversation. When they save it you are told `saved as <name>` and carry \
on: use it with `secret_use` or `secret_write`; never ask for a key in chat. Every request carries `steps` (numbered on the card) \
and the exact page as `create_url`: a card without them is refused. equipe never creates or rotates a key at the provider for you, \
and never touches billing. A website sign-in (a person's account, not a key) is `sign_in`, not this. For a service in equipe's \
catalog (anthropic, github, vercel, posthog) `create_url` is filled in when left out; for any other, give its key page (https).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "What to keep it as: letters, digits, . _ - (`anthropic-admin-key`)." },
                "service": { "type": "string", "description": "The service: `anthropic`, `github`, or a site (`app.example.com`)." },
                "kind": { "type": "string", "enum": kinds, "description": "What it is." },
                "why": { "type": "string", "description": "What it is for, in a line in the person's language: \"read API-key metadata, usage and cost reports\"." },
                "steps": steps_schema("what the person does to create and copy it"),
                "create_url": { "type": "string", "description": "The exact page where the person creates it (https, a real site: no credentials, port, IP or localhost). Required; left out only for a catalog service (the catalog's page is used)." },
                "no_page": no_page_schema(),
                "scopes_hint": { "type": "string", "description": "What to create there, in a short line in the person's language: \"read-only / usage scope\"." },
                "projects": { "type": "array", "items": { "type": "string" }, "description": "The squad's projects whose work may use it, by name. Left out: the project of your ticket." },
            },
            "required": ["name", "service", "kind", "why", "steps"],
        },
    })
}

/// At most this many steps on one card.
pub const MOST_STEPS: usize = 8;

/// The schema of a `steps` argument, shared by `ask_secret` and `request_secret`.
pub fn steps_schema(what: &str) -> Value {
    json!({
        "type": "array",
        "items": { "type": "string" },
        "minItems": 1,
        "maxItems": MOST_STEPS,
        "description": format!("Required. {what}: 1 to {MOST_STEPS} short imperative lines in the person's language, in order, naming the exact menu items and buttons (\"Open Settings > Admin keys\", \"Click Create key\", \"Copy the key: it is shown once\"). The card numbers them; an address inside a step becomes a link."),
    })
}

/// The schema of a `no_page` argument, shared by `ask_secret` and `request_secret`.
pub fn no_page_schema() -> Value {
    json!({ "type": "boolean", "description": "Only when the secret has no web page at all (a local NAS password): then no url is needed, and `steps` must say where to find it. Anything from a website or console needs its page as the url." })
}

/// The guide a card carries: what to do, and the page to do it on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guide {
    pub steps: Vec<String>,
    /// The page the secret comes from.
    pub url: Option<String>,
    /// The service's sign-in page, when it differs from `url`.
    pub sign_in_url: Option<String>,
}

/// Check the guide of a call: `steps` (1 to 8 lines), and the page under
/// `url_key` — else the catalog's key page for `service`. A call without
/// steps, or from a website without its page, is refused with what to add.
pub fn check_guide(args: &Value, service: &str, url_key: &str) -> Result<Guide, String> {
    let given: Vec<String> = match args.get("steps") {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty()).collect(),
        _ => Vec::new(),
    };
    if given.is_empty() || given.len() > MOST_STEPS {
        return Err(format!("Add `steps`: 1 to {MOST_STEPS} short imperative lines saying what the person does, in order, in their language, naming the exact menu items and buttons — the card shows them numbered. A request without them is refused."));
    }
    let steps = given.into_iter().map(|s| s.chars().take(300).collect()).collect();
    let catalog = Service::of(service);
    let said = args.get(url_key).and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let url = if said.is_empty() { catalog.and_then(|s| s.key_url).map(str::to_owned) } else { Some(clean_url(said)?) };
    let no_page = args.get("no_page").and_then(Value::as_bool).unwrap_or(false);
    if url.is_none() && !no_page {
        return Err(format!("Add `{url_key}`: the exact https page where the person gets it (the key page of the service's console). The card opens it with one button. Only a secret with no web page at all (a local NAS password) may leave it out: set `no_page` to true and say in `steps` where to find it."));
    }
    // The sign-in page as a second link, when the page is a service's own.
    let sign_in_url = catalog.or_else(|| url.as_deref().and_then(Service::of)).and_then(|s| s.sign_in_url).map(str::to_owned).filter(|s| url.as_ref() != Some(s));
    Ok(Guide { steps, url, sign_in_url })
}

/// A `request_secret` call, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub name: String,
    pub service: String,
    pub kind: SecretKind,
    pub why: String,
    pub create_url: Option<String>,
    pub sign_in_url: Option<String>,
    pub steps: Vec<String>,
    pub scopes_hint: Option<String>,
    pub projects: Option<Vec<String>>,
}

pub fn check_request(args: &Value) -> Result<Request, String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
    let name = text("name");
    if !ks::valid_name(&name) {
        return Err(format!("{name:?}: a name is letters, digits, `.`, `_` and `-`, up to 64 (`anthropic-admin-key`)."));
    }
    let service = text("service");
    if service.is_empty() || service.len() > 120 {
        return Err("Say in `service` which service or site it is for (`anthropic`).".into());
    }
    let kind = SecretKind::parse(&text("kind")).ok_or_else(|| format!("`kind` is one of {}.", SecretKind::ALL.iter().map(|k| k.code()).collect::<Vec<_>>().join(", ")))?;
    let why = text("why");
    if why.is_empty() {
        return Err("Say in `why` what it is for, in a line.".into());
    }
    // `url` is taken as `create_url` too: both name the page.
    let key = if text("create_url").is_empty() && !text("url").is_empty() { "url" } else { "create_url" };
    let Guide { steps, url: create_url, sign_in_url } = check_guide(args, &service, key)?;
    let scopes_hint = Some(text("scopes_hint")).filter(|h| !h.is_empty()).map(|h| h.chars().take(300).collect());
    let projects = args.get("projects").map(|_| crate::board::arg_strs(args, "projects"));
    Ok(Request { name, service, kind, why: why.chars().take(300).collect(), create_url, sign_in_url, steps, scopes_hint, projects })
}

/// A page address to open on the person's screen: https, a real host, no
/// credentials, no port, no spaces. Anything else is refused, so a card never
/// opens a local file, an app link or an address with a password in it.
pub fn clean_url(given: &str) -> Result<String, String> {
    let url = given.trim();
    let refuse = |why: &str| Err(format!("{url:?}: {why} (an https address like https://platform.claude.com/settings/admin-keys)."));
    if url.len() > 500 || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return refuse("not an address");
    }
    let Some(rest) = url.strip_prefix("https://") else { return refuse("only https addresses open") };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains(['@', ':', '\\']) {
        return refuse("no credentials or port in it");
    }
    let labels: Vec<&str> = authority.split('.').collect();
    let host_ok = labels.len() >= 2 && labels.iter().all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    if !host_ok || labels.last().is_some_and(|l| l.chars().all(|c| c.is_ascii_digit())) {
        return refuse("not a site's address");
    }
    Ok(url.to_owned())
}

/// What the member is told when the person saved its key: the name, never the value.
pub fn saved_note(name: &str) -> String {
    format!("The person saved the key you asked for: saved as {name}. Use it with `secret_use` or `secret_write`; never print, echo or copy the value.")
}

/// The host of a checked address, as the card shows it next to its button.
pub fn host_of(url: &str) -> String {
    site_of(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_entry_is_an_https_address_with_a_real_host() {
        let mut ids = std::collections::HashSet::new();
        for s in CATALOG {
            assert!(ids.insert(s.id), "{} twice", s.id);
            for url in [s.sign_in_url, s.key_url].into_iter().flatten() {
                assert_eq!(clean_url(url).as_deref(), Ok(url), "{}: {url}", s.id);
                assert!(s.hosts.iter().any(|h| serves(h, url)), "{}: {url} is not one of its hosts", s.id);
            }
            assert_eq!(s.takes_key(), s.key_name.is_some());
            assert!(Service::of(s.id).is_some_and(|f| f.id == s.id) && Service::of(s.label).is_some_and(|f| f.id == s.id));
        }
        assert_eq!(Service::by_id("anthropic").unwrap().key_url, Some("https://platform.claude.com/settings/admin-keys"));
        assert!(Service::by_id("openai").unwrap().key_url.is_none(), "not in the provider's public docs: left out");
    }

    #[test]
    fn a_service_is_known_by_id_label_site_or_subdomain() {
        for text in ["anthropic", "Anthropic Console", "platform.claude.com", "https://console.anthropic.com/settings/keys", "us.posthog.com", "app.posthog.com"] {
            assert!(Service::of(text).is_some(), "{text}");
        }
        assert_eq!(Service::of("eu.posthog.com").unwrap().id, "posthog");
        assert!(Service::of("example.com").is_none() && Service::of("").is_none());
    }

    const STEPS: [&str; 2] = ["Open Settings > Admin keys", "Click Create key and copy it"];

    #[test]
    fn a_request_is_checked_and_its_page_must_be_an_https_site() {
        let ok = check_request(&json!({ "name": "anthropic-admin-key", "service": "anthropic", "kind": "api_key", "why": "usage reports", "steps": STEPS })).unwrap();
        assert_eq!(ok.create_url.as_deref(), Some("https://platform.claude.com/settings/admin-keys"), "the catalog's page");
        assert_eq!(ok.sign_in_url.as_deref(), Some("https://platform.claude.com/"), "and its sign-in as a second link");
        assert_eq!(ok.steps, STEPS);
        let own = check_request(&json!({ "name": "k", "service": "acme", "kind": "token", "why": "x", "steps": STEPS, "create_url": "https://acme.example.com/keys?x=1", "scopes_hint": " read only " })).unwrap();
        assert_eq!((own.create_url.as_deref(), own.scopes_hint.as_deref(), own.sign_in_url.is_none()), (Some("https://acme.example.com/keys?x=1"), Some("read only"), true));
        // `url` names the page as well as `create_url`.
        let by_url = check_request(&json!({ "name": "k", "service": "acme", "kind": "token", "why": "x", "steps": STEPS, "url": "https://acme.example.com/keys" })).unwrap();
        assert_eq!(by_url.create_url.as_deref(), Some("https://acme.example.com/keys"));
        for bad in ["http://acme.example.com", "javascript:alert(1)", "file:///etc/passwd", "https://user:pw@acme.example.com", "https://acme.example.com:8443/", "https://localhost/", "https://127.0.0.1/", "https://acme.example.com/a b", "acme.example.com"] {
            assert!(check_request(&json!({ "name": "k", "service": "acme", "kind": "token", "why": "x", "steps": STEPS, "create_url": bad })).is_err(), "{bad}");
        }
        for missing in [json!({ "service": "a", "kind": "token", "why": "x" }), json!({ "name": "k", "kind": "token", "why": "x" }), json!({ "name": "k", "service": "a", "why": "x" }), json!({ "name": "k", "service": "a", "kind": "token" }), json!({ "name": "bad name", "service": "a", "kind": "token", "why": "x" })] {
            let mut args = missing.clone();
            args["steps"] = json!(STEPS);
            args["create_url"] = json!("https://a.example.com/k");
            assert!(check_request(&args).is_err(), "{missing}");
        }
    }

    #[test]
    fn a_request_without_steps_or_without_a_page_is_refused_with_what_to_add() {
        let base = json!({ "name": "k", "service": "acme", "kind": "token", "why": "x" });
        let with = |extra: Value| {
            let mut args = base.clone();
            args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            check_request(&args)
        };
        let no_steps = with(json!({ "create_url": "https://acme.example.com/keys" })).unwrap_err();
        assert!(no_steps.contains("`steps`"), "{no_steps}");
        assert!(with(json!({ "create_url": "https://acme.example.com/keys", "steps": [] })).is_err());
        // A service outside the catalog, no page: refused, saying `create_url` and the one way out.
        let no_url = with(json!({ "steps": STEPS })).unwrap_err();
        assert!(no_url.contains("`create_url`") && no_url.contains("no_page"), "{no_url}");
        assert!(with(json!({ "steps": STEPS, "no_page": true })).unwrap().create_url.is_none());
        // OpenAI is in the catalog but its docs name no address: still needs one from the member.
        assert!(check_request(&json!({ "name": "k", "service": "openai", "kind": "api_key", "why": "x", "steps": STEPS })).is_err());
    }

    #[test]
    fn both_tools_require_steps_in_their_schema_and_say_so() {
        for (tool, url_key) in [(tool(), "create_url"), (crate::secrets::tool(), "url")] {
            let required: Vec<&str> = tool["inputSchema"]["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
            assert!(required.contains(&"steps"), "{tool}");
            let steps = &tool["inputSchema"]["properties"]["steps"];
            assert_eq!((steps["minItems"].as_u64(), steps["maxItems"].as_u64()), (Some(1), Some(8)));
            assert!(tool["inputSchema"]["properties"][url_key].is_object() && tool["inputSchema"]["properties"]["no_page"].is_object());
            let said = tool["description"].as_str().unwrap();
            assert!(said.contains("`steps`") && said.contains("refused"), "{said}");
        }
    }

    /// What the charters stopped saying because the tool says it.
    #[test]
    fn the_tool_says_the_person_pastes_it_into_a_card_and_never_in_chat() {
        let said = tool()["description"].as_str().unwrap().to_owned();
        for part in ["ONE card", "never ask for a key in chat", "You never see the value", "never creates or rotates a key", "never touches billing"] {
            assert!(said.contains(part), "{part}");
        }
    }

    #[test]
    fn the_member_is_told_the_name_and_nothing_else() {
        let said = saved_note("anthropic-admin-key");
        assert!(said.contains("saved as anthropic-admin-key") && !said.contains("sk-"), "{said}");
    }

    #[test]
    fn a_card_is_titled_from_the_keys_name() {
        assert_eq!(title_of("anthropic-admin-key", "anthropic"), "Anthropic admin key");
        assert_eq!(title_of("acme_deploy.token", "acme"), "Acme deploy token");
    }

    fn secret(squad: &str, name: &str, service: &str, used: Option<u64>) -> SquadSecret {
        SquadSecret {
            squad: squad.into(),
            name: name.into(),
            kind: SecretKind::ApiKey,
            service: service.into(),
            projects: Vec::new(),
            any_project: false,
            created_by: "pm".into(),
            created_ticket: None,
            created_at: 1,
            note: String::new(),
            last_used: used.map(|at| SecretUse { at, member: "backend".into(), ticket: None, project: String::new(), how: "env".into() }),
        }
    }

    #[test]
    fn a_service_shows_its_keys_its_sign_in_and_its_last_use_for_its_own_squad() {
        let anthropic = Service::by_id("anthropic").unwrap();
        let secrets = vec![secret("acme", "anthropic-admin-key", "anthropic", Some(50)), secret("acme", "old", "platform.claude.com", Some(90)), secret("other", "theirs", "anthropic", Some(99)), secret("acme", "gh", "github", None)];
        let none = status(anthropic, "acme", &[], &[]);
        assert_eq!((none.keys.len(), none.signed_in, none.last_used.is_none()), (0, SignedIn::Unknown, true));
        let s = status(anthropic, "acme", &secrets, &[]);
        assert_eq!(s.keys, ["anthropic-admin-key", "old"]);
        assert_eq!(s.last_used.map(|u| u.at), Some(90));
        let fresh = status(Service::by_id("github").unwrap(), "other", &secrets, &[]);
        assert!(fresh.keys.is_empty(), "another squad's keys are not this squad's");
        assert_eq!(keys_of(anthropic, "acme", &secrets).len(), 2);
    }
}
