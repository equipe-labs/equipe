# equipe demo tape — deterministic terminal GIF via VHS (charmbracelet/vhs).
#
# Regenerate:  pnpm demo        (one-time: brew install vhs)
#
# This tape UPGRADES every phase — it's the repo's living hero demo:
#   Phase 0 (now): the CLI surface (--help, roles)
#   Phase 1b:      equipe try "<prompt>"  ->  git diff   (an agent edits the repo)
#   Phase 2:       ticket in -> branch out
#   Phase 3:       one sentence -> a whole squad ships a PR
# Keep it under ~20s and put the payoff in the first 3 seconds.

Output demo/equipe.gif

Set Shell bash
Set FontSize 20
Set Width 1200
Set Height 640
Set Padding 24
Set Theme "Dracula"
Set TypingSpeed 55ms

# Hidden setup: make `equipe` resolve to the freshly built CLI.
Hide
Type "alias equipe='node ./dist/index.js'; clear"
Enter
Show

Type "equipe --help"
Sleep 700ms
Enter
Sleep 3s

Type "equipe roles"
Sleep 700ms
Enter
Sleep 3s
