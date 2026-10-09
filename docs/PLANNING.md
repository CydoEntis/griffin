# Plan: Glyph

Source of truth for scope, order, decisions and rules. Tickets hold the detail.
Read this before starting work. If work conflicts with it, stop and say so.
Last reconciled: 2026-10-06 at `3e50a61` on `main`.
Verify: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

## Now

Active phase: **8 — Glyph opens on a splash and can switch projects.** Phases 9 and 10 are
ticketed ahead, as Cody asked to build them in one run (2026-10-08). The person checks for
Phases 6 and 7 are still open. Phases 12, 13 and 14 are ticketed ahead as well (Cody, 2026-10-08).
Phase 15 is ticketed ahead too (Cody, 2026-10-09).
Next unblocked: #124 — fix(project): open a folder onto the tree and an empty pane, not the
splash (PR #125).

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
6. #124 — fix(project): open a folder onto the tree and an empty pane, not the splash · after 4
7. #126 — fix(splash): splash layout from Cody's design · after 6
8. #127 — feat(splash): splash status bar, q to quit, tree hidden · after 7
9. #128 — feat(editor): no file open screen with the key list · after 8

### 9 — Install language servers from a catalog inside Glyph

Exit when:
- every ticket below is closed and its change is on `main`;
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passes on `main`;
- the spec's person check passes (install a missing server from the catalog on Windows and
  get completion in an open file without restarting).

[Language server catalog](features/glyph-catalog/intent.md) · [spec](features/glyph-catalog/spec.md)

1. #129 — feat(lsp): retry a language's server after it was missing
2. #130 — feat(lsp): install commands for each language server · can run alongside 1
3. #131 — feat(catalog): language servers catalog card · after 2
4. #132 — feat(catalog): install a server from the catalog and start it · after 1, 3

### 10 — Debug Rust, Python and Go through the Debug Adapter Protocol

Exit when:
- every ticket below is closed and its change is on `main`;
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passes on `main`;
- the spec's person check passes (break, step and read variables in a Rust, a Python and a
  Go program on Windows and Ubuntu).

[Debugger](features/glyph-debugger/intent.md) · [spec](features/glyph-debugger/spec.md)

1. #133 — feat(dap): Debug Adapter Protocol client and fake adapter
2. #134 — feat(debug): breakpoints in the gutter · can run alongside 1
3. #135 — feat(debug): debug adapters and launch settings · can run alongside 1
4. #136 — feat(debug): start and stop debugging, and the paused line · after 1, 2, 3
5. #137 — feat(debug): continue and step over, into and out · after 4
6. #138 — feat(debug): debug panel with call stack and variables · after 4
7. #139 — feat(catalog): debug adapters in the catalog · after 3, Phase 9's #132

### 11 — The v1 layout behaviours from the Aurora handoff

Work: [SPEC_V1_LAYOUT.md](features/glyph-aurora/design/SPEC_V1_LAYOUT.md) §1–§11 and §14
items not in Phase 7: minimum-size message, tree width and hiding while split, tab overflow
`‹N` and duplicate names, completion anchor shift, run panel scrollback, `‹ ›` scroll
markers, rename preselects the stem, an unmodified run-stop key.
Open decisions:
- Does the tree scale with width (SPEC_V1_LAYOUT §1) or stay 28 (design README §2.1)?
- Which of §14's items are in, and which go to Later?

### 12 — Brackets close themselves as you type

Exit when:
- every ticket below is closed and its change is on `main`;
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passes on `main`;
- the person check passes (in a .js and a .rs file: type `foo(`, a `{` block with Enter,
  Backspace an empty `[]`, wrap a selection in `(`, and turn it off with `auto_pairs = false`).

[Brackets close themselves](features/glyph-autopair/intent.md)

1. #155 — feat(editor): opening brackets insert their closer
2. #156 — feat(editor): Backspace between an empty bracket pair deletes both · after 1
3. #157 — feat(editor): Enter between a bracket pair opens an indented line · after 2
4. #158 — feat(editor): typing an opening bracket wraps the selection · after 3

### 13 — See and change settings, keys and server errors inside Glyph

