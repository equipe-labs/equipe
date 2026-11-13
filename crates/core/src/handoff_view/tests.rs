use super::*;

fn cookie(name: &str, domain: &str) -> HandoffCookie {
    HandoffCookie { name: name.into(), value: "s3cret-value".into(), domain: domain.into(), path: "/".into(), expires: None, secure: true, http_only: true, same_site: Some("Lax".into()) }
}

fn names(scoped: &Scoped) -> Vec<&str> {
    scoped.cookies.iter().map(|c| c.name.as_str()).collect()
}

fn hosts(list: &[&str]) -> Vec<String> {
    list.iter().map(|h| (*h).to_owned()).collect()
}

#[test]
fn only_the_sites_cookies_and_the_sign_ins_own_redirects_are_kept() {
    let all = vec![
        cookie("session", "chatgpt.com"),
        cookie("auth", ".auth.openai.com"),
        cookie("sso", ".openai.com"),
        cookie("tracker", ".doubleclick.net"),
        cookie("google", ".google.com"),
        cookie("other", "example.org"),
        cookie("tld", ".com"),
        cookie("bare", "localhost"),
    ];
    // Signed in to chatgpt.com through auth.openai.com: that, its parent, its site — nothing else.
    let scoped = scope_cookies(all.clone(), "https://chatgpt.com/", &hosts(&["auth.openai.com", "chatgpt.com"]));
    assert_eq!(names(&scoped), ["session", "auth", "sso"]);
    assert_eq!(scoped.dropped, 5);
    // The view never went to Google: its cookies are not carried, whatever the store held.
    assert!(!names(&scoped).contains(&"google"));
    // It did, as part of this sign-in (Google sign-in): then they are.
    let through_google = scope_cookies(all, "https://chatgpt.com/", &hosts(&["accounts.google.com", "auth.openai.com", "chatgpt.com"]));
    assert!(names(&through_google).contains(&"google"));
    assert!(!names(&through_google).contains(&"tracker") && !names(&through_google).contains(&"other") && !names(&through_google).contains(&"tld"));
}

#[test]
fn the_site_itself_counts_with_no_host_reported_and_a_sibling_subdomain_does_not() {
    let scoped = scope_cookies(vec![cookie("a", ".example.com"), cookie("b", "app.example.com"), cookie("c", "evil.example.com"), cookie("d", "notexample.com")], "https://app.example.com/login", &[]);
    assert_eq!(names(&scoped), ["a", "b"], "a parent's cookie covers the site; a sibling's doesn't, nor a lookalike");
}

#[test]
fn hosts_are_read_as_hosts_and_junk_counts_for_nothing() {
    let scoped = scope_cookies(vec![cookie("a", "good.example.com"), cookie("b", "bad.example.net")], "", &hosts(&["https://good.example.com:8443/path?x=1", "user@bad.example.net.evil", "no host here", "-bad-.com"]));
    assert_eq!(names(&scoped), ["a"]);
}

#[test]
fn nameless_oversized_and_too_many_cookies_are_left_out() {
    let mut big = cookie("big", "example.com");
    big.value = "x".repeat(MAX_VALUE + 1);
    let mut nameless = cookie("", "example.com");
    nameless.name.clear();
    let many: Vec<HandoffCookie> = (0..MAX_COOKIES + 20).map(|n| cookie(&format!("c{n}"), "example.com")).collect();
    assert!(scope_cookies(vec![big, nameless], "https://example.com/", &[]).cookies.is_empty());
    let scoped = scope_cookies(many, "https://example.com/", &[]);
    assert_eq!(scoped.cookies.len(), MAX_COOKIES);
    assert_eq!(scoped.dropped, 20);
}

#[test]
fn a_cookie_is_never_printed_with_its_value() {
    let c = cookie("session", "chatgpt.com");
    let shown = format!("{c:?}");
    assert!(shown.contains("session") && shown.contains("chatgpt.com") && !shown.contains("s3cret-value"), "{shown}");
    let command = equipe_protocol::Command::BrowserCookies { lane: "acme/pm".into(), hosts: vec![], cookies: vec![c], failed: false };
    assert!(!format!("{command:?}").contains("s3cret-value"));
}

