# Spec: Splash screen and opening projects

Intent: [intent.md](intent.md) · Status: accepted · Date: 2026-10-08

## Concerns

- **Overrides the design handoff**: SPEC_V1_LAYOUT §4 (9a) says "No launch splash" and
  draws a key-list screen instead. The handoff is kept as delivered. Recommend: record the
  override here, in the note on v1 [R1](../tome-v1/spec.md), and drop "empty-editor key
  list" from the layout phase. Done in this change.
- **Ctrl+Enter is unreliable**: many Unix terminals send Ctrl+Enter as plain Enter.
  Recommend: "open this folder" is the browser's first row, reached with Enter; Ctrl+Enter
  is only a shortcut for it and no test depends on it working on Unix.
- **Switching projects is new for the language servers**: `Lsp` starts servers per
  (language, root) (`src/lsp/mod.rs:584`, `:726`) and can only shut every server down at
  exit (`finish`, `:990`). Recommend: a new call that shuts down one root's servers and
  forgets them, tested with the fake server; R29 (a missing or crashed server never blocks
  editing) must keep holding during and after a switch.
- **Splits are never empty** (`src/app.rs:314-332`): every split always holds a tab.
  Recommend: the splash is a flag on `App` drawn over the editor area while the only tab
  is the untouched untitled buffer, not an empty split. Opening a file already replaces an
  untouched untitled tab (`src/app.rs:1357`).

- **The splash design changed after Cody tried it** (2026-10-08): Cody's images
  [design/splash.png](design/splash.png) and [design/no-file-open.png](design/no-file-open.png)
  replace the "centred card" mockup, and bring back SPEC_V1_LAYOUT's 9a key list as the
  screen shown whenever no file is open (S11). Colours follow the theme's ramp, not the
  image's purple → teal.

## Requirements

- **S1.** `tome` and `tome <folder>` show the splash in the editor area; `tome <file>`
  opens the file with no splash — checked by PTY tests.
- **S2.** The splash fills the editor area with no card, as in
  [design/splash.png](design/splash.png), centred as one block: the `tome` wordmark in
  block letters 5 rows tall (half-block pixels), each column coloured along the theme's
  accent → accent2 ramp (`theme::grad`), on a soft glow (background cells blended toward
  `accent2`, fading with distance); a one-row rule in the same ramp, fading at both ends; a
  blank row; the project path with the home folder as `~`, its last folder in `strong` bold
  and the rest in `muted`, cut with `…` to fit; a blank row; three rows one blank row apart
  — *New file* · `in <path>` · `n`, *New directory* · `in <path>` · `d`, *Open directory* ·
  `choose a folder` · `o` (label in `text`, hint in `muted`, key right-aligned in `muted`).
  The selected row lies on the dialog glow with `✦` in `accent2`, its label `strong` bold
  and its key `accent` bold. With too little room the wordmark becomes `✦ tome` on one
  row; with less, the hints go, then the path. In `mono` there's no glow, the wordmark and
  rule are plain and the selected row is reverse video — checked by PTY tests (text,
  positions, colours, mono) and unit tests of each size step.
- **S3.** ↑ / ↓ move the selection (wrapping), Enter or the row's letter runs it, a click
  on a row runs it. Esc or Ctrl+N leaves the splash for the empty untitled buffer. Opening
  any file (Ctrl+P, the tree) replaces the splash. With the tree open the
  splash has focus at launch; Ctrl+E moves focus to the tree as usual. While the splash is
  up the tree is hidden (Ctrl+B or Ctrl+E shows it), the status bar shows the project path
  where it would say `untitled` and, on the right, `↑↓ select  ⏎ choose  q quit` and
  Tome's version in place of the cursor position and language, and `q` quits as Ctrl+Q
  does — checked by PTY tests.
- **S4.** *New file* opens the name prompt the tree's `a` uses, creates the file in the
  project folder, and opens it in a tab; Esc in the prompt returns to the splash; an
  invalid or existing name shows the existing error and creates nothing — checked by PTY
  tests.
- **S5.** *New directory* opens the name prompt the tree's `A` uses, creates the folder in
  the project folder, and opens it as the project (S7) — checked by PTY test.
- **S6.** The folder browser is a dimmed dialog card in the cast style (same width and top
  as the cast palette, lit `▀` edge): header `✦ <folder path>` with `open · folders`
  right-aligned in `muted`; a `line2` rule; first row `⏎ open <folder path>`; then the
  folder's sub-folders sorted case-insensitively, dot-folders last, scrolling past the
  card; the footer `⏎ into  ← up  ctrl+⏎ open here  esc cancel`. It starts in the current
  project folder. Typing filters the folder rows fuzzily (the first row stays). Enter on
  the first row opens that folder; Enter on a folder goes into it; ← or Backspace on an
  empty query goes up (nothing above a drive or `/` root). Typing a path that names an
  existing folder (absolute, or with a separator) and Enter jumps there. A folder that
  can't be read shows the reason in the status line and the browser stays where it was.
  Esc closes it, changing nothing. A click outside the card closes it — checked by PTY
  tests and unit tests of the listing.
