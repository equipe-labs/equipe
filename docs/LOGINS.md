# Logging in once

How equipe keeps members signed in to the sites they work on, so the person
signs in once and isn't asked again for the same thing. The pieces:

| Piece | Code |
|---|---|
| Logins (site, username, password in the system's secret store) | `crates/core/src/vault.rs`, `crates/protocol/src/vault.rs` |
| Why the person was asked to sign in, and the measures | `crates/core/src/logins.rs`, `equipe logins stats` |
| Offers to keep a login, signing in by itself, one card per site | `crates/core/src/runtime/logins.rs` |
| The browser host's fill and session cookies | `crates/core/assets/browser-host.cjs`, `keep-sessions.js`, `gate.js` |
| Signing in to a site in equipe's own browser (`equipe browser`, the app's "Sign in to a site") | `crates/core/src/signin.rs`, `Runtime::start_sign_in` (`crates/core/src/runtime.rs`), `follow_person_tabs` (`runtime/logins.rs`) |
| Cards, the Logins page, Settings › Accounts | `crates/desktop/src/views/login_offers.rs`, `views/logins.rs` (logic: `crates/desktop/src/logins.rs`), `views/accounts.rs` |
| Login autopilot: codes, passkey, keep-alive, command-line sign-ins | `crates/core/src/autopilot/`, `crates/core/src/runtime/autopilot.rs`, `views/autopilot.rs` (on the Logins page) |

## Invariants

- A password never reaches a member, a screen other than equipe's own entry
  form, an event, telemetry or a log. The fill script answers what it did,
  never the values; `LoginDraft`'s `Debug` hides the password; the sign-in
  record keeps hosts, member names and causes only.
- Every allowance is the person's: a card's answer or a Settings control,
  sent as a person-only command (`SaveLogin`, `AllowLogin`, `SetLoginAuto`,
  `SetLoginScope`, `ForgetLoginAllowance`, `DismissLoginOffer`, `ForgetNeverOffer`; `permissions.rs`). Nothing a
  member says or a page shows turns one on.
- Passwords live in the system's secret store only — the Keychain on macOS,
  Credential Manager on Windows, the Secret Service (libsecret's
  `secret-tool`) on Linux. Where there is none, saving a login fails and
  says why; nothing falls back to a file.

## The Logins page

One page for everything about the logins, reached from the sidebar's "More"
(Logins), `/logins` in the command palette, a "Manage logins" link on every
"Save this login?" card, and Settings › Accounts (which keeps one line that
opens it). Top to bottom:

- **Sign in to a site** — the page's main button (also `/sign-in [site]` in
  the command palette and a line in Settings › Accounts). It asks for a site
  or an address, with quick picks: the sites members waited on or were sent
  to sign in to lately, then common ones (`signin::COMMON_SITES`). It opens
  equipe's own browser there — see "Signing in to a site yourself" below.
- **Waiting for you now** — one row per site members wait on a sign-in for
  (members joined into one row, as the cards are), with "Open browser to log
  in" (the browser panel on the member's page, in the person's hands; where
  that page is gone, the same sign-in as "Sign in to a site") and "Save this
  login" when none is kept.
- **Saved logins** — a search (site, username, note, sign-in page) and a row
  each: a letter tile (no icon is fetched from the site, so opening the page
  tells no one which sites have logins), site, username and note, a scope
  chip (all squads, this squad, another squad, several, or "asks first"),
  the "Signs in by itself" switch, an "Autopilot" button (the login's
  autopilot switches, seed and passkey open under its row; see "Login
  autopilot" below), a health badge (OK; password failed last
  time — equipe stopped signing in with it, or its last use failed and no
  newer password was saved; never used) and when it was last used. A row
  opens to change the scope (one `SetLoginScope`, so no state in between),
  edit the username, sign-in page and note, replace the password, or delete.
  Replacing and adding use equipe's own masked box; the stored password is
  never shown, copied out or read back, not even to the person. Delete asks
  once more, then hides the row for a few seconds with Undo; the daemon is
  told only when the time is up, so nothing is lost to a quit meanwhile.
