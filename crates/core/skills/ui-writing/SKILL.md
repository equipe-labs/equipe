---
name: ui-writing
description: Use for any words on a screen — labels, buttons, errors, empty states, confirmations and headings.
---

# Writing for interfaces

Interface copy is part of the design, not filler. Every word is the product's own, in its voice (`brand.md`), and
every fact on the screen is real: a name, number, date or claim nobody gave you is an open question, shown meanwhile
in neutral words — never invented.

## Principles

- **Plain and specific**: the words people use for the thing, not the system's ("Invoices", not "Billing documents
  entity"). Name the object and the action.
- **Short, front-loaded**: the key word first, so a scan catches it ("Billing settings", not "Settings for billing").
  Labels one to three words; buttons two or three; a helper line one sentence.
- **One name per thing**: pick a term and keep it on every screen, in every state — no "workspace" here and "team"
  there for the same thing.
- **Sentence case** for headings, labels and buttons unless the product's system says otherwise.
- **Second person**: talk to the person as "you"; don't mix "my" and "your" for the same thing.
- **Active voice, present tense**: "We couldn't save your changes", not "Your changes could not be saved".
- **Numerals for counts** ("3 projects"), a space between number and unit, glued with a non-breaking space
  ("10&nbsp;MB"), dates, times, numbers and currencies in the person's locale, and one currency precision in a view.
- **Typography**: the ellipsis character `…`, curly quotes, no exclamation marks except a rare real celebration.

## Buttons and links

- A verb and its object that say the outcome: "Create invoice", "Send 3 invites", "Save changes", "Delete project".
  Avoid "Submit", "OK", "Yes", "Continue" when a specific verb exists.
- A button that opens something needing more input ends with an ellipsis ("Rename…"); in-progress states too
  ("Saving…", "Generating…").
- Link text says where it goes ("View billing history"), never "Click here" or "Learn more" alone.

## Errors

- Say what happened and how to fix it, in that order, in the field's own words: "Card declined. Try another card or
  contact your bank." — "That name is taken. Try another."
- Don't blame, joke or apologise at length: no "Oops", "Invalid", "Illegal", "You forgot". No error codes on their own
  (put a code after the human sentence if support needs it).
- A system failure says what still works and what to do: "Reports are slow to load right now. Your data is safe —
  try again in a few minutes."

## Empty states

- Three parts, briefly: what this place is, why it is empty, the action that fills it. "No API keys yet. Create a key
  to connect your app." + "Create API key".
- No results: echo the search ("No results for 'acme'") and offer the way out ("Clear filters").

## Confirmations, toasts and notices

- **Confirmation dialog**: title asks the specific question ("Delete 3 projects?"); body says the consequence ("Their
  files and history are deleted for everyone. This can't be undone."); buttons repeat the verb ("Delete projects",
  "Cancel").
- **Toast**: past tense, a few words, with Undo where possible ("Project archived · Undo").
- **Banner**: what is true now and what to do ("Your trial ends in 3 days. Add a payment method to keep your data.").
- **Tooltip**: a few words for an icon or term; never information someone needs to complete the task.

## Headings

- **Web product**: nouns that name the place ("Invoices", "Team members").
- **Landing**: claims that make a point ("Close the books in a day"), backed by the section beneath; a subhead that
  says how. Concrete outcomes beat adjectives; a superlative needs proof the person gave.
- **Mobile app**: short titles that fit the navigation bar without truncating at the largest text size.

## Writing for every language

- Leave room: translations can run 30–40% longer (more for short labels); layouts wrap rather than clip.
- Never build a sentence from fragments ("You have " + n + " items"); use full messages with plural rules.
- No text inside images; brand and product names marked so automatic translation leaves them alone
  (`translate="no"`).
- Detect the language from the person's settings, not their location.

## Review checklist

- [ ] Every fact on screen is real; missing ones are open questions with neutral wording meanwhile.
- [ ] One term per concept across screens and states.
- [ ] Buttons say the outcome with a verb and object; no bare "Submit", "OK" or "Click here".
- [ ] Errors say what happened and how to fix it, without blame, jokes or bare codes.
- [ ] Empty states say what, why and what to do, with the action.
- [ ] Confirmations name the action and consequence; buttons repeat the verb; toasts offer Undo.
- [ ] Sentence case, numerals, glued units, locale formats, `…` and curly quotes.
- [ ] Copy fits when 40% longer; no concatenated sentences; no text in images.

Sources: GOV.UK content and error-message guidance (Open Government Licence); Vercel's Web Interface Guidelines (MIT),
including its copywriting section; Nielsen Norman Group on microcopy; Shopify Polaris and Atlassian content guidance
— in our own words.