- **S7.** Opening a folder makes it the project: every tab closes, the run panel's command
  is stopped, the old folder's language servers are shut down; the tree, the brand label,
  the Ctrl+P file list, project search, F5's `.tome.toml` and newly started language
  servers all use the new folder; Tome then shows the tree and an empty untitled pane,
  with the tree focused (no splash; changed 2026-10-08 after Cody tried it, #124) — checked by PTY tests (tree and brand show the new folder, Ctrl+P
  lists its files, F5 uses its `.tome.toml`) and a fake-server test (old server gets
  shutdown/exit, a file in the new folder starts a new one).
- **S8.** `>open directory` (title "Open directory") in the cast palette opens the same
  browser at any time; it has no default key and can be bound in `[keys]` as
  `open_directory` — checked by PTY test.
- **S9.** Opening a folder while tabs have unsaved changes first shows one confirm card
  "N files have unsaved changes" listing their names, with Save all / Discard / Cancel.
  Save all saves each (an untitled one asks for a name through save-as; cancelling that
  cancels the switch), then switches; Discard switches without saving; Cancel changes
  nothing. Before this card exists (S2–S8 tickets), opening a folder with unsaved tabs
  refuses with "save or close unsaved files first" in the status line — checked by PTY
  tests.
- **S11.** Whenever no file is open and the splash isn't up (after opening a folder, after
  closing the last tab), the editor area shows the key list of
  [design/no-file-open.png](design/no-file-open.png) and SPEC_V1_LAYOUT 9a, left-aligned
  in the upper third: `<project folder>/` in `strong` bold and `no file open` in `muted`,
  then rows two apart of key in `accent` bold and label in `text` — `Ctrl+P` cast · files
  and commands, `Ctrl+N` new file, `Ctrl+B` toggle tree, `Ctrl+Q` quit (keys read from the
  keymap, so a rebound key shows its new name). The tab bar shows no tab and typing does
  nothing; opening a file or Ctrl+N replaces it. In `mono` it's plain text — checked by PTY
  tests.
- **S10.** Every on-screen requirement has a PTY test; `cargo fmt --check && cargo clippy
  --all-targets -- -D warnings && cargo test` passes on Windows and Ubuntu.

## Design

- **Screens and states**:
  - Splash: S2, matching [design/splash.png](design/splash.png) (Cody, 2026-10-08; it
    replaced the "centred card" mockup). Header rows show no pills or thread while the
    splash is up.
  - No file open: S11, matching [design/no-file-open.png](design/no-file-open.png).
  - Folder browser: S6, matching the "cast-style folder browser" mockup chosen 2026-10-08.
  - Unsaved card: S9, in the confirm-card style of #92 (`src/ui/confirm.rs`).
- **Data**: none stored. The project folder stays owned by `Tree` (`src/workspace/tree.rs:21`),
  replaced with a new `Tree` on a switch; `App.project` (the brand label) is recomputed.
- **Interfaces**:
  - New `Action`s: `OpenDirectory` (global, title "Open directory", config name
    `open_directory`, no default key) and the splash's local actions (up, down, run,
    `n` / `d` / `o`, dismiss) bound in `src/keymap.rs`.
  - A way in `src/lsp/` to shut down and forget every server started for one root.
- **Reuses**:
  - `src/ui/mod.rs` — `dim`, `dialog_card`, `glow_row`, `enter_mark`, `footer`, for the
    browser and splash rows.
  - `src/ui/confirm.rs` — `Confirm`, `Choice`, for S9.
  - `src/app.rs` — `start_create` / `finish_name_prompt` (`:1720`, `:1768`), for S4–S5.
  - `src/ui/tree.rs:77-85` — the brand ramp (`theme::grad`), for the wordmark.
  - `src/run/tree.rs:113` — `ProcessTree::kill`, to stop the run on a switch.
  - `src/ui/picker.rs` — the cast palette's fuzzy matching and `>` command list
    (`Action::commands`, `src/keymap.rs:408`), for S6 and S8.

## Out of scope

- A recent-projects list; more than one project open at once; a file browser.
- Nested paths in the name prompt.
- Showing the splash again after closing the last tab.

## Verification

Every requirement's PTY or unit test passes in `cargo test` on Windows and Ubuntu CI.
Person check: at 160×45 in Windows Terminal, `tome` in an empty folder shows the splash
as the chosen mockup in `aurora` and `mono`; New file, New directory and Open directory
each work by key and by click; `>open directory` switches a project with an unsaved tab
through the card.

## Coverage

- S1 → #115
- S2 → #115, #126
- S3 → #115, #127
- S4 → #115
- S5 → #116
- S6 → #114
- S7 → #113, #114, #116, #124
- S8 → #114
- S9 → #114 (refusal), #117
- S11 → #128
- S10 → every ticket #113–#117, #124, #126–#128
