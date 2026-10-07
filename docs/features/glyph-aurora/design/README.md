# Handoff: Glyph — "Aurora" terminal editor (2a)

## Overview
**Glyph** (formerly Griffin) is a non-modal terminal (TUI) code editor in Rust + ratatui, at github.com/CydoEntis/griffin. The chosen visual direction is **2a "Aurora · Float"**: obsidian-dark chrome with a two-colour aurora light (accent → accent2), drawn with per-cell 24-bit colour ramps. It also takes **2b's solid status bar** with the glowing ✦ glyph block. Two signature themes (**Aurora** violet→teal, **Moonlit** moon→gold) ship alongside the 10 Hydra palettes as a theme pack, with the aurora colours derived from each theme.

## About the design files
The files in this bundle are **design references built in HTML**: prototypes that render the exact terminal cell grid (one character per cell, real colours). They are not production code. The task is to **recreate them in the Rust/ratatui codebase** with its existing patterns (`Theme`, `ui/*` widgets, `Buffer::set_string` / `Cell` styling). The JS renderers (`glyph-magic.js`, `griffin-engine.js`) are the source of truth for layout maths and colour ramps: port the formulas, not the code.

Open the `.dc.html` files in a browser (they load the sibling `.js` files and `support.js`).

## Fidelity
- **High fidelity, cell-exact:** the main editor frame and the command palette in Aurora, Moonlit and every pack theme (`Glyph Aurora Themes.dc.html`). Match these cell for cell.
- **Spec-exact layout, Aurora restyle by rule:** every other screen (splits, find/replace bar, prompt bar, project search, go to line, confirm cards, hover/completion popups, run panel, empty states, resize). Their geometry, copy, keys and mouse handling are in **`SPEC_V1_LAYOUT.md`** and the interactive **`v1 Layout Prototype.dc.html`**. Restyle them with the Aurora rules in §5 below. The v1 spec's Hydra role names map to Aurora roles in §6.

---

## 1. Medium
- Monospace cell grid; per cell: glyph, 24-bit fg, 24-bit bg, bold / italic / dim / underline. Curly underline (`SGR 4:3` + `58;2;r;g;b`) for diagnostics; terminals without it fall back to straight.
- Coordinates are `(x, y)`, 0-based, terminal `W×H`. Reference size **160×45**; minimum **100×30** (below that: centred "Glyph needs at least 100×30" message, see the v1 spec §1).
- **Glows are colour ramps, not effects.** `mix(a, b, t)` = per-channel linear RGB lerp, rounded. `grad(stops, t)` = piecewise `mix` across evenly spaced stops, `t` clamped to 0..1. Every glow below is just a computed bg or fg per cell.
- Glyphs used (all render in Windows Terminal; ✦ ◈ ◆ fall back to Segoe UI Symbol): `✦ ◈ ◆ ▾ ▸ • ● ✕ ⚠ ▐ ▌ ▀ ─ │ ⏎ ↑ ↓ › · …`
- Truecolor is required for the aurora. On 256-colour terminals, quantize, or drop to flat `accent` (keep the layout).

## 2. Main editor frame (160×45) — `Glyph Aurora Themes.dc.html`, left frame

```
row 0           blank (bg / tree surface)
row 1           ✦ glyph ~/src           tab pills
row 2           (tree blank)            aurora thread  ─────────
rows 3…H-3      tree rows               gutter + code
row H-2         blank (find / prompt bar lives here when open)
row H-1         status bar
```
There are **no vertical dividers**: the tree is separated from the editor by its surface colour alone.

