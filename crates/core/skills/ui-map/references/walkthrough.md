# A flow's walk and its video

Each flow keeps a `walk` beside its screens — its steps through the running app (`go`, `click`, `fill`, `press`,
`wait`, `say` a caption; `screen` where one appears) — and `equipe design flows record --flow <id>` plays it into
`flows/<flow>/walkthrough.<mode>.<width>.mp4` (earlier ones kept under `history/`); an app that records itself gives
its flow a `record_command` instead. The walk is recorded in an en-US browser unless the flow says its `locale`
(`de-DE`); `reducedMotion: true` and `permissions` (`clipboard-write`) are set on the flow too, and its states are shot
with the same unless they say otherwise.

When the person asks to see a flow as a video, record it on fixtures or demo data, never a real account or
credentials, and give them its absolute path as you would a picture's.

Whenever a screen changes, its flow's `walk` and video change in the same hand-in.
