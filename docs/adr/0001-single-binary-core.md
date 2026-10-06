# ADR-0001: Griffin is one binary with a rope buffer, compiled-in grammars and one event loop; tools run as child processes

Status: accepted
Date: 2026-10-06

## Context

Griffin is a new TUI editor (no code yet; `C:\dev\text-editor` was empty on
2026-10-06). v1 needs editing, highlighting for 8 languages, LSP and a run panel,
built in about a day by agents working small tickets in parallel. Those agents need
one shape to build into, or each ticket invents its own threading and text model.
Hydra, the sibling app, already settled on ratatui 0.30, crossterm 0.29 and tokio
(`CydoEntis/hydra` Cargo.toml).

## Decision

- **Text** lives in a `ropey` 1.x rope, one per open buffer. Positions are char
  indices into the rope; screen columns are computed with `unicode-width`.
- **Highlighting** uses `tree-sitter` + `tree-sitter-highlight` with every grammar
  compiled into the binary as a crate dependency. Adding a language means a rebuild.
- **One event loop.** A single `App` value owns all editor state and runs on the
  main tokio task. Terminal input, LSP messages, run-panel output and timers arrive
  as `AppEvent`s on one channel; the loop applies them and redraws. Background tasks
  never touch `App` directly.
- **Language servers and run commands are child processes** spawned with
  `tokio::process`, found on PATH or named in config. Griffin never bundles or
  installs them.
- **No plugin or scripting API.** Behaviour is extended by changing Griffin.

## Alternatives considered

- **Gap buffer or `Vec<String>` lines**: simpler, but slow edits in large files and
  no cheap snapshots for undo, tree-sitter input and LSP sync. Lost on scale.
- **Runtime-loaded grammars (`.so`/`.dll`, like Helix)**: new languages without a
  rebuild, but needs a grammar fetch/build step on two OSes and an ABI to manage.
  Lost on day-one cost; 8 fixed languages don't need it.
- **Threads with shared `Arc<Mutex<App>>`**: lets background work mutate state, but
  locking across render and LSP is where TUI editors deadlock and tear. Lost on
  safety.
- **Plugin API (Lua/WASM)**: declined on 2026-10-06 (plan Out list).

## Consequences

- Easier: every ticket knows where state lives and how async results come back;
  tests can feed `AppEvent`s directly.
- Easier: one `cargo install` gives a complete editor apart from language servers.
- Harder: adding a language or behaviour always means a code change and release.
- Harder: a missing language server is a normal state that every LSP feature must
  handle quietly.
- Ruled out: plugins, runtime grammar loading, bundled language servers, mutating
  `App` from a background task.
