# Intent: Debugger

Type: feature
Author: Cody · Status: accepted
Date: 2026-10-08 · Plan: [Phase 10](../../PLANNING.md)

## Problem

Tome can run a project (F5) but not debug it: no breakpoints, no stepping, no way to see
variables. Cody: "i think we need that for when we are programming im not saying make one
our selves cant we bring an existing one in".

## Proposed outcome

Tome talks to existing debuggers over the Debug Adapter Protocol (DAP), the way it talks to
language servers over LSP. In a Rust, Python or Go project you set breakpoints in the
gutter, start debugging, and when the program stops Tome shows the paused line, the call
stack and the variables; you continue, step over, step into, step out or stop. The
adapters install from the [catalog](../tome-catalog/intent.md).

## Affected users and systems

- Cody, on Windows Terminal and the Ubuntu box.
- A new `src/dap/` (transport, client, adapter config), the gutter (`src/view/`), a debug
  panel in `src/ui/`, `src/keymap.rs`, `src/config.rs` (`[debug]` in `.tome.toml`), the
  catalog, a fake adapter binary for tests.

## Constraints

- Adapters are external programs Tome starts as child processes
  ([ADR-0001](../../adr/0001-single-binary-core.md)); none are bundled.
- First languages: Rust (`lldb-dap`), Python (`debugpy`), Go (Delve, `dlv dap`)
  (decided 2026-10-08).
- One debug session at a time (decided 2026-10-08).
- A missing or crashed adapter never blocks editing, as R29 for servers.
- Keys only through `src/keymap.rs`; defaults avoid Ctrl+Shift. Aurora look; `mono` keeps
  working. Every on-screen requirement gets a PTY test against a fake adapter.

## Out of scope

- JavaScript / TypeScript debugging (js-debug), attaching to a running process, remote
  debugging.
- Conditional breakpoints, logpoints, watch expressions, a debug console that evaluates
  expressions, values on hover, breakpoints kept across restarts — all Later.
- More than one debug session at once.

## Open questions

## Tickets

#133–#139, in the order and with the blockers in [the plan](../../PLANNING.md).
