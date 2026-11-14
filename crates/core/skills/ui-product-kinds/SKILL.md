---
name: ui-product-kinds
description: Use first on any UI work — classify it as landing page, web product or mobile app; each has its own references and bar.
---

# Three kinds of product, three bars

Before references, before a wireframe, name the kind of the work in one line of your brief or hand-in — `Kind:
landing`, `Kind: web product` or `Kind: mobile app` — and why. The kind decides where references come from, which rules
weigh most, how much motion and decoration is welcome, and what "best in class" means. Judging one kind by another's
bar is the commonest way a careful screen still comes out wrong: a dashboard dressed as a landing page, a landing page
as quiet as a settings screen, a phone app that is a shrunken website.

## Classify

- **Landing** — a page that has to explain or persuade someone who hasn't committed yet: a marketing site, product or
  feature page, pricing, a launch, a waitlist, a docs home. Read once or twice per visitor. Success: they understand it
  and take one action.
- **Web product** — a tool people come back to for work: dashboards, tables, admin, settings, editors, SaaS apps,
  internal and developer tools. Used for hours, again and again. Success: the task done fast, without errors.
- **Mobile app** — a native or native-feeling app on a phone (iOS, Android, or one cross-platform app): onboarding,
  feeds, capture, forms, settings. Used one-handed, in short, interrupted sessions. Success: done with the thumb.

Mixed work is classified per surface: a SaaS company has a landing (its site), a web product (its app) and perhaps a
mobile app. A sign-in or checkout page belongs to the product it opens. A web product seen on a phone stays a web
product, with the touch rules of `ui-mobile` at its narrow widths. When the kind is unclear, ask one question rather
than guess.

## Landing

- **References**: galleries of marketing sites fit here and only here (Awwwards, Land-book, Lapa Ninja), plus the
  sites of the best companies in the same category. Reference families: "a product-led page in the manner of Linear's
  or Vercel's site", "an editorial page in the manner of Stripe's", "a calm one-screen page in the manner of
  Raycast's".
- **Narrative**: hero → proof → how it works → what it does for you (outcomes, not a feature list) → objections
  (pricing, security, FAQ) → one closing call to action. Each section makes one point; its heading states that point as
  a claim, not a label ("Ship reviews in minutes", not "Features").
- **Hero**: what it is and for whom in a headline of about 4–10 words, a subhead of at most about 25 words, one
  primary call to action and at most one secondary link, and real evidence of the product (a screenshot, a short
  muted loop, a live demo) — never an abstract blob standing in for it. All of it inside the first screenful at 1440
  and at 390.
- **Proof**: only real logos, numbers and quotes the person gave (the floor refuses invented ones). Put each next to
  the claim it backs. No proof yet: say less, show the product more.
- **Type and space**: display sizes are welcome — 48–80px headlines on desktop, 32–44px on phones; generous section
  spacing (96–160px desktop, 64–96px phone); body text 16–20px at 45–75 characters a line.
- **Motion budget**: reveals once, 300–600ms, ease-out, staggered by at most 60ms; nothing hides content a reader
  needs until it scrolls into view; no scroll-jacking; looping video muted, pausable, with a still for reduced motion.
- **Speed**: LCP under 2.5s on a phone at the 75th percentile; the hero image sized and prioritised; at most two
  font families.
- **Widths**: design at 1440 and 390; check 1280, 768 and 1920.
- **The bar**: a stranger can say what it is, who it is for and what to do next after five seconds on the first
  screenful; beside the references, it looks as considered as they do.
- **Don't**: three equal icon cards as the whole story; a gradient hero with no product in it; everything centred; a
  different call to action in every section; a feature list with no outcome.

## Web product

- **References**: dense tools only — public design systems built for them (Carbon, Polaris, Primer, Geist,
  Atlassian), and the best products in the category at work, not their marketing sites. Reference families: "a dense
  list in the manner of Linear", "a financial dashboard in the manner of Stripe or Mercury", "a developer console in
  the manner of Vercel's dashboard", "a calm document in the manner of Notion". Marketing galleries are never a
  reference for this kind.
- **Density**: built for repeated use. Body text 13–14px (16 for reading-heavy tools), table rows 32–40px, controls
  28–36px tall, a 4px grid. Information first; chrome thin.
