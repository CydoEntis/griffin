# Plan: Griffin

Source of truth for scope, order, decisions and rules. Tickets hold the detail.
Read this before starting work. If work conflicts with it, stop and say so.
Last reconciled: 2026-10-06 at `(no commits yet)` on `main`.
Verify: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

## Now

Active phase: **1 — Edit**
Next unblocked: #1 — chore: scaffold the griffin crate with CI

Tickets are filed for every phase at once (decided 2026-10-06, so the build loop
runs end to end). A ticket in a later phase is picked up only once every ticket in
the phases before it is closed. Run `/roadmap reconcile` at each phase boundary.

## Phases

### 1 — `griffin file.rs` opens, edits and saves one file safely, by keyboard and mouse · active

Exit when:
- every ticket below is closed and its change is on `main`;
- the verify command passes on `main`, in CI on Windows and Ubuntu;
- the PTY harness tests for R1–R11 pass against the real binary.

1. #1 — chore: scaffold the griffin crate with CI
2. #2 — test: drive the real binary in a pseudo-terminal · after 1
3. #3 — feat(keys): config file and remappable keymap · after 2
4. #4 — feat(editor): open a file and render it · after 3
5. #5 — feat(editor): move the cursor · after 4
6. #6 — feat(editor): type and delete text · after 5
7. #7 — feat(save): atomic save and unsaved-changes guard · after 6
8. #8 — feat(editor): undo and redo · after 6 · can run alongside 7
9. #9 — feat(editor): select text and use the clipboard · after 8
10. #10 — feat(editor): mouse in the editor · after 9

### 2 — `griffin .` works across a whole project

Exit when: tickets closed and on `main`; verify green; PTY tests for R12–R19 pass.

1. #11 — feat(backup): back up unsaved edits and offer recovery
2. #12 — feat(tree): file tree sidebar · after 1
3. #13 — feat(tree): create, rename and delete from the tree · after 2
4. #14 — feat(tabs): tabs, new file and save as · after 2 · can run alongside 3
5. #15 — feat(split): vertical split and focus cycling · after 4
6. #16 — feat(picker): go to file · after 4
7. #17 — feat(theme): Hydra's theme set and overrides · after 1 · can run alongside 2–6

### 3 — Find and replace, in a file and across the project

Exit when: tickets closed and on `main`; verify green; PTY tests for R20–R23 pass.

1. #18 — feat(find): find in file
2. #19 — feat(find): replace in file · after 1
3. #20 — feat(search): project search · after 1
4. #21 — feat(search): project replace · after 2 and 3

### 4 — All 8 languages are highlighted

Exit when: tickets closed and on `main`; verify green; snapshot tests for R24 pass.

1. #22 — feat(highlight): tree-sitter engine with Rust
2. #23 — feat(highlight): TypeScript, TSX, JavaScript and JSX · after 1
3. #24 — feat(highlight): HTML and CSS · after 2
4. #25 — feat(highlight): Go and Python · after 3
5. #26 — feat(highlight): SQL · after 4

### 5 — Start a dev server inside Griffin

Exit when: tickets closed and on `main`; verify green; PTY tests for R26–R28 pass.

1. #27 — feat(run): run panel from .griffin.toml
2. #28 — feat(run): stop, restart and kill the process tree · after 1
3. #29 — feat(run): detect run commands · after 1 · can run alongside 2

### 6 — LSP: diagnostics, definition, hover, completion, format

Exit when: tickets closed and on `main`; verify green; fake-server tests for
R29–R35 pass; a person has used every default key once in Windows Terminal (inside
Hydra) and on Ubuntu (see the spec's Verification).

1. #30 — feat(lsp): language server client core
2. #31 — feat(lsp): diagnostics · after 1
3. #32 — feat(lsp): go to definition · after 1 · can run alongside 2
4. #33 — feat(lsp): hover · after 1 · can run alongside 2–3
5. #34 — feat(lsp): completion · after 4
6. #35 — feat(lsp): format on save · after 1
7. #36 — feat(lsp): default servers and griffin --health · after 1

Phases 2–6 touch shared files (`src/app.rs`, `src/keymap.rs`); "can run alongside"
holds only where noted.

## In scope

- Griffin v1 — planned — [intent](features/griffin-v1/intent.md) ·
  [spec](features/griffin-v1/spec.md)

## Out

- Plugin / scripting system — declined 2026-10-06: Griffin is extended by changing
  Griffin ([ADR-0001](adr/0001-single-binary-core.md)). Bringing it back needs a new
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

## Decisions

- [ADR-0001](adr/0001-single-binary-core.md) — one binary: ropey buffer,
  compiled-in tree-sitter grammars, one event loop owning all state, tools as child
  processes. Rules out: plugins, runtime grammar loading, bundled language servers,
  mutating `App` from a background task.

## Rules

- Saves write a temp file in the target's folder and rename it over the target —
  `src/save.rs`, its unit tests — R7 · not yet: #7
- Key events reach actions only through the keymap; nothing outside `src/keymap.rs`
  matches on `KeyCode` — `rg -n "KeyCode::" src --glob '!src/keymap.rs'` prints
  nothing — R5 · not yet: #3
- A missing or crashed language server never blocks editing — fake-server test in
  `tests/` — R29 · not yet: #30

## Records

Glossary: none yet.
ADRs: `docs/adr/`. Feature docs: `docs/features/`.

## Rework
