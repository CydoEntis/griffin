# AGENTS.md

Tome is a non-modal TUI text editor in Rust + ratatui. Read this file, then the
plan, before changing anything.

## Plan

[`docs/PLANNING.md`](docs/PLANNING.md) is the source of truth for scope, order,
decisions and rules. Read it before starting work. If a ticket or change conflicts
with it (an active decision, a rule, or something under Out), stop and report the
conflict. Don't pick a side, and don't edit the plan to make the work fit.
With the roadmap skill installed, `/roadmap check` (Claude Code) or `$roadmap check`
(Codex) runs this check.

Requirements (R1–R35), the screen layout, the default keymap and the config format
live in [`docs/features/tome-v1/spec.md`](docs/features/tome-v1/spec.md).
Tickets cite them by number.

## Verify

```
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

All three must pass before a change is done. CI runs them on Windows and Ubuntu.

## Layout

| Path | Owns |
|---|---|
| `src/main.rs` | CLI args, terminal setup/teardown, panic hook, starting the loop |
| `src/app.rs` | `App` (all editor state), `AppEvent`, the event loop, top-level render |
| `src/keymap.rs` | `Action`, default bindings, parsing `[keys]`, turning key events into an `Action` or typed text |
| `src/config.rs` | loading `config.toml` and `.tome.toml` |
| `src/buffer/` | `Buffer`: rope, cursor, selection, edits, undo history, line endings |
| `src/view/` | editor viewport and rendering of a buffer |
| `src/ui/` | chrome: tab bar, status line, tree panel, popups, picker, find bar, run panel |
| `src/workspace/` | project root, file walking, tree model, file operations |
| `src/save.rs`, `src/backup.rs` | atomic save, crash backups |
| `src/theme.rs` | themes and overrides |
| `src/clipboard.rs` | `Clipboard` trait, OS impl (arboard) and test fake |
| `src/highlight/` | tree-sitter engine; one file per language under `languages/` |
| `src/run/` | run commands: spawning, process trees, detection |
| `src/lsp/` | transport, client, per-language server config |
| `src/dap/` | Debug Adapter Protocol: per-language adapters and launch settings |
| `src/bin/fake_lsp.rs` | scripted fake language server used by LSP tests |
| `src/framing.rs` | `Content-Length` message framing shared by LSP and DAP |
| `src/dap/` | Debug Adapter Protocol: adapter transport and session |
| `src/bin/fake_dap.rs` | scripted fake debug adapter used by DAP tests |
| `tests/harness/` | the PTY harness; `tests/*.rs` use it |

Create a path the first time a ticket needs it. Keep to this table; if a ticket
seems to need a new top-level area, say so instead.

## How the code works

- One `App` owns all state on the main task ([ADR-0001](docs/adr/0001-single-binary-core.md)).
  Background work (LSP, run output, timers, file walks) sends `AppEvent`s over the
  app channel. Never share `App` across tasks or put it behind a lock.
- Keys: only `src/keymap.rs` looks at `KeyCode`. Everything else receives an
  `Action` (or typed text). A new feature adds an `Action` variant, a default
  binding from the spec's keymap table, and handles the action.
- Positions in a buffer are char indices into the rope. Screen columns go through
  `unicode-width`. Never index a `String` by byte for cursor maths.
- Rendering is a pure function of `App`; it never mutates state.
- OS effects that tests can't run (clipboard, trash) sit behind a small trait with a
  fake for tests.

## Tests

- Logic (buffer edits, undo, save, parsing) gets unit tests next to the code.
- Anything a user sees or presses gets a PTY harness test in `tests/` that runs the
  real `tome` binary, sends input and asserts on screen text. A test that only
  calls internal functions doesn't prove a screen works.
- No sleeps to wait for output: use the harness's wait-for-text helper with a
  timeout.
- Fixtures go in `tests/fixtures/`.

## Commits

Conventional Commits, in plain words, with the issue: `feat(tabs): close a tab with
Ctrl+W (#12)`. One ticket, one branch, one PR, title equal to the ticket title.
Never commit with failing verify, never `--no-verify`.

## Code style

- `anyhow::Result` in the binary; no `unwrap()`/`expect()` outside tests except on
  invariants, with a comment saying why it holds.
- No `unsafe`, except OS process-tree calls in `src/run/` (Windows job objects, Unix
  process groups).
- Comments say why, not what.
