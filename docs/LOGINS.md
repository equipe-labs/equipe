# Logging in once

How equipe keeps members signed in to the sites they work on, so the person
signs in once and isn't asked again for the same thing. The pieces:

| Piece | Code |
|---|---|
| Logins (site, username, password in the system's secret store) | `crates/core/src/vault.rs`, `crates/protocol/src/vault.rs` |
| Why the person was asked to sign in, and the measures | `crates/core/src/logins.rs`, `equipe logins stats` |
| Offers to keep a login, signing in by itself, one card per site | `crates/core/src/runtime/logins.rs` |
| The browser host's fill and session cookies | `crates/core/assets/browser-host.cjs`, `keep-sessions.js`, `gate.js` |
| Cards and Settings › Accounts | `crates/desktop/src/views/login_offers.rs`, `views/accounts.rs` |
| Login autopilot: codes, passkey, keep-alive, command-line sign-ins | `crates/core/src/autopilot/`, `crates/core/src/runtime/autopilot.rs`, `views/autopilot.rs` |

## Invariants

- A password never reaches a member, a screen other than equipe's own entry
  form, an event, telemetry or a log. The fill script answers what it did,
  never the values; `LoginDraft`'s `Debug` hides the password; the sign-in
  record keeps hosts, member names and causes only.
- Every allowance is the person's: a card's answer or a Settings control,
  sent as a person-only command (`SaveLogin`, `AllowLogin`, `SetLoginAuto`,
  `ForgetLoginAllowance`, `DismissLoginOffer`; `permissions.rs`). Nothing a
  member says or a page shows turns one on.
- Passwords live in the system's secret store only — the Keychain on macOS,
  Credential Manager on Windows, the Secret Service (libsecret's
  `secret-tool`) on Linux. Where there is none, saving a login fails and
  says why; nothing falls back to a file.

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
`vault-never-offer.json`. Logins can also be added and changed in Settings ›
Accounts. equipe never reads what the person typed into the site. A sign-in
done in the real Chrome window (popped out, or on the Mac's screen with no
browser panel attached) isn't seen as the person's hand, so no offer comes
then; Settings › Accounts still takes the login.

## Signing in by itself

The person's to turn on or off for each login (on the offer's card, the
form, or the switch on its row in Settings › Accounts); a login they save
now has it on unless they turn it off there (below), and logins saved before
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
  such a sign-in — granting an app access — is the person's to see;
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

A login saved from the offer's card or Settings › Accounts signs in by itself
unless the person turns the switch off on the form (the switch is shown, on).
Logins saved before keep the setting they had. An "always" on a card, and a
login's switches, last until the person takes them back; nothing expires them.

## Login autopilot

Beyond typing the password, equipe can finish the rest of a sign-in itself —
each capability off until the person turns it on for that login (Settings ›
Accounts › a login's "Autopilot"), through person-only commands
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
| `keepalive` | Every few hours (Settings: off, 3, 6, 12 or 24; jittered ±20 %, one site at a time, at most one look every 3 minutes), opens the site's check page in a tab of equipe's own (`visit`/`look`, a lane no member has) and signs back in with the steps above when it finds the site signed out; records the look on the login. | Turns it on for the login (and lets it sign in by itself). | A CAPTCHA or a code equipe can't get is left for when a member needs the site. |
| `relogin` | Signing in by itself (above), now also when a member's page shows its sign-in ran out without a form: the site answered one of the page's own requests 401/419/440, or the page says the session expired (`match.session_expired_words`). equipe takes that page to the login's sign-in page, signs in, and brings it back where it was. | Saves the login. | Only for a page on the login's exact host. |
| `device_code` | A member's reply asking the person to run `gh auth login`, `aws sso login`/`aws login` or `vercel login` (a ```bash block) is run by equipe itself — so the code is the one its own CLI printed, never one a member or a page passed on — with `BROWSER=echo`; the printed page is opened in a tab of equipe's own in the shared, signed-in browser, the code typed where the page asks, and the approve button pressed only on the hosts the person named and only where the page shows that very code. The member is told when the CLI is signed in; the desktop doesn't open a terminal for it. | Turns on "Approve device sign-ins" on the github.com or vercel.com login, or adds a "Device sign-in" for `aws` with its hosts (`*.awsapps.com, device.sso.<region>.amazonaws.com`). | Anything off those hosts (an identity provider elsewhere, a CAPTCHA) goes back to the person with the command to run. |

Command-line credentials (Settings › Accounts › Login autopilot › Command-line
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
`keepalive`; a column added to journals from before). `equipe logins stats`,
the PM's retro line and the Settings card show them with a rough count of the
person's minutes saved (2 for a typed password or a keep-alive, 3 for a code or
passkey from equipe's own store, 4 for a code from mail or Messages, 5 for a
device sign-in). The goal: handoffs per site per week near zero, CAPTCHAs aside.

To check on a machine with Node and Chrome (no real accounts):

```sh
cargo test -p equipe-core --lib autopilot
```

## Left to the person

CAPTCHAs and "verify you are human" walls; push approvals and codes only a
phone shows; the person's own passkeys and security keys; sign-ins handed off
to another host (OAuth, SAML, "Sign in with Google" on another site's behalf —
including Google's password check for another site); consent screens, account
creation and billing. equipe records them so the measures show what is left,
and never gets around them.
