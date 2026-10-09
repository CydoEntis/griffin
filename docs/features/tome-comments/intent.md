# Intent: Ctrl+/ comments and uncomments lines

Type: feature
Author: Cody · Status: accepted
Date: 2026-10-08 · Plan: [Phase 14](../../PLANNING.md)

## Problem

"ctrl + / doesnt comment or uncomment based on the state of the line thats a feature we
need." Commenting code out in Tome means typing the marker on every line by hand.

## Proposed outcome

Ctrl+/ comments the cursor's line, or every line the selection touches, and uncomments
them if they're all commented already, in every language Tome highlights.

## Affected users and systems

- Cody, in all 8 languages.
- `src/keymap.rs` (the action and key), `src/highlight/languages/` (each language's
  comment markers), `src/buffer/edit.rs`, `src/app.rs`.

## Constraints

- If every non-blank line is commented, it uncomments; otherwise it comments all of them.
  Blank lines are left alone. Markers go at the least-indented line's column, followed by
  one space; uncommenting removes the marker and one space after it.
- Line markers: `//` Rust, Go, JS/TS; `#` Python; `--` SQL. HTML and CSS wrap each line
  in `<!-- … -->` / `/* … */` (decided 2026-10-08).
- One undo step; the selection still covers the same lines afterwards.
- Bound to `ctrl+/` and `ctrl+7`, because Unix terminals send Ctrl+/ as Ctrl+7.
- A file with no known language says `no comments for this file`.

## Out of scope

- Block-commenting a selection inside one line; per-language config for markers.

## Open questions

## Tickets

#173, #174
