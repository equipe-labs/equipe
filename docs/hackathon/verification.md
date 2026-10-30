# Verification

Two kinds of evidence, kept apart:

1. **Mock tests** (done): the client and the daemon against a **local mock server** that
   plays Token Factory with canned answers. They prove equipe's own behaviour — request
   shape, retries, errors, parsing, redaction, persistence, events. They do **not** prove
   that Nebius Token Factory or any NVIDIA model answered.
2. **A real end-to-end run** (**done, 2026-10-30 16:15 UTC**): the owner's key, the real
   Token Factory, a real NVIDIA Nemotron model — both a review and a member's coding turn.
   See [Real runs](#real-runs-2026-10-30) right below.

## Real runs (2026-10-30)

Run on the owner's Mac with the owner's Token Factory key (read from a local file into the
environment; never printed, logged or committed), equipe **0.90.0** (`e1c864b`), codex-cli
**0.154.0**, endpoint `https://api.tokenfactory.nebius.com/v1` asked in Chat Completions.

**1. Models (`equipe review --models`, `GET /v1/models`)** — the account's Nemotron models:
`nvidia/Nemotron-3.5-Super-VL`, `nvidia/nemotron-3-super-120b-a12b`,
`nvidia/Nemotron-3-Ultra-550b-a55b`, `nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B`,
`nvidia/Nemotron-3_5-Lightning`; equipe picked `nvidia/Nemotron-3.5-Super-VL`.

**2. A review (`equipe review`, `POST /v1/chat/completions`)** — a throwaway repository whose
change makes `check_token` return `{"id": "anonymous", "admin": True}` when the lookup
raises. 16:15:32 UTC. Verdict **needs attention / risky**: "an exception in lookup returns an
anonymous admin user: authentication bypass and privilege escalation", risk at `auth.py:5`,
three missing tests (the exception path, `None`, a valid token). **534 prompt + 848
completion tokens, 3.4 s, request id `773218c7f601b833c5c62f27d525f002`.** SUCCESS.

**3. A member's coding turn on Nemotron** (`crates/providers/tests/real_token_factory.rs`,
run by hand with `-- --ignored --nocapture`): the person's installed `codex app-server` in an
empty `CODEX_HOME` (no ChatGPT sign-in), driven by `codex_app::run_turn` — the daemon's own
path — with Token Factory as its provider behind equipe's relay. Task: "create hello.py that
prints `Hello from Nemotron`, then run it". Model `nvidia/Nemotron-3.5-Super-VL`.

| | |
|---|---|
| What the member did | one `Bash` step: `echo 'print("Hello from Nemotron")' > hello.py && python3 hello.py` → exit 0; final answer "Created `hello.py` and ran it — it prints `Hello from Nemotron`." |
| Checked after | `hello.py` exists; `python3 hello.py` prints `Hello from Nemotron` |
| Requests | 2, both `200` (`POST /v1/chat/completions`, streamed) |
| Request ids | `15eab69a82d6f65f0efb5d589197a28a`, `e4c1f0651d131607313e729857b1ed67` |
| Tokens | in 9,820 (cached 4,352), out 196 |
| Latency | 3,159 ms on requests; 3,514 ms wall |
| Outcome | **SUCCESS** — turn `completed` |

