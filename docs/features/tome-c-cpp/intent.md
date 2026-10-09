# Intent: C and C++

Type: feature
Author: Cody · Status: accepted
Date: 2026-10-09 · Plan: [Phase 16](../../PLANNING.md)

## Problem

"add C and C++ i wanna learn those." Tome knows 8 languages and C and C++ aren't among
them: a `.c` or `.cpp` file opens as plain text, with no colours, no completion, no
Ctrl+/, nothing on F5 and no debugger. Learning C in Tome today means compiling and
running every program by hand in another terminal.

## Proposed outcome

Open `hello.c` or `hello.cpp` and Tome treats it like Rust or Go: it's highlighted,
clangd gives completion and errors (installed from the catalog), Ctrl+/ comments lines,
F5 compiles and runs it in the run panel, where a prompt shows at once and you can type
the program's input, and Alt+F5 debugs it, stopping at F9 breakpoints.

## Affected users and systems

- Cody, learning C and C++ on Windows (VS 2022 Build Tools present, LLVM not yet
  installed) and Ubuntu.
- `src/highlight/languages/` (two new languages), `src/lsp/servers.rs`,
  `src/ui/catalog.rs`, `src/run/` (detection, a pseudo-terminal, input),
  `src/ui/run.rs`, `src/dap/adapters.rs`, `src/app.rs`.

## Constraints

- Extensions: C is `.c` `.h`; C++ is `.cpp` `.cc` `.cxx` `.hpp` `.hh` `.hxx`
  (`.h` is C, decided 2026-10-09). Status line names: `C`, `C++`. Registry ids `c` and
  `cpp`, which are also their LSP language ids and `[lsp.*]` / `[debug.*]` keys.
- Grammars are compiled in ([ADR-0001](../../adr/0001-single-binary-core.md)):
  `tree-sitter-c`, `tree-sitter-cpp`. Ctrl+/ uses `//` for both.
- Language server: `clangd` for both. Install from the catalog: Windows
  `winget install --id LLVM.LLVM -e` (the package the Rust debugger already uses);
  Ubuntu `sudo apt install clangd` (copy-only, like other sudo commands). LLVM tools
  that aren't on PATH are found in `%ProgramFiles%\LLVM\bin`, as `lldb-dap` already is.
- Compiler: clang (`clang` for C, `clang++` for C++), flags `-Wall -g`, clang's default
  standard. On Windows clang uses the MSVC Build Tools for headers and linking.
- F5, when `.tome.toml` has no `[[run]]` and the open file is a `.c`, `.cpp`, `.cc` or
  `.cxx`, offers first: compile that file to a program next to it (`hello.exe` on
  Windows, `hello` elsewhere, decided 2026-10-09) and run it. A project with a
  `Makefile` also gets `make`; one with `CMakeLists.txt` gets
  `cmake -B build && cmake --build build`, which builds only — running or debugging a
  CMake program means naming it in `.tome.toml`. Configured `[[run]]` entries still
  replace detection outright.
- The open-file run gets a pseudo-terminal (decided 2026-10-09), so the program's stdout
  is a terminal and prompts printed without a newline show at once. Other detected
  commands keep pipes, because dev servers redraw the screen and the run panel can't.
  A `[[run]]` entry can ask for one with `terminal = true`.
- While a program with a terminal runs, F6 can move focus to the run panel, where a line
  typed at the bottom is sent with Enter; the typed line also shows in the output, the
  way a terminal echoes it.
- Debugging (F9 breakpoints, Alt+F5 start) uses `lldb-dap` for both languages; the build
  step compiles the open file with the same command as Run, and the program is the same
  output. `.tome.toml` `[debug]` overrides it, as for Rust.
- Adds C and C++ to the language lists in the v1 spec (R24, R35); the v1 spec stays as
  shipped, this intent is the record of the addition.

## Out of scope

- Multi-file programs without a Makefile or CMake; choosing a compiler other than
  clang (gcc stays possible through `[[run]]` / `[debug]`); compile flags in config.
- A full terminal in the run panel (arrow keys, full-screen programs) — the Later
  "Embedded interactive terminal" item stays deferred. Input to a program while it is
  being debugged.
- `compile_commands.json` generation; clang-format on save beyond what clangd's
  formatting already gives through `format_on_save`.

## Open questions

## Tickets

#185, #186, #187, #188, #189, #190, #191, #192, #193
