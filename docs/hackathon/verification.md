# Verification

Two kinds of evidence, kept apart:

1. **Mock tests** (done): the client and the daemon against a **local mock server** that
   plays Token Factory with canned answers. They prove equipe's own behaviour — request
   shape, retries, errors, parsing, redaction, persistence, events. They do **not** prove
   that Nebius Token Factory or any NVIDIA model answered.
2. **A real end-to-end run** (not done yet): the owner, with their own key, calls the real
   Token Factory with a real NVIDIA Nemotron model. Until that is recorded below, the
   hackathon's "runtime call to Nebius Token Factory with an NVIDIA open model" requirement
   is **implemented but unverified**.

No real Token Factory request was made while building this (no key on the build machine,
no approval to spend credits).

## 1. Mock tests

Run from the repository root (no network, no key, no login):

```bash
cargo test -p equipe-core --lib nemotron                    # the client, against a mock server
cargo test -p equipe-core --test runtime -- nemotron review # the daemon's hook, against a mock server
cargo test -p equipe review::tests                          # how `equipe review` prints a review
cargo test -p equipe-desktop nemotron_reviews               # the desktop app keeps reviews
cargo test --workspace                                      # everything
cargo clippy --workspace --all-targets -- -D warnings
```

What the mock tests cover (`crates/core/src/nemotron.rs` › `tests`, `crates/core/tests/runtime.rs`):

| Test | Checks |
|---|---|
| `a_review_comes_back_cleaned_with_its_model_tokens_and_request_id` | Normal flow: `POST /v1/chat/completions`, bearer auth, diff fenced as data; verdict, summary, risks, missing tests, model id, usage, `x-request-id`. |
| `no_key_is_a_missing_key_and_the_key_never_shows_in_debug` | No/blank `NEBIUS_API_KEY` → `MissingKey`; `Debug` shows `[redacted]`; defaults. |
| `a_refused_key_is_unauthorized_at_once_and_its_body_never_echoes_the_key` | 401 → `Unauthorized`, one request (no retry); 400 not retried; key scrubbed from the error snippet. |
| `a_rate_limit_is_waited_out_and_tried_again` | 429 (with `Retry-After: 0`) then 200 → success after 2 requests; 429 three times → `RateLimited`; 503 then 200 → success. |
| `no_answer_in_time_is_a_timeout_after_trying_again` | A server that never answers → `Timeout` after 3 requests. |
| `an_empty_diff_asks_nothing` | Empty diff → `EmptyDiff`, no request. |
| `an_answer_that_isnt_a_review_is_bad_output` | Empty, prose, cut-off JSON, unknown verdict, empty summary, non-object, non-JSON body → `BadOutput`. |
| `a_reasoning_models_thinking_is_left_out` | `<think>…</think>` (even containing JSON), a dangling `</think>`, prose and code fences around the object. |
| `instructions_and_escapes_in_the_answer_stay_inert_bounded_text` | Injected "ignore previous instructions" / shell commands kept as plain text; ANSI, BEL, NUL, bidi overrides removed; ≤5 risks, ≤5 tests, lengths capped. |
| `a_long_diff_is_cut_and_the_review_says_so` | Diff > 48,000 chars → cut, `truncated: true`. |
| `the_model_is_picked_from_the_accounts_nvidia_nemotron_models` | No NVIDIA model (or only reward/embedding ones) → `NoModel` without a completion call; Super > Ultra > Nano; the picked id is sent. |
| `a_line_is_cleaned_and_cut` | Text cleaning and truncation helpers. |
| `code_handed_in_is_reviewed_by_nemotron_kept_and_shown_on_every_screen` | Daemon: a real git hand-in → diff sent to the mock → `Event::CodeReviewed`, `Snapshot.reviews`, ticket record entry by `nemotron`. |
| `a_review_that_cant_run_says_why_and_never_the_key` | Daemon: mock 401 → localized notice in the member's conversation, no key in any event, no review stored. |
| `nemotron_reviews_come_with_the_snapshot_and_as_they_arrive` (desktop) | The desktop Model keeps reviews from the snapshot and the event, ignores other squads', drops one when its ticket is sent back. |

Results on this branch (2026-10-30, macOS, `nemotron-review`): `cargo clippy --workspace
--all-targets -- -D warnings` clean; `cargo test --workspace --no-fail-fast`: all passed
(see the commit log for the exact commit). These numbers are about the mocks only.

## 2. The owner's real end-to-end run

Needs: a Token Factory API key from the owner's Nebius account, a decision on the budget
(none is approved yet), and a release or source build that contains `equipe review`.
Do **not** paste the key into a command line, a file in the repository, a screenshot or the
video.

```bash
# 0. Build this branch (a checkout of `nemotron-review`, or the release that contains it)
cd <checkout of nemotron-review>
cargo build --release -p equipe
EQUIPE="$PWD/target/release/equipe"; git rev-parse HEAD; "$EQUIPE" --version

# 1. The key, for this shell only (not echoed, not in history)
read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY

# 2. Which NVIDIA Nemotron models the account offers, and the one equipe picks
"$EQUIPE" review --models

# 3. A real review of a small, known change, in a throwaway repository
tmp=$(mktemp -d) && cd "$tmp" && git init -q
printf 'fn main() {}\n' > main.rs && git add . && git -c user.name=e2e -c user.email=e2e@example.com commit -qm init
printf 'fn main() {\n    let path = std::env::args().nth(1).unwrap();\n    std::fs::remove_dir_all(path).unwrap();\n}\n' > main.rs
date -u +%Y-%m-%dT%H:%M:%SZ
"$EQUIPE" review --goal "Delete only the cache folder the user names, after asking them to confirm"

# 4. (Optional, needs Claude Code or Codex) the daemon's automatic review
"$EQUIPE" daemon stop
read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY
"$EQUIPE" daemon run     # then, in another terminal, give the squad a small code ticket
# when it is handed in: ~/.equipe/daemon.log has "Nemotron reviewed T-…", the desktop peek shows it
```

Record the result in the template below — failures too. Nothing is filled in until it has
actually run.

### Evidence template (real run)

Secrets excluded: never record the key, an `Authorization` header, or a full diff of private code.

| Field | Value |
|---|---|
| Date and time (UTC) | `____` |
| equipe commit (`git rev-parse HEAD` of the build) | `____` |
| equipe version (`equipe --version`) | `____` |
| Endpoint | `https://api.tokenfactory.nebius.com/v1/chat/completions` (or `____` if overridden) |
| Models listed by `equipe review --models` | `____` |
| Model id used (as printed: "Nemotron review · …") | `____` |
| Request id (printed "request …", or `____` if none) | `____` |
| Result | `SUCCESS / FAILURE` — `____` (if failure: the printed reason, e.g. "Nebius Token Factory didn't accept the API key") |
| Verdict and summary (as printed) | `____` |
| Input (prompt) tokens | `____` |
| Output (completion) tokens | `____` |
| Latency (printed "…s") | `____` |
| Cost shown in the Token Factory console for this request, if visible | `____` |
| Daemon run (optional): log line `equipe: Nemotron reviewed T-… model …, request …, … tokens, … ms` | `____` |
| Screenshot / recording file (key not visible) | `____` |
| Recorded by | `____` |
