# Spec: Language server catalog

Intent: [intent.md](intent.md) · Status: accepted · Date: 2026-10-08

## Concerns

- **A missing server is never retried.** `Lsp::server_for` remembers a server that failed
  to start as `Client::failed` in `by_root` and never tries again (`src/lsp/mod.rs:748-790`);
  only `stop_root` clears it. An install must make Tome forget the failure for that
  language, or the new server won't start until the project is reopened.
- **PATH is read once, from Tome's own environment** (`servers::program`,
  `src/lsp/servers.rs:95-118`). A server installed into a folder that isn't on that PATH
  (a fresh `~/go/bin`) can't be found until the terminal is restarted. The catalog says so
  instead of claiming success.
- **The run panel can't take input** (stdin is null, `src/run/mod.rs:52-87`). Commands that
  need `sudo` or ask a question are shown to copy, not run.
- **Install commands go through the shell** (`cmd /C` / `sh -c`, `src/run/mod.rs:31-46`),
  so `npm` resolves to `npm.cmd` on Windows.

## Requirements

- **C1.** `>language servers` (title "Language servers", config name `language_servers`,
  no default key) in the cast palette opens the catalog — checked by PTY test.
- **C2.** The catalog is a dimmed dialog card in the cast style (`ui::dim`, `dialog_card`):
  header `✦ language servers`; one row per server — Rust, Go, TypeScript / JavaScript,
  Python, HTML, CSS, SQL — with the server command and its state: `installed` in `ok`,
  `missing` in `muted`, `needs <tool>` in `warn` when the install tool isn't on PATH, or
  `running` in `ok` when a server for it is up. ↑/↓ select (selected row on `glow_row`);
  Esc or a click outside closes it; footer `⏎ install  c copy command  esc close`. In
  `mono` the selected row is reverse video — checked by PTY tests and unit tests.
- **C3.** Each server has a default install command and the tool it needs: rust `rustup
  component add rust-analyzer` (rustup); go `go install golang.org/x/tools/gopls@latest`
  (go); typescript / javascript `npm install -g typescript-language-server typescript`
  (npm); python `npm install -g pyright` (npm); html and css `npm install -g
  vscode-langservers-extracted` (npm); sql `go install github.com/sqls-server/sqls@latest`
  (go). `[lsp.<lang>] install = "<command>"` in `config.toml` replaces it; a command
  starting with `sudo` is copy-only — checked by unit tests.
- **C4.** Enter on a `missing` row runs its install command in the run panel (opened if
  hidden), titled `install <server>`, streaming its output. With a command already running
  it refuses with `<name> is running; stop it first` in the status line. Enter does nothing
  on `installed` or `running` rows. On `needs <tool>` rows and copy-only commands, Enter
  (and `c` on any row) copies the command and the status line says `copied: <command>` —
  checked by PTY tests with a fake install command.
- **C5.** When the install exits 0 and the server is now on PATH, Tome forgets the failed
  start for that language in every root, starts the server for the open files of that
  language, and the status line says `<server> installed`. Exit 0 but still not on PATH:
  `installed, but <command> isn't on PATH; restart your terminal`. Non-zero exit: `install
  failed (exit N); see the run panel` — checked by PTY test and a fake-server test.
- **C6.** The "server not found" status message adds `· >language servers installs it` —
  checked by PTY test.
- **C7.** Every on-screen requirement has a PTY test; verify passes on Windows and Ubuntu.

## Design

- **Screens**: the catalog card (C2), built from `src/ui/mod.rs` helpers like the cast
  palette and folder browser; messages in the existing status line.
- **Data**: none stored. Install defaults live beside the server defaults in
  `src/lsp/servers.rs`; `LspServer` (`src/config.rs:27-36`) gains `install:
  Option<String>`.
- **Interfaces**: `Action::LanguageServers`; `Lsp::forget_failed(lang)` removing failed
  clients for a language from `servers` and `by_root` so the next `sync` retries; the run
  panel gains a way to start a one-off command that isn't a `.tome.toml` entry and to
  learn when that run exits.
- **Reuses**: `servers::find` for PATH checks, `start_entry` / `RunEntry` for running,
  `Clipboard` for copy.

## Out of scope

- Updating or uninstalling servers; installing npm, go, rustup or pip themselves.

## Verification

Every requirement's test passes in `cargo test` on Windows and Ubuntu CI. Person check: on
Windows, with `pyright` uninstalled, open a `.py` file, install it from the catalog, and
get completion without restarting Tome.

## Coverage

- C1 → #131
- C2 → #131
- C3 → #130
- C4 → #132
- C5 → #129, #132
- C6 → #132
- C7 → every ticket #129–#132
