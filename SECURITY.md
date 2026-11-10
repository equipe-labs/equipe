# Security policy

## Reporting a vulnerability

Please **don't** report security issues in public GitHub issues, discussions or pull requests.

Report them privately through GitHub instead:

1. Go to the repository's [**Security** tab](https://github.com/equipe-labs/equipe/security).
2. Choose **Report a vulnerability**.
3. Describe the issue, the version (`equipe --version`), your platform, and how to reproduce
   it. A proof of concept helps.

Only the maintainers can see the report. We'll acknowledge it, keep you updated while we work
on a fix, and credit you in the advisory when it's published, unless you'd rather stay
anonymous.

## Supported versions

equipe ships often. Security fixes go into the latest release; please update
(`equipe update`, or the desktop app's own updater) before reporting, and check the issue still
happens there.

## What's in scope

Especially welcome:

- Ways a member (an AI agent) could act as the person: approve its own requests, accept or
  merge work, change settings, or escape its sandbox or permission mode.
- Ways data could leave the machine that the person didn't turn on, or credentials could be
  read, stored or forwarded.
- Problems in the phone relay's end-to-end sealing or pairing, or in the install script's
  checks.

How equipe is meant to keep these safe is described in the README's
[Safety and privacy](README.md#safety-and-privacy) section.

Issues in Claude Code, Codex or other tools equipe drives should go to their makers; tell us
too if equipe makes them worse.
