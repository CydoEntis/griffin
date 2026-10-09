//! The catalog `>language servers` opens (glyph-catalog spec C1, C2;
//! glyph-debugger spec D8): a card in the cast's style listing each language
//! server, then each debug adapter under a `debuggers` heading, with its command
//! and whether it's installed, missing, waiting on its install tool, running, or
//! failed this session and why.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::buffer::movement::Motion;
use crate::config::LspServer;
use crate::dap::adapters;
use crate::keymap::{Action, Input};
use crate::lsp::servers;
use crate::theme::Theme;
use crate::ui::{dialog_card, dim, footer, glow_row};

/// The card's width before clamping to the screen, and the row it drops from:
/// the cast palette's, so the catalog reads as the palette turning into it.
const WIDTH: u16 = 86;
const TOP: u16 = 4;
/// The rows other than the servers and debuggers: the lit edge, a blank, the
/// header and the rule above them; a blank and the footer below.
const CHROME: u16 = 6;
/// The rows between the servers and the debuggers: a blank and the heading.
const GAP: u16 = 2;
const HEADER: &str = "language servers";
const DEBUGGERS: &str = "debuggers";
const FOOTER: &str = "⏎ install  c copy command  esc close";
/// The footer on a failed row, where Enter starts the server again.
const FOOTER_RETRY: &str = "⏎ retry  c copy command  esc close";
/// Cells for the row's name, so the commands line up: the longest name and two
/// blanks.
const NAME_WIDTH: u16 = 25;

/// One row per server, in the spec's order: its name and the languages it
/// serves. The first language's table gives the command and install; any of
/// them having a server up makes the row `running`, and any of them having a
/// failed server makes it `failed`.
const ENTRIES: [(&str, &[&str]); 7] = [
    ("Rust", &["rust"]),
    ("Go", &["go"]),
    (
        "TypeScript / JavaScript",
        &["typescript", "tsx", "javascript", "jsx"],
    ),
    ("Python", &["python"]),
    ("HTML", &["html"]),
    ("CSS", &["css"]),
    ("SQL", &["sql"]),
];

/// One row per debug adapter, in the spec's order: its name, the language whose
/// default adapter it is, and how to tell it's installed.
const ADAPTERS: [(&str, &str, Probe); 3] = [
    ("lldb-dap", "rust", Probe::Program),
    ("debugpy", "python", Probe::Module("debugpy")),
    ("dlv", "go", Probe::Program),
];

/// Where a server is, as its row says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// A server for it is up in some root.
    Running,
    /// A server for it was spawned this session and then crashed or refused to
    /// initialize; the row's `reason` says why.
    Failed,
    /// Its command is on PATH.
    Installed,
    /// Not on PATH, and neither is the program that installs it.
    Needs(String),
    /// Not on PATH; installing it is up to its install command.
    Missing,
}

impl State {
    /// The words at the row's right.
    pub fn label(&self) -> String {
        match self {
            State::Running => "running".into(),
            State::Failed => "failed".into(),
            State::Installed => "installed".into(),
            State::Needs(tool) => format!("needs {tool}"),
            State::Missing => "missing".into(),
        }
    }

    /// The colour of those words.
    pub fn color(&self, theme: &Theme) -> Color {
        match self {
            State::Running | State::Installed => theme.ok,
            State::Failed => theme.err,
            State::Needs(_) => theme.warn,
            State::Missing => theme.muted,
        }
    }
}

/// How a row's command is known to be installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// The command is found on PATH (or, for `lldb-dap`, in LLVM's folder).
    Program,
    /// The command is a Python that can import this module: pip puts debugpy
    /// beside Python rather than on PATH.
    Module(&'static str),
}

/// Where the catalog looks for what's installed.
pub struct Lookup<'a> {
    /// The `PATH` and `PATHEXT` values `servers::find` searches.
    pub path: Option<&'a OsStr>,
    pub pathext: Option<&'a OsStr>,
    /// Where LLVM's installer puts `lldb-dap` (see `adapters::llvm_bin`).
    pub llvm_bin: Option<&'a Path>,
    /// Whether the Python at a path can import a module (see `adapters::imports`).
    pub imports: &'a dyn Fn(&Path, &str) -> bool,
}

impl Lookup<'_> {
    /// Whether `command` is there as `probe` checks it.
    pub fn found(&self, command: &str, probe: Probe) -> bool {
        let program = adapters::find(command, self.path, self.pathext, self.llvm_bin);
        match probe {
            Probe::Program => program.is_some(),
            Probe::Module(module) => program.is_some_and(|python| (self.imports)(&python, module)),
        }
    }
}

