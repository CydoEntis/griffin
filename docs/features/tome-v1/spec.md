# Spec: Tome v1

Intent: [intent.md](intent.md) · Status: accepted · Date: 2026-10-06

## Concerns

- **Terminals swallow some keys.** Windows Terminal binds Ctrl+Tab, Ctrl+Shift+F,
  Ctrl+Shift+P, Ctrl+Shift+W, Ctrl+, and Alt+Space. Hydra's prefix is Ctrl+Space.
  Legacy Linux terminals can't tell Ctrl+Shift+X from Ctrl+X, send Ctrl+H as
  Backspace and Ctrl+J as Enter, and turn Ctrl+punctuation into control bytes;
  Alt+\ is the ESC `\` string terminator. Recommend: default bindings use only plain
  keys, Ctrl+letter (not H, I, J, M), Alt+letter or punctuation other than `\`, and
  F-keys. Everything stays remappable (R5). Applied in the keymap below.
- **Losing edits kills trust.** Recommend atomic saves (R7) and crash backups (R12)
  before any other workspace feature. Applied in the phase order.
- **New dependencies.** Every crate named under Design is new to this repo; each
  ticket that adds one lists it under Impact.
- **Deleting files.** Recommend tree delete moves to the OS trash (`trash` crate),
  so a wrong key never destroys work. Applied in R15.

## Requirements

Phase 1, Edit
- **R1.** `tome <file>` opens the file; `tome` alone opens an empty untitled buffer; Ctrl+Q quits and restores the terminal, also after a panic — checked by PTY harness tests. Since Phase 8, `tome` alone (or with a folder) opens on the splash screen, and Esc or Ctrl+N gives the empty untitled buffer ([Splash S1, S3](../tome-splash/spec.md)).
- **R2.** The PTY harness launches the real `tome` binary in a pseudo-terminal, sends keys and mouse input, and asserts on the parsed screen — checked by `cargo test --test harness_smoke`.
- **R3.** The buffer shows line numbers, wraps nothing (horizontal scroll instead), renders tabs at `tab_width` and wide characters at width 2 — checked by PTY test against a fixture with tabs, CJK and emoji.
- **R4.** Arrows, Home/End, PageUp/PageDown, Ctrl+Left/Right (word), Ctrl+Home/End move the cursor; the viewport follows it — checked by PTY tests.
- **R5.** Every action is dispatched through one keymap table loaded from defaults + `[keys]` in config; remapping an action in config changes its key without a rebuild — checked by a test that remaps `save` and drives it.
- **R6.** Typing, Enter (copies the line's indent), Backspace, Delete and Tab (spaces or tab per config) edit the buffer — checked by PTY tests.
- **R7.** Ctrl+S saves by writing a temp file in the same folder and renaming it over the target; line endings (LF/CRLF) and the final newline are preserved; a failed write leaves the original untouched — checked by unit tests in `save`.
- **R8.** The status line shows a dirty marker; Ctrl+Q / closing a dirty buffer asks Save / Discard / Cancel — checked by PTY test.
- **R9.** Ctrl+Z / Ctrl+Y undo and redo; consecutive typing undoes as one step — checked by unit tests.
- **R10.** Shift+movement selects, Ctrl+A selects all, Ctrl+C / Ctrl+X / Ctrl+V use the OS clipboard, bracketed paste inserts as one undo step — checked by unit + PTY tests (clipboard behind a trait so tests use a fake).
- **R11.** Mouse: click places the cursor, drag selects, double-click selects a word, wheel scrolls — checked by PTY tests sending SGR mouse sequences.

Phase 2, Workspace
- **R12.** Unsaved changes are backed up to the data dir within 2 s of the last edit; reopening a file with a newer backup offers Recover / Discard; saving or closing clean removes the backup — checked by tests.
- **R13.** `tome <dir>` opens the folder with a file tree on the left (Ctrl+B toggles, Ctrl+E switches focus); folders expand/collapse; Enter or click opens a file; `.gitignore`d files are hidden — checked by PTY test on a fixture folder.
- **R14.** In the tree: `a` new file, `A` new folder, `r` rename, `d` delete, each through a prompt; the tree refreshes; a new file opens in a tab — checked by PTY test.
- **R15.** Delete moves to the OS trash after a confirm — checked by test with the trash call behind a trait.
- **R16.** Tabs: each open buffer has a tab; Alt+, / Alt+. move between tabs, Alt+1..9 jump, Ctrl+W closes (dirty guard), Ctrl+N new untitled, Alt+S save as; click selects, middle-click closes — checked by PTY tests.
- **R17.** Alt+V toggles a vertical split showing two tabs side by side; F6 cycles focus tree → left → right → run panel; click focuses a split — checked by PTY test.
- **R18.** Ctrl+P opens a fuzzy file picker over the project (respecting `.gitignore`); typing filters, Enter opens — checked by PTY test; picker lists 50k files in under 1 s (bench test, ignored by default). Since Phase 7 the picker is the cast palette ([Aurora R11](../tome-aurora/spec.md)).
- **R19.** `theme` selects one of Hydra's 11 themes by name; `[theme_overrides]` sets any color — checked by unit test parsing every theme and an override.

Phase 3, Search
- **R20.** Ctrl+F opens a find bar; matches highlight as you type; Enter / Shift+Enter next / previous; Alt+C case, Alt+R regex, Esc closes — checked by PTY test.
- **R21.** Ctrl+R opens find + replace; Replace and Replace All; Replace All is one undo step — checked by unit + PTY tests.
- **R22.** Alt+F searches the project (respecting `.gitignore`), listing file:line:text; Enter opens the hit — checked by PTY test.
- **R23.** In project search, Tab to the replace field and Alt+A (was Alt+Enter, which Windows Terminal takes; changed 2026-10-07, [Aurora R14](../tome-aurora/spec.md)) replaces in every listed file (open buffers are edited in place, others saved atomically per R7) — checked by test.

Phase 4, Highlight
- **R24.** Files are colored by tree-sitter by extension: `.rs`; `.ts` `.mts` `.cts`; `.tsx`; `.js` `.mjs` `.cjs` `.jsx`; `.html` `.htm` (with `<style>`/`<script>` injected); `.css`; `.go`; `.py` `.pyi`; `.sql`. Highlight capture names map to theme colors — checked by snapshot test per language.
- **R25.** Highlighting updates incrementally on edit; typing in a 10k-line file keeps a frame under 16 ms (bench test, ignored by default) — checked by bench.

Phase 5, Run
- **R26.** F5 runs a command from `.tome.toml` `[[run]]` (a picker if several) in a bottom panel; stdout/stderr stream in with ANSI colors; F4 toggles the panel — checked by PTY test with a fixture command.
- **R27.** Shift+F5 stops, Ctrl+F5 restarts; stopping or quitting kills the whole process tree (Windows job object, Unix process group) — checked by test that the child's child is gone.
- **R28.** With no `[[run]]`, the picker offers detected commands: `package.json` scripts (via the lockfile's package manager), `cargo run`, `go run .` — checked by unit tests on fixture folders.

Phase 6, LSP
- **R29.** Opening a file starts its language's server once per project root and keeps it in sync (didOpen/didChange/didSave/didClose); a missing server shows once in the status line and editing carries on — checked by tests with a fake LSP server binary.
- **R30.** Diagnostics underline their range, mark the gutter, count in the status line; F8 / Shift+F8 jump to next / previous; hovering the cursor on one shows its message — checked by test with the fake server.
- **R31.** F12 or Ctrl+click goes to definition (opening the file if needed); Alt+Left jumps back — checked by test with the fake server.
- **R32.** Alt+K shows hover info in a popup — checked by test with the fake server.
- **R33.** Completion pops up after trigger characters or Alt+/; ↑↓ select, Enter/Tab accept, Esc dismisses — checked by test with the fake server.
- **R34.** With `format_on_save = true` for a language, save formats through the server first — checked by test with the fake server.
- **R35.** Default server commands exist for all 8 languages and `tome --health` prints, per language, the server command and whether it was found — checked by CLI output test.

## Design

**Screen** (Hydra's look: no boxes, `│` dividers, chrome on `sidebar_bg`):

The look below is superseded by [Tome Aurora](../tome-aurora/spec.md); the regions and behaviour still hold.

```
row 0      tab bar (per split)                       ● dirty marks
rows 1..   file tree (30 cols) │ editor [│ editor]
           run panel (toggle, ~30% height, title row + output)
           find / prompt bar (1 row, when open)
