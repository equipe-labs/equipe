//! Verification codes on the daemon (`crate::verify`): a member's page that
//! waits on a code the site emailed — seen by equipe, or named by the member
//! with `block_on_person`'s `verification` — gets ONE card for the person.
//! Only their click on "Fill code from email" reads the matching mail and
//! types the code into that page; the member is then told the person
//! confirmed access, never the code. "I'll type it" puts the page in their
//! hands instead. Connecting Gmail (read-only) is here too.
//!
//! Cards live as long as the page they stand for: they aren't journaled.

use std::collections::HashSet;

use crate::autopilot::mail;
use crate::protocol::vault::{Gmail, GmailClient};
use crate::protocol::verify::{Channel, MailboxRef, Phase, Verification, site_name};
use crate::verify::{self as vf, gmail};

use super::*;

/// How long a filled card stays, to be seen ticked — and long enough for
/// the page to move on; a page still asking for the code then didn't take it.
const TICKED_FOR: Duration = Duration::from_secs(20);
/// How long a code stays cut out of members' words after it was fetched.
const MASKED_FOR: Duration = Duration::from_secs(vf::WINDOW_SECS);

/// The cards and the Gmail connect under way.
#[derive(Default)]
pub(super) struct State {
    /// Cards, by squad.
    boards: HashMap<String, vf::Board>,
    /// Cards a member asked for (`block_on_person`): they stay while the page
    /// is open, whatever equipe reads off it.
    asked: HashSet<u64>,
    /// Lanes whose code equipe filled while the page was in the person's
    /// hands: when it comes back, its member hears "the person confirmed
    /// access" instead of the usual hand-back.
    confirmed: HashMap<String, (SessionId, String, String)>,
    /// When the mail was asked for, by card (unix seconds).
    requested: HashMap<u64, u64>,
    next: u64,
    connecting: Option<JoinHandle<()>>,
}

/// What comes back to the loop.
pub(super) enum Note {
    /// A click's look in the mailbox and fill ended.
    Done { squad: String, id: u64, outcome: vf::Outcome },
    /// A filled card has been seen long enough.
    Sweep { squad: String, id: u64 },
    /// Connecting Gmail: the consent page to open.
    ConnectUrl(String),
    /// Connecting Gmail ended: connected as whom (and the grant to keep), or why not.
    Connected(Result<(Gmail, gmail::Grant), String>),
}

/// The mailbox in use: Gmail by its API, or IMAP. Holds secrets: no `Debug`.
enum Source {
    Gmail { http: reqwest::Client, grant: gmail::Grant },
    Imap { host: String, port: u16, user: String, password: String },
}

impl vf::Mailbox for Source {
    async fn look(&self, wanted: &vf::Wanted, since: u64) -> Result<Vec<mail::Message>, String> {
        match self {
            Source::Gmail { http, grant } => gmail::look(http, grant, wanted, since).await,
            Source::Imap { host, port, user, password } => mail::matching_once(host, *port, user, password, since, &wanted.domains()).await,
        }
    }
}

/// The member's tab, through the browser host's `fill` (its tools held off the tab while it types).
struct HostTab {
    host: browser::SharedHost,
    waits: logins::FillWaits,
    lane: String,
    page: u64,
    token: u64,
}

impl vf::Page for HostTab {
    async fn run(&self, script: String) -> Result<vault::Filled, String> {
        super::autopilot::fill_once(&self.host, &self.waits, &self.lane, self.page, script, self.token).await
    }
}

/// What the person's mailbox is, as a card names it, and its webmail link for `site`.
fn mailbox_ref(settings: &crate::autopilot::Settings, site: &str) -> (Option<MailboxRef>, Option<String>) {
    if let Some(g) = &settings.gmail {
        return (Some(MailboxRef { address: g.address.clone(), kind: "gmail".into() }), vf::open_url("gmail", &g.address, None, site));
    }
    if let Some(m) = &settings.mailbox {
        return (Some(MailboxRef { address: m.username.clone(), kind: "imap".into() }), vf::open_url("imap", &m.username, Some(&m.host), site));
    }
    (None, None)
}

impl Runtime {
    fn verify_board(&mut self, squad: &str) -> &mut vf::Board {
        self.verify.boards.entry(squad.to_owned()).or_default()
    }

