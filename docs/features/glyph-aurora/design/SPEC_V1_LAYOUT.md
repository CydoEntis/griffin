# Griffin — TUI design spec (v1)

The spec an engineer matches cell for cell. Companion files:

- **Griffin Prototype.dc.html** — interactive cell grid. Scenario ids (1a, 6d…) below refer to its list. Hover any cell for its coordinates, region, fg/bg role, hex and modifiers. Click the grid and the default keymap works.
- **Griffin Themes.dc.html** — all 11 themes side by side + every role's value.
- **griffin-engine.js** — the reference renderer the prototype runs. Where this prose is ambiguous, its layout maths is the answer. **griffin-themes.js** — palettes copied from `src/theme.rs` plus the new roles.

---

## 0. Conventions

- Cells are `(x, y)`, 0-based from the top-left; terminal is `W×H`; ranges are half-open.
- Styles are written `fg / bg +mods`, by theme role. `syn.*` = syntax roles.
- **Filled** highlights (active tab, selected rows, selection) keep going through `Theme::highlight` (REVERSED over swapped colours) so `mono` and the PTY tests still work. The prototype resolves them to plain colours.
- Glyphs (all render in Windows Terminal with Cascadia Mono): `│ ─ ┌ ┐ └ ┘ ├ ┤ ▸ ▾ ● ○ ■ ✓ ✕ ⚠ ‹ › ⏎ ↑ ↓ … ·`. No other box/block glyphs.
- Cursor: the hardware cursor (`set_cursor_position`), shape left to the terminal. The prototype draws it as a 2-px bar.
- Diagnostics: curly underline (`SGR 4:3` + `58;2;r;g;b`). Terminals without it show a straight underline; that's fine.

## 1. Frame (every screen)

```
row 0              tree header │ tab bar per split            sidebar_bg
rows 1 … M−1       tree        │ editor [│ editor]
rows M … M+R−1     run panel: title row + output             when visible, full width
row H−2            find / prompt bar                          when open
row H−1            status line
```

| | 100×30 | 160×45 | 220×60 |
|---|---|---|---|
| tree width `clamp(round(W·0.19), 22, 36)` | 22 | 30 | 36 |
| editor width, no split | 77 | 129 | 183 |
| split panes (left + right) | 49 + 50, tree hidden | 64 + 64 | 91 + 91 |
| run panel rows `clamp(round(H·0.3), 6, 18)` | 9 | 14 | 18 |
| editor text rows, run panel open | 19 | 29 | 40 |
| editor text rows, nothing open | 28 | 43 | 58 |

- Tree divider at `x = treeW`: `│ line / bg` (row 0: `line / sidebar_bg`, so the chrome row reads as one strip).
- Editors start at `x0 = treeW + 1` (or 0 with the tree hidden). Split: `leftW = floor((W − x0 − 1) / 2)`, divider at `x0 + leftW`, the right split gets the rest.
- **The tree hides while split if `W < 120`.** Ctrl+B still shows it.
- Under **100×30**: fill `bg`, `Griffin needs at least 100×30` in `strong` bold, centred on row `floor(H/2) − 1`, then `This terminal is W×H` in `muted`. Only Ctrl+Q works. (9g)

## 2. Tab bar — row 0 of each split (1a, 3a)

- Fill `muted / sidebar_bg`. Each label is ` name ` plus `● ` when dirty, packed from the split's x0.
- Inactive: `muted`. Its `●` is `working`.
- Active, focused split: `tab_active_fg / tab_active_bg` bold (its `●` too).
- Active, unfocused split: `strong` + underline. That way only one filled tab is on screen, and it shows where keys go.
- **New:** the same file name twice in one split shows ` name · parent `.
- Overflow: tabs drop off the left until the active one fits (as today). **New:** ` ‹N ` in `muted` at x0 says N tabs are hidden.
- Mouse: click activates and focuses the split. Middle-click closes (dirty guard).

## 3. Tree (1a, 1b, 9c)

