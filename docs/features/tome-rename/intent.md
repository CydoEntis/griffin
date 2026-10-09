# Intent: Glyph becomes Tome

Type: change
Author: Cody · Status: accepted
Date: 2026-10-09 · Plan: [Phase 15](../../PLANNING.md)

## Problem

"I think i decided i wanna change the app name to tome like a spell tome." The editor is
called Glyph everywhere: the command you type, the splash wordmark, the status line,
the config folder, the per-project `.glyph.toml`, the docs. It was renamed once before,
from Griffin to Glyph (#79), and the GitHub repo still carries the first name,
`griffin`.

## Proposed outcome

The editor is Tome: you start it with `tome`, the splash wordmark reads `tome`, the UI
and docs say Tome, settings live in `<config dir>/tome/config.toml`, backups in
`<data dir>/tome`, a project's file is `.tome.toml`, and the repo is `CydoEntis/tome`.

## Affected users and systems

- Cody, on Windows and Ubuntu.
- `Cargo.toml` (package and binary), `src/main.rs` (CLI name, env vars), `src/config.rs`
  (config folder, `PROJECT_FILE`), `src/backup.rs` (data folder), `src/clipboard.rs`,
  `src/ui/splash.rs` (wordmark), every user-facing string, `tests/` and the harness,
  `.github/`, `README.md`, `AGENTS.md`, `docs/`.

## Constraints

- Clean break (decided 2026-10-09): Tome never reads or moves `.glyph.toml` or the
  `glyph` config and data folders, as the Griffin rename never read `griffin`'s.
- Environment variables become `TOME_CONFIG`, `TOME_DATA_DIR`, `TOME_CLIPBOARD_FILE`.
- The ordinary word stays: "glyph" meaning an icon or symbol (the status line's tone
  glyphs, the run panel's state glyph, the server state glyph) is not the product name
  and is not renamed.
- Feature doc folders `docs/features/glyph-*` become `docs/features/tome-*`, with every
  link updated. Design source files under `docs/features/*/design/` keep their names
  and contents: they are the design handoff as delivered.
- The GitHub repo is renamed to `tome` (decided 2026-10-09); GitHub redirects the old
  URLs, so existing issue and PR links keep working.
- Lands before Phase 16 (C and C++) is built (decided 2026-10-09).

## Out of scope

- Reading, moving or warning about old Glyph files.
- Publishing to crates.io (the `tome` crate name there is taken by a placeholder).
- Renaming local checkout folders (`C:\dev\glyph`, worktrees) — Cody's own.

## Open questions

## Tickets

#195, #196, #197
