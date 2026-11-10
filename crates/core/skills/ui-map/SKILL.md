---
name: ui-map
description: Use when a screen, state or flow changes, when you shoot, record or check the UI map in `design/flows/`, or when the person talks about one screen or asks to see a flow as a video.
---

# The UI map

The UI map is yours, so the person can see the whole service at a glance: `design/flows/flows.json` names every flow
(onboarding, each feature) and its screens in order, each screen with every state that can happen — those nobody meets
by using it included; a state's `kind` is `default`, `empty`, `loading`, `error`, `long` (long text), `success`,
`permission`, `offline` or `first` (first run) — and its pictures beside it,
`flows/<flow>/<screen>/<state>.<mode>.<width>.png`, light and dark at each width the product supports. A screen's
`expects` lists the states it should have. `equipe design flows init` starts it; `equipe design flows shoot`
(`--flow`, `--screen`) shoots them from the running app, a state reached through the fixture or URL parameter in its
`url`, in an en-US browser unless the state (or its screen, or its flow) says otherwise: `locale` (`de-DE`),
`js: false`, `reducedMotion: true`, `blockFonts: true` (as it looks while webfonts load), `fullPage: false` (the first
screen only), `selector` (that element alone, sticky headers out of the way), `permissions` (`clipboard-write`) and
`actions` (the walk's `click`, `fill`, `press`, `wait` steps, run before the shot); the page is scrolled through first,
so what appears on scroll has. A flow's `locale`, `reducedMotion` and `permissions` are also the browser its walk is
recorded in.

An app that isn't a web page (a desktop app, a phone simulator), or one that can render the person's own state
read-only (equipe's desktop app does): read `references/other-apps.md` next to this file before you shoot it.

Draw a state as a mockup, rendered to the same path, only when nothing can reach it.

Each flow keeps a `walk` beside its screens, played into a video of the flow: read `references/walkthrough.md` next to
this file whenever you write or change a `walk`, record a flow, or the person asks to see a flow as a video.

A screen whose code changed after its pictures is stale, and a stale picture is a defect: whenever a screen changes,
its states and pictures, and its flow's `walk` and video, change in the same hand-in, and you hand in only when
`equipe design flows check` finds nothing missing or stale.

When the person talks about one screen ("Onboarding › Sign in › Error", its picture attached), answer about that
screen and state, and propose the change as a picture beside the one it replaces.
