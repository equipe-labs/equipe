# Cost levers for hosted teams

A hosted team pays for every model call with credits, through a gateway that prices each call from the usage the
provider reports. These are the engine's parts of keeping that spend honest. All of them apply only on the **credits
profile** (`EQUIPE_AI=credits` in the daemon's environment, set by the hosted machine's setup) unless it says otherwise: a
person's own Claude plan, and every other provider, behave as before.

Code: `crates/core/src/cost_levers.rs` (profile, attribution, caps, effort), `runtime/heartbeat.rs` (PM wakes).

## Attribution

Each Claude Code turn carries `x-equipe-member`, `x-equipe-role`, `x-equipe-ticket` (`T-12`) and `x-equipe-turn` request
headers (Claude Code's `ANTHROPIC_CUSTOM_HEADERS`, merged with whatever was already set). The turn kind is one of
`pm_wake` (equipe woke the PM: a notice, a hand-in, a routine), `pm` (the PM answering or planning), `ticket` (a member's
lane), `design` (a designer's lane), `design_qa` (the light reviewer) and `desk` (a member's own conversation). Values are
short plain tokens (letters, digits, `-_.@`); never a prompt, a path or a person's words. The gateway strips them before the
request goes upstream and keeps them in the ledger line, so spend can be told by role, ticket and kind of turn.

## PM wakes

Every turn the PM takes reads its whole context, and a cold one writes it again, so a wake that cannot change anything is
money for nothing.

- Notices that only inform the PM no longer wake it. They are kept and go with its next message: a bottleneck
  escalation (the line itself says to bring it up in the next message to the person), and the digest of work members
  landed themselves (it says there is nothing to do if they are fine).
- What the heartbeat hands the PM in one pass (reminders to review, uncovered requests, stale waits, an idle board, a
  failing self-watch) goes in ONE wake, in the order it came, instead of one turn each.
- Everything that needs a decision still wakes the PM at once: the person's messages, questions, a ticket handed in, a
  ticket handed back, a signal that fails, a lane that couldn't start.

## Limits on what a turn drags along

- Claude Code's own Bash output is cut at 15,000 characters (its default is 30,000) and an MCP answer at 10,000 tokens
  (its default is 25,000; equipe's own tools already cut at 10,000, `tool_cap`). Reading a file is not limited: a file cut
  short costs more steps than it saves.
- A member's lane that no tier or person gave an effort (the PM's and the designer's are never touched) runs at
  `medium` instead of Claude Code's `xhigh`.

## Tried and not applied

- **Routine screens on the standard tier (Sonnet) when a hosted team's ticket names the fixed stack.** Cost per
  accepted ticket fell 46%, same smoke-check pass rate, but a blind Opus judge preferred the Opus build in 3 of 3 pairs
  (visual 8.1 vs 7.0, clarity 8.3 vs 7.5). Not applied; the existing rules for routine, checked changes are unchanged.
- **A limit of two designer rounds in design QA.** Of the 5 valid paired loops, 3 needed 3 to 4 rounds to clear a blocker,
  so the limit would have shipped them with the blocker open. Not applied.

## How they were checked

See the measurement notes in the cloud repository's `docs/cost-levers-ab-2026-11-22.md`: fixed ticket specs on a fixed
repository snapshot, current against changed, three runs each, cost per accepted ticket, bounce-backs, smoke-check pass
rate, precheck S1/S2 counts and a blind Opus judge.
