//! `FAKE_READING=1`: the PM's last reply is a long one with every kind
//! of text a reply has — headings, a numbered list with bullets under it,
//! bold, ticket numbers, a link, inline code and a code block — to compare
//! how the conversation reads before and after a change to its colours or type.

pub fn on() -> bool {
    std::env::var("FAKE_READING").is_ok_and(|v| v == "1")
}

pub const ASKED: &str = "How far along is the sign-in fix? Sum it up for me";

pub const REPLY: &str = "For the sign-in problem we **found the cause and are fixing it**. Here's what's done so far and what's left.

## The cause

The session cookie was set to `SameSite=Strict`, so people were signed out when they came back from the payment page. It's the same cause as the problem first reported in [T-42](ticket:42).

## Where it stands

1. **Backend**: changed the cookie to `Lax`.
   - The tests in `cargo test -p auth` all pass
   - Checked on staging
2. **Frontend**: fixed the way back after paying. Done together with [T-117](ticket:117).
3. **QA**: checking again in Safari and Chrome.

## What's left

- Check whether the mobile app's web view has the same problem
- Go through the [security checklist](https://owasp.org/www-project-top-ten/) once more before deploying

```sh
cargo test -p auth -- session
```

Once QA has checked, we can **deploy this evening**. Shall I go ahead?";
