# Submission checklist — Nebius x NVIDIA Global AI Hackathon

Deadline: **2026-10-30 10:00 America/Los_Angeles** (Submit on Devpost). Devpost draft (do not make
a new one): <https://devpost.com/submit-to/30790-nebius-x-nvidia-global-ai-hackathon/manage/submissions/1179199-equipe/project_details/edit>

Status words: **PASS** (met, with evidence) · **FAIL** (not met yet) · **UNKNOWN** (can't be told
from here, or depends on an owner action). Last updated from the `nemotron-review` branch.

## Hard requirements

| # | Requirement | Status | Evidence | Owner action |
|---|---|---|---|---|
| 1 | A runtime call to the **Nebius Token Factory** inference API | **UNKNOWN** — implemented, not verified | `crates/core/src/nemotron.rs` (`POST /v1/chat/completions`, `GET /v1/models` at `https://api.tokenfactory.nebius.com/v1/`); daemon hook in `runtime.rs`; `equipe review`. Mock tests only. | Do the real run in `verification.md` and fill its evidence table. |
| 2 | Uses at least one **NVIDIA open-source model** | **UNKNOWN** | Model picking accepts only `nvidia/…nemotron…` chat models and errors otherwise (`pick_model`, tested). The exact model used isn't known until the real run. | Record the model id from the real run; confirm it's an NVIDIA open model as the rules define it. |
| 3 | NVIDIA model called **through Nebius**, not another provider | **PASS** (by design) | The only default endpoint is Token Factory's; `EQUIPE_NEBIUS_BASE_URL` exists for tests and the judge relay, which itself forwards to Token Factory. | — |
| 4 | New project, created in the submission period | **PASS** (owner to double-check the period's start date) | First commit `0fa4b4a` on 2026-09-09 (`git log --reverse`). | Confirm 2026-09-09 is inside the period. |
| 5 | Public code repository | **FAIL** for the feature, until pushed | <https://github.com/equipe-labs/equipe> is public, but this work is only on the local branch `nemotron-review` (not pushed — pushing is the owner's call). | Review, merge to `main` and push (from a separate worktree, not the shared checkout). |
| 6 | Open-source license detected by GitHub (About sidebar) | **UNKNOWN** | `LICENSE` (MIT) at the root; `Cargo.toml` `license = "MIT"`. | Check that GitHub's About shows "MIT license". |
| 7 | Materials in English | **PASS** for drafts | README section, `docs/hackathon/*.md`, `demo.en.srt`, Devpost drafts are English. | Final pass on the Devpost text. |
| 8 | Public YouTube video, **under 3 minutes** | **FAIL** | Script and subtitles ready: `demo-script.md`, `demo.en.srt` (≈2:48). | Decide who records/uploads; record with real calls; upload **public**; paste the URL. |
| 9 | Working demo / test build URL | **FAIL** | No release contains `equipe review` yet. Proposal: a GitHub Release (CLI for macOS/Linux via `install.sh`, desktop `equipe.dmg`) + testing instructions. | Decide; cut the release (the release workflow signs/notarizes the dmg). |
| 10 | Judges can test free and without restriction until **2026-12-23 12:00 PT** | **UNKNOWN — unresolved** | `judge-paths.md`, `judge-access.md`. Bring-your-own-key is **not confirmed acceptable**. A relay is implemented but not deployed. | Choose an option; if BYOK, get the organizers' written confirmation; if the relay, deploy with an end date past 12/23 and a budget. |
| 11 | Testing instructions for judges | **PASS** (draft) / UNKNOWN (access) | `devpost-answers.md` › Testing instructions; `judge-paths.md`. | Fill in the release version and the access method. |
| 12 | Eligibility: entrant, residency, team | **UNKNOWN** (owner facts) | Owner reports: individual, sole entrant Abel Ko; residence South Korea; age of majority and no conflict with Sponsor/Administrator: yes. | Confirm South Korea is an eligible country under the Official Rules. |
| 13 | Track selected | **PASS** (owner) | Coding and Agentic Engineering. | Select it on the form. |
| 14 | Builders & Brews question | **PASS** (owner statement) | Owner attended in person in Seoul. | Answer per the Official Rules/FAQ. |
| 15 | AI tools disclosed | **PASS** (draft) | `devpost-answers.md` Q20. | Review the wording. |
| 16 | Publicity / license-to-use terms reviewed | **UNKNOWN** | — | Read the terms; giving answers isn't accepting them. |
| 17 | All Devpost steps complete (5/5) | **FAIL** | Draft is at 2/5, video empty. | Fill the steps; ratings Q9/Q13/Q14 and learning level (Q21) are still undecided. |
| 18 | **Submit** before the deadline | **FAIL** (not yet) | — | Owner only. |

## Technical checks on this branch

| Check | Status | How |
|---|---|---|
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS | run on this branch |
| `cargo test --workspace --no-fail-fast` | PASS (mock only) | run on this branch; see `verification.md` |
| No secret in the repository | PASS | key read from the environment only; `.env.example` has empty values; `.env` is git-ignored |
| Key never in logs / errors / `Debug` | PASS (tested with mocks) | `no_key_is_a_missing_key_and_the_key_never_shows_in_debug`, `a_refused_key_is_unauthorized_…`, `a_review_that_cant_run_says_why_and_never_the_key`, relay `settings_need_a_key_…` |
| Real NVIDIA-on-Nebius run | **NOT DONE** | owner, with a key: `verification.md` §2 |

## What can be done now vs. what needs the owner

Done (no key, no spending, nothing deployed or uploaded):
- Nemotron review in the daemon, `equipe review`, desktop peek, 8 languages, the judge relay code,
  mock tests, docs, Devpost drafts, video script and subtitles.

Can be done without new decisions, by anyone with repository access:
- Review the branch; merge it to `main` from a separate worktree.

Needs the owner's decision or action:
- **Key**: create (or not) a Token Factory key; which account.
- **Budget**: none approved. Any real call spends credits; the account may be charged when its
  balance goes negative; promo credits are not a cap.
- **Real E2E run** and filling the evidence table (`verification.md`).
- **Push / release**: push the branch, cut a release that contains `equipe review`.
- **Judge access**: BYOK (with organizers' written confirmation), relay deployment (host, domain,
  caps, end date, budget), or another arrangement.
- **Video**: who records and uploads; make it public; paste the URL.
- **Devpost**: ratings (Q9, Q13, Q14), learning level (Q21), the unknown questions' texts,
  publicity terms, and the final **Submit**.