/// One server's or adapter's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: &'static str,
    /// The command that starts the server or adapter.
    pub command: String,
    /// The adapter's args, shown after the command: `python` alone wouldn't say
    /// what runs. Servers show their command only.
    pub args: &'static [&'static str],
    pub state: State,
    /// Why its server failed, when `state` is `Failed`.
    pub reason: Option<String>,
    /// The command that installs it, if one is known.
    pub install: Option<String>,
    /// The languages it serves, whose failed starts an install makes Glyph
    /// forget. None for an adapter: nothing remembers a failed debug session.
    pub langs: &'static [&'static str],
    /// The install command has to be run by the user (see `servers::copy_only`).
    pub copy_only: bool,
    pub probe: Probe,
}

/// What a key did to the catalog, when it did more than move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Esc: close it.
    Close,
    /// Put this install command on the clipboard.
    Copy(String),
    /// `c` on a row with no install command: the status line says so.
    NoInstall(&'static str),
    /// Enter on a `missing` row: run its install command in the run panel.
    Install(Install),
    /// Enter on a `failed` row: try starting its server again for the open
    /// files.
    Retry {
        name: &'static str,
        langs: &'static [&'static str],
    },
}

/// An install Enter asked for: what to run, and what to check once it's done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    /// The row's name, which titles the run and the messages.
    pub name: &'static str,
    pub langs: &'static [&'static str],
    /// The command that starts the server or adapter, looked up afterwards.
    pub command: String,
    /// The command that installs it.
    pub install: String,
    /// How to tell the install worked.
    pub probe: Probe,
}

