# Intent: Splash screen and opening projects

Type: change
Author: Cody · Status: accepted
Date: 2026-10-08 · Plan: [Phase 8](../../PLANNING.md)

## Problem

Starting Glyph with nothing to open drops you into an empty untitled buffer, and once it's
running there's no way to open a different folder; you quit and relaunch. Cody wants Glyph
to open on a splash screen with *New file* (creates a new file in the directory Glyph was
opened in), *New directory* and *Open directory* ("opens a directory manager that lets you
open a directory"), and to be able to open a different project while working in Glyph.

This overrides v1 [R1](../glyph-v1/spec.md) ("`glyph` alone opens an empty untitled
buffer") and the Aurora handoff's "**No launch splash.**"
([SPEC_V1_LAYOUT §4, 9a](../glyph-aurora/design/SPEC_V1_LAYOUT.md)).

## Proposed outcome

Started with nothing to edit, Glyph shows a splash in the editor area. New file creates a
named file in the project folder and opens it. New directory creates a folder there and
opens it as the project. Open directory opens a folder browser and opens the chosen folder
as the project. The same browser is reachable while working, as `>open directory` in the
Ctrl+P cast palette.

## Affected users and systems

- Cody, on Windows Terminal and the Ubuntu box.
- Startup (`src/main.rs`, `src/workspace/mod.rs`), `App` state and render (`src/app.rs`),
  a new splash and folder browser in `src/ui/`, `src/keymap.rs` (new actions), `src/lsp/`
  (stopping one folder's servers), the run panel (stopped on a switch).

## Constraints

- The splash shows only when nothing is open at launch: bare `glyph`, or `glyph <folder>`
  before any file is open. `glyph file.rs` opens the file directly. Closing the last tab
  still leaves an untitled one, as today (decided 2026-10-08).
- It replaces the planned "no file open" key-list screen (SPEC_V1_LAYOUT 9a), which is not
  built (decided 2026-10-08).
- New file and New directory are created in the project folder, named through the same
  prompt the tree's `a` / `A` use.
- Switching projects with unsaved tabs shows one confirm card listing them: Save all /
  Discard / Cancel. A running command is stopped without asking (decided 2026-10-08).
- Keys only through `src/keymap.rs`. `>open directory` has no default key; default keys
  avoid Ctrl+Shift.
- Aurora look; `mono` keeps working. [ADR-0001](../../adr/0001-single-binary-core.md) holds.

## Out of scope

- A recent-projects list.
- More than one project open at once.
- A file (not folder) browser.
- Creating nested paths from the name prompt (rejected today, `src/workspace/ops.rs:35`).
- Showing the splash again after closing tabs.

## Open questions

## Tickets

#113–#117, in the order and with the blockers in [the plan](../../PLANNING.md).
