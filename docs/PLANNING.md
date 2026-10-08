# Plan: Glyph

Source of truth for scope, order, decisions and rules. Tickets hold the detail.
Read this before starting work. If work conflicts with it, stop and say so.
Last reconciled: 2026-10-06 at `3e50a61` on `main`.
Verify: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

## Now

Active phase: **8 — Glyph opens on a splash and can switch projects.** The person checks
for Phases 6 and 7 are still open.
Next unblocked: #113 — feat(lsp): stop one project's language servers;
#115 — feat(splash): splash screen when Glyph starts with nothing to edit.

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

### 7 — Glyph wears the Aurora look · complete except the person check, at `db1a00f`

Exit when:
- every ticket below is closed and its change is on `main`;
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passes on `main`;
- the spec's person check passes (main frame and cast match the design at 160×45).

[Glyph Aurora](features/glyph-aurora/intent.md) · [spec](features/glyph-aurora/spec.md)

1. #80 — feat(theme): Aurora roles and the aurora and moonlit themes
2. #81 — feat(search): project replace all on Alt+A · can run alongside 1
3. #82 — feat(tree): Aurora tree panel · after 1
4. #83 — feat(tabs): tab pills and the aurora thread · after 3
5. #84 — feat(status): Aurora status bar · after 1
6. #85 — feat(status): language and language server state · after 5
7. #86 — feat(editor): gutter without a divider and cursor-line glow · after 4
8. #87 — feat(editor): diagnostic marks, curly underlines and inline lens · after 7
9. #88 — feat(editor): find matches apart from the selection · after 7
10. #89 — feat(ui): dimmed dialogs with a lit edge, starting with project search · after 1, 2
11. #90 — feat(palette): cast palette for files on Ctrl+P · after 10
12. #91 — feat(palette): commands, go to line and project text in cast · after 11
13. #92 — feat(confirm): confirm cards with buttons · after 10
14. #93 — feat(find): Aurora find, replace and prompt bars · after 9
15. #94 — feat(lsp): rounded hover and completion popups · after 8
16. #95 — feat(run): Aurora run panel and run picker · after 10

### 8 — Glyph opens on a splash and can switch projects · active

Exit when:
- every ticket below is closed and its change is on `main`;
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passes on `main`;
- the spec's person check passes (splash, New file / New directory / Open directory, and a
  project switch with an unsaved tab, at 160×45 in `aurora` and `mono`).

[Splash and opening projects](features/glyph-splash/intent.md) · [spec](features/glyph-splash/spec.md)

1. #113 — feat(lsp): stop one project's language servers
2. #114 — feat(project): open another folder as the project from Ctrl+P · after 1
3. #115 — feat(splash): splash screen when Glyph starts with nothing to edit · can run alongside 1
4. #116 — feat(splash): New directory and Open directory on the splash · after 2, 3
5. #117 — feat(project): unsaved files card before switching projects · after 2

### 9 — The v1 layout behaviours from the Aurora handoff

Work: [SPEC_V1_LAYOUT.md](features/glyph-aurora/design/SPEC_V1_LAYOUT.md) §1–§11 and §14
items not in Phase 7: minimum-size message, tree width and hiding while split, tab overflow
`‹N` and duplicate names, completion anchor shift, run panel scrollback, `‹ ›` scroll
markers, rename preselects the stem, an unmodified run-stop key.
Open decisions:
- Does the tree scale with width (SPEC_V1_LAYOUT §1) or stay 28 (design README §2.1)?
- Which of §14's items are in, and which go to Later?

## In scope

- Glyph v1 — shipped — [intent](features/glyph-v1/intent.md) ·
  [spec](features/glyph-v1/spec.md)
- Glyph Aurora — shipped — [intent](features/glyph-aurora/intent.md) ·
  [spec](features/glyph-aurora/spec.md)
- Splash and opening projects — active — [intent](features/glyph-splash/intent.md) ·
  [spec](features/glyph-splash/spec.md). Replaces the layout handoff's empty-editor key
  list (SPEC_V1_LAYOUT 9a, "No launch splash"; decided 2026-10-08).

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

- `@` symbol search in the cast palette — deferred 2026-10-07: needs LSP document
  symbols. Comes back with LSP rename / code actions, or when files and text search
  aren't enough.

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
