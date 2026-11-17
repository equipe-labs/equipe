//! PostHog's "Set it up for me" (docs/METRICS.md): the person signs in to
//! PostHog once, in equipe's own browser, and equipe does the rest — makes a
//! read-only personal API key there, keeps it in the Keychain, connects the
//! project and picks the events its cards count.
//!
//! - **Looking** (`metrics.posthog_look`): the browser host's `posthog` op
//!   asks PostHog's API, on the person's session in equipe's tab, who is
//!   signed in on US and EU Cloud and to which organization and projects.
//!   The region is the address the browser is signed in at.
//! - **Making the key** (`metrics.posthog_setup`) only on the person's yes —
//!   the card that says what is made, with which scopes, on which project
//!   — never on a member's word: members have no way to send it. The host
//!   calls `POST /api/personal_api_keys/` (what PostHog's own settings page
//!   calls) with [`SCOPES`], scoped to that one project, named
//!   [`KEY_LABEL`]. Its value comes back to the daemon alone
//!   (`browser::Hub::ask`), goes into the Keychain as the connection's key,
//!   and is in nothing else: not an answer, an event, a log, a file.
//! - **Picking events**: the project's top events of the last 30 days, asked
//!   with the new key. No `$pageview`: the visit cards say the project sends
//!   no page views. No `work_accepted` (equipe's own event): the North Star
//!   counts the people who sent the project's most sent event of its own.
//!
//! When PostHog says no — signed out, a sign-in older than PostHog allows
//! for making keys (2 hours), a limit on keys — the person is told, and the
//! key page with the steps and a paste box is the way on ([`SetupOutcome`]).

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::adapters::posthog::{self, EVERY, PEOPLE};
use super::{BAD_PARAMS, Metrics, Refused, refuse, valid_name};
use crate::protocol::metrics::{
    ConnectParams, EventCount, PosthogAccount, PosthogLook, PosthogLookParams, PosthogProject, PosthogSetup, PosthogSetupParams, SetupOutcome, SourceKind,
};

/// The key's name in PostHog: what the person finds on their key page.
pub const KEY_LABEL: &str = "equipe metrics (read-only)";
/// The scopes it is made with: reading queries — all the Metrics screen asks.
/// Projects are listed on the person's session, so the key needs no more.
pub const SCOPES: &[&str] = &["query:read"];
/// PostHog Cloud's two regions, by the address the browser is signed in at.
pub const HOSTS: [(&str, &str); 2] = [("us", "https://us.posthog.com"), ("eu", "https://eu.posthog.com")];
/// How long one ask of the browser may take (a tab opens on each region).
const ASK_WAIT: Duration = Duration::from_secs(45);

/// Where equipe's browser is asked: the browser host's `posthog` op
/// ([`crate::browser::Hub`]), or a stand-in in tests.
pub trait Browser: Send + Sync {
    fn ask(&self, request: Value) -> Pin<Box<dyn Future<Output = Option<Value>> + Send + '_>>;
}

impl Browser for crate::browser::Hub {
    fn ask(&self, request: Value) -> Pin<Box<dyn Future<Output = Option<Value>> + Send + '_>> {
        Box::pin(async move { crate::browser::Hub::ask(self, request, ASK_WAIT).await.map(|answer| answer.0) })
    }
}

/// PostHog's key page on `host`: where the person sees and deletes the key.
pub fn keys_url(host: &str) -> String {
    format!("{}/settings/user-api-keys", host.trim_end_matches('/'))
}

/// The accounts in the host's answer to `look`: each region signed in, with
/// its organization's projects.
pub fn accounts(answer: &Value) -> Vec<PosthogAccount> {
    let hosts = answer.get("hosts").and_then(Value::as_array).cloned().unwrap_or_default();
    hosts
        .iter()
        .filter(|h| h.get("status").and_then(Value::as_u64) == Some(200))
        .filter_map(|h| {
            let bare = h.get("host").and_then(Value::as_str)?;
            let (region, host) = HOSTS.iter().find(|(_, url)| url.strip_prefix("https://") == Some(bare))?;
            let org = h.get("organization")?;
            let projects: Vec<PosthogProject> = org
                .get("teams")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|t| Some(PosthogProject { id: t.get("id")?.as_u64()?, name: t.get("name").and_then(Value::as_str).unwrap_or_default().to_owned() }))
                .collect();
            (!projects.is_empty()).then(|| PosthogAccount {
                host: (*host).to_owned(),
                region: (*region).to_owned(),
                organization: org.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
                projects,
                current: h.get("current").and_then(Value::as_u64),
            })
        })
        .collect()
}

