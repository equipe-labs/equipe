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
- **Only real calls for the Nemotron parts.** Shots 4–6 must show the real Token Factory
  answering (a real model id and request id). The fake daemon's `FAKE_REVIEW=1` reviews are
  canned: use them only to rehearse framing, or label them on screen "UI mock — not a model
  call". Never present them as Nemotron output.
- Use a small public demo repository (no private code goes to the API or on screen).
- Bigger font in the terminal (≥ 18 pt), desktop app window at 1440×900 or so; hide
  notifications; English UI (`equipe lang en`) so the subtitles match the screen.

## Shot list

| # | Time | What to record | Narration (English) |
|---|---|---|---|
| 1 | 0:00–0:14 | Title card: "equipe — a squad of AI coding agents, reviewed by NVIDIA Nemotron on Nebius Token Factory". Then the desktop app with the squad (PM + members) and the board. | "This is equipe: a squad of AI coding agents that works like a team — a PM, engineers, a board of tickets, and you accepting the work." |
| 2 | 0:14–0:38 | Talk to the PM: ask for a small feature in the demo repo. The PM creates a ticket; it's assigned; the engineer works on its own branch (status: in progress). | "I ask the PM for a small change. It writes a ticket with acceptance criteria and hands it to an engineer, who works on its own git branch, in its own worktree, so my checkout is never touched." |
| 3 | 0:38–0:58 | The engineer hands it in: ticket moves to Review; open the peek with the hand-in note and changed files. | "When the work is done, the engineer hands it in. Until now, the only one who had read this code was the agent that wrote it." |
| 4 | 0:58–1:32 | (Real call, daemon running with the key.) Seconds later the peek shows **"Nemotron review · nvidia/…"**: verdict chip, summary, risks as file:line, missing tests. Zoom on the model id. | "So every hand-in now goes to an NVIDIA Nemotron model on Nebius Token Factory. It reads the diff against the ticket and comes back with a verdict, a short summary, risks with the file and line, and the tests the change is missing — in my language." |
| 5 | 1:32–1:50 | Ticket record showing the "nemotron · reviewed" entry; then send the work back (or accept) — the person decides. | "It's advice, not authority: the review goes on the ticket's record, and I still decide — here I send it back with the risk it found." |
| 6 | 1:50–2:16 | Terminal (key already set, not shown): `equipe review --models`, then `equipe review --base main` in the demo repo; the output with verdict, risks, model id, tokens and request id. | "You don't need the whole squad to try it. With only a Nebius key, equipe review lists the account's Nemotron models and reviews any git repository's changes — same client, real tokens, real request id." |
| 7 | 2:16–2:36 | The architecture diagram from `docs/hackathon/architecture.md` (or a slide): hand-in → git diff → Token Factory chat completion → CodeReview → screens. Then a few lines of `nemotron.rs` (the prompt's "data, not instructions" line). | "Under the hood it's one small Rust client: an OpenAI-compatible chat completion to Token Factory. The diff is treated as data, not instructions; the answer must be one bounded JSON review, or it's rejected — never made up. The key stays in the environment and never reaches the logs." |
| 8 | 2:36–2:48 | End card: GitHub URL, "MIT", "Nebius Token Factory · NVIDIA Nemotron". | "equipe is open source under MIT. A team of agents, and a second pair of eyes from NVIDIA Nemotron on Nebius." |

Total ≈ 2:48. If it runs long, cut shot 5 to its last sentence.

## What to record in the real app / CLI

1. Demo repository: a tiny public web or Rust project with a `main` branch.
2. Daemon with the key: `equipe daemon stop`, then in a terminal that is **not** recorded:
   `read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY && equipe daemon run`.
3. Desktop app connected; squad with a PM and one engineer (needs Claude Code or Codex).
4. The ticket: something with a clear acceptance criterion the first attempt may miss
   (e.g. "the form refuses an empty email"), so the review has something to say.
5. CLI shots in a separate, recorded terminal (key exported earlier, off camera).
6. Keep the raw recording and the printed request id; add them to the evidence table in
   `verification.md`.
