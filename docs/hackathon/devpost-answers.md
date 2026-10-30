# Devpost answers — drafts (English)

Draft for the **Nebius x NVIDIA Global AI Hackathon** submission form. The submission already
exists as a draft — do not create a new one:
<https://devpost.com/submit-to/30790-nebius-x-nvidia-global-ai-hackathon/manage/submissions/1179199-equipe/project_details/edit>
(status when last told: DRAFT, 2 of 5 steps, video empty).

`[OWNER INPUT REQUIRED]` marks every fact, rating or decision only the owner can give. Nothing
here is approval to submit, to accept publicity terms, or to spend credits. **Submit is the
owner's action only.**

---

## Project name

equipe

## Tagline (elevator pitch)

A squad of AI coding agents that hand in work like a team — and an NVIDIA Nemotron model on
Nebius Token Factory that reviews every hand-in before you accept it.

## About the project

### Inspiration

One coding agent is great in a chat window and falls apart on a real project: nobody owns the
spec, work isn't split, and nothing gets reviewed. equipe gives agents what a team has — a PM, a
board of tickets, roles, their own git branches, and a person who accepts the work. What was
missing was a second pair of eyes: the agents that write the code were the only ones reading it.

### What it does

equipe runs a squad of AI agents (PM, frontend, backend, designer, QA, data…) on the user's own
Claude Code or Codex. The PM turns requests into tickets; each engineer works its ticket on an
isolated branch and **hands it in**. For this hackathon we added a reviewer with a narrow job:

- When a code ticket is handed in, equipe sends the branch's diff and the ticket's acceptance
  criteria to an **NVIDIA Nemotron** model on **Nebius Token Factory**.
- The model answers with a verdict (*looks good*, *needs attention*, *risky*), a short summary,
  up to five risks as `file:line — why`, and tests the change is missing — focused on correctness
  bugs, security, data loss and criteria the diff doesn't meet.
- The review appears in the desktop app under the hand-in and on the ticket's record, in the
  person's language (8 languages). It is advice: the person still accepts or sends the work back.
- `equipe review` runs the same review on any git repository from the command line with **only a
  Nebius key** — no agents, no subscriptions.

### How we built it

- Rust workspace: a daemon (`crates/core`) that runs the squad, a CLI/terminal UI (`crates/cli`),
  a macOS desktop app in GPUI (`crates/desktop`), and shared protocol types (`crates/protocol`).
- `crates/core/src/nemotron.rs` is a small OpenAI-compatible client for Token Factory
  (`reqwest` + `rustls` + `serde_json`): it lists `GET /v1/models` to pick an `nvidia/…nemotron…`
  chat model (or uses `EQUIPE_NEBIUS_MODEL`), calls `POST /v1/chat/completions` with a reviewer
  system prompt, retries rate limits / server errors / timeouts at most twice, and parses the
  answer defensively.
- The daemon runs the review off its main loop when a ticket is handed in, stores it in the
  squad's SQLite journal and pushes a `CodeReviewed` event to every screen.
- Tests run against a local mock Token Factory, so CI needs no key and no network.

### Challenges we ran into

- **Prompt injection both ways.** A diff can contain text aimed at the reviewer, and the answer
  can contain anything. The prompt fences the diff and ticket as data; the parser accepts one
  bounded JSON shape, drops reasoning blocks, strips control characters and terminal escapes,
  and caps every field. An answer that isn't a review is an error — never a made-up review.
- **Reasoning models.** Some Nemotron variants think out loud; the client tolerates `<think>`
  blocks and prose around the JSON.
- **Never blocking the squad.** Git and the network run off the daemon's core loop, with
  timeouts and bounded retries.
- **Secrets.** The key is read from the environment only; it is redacted from debug output and
  scrubbed from error messages, and the log carries only the model id, request id, token counts
  and latency.

### Accomplishments that we're proud of

- The review fits the existing flow: no new screen to learn, it is just there under the hand-in.
- Judges (or anyone) can try the NVIDIA-on-Nebius part with a single command and only a key.
- Every word on screen exists in 8 languages.

### What we learned

[OWNER INPUT REQUIRED — in your own words; a draft:] Small, single-purpose model calls with a
strict output contract are easier to trust than a general assistant, and treating model output
as untrusted input matters as much for a reviewer as for an agent.

### What's next for equipe

- Let the PM read Nemotron's review before it accepts work on the person's behalf.
- Show reviews in the terminal console and on the phone app.
- Measure review quality and cost on real tickets (not done yet).

## Built with

Rust · Tokio · reqwest · rustls · serde / serde_json · SQLite (rusqlite) · git · GPUI ·
**Nebius Token Factory** · **NVIDIA Nemotron** · Claude Code · OpenAI Codex · TypeScript ·
Cloudflare Workers (the phone relay)

## Links