Exit when:
- every ticket below is closed and its change is on `main`;
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passes on `main`;
- the person check passes (open `>settings`, change `tab_width` and save, it applies; rebind
  `save` in `>keybindings`; with TypeScript 7 installed, the catalog shows the JavaScript
  server failed and why, and Enter retries it).

[Settings, keys and server errors](features/glyph-settings/intent.md)

1. #164 — fix(lsp): keep what a failing language server prints
2. #165 — feat(catalog): failed servers say why and retry on Enter · after 1
3. #166 — feat(config): >settings opens config.toml · can run alongside 1
4. #167 — feat(config): saving config.toml applies keys, editor and theme · after 3
5. #168 — feat(config): saving config.toml restarts changed servers and adapters · after 4
6. #169 — feat(keys): >keybindings card · can run alongside 1, 3
7. #170 — feat(keys): rebind a command from the keybindings card · after 4, 6
8. #171 — feat(keys): moving a key another command already uses · after 7

### 14 — Ctrl+/ comments and uncomments lines

Exit when:
- every ticket below is closed and its change is on `main`;
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passes on `main`;
- the person check passes (Ctrl+/ on a line and on a selection in a .rs, .py and .html file,
  on Windows Terminal and Ubuntu).

[Ctrl+/ comments](features/glyph-comments/intent.md)

1. #173 — feat(editor): Ctrl+/ comments and uncomments lines
2. #174 — feat(editor): Ctrl+/ wraps lines in HTML and CSS comments · after 1

### 15 — C and C++ are highlighted, served, run and debugged

Exit when:
- every ticket below is closed and its change is on `main`;
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` passes on `main`;
- the person check passes (on Windows and Ubuntu: install clangd from the catalog; in a
  hello.c and a hello.cpp see colours, completion and a compile error; Ctrl+/ a line; F5 runs
  a program that asks `Name: ` and answers what you type; Alt+F5 stops at an F9 breakpoint
  and steps).

[C and C++](features/glyph-c-cpp/intent.md)

1. #185 — feat(highlight): C files are highlighted
2. #186 — feat(highlight): C++ files are highlighted · after 1
3. #187 — feat(lsp): clangd serves C and C++ · after 2
4. #188 — feat(catalog): clangd in the language servers catalog · after 3
5. #189 — feat(run): F5 compiles and runs the open C or C++ file · after 3
6. #190 — feat(run): Makefile and CMake projects get run commands · after 5
7. #191 — feat(run): terminal runs show prompts at once · after 5
8. #192 — feat(run): type a line into a running program · after 7
9. #193 — feat(debug): debug the open C or C++ file with lldb-dap · after 5

## In scope

- Glyph v1 — shipped — [intent](features/glyph-v1/intent.md) ·
  [spec](features/glyph-v1/spec.md)
- Glyph Aurora — shipped — [intent](features/glyph-aurora/intent.md) ·
  [spec](features/glyph-aurora/spec.md)
- Splash and opening projects — active — [intent](features/glyph-splash/intent.md) ·
  [spec](features/glyph-splash/spec.md). Overrides the layout handoff's "No launch
  splash" (SPEC_V1_LAYOUT 9a); its key list shows whenever no file is open (decided
  2026-10-08).
- Language server catalog — planned — [intent](features/glyph-catalog/intent.md) ·
  [spec](features/glyph-catalog/spec.md)
- Debugger — planned — [intent](features/glyph-debugger/intent.md) ·
  [spec](features/glyph-debugger/spec.md)
- Brackets close themselves — active — [intent](features/glyph-autopair/intent.md)
- Settings, keys and server errors — active — [intent](features/glyph-settings/intent.md)
- Ctrl+/ comments — active — [intent](features/glyph-comments/intent.md)
- C and C++ — active — [intent](features/glyph-c-cpp/intent.md)

## Out

- Plugin / scripting system — declined 2026-10-06: Glyph is extended by changing
  Glyph ([ADR-0001](adr/0001-single-binary-core.md)). Bringing it back needs a new
  decision.

## Later

- Embedded interactive terminal — deferred 2026-10-06: Hydra is the terminal; the
  run panel streams output. Comes back when the run panel's read-only output gets in
  the way daily. Phase 15 adds line input for runs in a pseudo-terminal; the full
  terminal stays deferred.
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
