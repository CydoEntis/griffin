# Plan: Glyph

Source of truth for scope, order, decisions and rules. Tickets hold the detail.
Read this before starting work. If work conflicts with it, stop and say so.
Last reconciled: 2026-10-06 at `3e50a61` on `main`.
Verify: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

## Now

Phases 1–5 complete. Phase 6 is complete except its person check: open a real
project in Windows Terminal (inside Hydra) and on Ubuntu, and use every default key
once (spec Verification).

## Phases

### 1 — `glyph file.rs` opens, edits and saves one file safely, by keyboard and mouse · complete 2026-10-06 at `e0effc0`

#1, #2, #3, #4, #5, #6, #7, #8, #9, #10

### 2 — `glyph .` works across a whole project · complete 2026-10-06 at `4ca0ec6`

#11, #12, #13, #14, #15, #16, #17

### 3 — Find and replace, in a file and across the project · complete 2026-10-06 at `3c95f84`

#18, #55, #19, #20, #21

### 4 — All 8 languages are highlighted · complete 2026-10-06 at `e4779b4`

#22, #23, #24, #25, #26

### 5 — Start a dev server inside Glyph · complete 2026-10-06 at `6b9c873`

#27, #28, #29

### 6 — LSP: diagnostics, definition, hover, completion, format · complete except the person check, at `3e50a61`

#30, #31, #32, #33, #34, #35, #36, #75, #76

## In scope

- Glyph v1 — shipped — [intent](features/glyph-v1/intent.md) ·
  [spec](features/glyph-v1/spec.md)

## Out

- Plugin / scripting system — declined 2026-10-06: Glyph is extended by changing
  Glyph ([ADR-0001](adr/0001-single-binary-core.md)). Bringing it back needs a new
  decision.

## Later

- Embedded interactive terminal — deferred 2026-10-06: Hydra is the terminal; the
  run panel streams output. Comes back when the run panel's read-only output gets in
  the way daily.
- Multi-cursor — deferred 2026-10-06. Comes back after v1 is the daily editor.
- Git integration (gutter diff, staging) — deferred 2026-10-06. Comes back after v1
  is the daily editor.
- LSP rename, code actions, signature help; soft wrap; more than two splits —
  deferred 2026-10-06. Come back when daily use asks for them.
- Known v1 limitations — deferred 2026-10-06, each comes back when it bites in daily use:
  a Tab inside text pasted into the find bar switches fields on Windows (#19);
  diagnostics are not shifted by edits until the server republishes (#31);
  expanding a folder reads it on the main task (#12); injected `<style>`/`<script>`
  regions re-parse in full on every edit (#24); one bad theme override drops all
  overrides (#17).

## Decisions

- [ADR-0001](adr/0001-single-binary-core.md) — one binary: ropey buffer,
  compiled-in tree-sitter grammars, one event loop owning all state, tools as child
  processes. Rules out: plugins, runtime grammar loading, bundled language servers,
  mutating `App` from a background task. Highlighting builds each language's queries
  with `tree-sitter-highlight` and runs them with a `QueryCursor` on the
  incrementally re-parsed tree (#22).

## Rules

- Saves write a temp file in the target's folder and rename it over the target —
  `src/save.rs`, its unit tests — R7
- Key events reach actions only through the keymap; nothing outside `src/keymap.rs`
  matches on `KeyCode` — `rg -n "KeyCode::" src --glob '!src/keymap.rs'` prints
  nothing — R5
- A missing or crashed language server never blocks editing — fake-server test in
  `tests/` — R29

## Records

Glossary: none yet.
ADRs: `docs/adr/`. Feature docs: `docs/features/`.

## Rework

- 2026-10-06 #17 → #55: the PTY harness trusted synchronized-update markers that
  Windows ConPTY sends before the frame is drawn.
- 2026-10-06 #33 → #76: key-release events closed popups on Windows.
- 2026-10-06 #36 → #75: server spawning did not resolve `.cmd` shims through PATHEXT.
