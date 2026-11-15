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
| Verification codes from email, filled on the person's click | `crates/core/src/verify/`, `crates/core/src/runtime/verify.rs`, `views/verify.rs` |

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
pages?" bubble away. Only that one field of the pinned profile's `Preferences`
(see "One Chrome profile" below; `Default` unless pinned otherwise) changes; a
profile Chrome never ran, or a file that isn't Chrome's JSON, is left alone.
The profile itself is never wiped. Each named profile's Chrome gets the same.

To check on a machine with Node and Chrome:

```sh
cargo test -p equipe-core --test session_cookies_live -- --ignored --nocapture
cargo test -p equipe-core --test relogin_live -- --ignored --nocapture
```

## One Chrome profile, pinned

Finding (an affected Mac, November 2026): equipe's user-data folder
(`browser/chrome`) held three Chrome profiles — `Default` and two named after a
Google Workspace domain, made by Chrome when the person signed in to a work
account (Chrome offers a separate profile for one, and an organization can
require it). Neither the host's Chrome nor the person's window named a
profile, so each start opened whichever profile Chrome had used last: members
worked in all three at different times, each with its own cookies, and each a
new device to Google. A sign-in made in one was gone at the next restart, every
other site's sign-in looked gone after a switch, and Google asked for its
second step again. Session sign-ins were lost on top of that: only
`Default/Preferences` was marked to keep them.

Fix (`browser::chrome_profile`): before any Chrome starts on the folder — the
host's, a resumed one, the person's window — the daemon decides its profile
and pins it in `equipe-profile` in the folder; both are started with
`--profile-directory=<pinned>`, and `keep-sessions` marks that profile. The
first pin is the profile Chrome last used (the one members last worked in), else
`Default`. Afterwards the pin holds, except when Chrome's own `Local State`
says another existing profile of the folder was used last: equipe's Chrome
only ever opens the pinned one, so that is the person's choice, or the profile
their sign-in was moved to — it is followed, and the daemon log says so.

## Seeing a hand-off through

A sign-in the person makes in their own Chrome window, or in a sign-in view,
happens where no member's tab can watch it. When such a browser is handed
back (and its finish wasn't seen while in the person's hands), the daemon
watches the member's page as it comes back for two minutes
(`logins::back_verdict`): on the site it was for — or, handed over on an
identity provider's page, on the site the provider sent it to — past any
sign-in, it is a sign-in finished: recorded as the person's, its cookies kept,
other members' pages waiting on it reloaded, and "Save this login?" offered
once. Before, these hand-offs never recorded a finish, so every later ask
counted as a first sign-in (`no_login_kept`) and no save was ever offered.

Before a hand-off, a member whose own page on that site shows no sign-in is
told to look first (once per site in ten minutes; asked again, the person is
asked). One person's step on a site — a sign-in, the code or passkey after it —
serves every member asking for it: a second member waits on the first
hand-off. A hand-off for a site's own second step tells the person it is the
site's, and to tick "Don't ask again on this device" where offered.

Google's sign-in is never carried from a sign-in view into the members' Chrome
(`handoff_view::binds_to_browser`): Google ends a session it sees come back
from another browser. It is made in the members' own profile (the person's
window on it), and a kept Google sign-in is never put back by copy. A kept
sign-in put back before asking only adds the cookies the profile lacks, never
over a fresher one.

## One store of sign-ins for every squad