- **Asked again** (7 or 30 days) — from `Command::LoginInsights`, the record of
  every squad read without writing: times asked, the share of sign-ins equipe
  did with no one's hand, how long a sign-in usually lasts before a site asks
  again, asks per day, by cause, by site, and "Top sites to fix" with the one
  step that would help: Save this login (a site with none kept, not on the
  never list, on a machine with a secret store), Turn on sign in by itself,
  or Replace password (it stopped working).
- **Never save** — the sites marked "Never for this site", each with "Offer
  again" (`ForgetNeverOffer`).
- **Login autopilot** — what equipe finished by itself this week, the
  mailbox codes are read from, codes from Messages, keep-alive looks and
  command-line sign-ins (`Command::Autopilot`, answered by `Event::Autopilot`).

New protocol: `Command::LoginInsights { days }` (read-only, shared) answered
by `Event::LoginInsights` (counts, hosts, causes, the never list, whether a
secret store is here), `Command::SetLoginScope` and `Command::ForgetNeverOffer`
(person-only). `Login::password_at` dates the last password. Logic without
drawing, tested with plain values: `crates/desktop/src/logins.rs`; the read
model: `equipe_core::logins::insights`.

## Signing in to a site yourself

`equipe browser <url>` and the app's "Sign in to a site" are one code path:
the daemon's `Runtime::start_sign_in` (`Command::BrowserSignIn { url, window }`
from a screen, the `browser_sign_in` request from the command line;
`crates/core/src/signin.rs` reads what was typed, `signin::resolve`: a host or
an http(s) address, nothing else). The daemon opens the page where the person
is: a new tab of the person's own in the shared Chrome, picked and shown in
the browser panel of an attached screen, else (or when asked for with
`window`) the separate Chrome window on the daemon's Mac. The command line
falls back to its own sign-in window only where no daemon can open one.

Over SSH the app talks to the daemon's Mac: the browser runs there and the
panel shows it here. A separate window opens on that Mac's screen, not on the
one in front of the person; the Logins page says so before and after.

The old `browser_window` request still opens the window (older command lines)
and starts the same follow-up.

### Seeing the sign-in through

The person's own tabs (lane `""`) are always the person's hands, whether the
panel or the Chrome window shows them. `signin::Tracker` (pure, fed with the
tabs as the browser host tells them) follows them:

- a tab that was on a sign-in (a password or code wall) and now is past it on
  the same site is a sign-in finished — with or without a sign-in the person
  started, in the panel or in a popped-out window;
- for the sign-in the person started, its tab (the first tab that wasn't open
  before) counts as signing in to the starting site even while it is on
  another host, so "Sign in with Google" (the provider's page, then back on
  the site past its sign-in) finishes it too;
- it is followed for 30 minutes; a closed tab ends it quietly.

When one finishes the daemon records it (`login_events`, by the person), reloads
members' pages waiting on that site (they go back to their members by
themselves), and offers to keep the login as before. `Event::BrowserSignIn`
tells every screen how it stands (opened, failed, signed in and whether a card
asks, closed). The Logins page shows the card there too, with equipe's own
masked boxes; "Signs in by itself" is on, and for an identity provider's own
login "Use this account for 'Sign in with …' on other sites" too, both the
person's to turn off (`LoginDraft::idp`).

A member's own browser the person pops out into the real Chrome window is the
person's hands too: popping out puts a hand on a lane the person holds
(`touched`, `browser-host.cjs`), so a sign-in finished there is theirs — the
panel's keys no longer have to be what says so.

## What is recorded, and the measures

Every time a member's browser goes into the person's hands — `show_my_browser`,
or a page equipe pauses on because it waits for a person (`human_check`) —
and every `sign_in` card, `sign_in` without a kept login, and finished
sign-in (by the person, or by equipe itself) is a row of `login_events` in
the squad's journal, with a cause:

`session_expired` (the site was signed in to before, or a login is kept for
it), `no_login_kept`, `captcha`, `two_factor`, `passkey`, `device_code`,
`consent`, `other`.

A paused page's cause comes from what it waits for; a `show_my_browser`'s
from the page it is on, else from the words of its `why` and its address
(`match.*_words` in the core locales, every language). From those rows:

- handoffs per site per week, by cause;
- how long a sign-in lasted before the site asked for a sign-in again (the
  median per site);
- handoffs of a site that came within a day of another member's for the same
  site — the same question asked twice;
- the share of finished sign-ins equipe did by itself with a kept login.

`equipe logins stats [--squad S] [--since 7d] [--json]` prints them for every
squad (they share one browser); the PM's weekly retro gets a line of them
when there were any.

## "Save this login?"

When a member's page was on a sign-in (a password wall, or the code step
after it) and turns into the same site past the sign-in, while the browser
was in the person's hands — handed to them by its member, or held for them
and touched by them — the person finished a sign-in. equipe then offers,
once per site, to keep the login: a card with equipe's own username and
password boxes (the password masked), a switch to let equipe sign in there by
itself, and Save / Not now / Never for this site. "Never" is kept in
`vault-never-offer.json`. Logins can also be added and changed on the Logins page. equipe never reads what the person typed into the site. A sign-in
done in the real Chrome window — popped out, or opened for the person with no
browser panel attached — is seen the same way (see "Signing in to a site
yourself"): the person's own tabs, and a member's lane they popped out, are
their hands. Logins done in another browser profile's Chrome are not offered
(another person's accounts).

## Signing in by itself

The person's to turn on or off for each login (on the offer's card, the
form, or the switch on its row on the Logins page); a login they save now
has it on unless they turn it off there (below), and logins saved before
keep what they had.
With it on, when a member's page in the shared browser waits on a sign-in,
equipe types the login in itself instead of handing the browser over, only
when all of these hold:

- the page's host is exactly the login's site or its sign-in page's host
  (`www.` aside) — never a subdomain, a parent or a lookalike;
- the page is https (plain http only on this machine, or where the login's
  own sign-in page is http);
- the address isn't an OAuth, SAML or SSO hand-off and doesn't send the
  browser to another host afterwards (`client_id`, `redirect_uri`,
  `/oauth/authorize`, a return address on another host): what comes after
  such a sign-in — granting an app access — is the person's to see (one
  narrow exception, for an identity provider's own login the person opted
  in: "Sign in with …" below);
- exactly one login matches (two accounts for a site: the person's choice);
- the browser is the person's main one, not another person's profile;
- fewer than three tries for that login in the last hour, a minute apart.

The fill goes through the browser host (`fill` op), not through the member's
own browser tools: nothing of it comes back to the member, and the host keeps
the member's tools off that tab while it types and for a few seconds after
(`gate.js`), so the password sitting in a field can't be read. The script
checks the page's host and scheme again at the moment it types.

After the submit equipe looks at where the page went:

- past the sign-in: done — recorded, the member's conversation told, other
  members' pages waiting on the same site reloaded;
- back on the form after the password: the kept password no longer works —
  equipe stops signing in with that login, tells the person once, and hands
  the browser over as before; it tries again only after the person saves a
  new password or turns it on again;
- a code, a CAPTCHA, a passkey, or anything else: handed to the person as
  before, with the cause recorded.

## "Sign in with …" on other sites

Most re-asks members hit came from one place: a site's "Sign in with Google"
(Microsoft, Apple, GitHub, Okta) sends the browser to the provider, and the
provider asks for its own password again. Those pages are hand-offs by
definition (`client_id`, `redirect_uri`), so the rule above hands them over.
The exception is narrow, and only where ALL of these hold
(`vault::idp_login_for`):

- the saved login's site is the identity provider's own sign-in host
  (`vault::idp_provider`: `accounts.google.com`, `login.microsoftonline.com`,
  `login.live.com`, `appleid.apple.com`, `github.com`, or an Okta tenant's own
  `<tenant>.okta.com`) — a login saved for `google.com` doesn't qualify;
- the person turned on that login's "Use this account for 'Sign in with …' on
  other sites" (`Autopilot::idp`, on by default when a provider's own account is saved; one switch turns it off, under the login's Autopilot
  on the Logins page, a person-only `SetLoginAutopilot`), and "Signs in by
  itself" is on and not stopped;