#[test]
fn the_cookies_command_crosses_the_line_whole_and_an_older_screens_ask_still_reads() {
    let command = equipe_protocol::Command::BrowserCookies { lane: "acme/pm".into(), hosts: vec!["chatgpt.com".into()], cookies: vec![cookie("session", "chatgpt.com")], failed: false };
    let line = serde_json::to_string(&command).unwrap();
    assert_eq!(serde_json::from_str::<equipe_protocol::Command>(&line).unwrap(), command);
    // The window event of a daemon from before the view reads on a screen that knows it, and back.
    let old: equipe_protocol::Event = serde_json::from_str(r#"{"type":"browser_window","squad":"acme","lane":"acme/pm","open":true}"#).unwrap();
    assert!(matches!(old, equipe_protocol::Event::BrowserWindow { view: None, challenged: false, open: true, .. }));
}

fn at(base: Instant, secs: f64) -> Instant {
    base + Duration::from_secs_f64(secs)
}

#[test]
fn cookies_come_once_for_a_view_in_progress_and_are_scoped_to_its_site() {
    let t = Instant::now();
    let mut v = Views::default();
    // A hand-off nobody started takes nothing: a screen can't put cookies into a member's browser.
    assert_eq!(v.cookies(t, "acme/pm", &[], vec![cookie("a", "chatgpt.com")], false), Taken::Ignored);
    v.begin(t, "acme/pm", "https://chatgpt.com/auth/login", "chatgpt.com");
    let Taken::Apply { url, scoped } = v.cookies(t, "acme/pm", &hosts(&["auth.openai.com"]), vec![cookie("a", "chatgpt.com"), cookie("x", "evil.test")], false) else { panic!("applied") };
    assert_eq!(url, "https://chatgpt.com/auth/login");
    assert_eq!(names(&scoped), ["a"]);
    // A second set (a replay) takes nothing.
    assert_eq!(v.cookies(t, "acme/pm", &[], vec![cookie("a", "chatgpt.com")], false), Taken::Ignored);
}

#[test]
fn done_hands_the_browser_back_once_the_page_has_loaded_again() {
    let t = Instant::now();
    let mut v = Views::default();
    v.begin(t, "acme/pm", "https://chatgpt.com/", "chatgpt.com");
    v.cookies(at(t, 60.), "acme/pm", &[], vec![cookie("a", "chatgpt.com")], false);
    assert!(v.done(at(t, 60.), "acme/pm"));
    // Not yet: too soon, or the page isn't there.
    assert!(v.ready(at(t, 60.5), |_| true).is_empty());
    assert!(v.ready(at(t, 63.), |_| false).is_empty());
    // The page is there.
    assert_eq!(v.ready(at(t, 63.), |_| true), [("acme/pm".to_owned(), false)]);
    assert!(v.is_empty());
}

#[test]
fn a_page_that_never_loads_is_handed_back_after_the_wait_and_done_without_cookies_at_once() {
    let t = Instant::now();
    let mut v = Views::default();
    v.begin(t, "acme/pm", "https://chatgpt.com/", "chatgpt.com");
    v.cookies(t, "acme/pm", &[], vec![], false);
    v.done(t, "acme/pm");
    assert!(v.ready(at(t, BACK_WAIT.as_secs_f64() - 1.), |_| false).is_empty());
    assert_eq!(v.ready(at(t, BACK_WAIT.as_secs_f64()), |_| false).len(), 1);
    // The person closed the view and no cookies came (the screen broke): nothing to wait for.
    v.begin(t, "acme/qa", "https://x.example/", "x.example");
    v.done(t, "acme/qa");
    assert_eq!(v.ready(t, |_| false), [("acme/qa".to_owned(), false)]);
}

#[test]
fn a_view_that_could_not_open_is_over_and_a_view_nobody_answers_is_given_up() {
    let t = Instant::now();
    let mut v = Views::default();
    v.begin(t, "acme/pm", "https://chatgpt.com/", "chatgpt.com");
    assert_eq!(v.cookies(t, "acme/pm", &[], vec![], true), Taken::Failed);
    assert!(!v.holds("acme/pm"));
    v.begin(t, "acme/qa", "https://x.example/", "x.example");
    assert!(v.ready(at(t, 60.), |_| true).is_empty(), "the person is still signing in");
    assert_eq!(v.ready(t + VIEW_WAIT, |_| true), [("acme/qa".to_owned(), true)]);
}

#[test]
fn two_members_each_have_a_view_and_a_screen_that_goes_ends_them_all() {
    let t = Instant::now();
    let mut v = Views::default();
    v.begin(t, "acme/pm", "https://a.example/", "a.example");
    v.begin(t, "acme/qa", "https://b.example/", "b.example");
    v.done(t, "acme/pm");
    assert_eq!(v.ready(t, |_| true), [("acme/pm".to_owned(), false)]);
    assert!(v.holds("acme/qa"));
    assert_eq!(v.end_all(), [("acme/qa".to_owned(), "https://b.example/".to_owned())]);
    assert!(v.is_empty());
    assert!(!v.done(t, "acme/qa"), "no view: Done is for the panel");
}
