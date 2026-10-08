//! The language server catalog `>language servers` opens (glyph-catalog spec
//! C1, C2): a card in the cast's style listing each server, its command, and
//! whether it's installed, missing, waiting on its install tool, or running.

use std::collections::BTreeMap;
use std::ffi::OsStr;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::buffer::movement::Motion;
use crate::config::LspServer;
use crate::keymap::{Action, Input};
use crate::lsp::servers;
use crate::theme::Theme;
use crate::ui::{dialog_card, dim, footer, glow_row};

/// The card's width before clamping to the screen, and the row it drops from:
/// the cast palette's, so the catalog reads as the palette turning into it.
const WIDTH: u16 = 86;
const TOP: u16 = 4;
/// The rows other than the servers: the lit edge, a blank, the header and the
/// rule above them; a blank and the footer below.
const CHROME: u16 = 6;
const HEADER: &str = "language servers";
const FOOTER: &str = "⏎ install  c copy command  esc close";
/// Cells for the row's name, so the commands line up: the longest name and two
/// blanks.
const NAME_WIDTH: u16 = 25;

/// One row per server, in the spec's order: its name and the languages it
/// serves. The first language's table gives the command and install; any of
/// them having a server up makes the row `running`.
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

/// Where a server is, as its row says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// A server for it is up in some root.
    Running,
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
            State::Installed => "installed".into(),
            State::Needs(tool) => format!("needs {tool}"),
            State::Missing => "missing".into(),
        }
    }

    /// The colour of those words.
    pub fn color(&self, theme: &Theme) -> Color {
        match self {
            State::Running | State::Installed => theme.ok,
            State::Needs(_) => theme.warn,
            State::Missing => theme.muted,
        }
    }
}

/// One server's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: &'static str,
    /// The command that starts the server.
    pub command: String,
    pub state: State,
    /// The command that installs it, if one is known.
    pub install: Option<String>,
    /// The languages it serves, whose failed starts an install makes Glyph
    /// forget.
    pub langs: &'static [&'static str],
    /// The install command has to be run by the user (see `servers::copy_only`).
    pub copy_only: bool,
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
}

/// An install Enter asked for: what to run, and what to check once it's done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    /// The row's name, which titles the run and the messages.
    pub name: &'static str,
    pub langs: &'static [&'static str],
    /// The command that starts the server, looked up on PATH afterwards.
    pub command: String,
    /// The command that installs it.
    pub install: String,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    rows: Vec<Row>,
    selected: usize,
}

impl Catalog {
    /// The catalog for the server tables `config`, with `running` saying which
    /// languages have a server up, and programs looked up in `path` and
    /// `pathext` as `servers::find` does.
    pub fn new(
        config: &BTreeMap<String, LspServer>,
        running: impl Fn(&str) -> bool,
        path: Option<&OsStr>,
        pathext: Option<&OsStr>,
    ) -> Self {
        // Fills in any language the tables leave out; ones already filled in
        // stay as they are.
        let config = servers::with_defaults(config.clone());
        let rows = ENTRIES
            .iter()
            .map(|&(name, langs)| {
                let server = config.get(langs[0]).cloned().unwrap_or_default();
                let install = servers::install_state(&server, path, pathext);
                let state = if langs.iter().any(|lang| running(lang)) {
                    State::Running
                } else if install.command_found {
                    State::Installed
                } else if let Some(tool) = install.tool.filter(|_| !install.tool_found) {
                    State::Needs(tool)
                } else {
                    State::Missing
                };
                Row {
                    name,
                    command: server.command.unwrap_or_default(),
                    state,
                    install: install.install,
                    langs,
                    copy_only: install.copy_only,
                }
            })
            .collect();
        Catalog { rows, selected: 0 }
    }

    #[cfg(test)]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// ↑ and ↓ move the selection, stopping at either end; `c` copies the
    /// selected row's install command; Enter installs a `missing` server, or
    /// copies the command when Glyph can't run it; Esc closes.
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
                }))
            }
            Input::Action(Action::Cancel) => Some(Step::Close),
            _ => None,
        }
    }

    /// Where the card goes in `area`: the cast's width, centred, from row 4, as
    /// tall as its rows need.
    pub fn card(&self, area: Rect) -> Rect {
        let width = WIDTH.min(area.width.saturating_sub(4));
        let y = area.y + TOP.min(area.height);
        let rows = u16::try_from(self.rows.len()).unwrap_or(u16::MAX);
        let height = CHROME.saturating_add(rows).min(area.bottom() - y);
        Rect {
            x: area.x + (area.width - width) / 2,
            y,
            width,
            height,
        }
    }

    /// Dims `area` and draws the card over it: the lit edge; `✦ language
    /// servers`; a rule; a row per server, the selected one on the glow row;
    /// the footer.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect) {
        dim(theme, frame.buffer_mut(), area);
        let card = self.card(area);
        dialog_card(theme, frame, card);
        let full = CHROME + u16::try_from(self.rows.len()).unwrap_or(u16::MAX);
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
            let y = card.y + 4 + u16::try_from(i).unwrap_or(u16::MAX);
            self.render_row(theme, out, card, y, row, i == self.selected);
        }

        footer(theme, out, card, FOOTER);
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
            out.set_stringn(
                command_x,
                y,
                &row.command,
                usize::from(state_x - command_x - 2),
                pick(Style::new().fg(theme.muted)),
            );
        }
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
            Some(dir.path().as_os_str()),
            None,
        ))
    }

    fn states(c: &Catalog) -> Vec<(&str, String)> {
        c.rows().iter().map(|r| (r.name, r.state.label())).collect()
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
        let commands: Vec<&str> = c.rows().iter().map(|r| r.command.as_str()).collect();
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
        let c = Catalog::new(&config, |_| false, Some(dir.path().as_os_str()), None);
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
        assert_eq!(c.selected, 6);
        assert_eq!(
            press(&mut c, Action::CatalogCopy),
            Some(Step::Copy(
                "go install github.com/sqls-server/sqls@latest".into()
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
        let mut c = Catalog::new(&config, |_| false, None, None);
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
            Some(dir.path().as_os_str()),
            None,
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
        assert_eq!(card, Rect::new(7, 4, 86, 13));
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

        assert_eq!(card_row(&buffer, card, card.y + 11).trim(), "");
        let foot = card_row(&buffer, card, card.y + 12);
        assert!(foot.starts_with(&format!("  {FOOTER}")), "{foot:?}");
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