- the page's host is exactly that provider host, over https (no http
  exception, no sign-in page host, no subdomain);
- the address isn't a grant step (`vault::idp_grant_page`: a path naming
  consent, approval, grant or permissions);
- exactly one such login.

What equipe does there is identity only (`FillGuard::idp`, the script in
`vault::fill_script` and `autopilot::fill::idp_js`):

- picks the account in the provider's account chooser — only a tile whose own
  `data-identifier` / `data-email` / `data-test-id` is exactly the login's
  username, never by text;
- types the email into the provider's own identity field (`type=email`,
  `autocomplete=username|email`, or a field named `identifier`, `loginfmt`,
  `email`, `username`, `login`), and refuses when that field already holds
  another account;
- types the password only when the page shows the login's own account (so it
  never goes to another account's password page), and never into a page with
  a new-password field or two password fields (changing a password, account
  settings);
- types the authenticator code (autopilot `totp` only — no mail, Messages or
  passkey there) on the provider's code page, again only where the page shows
  the account;
- never ticks "remember" or "trust this device" boxes there, never presses a
  button: before anything, the script refuses (`refused: grant`) a page with a
  visible Allow / Accept / Authorize / Grant / Approve / Agree / "Continue as"
  / Confirm / Yes button (and `match.consent_words` in every language) or such
  words in its text, and anything it doesn't recognize is left alone and goes
  to the person as before (`refused: unfamiliar`, `refused: account`, or
  nothing filled).

The chooser and email steps have no password field, so they are not sign-in
walls to the page checks; equipe looks at them itself (`see_pages`): a
hand-off page on the provider's exact host, not loading, once per page and
address, within the usual tries (three an hour, a minute apart). A fill
there continues to the password page in the same run (up to three rounds).
equipe calls it signed in only once the provider sends the page back to the
site that asked; still on the provider afterwards (a consent screen, a step it
doesn't take) is left to the member or the person, not recorded as done.
These sign-ins are recorded with `step=idp` — the password and the code steps
alike — so the measures show what the exception saves.

The risk, said plainly: the provider account is the key to every site that
uses it, and a page that asks for it on another site's behalf is where an
app gets granted access. equipe never grants: consent and permission screens,
scopes, "Allow", account settings and recovery prompts stay the person's (or
the member's, who asks the person), and the switch is per login, off until
the person turns it on, and taken back with one click.

## One card per site

- `sign_in`: a member asking for a login a card already asks about waits on
  that card. One answer serves every member waiting; a yes fills once, and
  the others are told the shared browser is signed in.
- `show_my_browser` to sign in where another member's browser is already in
  the person's hands for the same site: not handed over again; the member
  waits and is told when that sign-in is done.
- A page pausing on a sign-in the person is already doing for another member
  waits on it; the screens show one row for the site ("n members waiting")
  and announce it once. When the person finishes, the other waiting pages
  are reloaded and go back to their members by themselves.
- "Always" on a card is the login's, for the whole squad — every member and
  lane of it. Settings can extend it to every squad, and take either back.
  Allowances from before (`squad/member`) are moved to their squad when
  `vault.json` is loaded.

## Sign-ins that last "for the session"

Finding (Chrome 157, `crates/core/tests/session_cookies_live.rs`): a cookie
without an expiry date — the way many sites keep a sign-in — is dropped when
Chrome starts after a clean exit. The browser host's Chrome exits cleanly
every time the daemon stops (an update, a restart, the host started again):
when the host goes, Playwright closes Chrome through its pipe, and
`Preferences` records `exit_type: Normal`. So every such sign-in was lost on
every daemon restart. Cookies with an expiry date survived. A Chrome killed
outright loses whatever it hadn't written yet (Chrome writes cookies in
batches, up to about 30 seconds late).