    /// `squad`'s cards, for a screen attaching.
    pub(super) fn verification_cards(&self, squad: &str) -> Vec<Verification> {
        self.verify.boards.get(squad).map(|b| b.cards().to_vec()).unwrap_or_default()
    }

    fn publish_verification(&mut self, squad: &str, id: u64) {
        if let Some(card) = self.verify.boards.get(squad).and_then(|b| b.get(id)).cloned() {
            self.publish(Event::VerificationChanged { squad: squad.to_owned(), card });
        }
    }

    fn close_verification(&mut self, squad: &str, id: u64) -> Option<Verification> {
        let card = self.verify.boards.get_mut(squad)?.close(id)?;
        self.verify.asked.remove(&id);
        self.verify.requested.remove(&id);
        self.publish(Event::VerificationClosed { squad: squad.to_owned(), id });
        Some(card)
    }

    /// Put a card up for `lane`'s `page` on `site` — or keep the one up. Its id.
    fn raise_verification(&mut self, squad: &str, lane: &str, session: Option<SessionId>, page: Option<u64>, site: &str, asked: bool) -> u64 {
        if let Some(id) = self.verify.boards.get(squad).and_then(|b| b.of(lane, site)).map(|c| c.id) {
            if asked {
                self.verify.asked.insert(id);
            }
            return id;
        }
        let settings = self.vault.lock().map(|v| v.settings.clone()).unwrap_or_default();
        let (mailbox, open_url) = mailbox_ref(&settings, site);
        let member = session.and_then(|s| self.sessions.get(&s)).and_then(|s| s.name().map(str::to_owned)).unwrap_or_else(|| browser::member_of(lane.split_once('/').map_or(lane, |(_, k)| k)).to_owned());
        let ticket = session.and_then(|s| self.sessions.get(&s)).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        self.verify.next += 1;
        let now = now_secs();
        let card = Verification {
            id: now * 1000 + self.verify.next % 1000,
            session,
            member,
            ticket,
            site: site.to_owned(),
            name: site_name(site),
            channel: Channel::Email,
            mailbox,
            open_url,
            lane: lane.to_owned(),
            page,
            phase: Phase::Waiting,
            detail: None,
            at: now,
        };
        let id = card.id;
        let (new, _) = self.verify_board(squad).raise(card);
        if new {
            if asked {
                self.verify.asked.insert(id);
            }
            self.verify.requested.insert(id, now);
            crate::daemon_log!("equipe: {site} emailed a verification code for a member's page; the person is asked on a card");
            self.publish_verification(squad, id);
        }
        id
    }

