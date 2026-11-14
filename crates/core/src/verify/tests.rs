use std::sync::Mutex;
use std::time::Duration;

use super::*;
use crate::protocol::verify::{Channel, MailboxRef, Phase, Verification};

const NOW: u64 = 1_794_528_600;

fn signals(text: &str, otp: bool) -> Signals {
    Signals { otp, code: otp, text: text.to_lowercase(), ..Signals::default() }
}

fn mail(from: &str, ago: u64, subject: &str, text: &str) -> Message {
    Message { from: from.into(), at: NOW - ago, subject: subject.into(), text: text.into() }
}

fn github() -> Wanted {
    Wanted { site: "github.com".into(), mail_from: None, requested_at: NOW - 30 }
}

#[test]
fn githubs_sudo_page_googles_verify_page_and_any_we_emailed_page_are_email_steps() {
    let sudo = signals("Confirm access. We sent a verification code to your email m•••@example.com. Verification code", true);
    assert_eq!(email_step("https://github.com/settings/tokens/new", "Confirm access", Some(&sudo)).as_deref(), Some("github.com"));
    let google = signals("Verify it's you. An email with a verification code was just sent to s•••@example.com. Enter code", true);
    assert_eq!(email_step("https://accounts.google.com/v3/signin/challenge/ipe/verify", "Sign in - Google Accounts", Some(&google)).as_deref(), Some("accounts.google.com"));
    let generic = signals("Enter the 6-digit code we emailed to you", true);
    assert_eq!(email_step("https://app.example.io/login/verify", "Verify", Some(&generic)).as_deref(), Some("app.example.io"));
}

#[test]
fn a_code_from_a_phone_or_an_app_a_page_without_a_field_and_http_are_not() {
    let app = signals("Two-factor authentication. Open your authenticator app and enter the code. Lost access? Use your email recovery", true);
    assert_eq!(email_step("https://github.com/sessions/two-factor/app", "Two-factor", Some(&app)), None, "an authenticator code is the person's");
    let sms = signals("We sent a code by text message to your phone", true);
    assert_eq!(email_step("https://example.com/verify", "Verify", Some(&sms)), None);
    let no_field = signals("We emailed you a link. Check your inbox.", false);
    assert_eq!(email_step("https://example.com/verify", "Verify", Some(&no_field)), None, "a magic link isn't a code");
    let emailed = signals("Enter the code we emailed you", true);
    assert_eq!(email_step("http://example.com/verify", "Verify", Some(&emailed)), None, "https only");
    assert_eq!(email_step("https://example.com/verify", "Verify", None), None);
}

#[test]
fn only_the_sites_recent_verification_mail_counts_and_the_newest_wins() {
    let w = github();
    let inbox = [
        mail("Bank <alerts@bank.example>", 20, "Your verification code", "Code: 111111"),
        mail("x <noreply@github.com.evil.io>", 10, "[GitHub] Sudo email verification code", "Verification code: 222222"),
        mail("GitHub <noreply@github.com>", 900, "[GitHub] Sudo email verification code", "Verification code: 333333"),
        mail("GitHub <noreply@github.com>", 200, "[GitHub] Sudo email verification code", "Verification code: 444444"),
        mail("GitHub <noreply@github.com>", 25, "[GitHub] Sudo email verification code", "Verification code: 55512345"),
        mail("GitHub <noreply@github.com>", 5, "[GitHub] Your weekly digest", "12 new stars on 2 repositories since 202611"),
    ];
    let found = pick(&inbox, &w, NOW).expect("a code");
    assert_eq!(found.code, "55512345", "the newest of the site's own verification mail since it was asked");
    assert!(!format!("{found:?}").contains("55512345"), "Debug hides the code");
    // Ten minutes is the window, whatever the request time says.
    let late = Wanted { requested_at: NOW - 3_600, ..github() };
    assert_eq!(pick(&[mail("noreply@github.com", 700, "Verification code", "Code: 777777")], &late, NOW), None);
    // Mail from before the code was asked for (beyond the allowance) isn't this page's.
    assert_eq!(pick(&[mail("noreply@github.com", 30 + EARLY_SECS + 5, "Verification code", "Code: 888888")], &w, NOW), None);
    // From the site, but not a code: passed over.
    assert_eq!(pick(&[mail("noreply@github.com", 5, "Invoice", "Order total $123456 on 11/13")], &w, NOW), None);
    // A sender the person named for the site.
    let aws = Wanted { site: "console.aws.amazon.com".into(), mail_from: Some("signin.aws".into()), requested_at: NOW - 30 };
    assert_eq!(pick(&[mail("no-reply@signin.aws", 5, "AWS verification code", "Your code is 909192")], &aws, NOW).map(|p| p.code).as_deref(), Some("909192"));
}

