# Logging in once

How equipe keeps members signed in to the sites they work on, so the person
signs in once and isn't asked again for the same thing. The pieces:

| Piece | Code |
|---|---|
| Logins (site, username, password in the system's secret store) | `crates/core/src/vault.rs`, `crates/protocol/src/vault.rs` |
| Why the person was asked to sign in, and the measures | `crates/core/src/logins.rs`, `equipe logins stats` |
| Offers to keep a login, signing in by itself, one card per site | `crates/core/src/runtime/logins.rs` |
| The browser host's fill and session cookies | `crates/core/assets/browser-host.cjs`, `keep-sessions.js`, `gate.js` |
| Cards, the Logins page, Settings › Accounts | `crates/desktop/src/views/login_offers.rs`, `views/logins.rs` (logic: `crates/desktop/src/logins.rs`), `views/accounts.rs` |

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

- **Waiting for you now** — one row per site members wait on a sign-in for
  (members joined into one row, as the cards are), with "Open browser to log
  in" (the browser panel on that page) and "Save this login" when none is kept.
- **Saved logins** — a search (site, username, note, sign-in page) and a row
  each: a letter tile (no icon is fetched from the site, so opening the page
  tells no one which sites have logins), site, username and note, a scope
  chip (all squads, this squad, another squad, several, or "asks first"),
  the "Signs in by itself" switch, a health badge (OK; password failed last
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

New protocol: `Command::LoginInsights { days }` (read-only, shared) answered
by `Event::LoginInsights` (counts, hosts, causes, the never list, whether a
secret store is here), `Command::SetLoginScope` and `Command::ForgetNeverOffer`
(person-only). `Login::password_at` dates the last password. Logic without
drawing, tested with plain values: `crates/desktop/src/logins.rs`; the read
model: `equipe_core::logins::insights`.

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
done in the real Chrome window (popped out, or on the Mac's screen with no
browser panel attached) isn't seen as the person's hand, so no offer comes
then; the Logins page still takes the login.

## Signing in by itself

Off for every login until the person turns it on for that login (on the
offer's card, the form, or the switch on its row on the Logins page).
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

## Left to the person

CAPTCHAs and "verify you are human" walls, passkeys and security keys,
codes from a phone or authenticator, device-code flows (`aws sso login`,
`gh auth login`), consent screens, account creation and billing. equipe
records them so the measures show what is left, and never gets around them.
