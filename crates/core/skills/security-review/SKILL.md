---
name: security-review
description: Use for every security review or check you run — what to look at in code, dependencies and CI, secrets, what is exposed, the AI squad and this machine, and how to report it.
---

# A security review

What you look at, as a security review asks for it:

- Code: authentication and authorization, input handling and injection (SQL, shell, template, path traversal), SSRF,
  unsafe deserialization, cryptography misused (home-made crypto, weak hashes for passwords, keys in code, missing TLS
  checks), and what an error or a log gives away — the OWASP Top 10 and the CWEs behind them. Read recent changes first
  (`git log`, the tickets handed in), then the places that guard money, accounts and data.
- Dependencies and the supply chain: known vulnerabilities with the project's own auditor where it exists (`cargo
  audit`, `npm audit`, `pnpm audit`, `pip-audit`, `osv-scanner`; a one-off `npx`/`uvx` run is fine), new packages and
  where they come from (a lookalike name, a fresh publisher, an install script), lockfiles kept, and CI: workflow
  permissions, `pull_request_target`, third-party actions not pinned, tokens wider than the job needs.
- Secrets: keys, tokens and passwords in the code, in git history, in logs, in `.env` files that are committed, in CI
  settings; a scanner such as `gitleaks` or `trufflehog` if installed, else your own search for their shapes. Never
  show, copy or keep a secret's value anywhere — not in a report, a ticket, a message or a file: name where it is and
  its first four characters at most, and say it must be rotated.
- What is exposed: servers listening on every address rather than localhost, CORS, cookies and headers, TLS, files and
  folders others can read (keys and credentials should be 0600), Docker and cloud settings you can read.
- The AI squad itself: whether a member would follow instructions written in a web page, an issue, a file or a tool's
  output (prompt injection), whether any path lets work skip the person's approval, and whether permission rules and
  tools (MCP servers, browser, shell) are wider than the work needs.
- This machine, read only: FileVault, the firewall, SIP, automatic updates, remote login and sharing services
  (`fdesetup status`, `/usr/libexec/ApplicationFirewall/socketfilterfw --getglobalstate`, `csrutil status`,
  `softwareupdate --schedule`, `systemsetup -getremotelogin` where allowed). Report what you find; changing a setting is
  the person's.

How you report: each finding with its severity (Critical, High, Medium, Low — by what an attacker could actually do,
not by how it looks), where it is (file and line, or the setting), the evidence, what could happen, and the fix,
smallest first. Say what you checked and found clean, and what you could not check, so silence is never read as
safety. Keep each review as `security/<date>.md` in the squad's folder, and say what changed since the last one.
Propose the fixes to the PM as tickets for the engineers, the most severe first; anything Critical — a live secret
exposed, an open hole in production — you tell the PM and the person at once.
