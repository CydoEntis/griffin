# Intent: Brackets close themselves

Type: feature
Author: Cody · Status: accepted
Date: 2026-10-08 · Plan: [Phase 12](../../PLANNING.md)

## Problem

Typing `(`, `[` or `{` in Tome inserts only that character, so every closer is typed by
hand and Enter inside `{}` leaves the `}` on the cursor's line. Cody, editing clipper.js:
"it doesnt auto open and close brackets".

## Proposed outcome

Typing an opening bracket inserts its closer with the cursor between them; typing the
closer steps over it; Backspace on an empty pair removes both; Enter between a pair opens
an indented line with the closer below; typing an opener with text selected wraps it.
`[editor] auto_pairs = false` turns all of it off.

## Affected users and systems

- Cody, in every language Tome opens.
- `src/buffer/edit.rs` (typing, Backspace, Enter), `src/buffer/selection.rs`,
  `src/app.rs` (the `Input::Text` path), `src/config.rs` (`EditorConfig`).

## Constraints

- Pairs are `()`, `[]`, `{}` only; quotes don't pair (decided 2026-10-08).
- An opener pairs only when the next character is whitespace, a line end, the end of the
  file, or one of `) ] }`; typing `(` before a word inserts just `(`.
- Typing a closer when that same closer is right after the cursor moves past it, whether
  Tome inserted it or not.
- On by default; `auto_pairs` sits in `[editor]` beside `tab_width` and `insert_spaces`.
- Undo: an auto-inserted closer belongs to the typing run it came with (R9); the pair
  Backspace, the Enter expansion and the wrap are each one step.
- A paste never pairs. On Windows a one-line paste arrives as keypresses
  (`src/keymap.rs:872`); stepping over closers keeps pasted `foo(bar)` intact.
- No new keys or actions: this is typed text, so `src/keymap.rs` doesn't change.

## Out of scope

- Quote, backtick and `<>` pairing; per-language pair lists.
- Adding `()` when a completion is accepted.
- Highlighting the matching bracket, or jumping to it.

## Open questions

## Tickets

#155, #156, #157, #158
