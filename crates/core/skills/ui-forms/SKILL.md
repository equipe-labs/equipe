---
name: ui-forms
description: Use for forms and inputs — labels, controls, validation, errors, submit and destructive actions.
---

# Forms and inputs

A form is a conversation the person didn't ask for. Ask only what is needed, in the order they'd expect, and make
every answer easy to give and easy to fix.

## Structure

- **One column.** Fields stacked, labels above them; side-by-side only for pairs read as one (first and last name,
  city and postcode, start and end date).
- **Ask less**: every field needs a reason; derive what you can; don't ask twice for what the person already gave in
  this flow (WCAG 2.2, Redundant Entry).
- **Group** related fields under a short heading; 32–48px between groups, 16–24px between fields.
- **Long or complex forms** split into steps, each with a clear title and a progress indicator ("Step 2 of 4"); a
  public-service or high-stakes form may ask one question per page.
- **Field width hints at the answer**: a postcode field is short, an address field long. Full-width everything hides
  that hint.
- **Required and optional**: when most fields are required, mark the few optional ones "(optional)"; when most are
  optional, mark the required. Never rely on a red asterisk alone.

## Labels, hints, placeholders

- Every control has a visible label, tied to it in code (`<label for>`), so clicking the label focuses the field.
- A hint (format, why it is asked) sits between the label and the field, not in a tooltip.
- A placeholder is an example of the format (`name@company.com`), never the label — it vanishes when typing starts.
- Labels are short nouns ("Work email"); questions are fine when the form is a dialogue ("What's your role?").

## The right control

- Text: the right `type`, `inputmode` and `autocomplete` (`email`, `tel`, `one-time-code`, `street-address`…) so
  keyboards and autofill help; `spellcheck` off for codes, emails and usernames.
- 2–5 options, all worth seeing → radio buttons (or a segmented control for view modes); 6 or more → a select or a
  searchable combobox; many with search → a combobox.
- On/off that applies at once → a switch; a choice submitted with the form → a checkbox. Don't mix them.
- Dates: a known date (a birthday) is typed — three fields or one with a clear format; a date near today is picked
  from a calendar; a range offers presets.
- Numbers: a plain text field with `inputmode="decimal"` beats a spinner for anything but tiny ranges; a slider only
  when the exact value doesn't matter.
- Phone numbers, card numbers and codes go in one field that tolerates spaces and dashes; one-time codes accept paste.
- Passwords: a show/hide toggle; paste and password managers allowed; no puzzle or memory test to sign in (WCAG 2.2,
  Accessible Authentication).
- On phones, input text is at least 16px so the browser doesn't zoom in.

## Validation and errors

- **When**: check a field's format when the person leaves it, never on every keystroke while they type; check
  everything on submit. Once a field shows an error, clear it as soon as the input is fixed.
- **Don't block**: let people type and paste anything and explain what's wrong; trim stray spaces silently; keep
  submit enabled until it is in flight, so pressing it shows what's missing.
- **Where**: the message sits next to its field (below the label or the field, consistently), the field is marked
  (border plus icon, not colour alone) and linked to it in code (`aria-describedby`, `aria-invalid`). A long form
  also lists the errors in a summary at the top, each linking to its field; on submit, focus moves to the summary or
  the first error.
- **What it says**: the problem and the fix, in the words of the label — "Enter a work email, such as
  name@company.com", "Start date must be today or later". For an empty field, an instruction ("Enter your company name"); for a
  wrong one, a description of what's needed. Never "Invalid input", "Error", "This field is required", jokes, or
  blame.
- **Keep the input**: never clear what the person typed when showing an error, correct answers included.
- **Server errors** that aren't about one field get a message above the form with what to do; the form keeps its data.

## Submit

- The primary button names the outcome with a verb and noun: "Create project", "Send invite", "Save changes" — not
  "Submit" or "OK".
- One primary; "Cancel" or "Back" as secondary or text. In page forms, buttons align with the fields' left edge; in
  dialogs, follow the platform (primary at the trailing end on the web and macOS).
- While sending: the button shows a spinner and keeps its label ("Saving…"), double submits are blocked, and the
  request carries an idempotency key.
- Enter submits a single-field form; in a text area, Enter is a new line and Cmd/Ctrl+Enter submits.
- After success, say what happened and where it went, or take the person there.
- Warn before leaving with unsaved changes; long forms save drafts.

## Destructive actions

- Prefer undo over confirmation for reversible things (archive, remove from list): act at once, offer "Undo" in a
  toast for 5–10s.
- Confirm what can't be undone: the dialog title names the action and object ("Delete 'Q3 report'?"), the body says
  the consequence, the confirm button repeats the verb ("Delete report") in the danger style, and Cancel is the safe
  default focus.
- For the irreversible and wide (deleting a workspace), ask the person to type the name.

## Review checklist

- [ ] One column; only necessary fields; nothing asked twice; groups and steps where the form is long.
- [ ] Every control has a visible, linked label; hints visible; placeholders only as examples.
- [ ] The right control and the right `type`, `inputmode` and `autocomplete` for each answer; 16px inputs on phones.
- [ ] Validation on leave and on submit, never per keystroke; typing and paste never blocked; submit never pre-disabled.
- [ ] Errors beside their fields, marked by more than colour, linked in code, saying how to fix; a summary for long
      forms; focus moves to the first error; input kept.
- [ ] The primary button names the outcome; loading keeps the label; double submit blocked.
- [ ] Undo for reversible actions; confirmation naming the consequence for irreversible ones.
- [ ] Unsaved changes protected.

Sources: GOV.UK Design System (question pages, error messages; Open Government Licence); WCAG 2.2 (Redundant Entry,
Accessible Authentication, Error Identification); Vercel's Web Interface Guidelines (MIT); Nielsen Norman Group on
form design — in our own words.
