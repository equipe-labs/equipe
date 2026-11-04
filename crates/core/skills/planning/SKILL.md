---
name: planning
description: Use when work went through a brief or will take several tickets — write the plan with `write_plan`, have it reviewed, put the scope to the person, then cut the tickets.
---

# A plan before the tickets

Work that went through a brief, or that will take several tickets, gets a plan before any ticket: write it with
`write_plan` — the goal, the approach, the architecture and data flow, the edge cases, the test plan as concrete
cases, design notes and risks, and the most ambitious version worth building, so the scope is a real choice. In the
same call ask for its reviews (`reviewers`), all at once: an engineer for the architecture, edge cases and test plan,
the designer for what people will see against the UI map, QA for whether it can be tested; each writes its review into
the plan. Take in what they found, then put the scope to the person as one `ask_decision` question with four options —
expand it, expand selectively, hold the scope, reduce it — your recommendation first, each saying in a line what it
adds or cuts; write their pick into the plan (`scope`). Decide small things yourself by stated principles — the
simpler way, what the code already does, what can be undone, the smaller scope when in doubt — and note each in the
plan: ask only what changes the outcome. Then cut its tickets with `create_ticket`, each with `plan`, its acceptance
criteria and its cases from the test plan (`tests`). A small fix skips all of this.
The plan is kept in the squad's own folder, never in the project's repository: its tickets name it by its absolute
path, which their owners read from their worktrees, and what in it is about the person or their team never goes into
the repository's files, commits or pull requests.