Chrome keeps session cookies only after a crash, or with "Continue where you
left off" (`--restore-last-session`), which also reopens every old tab and
window — members' tabs piling up as the person's, and windows coming up on
their screen at every start.

Fix: before the host starts Chrome, it marks the profile's last exit the way
Chrome marks a crash (`profile.exit_type: "Crashed"`, `keep-sessions.js`).
Chrome then restores the old session's cookies and nothing else: no tab or
window comes back, and `--hide-crash-restore-bubble` keeps the "Restore
pages?" bubble away. Only that one field of `Default/Preferences` changes; a
profile Chrome never ran, or a file that isn't Chrome's JSON, is left alone.
The profile itself is never wiped. Each named profile's Chrome gets the same.

To check on a machine with Node and Chrome:

```sh
cargo test -p equipe-core --test session_cookies_live -- --ignored --nocapture
cargo test -p equipe-core --test relogin_live -- --ignored --nocapture
```

## Saving a login is the person's leave

A login saved from the offer's card or the Logins page signs in by itself
unless the person turns the switch off on the form (the switch is shown, on).
Logins saved before keep the setting they had. An "always" on a card, and a
login's switches, last until the person takes them back; nothing expires them.

## Login autopilot

Beyond typing the password, equipe can finish the rest of a sign-in itself —
each capability off until the person turns it on for that login (the Logins
page › a login's "Autopilot"), through person-only commands
(`SetLoginAutopilot`, `SetLoginTotp`, `ScanLoginTotp`, `RegisterLoginPasskey`,
`ForgetLoginPasskey`, `SaveMailbox`, `SetSmsCodes`, `SetKeepAlive`,
`SaveCliLogin`, `DeleteCliLogin`; `permissions.rs`). Every one acts only on the
login's exact host over https (the rules of "Signing in by itself" above: no
subdomain, no lookalike, no OAuth/SAML hand-off, exactly one login), through
the browser host's `fill` with the member's tools held off the tab, and the
script checks the host again as it types. Seeds, codes, passkeys, the mailbox's
app password and command-line tokens live in the secret store only, under
names of their own (`vault::account`); `vault.json`, `autopilot.json` and
`cli-logins.json` say only whether one is kept.

