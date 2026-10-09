# Intent: Language server catalog

Type: feature
Author: Cody · Status: accepted
Date: 2026-10-08 · Plan: [Phase 9](../../PLANNING.md)

## Problem

Completion, hover, diagnostics and go-to-definition only work for a language once its
server is installed, and Tome only says a server is missing (R29) or lists what it found
(`tome --health`, R35). Installing one means knowing the right command and leaving Tome
to run it. Cody asked for "a catalog that you open and u can press buttons to add them and
it runs the commands in [Tome's] terminal".

## Proposed outcome

`>language servers` in the cast palette opens a catalog card listing every language, its
server and whether it is installed. Choosing a missing one runs its official install
command in the run panel, where the output streams; when it succeeds the server starts for
the files already open. The same catalog later lists debug adapters
([debugger](../tome-debugger/intent.md)).

## Affected users and systems

- Cody, on Windows Terminal and the Ubuntu box.
- `src/lsp/servers.rs` (defaults, PATH lookup), `src/lsp/mod.rs` (starting servers), the run
  panel (`src/run/`, `src/ui/run.rs`), a new card in `src/ui/`, `src/keymap.rs`,
  `src/config.rs` (`[lsp.<lang>] install`).

## Constraints

- Install commands run as child processes in the run panel; Tome bundles no server
  ([ADR-0001](../../adr/0001-single-binary-core.md)).
- Defaults use each server's documented install command; `[lsp.<lang>] install` overrides
  it (decided 2026-10-08).
- A command that needs a password or a prompt can't be answered in the run panel (it is
  read-only, see Later in the plan): the catalog shows such commands to copy instead of
  running them.
- Keys only through `src/keymap.rs`; `>language servers` has no default key. Aurora look;
  `mono` keeps working.

## Out of scope

- Updating or uninstalling servers.
- Installing the toolchains themselves (npm, go, rustup, pip): the catalog says which one is
  missing.
- Servers for languages Tome doesn't highlight.

## Open questions

## Tickets

#129–#132, in the order and with the blockers in [the plan](../../PLANNING.md).
