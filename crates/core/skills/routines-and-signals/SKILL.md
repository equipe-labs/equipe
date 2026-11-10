---
name: routines-and-signals
description: Use when a message starts with [Routine · …], [Idle · …] or [Signal · …], or carries [Retro facts · …], and when regular work or a check of something could help the squad.
---

# Routines, the idle board, the retro and signals

A message that starts with [Routine · …] is one of the person's routines, set up to come on a schedule: do what it
asks as if they had just written it, and keep what it produces where it can be found again{{hq_in}}. A message that
starts with [Idle · …] is equipe waking you because the board has lain empty: nobody else will start the next work.
For either, put the next work up as proposed tickets (`create_ticket` with `proposal`), tied to the squad's goal — its
stage, North Star and OKRs — rather than starting it unasked, and gather everything the person must decide into one
card (one `ask_decision` with up to four questions), never a string of separate asks. Routines are the person's to
turn on, never yours: when regular work would help — a weekly check-in, a monthly look at the dependencies — propose
it with `propose_routine`, and only their click on its card turns it on. A routine that carries [Retro facts · …] is
the weekly retro: write it from those facts, not from memory, and propose its changes on one card — never change a
setting or a routine yourself for it; the lessons the person confirms equipe keeps in the squad's memory under
"lessons", which you read before planning. A message that starts with [Signal · …] is one of the squad's signals
failing — a check of something the squad measures or ships: treat it as a breakage nobody else has noticed, and at
once put up a ticket to fix it or put what the person must do in front of them as a card. Read the signals (`signals`)
at every check-in; when something could break unnoticed — an analytics tag, a workflow, a store listing — propose a
signal with `propose_signal`, which the person turns on on its card.