    /// The host's word on a lane's pages: a page waiting on an emailed code
    /// gets its card; a card whose page moved on (the person typed the code,
    /// the member went elsewhere) or closed goes.
    pub(super) fn see_verification_pages(&mut self, lane: &str, pages: &[browser::HostPage]) {
        if lane.is_empty() || lane.starts_with("equipe:") || self.relogins.contains_key(lane) {
            return;
        }
        let (Some(squad), session) = self.lane_session(lane) else { return };
        for page in pages.iter().filter(|p| !p.loading) {
            if let Some(site) = vf::email_step(&page.url, &page.title, page.signals.as_ref()) {
                self.raise_verification(&squad, lane, session, Some(page.id), &site, false);
            }
        }
        let gone: Vec<u64> = self
            .verify
            .boards
            .get(&squad)
            .map(|b| {
                b.of_lane(lane)
                    .into_iter()
                    .filter(|id| {
                        let Some(card) = b.get(*id) else { return false };
                        if !card.phase.open() {
                            return false;
                        }
                        let here = card.page.and_then(|p| pages.iter().find(|pg| pg.id == p));
                        match here {
                            None => card.page.is_some(),
                            Some(_) if self.verify.asked.contains(id) => false,
                            Some(pg) => !pg.loading && vf::email_step(&pg.url, &pg.title, pg.signals.as_ref()).is_none_or(|s| s != card.site),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        for id in gone {
            self.close_verification(&squad, id);
        }
    }

    /// A member's `block_on_person` with `verification`: the card for its
    /// browser's page on that site; the ticket stays the member's.
    pub(super) fn verification_asked(&mut self, squad: &str, id: SessionId, asked: &Value) -> Result<String, String> {
        let site_typed = asked.get("site").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or("Say which site emailed the code: `verification: {site: \"github.com\", channel: \"email\"}`.")?;
        let channel = asked.get("channel").and_then(Value::as_str).unwrap_or("email");
        if Channel::parse(channel).is_none() {
            return Err("Only a code the site emailed can be filled from the person's mailbox. A code on their phone or in an authenticator app is theirs to type: hand that step to them with `show_my_browser`, and never ask for the code in chat.".into());
        }
        let site = vault::site_of(site_typed);
        let (_, key) = self.browser_key(id).ok_or("You have no browser in this conversation.")?;
        let lane = format!("{squad}/{key}");
        let page = match self.tabs.get(&lane) {
            Some(Event::BrowserTabs { pages, agent_page, .. }) => {
                let on_site = |p: &&equipe_protocol::BrowserPage| vault::serves(&site, &p.url) || vault::site_of(&p.url) == site;
                agent_page.and_then(|a| pages.iter().find(|p| p.id == a)).filter(on_site).or_else(|| pages.iter().find(on_site)).map(|p| p.id)
            }
            _ => None,
        };
        let Some(page) = page else {
            return Err(format!("Your browser has no tab on {site}. Open the page that asks for the emailed code first, then call this again."));
        };
        let card = self.raise_verification(squad, &lane, Some(id), Some(page), &site, true);
        let connected = self.verify.boards.get(squad).and_then(|b| b.get(card)).is_some_and(|c| c.mailbox.is_some());
        let how = if connected {
            "with one click equipe reads the code from their connected mailbox and types it into your page"
        } else {
            "their mailbox isn't connected to equipe yet, so the card offers to connect it, or they type the code into your page themselves"
        };
        Ok(format!(
            "The person has a card: \"{} sent a verification code\" — {how}. You never see, receive or type the code, and never ask for it in chat. \
Your ticket stays yours. End your turn now with one short line saying you wait for them to confirm access on {site}; you're told when \
they have, and carry on in this browser.",
            site_name(&site)
        ))
    }

    /// "Fill code from email": the one way a code is fetched and typed.
    pub(super) fn fill_verification(&mut self, squad: &str, id: u64) {
        if self.verify_board(squad).click(id) != vf::Act::Fetch {
            return;
        }
        let Some(card) = self.verify.boards.get(squad).and_then(|b| b.get(id)).cloned() else { return };
        let (Some(host), Some(page), Some(mailbox)) = (self.host.clone(), card.page, card.mailbox.clone()) else {
            self.verification_ended(squad, id, vf::Outcome::Failed("the page isn't open in equipe's browser any more".into()));
            return;
        };
        self.publish_verification(squad, id);
        let requested_at = self.verify.requested.get(&id).copied().unwrap_or(card.at);
        let mail_from = self.logins.iter().find(|l| vault::serves(&l.site, &card.site)).and_then(|l| l.autopilot.mail_from.clone());
        let wanted = vf::Wanted { site: card.site.clone(), mail_from, requested_at };
        let settings = self.vault.lock().map(|v| v.settings.clone()).unwrap_or_default();
        let (store, internal, waits) = (self.vault.clone(), self.internal.clone(), self.fill_waits.clone());
        let tab = HostTab { host, waits, lane: card.lane.clone(), page, token: self.next_fill_token() };
        let squad_s = squad.to_owned();
        crate::daemon_log!("equipe: the person asked equipe to fill {}'s emailed verification code ({})", card.site, mailbox.kind);
        tokio::spawn(async move {
            // The mailbox's secret is read off the loop: the secret store may ask the person.
            let is_gmail = mailbox.kind == "gmail";
            let source = tokio::task::spawn_blocking(move || -> Result<Source, String> {
                let v = store.lock().map_err(|_| "the logins are unreadable".to_owned())?;
                if is_gmail {
                    let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
                    return Ok(Source::Gmail { http, grant: v.gmail_grant()? });
                }
                let m = settings.mailbox.ok_or("no mailbox is connected")?;
                Ok(Source::Imap { host: m.host, port: m.port, user: m.username, password: v.secret(vault::account::MAILBOX)? })
            })
            .await
            .map_err(|_| "stopped".to_owned())
            .and_then(|r| r);
            let mask = format!("verify:{squad_s}:{id}");
            let outcome = match source {
                Ok(source) => vf::fill_from_mail(&source, &tab, &wanted, &mask, vf::WAIT, vf::TICK, now_secs).await,
                Err(why) => vf::Outcome::Failed(why),
            };
            let _ = internal.send(Internal::Verify(Note::Done { squad: squad_s, id, outcome }));
            // A code fetched stays cut out of members' words a while, then the watch goes.
            tokio::time::sleep(MASKED_FOR).await;
            crate::squad_secrets::unwatch(&mask);
        });
    }

    fn verification_ended(&mut self, squad: &str, id: u64, outcome: vf::Outcome) {
        let Some(card) = self.verify.boards.get_mut(squad).and_then(|b| b.get_mut(id)) else { return };
        let site = card.site.clone();
        match &outcome {
            vf::Outcome::Filled { .. } => {
                card.phase = Phase::Filled;
                card.detail = None;
            }
            vf::Outcome::NoMail => {
                card.phase = Phase::NoMail;
                card.detail = None;
            }
            vf::Outcome::Refused(why) | vf::Outcome::Failed(why) => {
                card.phase = Phase::Failed;
                card.detail = Some(why.chars().take(160).collect());
            }
        }
        let card = card.clone();
        crate::daemon_log!(
            "equipe: {site}'s emailed code: {}",
            match &outcome {
                vf::Outcome::Filled { submitted: true } => "filled and sent".to_owned(),
                vf::Outcome::Filled { submitted: false } => "filled".to_owned(),
                vf::Outcome::NoMail => "no matching mail came in time".to_owned(),
                vf::Outcome::Refused(why) => format!("not typed ({why})"),
                vf::Outcome::Failed(why) => format!("failed ({why})"),
            }
        );
        self.publish_verification(squad, id);
        if card.phase == Phase::Filled {
            // The member hears it once the page is back in its hands — at
            // once when it never left them.
            if let Some(session) = card.session {
                let prompt = format!(
                    "[equipe: the person confirmed access on {}: on their click equipe filled the verification code {} emailed. You never \
see that code, and never ask anyone for one. Your browser is yours; check the page yourself (snapshot or read it) and carry on where you left off.]",
                    card.site, card.name
                );
                if self.checking.contains_key(&card.lane) {
                    self.verify.confirmed.insert(card.lane.clone(), (session, prompt, card.name.clone()));
                } else {
                    let notice = say!(self, "notice.verify_confirmed", site = card.name);
                    let (internal, notice_prompt) = (self.internal.clone(), prompt);
                    self.publish(Event::Notice { undo: None, session, text: notice });
                    let _ = internal.send(Internal::Wake { session, prompt: notice_prompt });
                }
            }
            let (internal, swept) = (self.internal.clone(), squad.to_owned());
            tokio::spawn(async move {
                tokio::time::sleep(TICKED_FOR).await;
                let _ = internal.send(Internal::Verify(Note::Sweep { squad: swept, id }));
            });
        }
    }

    /// A lane came back from the person after equipe filled its code: its
    /// member is told the person confirmed access. `true`: told, so the usual
    /// hand-back isn't said too.
    pub(super) async fn verification_handed_back(&mut self, lane: &str) -> bool {
        let Some((session, prompt, name)) = self.verify.confirmed.remove(lane) else { return false };
        let notice = say!(self, "notice.verify_confirmed", site = name);
        self.carry_on(session, notice, prompt).await;
        true
    }

    /// "I'll type it": the card closes, and the page is the person's to type in.
    pub(super) fn type_verification(&mut self, squad: &str, id: u64) {
        let Some(card) = self.close_verification(squad, id) else { return };
        let Some(page) = card.page else { return };
        if !self.checking.contains_key(&card.lane) && !self.asked_in.contains_key(&card.lane) {
            // Not yet in their hands: handed to them, and its member told when they give it back.
            self.asked_in.insert(card.lane.clone(), format!("type the verification code {} emailed", card.name));
            self.hub.op(&json!({ "op": "control", "lane": card.lane, "by": "person", "why": "check", "page": page }));
        }
        // The page's code field takes their typing.
        let token = self.next_fill_token();
        self.hub.op(&json!({ "op": "fill", "lane": card.lane, "page": page, "script": crate::autopilot::fill::focus_code_script(&vf::guard_for(&card.site)), "token": token }));
    }

    pub(super) async fn verify_note(&mut self, note: Note) {
        match note {
            Note::Done { squad, id, outcome } => self.verification_ended(&squad, id, outcome),
            Note::Sweep { squad, id } => {
                let Some(card) = self.verify.boards.get(&squad).and_then(|b| b.get(id)).cloned() else { return };
                let still_asks = card.page.is_some_and(|p| self.checking.get(&card.lane).is_some_and(|c| c.0 == p));
                if card.phase == Phase::Filled && still_asks {
                    // The page still waits on the code: the site didn't take it. The person decides.
                    self.verify.confirmed.remove(&card.lane);
                    if let Some(c) = self.verify.boards.get_mut(&squad).and_then(|b| b.get_mut(id)) {
                        c.phase = Phase::Failed;
                        c.detail = Some("not_taken".into());
                    }
                    self.publish_verification(&squad, id);
                } else if card.phase == Phase::Filled {
                    self.close_verification(&squad, id);
                }
            }
            Note::ConnectUrl(url) => self.publish(Event::MailboxConnect { url: Some(url), error: None }),
            Note::Connected(Ok((gmail, grant))) => {
                self.verify.connecting = None;
                crate::daemon_log!("equipe: Gmail is connected read-only for verification codes");
                self.vault_then_status_verify(move |v| v.save_gmail(Some((gmail, grant))));
            }
            Note::Connected(Err(why)) => {
                self.verify.connecting = None;
                crate::daemon_log!("equipe: Gmail wasn't connected: {why}");
                self.publish(Event::MailboxConnect { url: None, error: Some(why) });
            }
        }
    }

    /// A vault change off the loop, then login autopilot's status (which carries the mailbox) again.
    fn vault_then_status_verify(&mut self, change: impl FnOnce(&mut vault::Vault) -> Result<(), String> + Send + 'static) {
        let (vault, internal) = (self.vault.clone(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let result = vault.lock().map_err(|_| "the logins are unreadable".to_owned()).and_then(|mut v| change(&mut v));
            let _ = internal.send(Internal::Autopilot(super::autopilot::Note::Changed(result)));
        });
    }

    /// Connect Gmail: Google's consent page in the person's own browser,
    /// back to a one-off listener on this machine, the grant kept.
    pub(super) fn connect_gmail(&mut self, client: Option<GmailClient>) {
        let client = client.map(|c| gmail::Client { id: c.id.trim().to_owned(), secret: c.secret.trim().to_owned() }).filter(|c| !c.id.is_empty()).or_else(gmail::builtin);
        let Some(client) = client else {
            let error = say!(self, "error.gmail_no_client");
            self.publish(Event::MailboxConnect { url: None, error: Some(error) });
            return;
        };
        if let Some(task) = self.verify.connecting.take() {
            task.abort();
        }
        let internal = self.internal.clone();
        let (ok_title, ok_body, no_title) = (say!(self, "verify.connected_title"), say!(self, "verify.connected_body"), say!(self, "verify.failed_title"));
        let no_body = crate::i18n::core(self.config.lang).get("verify.failed_body").to_owned();
        self.verify.connecting = Some(tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            let result: Result<(Gmail, gmail::Grant), String> = async {
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| e.to_string())?;
                let port = listener.local_addr().map_err(|e| e.to_string())?.port();
                let flow = gmail::Flow::new(port)?;
                let _ = internal.send(Internal::Verify(Note::ConnectUrl(flow.consent_url(&client.id))));
                let (line, mut stream) = tokio::time::timeout(gmail::CONNECT_WAIT, gmail::redirect(&listener)).await.map_err(|_| "it took longer than five minutes".to_owned())??;
                let done: Result<(Gmail, gmail::Grant), String> = async {
                    let code = flow.code_from(&line)?;
                    let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
                    let grant = gmail::exchange(&http, &client, &flow, &code).await?;
                    let token = gmail::access(&http, &grant).await?;
                    let address = gmail::address(&http, &token).await?;
                    Ok((Gmail { address, client_id: client.id.clone() }, grant))
                }
                .await;
                let page = match &done {
                    Ok(_) => gmail::redirect_page(&ok_title, &ok_body),
                    Err(why) => gmail::redirect_page(&no_title, &no_body.replace("{why}", why)),
                };
                let _ = stream.write_all(page.as_bytes()).await;
                let _ = stream.shutdown().await;
                done
            }
            .await;
            let _ = internal.send(Internal::Verify(Note::Connected(result)));
        }));
    }

    pub(super) fn disconnect_gmail(&mut self) {
        if let Some(task) = self.verify.connecting.take() {
            task.abort();
        }
        self.vault_then_status_verify(|v| v.save_gmail(None));
    }
}
