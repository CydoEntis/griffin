# Spec: Glyph Aurora

Intent: [intent.md](intent.md) · Status: accepted · Date: 2026-10-07

Design source: [design/README.md](design/README.md) (main frame, cast palette, themes,
restyle rules) wins over [design/SPEC_V1_LAYOUT.md](design/SPEC_V1_LAYOUT.md), which gives
geometry and copy for the other screens. Behaviour SPEC_V1_LAYOUT adds beyond what README
needs is Phase 11, not this spec.

## Concerns

- **The two design docs disagree on the main frame** (tree 28 fixed vs `clamp(W·0.19,22,36)`,
  brand row vs header row, no dividers vs `│`). Decided: README wins (2026-10-07).
- **Ctrl+Shift+P** (README §3) is taken by Windows Terminal (v1 spec Concerns). Decided:
  Ctrl+P only.
- **Alt+Enter never reaches Glyph in Windows Terminal** (SPEC_V1_LAYOUT §14.1). Decided:
  project replace all → Alt+A; R23 of the v1 spec changes.
- **Selected rows stop using reverse video.** v1 draws selection with `Theme::highlight`
  (REVERSED) and PTY tests detect it with `reversed_text`. Glow rows are bg colours, so
  tests move to `bg_at`; `mono` keeps REVERSED for selected rows (it has no ramps).
- **Many PTY tests assert old chrome text** (`"glyph  "` status prefix, `" N │ "` gutter,
  30-col tree, `"Go to file:"`, `"[S]ave [D]iscard"`). Each ticket updates the tests for the
  screen it changes, and only those.
- **Curly underline** (`SGR 4:3` + `58;2`) isn't a ratatui `Modifier`. Recommend: emit
  through ratatui's `underline-color` feature + `UNDERLINED`, and accept straight underlines
  where curly isn't available (README §1).
- **Truecolor**: required already; no fallback in this spec.

## Requirements

- **R1.** `Theme` carries the Aurora roles (README §4: `bg deep surface raised raised2
  line2 guide muted text fg strong accent accent2 acc_ink err warn ok info err_soft
  warn_soft info_soft sel cur_line gutter scrim` + `syn.*`). `aurora` and `moonlit` exist;
  the 10 Hydra pack themes derive their roles by README §4's table, and every theme's
  values equal README §4.1/§4.2 exactly. `[theme_overrides]` accepts every role (including
  `accent2`) and the v1 names as aliases (README §6). — unit tests in `src/theme.rs`.
- **R2.** `mix` (per-channel lerp, rounded) and `grad` (piecewise `mix`, `t` clamped) from
  README §1 are available to renderers. — unit tests in `src/theme.rs`.
- **R3.** Tree per README §2.1: 28 cols on `surface`, `✦ glyph` brand + project dir on row 1,
  rows from 3, indent guides, active-file glow row, dirty `•`, no divider. — PTY `tests/tree.rs`.
- **R4.** Tabs per README §2.2–2.3: row 0 blank, pills on row 1, aurora thread on row 2
  following the active pill, editor from row 3. Split per README §5.6: one `│` in `guide`,
  each split with its own pills and thread, unfocused thread at 35 %. — PTY `tests/tabs.rs`,
  `tests/split.rs`.
- **R5.** Status bar per README §2.5: glyph block (gradient bg, `✦ glyph` in `acc_ink`),
  path at x=20 (dir `muted`, name `strong`, ` •` `warn` when dirty), `Ln l, Col c` and
  diagnostic counts right-aligned; transient messages and the diagnostic under the cursor
  take the path slot until the next key. — PTY tests.
- **R6.** The status bar shows the language and the server state (ready `● ok` · starting
  `○ warn` · not found `○ muted` + `no server` · crashed `✕ err`), dropping server → language
  → directory as the row narrows (README §2.5). — PTY `tests/lsp.rs` with the fake server.
- **R7.** Editor per README §2.4: gutter `mark + number + 2 blanks` (no `│`), cursor-line
  glow and `accent` bold number in the focused split only, comments italic. — PTY
  `tests/editor_render.rs`.
- **R8.** Diagnostics per README §2.4: `◆` gutter mark in the worst severity, curly
  underline in `err`/`warn`/`info`, inline lens `◈ message` in `<sev>_soft` italic when it
  fits. — PTY `tests/lsp.rs`.