impl Install {
    /// The status line when the install exited 0 but what it installs still
    /// can't be found.
    pub fn not_found(&self) -> String {
        match self.probe {
            Probe::Program => format!(
                "installed, but {} isn't on PATH; restart your terminal",
                self.command
            ),
            Probe::Module(module) => {
                format!("installed, but {} can't import {module}", self.command)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Catalog {
    /// The servers, then the adapters.
    rows: Vec<Row>,
    /// How many of `rows` are servers; the `debuggers` heading comes after them.
    servers: usize,
    selected: usize,
}

impl Catalog {
    /// The catalog for the server tables `config`, with `running` saying which
    /// languages have a server up, `failure` why a language's server failed this
    /// session, and programs looked up through `lookup`.
    pub fn new(
        config: &BTreeMap<String, LspServer>,
        running: impl Fn(&str) -> bool,
        failure: impl Fn(&str) -> Option<String>,
        lookup: &Lookup,
    ) -> Self {
        // Fills in any language the tables leave out; ones already filled in
        // stay as they are.
        let config = servers::with_defaults(config.clone());
        let mut rows: Vec<Row> = ENTRIES
            .iter()
            .map(|&(name, langs)| {
                let server = config.get(langs[0]).cloned().unwrap_or_default();
                let install = servers::install_state(&server, lookup.path, lookup.pathext);
                let running = langs.iter().any(|lang| running(lang));
                let reason = langs.iter().find_map(|lang| failure(lang));
                let state = state(running, reason.is_some(), install.command_found, &install);
                Row {
                    name,
                    command: server.command.unwrap_or_default(),
                    args: &[],
                    reason: reason.filter(|_| state == State::Failed),
                    state,
                    install: install.install,
                    langs,
                    copy_only: install.copy_only,
                    probe: Probe::Program,
                }
            })
            .collect();
        let servers = rows.len();
        // The spec's adapters, whatever `[debug.<lang>]` says: the install
        // commands install these programs, not ones a table names.
        rows.extend(ADAPTERS.iter().filter_map(|&(name, lang, probe)| {
            let (command, args) = adapters::default_adapter(lang)?;
            let server = LspServer {
                command: Some(command.to_string()),
                install: adapters::default_install(lang).map(str::to_string),
                ..LspServer::default()
            };
            let install = servers::install_state(&server, lookup.path, lookup.pathext);
            Some(Row {
                name,
                command: command.to_string(),
                args,
                state: state(false, false, lookup.found(command, probe), &install),
                reason: None,
                install: install.install,
                langs: &[],
                copy_only: install.copy_only,
                probe,
            })
        }));
        Catalog {
            rows,
            servers,
            selected: 0,
        }
    }

    #[cfg(test)]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// ↑ and ↓ move the selection, stopping at either end; `c` copies the
    /// selected row's install command; Enter retries a `failed` server, installs
    /// a `missing` one, or copies the command when Glyph can't run it; Esc
    /// closes.
    pub fn handle(&mut self, input: Input) -> Option<Step> {
        match input {
            Input::Action(Action::Move(Motion::Up)) => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            Input::Action(Action::Move(Motion::Down)) => {
                self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1));
                None
            }
            Input::Action(Action::CatalogCopy) => {
                let row = self.rows.get(self.selected)?;
                Some(match &row.install {
                    Some(install) => Step::Copy(install.clone()),
                    None => Step::NoInstall(row.name),
                })
            }
            Input::Action(Action::Newline) => {
                let row = self.rows.get(self.selected)?;
                // Installing again wouldn't change what made it fail; starting
                // it again might, once whatever it lacked is fixed.
                if row.state == State::Failed {
                    return Some(Step::Retry {
                        name: row.name,
                        langs: row.langs,
                    });
                }
                // Nothing to do for a server that's already there.
                if matches!(row.state, State::Installed | State::Running) {
                    return None;
                }
                let Some(install) = &row.install else {
                    return Some(Step::NoInstall(row.name));
                };
                // The run panel can't answer a prompt, and an install whose
                // tool is missing would only fail: the user runs it instead.
                if row.copy_only || matches!(row.state, State::Needs(_)) {
                    return Some(Step::Copy(install.clone()));
                }
                Some(Step::Install(Install {
                    name: row.name,
                    langs: row.langs,
                    command: row.command.clone(),
                    install: install.clone(),
                    probe: row.probe,
                }))
            }
            Input::Action(Action::Cancel) => Some(Step::Close),
            _ => None,
        }
    }

    /// The selected row's failure reason, which takes the line below it.
    fn reason(&self) -> Option<&str> {
        self.rows
            .get(self.selected)
            .filter(|row| row.state == State::Failed)?
            .reason
            .as_deref()
    }

    /// The rows the card's body takes: every row, the selected row's reason,
    /// and the gap before the adapters when there are any.
    fn body_height(&self) -> u16 {
        let rows = u16::try_from(self.rows.len()).unwrap_or(u16::MAX);
        let gap = if self.rows.len() > self.servers {
            GAP
        } else {
            0
        };
        let reason = u16::from(self.reason().is_some());
        rows.saturating_add(gap).saturating_add(reason)
    }

    /// The row `i` is drawn on, below the card's top: past the heading for an
    /// adapter, and past the selected row's reason for a row below it.
    fn row_offset(&self, i: usize) -> u16 {
        let gap = if i >= self.servers { GAP } else { 0 };
        let reason = u16::from(i > self.selected && self.reason().is_some());
        4 + u16::try_from(i)
            .unwrap_or(u16::MAX)
            .saturating_add(gap)
            .saturating_add(reason)
    }

    /// Where the card goes in `area`: the cast's width, centred, from row 4, as
    /// tall as its rows need.
    pub fn card(&self, area: Rect) -> Rect {
        let width = WIDTH.min(area.width.saturating_sub(4));
        let y = area.y + TOP.min(area.height);
        let height = CHROME
            .saturating_add(self.body_height())
            .min(area.bottom() - y);
        Rect {
            x: area.x + (area.width - width) / 2,
            y,
            width,
            height,
        }
    }

    /// Dims `area` and draws the card over it: the lit edge; `✦ language
    /// servers`; a rule; a row per server, a blank, `debuggers` and a row per
    /// adapter, the selected one on the glow row with its failure reason, if
    /// any, below it; the footer.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect) {
        dim(theme, frame.buffer_mut(), area);
        let card = self.card(area);
        dialog_card(theme, frame, card);
        let full = CHROME.saturating_add(self.body_height());
        if card.height < full || card.width < 40 {
            return;
        }
        let out = frame.buffer_mut();
        let left = card.x + 4;

        let header_y = card.y + 2;
        out.set_string(
            left,
            header_y,
            "✦",
            Style::new().fg(theme.accent2).add_modifier(Modifier::BOLD),
        );
        out.set_string(left + 2, header_y, HEADER, Style::new().fg(theme.strong));

        for x in card.x + 2..card.right() - 2 {
            out[(x, card.y + 3)].set_symbol("─").set_fg(theme.line2);
        }

        for (i, row) in self.rows.iter().enumerate() {
            let y = card.y + self.row_offset(i);
            self.render_row(theme, out, card, y, row, i == self.selected);
        }
        if let Some(reason) = self.reason() {
            // Under the name, as far as the states reach: a reason is often a
            // long error, and the card doesn't grow for it.
            let x = left + 2;
            let y = card.y + self.row_offset(self.selected) + 1;
            out.set_stringn(
                x,
                y,
                reason,
                usize::from((card.right() - 4).saturating_sub(x)),
                Style::new().fg(theme.muted),
            );
        }
        if self.rows.len() > self.servers {
            // As the cast labels its sections.
            let y = card.y + self.row_offset(self.servers) - 1;
            out.set_string(
                left,
                y,
                DEBUGGERS,
                Style::new().fg(theme.muted).add_modifier(Modifier::BOLD),
            );
        }

        let hints = if self.reason().is_some() {
            FOOTER_RETRY
        } else {
            FOOTER
        };
        footer(theme, out, card, hints);
    }

    /// The name, the command in `muted`, and the state at the right in its
    /// colour. A selected row is on the glow row, its text in `strong`; in
    /// `mono` it carries no colours, so the reverse video reads.
    fn render_row(
        &self,
        theme: &Theme,
        out: &mut Buffer,
        card: Rect,
        y: u16,
        row: &Row,
        selected: bool,
    ) {
        let left = card.x + 4;
        let right = card.right() - 4;
        if selected {
            glow_row(theme, out, card.x, card.right(), y);
        }
        let plain = selected && !theme.ramps();
        let pick = |style: Style| if plain { Style::new() } else { style };

        let state = row.state.label();
        let state_x = right.saturating_sub(width_of(&state));
        out.set_string(
            state_x,
            y,
            &state,
            pick(Style::new().fg(row.state.color(theme))),
        );

        let name = pick(Style::new().fg(if selected { theme.strong } else { theme.fg }));
        out.set_stringn(
            left,
            y,
            row.name,
            usize::from(state_x.saturating_sub(left + 2)),
            name,
        );
        // Two blanks short of the state, so a long command never runs into it.
        let command_x = left + NAME_WIDTH;
        if command_x + 2 < state_x {
            let line = std::iter::once(row.command.as_str())
                .chain(row.args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            out.set_stringn(
                command_x,
                y,
                &line,
                usize::from(state_x - command_x - 2),
                pick(Style::new().fg(theme.muted)),
            );
        }
    }
}

/// A row's state: `running` beats everything. A server no longer found is
/// missing (or needs its install tool) whatever it did before, so the row
/// offers the install again; one that is found but failed this session is
/// failed rather than installed, since being on PATH didn't make it work.
fn state(running: bool, failed: bool, found: bool, install: &servers::InstallState) -> State {
    if running {
        State::Running
    } else if !found {
        match install.tool.clone().filter(|_| !install.tool_found) {
            Some(tool) => State::Needs(tool),
            None => State::Missing,
        }
    } else if failed {
        State::Failed
    } else {
        State::Installed
    }
}

/// How many cells `text` takes.
fn width_of(text: &str) -> u16 {
    u16::try_from(text.width()).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::theme::mix;

    fn exe(name: &str) -> String {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        }
    }

    fn touch(path: &Path) -> anyhow::Result<()> {
        std::fs::write(path, "")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    /// A catalog over the default servers, with `programs` on its PATH and
    /// `running` languages up.
    fn catalog(programs: &[&str], running: &[&str]) -> anyhow::Result<Catalog> {
        let dir = tempfile::tempdir()?;
        for program in programs {
            touch(&dir.path().join(exe(program)))?;
        }
        Ok(Catalog::new(
            &BTreeMap::new(),
            |lang| running.contains(&lang),
            |_| None,
            &on(Some(dir.path())),
        ))
    }

    fn never(_: &Path, _: &str) -> bool {
        false
    }

    /// A lookup in `dir` alone, with no LLVM folder and no Python importing
    /// anything.
    fn on(dir: Option<&Path>) -> Lookup<'_> {
        Lookup {
            path: dir.map(Path::as_os_str),
            pathext: None,
            llvm_bin: None,
            imports: &never,
        }
    }

    /// The server rows' names and states.
    fn states(c: &Catalog) -> Vec<(&str, String)> {
        c.rows()[..c.servers]
            .iter()
            .map(|r| (r.name, r.state.label()))
            .collect()
    }

    #[test]
    fn each_server_is_installed_missing_needs_its_tool_or_running() -> anyhow::Result<()> {
        let c = catalog(&["rust-analyzer", "npm", "gopls"], &["javascript"])?;
        assert_eq!(
            states(&c),
            [
                ("Rust", "installed".to_string()),
                ("Go", "installed".to_string()),
                ("TypeScript / JavaScript", "running".to_string()),
                ("Python", "missing".to_string()),
                ("HTML", "missing".to_string()),
                ("CSS", "missing".to_string()),
                ("SQL", "needs go".to_string()),
            ]
        );
        let commands: Vec<&str> = c.rows()[..c.servers]
            .iter()
            .map(|r| r.command.as_str())
            .collect();
        assert_eq!(
            commands,
            [
                "rust-analyzer",
                "gopls",
                "typescript-language-server",
                "pyright-langserver",
                "vscode-html-language-server",
                "vscode-css-language-server",
                "sqls",
            ]
        );
        Ok(())
    }

    #[test]
    fn a_configured_server_and_install_are_used() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        touch(&dir.path().join(exe("pip")))?;
        let config = BTreeMap::from([(
            "python".to_string(),
            LspServer {
                command: Some("pylsp".into()),
                install: Some("pip install python-lsp-server".into()),
                ..LspServer::default()
            },
        )]);
        let c = Catalog::new(&config, |_| false, |_| None, &on(Some(dir.path())));
        let python = &c.rows()[3];
        assert_eq!(python.command, "pylsp");
        assert_eq!(python.state, State::Missing);
        assert_eq!(
            python.install.as_deref(),
            Some("pip install python-lsp-server")
        );
        Ok(())
    }

