---
name: mobile-qa
description: Use before you test the squad's mobile app on its simulator or emulator with the `phone` tools — write the flow, record it, check each step on its frames, report with them.
---

# Testing the mobile app as a person does

A flow passes on what the screens showed at each step, not on the last screenshot and not on the code. Work in
this order.

1. **Write the flow down first.** Before you touch the phone, list its steps (what a person does) and, for each,
   the screen it should lead to — the words, buttons and values a person would see. Take them from the ticket's
   acceptance criteria and its test-plan cases (`TP-…`) when it has them; add the edges a person hits: an empty
   field, a wrong value, going back, a long name, the keyboard covering a button.
2. **Start the ticket's phone with the build under test.** Build the app for the simulator or emulator (the device
   id `start` gives is what to build for), then `start` with the built `.app` or `.apk`. A build that fails is the
   finding: report it, don't test an older build.
{{#if builder}}
   You check your own work this way before you hand it in.
{{/if}}
3. **Look before each step.** `look` shows the screen and its elements; aim by the label a person reads, an id
   when labels repeat, a point only when nothing else names it. A refused tap means the screen isn't what you
   thought: look again rather than retrying.
4. **Record the whole flow.** `record` `start` before the first step, give each action its `step` as you go, `record`
   `stop` after the last. One flow, one recording: a second flow gets its own.
5. **Check every step against its frames.** `frames` gives the moment before each action and the screen it led to.
   Compare each with what you wrote in step 1, word by word and value by value. A step whose frame shows a spinner, an
   alert or the wrong screen is not a pass. Open a frame full size when text is small.
6. **Report with `report`.** Each step: what you expected, what the frame shows, `pass`, `fail` or `blocked`, and the
   frame. Never pass a step you didn't see; one you couldn't reach is `blocked`, with why.
7. **File each failure as a bug** with `create_ticket`: the steps to reproduce, expected against seen, the
   recording's path and the frame's path. One bug a ticket.
{{#if builder}}
   A failure in your own ticket's work you fix instead, then test the flow again.
{{/if}}
8. **Put the phone away** with `stop` before you hand in, and name the report's recording and key frames in your
   hand-in.

The device and its recordings are the ticket's: equipe deletes them when the ticket is done. Never use another
simulator, emulator or a real phone, and never sign in with a real person's account on the phone — ask the person
for a test account with `block_on_person`.