/// A mailbox that answers with what it holds, and counts how often it was asked.
struct FakeMailbox {
    inbox: Vec<Message>,
    asked: Mutex<Vec<(Vec<String>, u64)>>,
    refuse: bool,
}

impl Mailbox for FakeMailbox {
    async fn look(&self, wanted: &Wanted, since: u64) -> Result<Vec<Message>, String> {
        self.asked.lock().unwrap().push((wanted.domains(), since));
        if self.refuse {
            return Err("the mailbox refused the app password".into());
        }
        // A real mailbox is searched by sender on its side; the fake returns
        // everything, to show the matcher passes the rest over too.
        Ok(self.inbox.clone())
    }
}

/// A page that keeps the scripts it was given and answers as a page would.
struct FakePage {
    scripts: Mutex<Vec<String>>,
    answer: Filled,
}

impl Page for FakePage {
    async fn run(&self, script: String) -> Result<Filled, String> {
        self.scripts.lock().unwrap().push(script);
        Ok(self.answer.clone())
    }
}

fn typed() -> Filled {
    Filled { filled: vec!["code".into()], submitted: true, step: Some("code".into()), ..Filled::default() }
}

#[tokio::test]
async fn a_click_types_the_newest_code_into_the_page_and_nowhere_else() {
    let mailbox = FakeMailbox { inbox: vec![mail("Fake <noreply@other.example>", 3, "Your code", "Code: 121212"), mail("GitHub <noreply@github.com>", 4, "[GitHub] Sudo email verification code", "Verification code: 48213907")], asked: Mutex::new(Vec::new()), refuse: false };
    let page = FakePage { scripts: Mutex::new(Vec::new()), answer: typed() };
    let outcome = fill_from_mail(&mailbox, &page, &github(), "verify:test:1", Duration::from_millis(50), Duration::from_millis(10), || NOW).await;
    assert_eq!(outcome, Outcome::Filled { submitted: true });
    let scripts = page.scripts.lock().unwrap();
    assert_eq!(scripts.len(), 1);
    assert!(scripts[0].contains(r#"const C = "48213907";"#) && scripts[0].contains(r#"const HOSTS = ["github.com"], SUB = false, HTTP = false, REMEMBER = false;"#), "the page's own host, https");
    assert!(!scripts[0].contains("121212"), "another sender's code never reaches the page");
    // What comes back from the click, and what a member may see, carry no code.
    assert!(!format!("{outcome:?}").contains("48213907"));
    let member_wrote = crate::squad_secrets::mask("the page now says 48213907 was accepted");
    assert!(!member_wrote.contains("48213907"), "cut out of a member's words while the card lives: {member_wrote}");
    crate::squad_secrets::unwatch("verify:test:1");
    // The mailbox was asked only for the site's sender, since the code was asked for.
    let asked = mailbox.asked.lock().unwrap();
    assert_eq!(asked[0], (vec!["github.com".to_owned()], github().earliest(NOW)));
}

#[tokio::test]
async fn no_matching_mail_in_time_says_so_and_types_nothing() {
    let mailbox = FakeMailbox { inbox: vec![mail("GitHub <noreply@github.com>", 2_000, "Verification code", "Code: 313131")], asked: Mutex::new(Vec::new()), refuse: false };
    let page = FakePage { scripts: Mutex::new(Vec::new()), answer: typed() };
    let outcome = fill_from_mail(&mailbox, &page, &github(), "verify:test:2", Duration::from_millis(40), Duration::from_millis(10), || NOW).await;
    assert_eq!(outcome, Outcome::NoMail);
    assert!(page.scripts.lock().unwrap().is_empty());
    assert!(mailbox.asked.lock().unwrap().len() >= 2, "looked again while it waited");
    let refusing = FakeMailbox { inbox: Vec::new(), asked: Mutex::new(Vec::new()), refuse: true };
    let outcome = fill_from_mail(&refusing, &page, &github(), "verify:test:3", Duration::from_millis(40), Duration::from_millis(10), || NOW).await;
    assert_eq!(outcome, Outcome::Failed("the mailbox refused the app password".into()));
}

#[tokio::test]
async fn a_page_on_another_host_or_without_a_field_takes_nothing() {
    let mailbox = FakeMailbox { inbox: vec![mail("noreply@github.com", 4, "Verification code", "Code: 48213907")], asked: Mutex::new(Vec::new()), refuse: false };
    let elsewhere = FakePage { scripts: Mutex::new(Vec::new()), answer: Filled { refused: Some("host".into()), ..Filled::default() } };
    let outcome = fill_from_mail(&mailbox, &elsewhere, &github(), "verify:test:4", Duration::from_millis(20), Duration::from_millis(10), || NOW).await;
    assert_eq!(outcome, Outcome::Refused("host".into()));
    let blank = FakePage { scripts: Mutex::new(Vec::new()), answer: Filled { step: Some("code".into()), ..Filled::default() } };
    let outcome = fill_from_mail(&mailbox, &blank, &github(), "verify:test:5", Duration::from_millis(20), Duration::from_millis(10), || NOW).await;
    assert_eq!(outcome, Outcome::Refused("no_field".into()));
    crate::squad_secrets::unwatch("verify:test:4");
    crate::squad_secrets::unwatch("verify:test:5");
}

fn card(id: u64, mailbox: bool) -> Verification {
    Verification {
        id,
        session: Some(1),
        member: "backend".into(),
        ticket: None,
        site: "github.com".into(),
        name: "GitHub".into(),
        channel: Channel::Email,
        mailbox: mailbox.then(|| MailboxRef { address: "squad@example.com".into(), kind: "gmail".into() }),
        open_url: None,
        lane: "acme/backend".into(),
        page: Some(4),
        phase: Phase::Waiting,
        detail: None,
        at: NOW,
    }
}

#[test]
fn only_the_persons_click_fetches_a_code_and_only_once_at_a_time() {
    let mut board = Board::default();
    assert_eq!(board.raise(card(1, true)), (true, Act::Wait), "raising a card never fetches");
    assert_eq!(board.raise(card(2, true)), (false, Act::Wait), "one card per lane and site");
    assert_eq!(board.cards().len(), 1);
    assert_eq!(board.click(1), Act::Fetch);
    assert_eq!(board.get(1).unwrap().phase, Phase::Fetching);
    assert_eq!(board.click(1), Act::Wait, "not again while it fetches");
    board.get_mut(1).unwrap().phase = Phase::NoMail;
    assert_eq!(board.click(1), Act::Fetch, "pressed again after no mail came");
    assert_eq!(board.click(99), Act::Wait);
    let mut bare = Board::default();
    bare.raise(card(3, false));
    assert_eq!(bare.click(3), Act::Wait, "without a mailbox there is nothing to fetch from");
    assert_eq!(board.of_lane("acme/backend"), vec![1]);
    assert!(board.close(1).is_some() && board.cards().is_empty());
}

#[test]
fn open_email_searches_the_mailboxs_own_web_page_for_the_sites_mail() {
    let url = open_url("gmail", "squad@example.com", None, "github.com").unwrap();
    assert_eq!(url, "https://mail.google.com/mail/?authuser=squad%40example.com#search/from%3Agithub.com%20newer_than%3A1h");
    assert!(open_url("imap", "me@gmail.com", Some("imap.gmail.com"), "accounts.google.com").unwrap().contains("from%3Agoogle.com"));
    assert_eq!(open_url("imap", "me@example.org", Some("mail.example.org"), "github.com"), None);
}
