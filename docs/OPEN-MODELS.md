# Open models: NVIDIA Nemotron, Nebius Token Factory and OpenAI-compatible servers

equipe's members usually run on Claude Code or Codex with your own plan. This page covers
the other way: members on open models through the Codex CLI's agent loop, billed per token
to a key of your own — and the Nemotron code review. Back to the [README](../README.md).

## Run the whole squad on Nebius Token Factory

**What.** Every member — the PM, the engineers, QA — can run on an **NVIDIA Nemotron**
model on **Nebius Token Factory**, with no Claude Code and no ChatGPT plan. The agent loop
is the **Codex CLI**'s own (free and open source; *no* `codex login` is needed for this):
Nemotron decides, and Codex's tools read files, run commands and tests, and change files,
in the member's worktree and sandbox, as for any Codex member. equipe's board tools (take a
ticket, hand it in, ask the person) reach the model like Codex's own.

**The key, the easy way:** in the desktop app, **Settings → AI platforms → Nebius Token
Factory**, paste the key and press **Connect** — or run `equipe login nebius` in a terminal.
Either saves it on the daemon's machine (`~/.equipe/keys/nebius`, readable only by you); the
daemon uses it from then on, no restart and no shell setup needed. The card shows where the key
comes from and removes it again. Claude Code and Codex connect from the same page.

Or from a shell, for this daemon only:

```bash
curl -fsSL https://chatgpt.com/codex/install.sh | sh  # Codex CLI, no Node needed (or: brew install codex)
read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY    # paste the key; it isn't echoed or kept in history
export EQUIPE_NEBIUS_MODEL=nvidia/…                 # optional: a model your account offers (see `equipe review --models`)
equipe daemon stop; equipe daemon run               # from this shell, so the daemon has the key; Ctrl-C stops it
```

Then, in another terminal (with `NEBIUS_API_KEY` exported there too, so the first-run
setup sees it), run `equipe` as usual — or open the desktop app. `equipe status` has a
`nemotron` section that says whether the key and the codex CLI are there.

**Which members go on it.**
- **Nothing signed in** (no Claude Code, Codex not logged in) and a key for the daemon:
  every member starts on Nemotron — `EQUIPE_NEBIUS_MODEL`, else the account's best
  Nemotron chat model (Super, then Ultra, then Nano, from `GET /models`). Each says so
  when it joins, and its model shows as `nvidia/…` wherever models are shown.
- **Claude Code or Codex signed in:** nothing changes. Move a Codex member over with
  `/model <member> nvidia/…`, and back with an OpenAI model id.

### Swap the model any time

The agent (Codex's loop, its sandbox, equipe's board tools) stays; the **model** is
whatever you point it at. Any id with an organization or a tag — `nvidia/…`, `Qwen/…`,
`deepseek-ai/…`, `meta-llama/…`, `qwen3-coder:30b` — runs on the open provider; OpenAI
and Anthropic ids (no `/`) stay on their own plans.

- **Another model on Token Factory:** `/model <member> Qwen/Qwen3-Coder-480B-A35B-Instruct`
  (or any id `equipe review --models` / your account lists), from the next turn on; or
  `EQUIPE_NEBIUS_MODEL=…` for every member that starts.
- **Another server entirely** — anything that speaks OpenAI's Chat Completions API
  (OpenRouter, Together, Groq, Fireworks, vLLM, Ollama, LM Studio…). The desktop app has
  these as presets under **Settings → AI platforms**; from a shell:

  ```bash
  export EQUIPE_MODEL_BASE_URL=https://openrouter.ai/api/v1   # or http://localhost:11434/v1 for Ollama
  read -rs EQUIPE_MODEL_API_KEY && export EQUIPE_MODEL_API_KEY  # a server on this machine needs none
  export EQUIPE_MODEL=qwen/qwen3-coder                          # optional: else a Nemotron, or the first listed
  equipe daemon stop; equipe daemon run
  ```

  Set, `EQUIPE_MODEL_BASE_URL` is used in place of Token Factory — for members' turns and
  for reviews alike.

**How.** Codex asks its provider in OpenAI's Responses API only. equipe's relay on the
loopback (`crates/providers/src/relay.rs`, `crates/providers/src/chat_wire.rs`) says each
request in **Chat Completions** — the API nearly every server speaks — and the server's
streamed answer back in Responses events, tool calls included. `EQUIPE_MODEL_WIRE=responses`
(or `EQUIPE_NEBIUS_WIRE`) sends Responses requests as they are, for a server that serves them.

