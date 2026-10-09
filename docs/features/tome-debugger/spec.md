# Spec: Debugger

Intent: [intent.md](intent.md) · Status: accepted · Date: 2026-10-08

## Concerns

- **A new top-level area, `src/dap/`.** AGENTS.md asks for new areas to be named. The
  debugger gets `src/dap/` (transport, client, adapters) and a fake adapter
  `src/bin/fake_dap.rs`; AGENTS.md's layout table gains both lines.
- **Shared framing.** DAP uses LSP's `Content-Length` framing. `encode` and `read_message`
  in `src/lsp/transport.rs:18-61` are generic over `serde_json::Value`; they move to a
  module both use rather than being copied.
- **Keys.** F6 is CycleFocus and F4, F5, Shift+F5, Ctrl+F5 belong to the run panel
  (`src/keymap.rs:542-561`). F11 is Windows Terminal's fullscreen key and never reaches
  Tome. Defaults: F9 toggle breakpoint, Alt+F5 start or continue, Alt+F6 stop debugging,
  F10 step over, Alt+F10 step into, Shift+F10 step out — all rebindable.
- **lldb-dap on Windows.** `winget install LLVM.LLVM` puts `lldb-dap.exe` in
  `C:\Program Files\LLVM\bin`, not on PATH by default; the adapter lookup checks that
  folder too. LLDB reads MSVC-built Rust through its PDB support, which is thinner than
  on Linux; the person check covers it, and `[debug.rust] adapter` can name another
  adapter.
- **Breakpoint lines move with edits**: lines inserted or removed above a breakpoint move
  it with its text.

## Requirements

- **D1.** `src/dap/` speaks DAP to an adapter child process: `initialize`, `launch`,
  `setBreakpoints`, `configurationDone`, `threads`, `stackTrace`, `scopes`, `variables`,
  `continue`, `next`, `stepIn`, `stepOut`, `disconnect`, and the `initialized`, `stopped`,
  `output`, `terminated` and `exited` events, reporting to `App` as `AppEvent`s. A missing
  or crashed adapter shows one status message and never blocks editing — checked by tests
  against `fake_dap`.
- **D2.** F9, or a click in the gutter's mark cell, toggles a breakpoint on that line: `●`
  in `err` in the mark cell, in place of a diagnostic mark on that line. Breakpoints belong
  to files, survive closing the tab while Tome runs, and move with lines inserted or
  removed above them. `>toggle breakpoint` and `>clear breakpoints` are cast commands —
  checked by PTY tests.
- **D3.** Adapters and launch defaults: rust → `lldb-dap` (also looked for in
  `C:\Program Files\LLVM\bin` on Windows), build `cargo build`, program
  `target/debug/<package name>`; python → `python -m debugpy.adapter`, program the current
  file; go → `dlv dap`, program the current file's package folder, mode `debug`.
  `.tome.toml` `[debug]` (`program`, `args`, `cwd`, `build`) overrides the launch for the
  project; `config.toml` `[debug.<lang>]` `adapter` / `args` overrides the adapter —
  checked by unit tests.
- **D4.** Alt+F5 with no session starts one for the active file's language: it runs the
  build command in the run panel first if there is one (a failed build stops there), then
  starts the adapter, sends every breakpoint and launches. Program output goes to the run
  panel titled `debug <program>`. The status bar shows `● debugging` while running and
  `‖ paused <file>:<line>` while stopped. Alt+F6 stops the session and the program. A
  language with no adapter says `no debugger for <lang>` — checked by PTY tests.
- **D5.** When the program stops (breakpoint, step, exception) Tome opens the file at the
  line, marks it with `▶` in `accent2` in the mark cell and a cursor-line glow in
  `accent2`, and keeps the marker until the program runs again — checked by PTY tests.
- **D6.** While paused, Alt+F5 continues, F10 steps over, Alt+F10 steps into, Shift+F10
  steps out; while running they do nothing. The program ending ends the session with
  `exited N` in the status line — checked by PTY tests.
- **D7.** While a session is active the bottom panel is the debug panel (F4 toggles it; the
  run panel returns when the session ends): the call stack on the left (frame name and
  `file:line`, the selected frame on `glow_row`) and the selected frame's variables on the
  right, by scope, as `name = value  type` with `▸` / `▾` on structured values. F6 cycles
  focus into it; ↑/↓ move, Enter or → expands, ← collapses, Enter on a frame jumps to it.
  In `mono` the selection is reverse video — checked by PTY tests.
- **D8.** The catalog (Phase 9) lists debug adapters under a `debuggers` heading with the
  same states: `lldb-dap` (Windows `winget install --id LLVM.LLVM -e
  --accept-source-agreements --accept-package-agreements`; Linux `sudo apt install lldb`,
  copy-only), `debugpy` (`python -m pip install debugpy`, needs python), `dlv` (`go install
  github.com/go-delve/delve/cmd/dlv@latest`, needs go) — checked by PTY test.
- **D9.** Every on-screen requirement has a PTY test against `fake_dap`; verify passes on
  Windows and Ubuntu.

## Design

- **Screens**: gutter marks (D2, D5) in the mark cell of `src/view/mod.rs` (`render_buffer`
  :178-345, diagnostic mark :298-306); the debug panel (D7) in the run panel's slot
  (`Panes::new`, `src/app.rs:3533-3595`); a status bar segment (D4) in `src/ui/status.rs`.
- **Data**: breakpoints in `App`, by file path, not saved to disk.
- **Interfaces**: new `Action`s `ToggleBreakpoint`, `ClearBreakpoints`, `DebugStart` (start
  or continue), `DebugStop`, `StepOver`, `StepInto`, `StepOut`, and the panel's actions in a
  new `Scope::Debug`; `ProjectConfig` gains `debug`; `Config` gains `debug:
  BTreeMap<String, DebugAdapter>`.
- **Reuses**: the LSP framing (moved), `run::spawn` and the run panel for the build and the
  program's output, `servers::find` for adapter lookup, `ui::glow_row`, the catalog card.

## Out of scope

- JS/TS, attach, remote; conditional breakpoints, logpoints, watches, evaluate, hover
  values, saved breakpoints; several sessions at once.

## Verification

Every requirement's test passes in `cargo test` on Windows and Ubuntu CI. Person check: on
Windows and Ubuntu, set a breakpoint in a small Rust, Python and Go program, start
debugging, see the paused line and variables, step over, into and out, and stop.

## Coverage

- D1 → #133
- D2 → #134
- D3 → #135
- D4 → #136
- D5 → #136
- D6 → #137
- D7 → #138
- D8 → #139
- D9 → every ticket #133–#139