/// What PostHog answered the key's making.
#[derive(PartialEq)]
pub enum Minted {
    /// The key's value: for the Keychain only.
    Key(String),
    SignedOut,
    /// A sign-in older than PostHog allows for making a key.
    SignInAgain,
    /// No, in PostHog's words when it said any.
    Refused(Option<String>),
}

impl std::fmt::Debug for Minted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Minted::Key(_) => write!(f, "Key(•••)"),
            Minted::SignedOut => write!(f, "SignedOut"),
            Minted::SignInAgain => write!(f, "SignInAgain"),
            Minted::Refused(why) => write!(f, "Refused({why:?})"),
        }
    }
}

/// The host's answer to `key`, read.
pub fn minted(answer: Option<&Value>) -> Minted {
    let Some(answer) = answer else { return Minted::Refused(None) };
    let status = answer.get("status").and_then(Value::as_u64).unwrap_or(0);
    let said = answer.get("detail").and_then(Value::as_str).map(str::to_owned);
    match status {
        200 | 201 => match answer.get("value").and_then(Value::as_str).filter(|v| v.len() >= 8) {
            Some(value) => Minted::Key(value.to_owned()),
            None => Minted::Refused(None),
        },
        401 => Minted::SignedOut,
        403 if answer.get("code").and_then(Value::as_str) == Some("sensitive_action_required_reauth") || said.as_deref().is_some_and(|s| s.contains("recently authenticated")) => Minted::SignInAgain,
        // A 403 that isn't about the sign-in's age: signed in as someone without rights there.
        _ => Minted::Refused(said),
    }
}

/// The cards' events, from the project's top events: whether it sends page
/// views, and — when it doesn't send equipe's `work_accepted` — the event of
/// its own it sends most, for the North Star.
pub fn pick(events: &[EventCount]) -> (bool, Option<String>) {
    let page_views = events.iter().any(|e| e.event == "$pageview");
    let north = if events.iter().any(|e| e.event == "work_accepted") { None } else { events.iter().find(|e| !e.event.starts_with('$') && e.count > 0).map(|e| e.event.clone()) };
    (page_views, north)
}

/// The host a setup may send the key to: PostHog Cloud's, or (tests) a
/// stand-in on this machine.
fn known_host(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('/');
    if HOSTS.iter().any(|(_, h)| *h == host) || (cfg!(test) && host.starts_with("http://127.0.0.1:")) { Some(host.to_owned()) } else { None }
}

impl Metrics {
    /// `metrics.posthog_look`.
    pub async fn posthog_look(&self, browser: Option<&dyn Browser>, p: &PosthogLookParams) -> Result<PosthogLook, Refused> {
        if !valid_name(&p.squad) {
            return Err(refuse(BAD_PARAMS, "name a squad"));
        }
        let Some(browser) = browser else { return Ok(PosthogLook::default()) };
        if let Some(host) = p.sign_out.as_deref().and_then(known_host) {
            let bare = host.split("://").nth(1).unwrap_or(&host).to_owned();
            browser.ask(json!({ "op": "posthog", "step": "logout", "host": bare })).await;
        }
        let answer = browser.ask(json!({ "op": "posthog", "step": "look" })).await;
        Ok(PosthogLook { browser: answer.is_some(), accounts: answer.as_ref().map(accounts).unwrap_or_default() })
    }