last row   status: message · path · Ln:Col · language · LSP · ⚠ diag count
```
Dialogs (go to file, confirm, recover) are centred cards over a dimmed screen, as
in Hydra. Hover and completion are small cards anchored at the cursor (below it,
flipping above or shifting left near the screen edges), with no dimming
(decided 2026-10-06).

**Default keymap** (all remappable, R5):

| Action | Key | Action | Key |
|---|---|---|---|
| quit | Ctrl+Q | find | Ctrl+F |
| save | Ctrl+S | replace | Ctrl+R |
| save as | Alt+S | project search | Alt+F |
| new file | Ctrl+N | go to file | Ctrl+P |
| close tab | Ctrl+W | go to line | Ctrl+G |
| undo / redo | Ctrl+Z / Ctrl+Y | toggle tree / focus tree | Ctrl+B / Ctrl+E |
| copy / cut / paste | Ctrl+C / X / V | toggle split | Alt+V |
| select all | Ctrl+A | cycle focus | F6 |
| prev / next tab | Alt+, / Alt+. | run / stop / restart | F5 / Shift+F5 / Ctrl+F5 |
| tab n | Alt+1..9 | toggle run panel | F4 |
| definition / back | F12 / Alt+Left | next / prev diagnostic | F8 / Shift+F8 |
| hover | Alt+K | complete | Alt+/ |
| toggle comment | Ctrl+/ (or Ctrl+7) | | |

**Config** at the OS config dir (`%APPDATA%\tome\config.toml`,
`~/.config/tome/config.toml`; `TOME_CONFIG` overrides):

```toml
theme = "hydra"
[theme_overrides]          # any theme colour: "#rrggbb", 0-255, or a name
[editor]
tab_width = 4
insert_spaces = true
[keys]                     # action = "key" or ["key", "key"]
save = "ctrl+s"
[lsp.python]               # one table per language id
command = "pyright-langserver"
args = ["--stdio"]
format_on_save = false
```

Project file `.tome.toml` at the project root:

```toml
[[run]]
name = "dev"
command = "npm run dev"
cwd = "."                  # optional, relative to the root
```

**Default language servers**: rust → `rust-analyzer`; go → `gopls`;
typescript / tsx / javascript / jsx → `typescript-language-server --stdio`;
python → `pyright-langserver --stdio`; html → `vscode-html-language-server --stdio`;
css → `vscode-css-language-server --stdio`; sql → `sqls`.

**Data**: backups in the OS data dir under `backups/`, one file per buffer, named by
a hash of the path (untitled buffers by a session id). No other stored state.

**Crates** (new): ratatui 0.30, crossterm 0.29 (`event-stream`), tokio, ropey 1.x,
unicode-width, unicode-segmentation, toml, serde, directories, arboard, ignore,
nucleo, regex, trash, tree-sitter 0.27 + tree-sitter-highlight and one grammar crate
per language, lsp-types, serde_json, ansi-to-tui, futures-util; windows-sys
(Windows) and libc (Unix) for process trees; dev: portable-pty, vt100, tempfile.

## Out of scope

- Plugins; an embedded interactive terminal; multi-cursor; git integration;
  more than two splits; soft wrap; LSP rename, code actions and signature help.

## Verification

- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
  green on Windows and Ubuntu in CI.
- At each phase exit, the phase's PTY harness tests pass against the real binary.
- Before calling v1 done, a person opens a real project in Windows Terminal (inside
  Hydra) and on Ubuntu, and uses every default key in the table once.

## Coverage

- R1 → #1, #2, #4
- R2 → #2, #55
- R3 → #4
- R4 → #5
- R5 → #3
- R6 → #6
- R7 → #7
- R8 → #7
- R9 → #8
- R10 → #9
- R11 → #10
- R12 → #11
- R13 → #12
- R14 → #13
- R15 → #13
- R16 → #14
- R17 → #15
- R18 → #16
- R19 → #17
- R20 → #18
- R21 → #19
- R22 → #20
- R23 → #21
- R24 → #22, #23, #24, #25, #26
- R25 → #22
- R26 → #27
- R27 → #28
- R28 → #29
- R29 → #30, #75
- R30 → #31
- R31 → #32
- R32 → #33, #76
- R33 → #34
- R34 → #35
- R35 → #36, #75