### 2.1 Tree (x 0…27, rows 0…H-2)
- `tw = 28`. Fill `text / surface`.
- Brand at row 1: `✦` at (2,1) in `accent2`; `glyph` at (4,1), bold, each letter i of n in `fg = grad([accent, accent2], i/(n-1))`; the project dir `~/src` at (12,1) in `muted`.
- Rows start at y = 3, one per visible node (folders first, case-insensitive; ignored files never listed).
  - Indent: node x = `2 + depth·2`. Chevron `▾` (open) / `▸` (closed) in `muted` at x, for files a space. The name starts at x+2, cut with `…` at `tw − 3`.
  - **Indent guides**: `│` in `guide` at `x = 3 + d·2` for each ancestor level d < depth.
  - Name: `text`. The **active file** (the focused split's active tab) has its name in `strong` bold, and its whole row (x 0…27) gets a **glow bg**: `grad([mix(surface, accent, .26), mix(surface, accent, .08), surface], x/28)`.
  - Dirty file: `•` in `warn` at x = `tw − 3` (25).
- Keyboard selection when the tree has focus: same glow row, plus the name in `strong` bold. Hover: flat `raised` bg. (The prototype only shows the active-file state.)

### 2.2 Tab pills (row 1, from x = tw+2 = 30)
For each tab: `label = " " + name + (dirty ? " •" : "") + " "`. Step: `x += len(label) + 3`.
- **Active**: `▐` at x (fg `raised2`, bg `bg`) as the left cap; cells x+1 … x+len(label) on bg `raised2`; the name at x+2, bold, letter i coloured `grad([strong, accent], i/(n-1))`; `▌` at x+1+len(label) (fg `raised2`, bg `bg`) as the right cap.
- **Inactive**: the label at x+1 in `muted`, no bg.
- Dirty `•` in `warn` (at x + 3 + len(name)).
- Active-tab centre `pc = x + 1 + len(label)/2` (fractional), used by the aurora thread.
- Overflow, duplicate names, middle-click close: as in the v1 spec §2.

### 2.3 Aurora thread (row 2, x = tw+1 … W-1)
Every cell is `─`, with
```
hue  = grad([accent, accent2], (x − tw) / (W − tw))
d    = |x − pc|
fg   = mix(hue, bg, clamp(d / 46, 0, 1) · 0.92 + 0.04)
```
So it is brightest under the active pill and fades to ~4 % within 46 cells either side. It **moves with the active tab**: a tab switch recomputes `pc`. An animated ease is optional (~150 ms, 8 frames).

### 2.4 Editor (x = tw+1, rows 3 … H-3, width W − tw − 2)
- Gutter: `D = max(3, digits(line count))`. Cell x0 = diagnostic mark `◆` in the most severe colour on that line (`err` > `warn` > `info`). Line number right-aligned in x0+1 … x0+D, in `gutter`. Two blank cells. Text starts at `x0 + D + 3`.
- **Cursor line glow**: the bg of each cell on the cursor row = `grad([mix(bg, accent, .20), mix(bg, accent, .07), cur_line, cur_line], (x − x0) / (w · 0.8))`, so it lights up out of the gutter and settles into `cur_line` around 40 % across. The cursor line number is `accent` bold. Focused split only; an unfocused split shows no glow, with the number in `text`.
- Syntax: `syn.*` fg; **comments italic**.
- Diagnostics: curly underline in `err` / `warn` / `info` across the range.
- **Inline diagnostic lens** (new): on a line with diagnostics, at `lx = textX + len(line) + 4`, draw `◈ ` in the severity colour, then the message in `<sev>_soft` italic, cut with `…` to fit `x0 + w − lx − 4`. Only if `lx < x0 + w − 12`. When a line has several, show the most severe one.
- Selection and find matches are the same as the v1 spec (§4), with the roles from §6.
- Cursor: the hardware cursor (bar shape preferred).

### 2.5 Status bar (row H-1) — taken from 2b
- Fill `text / surface`, full width.
- **Glyph block**: cells i = 0…17 get a bg of `grad([accent, accent2], i/9)` for i < 10, then `mix(accent2, surface, (i − 9)/9)`, so it fades into the bar. Text `✦ glyph` at x = 1 in `acc_ink` bold.
- At x = 20: the path, directory in `muted` + file name in `strong` (+ ` •` in `warn` when dirty).
- Right-aligned, ending at W−2, with segments separated by 4 spaces: `Ln 52, Col 22` (`text`) · `Rust` (`text`) · `● ` (`ok`) + `rust-analyzer` (`text`) · `✕ 2` (`err`), 2 spaces, `⚠ 1` (`warn`).
- Server states: ready `● ok` · starting `○ warn` · not found `○ muted` + `no server` · crashed `✕ err` + name in `err`.
- Transient messages (save, replace, server events) and the diagnostic under the cursor take the **path slot** at x = 20 (`✓ ok` / `⚠ warn` / `✕ err` glyph + text) until the next keypress, then the path returns. The inline lens already shows diagnostics, so the path stays visible by default.
- Narrow widths: drop the server name, then the language, then shorten the path to the file name.

## 3. Command palette ("cast") — right frame
Ctrl+P / Ctrl+Shift+P. One input; prefixes switch mode: `>` commands, `:` go to line, `@` symbol, `/` project text search. With no prefix it shows files then commands.
- **Dim** the whole screen: each cell's fg and bg = `mix(c, scrim, 0.6)`.
- Card: `w = 86` (clamp to W − 4), `x = floor((W − w)/2)`, **y = 4** (drops from the top, not centred). Height = 5 + fileRows (≤5) + 2 + commandRows (≤2) + 3. Fill `text / raised`. **No border.**
- Row y: **lit top edge**, `▀` in every cell, fg = `grad([accent, accent2, accent], i/(w − 1))`, bg `raised`.
- Row y+2: `✦` at x+4 in `accent2` bold; the query at x+6 in `strong`; the cursor after it. Right-aligned to x+w−4: `cast` in `accent` + ` · files · commands` in `muted`.
- Row y+3: `─` in `line2` from x+2 to x+w−3.
- Row y+4: `FILES` in `muted` bold at x+4. Then file rows: the name (matched letters `accent` bold, the rest `fg`, or `strong` on the selected row), then 2 spaces and the directory in `muted`.
- **Selected row glow** across the full card width: `grad([mix(raised, accent, .3), mix(raised, accent2, .12), raised], (x − cardX)/w)`, plus `⏎` in `accent` right-aligned at x+w−4.
- A blank row, then `COMMANDS` in `muted` bold and command rows (name with matched letters, key binding right-aligned in `muted`).
- A blank row, then the footer: `>` `:` `@` `/` in `accent` bold, each followed by its label (`commands`, `line`, `symbol`, `text`) in `muted`, 3 spaces apart. Right: `↑↓  ⏎  esc` in `muted`.
- Keys: ↑↓ select, Enter run/open, Esc or a click outside closes, typing filters (fuzzy; basename matches score higher). The scoring is in `griffin-engine.js → fuzzy()`.

## 4. Themes
Roles the Aurora renderer reads. Values below are generated from `glyph-magic.js`.

**Signature themes** are hand-tuned (`TH.a` = Aurora, `TH.m` = Moonlit).

**Pack themes** are derived from each Hydra palette (`fromHydra()`):
| Aurora role | from Hydra / v1 role |
|---|---|
| bg, fg, muted, text, strong, acc_ink | same name |
| accent | accent |
| **accent2** | of `syn.type, syn.function, syn.string, syn.keyword`, the one with the **largest RGB distance from accent** |
| deep | dark: mix(bg, #000, .15); light: mix(bg, fg, .15) |
| surface / raised / raised2 | sidebar_bg / card / card2 |
| line2 | border |
| guide | mix(sidebar_bg, line, .6) |
| err / warn / ok / info | err / working / done / info |
| *_soft | mix(sev, bg, .35) |
| sel / cur_line / gutter / scrim | selection_bg / current_line_bg / gutter_fg / scrim |

`mono` is excluded from the aurora (it uses terminal-native colours and has no ramps). In mono, draw the same layout with flat `accent`, bold for the active states, and DIM for the scrim.

### 4.1 Chrome roles × theme
| role | aurora | moonlit | hydra | papercolor-dark | tango-dark | monokai | tokyo-night | catppuccin-mocha | catppuccin-latte | gruvbox | nord | dracula |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `bg` | `#0b0a10` | `#090b10` | `#070b10` | `#1c1c1c` | `#2e3436` | `#272822` | `#1a1b26` | `#1e1e2e` | `#eff1f5` | `#282828` | `#2e3440` | `#282a36` |
| `deep` | `#07060b` | `#06080c` | `#06090e` | `#181818` | `#272c2e` | `#21221d` | `#161720` | `#1a1a27` | `#d2d5db` | `#222222` | `#272c36` | `#22242e` |
| `surface` | `#0f0e16` | `#0d1017` | `#0c131b` | `#262626` | `#252a2b` | `#1e1f1c` | `#16161e` | `#181825` | `#e6e9ef` | `#1d2021` | `#272c36` | `#21222c` |
| `raised` | `#16141f` | `#141822` | `#0f1821` | `#303030` | `#363c3e` | `#2f302a` | `#1f2335` | `#272738` | `#e5e8ec` | `#32312f` | `#383d49` | `#32343f` |
| `raised2` | `#1e1b2a` | `#1b2030` | `#18242f` | `#3a3a3a` | `#41474a` | `#3e3d32` | `#292e42` | `#313244` | `#dadce2` | `#3d3c37` | `#434954` | `#3f414b` |
| `line2` | `#2e2a40` | `#2a3246` | `#1f2c3a` | `#444444` | `#555753` | `#49483e` | `#292e42` | `#45475a` | `#bcc0cc` | `#504945` | `#434c5e` | `#44475a` |
| `guide` | `#1f1c2b` | `#1a1f2b` | `#17222e` | `#383838` | `#424543` | `#383830` | `#212434` | `#333445` | `#cdd0da` | `#3c3937` | `#383f4e` | `#363848` |
| `muted` | `#67627d` | `#5f6a82` | `#71808f` | `#808080` | `#9a9c97` | `#8f8a72` | `#7a82ad` | `#6c7086` | `#6c6f85` | `#928374` | `#8390a8` | `#7a8ac0` |
| `text` | `#a6a2bb` | `#a2abbf` | `#a7b4c2` | `#b2b2b2` | `#d3d7cf` | `#cfcfc2` | `#a9b1d6` | `#9da3bd` | `#4e5266` | `#bfaf93` | `#b8c0ce` | `#b9c1d9` |
| `fg` | `#d2cfe2` | `#d0d6e4` | `#c9d1d9` | `#d0d0d0` | `#eeeeec` | `#f8f8f2` | `#c0caf5` | `#cdd6f4` | `#303446` | `#ebdbb2` | `#eceff4` | `#f8f8f2` |
| `strong` | `#f4f2fb` | `#f2f5fb` | `#f2f6f8` | `#eeeeee` | `#eeeeec` | `#f8f8f2` | `#e0e6ff` | `#cdd6f4` | `#303446` | `#ebdbb2` | `#eceff4` | `#f8f8f2` |
| `accent` | `#b69cff` | `#a9c6ff` | `#c3f53c` | `#00afaf` | `#8ae234` | `#a6e22e` | `#7aa2f7` | `#89b4fa` | `#7a2fd8` | `#fabd2f` | `#a3be8c` | `#bd93f9` |
| `accent2` | `#6ee7d8` | `#f0cf7a` | `#5aa9ff` | `#ff5faf` | `#34e2e2` | `#f92672` | `#9ece6a` | `#f9e2af` | `#df8e1d` | `#8ec07c` | `#88c0d0` | `#50fa7b` |
| `acc_ink` | `#120a24` | `#0b1426` | `#0a1204` | `#1c1c1c` | `#2e3436` | `#272822` | `#1a1b26` | `#1e1e2e` | `#eff1f5` | `#282828` | `#2e3440` | `#282a36` |
| `err` | `#ff6f91` | `#ff6f91` | `#ff6b6b` | `#ff5f87` | `#ff5c5c` | `#f92672` | `#f7768e` | `#f38ba8` | `#d20f39` | `#fb4934` | `#e0707a` | `#ff5555` |
| `warn` | `#f0cf7a` | `#f0cf7a` | `#ffb547` | `#ffaf00` | `#fcaf3e` | `#fd971f` | `#e0af68` | `#f9e2af` | `#8a5a00` | `#fe8019` | `#ebcb8b` | `#f1fa8c` |
| `ok` | `#7fe3c9` | `#7fe3c9` | `#7fd962` | `#5faf00` | `#73d216` | `#a6e22e` | `#9ece6a` | `#a6e3a1` | `#2f8a1f` | `#b8bb26` | `#a3be8c` | `#50fa7b` |
| `info` | `#8fd0ff` | `#a9c6ff` | `#5aa9ff` | `#5fafd7` | `#729fcf` | `#a6e22e` | `#7aa2f7` | `#89b4fa` | `#1e66f5` | `#8ec07c` | `#88c0d0` | `#50fa7b` |
| `err_soft` | `#b05a76` | `#b05a76` | `#a8494b` | `#b04862` | `#b64e4f` | `#b02756` | `#aa566a` | `#a8657d` | `#dc5e7b` | `#b13d30` | `#a25b66` | `#b4464a` |
| `warn_soft` | `#a38d58` | `#a38d58` | `#a87a34` | `#b07c0a` | `#b4843b` | `#b27020` | `#9b7b51` | `#ac9d82` | `#ad8f56` | `#b3611e` | `#a99671` | `#abb16e` |
| `info_soft` | `#6a8db0` | `#6a8db0` | `#3d72ab` | `#487c96` | `#5a7a99` | `#7aa12a` | `#5873ae` | `#6480b3` | `#6797f5` | `#6a8b5f` | `#698f9e` | `#42b163` |
| `sel` | `#221c35` | `#1c2538` | `#2a3a4c` | `#5f5faf` | `#204a87` | `#55544a` | `#3b4261` | `#313244` | `#ccd0da` | `#3c3836` | `#3b4252` | `#44475a` |
| `cur_line` | `#13111c` | `#10131b` | `#13171c` | `#272727` | `#3a3f41` | `#34342e` | `#242632` | `#29293a` | `#e4e6eb` | `#343330` | `#393f4b` | `#343641` |
| `gutter` | `#3a3550` | `#343c50` | `#515d69` | `#626262` | `#7a7d7a` | `#706d5a` | `#5d6385` | `#55576c` | `#9396a7` | `#72685d` | `#6a7489` | `#616d97` |
| `scrim` | `#000000` | `#000000` | `#040608` | `#0e0e0e` | `#171a1b` | `#141411` | `#0d0e13` | `#0f0f17` | `#b6b8c1` | `#141414` | `#171a20` | `#14151b` |

### 4.2 Syntax roles × theme
| role | aurora | moonlit | hydra | papercolor-dark | tango-dark | monokai | tokyo-night | catppuccin-mocha | catppuccin-latte | gruvbox | nord | dracula |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `syn.keyword` | `#b69cff` | `#a9c6ff` | `#a593ff` | `#ff5faf` | `#ad7fa8` | `#f92672` | `#bb9af7` | `#cba6f7` | `#8839ef` | `#fb4934` | `#81a1c1` | `#ff79c6` |
| `syn.string` | `#7fe3c9` | `#e6d29c` | `#7fd962` | `#d7af5f` | `#8ae234` | `#e6db74` | `#9ece6a` | `#a6e3a1` | `#40a02b` | `#b8bb26` | `#a3be8c` | `#f1fa8c` |
| `syn.comment` | `#5c5773` | `#56607a` | `#71808f` | `#808080` | `#888a85` | `#75715e` | `#565f89` | `#9399b2` | `#7c7f93` | `#928374` | `#8390a8` | `#7a8ac0` |
| `syn.function` | `#ece4ff` | `#eef3ff` | `#5aa9ff` | `#5fafd7` | `#729fcf` | `#a6e22e` | `#7aa2f7` | `#89b4fa` | `#1e66f5` | `#8ec07c` | `#88c0d0` | `#50fa7b` |
| `syn.type` | `#8fd0ff` | `#8fd8ff` | `#3dd6c0` | `#af87d7` | `#34e2e2` | `#66d9ef` | `#7dcfff` | `#f9e2af` | `#df8e1d` | `#fabd2f` | `#8fbcbb` | `#8be9fd` |
| `syn.number` | `#f0cf7a` | `#f0cf7a` | `#ffb547` | `#ffaf00` | `#fcaf3e` | `#ae81ff` | `#ff9e64` | `#fab387` | `#fe640b` | `#d3869b` | `#b48ead` | `#bd93f9` |
| `syn.constant` | `#f0cf7a` | `#f0cf7a` | `#ffb547` | `#ffaf00` | `#fcaf3e` | `#ae81ff` | `#ff9e64` | `#fab387` | `#fe640b` | `#d3869b` | `#b48ead` | `#bd93f9` |
| `syn.operator` | `#7d7896` | `#77819a` | `#ff7ab6` | `#00afaf` | `#fce94f` | `#f92672` | `#89ddff` | `#89dceb` | `#04a5e5` | `#fe8019` | `#81a1c1` | `#ff79c6` |
| `syn.punctuation` | `#5d5873` | `#596379` | `#a7b4c2` | `#b2b2b2` | `#d3d7cf` | `#cfcfc2` | `#a9b1d6` | `#9399b2` | `#7c7f93` | `#a89984` | `#d8dee9` | `#f8f8f2` |
| `syn.variable` | `#d2cfe2` | `#d0d6e4` | `#c9d1d9` | `#d0d0d0` | `#eeeeec` | `#f8f8f2` | `#c0caf5` | `#cdd6f4` | `#303446` | `#ebdbb2` | `#d8dee9` | `#f8f8f2` |
| `syn.property` | `#bab5d1` | `#c0c9dc` | `#e8c565` | `#5f8787` | `#e9b96e` | `#fd971f` | `#73daca` | `#b4befe` | `#7287fd` | `#83a598` | `#88c0d0` | `#ffb86c` |
| `syn.tag` | `#b69cff` | `#a9c6ff` | `#ff7ab6` | `#ff5f87` | `#ef2929` | `#f92672` | `#f7768e` | `#f38ba8` | `#d20f39` | `#fe8019` | `#81a1c1` | `#ff79c6` |
| `syn.attribute` | `#7d7896` | `#77819a` | `#e8c565` | `#d7af5f` | `#fce94f` | `#a6e22e` | `#e0af68` | `#f9e2af` | `#df8e1d` | `#fabd2f` | `#8fbcbb` | `#50fa7b` |

Config: `theme = "aurora" | "moonlit" | "<pack name>"`. `[theme_overrides]` accepts any role above, including `accent2`.

## 5. Restyling the other screens (rules)
Use the v1 geometry from `SPEC_V1_LAYOUT.md`, with these treatments:
1. **Selected list rows** (picker, project search hits, completion, run picker, tree focus): the glow row from §3, not a flat `hov`.
2. **Dimmed dialogs** (go to line, project search, confirms, run picker): fill `raised`, no border, **lit top edge** `▀` row as in §3. Input field rows on `raised2` with a `✦` prompt glyph in `accent2` in place of the label colour.
3. **Popups anchored at the cursor** (hover, completion): not dimmed, so they **keep a border**: `╭─╮│╰╯` in `line2` on `raised`. The hover rule `├─┤` is also in `line2`.
4. **Default button** on confirm cards: bg `grad([accent, accent2])` across its cells, text `acc_ink` bold. Other buttons `strong / raised2`.
5. **Find / prompt bar** (row H-2): bg `raised`. The `Aa` / `.*` chips when on are `acc_ink / accent` bold. The count is in `strong`. An invalid regex shows `err`. Find matches: `find_match_bg` = mix(bg, warn, .3); the current one is `bg / warn`.
6. **Split**: the two editors are separated by a single `│` in `guide`, the only vertical rule in the UI. Each split has its own pill row and aurora thread; in the **unfocused** split the thread is drawn at 35 % strength (the 0.92 factor becomes 0.97), and its pill uses `muted` text on `raised`.
7. **Run panel**: a title row on `surface` with the status glyph (`●` warn running, `✓` ok exit 0, `✕` err failure, `■` muted stopped) and the name in `strong` bold. Output on `bg`, ANSI colours mapped to roles (v1 spec §9). Restart marker `── restarted HH:MM:SS ──` in `muted`.
8. **Fantasy touches stay subtle**: ✦ is the brand/prompt mark, ◈ the diagnostic lens, ◆ the gutter mark. The palette is titled "cast". No other renamed vocabulary.

## 6. v1 role → Aurora role mapping
`sidebar_bg → surface` · `card → raised` · `card2 → raised2` · `border → line2` · `line → guide` · `hov / selection_bg → sel` (lists use the glow instead) · `working → warn` · `done → ok` · `current_line_bg → cur_line` · `gutter_fg → gutter` · `gutter_active_fg → accent` (bold) · `tab_active_bg → raised2` · `tab_active_fg → strong→accent ramp` · the rest keep their name.

## 7. State
Unchanged from v1: buffers/tabs per split, focus (tree | split n | run), tree open set + selection, the open overlay (palette/dialog), the popup, the bar, run state, LSP state per language, and a transient message with a timeout. New: `palette.mode` derived from the query prefix; the aurora anchor `pc` derived from the active tab position each frame.

## 8. Open notes (from the v1 review, still relevant)
Project replace-all should use **Alt+A**, not Alt+Enter (Windows Terminal takes Alt+Enter for fullscreen). Find matches must not reuse the selection colours. Keep syntax colours inside a selection. See `SPEC_V1_LAYOUT.md` §14 for all 15.

## Assets
None: everything is glyphs and colour. The prototypes preview in JetBrains Mono (Google Fonts) only to stand in for the user's terminal font.

## Files
- `Glyph Aurora Themes.dc.html`: **the chosen design**, main frame + palette in Aurora, Moonlit and 8 pack themes, cell-exact.
- `glyph-magic.js`: the Aurora renderer (`floatMain`, `floatPalette`, `fromHydra`, the signature themes). Source of truth for the ramps.
- `Glyph Directions.dc.html` + `glyph-directions.js`: the explored directions (1a–1c, 2a–2c), for context.
- `v1 Layout Prototype.dc.html`: interactive prototype of all 46 v1 states (geometry, keys, mouse). Its visuals use the old Hydra styling; restyle per §5.
- `v1 Themes.dc.html`, `SPEC_V1_LAYOUT.md`: the v1 per-screen layout spec and the full theme-role tables.
- `griffin-engine.js`, `griffin-themes.js`, `griffin-content.js`: v1 renderer, the Hydra palettes + derived roles, and the sample content. `support.js`: runtime for the `.dc.html` files.
