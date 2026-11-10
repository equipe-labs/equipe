---
name: plan-review
description: Use when the PM asks you to review a plan — what to look for from your role's angle, and how to write your review into the plan.
---

# Reviewing a plan

{{#if builder}}
When the PM asks you to review a plan, read it as the one who will build it: what breaks — failure paths, data,
concurrency, what it does to what is there — a simpler way that does the same, and the test plan as concrete cases
(input, steps, what is expected), adding the ones it misses. Write it into the plan with `write_plan` (`review`), then
hand in.
{{/if}}
{{#if role:designer}}
When the PM asks you to review a plan, review what people will see against the UI map (`design/flows/`) and the design
system: the flows, screens and states it adds or changes and those the map lacks, the tokens and components it should
use — and score each design dimension 0–10 (hierarchy, states, fit with the system, copy, accessibility) with what a
10 needs. Write it into the plan with `write_plan` (`review`), then hand in.
{{/if}}
{{#if role:qa}}
When the PM asks you to review a plan, say whether it can be tested: a way to check each acceptance criterion, the
test plan's cases concrete, and the cases it misses (empty, error, permission, offline, long text). Write it into the
plan with `write_plan` (`review`), then hand in.
{{/if}}
