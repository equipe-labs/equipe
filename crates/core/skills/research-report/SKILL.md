---
name: research-report
description: Use when the person asks you to research, investigate, compare or explain something in depth, or an answer would run past a screen — a report and a deck instead of a long reply.
---

# A report and a deck

Write the report (`report.md`) for someone who wasn't there: the answer first, in a sentence or two; then three to
five key findings; then a section for each part, under a heading, in short paragraphs and plain words, each term
explained where it first appears; a table where there are options to compare; what you recommend and the next steps;
what you aren't sure of; and the sources, each a link. Show as well as tell: put a visual wherever it makes something
easier to grasp — a Mermaid diagram in a ```mermaid block (a flowchart for how something works, a sequence diagram for
who does what, a timeline, a mindmap, a quadrantChart for where things stand, an xychart-beta or a pie for numbers), a
table, or a picture saved beside the report (a screenshot, a chart drawn with a script) embedded by its absolute path.
Every number in a chart comes from your sources; never invent one to fill it. Make the deck (`deck.md`) so it is
understood in a few minutes: Marp's Markdown — a front matter of `marp: true`, slides split by a line of `---` — six
to twelve slides: the title, the answer, one idea a slide in at most five short lines, a visual on most, the
recommendation last. The same file becomes a PDF or a PowerPoint with Marp's CLI (`npx @marp-team/marp-cli deck.md
--pdf`, or `--pptx`) when the person wants one. Then reply briefly: the answer in two or three sentences and both
files' paths in backticks — equipe opens the report beside the conversation and the deck as a presentation, each with
a click.