The sign-ins equipe keeps (`signin_store`) are one sealed store for every squad
on the machine (`sign-ins.sealed` in equipe's home, key `sign-ins` in the secret
store): one sign-in counts everywhere in equipe. A squad working for another
company can keep its own: `equipe logins share <squad> off` (a
`sign-ins.own` mark in its folder; `on` shares again). Squads' own files from
before are moved into the shared store the first time it is read — opened with
their keys and taken in, newer sign-in per site winning, nothing printed —
then set aside as `sign-ins.sealed.moved`. A squad that keeps its own still
shares the Mac's one Chrome; for accounts that must never meet, use another
browser profile (another person's accounts, above).

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
| `email` | Watches the connected mailbox (IMAP over TLS, read-only `EXAMINE`/`BODY.PEEK`) for up to 3 minutes for mail from the site's own domain (or the domain the person named) — searched for on the server, so no other mail is fetched — takes the code-shaped token, types it. | Connects a mailbox with an app password. | IMAP only (Gmail's API is used by verification codes, below); magic links aren't followed. |
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
- **Every secret request carries a guide**, `ask_secret` and `request_secret`
  alike: `steps` (1 to 8 short imperative lines, in the person's language) and
  the page the secret comes from (`url` / `create_url`, https, a real host: no
  credentials, port, IP, localhost, `javascript:` or `file:`). A call without
  steps, or a website's secret without its page, is refused with what to add,
  so a card never appears bare; only a secret with no web page at all (a local
  NAS password, `no_page: true`) may leave the page out, and its steps say where
  to find it. For a catalog service (Connections) a missing page is filled with
  the catalog's key page, and its sign-in page rides along as a second link. The
  card shows the purpose, the numbered steps (an address inside a step gets its
  own button), a primary **Open <host>** button, **Copy link**, then the masked
  field and Save. The button runs in the desktop app, so it opens on the screen
  the person is looking at even when the daemon is on another Mac. A card the
  person starts from Connections has no member's steps: it shows the catalog's
  own (`connections.svc.<id>.steps` in the locale files).

**`cli_sign_in`** (a member's tool) is the hand-off for a CLI sign-in the
person has to finish and no machine credential covers. equipe runs the tool's
own cross-device mode on the daemon's Mac, in a pseudo-terminal (`gh` over pipes:
in a terminal it first asks about the git credential helper), reads the page and
the code from its output, and shows one card: "Open sign-in page" (opened on the
Mac the person is looking at), the device code to confirm, and a box when the
flow has the person paste a code back (`aws login --remote`, `gcloud auth login
--no-launch-browser`). The pasted code is written to the process's standard
input and dropped: not in an event, a journal or a log, and the terminal's echo is
off. Expired or failed: "Get a new link" runs it again. When it ends signed in the
member is told as whom, from the tool's own whoami (`aws sts get-caller-identity`,
`gh api user`, `gcloud config get-value account`, `az account show`) — no secrets.
Where "Approve device sign-ins" is on for `gh` or `aws sso`, the same approval as
`device_code` above approves the page by itself. The modes: `aws login --remote`,
`aws sso login --no-browser --use-device-code`, `gh auth login --web`, `gcloud auth
login --no-launch-browser`, `az login --use-device-code`; flags that would hand
equipe a token, password or key are refused.

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

## Verification codes from email

Some steps exist to show a person is present — GitHub's "Confirm access"
(sudo), Google's "Verify it's you", any "enter the code we emailed you".
When a member's page reaches one, the code is never the member's: it never
reads, receives or types it, and never asks for it in chat (its charter,
`browser` skill, `block_on_person` description and the browser gate all say
so).

| Piece | Code |
|---|---|
| Which page is an emailed-code step, which mail is its, the click | `crates/core/src/verify/mod.rs` |
| Gmail: read-only OAuth (PKCE, loopback), the search, the text | `crates/core/src/verify/gmail.rs` |
| IMAP: the sender searched for on the server | `Imap::matching` in `crates/core/src/autopilot/mail.rs` |
| Cards, the click, connecting Gmail, the member told | `crates/core/src/runtime/verify.rs` |
| The card (`Verification`, never a code), commands, events | `crates/protocol/src/verify.rs` |
| The card over the composer, Logins › Connections' mailbox row | `crates/desktop/src/views/verify.rs` |

- **One card.** A member's page with a code field that says the code went
  by email (`verify::email_step`), or a member's `block_on_person` with
  `verification: {site, channel: "email"}`, raises one card per browser and
  site: "GitHub sent a verification code to <mailbox>", the steps, **Fill
  code from email**, **Open email** (the mailbox's own web page searched for
  the site's mail of the last hour, in the person's own browser) and **I'll
  type it** (the page in their hands, its code field focused). Without a
  mailbox the card offers to connect one.
- **Only on the click.** Raising a card never fetches anything
  (`verify::Board`); the person-only `FillVerificationCode` is the one way
  to a fetch. The daemon then looks in the mailbox every 5 seconds for up
  to a minute, takes the code out of the matching mail, and types it into
  that page through the browser host's `fill` (the page's own host, https,
  the member's tools held off the tab), pressing the page's own button as
  part of the same click. No matching mail in time: the card says so, with
  how to get a new code.
