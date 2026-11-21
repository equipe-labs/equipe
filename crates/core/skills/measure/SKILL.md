---
name: measure
description: Use when a ticket adds or changes anything people see or do in a product, and when you plan one — every new user-facing feature and key action ships with its analytics events, named the same way.
---

# Measure what you build

A feature nobody measures is a guess. Every new user-facing feature and key action ships with its analytics events **in
the same ticket**, not in a later one. On equipe cloud every deployed page already counts visitors, sources and devices;
what only you know is which actions matter.

## How to track

- In code: `equipe.track("signup_completed", { plan: "team" })`. `equipe.convert(name)` is the same call.
- With no code: put `data-eq-track="invite_sent"` on the button or link; each click is counted.
- Locally `window.equipe` may not exist (the script is added when the page is deployed): write `window.equipe?.track?.(...)`.

## Names

- Lowercase snake_case, `<noun>_<past-verb>`: `signup_completed`, `booking_requested`, `checkout_started`, `invite_sent`.
- The platform's reports recognize these as outcomes: `signup`, `sign_up`, `booking`, `book`, `purchase`, `checkout`,
  `subscribe`, `lead`, `contact`, `order`, `trial_start`, `waitlist`. When the action is the one the person cares about (the
  sign-up, the booking, the purchase), fire that exact name as well, so it shows in the weekly report and the Growth page.
- One event per meaningful step of a flow (`checkout_started`, then `purchase`), so a funnel can be read from them.
- Never personal data or free text in a name or in props: no emails, names, search text or messages. Short fixed values
  only (a plan name, a step number, a variant key).

## Done means

The ticket's acceptance criteria include: "the events fire and show in `query_analytics`" (open the deployed page, do the
action, then check). The plan says what the events are and which number should move; the PM names that success metric in
plain words ("more people finish signing up"), never in event jargon, when it tells the person.