- Code: <https://github.com/equipe-labs/equipe> (public, MIT)
- Video (YouTube, public, under 3 minutes): `[OWNER INPUT REQUIRED — URL after upload]`
- Demo / test build: `[OWNER INPUT REQUIRED]` — proposal: the GitHub Release that contains
  `equipe review` (CLI for macOS/Linux, desktop `equipe.dmg`), plus the instructions in
  `docs/hackathon/judge-paths.md`. A release has not been cut yet.
- Testing instructions: see "Testing instructions for judges" below.

## Testing instructions for judges

```bash
curl -fsSL https://github.com/equipe-labs/equipe/releases/latest/download/install.sh | sh
#   (equipe 0.88.0 or later contains `equipe review`)
read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY     # a Nebius Token Factory key — see below
cd any-git-repository-with-changes
equipe review --models                 # NVIDIA Nemotron models on the account
equipe review                          # review uncommitted changes against HEAD
equipe review --base main              # review a branch's changes since main
```

Access to a key: equipe is open source and has no hosted service or shared key — judges use their
own Nebius Token Factory key (hackathon participants can get Token Factory credits with the code
`NEBIUS-DEVPOST-GLOBAL26`, per the Resources page). The key is read from the environment only.

The full squad (agents writing code, then the automatic review of each hand-in) also needs
Claude Code or Codex signed in with the judge's own plan — see `docs/hackathon/judge-paths.md`.

---

## Additional info (Q1–Q21)

The exact wording of each question is on the Devpost form; numbering below follows what the
owner reported. Where the question text wasn't available here it is marked.

| # | Question (as known) | Draft answer |
|---|---|---|
| Q1 | Submitter type | Individual — Abel Ko (sole entrant). |
| Q2 | Organization | N/A |
| Q3 | Country of residence / province | South Korea (Republic of Korea); province: N/A |
| Q4 | Track | Coding and Agentic Engineering |
| Q5 | New or existing project | New project, started within the submission period: first commit 2026-09-09 (`git log --reverse`). |
| Q6 | `[OWNER INPUT REQUIRED: question text]` | `[OWNER INPUT REQUIRED]` — if it asks which Nebius product: Nebius Token Factory (inference API, OpenAI-compatible), called at runtime from `crates/core/src/nemotron.rs`. |
| Q7 | `[OWNER INPUT REQUIRED: question text]` | `[OWNER INPUT REQUIRED]` — if it asks which NVIDIA model: an NVIDIA Nemotron model on Token Factory; exact id `[OWNER INPUT REQUIRED — from the real run, e.g. as printed by equipe review]`. |
| Q8 | `[OWNER INPUT REQUIRED: question text]` | `[OWNER INPUT REQUIRED]` — if it asks how the model is used: it reviews each handed-in code change against its ticket and returns a structured verdict, risks and missing tests (see About). |
| Q9 | Rating `[question text]` | `[OWNER INPUT REQUIRED — undecided; depends on a real run]` |
| Q10 | `[OWNER INPUT REQUIRED: question text]` | `[OWNER INPUT REQUIRED]` |
| Q11 | Comparison with other models/providers | No comparison was done. We did not measure Nemotron against other models, so we make no claim about relative quality, speed or cost. |
| Q12 | `[OWNER INPUT REQUIRED: question text]` | `[OWNER INPUT REQUIRED]` |
| Q13 | Rating `[question text]` | `[OWNER INPUT REQUIRED — undecided; depends on a real run]` |
| Q14 | Rating `[question text]` | `[OWNER INPUT REQUIRED — undecided; depends on a real run]` |
| Q15 | `[OWNER INPUT REQUIRED: question text]` | `[OWNER INPUT REQUIRED]` |
| Q16 | `[OWNER INPUT REQUIRED: question text]` | `[OWNER INPUT REQUIRED]` |
| Q17 | Did you use Tavily? | No |
| Q18 | Builders & Brews | Yes — attended in person, Seoul (as the owner reports; eligibility per the Official Rules/FAQ is the owner's to confirm). |
| Q19 | Age of majority; no relationship/conflict with Sponsor/Administrator | Yes — the owner confirms both. |
| Q20 | AI tools used | Yes. Claude Code (Anthropic) wrote much of the code, including this Nemotron feature, under the owner's direction and review. Claude Code and OpenAI Codex are also the engines equipe orchestrates at runtime (on each user's own plan). NVIDIA Nemotron on Nebius Token Factory is the runtime reviewer this submission adds. |
| Q21 | Career value of AI; learning level | Career value: Yes. Learning level: `[OWNER INPUT REQUIRED — not answered; do not infer]` |

## Not decided — owner only

- Real Token Factory run and its evidence (`docs/hackathon/verification.md`).
- Any budget or credit use (none approved). Token Factory onboarding needs a card and can charge
  when the balance goes negative; promo credits are not a spending cap.
- Judge access (`docs/hackathon/judge-access.md`), release/demo build, video recording, upload
  and making it public, publicity terms, and the final **Submit**.