    #[test]
    fn states_take_the_themes_ok_muted_and_warn() {
        let theme = Theme::default();
        assert_eq!(State::Installed.color(&theme), theme.ok);
        assert_eq!(State::Running.color(&theme), theme.ok);
        assert_eq!(State::Missing.color(&theme), theme.muted);
        assert_eq!(State::Needs("npm".into()).color(&theme), theme.warn);
        assert_eq!(State::Failed.color(&theme), theme.err);
        assert_eq!(State::Failed.label(), "failed");
    }

    /// A catalog with `programs` on its PATH, `running` languages up and
    /// `failed` ones having failed with a reason naming the language.
    fn failing(programs: &[&str], running: &[&str], failed: &[&str]) -> anyhow::Result<Catalog> {
        let dir = tempfile::tempdir()?;
        for program in programs {
            touch(&dir.path().join(exe(program)))?;
        }
        Ok(Catalog::new(
            &BTreeMap::new(),
            |lang| running.contains(&lang),
            |lang| failed.contains(&lang).then(|| format!("{lang} said no")),
            &on(Some(dir.path())),
        ))
    }

    #[test]
    fn a_failed_server_is_failed_over_installed_but_not_over_running_or_missing()
    -> anyhow::Result<()> {
        // Rust is installed and failed; TypeScript failed for one of its
        // languages; Go failed but is running again elsewhere; Python and SQL
        // failed but are gone from PATH since, Python's npm still there and
        // SQL's go not.
        let c = failing(
            &[
                "rust-analyzer",
                "gopls",
                "typescript-language-server",
                "npm",
            ],
            &["go"],
            &["rust", "go", "tsx", "python", "sql"],
        )?;
        let states: Vec<String> = states(&c).into_iter().map(|(_, s)| s).collect();
        assert_eq!(states[..4], ["failed", "running", "failed", "missing"]);
        assert_eq!(states[6], "needs go");
        let reasons: Vec<Option<&str>> = c.rows()[..c.servers]
            .iter()
            .map(|r| r.reason.as_deref())
            .collect();
        assert_eq!(
            reasons,
            [
                Some("rust said no"),
                None,
                Some("tsx said no"),
                None,
                None,
                None,
                None
            ]
        );
        Ok(())
    }