    /// `metrics.posthog_setup`: the person said yes on the card that names
    /// the key, its scopes and the project. The first fetch is the caller's
    /// to start, as after `metrics.connect`.
    pub async fn posthog_setup(self: &Arc<Self>, browser: Option<&dyn Browser>, p: PosthogSetupParams) -> Result<PosthogSetup, Refused> {
        if !valid_name(&p.squad) {
            return Err(refuse(BAD_PARAMS, "name a squad"));
        }
        let host = known_host(&p.host).ok_or_else(|| refuse(BAD_PARAMS, "the host is PostHog Cloud's, US or EU"))?;
        let mut out = PosthogSetup { key_label: KEY_LABEL.into(), scopes: SCOPES.iter().map(|s| (*s).to_owned()).collect(), keys_url: keys_url(&host), page_views: true, ..PosthogSetup::default() };
        let Some(browser) = browser else {
            out.why = Some("equipe's browser isn't available on this Mac".into());
            return Ok(out);
        };
        let bare = host.split("://").nth(1).unwrap_or(&host).to_owned();
        let request = json!({ "op": "posthog", "step": "key", "host": bare, "label": KEY_LABEL, "scopes": SCOPES, "teams": [p.project_id] });
        let key = match minted(browser.ask(request).await.as_ref()) {
            Minted::Key(key) => key,
            Minted::SignedOut => return Ok(PosthogSetup { outcome: SetupOutcome::SignIn, ..out }),
            Minted::SignInAgain => return Ok(PosthogSetup { outcome: SetupOutcome::SignInAgain, ..out }),
            Minted::Refused(why) => return Ok(PosthogSetup { why, ..out }),
        };
        crate::daemon_log!("equipe: metrics: a read-only PostHog key was made for project {} on the person's yes", p.project_id);
        // The cards' events, asked with the new key before it is kept.
        let events = match posthog::top_events(&self.client, &host, p.project_id, &key).await {
            Ok(events) => events,
            Err(e) => {
                crate::daemon_log!("equipe: metrics: PostHog's events couldn't be read yet ({:?})", e.code);
                Vec::new()
            }
        };
        let (page_views, north) = if events.is_empty() { (true, None) } else { pick(&events) };
        let kept_north = p.id.as_ref().and_then(|id| super::connections(&self.home).into_iter().find(|c| &c.id == id)).is_some_and(|c| c.settings.contains_key("north_star_event"));
        let north = north.filter(|_| !kept_north);
        let mut settings: BTreeMap<String, String> = [("api_key".to_owned(), key), ("project_id".to_owned(), p.project_id.to_string()), ("host".to_owned(), host.clone())].into();
        if let Some(event) = &north {
            settings.insert("north_star_event".into(), event.clone());
            settings.insert("north_star_property".into(), PEOPLE.into());
            settings.insert("north_star_where".into(), EVERY.into());
        }
        let params = ConnectParams { squad: p.squad.clone(), project: p.project.clone(), kind: p.id.is_none().then_some(SourceKind::Posthog), id: p.id.clone(), settings };
        let connection = self.keyed(move |me| me.connect(params, super::adapters::unix_now())).await?;
        Ok(PosthogSetup { outcome: SetupOutcome::Connected, connection: Some(connection), events, page_views, north_star_event: north, ..out })
    }
}

#[cfg(test)]
mod tests {
    use super::super::adapters::stand_in::{Reply, serve};
    use super::super::{InMemory, Keys};
    use super::*;

    const KEY: &str = "phx_made_by_equipe_0123456789";

    /// A browser that answers as PostHog's API would, and keeps what it was asked.
    struct Stand {
        look: Value,
        key: Option<Value>,
        asked: std::sync::Mutex<Vec<Value>>,
    }

