# Intent: Glyph Aurora

Type: change
Author: Cody · Status: accepted
Date: 2026-10-07 · Plan: [Phase 7](../../PLANNING.md#7--glyph-wears-the-aurora-look)

## Problem

Glyph v1 copies Hydra's look ([v1 spec, Design](../glyph-v1/spec.md#design): "Hydra's
look: no boxes, `│` dividers, chrome on `sidebar_bg`"). Now that it's Glyph, not Griffin,
it should have its own look. The chosen direction is "2a Aurora · Float": "obsidian-dark
chrome with a two-colour aurora light (accent → accent2), drawn with per-cell 24-bit colour
ramps", with 2b's solid status bar ([design README](design/README.md#overview)).

## Proposed outcome

Every screen Glyph draws matches the Aurora design: the main frame and the "cast" command
palette match the design cell for cell, and every other screen keeps its current behaviour
restyled by the design's rules. Two signature themes (`aurora`, `moonlit`) ship alongside
Hydra's ten, and those ten render in the Aurora style too.

## Affected users and systems

- Me, on Windows Terminal and the Ubuntu box, every screen.
- `src/theme.rs`, `src/ui/*`, `src/view/`, `src/app.rs` render and layout, `src/keymap.rs`
  (Ctrl+P, Ctrl+G, Alt+A), and most PTY tests' expected screen text.

## Constraints

- Overrides the v1 spec's "Hydra's look" Design section; v1's behaviour (R1–R35) stays,
  except R18 (Ctrl+P opens the cast palette) and R23 (replace all is Alt+A).
- The default theme stays `hydra`, now drawn in the Aurora style (decided 2026-10-07).
- Default keys avoid Ctrl+Shift (v1 spec Concerns): the palette is Ctrl+P only.
- Rendering stays a pure function of `App`; no animation timer (decided 2026-10-07).
- `mono` keeps working with terminal-native colours: same layout, flat `accent`, bold for
  active states, DIM for the scrim (design README §4).
- [ADR-0001](../../adr/0001-single-binary-core.md) holds.

## Out of scope

- The v1-layout behaviour changes the design also proposes (minimum size message, scaling
  tree, tab overflow marker and duplicate names, completion anchor shift, run panel
  scrollback, `‹ ›` scroll markers, rename stem preselect, F7 stop):
  Phase 11.
- `@` symbol search in the palette: Later (needs LSP document symbols).
- 256-colour fallback / truecolor detection: themes already need truecolor today.
- Animating the aurora thread.

## Open questions

## Tickets

#80–#95, in the order and with the blockers in [the plan](../../PLANNING.md#7--glyph-wears-the-aurora-look).