    #[test]
    fn enter_on_a_server_failed_and_gone_installs_it_again() -> anyhow::Result<()> {
        let mut c = failing(&["npm"], &[], &["python"])?;
        c.selected = 3;
        assert!(matches!(
            c.handle(Input::Action(Action::Newline)),
            Some(Step::Install(Install { name: "Python", .. }))
        ));
        Ok(())
    }

    #[test]
    fn enter_on_a_failed_row_retries_it_and_c_still_copies() -> anyhow::Result<()> {
        let programs = ["typescript-language-server", "npm"];
        let mut c = failing(&programs, &[], &["javascript"])?;
        c.selected = 2;
        assert_eq!(
            c.handle(Input::Action(Action::Newline)),
            Some(Step::Retry {
                name: "TypeScript / JavaScript",
                langs: &["typescript", "tsx", "javascript", "jsx"],
            })
        );
        assert_eq!(
            c.handle(Input::Action(Action::CatalogCopy)),
            Some(Step::Copy(
                "npm install -g typescript-language-server typescript".into()
            ))
        );
        Ok(())
    }

    #[test]
    fn the_selected_failed_row_has_its_reason_below_and_a_retry_footer() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        touch(&dir.path().join(exe("rust-analyzer")))?;
        let long = format!("Cannot start: {}", "x".repeat(100));
        let mut c = Catalog::new(
            &BTreeMap::new(),
            |_| false,
            |lang| (lang == "rust").then(|| long.clone()),
            &on(Some(dir.path())),
        );
        let theme = Theme::default();
        let area = Rect::new(0, 0, 100, 30);
        let card = c.card(area);
        // One line taller than without a reason.
        assert_eq!(card, Rect::new(7, 4, 86, 19));
        let buffer = draw(&c, &theme)?;
        let rust = card_row(&buffer, card, card.y + 4);
        assert!(rust.ends_with("failed    "), "{rust:?}");
        assert_eq!(buffer[(card.right() - 10, card.y + 4)].fg, theme.err);

        // Under the name, in `muted`, cut where the states end.
        let reason = card_row(&buffer, card, card.y + 5);
        let shown = &long[..usize::from(card.width - 10)];
        assert_eq!(reason, format!("      {shown}    "));
        assert_eq!(buffer[(card.x + 6, card.y + 5)].fg, theme.muted);
        assert_ne!(
            buffer[(card.x, card.y + 5)].bg,
            mix(theme.raised, theme.accent, 0.3)
        );
        // The rows below move down for it.
        let go = card_row(&buffer, card, card.y + 6);
        assert!(go.starts_with("    Go "), "{go:?}");
        let foot = card_row(&buffer, card, card.bottom() - 1);
        assert!(foot.starts_with(&format!("  {FOOTER_RETRY}")), "{foot:?}");