    impl Browser for Stand {
        fn ask(&self, request: Value) -> Pin<Box<dyn Future<Output = Option<Value>> + Send + '_>> {
            self.asked.lock().unwrap().push(request.clone());
            let answer = if request["step"] == "look" { Some(self.look.clone()) } else { self.key.clone() };
            Box::pin(async move { answer })
        }
    }

    fn signed_in(teams: Value) -> Value {
        json!({ "hosts": [
            { "host": "us.posthog.com", "status": 200, "current": 735144, "organization": { "id": "o1", "name": "Atinum", "teams": teams } },
            { "host": "eu.posthog.com", "status": 401 },
        ] })
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-posthog-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Metrics whose keys are kept in memory, shared with the test.
    struct Shared(Arc<std::sync::Mutex<InMemory>>);

    impl Keys for Shared {
        fn set(&mut self, id: &str, keys: &BTreeMap<String, String>) -> Result<(), String> {
            self.0.lock().unwrap().set(id, keys)
        }
        fn get(&self, id: &str) -> Result<BTreeMap<String, String>, String> {
            self.0.lock().unwrap().get(id)
        }
        fn delete(&mut self, id: &str) -> Result<(), String> {
            self.0.lock().unwrap().delete(id)
        }
    }

    #[test]
    fn the_accounts_are_read_with_their_region_and_projects() {
        let all = accounts(&signed_in(json!([{ "id": 735144, "name": "Atinum MCP" }, { "id": 9, "name": "Site" }])));
        assert_eq!(all.len(), 1, "EU said 401: not signed in there");
        assert_eq!((all[0].host.as_str(), all[0].region.as_str(), all[0].organization.as_str(), all[0].current), ("https://us.posthog.com", "us", "Atinum", Some(735144)));
        assert_eq!(all[0].projects.iter().map(|p| p.id).collect::<Vec<_>>(), [735144, 9]);
        let look = PosthogLook { browser: true, accounts: all };
        assert!(look.signed_in() && look.only_project().is_none(), "two projects: the person picks");
        let one = PosthogLook { browser: true, accounts: accounts(&signed_in(json!([{ "id": 735144, "name": "Atinum MCP" }]))) };
        assert_eq!(one.only_project().map(|(a, p)| (a.region.as_str(), p.id)), Some(("us", 735144)));
        // A host that isn't PostHog Cloud's is no account, whatever it says.
        assert!(accounts(&json!({ "hosts": [{ "host": "posthog.evil.example", "status": 200, "organization": { "teams": [{ "id": 1 }] } }] })).is_empty());
        assert!(accounts(&json!({ "hosts": [] })).is_empty());
    }

    #[test]
    fn what_posthog_answers_the_key_is_read_and_the_key_never_printed() {
        let made = minted(Some(&json!({ "status": 201, "id": "k1", "value": KEY })));
        assert_eq!(made, Minted::Key(KEY.into()));
        assert!(!format!("{made:?}").contains(KEY));
        assert_eq!(minted(Some(&json!({ "status": 401 }))), Minted::SignedOut);
        let old = json!({ "status": 403, "code": "sensitive_action_required_reauth", "detail": "This action requires you to be recently authenticated." });
        assert_eq!(minted(Some(&old)), Minted::SignInAgain);
        assert_eq!(minted(Some(&json!({ "status": 400, "detail": "You can only have 10 personal API keys." }))), Minted::Refused(Some("You can only have 10 personal API keys.".into())));
        assert_eq!(minted(None), Minted::Refused(None));
        assert_eq!(minted(Some(&json!({ "status": 201 }))), Minted::Refused(None), "no value: nothing to keep");
    }

    #[test]
    fn the_events_pick_the_cards() {
        let e = |event: &str, count| EventCount { event: event.into(), count };
        assert_eq!(pick(&[e("$pageview", 900), e("cta_click", 20)]), (true, Some("cta_click".into())));
        assert_eq!(pick(&[e("$identify", 50), e("mcp_tool_called", 812)]), (false, Some("mcp_tool_called".into())));
        assert_eq!(pick(&[e("$pageview", 900), e("work_accepted", 3)]), (true, None), "equipe's own North Star is there");
        assert_eq!(pick(&[e("$autocapture", 5)]), (false, None));
    }

    #[tokio::test]
    async fn set_it_up_makes_a_read_only_key_keeps_it_and_picks_the_events() {
        let (url, seen) = serve(|_| Reply::ok(br#"{"results": [["mcp_tool_called", 812], ["$identify", 40]]}"#.to_vec())).await;
        let home = scratch("made");
        let keys = Arc::new(std::sync::Mutex::new(InMemory::default()));
        let metrics = Arc::new(Metrics::new(&home, Box::new(Shared(keys.clone()))));
        let browser = Stand { look: signed_in(json!([{ "id": 735144, "name": "Atinum MCP" }])), key: Some(json!({ "status": 201, "id": "k1", "value": KEY })), asked: Default::default() };
        let p = PosthogSetupParams { squad: "atinum".into(), project: None, host: url.clone(), project_id: 735144, id: None };
        let done = metrics.posthog_setup(Some(&browser), p).await.unwrap();

        assert_eq!(done.outcome, SetupOutcome::Connected);
        let c = done.connection.clone().unwrap();
        assert_eq!(c.kind, SourceKind::Posthog);
        assert_eq!(c.settings.get("project_id").map(String::as_str), Some("735144"));
        assert_eq!(c.settings.get("host"), Some(&url), "the region the browser is signed in at");
        assert_eq!(c.keys, ["api_key"]);
        assert_eq!(keys.lock().unwrap().get(&c.id).unwrap().get("api_key").map(String::as_str), Some(KEY), "the key is kept sealed");
        // No page views, no work_accepted: the North Star counts the people of its own top event.
        assert!(!done.page_views);
        assert_eq!(done.north_star_event.as_deref(), Some("mcp_tool_called"));
        assert_eq!((c.settings["north_star_property"].as_str(), c.settings["north_star_where"].as_str()), (PEOPLE, EVERY));
        // Made with only the read scope, on that one project, under equipe's name.
        let asked = browser.asked.lock().unwrap();
        assert_eq!(asked[0]["scopes"], json!(["query:read"]));
        assert_eq!(asked[0]["teams"], json!([735144]));
        assert_eq!(asked[0]["label"], KEY_LABEL);
        assert_eq!(seen.lock().unwrap()[0].headers.get("authorization").map(String::as_str), Some(format!("Bearer {KEY}").as_str()));
        // The key is in no answer and no file.
        assert!(!serde_json::to_string(&done).unwrap().contains(KEY));
        let mut files = String::new();
        for e in walk(&home) {
            files.push_str(&std::fs::read_to_string(e).unwrap_or_default());
        }
        assert!(!files.contains(KEY));
        assert!(done.keys_url.ends_with("/settings/user-api-keys"));
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if e.path().is_dir() { out.extend(walk(&e.path())) } else { out.push(e.path()) }
        }
        out
    }

    #[tokio::test]
    async fn when_posthog_says_no_nothing_is_kept_and_the_person_is_told_the_way_on() {
        let home = scratch("no");
        let metrics = Arc::new(Metrics::new(&home, Box::new(InMemory::default())));
        let p = || PosthogSetupParams { squad: "atinum".into(), project: None, host: "https://us.posthog.com".into(), project_id: 735144, id: None };
        let cases = [
            (Some(json!({ "status": 403, "code": "sensitive_action_required_reauth" })), SetupOutcome::SignInAgain),
            (Some(json!({ "status": 401 })), SetupOutcome::SignIn),
            (Some(json!({ "status": 400, "detail": "Too many keys" })), SetupOutcome::Manual),
            (None, SetupOutcome::Manual),
        ];
        for (key, outcome) in cases {
            let browser = Stand { look: json!({}), key, asked: Default::default() };
            let done = metrics.posthog_setup(Some(&browser), p()).await.unwrap();
            assert_eq!(done.outcome, outcome);
            assert!(done.connection.is_none());
            assert_eq!(done.keys_url, "https://us.posthog.com/settings/user-api-keys", "the page to make one by hand");
        }
        assert!(super::super::connections(&home).is_empty());
        // No browser: by hand.
        assert_eq!(metrics.posthog_setup(None, p()).await.unwrap().outcome, SetupOutcome::Manual);
        // A host that isn't PostHog Cloud's is never sent a key.
        let browser = Stand { look: json!({}), key: Some(json!({ "status": 201, "value": KEY })), asked: Default::default() };
        let bad = PosthogSetupParams { host: "https://posthog.evil.example".into(), ..p() };
        assert!(metrics.posthog_setup(Some(&browser), bad).await.is_err());
        assert!(browser.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn looking_without_a_shared_browser_says_so() {
        let metrics = Metrics::new(&scratch("look"), Box::new(InMemory::default()));
        let look = metrics.posthog_look(None, &PosthogLookParams { squad: "atinum".into(), sign_out: None }).await.unwrap();
        assert!(!look.browser && !look.signed_in());
        let browser = Stand { look: signed_in(json!([{ "id": 1, "name": "A" }])), key: None, asked: Default::default() };
        let look = metrics.posthog_look(Some(&browser), &PosthogLookParams { squad: "atinum".into(), sign_out: None }).await.unwrap();
        assert!(look.browser && look.signed_in());
        assert_eq!(browser.asked.lock().unwrap().len(), 1, "a look asks nothing else");
        // "Sign in again": PostHog's sign-out on that host first — never another host's.
        metrics.posthog_look(Some(&browser), &PosthogLookParams { squad: "atinum".into(), sign_out: Some("https://eu.posthog.com".into()) }).await.unwrap();
        metrics.posthog_look(Some(&browser), &PosthogLookParams { squad: "atinum".into(), sign_out: Some("https://evil.example".into()) }).await.unwrap();
        let asked = browser.asked.lock().unwrap();
        assert_eq!((asked[1]["step"].as_str(), asked[1]["host"].as_str()), (Some("logout"), Some("eu.posthog.com")));
        assert_eq!(asked.iter().filter(|a| a["step"] == "logout").count(), 1);
    }
}
