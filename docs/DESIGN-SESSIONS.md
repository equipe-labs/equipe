# Design sessions: the live design loop

> See it applied, look at it, touch it, give feedback, iterate — and only then move it into real code.

A design session is the person and the squad's designer on one page, together, in rounds of minutes. The designer
works on a **runnable prototype** that is live in the person's window; the person clicks on it to comment, compares
variants side by side, and picks one with **Use this**. Only then does an implementation ticket — with the prototype
as its spec — run the full design checks.

This page describes the loop end to end. The rules live in `crates/core/src/design/session.rs` (no runtime), the
daemon side in `crates/core/src/runtime/design_session.rs`, the screens in `crates/desktop/src/design_session.rs`
and `crates/desktop/src/views/{design_session,preview}.rs`, and the designer's instructions in
`crates/core/skills/design-session/SKILL.md`.

## 1. Start

The design screen's **Start a design session** (or the PM's proposal card) sends `Command::StartDesignSession`. The
daemon sends the designer a `<design-session>` kickoff as the person's message, tells every screen
(`Event::DesignSessionStarted`), and from then on keeps the session **on** (`design::session::Active`) until the person
ends it (`Command::EndDesignSession`), picks a variant, or leaves it idle for four hours.

The kickoff puts the designer in **prototype mode**:

- **References first.** Before proposing a direction — and whenever the person asks for new directions — it pulls 3–5
  references from the connected design MCPs (21st.dev, Mobbin; else the web) and shows them with `show_references`
  (cards with "Try it").
- **A runnable prototype** in `design/sessions/<date>-<page>/` (HTML/CSS from the project's tokens, or a route of the
  dev server in its worktree), shown with `equipe design session show <file | http://localhost:port/path>`.
- **Variants** with `equipe design session show --variant A|B|C <target>`: the screens put them side by side.
- **A change log**, `CHANGELOG.md` beside the prototype, one line per round.

## 2. Fast rounds

While a session is on, each of the person's messages to the designer opens with equipe's round note
(`design::session::ROUND_RULES`): reply in a few lines, apply the feedback to the prototype within a couple of minutes
and show it again, add a change-log line — and **no heavy checks in a round**: no width × theme × text-size matrix, no
axe or contrast runs, no design QA, no new ticket per feedback item.

**The person's messages come first** (`Runtime::person_first`, `design::session::preempt`). A message said while the
designer's turn runs is taken in as a steer and read at its next step; if a command is running (a build, a capture
run) that command is stopped so the step comes at once. On an engine that can't take words mid-turn, the turn is
stopped and the message goes in as the next one. Ticket runs are separate sessions, so they never hold up the
designer's conversation.

## 3. Live, side by side

What the designer shows opens beside the conversation. A page served on the daemon's machine opens in the **live
preview** — a real WebKit view on the person's screen, reached over the window's SSH forward when the daemon is on
another Mac — and reloads each time the designer shows it.

With two or more variants, the bar shows a letter for each and **Side by side**: one `data:` page of frames, each
variant at its address through the preview's own forward, scaled to its column
(`design_session::side_by_side_page`). Variants are compared live, not as screenshots.

Before a page goes out, `show` checks it. Every file it names has to be inside its design folder (or, for an
address from the root, in the project). The page also has to open, styled and not blank, in headless Chrome. If
any of that fails, the designer gets "Not sent:" with the list and a screenshot, and nothing reaches the person.
If it passes, the designer gets the screenshot of what the person sees (docs/REMOTE.md).

## 4. Point and comment

**Drop a pin** in the preview's bar, click an element, and type what should change. The pin goes with the next
message as a `<design-pins>` block:

```
<design-pins>
on: http://localhost:5173/pricing at 1440 wide
- pin 1 at 130,1210: tighter, less shouty
  element: main > section:nth-of-type(2) > button
  text: "Start free trial"
  box: 100,1190 160x40
  crop: attached picture 1
…
</design-pins>
```

The crop is a snapshot of the preview's own web view (`WKWebView takeSnapshotWithConfiguration:`) around the element
— WebKit's offscreen rendering of that page, **never a capture of the person's screen**. The designer marks each pin
done with `equipe design session done <n> "<what changed>"`, and the pin turns into a check.

## 5. Use this → code

**Use this** (or **Use B**) sends `Command::UseDesignVariant`. The daemon opens the implementation ticket — titled
"Build <page> from prototype B", for the squad's frontend member when it has one — whose body names the page,
project, variant, prototype address and change log, and says the full checks run there
(`design::session::implementation_body`). Picking again before that ticket starts updates it instead of opening
another. The designer is told the rounds are over (`design::session::picked_line`) and its turns stop being prototype
rounds. Every screen hears `Event::DesignVariantUsed`; the composer says which ticket was opened.

Only that ticket runs the full design pass: every state, light and dark, the widths, larger text and accessibility.
Its **design QA gets no designer turn**: the ticket carries an `approved prototype` record, and its design QA's
precheck alone decides (`design_qa::gate_prototype`, whatever the squad's gate mode). Built screens that rendered at the
checked widths with no new S1/S2 finding pass straight on to review, with the precheck's pictures on the ticket and
"See the result" pointing at them and at the prototype. Only a precheck that failed, saw too little, or found S1/S2
problems hands it to the designer, with the report. (The precheck checks the built screens; a pixel comparison with
the prototype itself is not done yet.)

## Members changing their own work from the conversation

The person shouldn't have to answer a stale card or edit the board to steer work they are already discussing:

- `withdraw_decision` — a member takes back its own open decision cards (one, those of a ticket, or all), and asks
  anew with `ask_decision` if something is still the person's to choose.
- `revise_ticket` — a member changes one of its own tickets that hasn't started (title, body, waits) or drops it.
- While a member has cards open, the person's next message to it carries a note listing them and saying to withdraw
  or replace one their words settle — never to ask them to answer it a certain way.

Each change shows as a line of news in the member's conversation, and the PM hears of it.

## The live line

Every member conversation shows, at its foot, one quiet line while the member has a turn under way or is otherwise
busy (`crates/desktop/src/status_line.rs`): what it is doing now in plain words (reading the person's message,
thinking, the step it runs and on what, writing, waiting on a background command, summarizing the conversation,
waiting for a sign-in or an approval), for how long, how many steps and tokens so far — opening to the step's detail —
and, while the person's words wait, when they will be read with **Read now**.
