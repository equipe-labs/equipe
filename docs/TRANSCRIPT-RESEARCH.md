# How agent apps draw a transcript

Notes behind the conversation view's tool groups, news lines and reply
actions (desktop, `views/chat.rs`). What the others do, what we took.

## What the others do

- **Claude Code** (terminal and desktop). Consecutive reads and searches
  collapse to one summary line ("Read 3 files", "Searched for 2 patterns");
  Ctrl+O shows the full transcript, and the desktop app opens a group in
  place. A collapsed line keeps a live elapsed-time counter while a call
  runs, so a long command doesn't look stuck (v2.1.210). Thinking is a
  collapsed "Thought for 8s" disclosure; subagents and tool I/O expand in
  place. Under a finished reply sits a small action row (copy, a relative
  time), visible on the latest reply and on hover for the rest. The person's
  messages are right-aligned bubbles; replies run full width with no name
  header. Code blocks are borderless fills with a copy icon inside.
- **Cursor** (agent window, 1.4 → 3.x). Reads, greps, globs and fetches fold
  into one "Explored N files" line at every density; a compact mode hides
  tool icons and collapses diffs by default; each row of an opened group
  opens on its own. Edits show a diff stat ("+12 −3") that opens to the diff.
- **Codex app.** Shell, Read and Search activity is collapsed by default to
  high-level steps ("Explored", "Ran …") that open to the real command and
  its output; users asked for an "always expand" switch, so the collapse is
  the default but not a wall.
- **Zed agent panel.** Tool cards: a terminal card can be collapsed even
  while running (`expand_terminal_card`), output sits in a height-capped
  block; thinking renders nearly expanded with a max height, auto-scrolling.

## What equipe does now

- Consecutive tool calls are **one group**: a quiet line with the group's
  meaning — the one call it holds, a delegated task's description, or a
  summary ("명령 3개 실행 · 파일 2개 읽음") — then a chevron. While a call
  runs the line names it in the present tense with a spinner ("실행 중 cargo
  test"); done, it settles into the summary on the same single line.
- Opened, the group is a hairline card with **one row per call**: a muted verb
  phrase, the object (truncated to one line), its time, and a chevron when
  the row has more — a command's full text and output (mono, height-capped,
  scrolling), an edit's diff (stat "+12 −3" on the row), a long comment or
  task as text. Rows are keyboard-reachable (Tab/arrows, Enter/Space); the
  focused row gets a 1px accent ring.
- A **failed call** breaks the group: it stands on its own, red-tinted, open,
  with its output showing.
- **Ticket references** in a row open the ticket beside the conversation
  (the board's peek panel, with back, expand and close).
- **News** (handed in, accepted, proposed…) are one-line event rows with an
  icon per kind; more than three in a row fold to "소식 4개". Lines stored in
  another language are recognised against every language's template and
  re-worded in the one in use.
- **Thinking** is "생각함 · 8초 ›", opening in place.
- Expand and collapse ease height and opacity, the chevron turns
  (`motion::openness`, `motion::Reveal`); groups stay as they were left while
  the app runs.
- A finished turn's last reply has an **action row** under it — copy, retry,
  how long the turn worked and how long ago — always on the latest reply, on
  hover for the others; nothing floats over the text.
- Inline code is a quiet fill without an icon; a path that exists on this Mac
  underlines on hover and opens.

## Sources

- Claude Code changelog, v2.1.210 (elapsed time on the collapsed tool line):
  <https://newreleases.io/project/github/anthropics/claude-code/release/v2.1.210>
- Collapsed Read/Search summaries (Claude Code issue #22458):
  <https://claudeissues.com/issue/22458-collapsed-read-search-summaries-no-longer-show-file-names-or-patterns>
- Claude Code interactive mode (Ctrl+O transcript):
  <https://www.mintlify.com/jackdog668/claude-code/usage/interactive-mode>
- Cursor 1.4 changelog (compact chat, diffs collapsed):
  <https://cursor.com/en/changelog/1-4>
- Cursor forum, "Explored N tools" folding:
  <https://forum.cursor.com/t/new-version-hides-agent-tool-call-details-in-defiance-of-setting/165292>
- Codex app first look: <https://ndurner.substack.com/p/openai-codex-app-agentic-ai-run-locally>
- Codex activity collapsed by default:
  <https://community.openai.com/t/feature-request-always-expand-codex-activity-tool-logs/1400676>
- Zed agent settings (`expand_terminal_card`, thinking display):
  <https://zed.dev/docs/ai/agent-settings>
- Screenshots of Claude Code desktop the person sent (action row, bubbles,
  code blocks, activity groups and the side panel).
