# Settings: what the good apps do

Research for the desktop app's settings page (October 2026). It covers Claude
(desktop, claude.ai, Claude Code), OpenAI Codex (app, IDE extension, CLI),
Linear, Raycast, Arc, VS Code, and Apple's macOS System Settings and Human
Interface Guidelines. Claims marked [3P] come from third-party write-ups,
because the product's own pages didn't say.

## What we found

**Window or page.** Native Mac apps (Raycast, Arc, the Claude desktop app,
Codex) open settings in a window of its own with ⌘,. Web-first apps (Linear,
claude.ai) use a page inside the app with a left list of categories. VS Code
puts an overlay over the editor. Apple: ⌘, from the app menu, the pane list
can't be customised, the open pane is always marked, and the last pane comes
back when settings reopen. ([HIG: Settings][hig-settings], [Claude
desktop][claude-desktop], [Codex app][codex-settings], [Raycast][raycast])

**Categories.** These are short lists in the order people use them:
- Claude desktop: General · Claude Code · Connectors · This computer ·
  Developer · Extensions.
- Codex app: General · Profile · Keyboard shortcuts · Notifications ·
  Appearance · Browser · Computer Use · Personalization · Archived chats.
- Linear groups its list under headings: Account · Features · Administration
  · Your teams. [3P]
- Raycast puts About last, with the version, the changelog and Check for
  Updates.

**Rows.** Each row has the label on the left, one muted line under it saying
what the setting does, and the control on the right. Rows sit in rounded,
inset groups under short headers. This is SwiftUI's grouped `Form`, which
System Settings and Linear's Preferences also use. ([Linear
preferences][linear-prefs], [SwiftUI Settings][swiftui-settings])

**No Save button.** Every product applies a change the moment it is made,
including VS Code ("apply directly as you change them"). Only actions that
can't be undone ask first. ([VS Code settings][vscode])

**Controls.** Apple's guidance:
- A switch for something on or off, kept mini in a form so rows stay one
  height ([toggles][hig-toggles]).
- A segmented control for a few related choices ([segmented][hig-segmented]).
- A pop-up for a longer list, or one that changes ([pop-up
  buttons][hig-popup]).

Appearance is always "Match system / Light / Dark" with System as the
default: claude.ai calls it "Color mode" [3P], and Raycast has "Follow System
Appearance". Apple asks apps not to ask for what they can detect.

**Search.** VS Code filters the list down to the matching rows, matching
descriptions too, and adds `@modified` and `@tag:`. Linear ranks whole pages
above single rows. System Settings keeps its results in the sidebar while you
try each one [3P]. Codex's shortcut list has its own search field.