**4. The whole squad through the daemon** (16:19–16:24 UTC): an isolated equipe 0.90.0 daemon
(its own `HOME`, no Claude Code on `PATH`, Codex signed out in an empty `CODEX_HOME`, only
`NEBIUS_API_KEY`), so `equipe status` showed **nemotron** as the only engine ready. A PM and a
backend member started, both on `nvidia/Nemotron-3.5-Super-VL` through Token Factory ("Runs on
NVIDIA Nemotron … through Nebius Token Factory, on the Codex CLI's agent"). The person asked
the PM for one small ticket.

| Step | What happened (from the daemon's events and log) |
|---|---|
| PM turn | Read the board and the repository with its board tools and `Bash`, then `create_ticket` **T-1 "Add hello.py and test_hello.py to the demo repo"**, assigned to backend. 6 requests, all `200`; 14,232 ms on requests; tokens in 41,649 (cached 102,272) out 2,264 |
| Backend turn | Took T-1 on its own branch `equipe/backend-t1-6ae4c3b0` in its worktree; wrote `hello.py` and `test_hello.py`; ran `python3 hello.py` and `python3 -m unittest test_hello.py`; asked **the person** to allow `git commit` (outside the sandbox) — approved by the person — committed `dff749c` "Add hello.py and test_hello.py"; handed T-1 in (`submit_for_review`). 11 requests, all `200`; 21,673 ms on requests; tokens in 57,851 (cached 182,784) out 2,628 |
| Nemotron review (automatic, on hand-in) | **looks good** — "The diff adds hello.py with the required hello() function and script behavior, plus test_hello.py using stdlib unittest that asserts the return value. Both acceptance criteria are covered by the change itself." Request `b94bbf1318e14e2373b696e3b2bcf08f`, 788 prompt + 718 completion tokens, 3,469 ms |
| PM after the hand-in | Ran the handed-in code in the member's worktree to check it |

Outcome: **SUCCESS** — PM → ticket → member on its branch → tests → the person's approval →
hand-in → Nemotron review, all on Token Factory, no other engine. Cost in the Token Factory
console: `[OWNER INPUT REQUIRED]` (not read by us).

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

## 3. Members on Nemotron (the whole squad on Token Factory)

### 3.1 Mock-verified (done) — the real `codex`, a stand-in Token Factory

These run the **real installed Codex CLI** (`codex-cli 0.154.0` on the build machine) through
equipe's own code against a **stand-in Responses API on localhost** with the dummy key
`test-not-a-key`. They are skipped automatically where `codex` isn't installed
(`EQUIPE_CODEX` names another binary). They prove the plumbing — provider config, the
relay, the key's path, the request shape, tool calls and their outputs, the engine choice —
**not** that Token Factory accepts these requests or that a Nemotron model drives the tools well.

```bash
cargo test -p equipe-providers --test nebius_codex -- --nocapture   # prints what codex sent
cargo test -p equipe-core --test nebius -- --nocapture
cargo test -p equipe-providers relay
cargo test -p equipe-core --lib nemotron
cargo test -p equipe-core --lib token_factory
```

| Test | Checks |
|---|---|
| `the_real_codex_runs_a_members_turn_on_a_stand_in_token_factory` | The stand-in answers with a `function_call` to `exec_command`; the command runs in the turn's folder (the file is there); the turn completes with the stand-in's final message; every request is `POST /v1/responses`, `Authorization: Bearer test-not-a-key`, `model: nvidia/nemotron-mock-super`, `stream: true`, function tools only; the second request carries `function_call` + `function_call_output` with the same `call_id`; the key is **not** visible to the command (`$NEBIUS_API_KEY` empty in it); the relay kept each request's status and `x-request-id`. |
| `an_mcp_servers_tools_reach_the_provider_as_function_tools_and_its_calls_reach_the_server` | An MCP server (a stand-in for equipe's board) — Codex sends its tools in a `namespace`; the stand-in sees them as the function tool `mcp__board__list_tickets`; its call reaches the MCP server as `tools/call list_tickets`; the server's answer goes back to the model; the tools Token Factory can't serve stay off. |
| `the_real_codex_runs_a_members_turn_on_any_chat_completions_server` | The model swapped (`Qwen/Qwen3-Coder-480B-A35B-Instruct`, provider `custom`, key in `EQUIPE_MODEL_API_KEY`) on a stand-in that speaks **only** Chat Completions and streams its answer in chunks (the call's arguments split across two): every request reaching it is `POST /v1/chat/completions` — none in Responses — with a system message first, the tools as `{type: function, function: {name…}}` and the key's header; the command runs; the second request carries the `tool` message for `call_mock_1`; the turn completes with the final message, assembled from its chunks. |
| `a_refused_key_ends_the_turn_and_says_so` | A 401 from the provider: Codex tries 6 times in ≈6 s, then the turn fails with `unexpected status 401 Unauthorized … request id: …` (never the key). |
| `a_providers_args_point_codex_at_it_and_keep_the_requests_plain` | The exact `-c` overrides. |
| `with_nothing_signed_in_a_token_factory_key_starts_members_on_nemotron` (core) | No Claude Code, Codex signed out, key set: a member starts on Codex with the `nvidia/…` model, says so; its turn's app-server gets the provider overrides and the key in its environment, never on its command line; no plan usage shown. |
| `with_nothing_signed_in_a_member_named_for_an_engine_starts_on_nemotron_too` (core) | Same, for a member named for Claude or Codex. |
| `with_another_engine_signed_in_a_signed_out_codex_still_says_how_to_sign_in` (core) | Claude Code signed in: nothing moves to Nemotron unless its model is `nvidia/…`. |
| `a_signed_in_codex_runs_as_before_with_a_key_present` (core) | A ChatGPT-signed-in Codex: the account's own model, no overrides, plan usage as before. |
| `a_member_moved_to_an_nvidia_model_runs_its_next_turn_on_token_factory` (core) | `/model nvidia/…` → the next turn on Token Factory, an effort it doesn't take left out; back to an OpenAI model → the person's own Codex. |
| `without_a_key_nothing_signed_in_says_so_and_how` (core) | No key: "No engine is ready … or set NEBIUS_API_KEY …". |
| `mock_verified_a_member_with_nothing_signed_in_writes_code_on_nemotron_through_the_real_codex` (core) | The daemon's runtime, the real `codex` in an empty `CODEX_HOME` (signed out), the stand-in: a member writes `made.rs` in its worktree, and on close it lands on the member's branch. |

What Codex 0.154.0 sent the stand-in, through equipe's relay (captured by the first test,
`--nocapture`; the relay keeps Codex's `client_metadata` and `x-codex-*` headers back):

```
POST /v1/responses   accept: text/event-stream   authorization: Bearer test-not-a-key
body keys: include input instructions model parallel_tool_calls
           prompt_cache_key reasoning store stream tool_choice tools
model: nvidia/nemotron-mock-super   stream: true   store: false   tool_choice: auto
parallel_tool_calls: true   reasoning: {summary: auto}   include: [reasoning.encrypted_content]
tools: function exec_command, function write_stdin, function request_user_input
       (+ function mcp__<server>__<tool> for each MCP tool, flattened by equipe's relay)
input, 1st request: message(developer) message(user) message(user)
input, 2nd request: … function_call(call_mock_1) function_call_output(call_mock_1)
answers used: SSE response.created → response.output_item.added/done {function_call | message}
              → response.completed {usage}
```

Results on `nebius-engine` (2026-10-30, macOS, codex-cli 0.154.0 installed): `cargo clippy
--workspace --all-targets -- -D warnings` clean; `cargo test --workspace --no-fail-fast`: 1,599
passed, 0 failed — the tests above included, with the real `codex`. Mock only.

### 3.2 The owner's real run (not done)

Needs: a Token Factory key, a **budget decision** (an agent turn is many requests; none is
approved yet), the codex CLI (`npm install -g @openai/codex` or `brew install codex`; no
`codex login`), and a build of this branch. Claude Code and Codex may stay signed out — that
is the point. The key never goes on a command line or in a file.

```bash
cd <checkout of nebius-engine> && cargo build --release -p equipe
EQUIPE="$PWD/target/release/equipe"; git rev-parse HEAD; "$EQUIPE" --version; codex --version
read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY
"$EQUIPE" review --models                      # pick a model; optional: export EQUIPE_NEBIUS_MODEL=<it>
"$EQUIPE" status                               # the "nemotron" section says ready
"$EQUIPE" daemon stop
"$EQUIPE" daemon run 2>&1 | tee /tmp/equipe-nemotron-e2e.log   # this shell has the key

# another terminal: the same key exported, then a throwaway repository
read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY
tmp=$(mktemp -d) && cd "$tmp" && git init -q && printf '# demo\n' > README.md && git add . && git -c user.name=e2e -c user.email=e2e@example.com commit -qm init
"$EQUIPE"                                       # first run: a squad here; members join on Nemotron
# ask the PM for one small code ticket, e.g. "Add a `hello` CLI in Rust that prints Hello and a test for it"
# watch: members say "Runs on NVIDIA Nemotron (…) through Nebius Token Factory"; Bash steps run cargo;
# the ticket is handed in; the Nemotron review appears on it.
grep 'turn on Nemotron via Nebius Token Factory' /tmp/equipe-nemotron-e2e.log
```

Each member turn logs one line: `equipe: turn on Nemotron via Nebius Token Factory: model …,
N request(s) [statuses], request ids […], … ms on requests, tokens in … (cached …) out …,
completed` — never the key or the text. Stop with Esc / `equipe daemon stop` when done;
check the spend on the Token Factory console.

### Evidence template (real member run)

Secrets excluded: never record the key or an `Authorization` header.

| Field | Value |
|---|---|
| Date and time (UTC) | `____` |
| equipe commit / version | `____` |
| codex CLI version (`codex --version`) | `____` |
| Engines signed in during the run (expected: none) | `____` |
| Endpoint | `https://api.tokenfactory.nebius.com/v1/chat/completions` (default since 0.90.0; `…/v1/responses` with `EQUIPE_NEBIUS_WIRE=responses`; or `____` if overridden) |
| Model id (as the member's notice and the log say) | `____` |
| Request ids (from the log line) | `____` |
| Requests / statuses for one member turn | `____` |
| Tokens for that turn (in / cached / out) | `____` |
| Latency (ms on requests; the turn's wall time) | `____` |
| Outcome | `SUCCESS / FAILURE` — `____` (what the member did: files changed, commands and tests run, handed in or not; or the error shown) |
| Ticket handed in, and its Nemotron review verdict | `____` |
| Cost shown in the Token Factory console | `____` |
| Screenshot / recording (key not visible) | `____` |
| Recorded by | `____` |
