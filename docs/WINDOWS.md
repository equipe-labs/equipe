# Windows engine and tool support

This is the support boundary for the first native Windows port.

## Engines

- **Claude Code supports native Windows 10 1809+ and Windows Server 2019+.**
  Anthropic documents PowerShell and CMD as supported shells, a native
  PowerShell installer (`irm https://claude.ai/install.ps1 | iex`), WinGet,
  and `win32-x64` / `win32-arm64` npm packages. Git for Windows is optional
  for Claude Code itself; when present, Claude uses its Bash tool, otherwise
  it uses PowerShell. Source: [Claude Code advanced setup](https://code.claude.com/docs/en/getting-started#system-requirements).
- **Codex CLI supports Windows.** OpenAI's CLI guide has a Windows install
  path alongside macOS/Linux and npm, and the Windows documentation covers
  the native Windows sandbox and WSL. equipe uses the npm-installed
  `codex.cmd` on native Windows. Source: [OpenAI Codex CLI](https://learn.chatgpt.com/docs/codex/cli).

equipe searches the inherited `PATH`, `%APPDATA%\npm`, and the native
`%USERPROFILE%\.local\bin` location. A bare `claude` or `codex` resolves
Windows executable suffixes, including npm's `.cmd` launcher and native
`.exe` files. Explicit paths continue to work, including paths with spaces or
non-ASCII characters.

## Shells

Installers run in PowerShell on Windows. Features whose command text is POSIX
shell syntax must use Git for Windows Bash. equipe looks first at
`CLAUDE_CODE_GIT_BASH_PATH`, then the standard per-machine and per-user Git
locations. If Git Bash is absent, the feature must return an installation
message instead of passing Bash syntax to PowerShell.

## Browsers

Member browsers keep equipe-owned profiles below
`%USERPROFILE%\.equipe\browser`; equipe does not open Chrome or Edge's live
`Default` profile. The browsers' standard profile roots are:

- Chrome: `%LOCALAPPDATA%\Google\Chrome\User Data`
- Edge: `%LOCALAPPDATA%\Microsoft\Edge\User Data`

Chrome is preferred. If it is absent and Edge is installed, Playwright uses
the `msedge` channel. Both per-machine (`%ProgramFiles%` and
`%ProgramFiles(x86)%`) and per-user (`%LOCALAPPDATA%`) application locations
are detected.

## Desktop control

Desktop control runs in the signed-in interactive session. Windows Graphics
Capture supplies display and individual-window JPEGs; UI Automation supplies
the element tree, hit testing, focus, values, Invoke actions, and window
selection. Unicode `SendInput` handles text, while every input remains behind
the existing one-member screen lease and send approval gate. The helper link is
a local named pipe that rejects remote clients and authenticates requests with
the same private token used on macOS.

Windows has no macOS-style Screen Recording or Accessibility consent toggles.
UIPI can still refuse control of a window running at a higher integrity level;
the tool reports that refusal instead of elevating equipe.