**Scope.** Claude Code names each scope by who it affects ("You, this
project", "Everyone in the project"). VS Code's User / Workspace / Folder tabs
mark a setting that differs from the default and offer "Reset to default".
([Claude Code settings][cc-settings])

**Agent permissions.**
- Claude and Codex both describe rules as allow / ask / block.
- Claude Code's `/permissions` lists every rule and where it came from, and
  removing a rule takes effect from the next tool call. "Yes, and don't ask
  again" writes a rule you can find there later.
- Codex's app gives each site its own "Always allow / Requires approval /
  Block", and removing a site's rules puts it back on the defaults.
- The default autonomy mode is a picker with a sentence for each choice. Full
  access is marked dangerous and takes an explicit opt-in.

([Claude Code permissions][cc-perms], [Codex approvals][codex-approvals],
[Codex settings][codex-settings])

**MCP and connectors.** Each server is a row with its state (connected, needs
sign-in, failed with the error, off), its scope and how many tools it has,
plus actions: reconnect, sign in, turn off. Claude.ai's connectors give each
tool "Always allow / Needs approval / Blocked". ([Claude Code
MCP][cc-mcp], [Claude connectors][claude-connectors])

**Notifications.** You choose which events notify you (finished, needs
approval, failed), only when the window is out of sight or always, and
whether a sound plays. You don't repeat the system's own permission switch.
([Codex config][codex-config], [Claude desktop][claude-desktop])

**About and updates.** These show the version (copyable), Check for updates
now, automatic updates on or off, release notes, and the licence. Claude's
app restarts itself to install an update once it is downloaded.

**Danger.** None of these apps has a red "danger zone" mixed in with everyday
settings. Destructive actions sit apart, in red, and ask first.

## What equipe takes from it

1. **Opening it.** ⌘, opens settings, and so does the gear at the sidebar's
   foot. Esc or ⌘, again goes back to the work. It is a page inside the window
   (as in Linear and Codex's app): settings' own category list takes the
   sidebar's place, so the traffic lights, the window's size and the
   conversation underneath stay as they were. The last category comes back.
2. **Categories.** In use order: General · Squad & machine · Team ·
   Permissions · Projects · Browser · Keyboard shortcuts · Updates & about.
3. **Rows.** The label and a muted line of description on the left, the
   control on the right, in rounded cards under short headers, all on the
   app's own tokens (`R_LG` cards, `ROW`/`CONTROL` heights, `ui::button`, the
   tooltip layer).
4. **No Save button.** A change applies at once. A setting the daemon keeps
   (mode, rules, effort, model, suggestions, browsers, the Mac's screen) is
   sent to the daemon. One only this window uses (appearance, notifications,
   the start screen, merge defaults, language) is kept in `desktop-target`.
5. **Controls.** A switch for on or off, a segmented control for 2–4 fixed
   choices, a pop-up menu for longer or changing lists (language, model,
   effort).
6. **Appearance.** System, Light or Dark, and System until the person picks
   another.
7. **Search.** It sits at the top of the category list and filters every
   category's rows, matching labels, descriptions and keywords. The matching
   rows show under their category's name.
8. **Permissions.**
   - The squad's default mode is a picker with a sentence for each choice.
   - The squad's "always allow" rules are listed one per row, each with a
     button that takes it back (`Command::Disallow`, which the daemon
     answers with `Event::Allowed`). A "항상" answer on a request shows up
     here at once.
   - The shared sign-in caution is said plainly.
9. **Scope.** The page says whose setting each one is: this Mac's window, the
   squad (daemon), a member or a project. Team rows set one member's model,
   effort and mode, and mode can go back to "follows the squad".
10. **Notifications.** Each event can be turned off on its own (asks for
    permission, ready for review, waits on you, a turn failed), plus a sound
    switch. They only ever come while the window is out of sight.
11. **Updates.** Version, Check now, automatic updates (Sparkle's own
    setting), the "다시 시작해 업데이트" state, and links to the site,
    GitHub and the licence. Without Sparkle (a dev build) the controls say
    why they're off.
12. **Shortcuts.** A read-only list of every key, searchable, in the app's
    `kbd` chips.

[hig-settings]: https://developer.apple.com/design/human-interface-guidelines/settings
[hig-toggles]: https://developer.apple.com/design/human-interface-guidelines/toggles
[hig-segmented]: https://developer.apple.com/design/human-interface-guidelines/segmented-controls
[hig-popup]: https://developer.apple.com/design/human-interface-guidelines/pop-up-buttons
[swiftui-settings]: https://developer.apple.com/documentation/swiftui/settings
[claude-desktop]: https://code.claude.com/docs/en/desktop
[claude-connectors]: https://support.claude.com/en/articles/11176164-use-connectors-to-extend-claude-s-capabilities
[cc-settings]: https://code.claude.com/docs/en/settings
[cc-perms]: https://code.claude.com/docs/en/permissions
[cc-mcp]: https://code.claude.com/docs/en/mcp
[codex-settings]: https://developers.openai.com/codex/app/settings.md
[codex-approvals]: https://learn.chatgpt.com/docs/agent-approvals-security
[codex-config]: https://learn.chatgpt.com/docs/config-file/config-reference
[linear-prefs]: https://linear.app/docs/account-preferences
[raycast]: https://manual.raycast.com/settings.md
[vscode]: https://code.visualstudio.com/docs/configure/settings
