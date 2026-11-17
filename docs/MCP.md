# Outside MCP servers

A squad can connect outside MCP servers for its members: a design library's
official server from the Connections catalog (Mobbin, 21st.dev, Figma,
Framer), or any server by its address. The person connects a server once, and
it keeps working for every member, on every engine, from then on. This page
covers how that works and what equipe does to keep it working.

Code: `crates/core/src/squad_mcp.rs` (settings, the proxy entry, probing),
`crates/core/src/runtime/squad_mcp.rs` (status, cards, health checks),
`crates/core/src/mcp_oauth.rs` (equipe as the OAuth client),
`crates/core/src/mcp_gateway.rs` (the proxy), `crates/core/src/mcp_keys.rs`
(where credentials are kept), `crates/core/src/engine_bind.rs` (which member
a connection belongs to).

## How a member reaches a server

Each member's engine (Claude Code or Codex) gets a stdio entry for equipe's
proxy, `equipe mcp proxy --squad … --server … --member … --home …`. The entry
holds no credential. Every message goes to the daemon, which forwards it to
the server and adds the credential itself: the OAuth access token, or the
API key. No token, key or header ever appears in a member's config,
environment, files or process.

The daemon only serves a proxy for the member whose turn runs the engine the
proxy runs under. When a turn's engine starts, the turn notes its process id
right away (`engine_bind`), without going through the daemon's main loop. A
connection that arrives while the loop is busy (a long merge, say) is still
recognized. This covers a member's first turn in a fresh conversation (after
rotation or compaction), a new run lane, and an engine move between Claude
and Codex: each starts a new engine, and each is bound the moment it starts.

## Where credentials are kept

OAuth tokens and keyed servers' API keys live in equipe's own file,
`~/.equipe/credentials/mcp-keys.sealed`, sealed with ChaCha20-Poly1305 under
a random key in `~/.equipe/credentials/mcp-keys.key`. Both files are 0600 in
a 0700 folder, and `credentials/` is on the list of paths no member's fenced
command may read.

Why not the Keychain: the daemon reads these credentials unattended, at any
hour, often on a Mac nobody is sitting at. macOS shows a Keychain dialog
whenever the reading program's identity isn't one the item trusts, for
example after an update that shipped ad hoc. An unanswered dialog left every
outside server failing until someone came to that Mac.

Threat model: the file protects against other accounts on the Mac, against
the file being copied or backed up without its key, and against members'
commands. It doesn't protect against a program running as the person outside
equipe's fences. That is the same protection Claude Code's and Codex's own
sign-in files get. Logins and the keys members save for their own work
(`squad_secrets`) stay in the Keychain.

Credentials an earlier equipe kept in the Keychain move into the file the
first time the daemon reads them with the Keychain open. That happens when
the person saves a key or signs in again, or answers the Keychain dialog
once. After that they are read from the file, whatever the Keychain does.

## Staying connected

- **Health check.** Every 10 minutes, and once right after the daemon
  starts, the daemon checks every server through its own connection.
- **Token refresh.** The same check renews any OAuth token that runs out
  within 20 minutes, using the refresh token. A member's call never meets an
  expired token. If a server refuses a token early anyway, the proxy renews
  it once and retries the call.
- **When a refresh fails.** A busy or unreachable server (429, 5xx, network)
  counts as "failing for now": it is checked again later, and the person
  isn't asked. Only a refresh token the server rejects (400/401/403) ends the
  sign-in.
- **When a server stops working.** If a server that was connected stops
  working and equipe can't repair it itself, one card goes to the person
  saying what happened and the one thing to do. Members are told the server
  is unavailable and why.

## Sign-in cards

- A card waits for the person for a day. A remote person who checks in hours
  later still finds it. It shows up under **Your turn** (Needs you), in
  Connections, and docked in the conversation of the member who needs it.
  That member's conversation also gets a note saying the card is there.
- A card the person puts off ("Not now"), or that runs out, isn't put up
  again by members' turns. The exception: the next time a member needs that
  server, equipe offers it **once more**, with the reason ("designer needs
  Mobbin again for T-…"). After that, only the person brings it back.
- **Sign in** in Connections, or `equipe mcp login <name>`, always clears a
  put-off and starts a fresh sign-in.
- The sign-in page redirects to the daemon's own Mac. With the daemon on
  another Mac, the app forwards that port over SSH. Otherwise, paste the
  address the page ended on into the card.
- `equipe mcp login <name>` run over SSH prints the page instead of opening
  it on the daemon's Mac, and accepts the pasted address in the terminal.

## What a member is told

A member's instructions list the outside servers it has. Servers it would
have but doesn't get this turn are listed too, each with the reason ("equipe
isn't signed in to it; the person has a card for it in Needs you", "its API
key isn't available to equipe (…)"). The member is told not to look for
another way in, and not to guess at the cause.

## What the person does once

- **An OAuth server (Mobbin, Figma, …)**: sign in once on its card.
- **A keyed server (21st.dev)**: save its API key once on its key card.
- **After moving from an older equipe**: if a credential is still behind a
  Keychain dialog on the squad's Mac, Connections says so. Either sign in or
  enter the key again from the app (it goes straight to equipe's own file),
  or approve the dialog on that Mac's screen once. Either way, it isn't
  asked again.
