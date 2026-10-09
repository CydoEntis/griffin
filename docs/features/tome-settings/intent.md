# Intent: Settings, keys and server errors inside Tome

Type: feature
Author: Cody · Status: accepted
Date: 2026-10-08 · Plan: [Phase 13](../../PLANNING.md)

## Problem

Cody can't see or change Tome's settings or keys from inside it: "does [Tome] have … a
settings and a keybinds view i so i can see everything?" config.toml has to be found and
edited by hand, and is only read at startup. And when a language server fails, Tome says
so once in a passing status message and then shows only a red ✕, while the catalog still
says "installed": "i have the lsp servers installed but its not working". The cause
(TypeScript 7 ships no tsserver) was never visible.

## Proposed outcome

`>settings` opens config.toml in a tab, and saving it applies the changes at once.
`>keybindings` lists every command with its keys and config name, searchable, and
rebinds one by pressing the new key. A failed language server shows `failed` and the
reason in the catalog, and Enter there retries it.

## Affected users and systems

- Cody, on Windows Terminal and Ubuntu.
- `src/main.rs` / `src/config.rs` (loading), `src/app.rs`, `src/keymap.rs`,
  `src/lsp/transport.rs` and `client.rs` (stderr, failure reasons), `src/ui/catalog.rs`,
  a new card in `src/ui/`.

## Constraints

- Settings stay in config.toml; there's no second store. A missing file is created with
  every `[editor]` option commented out.
- Saving config.toml from Tome applies keys, `[editor]` and theme at once, and restarts a
  language's server or debug adapter whose table changed. A broken file keeps the settings
  already in use and shows the error (decided 2026-10-08).
- Rebinding writes `[keys]` and keeps the rest of the file, comments included. A key
  another command uses in the same scope asks before moving it; a key reused in another
  scope (the tree's `a`) is not a conflict.
- The keybindings card is in the catalog card's style (tome-catalog C2). `>settings` and
  `>keybindings` have no default key.
- A failed server keeps the reason it gave: its error reply, else the last line it printed
  to stderr, else its exit code. Enter on a failed catalog row retries it; `c` still copies
  the install command.

## Out of scope

- A settings form with toggles.
- Watching config.toml for edits made outside Tome.
- Fixing the user's TypeScript install; the catalog only says what's wrong.

## Open questions

## Tickets

#164, #165, #166, #167, #168, #169, #170, #171