- **R9.** Selection keeps the syntax fg on `sel`; find matches use `find_match_bg =
  mix(bg, warn, .3)` and the current match is `bg / warn` (README §5.5, SPEC_V1_LAYOUT §4).
  — PTY `tests/find.rs`, `tests/editor_select.rs`.
- **R10.** Dimmed dialogs per README §3 + §5.1–5.2: every screen cell mixed 0.6 toward
  `scrim`, card on `raised` with no border and a lit `▀` top edge, input rows on `raised2`
  with a `✦` prompt, selected rows as the glow row with `⏎`. Project search uses it, with
  SPEC_V1_LAYOUT §7.3 geometry. — PTY `tests/project_search.rs`.
- **R11.** Ctrl+P opens the cast palette (README §3) listing files with fuzzy matching;
  Enter opens, Esc or a click outside closes. It replaces the "Go to file" picker. — PTY
  `tests/picker.rs`.
- **R12.** In cast, `>` lists commands (every global action with its bound key), `:N`
  Enter goes to line N (out of range shows `err`), `/text` Enter opens project search with
  `text`; with no prefix, files then commands. Ctrl+G opens cast prefilled with `:`.
  Footer per README §3 without `@`. — PTY tests.
- **R13.** Confirm cards per SPEC_V1_LAYOUT §7.4 (copy, keys) with README §5.4 buttons:
  default button on a `grad(accent, accent2)` bg in `acc_ink` bold; letter, ← → / Tab,
  Enter, Esc and click work. — PTY `tests/save.rs`, `tests/backup.rs`,
  `tests/project_replace.rs`.
- **R14.** Project replace all is Alt+A (was Alt+Enter); the find bar's Alt+A is unchanged.
  — keymap unit test + PTY `tests/project_replace.rs`.
- **R15.** Find, replace and prompt bars per README §5.5 + SPEC_V1_LAYOUT §8 geometry. —
  PTY `tests/find.rs`, `tests/replace.rs`, `tests/tree_ops.rs`.
- **R16.** Hover and completion keep a border, `╭─╮│╰╯` in `line2` on `raised`, hover rule
  `├─┤`; completion's selected row is the glow row (README §5.3). — PTY `tests/lsp.rs`.
- **R17.** Run panel title row per README §5.7 + SPEC_V1_LAYOUT §9 (glyph, name, state word,
  command, hints), ANSI colours mapped to roles, restart marker in `muted`; the F5 run picker
  uses the R10 card. — PTY `tests/run.rs`.
- **R18.** `mono` draws every screen above with flat `accent`, bold active states, REVERSED
  selected rows, DIM for the scrim. — PTY `tests/theme.rs`.

## Design

- **Screens and states**: as cited per requirement. Reference size 160×45; tests run at
  100×30 (harness `COLS`/`ROWS`).
- **Data**: none stored. New theme roles only.
- **Interfaces**: config `theme = "aurora" | "moonlit" | <pack>`; `[theme_overrides]` gains
  the Aurora role names. New actions: none for cast's modes beyond `GoToFile` (Ctrl+P) and
  `GoToLine` (Ctrl+G, now opens cast with `:`).
- **Reuses**: `src/theme.rs` `mix`; `src/ui/picker.rs` nucleo matching; `src/ui/search.rs`
  `ProjectSearch`; `Action::ALL` + `Action::name` for the command list.

## Out of scope

- Everything listed under the intent's Out of scope.

## Verification

- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` green on
  Windows and Ubuntu CI.
- Person check: open a project at 160×45 in Windows Terminal with `theme = "aurora"`, and
  compare the main frame and the cast palette against `design/Glyph Aurora Themes.dc.html`;
  repeat with `moonlit`, `hydra`, `mono`.

## Coverage

- R1, R2 → #80 · R3 → #82 · R4 → #83 · R5 → #84 · R6 → #85 · R7 → #86 · R8 → #87 · R9 → #88 ·
  R10 → #89 · R11 → #90 · R12 → #91 · R13 → #92 · R14 → #81 · R15 → #93 · R16 → #94 ·
  R17 → #95 · R18 → every ticket from #82 on (each keeps `mono` drawing its screen)
