# Judge access

**Decision (owner, 2026-10-30): judges bring their own Nebius Token Factory key.** equipe is open
source and runs on the judge's own machine; there is no hosted equipe service and no shared key.
This is the owner's decision; the organizers have not confirmed it in writing.

## What a judge needs

- A Mac (desktop app and CLI) or Linux (CLI) machine.
- A Nebius Token Factory API key in `NEBIUS_API_KEY`. Hackathon participants can get Token Factory
  credits with the code `NEBIUS-DEVPOST-GLOBAL26` (see the hackathon's Resources page). A review is a
  single chat completion over a diff of at most ~48k characters, with `max_tokens` 2048 by default.
- For the core equipe workflow (a squad of members doing tickets): Claude Code or Codex signed in on
  that machine. `equipe review` needs neither — only the Token Factory key.

## Steps

```bash
curl -fsSL https://github.com/equipe-labs/equipe/releases/latest/download/install.sh | sh
read -rs NEBIUS_API_KEY && export NEBIUS_API_KEY   # typed, not echoed, not saved
equipe review --models                            # the NVIDIA Nemotron models the key can use
cd <any git repository with uncommitted changes>
equipe review                                     # Nemotron's review of `git diff HEAD`
```

The automatic review of handed-in work runs in the daemon; start it from a shell that has the key
(`equipe daemon stop`, then `equipe daemon run`) so it sees `NEBIUS_API_KEY`.

## What stays true

- The key is read from the environment only: never written to disk, logged, or shown.
- Without a key nothing is sent anywhere.
- Every call goes to Token Factory (`https://api.tokenfactory.nebius.com/v1/`) and uses an NVIDIA
  Nemotron model; there is no fallback to another provider.