        // Off the failed row: no reason, the usual footer.
        c.handle(Input::Action(Action::Move(Motion::Down)));
        assert_eq!(c.card(area).height, 18);
        let buffer = draw(&c, &theme)?;
        let go = card_row(&buffer, card, card.y + 5);
        assert!(go.starts_with("    Go "), "{go:?}");
        let foot = card_row(&buffer, card, card.y + 17);
        assert!(foot.starts_with(&format!("  {FOOTER}")), "{foot:?}");
        Ok(())
    }

    #[test]
    fn up_and_down_stop_at_the_ends_c_copies_and_esc_closes() -> anyhow::Result<()> {
        let mut c = catalog(&[], &[])?;
        let press = |c: &mut Catalog, a: Action| c.handle(Input::Action(a));
        assert_eq!(press(&mut c, Action::Move(Motion::Up)), None);
        assert_eq!(c.selected, 0);
        assert_eq!(
            press(&mut c, Action::CatalogCopy),
            Some(Step::Copy("rustup component add rust-analyzer".into()))
        );
        for _ in 0..10 {
            press(&mut c, Action::Move(Motion::Down));
        }
        // The last row is the last adapter's.
        assert_eq!(c.selected, 9);
        assert_eq!(
            press(&mut c, Action::CatalogCopy),
            Some(Step::Copy(
                "go install github.com/go-delve/delve/cmd/dlv@latest".into()
            ))
        );
        // Typed letters do nothing here.
        assert_eq!(c.handle(Input::Text('x')), None);
        assert_eq!(press(&mut c, Action::Cancel), Some(Step::Close));
        Ok(())
    }

    #[test]
    fn c_on_a_row_without_an_install_command_says_so() {
        let config = BTreeMap::from([(
            "rust".to_string(),
            LspServer {
                command: Some("my-ra".into()),
                ..LspServer::default()
            },
        )]);
        let mut c = Catalog::new(&config, |_| false, |_| None, &on(None));
        assert_eq!(
            c.handle(Input::Action(Action::CatalogCopy)),
            Some(Step::NoInstall("Rust"))
        );
    }

    #[test]
    fn enter_installs_a_missing_row_copies_what_it_cant_run_and_skips_the_rest()
    -> anyhow::Result<()> {
        // Rust needs rustup, Go is installed, TypeScript is running, Python
        // is missing with npm there, and HTML's install needs sudo.
        let dir = tempfile::tempdir()?;
        for program in ["gopls", "npm"] {
            touch(&dir.path().join(exe(program)))?;
        }
        let config = BTreeMap::from([(
            "html".to_string(),
            LspServer {
                command: Some("vscode-html-language-server".into()),
                install: Some("sudo npm install -g vscode-langservers-extracted".into()),
                ..LspServer::default()
            },
        )]);
        let mut c = Catalog::new(
            &config,
            |lang| lang == "tsx",
            |_| None,
            &on(Some(dir.path())),
        );
        let mut enter_on = |row: usize| {
            c.selected = row;
            c.handle(Input::Action(Action::Newline))
        };
        assert_eq!(
            enter_on(0),
            Some(Step::Copy("rustup component add rust-analyzer".into()))
        );
        assert_eq!(enter_on(1), None);
        assert_eq!(enter_on(2), None);
        assert_eq!(
            enter_on(3),
            Some(Step::Install(Install {
                name: "Python",
                langs: &["python"],
                command: "pyright-langserver".into(),
                install: "npm install -g pyright".into(),
                probe: Probe::Program,
            }))
        );
        assert_eq!(
            enter_on(4),
            Some(Step::Copy(
                "sudo npm install -g vscode-langservers-extracted".into()
            ))
        );
        Ok(())
    }

    fn draw(c: &Catalog, theme: &Theme) -> anyhow::Result<Buffer> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| c.render(theme, frame, frame.area()))?;
        Ok(terminal.backend().buffer().clone())
    }

    fn card_row(buffer: &Buffer, card: Rect, y: u16) -> String {
        (card.x..card.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn the_card_is_the_casts_with_header_rule_rows_and_footer() -> anyhow::Result<()> {
        let c = catalog(&["rust-analyzer", "npm"], &[])?;
        let theme = Theme::default();
        let card = c.card(Rect::new(0, 0, 100, 30));
        assert_eq!(card, Rect::new(7, 4, 86, 18));
        let buffer = draw(&c, &theme)?;
        assert_eq!(buffer[(card.x, card.y)].symbol(), "▀");
        let header = card_row(&buffer, card, card.y + 2);
        assert!(header.starts_with("    ✦ language servers "), "{header:?}");
        assert_eq!(buffer[(card.x + 4, card.y + 2)].fg, theme.accent2);
        assert_eq!(buffer[(card.x + 5, card.y + 3)].symbol(), "─");

        let rust = card_row(&buffer, card, card.y + 4);
        assert_eq!(
            rust,
            format!("    {:<25}{:<44}installed    ", "Rust", "rust-analyzer")
        );
        // The first row is selected: on the glow, its name in `strong`.
        assert_eq!(
            buffer[(card.x, card.y + 4)].bg,
            mix(theme.raised, theme.accent, 0.3)
        );
        assert_eq!(buffer[(card.x + 4, card.y + 4)].fg, theme.strong);
        assert_eq!(buffer[(card.x + 29, card.y + 4)].fg, theme.muted);
        assert_eq!(buffer[(card.right() - 13, card.y + 4)].fg, theme.ok);

        let go = card_row(&buffer, card, card.y + 5);
        assert!(go.ends_with("needs go    "), "{go:?}");
        let state_x = card.right() - 4 - 8;
        assert_eq!(buffer[(state_x, card.y + 5)].fg, theme.warn);
        assert_eq!(buffer[(card.x, card.y + 5)].bg, theme.raised);
        let ts = card_row(&buffer, card, card.y + 6);
        assert!(ts.starts_with("    TypeScript / JavaScript  typescript-language-server"));
        assert!(ts.ends_with("missing    "), "{ts:?}");
        assert_eq!(buffer[(card.right() - 11, card.y + 6)].fg, theme.muted);
        let sql = card_row(&buffer, card, card.y + 10);
        assert!(sql.starts_with("    SQL "), "{sql:?}");

        // A blank, the `debuggers` heading as the cast labels a section, and
        // a row per adapter.
        assert_eq!(card_row(&buffer, card, card.y + 11).trim(), "");
        let heading = card_row(&buffer, card, card.y + 12);
        assert_eq!(heading.trim_end(), "    debuggers", "{heading:?}");
        assert_eq!(buffer[(card.x + 4, card.y + 12)].fg, theme.muted);
        assert!(
            buffer[(card.x + 4, card.y + 12)]
                .modifier
                .contains(Modifier::BOLD)
        );
        let debugpy = card_row(&buffer, card, card.y + 14);
        assert_eq!(
            debugpy,
            format!(
                "    {:<25}{:<41}needs python    ",
                "debugpy", "python -m debugpy.adapter"
            )
        );
        let dlv = card_row(&buffer, card, card.y + 15);
        assert!(dlv.starts_with("    dlv                      dlv dap "));

        assert_eq!(card_row(&buffer, card, card.y + 16).trim(), "");
        let foot = card_row(&buffer, card, card.y + 17);
        assert!(foot.starts_with(&format!("  {FOOTER}")), "{foot:?}");
        Ok(())
    }

    /// A catalog with `programs` on its PATH, `llvm` as LLVM's folder holding
    /// `lldb-dap` when given, and Python importing debugpy when `debugpy`.
    fn adapters(programs: &[&str], llvm: bool, debugpy: bool) -> anyhow::Result<Catalog> {
        fn yes(_: &Path, module: &str) -> bool {
            module == "debugpy"
        }
        let dir = tempfile::tempdir()?;
        for program in programs {
            touch(&dir.path().join(exe(program)))?;
        }
        let llvm_dir = tempfile::tempdir()?;
        touch(&llvm_dir.path().join(exe("lldb-dap")))?;
        let lookup = Lookup {
            path: Some(dir.path().as_os_str()),
            pathext: None,
            llvm_bin: llvm.then_some(llvm_dir.path()),
            imports: if debugpy { &yes } else { &never },
        };
        Ok(Catalog::new(&BTreeMap::new(), |_| false, |_| None, &lookup))
    }

    /// The adapter rows' names, commands with args, states and installs.
    fn adapter_rows(c: &Catalog) -> Vec<(&str, String, String, Option<&str>)> {
        c.rows()[c.servers..]
            .iter()
            .map(|r| {
                let line = std::iter::once(r.command.as_str())
                    .chain(r.args.iter().copied())
                    .collect::<Vec<_>>()
                    .join(" ");
                (r.name, line, r.state.label(), r.install.as_deref())
            })
            .collect()
    }

    const LLDB: &str = if cfg!(windows) {
        "winget install --id LLVM.LLVM -e --accept-source-agreements --accept-package-agreements"
    } else {
        "sudo apt install lldb"
    };
    const LLDB_TOOL: &str = if cfg!(windows) { "winget" } else { "apt" };
    const DEBUGPY: &str = "python -m pip install debugpy";
    const DLV: &str = "go install github.com/go-delve/delve/cmd/dlv@latest";

    #[test]
    fn debuggers_list_lldb_dap_debugpy_and_dlv_with_their_installs() -> anyhow::Result<()> {
        let c = adapters(&[], false, false)?;
        assert_eq!(c.servers, 7);
        assert_eq!(
            adapter_rows(&c),
            [
                (
                    "lldb-dap",
                    "lldb-dap".to_string(),
                    format!("needs {LLDB_TOOL}"),
                    Some(LLDB)
                ),
                (
                    "debugpy",
                    "python -m debugpy.adapter".to_string(),
                    "needs python".to_string(),
                    Some(DEBUGPY)
                ),
                (
                    "dlv",
                    "dlv dap".to_string(),
                    "needs go".to_string(),
                    Some(DLV)
                ),
            ]
        );
        // Linux's lldb comes from a `sudo` install, which only copies.
        let copy_only: Vec<bool> = c.rows()[c.servers..].iter().map(|r| r.copy_only).collect();
        assert_eq!(copy_only, [!cfg!(windows), false, false]);
        Ok(())
    }

    #[test]
    fn an_adapter_is_missing_until_found_and_debugpy_until_python_imports_it() -> anyhow::Result<()>
    {
        // The install tools are there; the adapters aren't.
        let c = adapters(&[LLDB_TOOL, "python", "go"], false, false)?;
        let states: Vec<String> = adapter_rows(&c).into_iter().map(|r| r.2).collect();
        assert_eq!(states, ["missing", "missing", "missing"]);

        // lldb-dap in LLVM's folder, a Python that imports debugpy, dlv on PATH.
        let c = adapters(&["python", "dlv"], true, true)?;
        let states: Vec<String> = adapter_rows(&c).into_iter().map(|r| r.2).collect();
        assert_eq!(states, ["installed", "installed", "installed"]);

        // Importing debugpy needs a Python to import it with.
        let c = adapters(&[], false, true)?;
        assert_eq!(adapter_rows(&c)[1].2, "needs python");
        Ok(())
    }

    #[test]
    fn enter_on_an_adapter_installs_it_as_a_server_is() -> anyhow::Result<()> {
        let mut c = adapters(&[LLDB_TOOL, "python", "go"], false, false)?;
        let mut enter_on = |row: usize| {
            c.selected = row;
            c.handle(Input::Action(Action::Newline))
        };
        let lldb = if cfg!(windows) {
            Step::Install(Install {
                name: "lldb-dap",
                langs: &[],
                command: "lldb-dap".into(),
                install: LLDB.into(),
                probe: Probe::Program,
            })
        } else {
            Step::Copy(LLDB.into())
        };
        assert_eq!(enter_on(7), Some(lldb));
        assert_eq!(
            enter_on(8),
            Some(Step::Install(Install {
                name: "debugpy",
                langs: &[],
                command: "python".into(),
                install: DEBUGPY.into(),
                probe: Probe::Module("debugpy"),
            }))
        );
        assert_eq!(
            enter_on(9),
            Some(Step::Install(Install {
                name: "dlv",
                langs: &[],
                command: "dlv".into(),
                install: DLV.into(),
                probe: Probe::Program,
            }))
        );
        // `c` copies an adapter's install too.
        assert_eq!(
            c.handle(Input::Action(Action::CatalogCopy)),
            Some(Step::Copy(DLV.into()))
        );
        Ok(())
    }

    #[test]
    fn an_install_checks_what_it_installed_the_way_its_row_does() -> anyhow::Result<()> {
        fn yes(_: &Path, module: &str) -> bool {
            module == "debugpy"
        }
        let dir = tempfile::tempdir()?;
        touch(&dir.path().join(exe("python")))?;
        let lookup = |imports: &'static dyn Fn(&Path, &str) -> bool| Lookup {
            path: Some(dir.path().as_os_str()),
            pathext: None,
            llvm_bin: None,
            imports,
        };
        assert!(lookup(&yes).found("python", Probe::Module("debugpy")));
        assert!(!lookup(&never).found("python", Probe::Module("debugpy")));
        assert!(lookup(&never).found("python", Probe::Program));
        assert!(!lookup(&yes).found("dlv", Probe::Program));

        let install = |command: &str, probe| Install {
            name: "x",
            langs: &[],
            command: command.into(),
            install: "true".into(),
            probe,
        };
        assert_eq!(
            install("dlv", Probe::Program).not_found(),
            "installed, but dlv isn't on PATH; restart your terminal"
        );
        assert_eq!(
            install("python", Probe::Module("debugpy")).not_found(),
            "installed, but python can't import debugpy"
        );
        Ok(())
    }

    #[test]
    fn mono_reverses_the_selected_row() -> anyhow::Result<()> {
        let mut c = catalog(&[], &[])?;
        let mono = Theme::named("mono").expect("mono exists");
        let card = c.card(Rect::new(0, 0, 100, 30));
        let buffer = draw(&c, &mono)?;
        for x in [card.x, card.x + 4, card.right() - 8, card.right() - 1] {
            assert!(
                buffer[(x, card.y + 4)]
                    .modifier
                    .contains(Modifier::REVERSED),
                "{x}"
            );
        }
        assert!(
            !buffer[(card.x + 4, card.y + 5)]
                .modifier
                .contains(Modifier::REVERSED)
        );
        c.handle(Input::Action(Action::Move(Motion::Down)));
        let buffer = draw(&c, &mono)?;
        assert!(
            buffer[(card.x + 4, card.y + 5)]
                .modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            !buffer[(card.x + 4, card.y + 4)]
                .modifier
                .contains(Modifier::REVERSED)
        );
        Ok(())
    }
}