For a member whose model is `nvidia/…` (or any open model, above), equipe runs
`codex app-server` with Token Factory as Codex's model provider, pointed at equipe's relay.
What such a provider can't serve is turned off for these turns (web search, image tools,
subagents, OpenAI's remote compaction). Codex's own metadata for OpenAI (`client_metadata`,
`x-codex-*` headers — the workspace's paths among it) stays on your machine. The key goes to
the app-server in its environment only — never on a command line — Codex reads it there, the
agent's own commands don't see it, and the relay passes the request's header on without
reading, keeping or logging it. Code: `crates/core/src/nemotron.rs` (members on Token
Factory), `crates/providers/src/codex_app.rs` (`Provider`, `provider_args`). Each member turn
leaves one line in the daemon's log — model, request count and statuses, request ids, time
and tokens — never the key or the text.

**What is verified.** The tests run the real installed `codex` through equipe against a
stand-in Chat Completions server and a stand-in Responses API on localhost with a dummy key
(`crates/providers/tests/nebius_codex.rs`, `crates/core/tests/nebius.rs`; skipped without
`codex`): a function call runs a shell command in the member's worktree and the file lands on
the member's branch; an MCP tool call reaches the MCP server and its answer goes back to the
model; a refused key (401) ends the turn with *"Nebius Token Factory refused the key"*. A run
against the real Token Factory is recorded in
[hackathon/verification.md](hackathon/verification.md#real-runs-2026-10-30).

**Limits.**
- Files change through shell commands (`exec_command`): Codex offers its `apply_patch` tool
  only in a form a function-tool provider can't serve. Edits show as `Bash` steps, not as
  diffs of their own (the branch's diff is still reviewed as usual).
- No web search, image input or subagents on these turns. No plan usage to show — Token
  Factory bills per token. Effort levels Token Factory doesn't take (`max`, `ultra`) are
  left out.
- For these turns, `shell_environment_policy.exclude` is `["NEBIUS_API_KEY"]` in place of
  any list of your own in `~/.codex/config.toml`.
- A refused key is retried by Codex a few times (seconds) before the turn fails.

**Cost — no measured numbers.** An agent turn is not one request: every tool call is
another round trip that resends the conversation, so one ticket can take many requests and
hundreds of thousands of input tokens on a long task. Watch your provider's usage page while
trying it, set a spending limit there, and stop a member with Esc. equipe shows each turn's
tokens as Codex reports them.

## Nemotron review on Nebius Token Factory

**What.** When a member hands in a code ticket, an **NVIDIA Nemotron** model on
**Nebius Token Factory** reads the branch's diff against the ticket and writes a
short review: a verdict (*looks good* · *needs attention* · *risky*), a summary,
up to five risks as `file:line — why`, and up to five tests the change is missing.
It shows in the desktop app's ticket peek, under the hand-in, and is kept on the
ticket's record. It is **advice only**: it never accepts, merges or sends work back.

**Where the call happens.** `crates/core/src/nemotron.rs` is the whole client:
`POST <base>/chat/completions` (and `GET <base>/models` to pick a model), OpenAI-compatible,
`Authorization: Bearer $NEBIUS_API_KEY`. The daemon calls it off its main loop when a
code ticket is handed in, stores the result in the squad's journal (`reviews` table) and
sends `Event::CodeReviewed` to every screen. `equipe review` calls the same client from the
command line.

**Try it with only a Nebius key** — no squad, no Claude or ChatGPT plan needed:

```bash
read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY   # paste the key; it isn't echoed or kept in history
cd your-repo
equipe review --models          # the NVIDIA Nemotron models your account offers
equipe review                   # your uncommitted changes (working tree + staged) against HEAD
equipe review --base main       # what this branch changed since it left main
equipe review --base main --goal "Refuse expired tokens with 401"   # and check it against what it should do
```

New files are reviewed once they're added (`git add -N` is enough). It prints the
verdict, summary, risks, missing tests, the model id, the tokens used and Token
Factory's request id.

Without a key nothing is sent anywhere: the daemon logs once that reviews are off. equipe
brings no key of its own: each person uses their own.

| Variable | |
|---|---|
| `NEBIUS_API_KEY` | Your Token Factory API key — from the environment, or the key saved by `equipe login nebius` or the desktop app (`~/.equipe/keys/nebius`, readable only by you). Never logged or shown (`Debug` redacts it; error messages never include it or the auth header). |
| `EQUIPE_NEBIUS_MODEL` | A model id to use. Unset: equipe lists `GET /models` and picks an NVIDIA Nemotron chat model (Super, then Ultra, then Nano). No such model → a clear error, no fallback to another vendor's. |
| `EQUIPE_NEBIUS_BASE_URL` | Another server with the same API (default `https://api.tokenfactory.nebius.com/v1/`). The tests point it at a local mock. |
| `EQUIPE_NEBIUS_MAX_TOKENS` | The longest answer asked for (default 2048). |

[`.env.example`](../.env.example) lists them; equipe itself doesn't read `.env` files.

**Security notes.**
- The diff and the ticket are sent to Nebius Token Factory. Don't enable it on code
  you may not send to a third party; without a key nothing is sent.
- The prompt tells the model that the diff and ticket are data, never instructions,
  and fences them. What comes back is treated as untrusted: thinking blocks dropped,
  only the first JSON object read, control characters and terminal escape sequences
  removed, every field cut to a short length, at most five risks and five tests.
  An answer that isn't a review is an error — a review is never made up.
- The daemon's log records the model id, Token Factory's request id, token counts and
  latency — never the key, the diff or the review's text.
- Requests time out after 90 s (10 s to connect); a rate limit (429), a server error or
  a timeout is tried again at most twice, with backoff; 400/401/403 are not retried.