- **Shell**: a sidebar (200–260px, collapsible) once there are more than about five sections, else a top bar; a page
  header with the title, its context and the one primary action on the right; full width for tables and boards; forms
  and reading at 640–720px.
- **Type**: two or three sizes per block, headings 16–24px, no display type; tabular figures everywhere numbers line
  up.
- **Colour**: neutral surfaces, one accent for the primary action, selection and focus; status colours used only for
  status.
- **Motion**: 100–200ms, only to show a state change or where something went; none on keyboard actions or on load.
- **Keyboard**: frequent actions have shortcuts; a large app has a command palette (`⌘K` / `Ctrl+K`); focus is
  managed through every overlay.
- **Widths**: design at 1280 and 1440; 1024 must work fully; at 390 people can read and do the simple actions.
- **The bar**: someone who uses it daily finds and does the thing in seconds; every state is designed; beside the
  references it is as dense, as quiet and as aligned.
- **Don't**: hero headings, illustrations, gradient panels or marketing copy inside the tool; a card around every
  block; KPI tiles with no comparison; charts as decoration.

## Mobile app

- **References**: the platform's own guidance first — Apple's Human Interface Guidelines for iOS, Material 3 for
  Android — then the best apps in the category on the same platform, and mobile flows from a library the squad has
  connected (Mobbin, through its official MCP server or the person's own signed-in browser). Never a website's layout.
- **Platform**: iOS — a tab bar of three to five items, navigation bars with back at the top left and edge swipe to
  go back, sheets for focused tasks; Android — a navigation bar or rail, a top app bar, the system back. A
  cross-platform app shares its brand and follows each platform for navigation and controls.
- **Touch**: targets at least 44×44pt (iOS) or 48×48dp (Android), 8pt apart; the primary action in the bottom third,
  within thumb reach; destructive actions away from frequent ones.
- **Type**: the platform's text styles (iOS body 17pt, Android body 14–16sp), and the screen still works at the
  largest accessibility sizes.
- **Layout**: one column, 16–20pt side margins, safe areas respected (status bar, notch, home indicator), nothing
  under the keyboard.
- **Motion**: the platform's transitions (push, sheet, fade); 200–350ms; a light haptic to confirm, sparingly.
- **Widths**: design at 390×844 and 360×800; check 375×667 (a small phone) and 430×932 (a large one); landscape and
  tablets only where the app supports them.
- **The bar**: beside the best apps on that platform it feels native — the same navigation, the same controls, the
  same weight — and every task is done with one thumb.
- **Don't**: a hamburger menu as the main navigation; a web-style footer; hover-only affordances; tiny text links as
  the main action; custom controls where the platform has one.

## What the kind changes, at a glance

| | Landing | Web product | Mobile app |
|---|---|---|---|
| First question | What is it, for whom, what next? | Can I do the task fast? | Can I do it with one thumb? |
| References | Marketing galleries, category sites | Dense tools, design systems | HIG, Material, mobile flows |
| Primary widths | 1440, 390 | 1280, 1440 | 390, 360 |
| Body text | 16–20px | 13–14px | 17pt / 14–16sp |
| Motion | Reveals up to 600ms | 100–200ms, state only | Platform transitions |
| Decoration | Welcome when it carries the story | Almost none | The platform's own |

The other `ui-*` skills hold for every kind; where a rule differs by kind, they say so.

## Review checklist

- [ ] The kind is named in the brief or hand-in, with why; mixed work is classified per surface.
- [ ] The references come from sources that fit the kind (`fits:` in the sources file) — no marketing gallery behind a
      web product, no website behind a mobile app.
- [ ] Designed at the kind's primary widths, checked at the others.
- [ ] Landing: the first screenful says what, for whom and what next, with one call to action and real product
      evidence.
- [ ] Web product: density, type sizes and motion inside the kind's ranges; no marketing devices in the tool.
- [ ] Mobile app: platform navigation and controls, targets at the platform's minimum, the primary action in thumb
      reach, safe areas.

Sources: Apple Human Interface Guidelines; Material Design 3; web.dev on Core Web Vitals; IBM Carbon; Shopify Polaris;
GitHub Primer; Vercel Geist — summarised in our own words (see `ui-review`'s `references/sources.md`).