| Step | What equipe does | What the person does once | Limits |
|---|---|---|---|
| `totp` | On the site's code page, types the code made from the login's authenticator seed (RFC 6238; SHA-1/256/512, 6–8 digits). | Pastes the `otpauth://` address or the secret, or presses "Read QR" with the site's setup page open in their own tab of equipe's browser (the picture is read on this machine, `rqrr`, and dropped). | Time-based codes only; push approvals (Microsoft Authenticator, Google prompts, Apple trusted devices) stay the person's. |
| `email` | Watches the connected mailbox (IMAP over TLS, read-only `EXAMINE`/`BODY.PEEK`) for up to 3 minutes for mail from the site's own domain (or the domain the person named), takes the code-shaped token, types it. | Connects a mailbox with an app password. | Gmail API OAuth isn't built (IMAP with an app password covers Gmail); magic links aren't followed. |
| `sms` | Reads the newest text from Messages (`~/Library/Messages/chat.db`, read-only) that is bound to the site's host (WebOTP `@host #code`) or names it, within 3 minutes. | Turns it on, and grants equipe Full Disk Access (System Settings › Privacy & Security › Full Disk Access). Without the grant it refuses and says so. | macOS with Text Message Forwarding only; Windows and Linux refuse. |
| `passkey` | Gives the page equipe's own passkey for the moment (Chrome's virtual authenticator, CDP `WebAuthn`) and presses its passkey button; the credential's counter is kept up to date. | Presses "Add equipe's passkey", then adds a passkey on the site's security page in equipe's browser within 10 minutes. It is a separate device on the site, revocable there. | The person's own Touch ID passkeys are never used. A site that offers no passkey button, or only a conditional prompt, stays the person's. |
| `remember` | Ticks "Remember me", "Keep me signed in", "Trust this device" boxes on the forms equipe fills (words of every locale, `match.remember_words`). | Nothing: on by default for a login equipe signs in with; one switch turns it off. | Only boxes on the site's own form. |
| `keepalive` | Every few hours (the Logins page's autopilot card: off, 3, 6, 12 or 24; jittered ±20 %, one site at a time, at most one look every 3 minutes), opens the site's check page in a tab of equipe's own (`visit`/`look`, a lane no member has) and signs back in with the steps above when it finds the site signed out; records the look on the login. | Turns it on for the login (and lets it sign in by itself). | A CAPTCHA or a code equipe can't get is left for when a member needs the site. |
| `relogin` | Signing in by itself (above), now also when a member's page shows its sign-in ran out without a form: the site answered one of the page's own requests 401/419/440, or the page says the session expired (`match.session_expired_words`). equipe takes that page to the login's sign-in page, signs in, and brings it back where it was. | Saves the login. | Only for a page on the login's exact host. |
| `device_code` | A member's reply asking the person to run `gh auth login`, `aws sso login`/`aws login` or `vercel login` (a ```bash block) is run by equipe itself — so the code is the one its own CLI printed, never one a member or a page passed on — with `BROWSER=echo`; the printed page is opened in a tab of equipe's own in the shared, signed-in browser, the code typed where the page asks, and the approve button pressed only on the hosts the person named and only where the page shows that very code. The member is told when the CLI is signed in; the desktop doesn't open a terminal for it. | Turns on "Approve device sign-ins" on the github.com or vercel.com login, or adds a "Device sign-in" for `aws` with its hosts (`*.awsapps.com, device.sso.<region>.amazonaws.com`). | Anything off those hosts (an identity provider elsewhere, a CAPTCHA) goes back to the person with the command to run. |

Command-line credentials (the Logins page › Login autopilot › Command-line
sign-ins), kept in the secret store:

- **GitHub token**: given to `gh` over its standard input (`gh auth login
  --with-token`), then `gh auth setup-git`; never on a command line.
- **GitHub App** (`<app id>/<installation id>` and its private key): equipe
  signs the app's JWT (RS256, `ring`), mints an installation token and gives it
  to `gh`, again ten minutes before each runs out.
- **AWS key** (profile name and access key id; the secret key in the store):
  the profile's `credential_process` in `~/.aws/config` (or `AWS_CONFIG_FILE`)
  runs `equipe credential aws <profile>`, which answers the AWS CLI and refuses
  a member's command calling it directly (its parent must be the AWS CLI).
- **Environment value** (`NPM_TOKEN`, `VERCEL_TOKEN`, …): in the environment of
  members' runs (every squad, or one), never written to a file or a prompt.
- `ask_secret` values go the same way: kept in the secret store as that squad's
  environment values, `$KEY` in every member turn that starts after; a file is
  written too only when the member names one (`save_to`) for a program equipe
  doesn't run.

A member can still print an environment variable its command was given; the
values reach only commands run for members of that squad, and the guard's deny
rules keep members away from the stores themselves.

### CAPTCHAs

Never solved or got around: no solver service, no fingerprint changes. They
come less often because the shared Chrome profile is the same real Chrome
across restarts (its session cookies kept, `keep-sessions.js`; nothing clears
it on an update), keep-alive keeps sessions from running out, and fewer
sign-ins mean fewer checks. When one comes, it goes to the person as before:
the desktop's notification opens the browser panel on that page, Telegram
(when set up) says which member waits, and the member's tools carry on by
themselves once it is solved. It is recorded with `cause=captcha`.

### Measures

`login_events` rows of sign-ins equipe finished carry the step that did it
(`step`: `relogin`, `totp`, `email`, `sms`, `passkey`, `device_code`,
`keepalive`, `idp`; a column added to journals from before). `equipe logins stats`,
the PM's retro line and the Logins page's autopilot card show them with a rough count of the
person's minutes saved (2 for a typed password, a keep-alive or a "Sign in
with …" step, 3 for a code or
passkey from equipe's own store, 4 for a code from mail or Messages, 5 for a
device sign-in). The goal: handoffs per site per week near zero, CAPTCHAs aside.

