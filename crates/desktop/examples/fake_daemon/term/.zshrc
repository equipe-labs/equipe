# The terminal panel's fixture (see `shell` beside it): what a sign-in shows,
# and a prompt like powerlevel10k's two lines — wide CJK letters, emoji, Nerd Font and
# Powerline glyphs, box drawing, and a long URL that wraps.
unsetopt PROMPT_SP
HISTFILE=/dev/null
place="/tmp/equipe-term/日本語 フォルダ"
mkdir -p "$place/报告" && cd "$place"
print -r -- "CJK: こんにちは、ターミナルです — 全角文字は二マス"
print -r -- "emoji: ✔ ✅ 🚀 🐳 👍 ok"
print -r -- $'nerd:       end'
print -r -- "box: ╭─ ─╮ ╰─ │ ┃ ━"
print -r -- "Open this URL to sign in:"
print -r -- "https://claude.ai/oauth/authorize?code=true&client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e&response_type=code&redirect_uri=https%3A%2F%2Fconsole.anthropic.com%2Foauth%2Fcode%2Fcallback&scope=org%3Acreate_api_key+user%3Aprofile&state=equipe"
ls
# The first line runs to the right edge, as powerlevel10k's does: its right
# part ends at the last column, so a grid wider or narrower than the shell
# thinks breaks it.
precmd() {
    local left=$'╭─  '"${PWD/#$HOME/~}"$'  main' right=$'✔ abel@Mac ─╮'
    local fill=$(( COLUMNS - ${(m)#left} - ${(m)#right} - 2 ))
    (( fill < 1 )) && fill=1
    print -r -- "$left ${(pl:$fill::─:)${:-}} $right"
}
PROMPT='╰─ '
# A title, as a program run in it sets: the panel's bar shows it.
print -n $'\e]0;claude auth login\a'
