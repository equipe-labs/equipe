# Demo video — script and shot list

Target length **2:40–2:50** (the rules require under 3 minutes). Public YouTube video.
English narration below; subtitles in [`demo.en.srt`](demo.en.srt) match it line for line.
A Korean narration with these accurate English subtitles is also acceptable — if so, translate
the narration and keep the SRT as is. **Who records, who uploads, and making it public are the
owner's decisions** (not decided yet).

## Rules for the recording

- **No secrets on screen**: never show `NEBIUS_API_KEY`, the Token Factory console's key page,
  `env`/`printenv`, shell history, `~/.equipe` files, or an `Authorization` header. Set the key
  with `read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY` *before* recording the terminal.
- **Only real calls for the Nemotron parts.** Shots 2–6 must show the real Token Factory
  answering (a real model id and request id). The fake daemon's `FAKE_REVIEW=1` reviews are
  canned: use them only to rehearse framing, or label them on screen "UI mock — not a model
  call". Never present them as Nemotron output.
- Use a small public demo repository (no private code goes to the API or on screen).
- Bigger font in the terminal (≥ 18 pt), desktop app window at 1440×900 or so; hide
  notifications; English UI (`equipe lang en`) so the subtitles match the screen.

## Shot list

The whole squad runs on NVIDIA Nemotron through Nebius Token Factory in this version: record
it with **no other engine signed in** (Claude Code signed out, Codex not logged in). Shots 2–4
need the owner's **real member run** (`verification.md` §3.2) to have worked; until it has,
the members' part is only mock-verified and must not be shown as working. If it can't be
recorded, use the review-only script from Release 0.88.0 (`git show 2b9af39:docs/hackathon/demo-script.md`
and its `demo.en.srt`) instead.

| # | Time | What to record | Narration (English) |
|---|---|---|---|
| 1 | 0:00–0:14 | Title card: "equipe — a squad of AI coding agents on NVIDIA Nemotron, Nebius Token Factory". Then the desktop app with the squad (PM + members) and the board. | "This is equipe: a squad of AI coding agents that works like a team — a PM, engineers, a board of tickets, and you accepting the work." |
| 2 | 0:14–0:38 | `equipe status` (key set off camera): Claude Code and Codex not signed in, **nemotron · ready**. Then talk to the PM: ask for a small feature in the demo repo; the PM writes a ticket; it's assigned. Zoom on a member's notice "Runs on NVIDIA Nemotron (nvidia/…) through Nebius Token Factory". | "Every member here runs on an NVIDIA Nemotron model on Nebius Token Factory — no other AI subscription, just a Nebius key and the open-source Codex CLI as the agent loop. I ask the PM for a small change; it writes a ticket and hands it to an engineer." |
| 3 | 0:38–0:58 | **A member writing code on Nemotron**: the engineer's conversation in its own worktree — `Bash` steps writing the file, running `cargo test` (or the project's tests), a failing run then a passing one if it happens; then it hands the ticket in (ticket → Review). | "Nemotron decides what to do and Codex's tools do it, in the engineer's own git worktree: it writes the code, runs the tests in a sandbox, and hands the work in. My checkout is never touched." |
| 4 | 0:58–1:32 | (Real call.) Seconds later the peek shows **"Nemotron review · nvidia/…"**: verdict chip, summary, risks as file:line, missing tests. Zoom on the model id. | "Then a second Nemotron call reviews the hand-in. It reads the diff against the ticket and comes back with a verdict, a short summary, risks with the file and line, and the tests the change is missing — in my language." |
| 5 | 1:32–1:50 | Ticket record showing the "nemotron · reviewed" entry; then send the work back (or accept) — the person decides. | "It's advice, not authority: the review goes on the ticket's record, and I still decide — here I send it back with the risk it found." |
| 6 | 1:50–2:16 | Terminal (key already set, not shown): `equipe review --models`, then `equipe review --base main` in the demo repo; the output with verdict, risks, model id, tokens and request id. Then `grep 'turn on Nemotron' ` on the daemon's log: request ids and tokens of the members' turns. | "Everything is on the record: equipe review lists the account's Nemotron models and reviews any repository's changes, and the daemon logs each member turn's Token Factory request ids and tokens." |
| 7 | 2:16–2:36 | The diagram from `docs/hackathon/architecture.md`: member → codex app-server → equipe's relay → Token Factory `/v1/responses`; hand-in → chat completion → review. | "Under the hood, members run Codex's agent with Token Factory's Responses API as its provider, through a small relay that keeps every tool a plain function. The key stays in the environment and never reaches the logs." |
| 8 | 2:36–2:48 | End card: GitHub URL, "MIT", "Nebius Token Factory · NVIDIA Nemotron". | "equipe is open source under MIT. A whole team of agents, running on NVIDIA Nemotron on Nebius." |

Total ≈ 2:48. If it runs long, cut shot 5 to its last sentence. A member turn takes as long as
it takes: speed shot 3 up in editing (say so on screen: "sped up").

## What to record in the real app / CLI

1. Demo repository: a tiny public web or Rust project with a `main` branch.
2. Daemon with the key: `equipe daemon stop`, then in a terminal that is **not** recorded:
   `read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY && equipe daemon run`.
3. Desktop app connected; squad with a PM and one engineer, on Nemotron (nothing else signed in;
   the codex CLI installed; `EQUIPE_NEBIUS_MODEL` set to the model you want on screen).
4. The ticket: something with a clear acceptance criterion the first attempt may miss
   (e.g. "the form refuses an empty email"), so the review has something to say.
5. CLI shots in a separate, recorded terminal (key exported earlier, off camera).
6. Keep the raw recording, the printed request id and the daemon log's `turn on Nemotron`
   lines; add them to the evidence tables in `verification.md` (§2 and §3.2).
7. Agent turns can take many tokens: set a spending limit on the Token Factory account
   before rehearsing, and keep the ticket small.
