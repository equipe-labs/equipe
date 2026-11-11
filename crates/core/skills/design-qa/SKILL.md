---
name: design-qa
description: Use when equipe gives you a design QA ticket — check what an engineer built against the spec, the tokens and accessibility, and finish it with `design_qa`.
---

# Design QA

Design QA: when an engineer hands in a code ticket that changes what users see, equipe gives you a design QA ticket
before the person reviews it. What sends it back is a blocker (S1/S2): function or data broken, layout broken at a
major width, a WCAG 2.2 A/AA failure — content or a control lost at 200% text or at 320px reflow, contrast — or a clear
difference from the spec. What keeps it from looking finished — rhythm, alignment, density, hierarchy, the first
impression — is polish (S3/S4): always written down, even on a pass, and never a reason to send it back. Finish it
with `design_qa`, its report written under `design/qa/`.

## With equipe's precheck report

Most design QA tickets come with a report from equipe's precheck: it already rendered the screens the change touches
— each width, light and dark, at normal size, with text at 200% and zoomed to 200% — measured them (text cut off, text
over text, sideways scroll, axe-core's WCAG rules), and compared each picture with the UI map's at the base. Start from
it, not from scratch:

- Read its summary and look at its flagged pictures and the changed states only. Don't shoot every condition again,
  and don't re-measure what it measured; a finding it marks as there before isn't this change's.
- Spend your look on what it can't judge: whether the changed screens match the spec or mockup and what the ticket
  asked, their states (empty, loading, error, long text, hover, focus), and whether a flagged finding truly costs the
  person something.
- Render yourself only what it couldn't — pictures that failed, states it left out, changed UI files no screen lists
  — or a state you need to see: its preview from its worktree, `equipe design render http://localhost:…` or
  `equipe design flows shoot --screen <screen>`. Its pictures count as your own for a pass.

## Without one

Run the built screens yourself — its preview from its worktree, shot with `equipe design flows shoot` (`equipe design
render http://localhost:…` for what isn't on the map) at each width, light and dark — and put them beside the spec;
never pass on the engineer's screenshots alone.

## Either way, on the changed screens

Pass only when the map's pictures of what it changed are up to date. Hold each changed screen to the system in code
too: `equipe design lint <its changed source> --render --url <its preview> --screen <flow/screen>`. A floor finding that
loses content or fails WCAG A/AA is a blocker, never waived; a habit is polish unless it breaks the spec; finishing
notes go in the report as polish. Before you pass a changed screen, `equipe design second-look <flow/screen>
--criteria "<the ticket's acceptance criteria>"` gives it fresh eyes; what it finds is a blocker only when it is S1/S2.
A redesign of a screen also passes `equipe design kept check` against what was recorded before it.
