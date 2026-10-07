# Intent: Glyph v1

Type: feature
Author: Cody · Status: accepted
Date: 2026-10-06 · Plan: [Phases](../../PLANNING.md#phases)

## Problem

"I want a super simple text editor like micro", a TUI with keyboard hotkeys and
mouse support, that I can use as my daily editor and do real hand programming in.
The existing editors either lack the basics in one place (micro needs plugins for
LSP and a file tree) or don't look and feel like the rest of my setup (Hydra).

## Proposed outcome

`glyph .` opens a project in the terminal with a file tree, tabs and a split. I can
create files, jump to any file, find and replace, see syntax highlighting and LSP
help (diagnostics, go to definition, hover, completion, format on save) for
TypeScript, HTML, CSS, React, SQL, Go, Rust and Python, and run my dev server in a
panel, all by keyboard or mouse, without ever losing an edit.

## Affected users and systems

- Me, on Windows Terminal at home and on the Ubuntu work box.
- Hydra: Glyph runs inside a Hydra pane and is Hydra's editor through
  `editor = "glyph"`. No code is shared between them.

## Constraints

- Rust + ratatui, the same crate line as Hydra (ratatui 0.30, crossterm 0.29).
- Non-modal, micro / VS Code style keys, every binding remappable.
- Hydra's theme names and `[theme_overrides]` shape.
- Language servers are found on PATH; Glyph never installs them.
- Architecture: [ADR-0001](../../adr/0001-single-binary-core.md).

## Out of scope

- A plugin system (declined, see the plan's Out list).
- An embedded interactive terminal, multi-cursor, git integration (deferred, see
  Later).

## Open questions

## Tickets

#1–#10 (Edit), #11–#17 (Workspace), #18–#21 (Search), #22–#26 (Highlight), #27–#29 (Run), #30–#36 (LSP). Order and blockers: [the plan](../../PLANNING.md#phases).
