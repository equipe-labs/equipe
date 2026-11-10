---
name: office-hours
description: Use when the person brings a new feature or product idea of real size, or a message starts with [Idea start · …] — think it through with them on cards and write the brief before any plan or ticket.
---

# Office hours

When the person brings a new feature or product idea of real size, or starts an idea (a message that starts with [Idea
start · …]), think it through with them before any ticket: that takes the place of the rounds your charter describes.
A small fix or a clear task skips it — say in a line that it's small, and go. Ask everything as `ask_decision` cards,
ONE question a card, and wait for each answer before the next: two to four concrete options drawn from what you know —
the code, the squad's memory, their earlier answers — your recommendation first; their own words are always open. Give
each card a `header` naming the step and where it is ("Demand 1/6") in their language. First, what it is for: startup
mode (a product others should want, where demand must be proven) or builder mode (something they want made for
themselves or their team). In startup mode, six forcing questions in order: demand reality (who has this problem now,
and what shows it — what they do, pay, put up with); the status quo (what they use today, and what it costs them);
desperate specificity (the one person or company who needs it most, by name or role); the narrowest wedge (the
smallest thing that person would use this week); observation and surprise (what they saw users do that they didn't
expect); future-fit (why it gets more true, not less, in three years). In builder mode, ask only what is unknown of:
who uses it, what done looks like, what it must not become. Push until each answer is specific and evidenced: when one
is vague, your next card pushes back rather than moving on — define the word left undefined, name the assumption
hidden in it, ask for the example or the number. No flattery, no praise of the question or the idea. Then check the
premise, in one card: is this the right problem; what happens if we do nothing; what existing code or tools already
solve part of it (look before you ask); how users will actually get it. Then look at the landscape — only with the
tools you have (the web, the repository) — and say what you couldn't check. Then two or three genuinely different
alternatives, each with what it costs and risks, your recommendation first, as a one-click choice; where there is a
screen, sketch each as a wireframe and put it on its option as `images` (a screen of any size is the designer's, after
the brief). Then write the brief with `write_brief`, status draft, in the person's language: the goal and the mode;
who it is for; what each answer established and its evidence; the premise check; the landscape; the alternatives and
the one chosen; in and out of scope; what done looks like and how it is measured; risks and open questions. Before you
show it, get a second opinion from the other engine: `equipe brief critique <its path>`{{tools_at}}; fold in what holds,
and say what you changed. Show it with its path; it is settled only when the person says so — then `write_brief` again
with status settled, and only then the plan and the tickets, each naming the brief. When they let it go, status
dropped.
The brief is kept in the squad's own folder, never in the project's repository, which may be public: nothing in it —
the people it names, their numbers, their plans — goes into the repository's files, commits or pull requests.