To check on a machine with Node and Chrome (no real accounts):

```sh
cargo test -p equipe-core --lib autopilot
```

## Keys members obtain

A member that gets a key — a PostHog project key from a dashboard, a token a
CLI printed, a database password — keeps it with `secret_save`
(`crates/core/src/squad_secrets.rs`), instead of it living only in a host's
environment variables, so a later ticket can use it.

- **Where.** The value is in the same Keychain item as the logins, as
  `equipe.secrets:<squad>/<name>` (elsewhere: the secret store, never a file).
  The facts — name, kind (`api_key`, `token`, `password`,
  `connection_string`, `other`), service, the projects that may use it, who
  saved it, on which ticket and when, its last use — are the squad's journal's.
- **Member tools** (the board's MCP server): `secret_save {name, service,
  kind, value, projects?, note?}` (the answer never repeats the value),
  `secret_list` (facts only), `secret_use {name, as_env}` and
  `secret_write {name, path, format?, key?}`. No engine shell can have a
  variable added mid-turn, so `secret_use` makes the key an environment
  variable of the member's **next** turns (and is checked again each time),
  and `secret_write` writes it now into a file of the member's worktree:
  `env` (a `KEY="value"` line, other lines kept), `json` (a field) or `raw`.
  The file must be inside the worktree, not a link, not in `.git`,
  **git-ignored and untracked** — otherwise refused, saying how to fix it.
  It is made readable by the person only. To hand a key to a service the
  member uses the CLI it already has against that file (for example
  `vercel env add POSTHOG_KEY < …`, `gh secret set`); equipe itself sends
  nothing off the machine.
- **Who and where.** Only the saving squad's members; on a project only where
  the person allowed it. The first use on a new project is a decision card
  (this project / any project / no), asked once; "any project" is also the
  person's switch on the Logins page. Every use is recorded — who, ticket,
  project, when — in the journal and on the ticket.
- **Masking.** Every stored value is cut out verbatim (and its JSON-escaped
  form, and each line of a multi-line one) from what leaves a member: events
  (the conversation, tool output, notices, ticket text), the arguments of its
  board tools (tickets, comments, hand-ins, memory), and the daemon's log. A
  streaming answer is held back by the longest value's length so a value split
  across fragments is still cut whole. Values shorter than six characters
  aren't masked. What *looks* like a key but wasn't saved (a well-known
  prefix, the password of a connection string, a `…KEY=`/`…TOKEN:` value,
  `Bearer …`) is cut out of command output and the member's board-tool
  arguments, and the member is told, with its next tool's answer, to
  `secret_save` it.
- **The person.** Logins › Keys & secrets lists them, with show-once (after a
  confirmation; hidden after 30 seconds, never to a phone), copy, the projects
  it may be used on, and delete. `equipe secrets list | show <name> | delete
  <name>` does the same from a terminal; `show` and `delete` ask first and
  refuse a member's command or anything that isn't a terminal.

What it does not do: the engine's own session files hold what a member typed
before it saved a key (equipe can't edit those); a key already committed in
code is not found; and a value a member splits or re-encodes (base64, spaced
out) isn't recognised.

## Left to the person

CAPTCHAs and "verify you are human" walls; push approvals and codes only a
phone shows; the person's own passkeys and security keys; sign-ins handed off
to another host (OAuth, SAML, "Sign in with Google" on another site's behalf),
except the provider's own identity steps for a login the person opted in
("Sign in with …" above); consent screens, account creation and billing. equipe records them so the measures show what is left,
and never gets around them.
