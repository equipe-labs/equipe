---
name: design-qa
description: Use when equipe gives you a design QA ticket — check what an engineer built against the spec, the tokens and accessibility, and finish it with `design_qa`.
---

# Design QA

Design QA: when an engineer hands in a code ticket that changes what users see, equipe gives you a design QA ticket
before the person reviews it. Run the built screens yourself — its preview from its worktree, shot with `equipe design
flows shoot` (`equipe design render http://localhost:…` for what isn't on the map) at each width, light and dark — and
put them beside the spec; never pass on the engineer's screenshots alone, and pass only when the map's pictures of
what it changed are up to date. Differences from the spec, the tokens and accessibility are blockers and send it back;
what keeps it from looking finished — rhythm, alignment, density, hierarchy, the first impression — is polish, always
written down, even on a pass. Finish it with `design_qa`, its report written under `design/qa/`.

Hold each changed screen to the system in code too: `equipe design lint <its changed source> --render --url <its
preview> --screen <flow/screen>`. A floor finding is a blocker and sends it back, never waived; a habit is a blocker
unless the spec or the person kept it (then it is in `design/decisions/`); finishing notes go in the report as
polish. Before you pass it, `equipe design second-look <flow/screen> --criteria "<the ticket's acceptance
criteria>"` gives it fresh eyes, and what that look asks to fix is a blocker too. A redesign of a screen also passes
`equipe design kept check` against what was recorded before it.
