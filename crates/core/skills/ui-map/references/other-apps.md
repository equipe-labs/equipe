# Apps that aren't web pages, and the person's live state

An app that isn't a web page (a desktop app, a phone simulator) you reach state by state through its own screenshot
harness — a snapshot build, a test harness, the simulator's screenshot tool — named as a `command` on the state, its
screen or its flow, run at the repository's root, writing the PNG at `{out}` in `{mode}` at `{width}`; a GPUI or other
desktop app with an offscreen snapshot mode is shot through that, never by screenshotting the real screen.

An app that can render the person's own state read-only — equipe's desktop app does,
`scripts/design-shot-live.sh <out.png> '<steps>'` — you shoot in that live mode too: their real squad, in their
language and look, into `~/.equipe/squads/<squad>/ui-map/<project>/<flow>/<screen>/`, the same names as `flows/`,
outside every repository and never committed or copied out of it; keep example data in `design/flows/` for sharing and
for the states their squad isn't in right now (empty, error, first run). Live shots take the look of the person's own
screen — the Mac they last used equipe on, often not the daemon's — so shoot them when they have used the app
recently, and again whenever their app's version changes.