- **Only the matching mail** (`verify::pick`): from the site's own sender
  domain (or the domain the person named on its login), received after the
  page asked for it (90 seconds' allowance) and within the last 10 minutes,
  saying it is a verification code; the newest wins; only a strict code
  (6–8 digits, or a short run of capitals and digits next to a code word)
  leaves it. Gmail is searched with `from:(<domain>) after:<time>` and only
  the newest three matches' text is read; IMAP is searched `FROM "<domain>"`
  on the server, and only matches' headers, then texts, are fetched.
- **Nowhere but the page.** The card and every event carry no code;
  `verify::Outcome` has no place for one and `verify::Pick`'s `Debug` hides
  it; the log says only what happened. For 10 minutes after it is fetched
  the code is cut out of anything a member writes (`squad_secrets::watch`).
  The member hears "the person confirmed access on <site>" — after the page
  is back in its hands — and, if the site didn't take the code, the card
  says so and the person decides.

### The mailbox

Gmail, over its API with the one scope `gmail.readonly` — the narrowest that
can read a message's text (`gmail.metadata` can't, and can't search). The
grant can't send, delete or change anything. It is chosen over reading
webmail in a signed-in browser profile: that would put the mailbox in the
squad's shared browser that members drive, and scrape a page that changes.
Any other mailbox works over IMAP (the one Login autopilot uses, with an app
password; read-only `EXAMINE`/`BODY.PEEK`).

Connecting is Google's flow for desktop apps: **Connect Gmail** in Logins ›
Connections opens Google's consent page in the person's own browser; it
comes back to a one-off listener on `127.0.0.1` with a code, which the
daemon trades, with its PKCE verifier, for a refresh token. A grant for more
than read-only mail is refused. The token (and the client it belongs to) goes
to the secret store under `mailbox.gmail`, never to a member, a screen, an
event or the log; `autopilot.json` keeps only the address. **Disconnect**
drops it (revoke it in the Google Account too).

The OAuth client: a build carries one when `EQUIPE_GOOGLE_CLIENT_ID` and
`EQUIPE_GOOGLE_CLIENT_SECRET` are set at build time or in the daemon's
environment. Without one, the Connections row asks for the person's own: a
Google Cloud project with the Gmail API on, its consent screen published
(a testing one's grants run out after 7 days), and an OAuth client of type
"Desktop app" — its ID and secret pasted once.

Limits: the listener is on the daemon's machine, so connect Gmail from the
Mac equipe runs on; a code that arrives only by text message or in an
authenticator app stays the person's to type; magic links aren't followed.

```sh
cargo test -p equipe-core --lib verify
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

## Outside MCP servers

A squad can connect an outside MCP server for its members — a design
library's official server from the Connections catalog (Mobbin: "search real
product screens and flows for design references", Pro plan or higher;
21st.dev: "proven React, Tailwind and shadcn/ui components to adapt"), or any
server by its https address (`crates/core/src/squad_mcp.rs`). The person
connects it to equipe **once**, and it works for every engine a member runs
on — Claude Code, Codex, and the model providers that run on Codex's agent.

- **Kept.** In the squad's journal (`mcp_servers`): name, address (https only,
  checked as strictly as a key card's page), transport (streamable HTTP, or
  the older HTTP+SSE), the roles that get it (the catalog's by default, else
  every member), an on/off switch, and who holds its credential (`auth`). The
  person adds one from Logins › Connections ("Connect" on a catalog row, or
  "Add an MCP server" at the end), from a terminal (`equipe mcp add mobbin`,
  `equipe mcp add <name> <url> --roles designer,frontend`, `list`, `remove`,
  `login`), or through the PM's `change_setting` (`mcp_server`), which adds or
  turns one on only after the person's tap on an approval card. Members never
  add one. The app and the CLI send the same commands to the daemon and show
  the same states; adding one already there changes nothing.
- **Signed in by equipe.** equipe is the OAuth client (`crates/core/src/mcp_oauth.rs`):
  the server's 401 names its protected-resource metadata (RFC 9728), which
  names its authorization server and its metadata (RFC 8414, or OpenID
  discovery); equipe registers itself for the sign-in (RFC 7591 dynamic client
  registration), opens the page with PKCE S256 and the server as `resource`
  (RFC 8707, plus `offline_access` where the server offers it, for a refresh
  token), and the page's redirect comes back to a one-off listener on the
  daemon's loopback address. The access token, refresh token, expiry and
  client registration are one JSON value in the squad's secret store (the
  Keychain on a Mac) under `mcp-oauth-<server>` — never in a journal, an
  event, a log, a screen or a member's reach — and on the masking list. A
  token about to run out, or one the server refuses, is renewed with the
  refresh token, one renewal at a time per server; a renewal the server
  refuses means the person signs in again, on one card.
- **The card.** Connect, Sign in (and "Sign in again" in a connected row's ⋯
  menu) open the sign-in card as a sheet over the page: "Open sign-in page",
  its progress, and a box for the address the page ended on. A member who
  gets a server that needs a sign-in puts the same card up once (in the
  conversation); "Not now" isn't asked again until the person asks. From the
  app on another Mac over SSH, "Open sign-in page" forwards the redirect's
  loopback port to the daemon's Mac for the sign-in's length (over the
  window's shared SSH connection, else an `ssh -N -L` of its own that never
  asks anything — `crates/desktop/src/tunnel.rs`), so the page finishes by
  itself; when the port is taken here or no connection takes the forward,
  the page opens in the daemon's own browser in the browser panel; the paste
  box is the last resort (`equipe_core::cli_signin::route`). Gmail's mailbox
  connect gets the same tunnel. `equipe mcp login <name>` starts the same
  card on the daemon and opens its page in the terminal's browser.
- **Given.** From their next turn, each member of those roles gets the
  server through equipe's proxy: a stdio entry (`equipe mcp proxy --squad …
  --server … --member … --home …`) in Claude Code's `--mcp-config`
  (`--strict-mcp-config` kept) and in Codex's thread `mcp_servers` alike, and
  one line in its charter naming it. The proxy holds nothing: it hands each
  JSON-RPC message to the daemon over its socket (`mcp_proxy`), where
  `crates/core/src/mcp_gateway.rs` forwards it — streamable HTTP (JSON or
  event-stream answers, the session id kept) or HTTP+SSE — with the
  credential added, and hands back what the server says. The daemon serves a
  proxy only for a member whose role gets the server. No token, key, header
  or address is in a member's config, environment, files or process. Tool
  calls are logged by name only. Its tools run without asking in Auto and
  Full, and ask in Ask. A server that needs a sign-in (or its key) isn't
  given until it has one.
- **Status.** Connected, needs sign-in, needs its key, or not reachable (with
  why), from equipe's own connection — an `initialize` with the credential,
  and what the proxies' calls show — the same for every engine. It is looked
  at when connected, when a member who gets it starts a turn and it isn't
  known (or was seen long ago), after a sign-in, and when a kept key changes;
  never on a timer. The daemon logs a server's state only when it changes.
- **Removed.** Members stop getting it, its cards close, and equipe's tokens
  for it leave the secret store (a kept key stays, on the Keys list).

### Who holds the credential

Each server's `auth` (`McpAuth`, `crates/protocol/src/mcp_servers.rs`) says
who holds its credential; `squad_mcp::credential` turns it into what a
member's engine is given at launch.

- **`oauth`** (the default, and what a record from before reads as; Mobbin):
  equipe's own connection, as above. A server that asks for no sign-in is
  simply connected.
- **`key`** (21st.dev, `https://21st.dev/api/mcp`, header `x-api-key`): the
  person pastes the API key on the usual key card (steps, and one click to
  `https://21st.dev/mcp`, where the key is created) into the squad's secret
  store. It needs its key — never a sign-in — and stands connected once the
  key is kept and the server takes it; a key it refuses shows as needing its
  key again. The proxy adds the header in the daemon, as for a token.
- **`engine`** (a fallback, `equipe mcp add … --engine-signin`): for a server
  that won't let other apps register. Each engine's own CLI signs in and
  keeps the token — `claude mcp login`, with the server added in local scope
  to the squad's own folder (`squads/<squad>/mcp-signin`, under a lock, an
  entry already there reused, so adding never fails with "already exists"),
  or `codex mcp login` with a config override — and members get the
  server's address. Its status comes from those tools (`claude mcp get`,
  `codex mcp list --json`); one card per engine. Removed: the engines' own
  entry and sign-in are cleared unless another squad uses the same server
  that way.

## Left to the person

CAPTCHAs and "verify you are human" walls; push approvals and codes only a
phone shows; the person's own passkeys and security keys; sign-ins handed off
to another host (OAuth, SAML, "Sign in with Google" on another site's behalf),
except the provider's own identity steps for a login the person opted in
("Sign in with …" above); consent screens, account creation and billing. equipe records them so the measures show what is left,
and never gets around them.