- Fill `text / sidebar_bg`. Row 0 is a header (**new**): the project folder name in capitals at x=1, `muted` bold, `accent` bold while the tree has focus.
- Rows 1+: `x = 1 + depth·2`. Folders get `▾ ` / `▸ ` in `muted`, files two spaces. Then the name in `text`, cut with `…` at `treeW − 2`. Column `treeW − 2` shows `● working` when the file has unsaved changes.
- Selected row: full width on `hov`. `strong` bold while focused, `text` otherwise. (Today it's DIM on reverse, which renders differently in each terminal.)
- Mouse hover: full width on `card`.
- The file in the focused split's active tab has its name in `accent` (unless that row is selected).
- Ignored files are never listed: no ghost rows, no count. Folders come first, then files, case-insensitive.
- Empty project: `No files yet` in `text` at (2,2), then `a` / `A` in `strong` bold at x=2 with `new file` / `new folder` in `muted` at x=5, on rows 4 and 5.
- Keys: ↑ ↓, → expand, ← collapse or go to the parent, Enter opens a file or toggles a folder, `a` new file, `A` new folder, `r` rename, `d` trash. Ctrl+E focus, Ctrl+B toggle. Mouse: click selects, then opens the file or toggles the folder. The wheel scrolls.

## 4. Editor (1a, 2a–2c, 4a, 9d)

**Gutter**, width `D + 3` with `D = max(3, digits(line count))`. This replaces ` N │ `:

| cells | content |
|---|---|
| x0 | diagnostic mark `●` in the most severe colour on that line (`err`, `working`, `info`), else blank |
| x0+1 … x0+D | line number right-aligned, `gutter_fg` |
| x0+D+1 … x0+D+2 | blank |
| x0+D+3 … | text |

- **Current line** (focused split only): `current_line_bg` across the whole row, gutter included. The number is `gutter_active_fg` bold; the lines around it stay `gutter_fg`. In the unfocused split there's no band, the number is `text`, and there's no cursor. (2a, 3a)
- Syntax: `syn.*` fg. `mono` adds bold or dim (see the themes sheet).
- **Selection**: `selection_bg` behind, **syntax fg kept** (today fg is forced to `fg`). A selected line break shows as one `selection_bg` cell past the end of the line. (2b)
- **Find matches**: `find_match_bg`, fg unchanged. The current match is `find_current_fg / find_current_bg`. (2c)
- **Diagnostics**: curly underline in `err`, `working` or `info`, plus the gutter mark. (4a)
- **Horizontal scroll** (no wrap): with `left > 0`, the first text column shows `‹ muted` on lines that have text. A line running past the right edge shows `› muted` in the last column. Scrolling keeps 6 columns of context. (9d)
- With no buffer open (9a): `griffin` in `strong` bold + `no file open` in `muted`, centred, then key/label pairs every second row: key in `accent` bold, label in `text` at +10. Copy: Ctrl+P go to file · Alt+F search the project · Ctrl+N new file · Ctrl+B toggle tree · F5 run a command · Ctrl+Q quit. **No launch splash.** This screen only shows when there's nothing to edit.
- Mouse: click places the cursor, drag selects, double-click selects a word, the wheel scrolls 3 lines, Ctrl+click goes to the definition.

## 5. Status line — row H−1 (all)

Fill `text / sidebar_bg`. The message is left-aligned from x=1. The rest is right-aligned, ending at `W − 2`, with two spaces between segments:

```
✕ mismatched types: expected u16, found usize        src/ui/status.rs ●  Ln 52, Col 22  Rust  ● rust-analyzer  ⚠ 1  ✕ 2
```

| segment | style |
|---|---|
| path | directory `muted`, file name `strong`, ` ●` `working` when dirty |
| position | `Ln l, Col c` in `text`, 1-based |
| language | `text` |
| server | ready `● done` + name `text` · starting `○ working` + name · not found `○ muted` + `no server` `muted` · crashed `✕ err` + name `err`. Left out for languages without a server. |
| counts | only when > 0: `⚠ n working`, `✕ n err` |

- What the message slot shows, highest priority first: a transient message (save, replace, server events) → the diagnostic under the cursor (severity glyph in its colour + message in `text`) → blank. Success messages start with `✓ done`.
- When the row is short, keeping at least 24 cells for the message: drop the server, then the language, then shorten the path to the file name, then cut the message with `…`.
- The leading `griffin ` goes.
- With no buffer, the right side shows the project root in `muted`.

## 6. Popups — anchored at the cursor, no dimming (5a–5d)

- Placement is the existing `hover::anchor` (below, flips above, shifts left, otherwise the roomier side cut to fit). Bounds: full width, rows 0 up to (bar or status) − 1. The cursor cell is never covered.
- Card: fill `card`, border `┌─┐│└┘` in `border / card`, one cell of padding inside.
- **Hover**: the code block highlighted with `syn.*`, a `├──┤` rule in `border`, then the docs wrapped in `text`. Text is at most 60×12.
- **Completion**: each row is the kind (`muted`, padded to the longest kind), a space, the label (`fg`, with the typed prefix in `accent` bold), then the detail right-aligned in `muted`. The selected row is `hov` across the inner width, label `strong`. At most 10 rows. **New:** anchor `x = cursorX − typed − kindW − 3`, so labels line up under the word being typed.
- Keys: ↑ ↓ select, Enter/Tab accept, Esc dismisses, typing filters. Any key closes hover. Clicking elsewhere closes both.

## 7. Dialogs — centred cards over a dimmed screen (6a–6i, 8f)

- **Dim**: every cell on screen gets fg and bg mixed 60% towards `scrim` (`mono`: the DIM modifier). The hardware cursor moves into the dialog. Clicking outside the card = Esc.
- **Card**: fill `card`, **no border** (**new**: dialogs drop the `Block` border; popups keep theirs because nothing dims behind them).
- **Input-card pattern**: row 0 is a field on `card2` — ` Label  value▏`, label `text`, value `strong`. Then the body. The last row has footer hints in `muted` at x+2, with a count right-aligned.

**7.1 Go to file** (Ctrl+P; 6a, 6b). `w = clamp(floor(W·3/5), 40, 80)`, `h = min(20, H − 4)`, `x = floor((W − w)/2)`, `y = floor((H − h)/2)`. Rows start at x+2: directory `muted` (`text` on the selected row), file name `strong`. Matched letters are `accent` bold; on the selected row they become `acc_ink / accent`. The selected row is `hov` full width. The list scrolls just enough to keep the selection on the last row. Footer: `↑↓ select   ⏎ open   esc close`, right `N of total`. No matches: `no matching files` in `muted`. Before the walk returns: `listing files…`.

**7.2 Go to line** (Ctrl+G; 6c). `w = 44`, `h = 2`. Field `Go to line`, digits only. Footer `⏎ go   esc cancel`, right `1–N` in `muted`. Out of range: the value and the range turn `err`.

**7.3 Project search** (Alt+F; 6d, 6e). `w = clamp(W − 20, 60, 140)`, `h = clamp(H − 8, 14, 40)`.
- Row 0: `Search ` field, with the `Aa` / `.*` chips right-aligned (see 8).
- Row 1: `Replace` field. Empty and unfocused, it shows `tab to replace` in `muted`.
- Row 2: `N matches in M files` in `muted`, or `invalid regex` in `err`.
- Rows 3 … h−2: results. The path column is `min(34, longest "path:line")` wide: directory `muted`, file name `strong`, `:line` `muted`. Then 2 spaces and the line's text from its first non-blank character, scrolled so the match shows (`…` in front when cut). The match is on `find_match_bg`; on the selected row (`hov`) it's `find_current_*`. With replace text: the match in `err` underlined, with the replacement right after it in `done` bold.
- Footer: `↑↓ select   ⏎ open   tab replace field   alt+a replace all   esc close`.
- Keys: Tab switches field, ↑ ↓, Enter opens the hit, Alt+C, Alt+R, **Alt+A → confirm** (instead of Alt+Enter, see Notes 1).

**7.4 Confirm cards** (6f–6i). `w = max(44, longest line + 8)`, `h = 7`. The question is `strong` bold at (x+3, y+1), the explanation `muted` at (x+3, y+2). Buttons on row y+4 from x+3, each ` Label `, 2 cells apart. The default (first) button is `tab_active_fg / tab_active_bg` bold; the others `strong / btn`. The key letter is underlined. `esc` in `muted`, right-aligned. Keys: the letter answers, ← → / Tab move between buttons, Enter presses, Esc cancels. Click works too.

| kind | question | explanation | buttons (key) |
|---|---|---|---|
| replace | `Replace N matches in M files?` | `Open buffers are edited in place (undo with Ctrl+Z); other files are saved.` | Replace (r) · Cancel (c) |
| unsaved | `<name> has unsaved changes` | `Closing discards them unless you save.` | Save (s) · Discard (d) · Cancel (c) |
| recover | `Recover unsaved changes to <name>?` | `A backup from HH:MM is newer than the file on disk.` | Recover (r) · Discard (d) |
| trash | `Move <name> to the trash?` | `You can restore it from the system trash.` | Move to trash (y) · Cancel (n) |

## 8. Bars — row H−2 on `card2` (2c, 7a–7e)

- **Find** (Ctrl+F): field `Find` from x=0 up to the chips. Chips, right-aligned: ` Aa `, space, ` .* `, 2 spaces, count, space. A chip that's on is `acc_ink / accent` bold; off, it's `muted`. The count is `strong`: `n/m`, `0/0`, or blank while the field is empty. A bad pattern makes the count `invalid regex` in `err` and the pattern text `err` too.
- **Find + replace** (Ctrl+R): Find covers `[0, floor(W/2))`, `│ line` sits at `floor(W/2)`, and Replace runs up to the chips. The field with focus shows the cursor and its label in `text`; the other label is `muted`. Enter replaces the current match and moves on, Alt+A replaces all (one undo step), Tab switches field.
- **Prompt** (new file / folder / rename / save as): ` Label  value▏`, with the hint right-aligned in `muted` ending at W−2: `⏎ create   esc cancel` / `⏎ rename   esc cancel`. New file is prefilled with the selected folder. **New:** rename preselects the stem (`selection_bg`), so typing keeps the extension.

## 9. Run panel (8a–8g)

- Title row on `sidebar_bg`: at x=1 the status glyph + space, the name in `strong` bold (`accent` while the panel has focus), 2 spaces, the state word in the state colour, 3 spaces, the command in `muted` (cut to fit). Hints right-aligned in `muted`.

| state | glyph · word | role | hints |
|---|---|---|---|
| running | `●` running | working | `shift+F5 stop   ctrl+F5 restart   F4 hide` |
| exited 0 | `✓` exited 0 | done | `F5 run again   F4 hide` |
| exited n | `✕` exited n | err | same |
| killed by a signal | `✕` exited | err | same |
| stopped | `■` stopped | idle | same |

- Body on `bg` from x=1: the latest lines (tail-follow), bold kept. ANSI colours map onto roles, so program output matches the theme: red→`err`, green→`done`, yellow→`working`, blue→`syn.function`, magenta→`syn.keyword`, cyan→`syn.type`, white→`strong`, black / bright black→`muted`. 256-colour and RGB pass through.
- Restart: earlier output stays, under a rule `── restarted HH:MM:SS ───…` in `muted` dim across the panel.
- Before anything has run: title `run`, then `F5 runs a command from .griffin.toml` in `muted` at (2, row 1).
- **Command picker** (F5 when there's more than one): the 7.1 card titled `Run`, `h = min(8, H − 4)`. Each row: name (matched letters `accent`), command in `muted` at x+14, source (`.griffin.toml` / `detected`) right-aligned in `muted`.

## 10. Edge states

| id | state | what shows |
|---|---|---|
| 9a | `griffin .`, nothing open | tree + the key list in the editor (4) |
| 9b | `griffin` alone | no tree, tab ` untitled-1 `, one empty line, `Plain text`, no server segment |
| 9c | empty project | tree help (3) + the key list in the editor |
| 9d | file too wide | `‹` / `›` markers (4) |
| 9e | server not found | message `⚠ rust: server not found (rust-analyzer)` once; the server segment says `○ no server`. Editing carries on. |
| 9f | server crashed | message `✕ rust: server crashed (exit code 101)` in `err`; segment `✕ rust-analyzer` in `err`; diagnostics cleared |
| 9g | below 100×30 | (1) |

## 11. Resizing (10a–10c; or type any W×H in the prototype)

Everything comes from section 1's formulas on every resize. Nothing is remembered per size. In order, as the terminal shrinks:
1. The editor loses width first. The tree keeps 19% (at least 22).
2. Split under 120 cols: the tree hides.
3. The status line drops server → language → directories → cuts the message.
4. Find+replace keeps its halves. The `Replace` field shrinks first; the chips never do.
5. Popups cut to the roomier side. Dialogs clamp to `W − 2 × H − 2`.
6. Under 100×30: the size message (9g).

Growing, the tree stops at 36 and the run panel at 18 rows; the rest goes to the editors. The viewport keeps following the cursor after a resize.

## 12. Keys

The spec's default keymap is unchanged except **project replace all: Alt+A** (was Alt+Enter). The prototype implements: Ctrl+P, Ctrl+G, Ctrl+F, Ctrl+R, Alt+F, Alt+C/R/A, Ctrl+B, Ctrl+E, Alt+V, F6, F4, F5, Shift+F5, Ctrl+F5, Alt+K, Alt+/, F8/Shift+F8, Ctrl+S, Ctrl+W, Ctrl+N, Ctrl+Q, Alt+, / Alt+. / Alt+1–9, arrows/Home/End/PgUp/PgDn (+Shift), typing, Enter (keeps indent), Backspace, Delete, Ctrl+A. The browser may keep Ctrl+W/N/T.

## 13. Theme roles

New roles — all derived, so the existing `design()` / `classic()` tables don't change, and each one can be overridden in `[theme_overrides]`:

| role | derivation (non-mono) | mono |
|---|---|---|
| `current_line_bg` | `mix(bg, fg, 0.06)` | `Reset` (the number's weight carries it) |
| `gutter_fg` | `mix(muted, bg, 0.30)` | `DarkGray` |
| `gutter_active_fg` | `strong` | `White` |
| `find_match_bg` | `mix(bg, working, 0.30)` (light themes 0.32) | `DarkGray` + underline (same as selection otherwise) |
| `find_current_bg` | `working` | `Yellow` |
| `find_current_fg` | `bg` | `Black` |
| `info` | `syn.function` | `Blue` |
| `scrim` | dark themes `mix(bg, #000, 0.5)`; light `mix(bg, fg, 0.3)`; dim = mix 0.6 towards it | DIM modifier |

The brief's names and the code's are aliases: `working` = `warning`, `done` = `ok`, `idle` = `muted`. Pick one canonical set (Notes 12).

### All roles × all themes

`mono` shows terminal colour names (hex = Windows Terminal Campbell preview).

| role | hydra | papercolor-dark | tango-dark | monokai | tokyo-night | catppuccin-mocha | catppuccin-latte | gruvbox | nord | dracula | mono |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `bg` | `#070b10` | `#1c1c1c` | `#2e3436` | `#272822` | `#1a1b26` | `#1e1e2e` | `#eff1f5` | `#282828` | `#2e3440` | `#282a36` | Reset |
| `fg` | `#c9d1d9` | `#d0d0d0` | `#eeeeec` | `#f8f8f2` | `#c0caf5` | `#cdd6f4` | `#303446` | `#ebdbb2` | `#eceff4` | `#f8f8f2` | Reset |
| `muted` | `#71808f` | `#808080` | `#9a9c97` | `#8f8a72` | `#7a82ad` | `#6c7086` | `#6c6f85` | `#928374` | `#8390a8` | `#7a8ac0` | DarkGray |
| `accent` | `#c3f53c` | `#00afaf` | `#8ae234` | `#a6e22e` | `#7aa2f7` | `#89b4fa` | `#7a2fd8` | `#fabd2f` | `#a3be8c` | `#bd93f9` | White |
| `border` | `#1f2c3a` | `#444444` | `#555753` | `#49483e` | `#292e42` | `#45475a` | `#bcc0cc` | `#504945` | `#434c5e` | `#44475a` | DarkGray |
| `border_active` | `#c3f53c` | `#00afaf` | `#8ae234` | `#a6e22e` | `#7aa2f7` | `#89b4fa` | `#7a2fd8` | `#fabd2f` | `#a3be8c` | `#bd93f9` | White |
| `sidebar_bg` | `#0c131b` | `#262626` | `#252a2b` | `#1e1f1c` | `#16161e` | `#181825` | `#e6e9ef` | `#1d2021` | `#272c36` | `#21222c` | Reset |
| `selection_bg` | `#2a3a4c` | `#5f5faf` | `#204a87` | `#55544a` | `#3b4261` | `#313244` | `#ccd0da` | `#3c3836` | `#3b4252` | `#44475a` | DarkGray |
| `tab_active_bg` | `#c3f53c` | `#00afaf` | `#8ae234` | `#a6e22e` | `#7aa2f7` | `#89b4fa` | `#7a2fd8` | `#fabd2f` | `#a3be8c` | `#bd93f9` | White |
| `tab_active_fg` | `#0a1204` | `#1c1c1c` | `#2e3436` | `#272822` | `#1a1b26` | `#1e1e2e` | `#eff1f5` | `#282828` | `#2e3440` | `#282a36` | Black |
| `card` | `#0f1821` | `#303030` | `#363c3e` | `#2f302a` | `#1f2335` | `#272738` | `#e5e8ec` | `#32312f` | `#383d49` | `#32343f` | Black |
| `card2` | `#18242f` | `#3a3a3a` | `#41474a` | `#3e3d32` | `#292e42` | `#313244` | `#dadce2` | `#3d3c37` | `#434954` | `#3f414b` | DarkGray |
| `btn` | `#1d2a37` | `#3a3a3a` | `#4a5052` | `#3e3d32` | `#292e42` | `#37384a` | `#d4d7dd` | `#43413b` | `#494e59` | `#454750` | DarkGray |
| `hov` | `#2a3a4c` | `#5f5faf` | `#204a87` | `#55544a` | `#3b4261` | `#313244` | `#ccd0da` | `#3c3836` | `#3b4252` | `#44475a` | DarkGray |
| `line` | `#1f2c3a` | `#444444` | `#555753` | `#49483e` | `#292e42` | `#45475a` | `#bcc0cc` | `#504945` | `#434c5e` | `#44475a` | DarkGray |
| `text` | `#a7b4c2` | `#b2b2b2` | `#d3d7cf` | `#cfcfc2` | `#a9b1d6` | `#9da3bd` | `#4e5266` | `#bfaf93` | `#b8c0ce` | `#b9c1d9` | Gray |
| `strong` | `#f2f6f8` | `#eeeeee` | `#eeeeec` | `#f8f8f2` | `#e0e6ff` | `#cdd6f4` | `#303446` | `#ebdbb2` | `#eceff4` | `#f8f8f2` | White |
| `acc_ink` | `#0a1204` | `#1c1c1c` | `#2e3436` | `#272822` | `#1a1b26` | `#1e1e2e` | `#eff1f5` | `#282828` | `#2e3440` | `#282a36` | Black |
| `err` | `#ff6b6b` | `#ff5f87` | `#ff5c5c` | `#f92672` | `#f7768e` | `#f38ba8` | `#d20f39` | `#fb4934` | `#e0707a` | `#ff5555` | Red |
| `working` | `#ffb547` | `#ffaf00` | `#fcaf3e` | `#fd971f` | `#e0af68` | `#f9e2af` | `#8a5a00` | `#fe8019` | `#ebcb8b` | `#f1fa8c` | Yellow |
| `done` | `#7fd962` | `#5faf00` | `#73d216` | `#a6e22e` | `#9ece6a` | `#a6e3a1` | `#2f8a1f` | `#b8bb26` | `#a3be8c` | `#50fa7b` | Green |
| `idle` | `#71808f` | `#808080` | `#9a9c97` | `#8f8a72` | `#7a82ad` | `#6c7086` | `#6c6f85` | `#928374` | `#8390a8` | `#7a8ac0` | DarkGray |
| `current_line_bg` **new** | `#13171c` | `#272727` | `#3a3f41` | `#34342e` | `#242632` | `#29293a` | `#e4e6eb` | `#343330` | `#393f4b` | `#343641` | Reset |
| `gutter_fg` **new** | `#515d69` | `#626262` | `#7a7d7a` | `#706d5a` | `#5d6385` | `#55576c` | `#9396a7` | `#72685d` | `#6a7489` | `#616d97` | DarkGray |
| `gutter_active_fg` **new** | `#f2f6f8` | `#eeeeee` | `#eeeeec` | `#f8f8f2` | `#e0e6ff` | `#cdd6f4` | `#303446` | `#ebdbb2` | `#eceff4` | `#f8f8f2` | White |
| `find_match_bg` **new** | `#513e21` | `#604814` | `#6c5938` | `#674921` | `#55473a` | `#605955` | `#cfc1a7` | `#684224` | `#676157` | `#646850` | DarkGray |
| `find_current_bg` **new** | `#ffb547` | `#ffaf00` | `#fcaf3e` | `#fd971f` | `#e0af68` | `#f9e2af` | `#8a5a00` | `#fe8019` | `#ebcb8b` | `#f1fa8c` | Yellow |
| `find_current_fg` **new** | `#070b10` | `#1c1c1c` | `#2e3436` | `#272822` | `#1a1b26` | `#1e1e2e` | `#eff1f5` | `#282828` | `#2e3440` | `#282a36` | Black |
| `info` **new** | `#5aa9ff` | `#5fafd7` | `#729fcf` | `#a6e22e` | `#7aa2f7` | `#89b4fa` | `#1e66f5` | `#8ec07c` | `#88c0d0` | `#50fa7b` | Blue |
| `scrim` **new** | `#040608` | `#0e0e0e` | `#171a1b` | `#141411` | `#0d0e13` | `#0f0f17` | `#b6b8c1` | `#141414` | `#171a20` | `#14151b` | DIM modifier |
| `syn.keyword` | `#a593ff` | `#ff5faf` | `#ad7fa8` | `#f92672` | `#bb9af7` | `#cba6f7` | `#8839ef` | `#fb4934` | `#81a1c1` | `#ff79c6` | Reset + BOLD |
| `syn.string` | `#7fd962` | `#d7af5f` | `#8ae234` | `#e6db74` | `#9ece6a` | `#a6e3a1` | `#40a02b` | `#b8bb26` | `#a3be8c` | `#f1fa8c` | Reset |
| `syn.comment` | `#71808f` | `#808080` | `#888a85` | `#75715e` | `#565f89` | `#9399b2` | `#7c7f93` | `#928374` | `#8390a8` | `#7a8ac0` | Reset + DIM |
| `syn.function` | `#5aa9ff` | `#5fafd7` | `#729fcf` | `#a6e22e` | `#7aa2f7` | `#89b4fa` | `#1e66f5` | `#8ec07c` | `#88c0d0` | `#50fa7b` | Reset + BOLD |
| `syn.type` | `#3dd6c0` | `#af87d7` | `#34e2e2` | `#66d9ef` | `#7dcfff` | `#f9e2af` | `#df8e1d` | `#fabd2f` | `#8fbcbb` | `#8be9fd` | Reset + BOLD |
| `syn.number` | `#ffb547` | `#ffaf00` | `#fcaf3e` | `#ae81ff` | `#ff9e64` | `#fab387` | `#fe640b` | `#d3869b` | `#b48ead` | `#bd93f9` | Reset |
| `syn.constant` | `#ffb547` | `#ffaf00` | `#fcaf3e` | `#ae81ff` | `#ff9e64` | `#fab387` | `#fe640b` | `#d3869b` | `#b48ead` | `#bd93f9` | Reset + BOLD |
| `syn.operator` | `#ff7ab6` | `#00afaf` | `#fce94f` | `#f92672` | `#89ddff` | `#89dceb` | `#04a5e5` | `#fe8019` | `#81a1c1` | `#ff79c6` | Reset |
| `syn.punctuation` | `#a7b4c2` | `#b2b2b2` | `#d3d7cf` | `#cfcfc2` | `#a9b1d6` | `#9399b2` | `#7c7f93` | `#a89984` | `#d8dee9` | `#f8f8f2` | Reset + DIM |
| `syn.variable` | `#c9d1d9` | `#d0d0d0` | `#eeeeec` | `#f8f8f2` | `#c0caf5` | `#cdd6f4` | `#303446` | `#ebdbb2` | `#d8dee9` | `#f8f8f2` | Reset |
| `syn.property` | `#e8c565` | `#5f8787` | `#e9b96e` | `#fd971f` | `#73daca` | `#b4befe` | `#7287fd` | `#83a598` | `#88c0d0` | `#ffb86c` | Reset |
| `syn.tag` | `#ff7ab6` | `#ff5f87` | `#ef2929` | `#f92672` | `#f7768e` | `#f38ba8` | `#d20f39` | `#fe8019` | `#81a1c1` | `#ff79c6` | Reset + BOLD |
| `syn.attribute` | `#e8c565` | `#d7af5f` | `#fce94f` | `#a6e22e` | `#e0af68` | `#f9e2af` | `#df8e1d` | `#fabd2f` | `#8fbcbb` | `#50fa7b` | Reset + DIM |


## 14. Notes — current behaviour worth changing before building

1. **Alt+Enter never reaches Griffin in Windows Terminal**: it's WT's default fullscreen toggle. Project replace all → **Alt+A**, which is already the find bar's replace-all.
2. **Find matches use the selection colours.** You can't tell a match from a selection, and the current match looks like the rest. → `find_match_bg` / `find_current_*`.
3. **Selection forces fg to `fg`** and drops syntax colour. Keep the syntax fg on `selection_bg`.
4. **Gutter ` N │ `** puts a divider inside a pane (Hydra: a single `│` between panes only) and costs a column. → mark + number + 2 spaces, with the current line number popping instead.
5. **Status line** starts with `griffin` (9 cells on every screen) and has no language or LSP state, which the spec asks for. Reordered per section 5, with a drop order.
6. **The unfocused tree selection is DIM on reverse video.** Windows Terminal, Ubuntu's VTE and others render that differently. → `hov` bg + `text` fg.
7. **Dialogs are drawn in a bordered `Block`.** Over a dimmed screen the border is the kind of box Hydra avoids. Drop it for dialogs; keep it for popups, which sit on an undimmed screen.
8. **Confirms are one line, `[S]ave [D]iscard [C]ancel`**, with no default and Enter doing nothing. → buttons with a default (Enter), ← →, and the letters still working.
9. **Tab overflow drops tabs silently**, and two `mod.rs` tabs look identical. → `‹N` marker and ` · parent`.
10. **The tree is a fixed 30 cols.** At 100×30 with a split, that leaves 34-col panes (28 cols of text). → proportional, and it hides while split under 120.
11. **Completion is anchored at the cursor**, so labels sit right of the word and the kind column pushes them further. → shift the card left so labels line up with the word.
12. **Role names drift**: code `warning`/`ok`, brief `working`/`done`/`idle`. Pick one canonical set and keep the others as override aliases (today's `slot()` already does this).
13. **`mono` cards are `Black`**, which equals the background in most terminal schemes. Dialogs are then told apart only by DIM. Consider keeping the border on dialogs for `mono` only.
14. **The run panel only shows the tail.** There's no way to read back past the panel height. Suggest wheel / PgUp scrolling while it's focused (follow resumes at the bottom).
15. **Ctrl+F5 / Shift+F5** are fine in WT and VTE, but tmux and older xterm modes don't send distinct modified F-keys. Worth an unmodified fallback (e.g. F7 stop) in the default map.
