// Sample project content for the Glyph prototype (fixture-like, not real file contents).
(function () {
const R = String.raw;
const FILES = {
'src/ui/status.rs': R`//! The status line: one row at the bottom of the screen.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::lsp::ServerState;
use crate::theme::Theme;

/// Segments drop off from the right as the row narrows.
const DROP_ORDER: [Segment; 3] = [Segment::Server, Segment::Language, Segment::Dirs];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    Message,
    Dirs,
    Position,
    Language,
    Server,
    Counts,
}

pub struct Status<'a> {
    pub path: &'a str,
    pub dirty: bool,
    pub message: Option<&'a str>,
    pub position: (usize, usize),
    pub language: &'a str,
    pub server: ServerState,
    pub warnings: usize,
    pub errors: usize,
}

impl Status<'_> {
    /// Ln 42, Col 17: the cursor, shown 1-based.
    pub fn position(&self) -> String {
        let (line, col) = self.position;
        format!("Ln {}, Col {}", line + 1, col + 1)
    }

    fn counts(&self, theme: &Theme) -> Vec<Span<'static>> {
        let gap = 2;
        vec![
            Span::styled(format!("⚠ {}", self.warnings), Style::new().fg(theme.working)),
            Span::raw("  "),
            Span::styled(format!("✕ {}", self.errors), Style::new().fg(theme.err)),
        ]
    }
}

pub fn render_status(theme: &Theme, status: &Status, area: Rect) -> Line<'static> {
    let width: u16 = area.width as usize;
    let mut left = Vec::new();
    if let Some(message) = status.message {
        left.push(Span::styled(format!(" {message}"), Style::new().fg(theme.text)));
    }
    let mut right = vec![
        Span::styled(status.path.to_string(), Style::new().fg(theme.strong)),
        Span::raw("  "),
        Span::raw(status.position()),
    ];
    for segment in DROP_ORDER {
        if fits(&left, &right, width) {
            break;
        }
        drop_segment(&mut right, segment);
    }
    let bold = Style::new().add_modifier(Modifier::BOLD);
    Line::from([left, right].concat()).style(bold.bg(theme.sidebar_bg))
}

fn fits(left: &[Span], right: &[Span], width: usize) -> bool {
    let used: usize = left.iter().chain(right).map(|s| s.width()).sum();
    used <= width
}`,
'src/app.rs': R`//! App: all editor state, the event loop and the top-level render.

use std::path::PathBuf;

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::theme::Theme;
use crate::ui::status::{render_status, Status};
use crate::workspace::Tree;

pub struct App {
    pub theme: Theme,
    pub tree: Tree,
    pub root: PathBuf,
    pub message: Option<String>,
    tabs: Tabs,
    splits: Vec<Split>,
    find: Option<FindBar>,
    run: Option<RunView>,
}

impl App {
    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let panes = Panes::compute(area, self.tree.visible, self.splits.len(), self.run.as_ref().map(|run| run.visible).unwrap_or(false), self.find.is_some() || self.prompt.is_some());
        if let Some(tree) = panes.tree {
            render_tree(&self.theme, &self.tree, self.focus == Focus::Tree, tree, frame);
            render_divider(&self.theme, panes.tree_divider, frame);
        }
        for (index, split) in self.splits.iter().enumerate() {
            let focused = self.focus == Focus::Split(index);
            render_tabs(&self.theme, &split.labels(&self.tabs), split.active, focused, panes.tabs[index], frame);
            render_buffer(&self.theme, self.tabs.buffer(split.active), panes.editors[index], focused, frame);
        }
        let status = Status {
            path: self.tabs.active().display_path(&self.root),
            dirty: self.tabs.active().dirty(),
            message: self.message.as_deref(),
            position: self.tabs.active().cursor_position(),
            language: self.tabs.active().language().name(),
            server: self.lsp.state(self.tabs.active().language()),
            warnings: self.diagnostic_count(Severity::Warning),
            errors: self.diagnostic_count(Severity::Error),
        };
        frame.render_widget(render_status(&self.theme, &status, panes.status), panes.status);
    }
}`,
'src/theme.rs': R`//! Hydra's 11 themes and [theme_overrides].

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Editor ground.
    pub bg: Color,
    pub fg: Color,
    pub muted: Color,
    pub accent: Color,
    /// The cursor's line, behind the gutter and the text.
    pub current_line_bg: Color,
    /// Line numbers; the cursor's own is drawn in gutter_active_fg.
    pub gutter_fg: Color,
    pub gutter_active_fg: Color,
    pub find_match_bg: Color,
    pub find_current_bg: Color,
    pub sidebar_bg: Color,
    pub selection_bg: Color,
    pub syntax: Syntax,
}

impl Theme {
    pub fn highlight(bg: Color, fg: Color) -> Style {
        Style::new().fg(bg).bg(fg).add_modifier(Modifier::REVERSED)
    }
}`,
'src/main.rs': R`use anyhow::Result;

mod app;
mod theme;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut terminal = ratatui::init();
    let result = app::run(&mut terminal, args).await;
    ratatui::restore();
    result
}`,
'src/ui/mod.rs': R`pub mod completion;
pub mod find;
pub mod status;

use ratatui::widgets::Block;

use crate::theme::Theme;

/// Popups keep a border; dialogs sit on the dimmed screen without one.
pub fn card_block(theme: &Theme) -> Block<'static> {
    Block::bordered().border_style(theme.border).style(theme.card)
}`,
'src/lsp/mod.rs': R`//! Language servers: one per language per project root.

pub mod client;

pub enum ServerState {
    Starting,
    Ready,
    NotFound,
    Crashed(Option<i32>),
}`,
'Cargo.toml': R`[package]
name = "glyph"
version = "0.1.0"
edition = "2024"

[dependencies]
ratatui = "0.30"
crossterm = { version = "0.29", features = ["event-stream"] }
tokio = { version = "1", features = ["full"] }
ropey = "1.6"
tree-sitter = "0.27"
nucleo = "0.5"`,
'.glyph.toml': R`# Run commands for F5.
[[run]]
name = "dev"
command = "cargo run -- tests/fixtures/project"

[[run]]
name = "test"
command = "cargo test"`,
'README.md': R`# Glyph

A non-modal terminal text editor, sibling of Hydra.

    glyph .          open the current folder
    glyph file.rs    open one file`,
};

const PATHS = ['.glyph.toml', 'Cargo.toml', 'README.md', 'docs/PLANNING.md', 'docs/features/glyph-v1/spec.md',
  'src/app.rs', 'src/backup.rs', 'src/clipboard.rs', 'src/config.rs', 'src/keymap.rs', 'src/main.rs', 'src/save.rs', 'src/search.rs', 'src/theme.rs',
  'src/buffer/edit.rs', 'src/buffer/history.rs', 'src/buffer/mod.rs', 'src/buffer/movement.rs', 'src/buffer/selection.rs',
  'src/highlight/mod.rs', 'src/lsp/client.rs', 'src/lsp/mod.rs', 'src/lsp/position.rs', 'src/lsp/servers.rs', 'src/lsp/transport.rs',
  'src/run/detect.rs', 'src/run/mod.rs', 'src/run/tree.rs',
  'src/ui/completion.rs', 'src/ui/confirm.rs', 'src/ui/find.rs', 'src/ui/hover.rs', 'src/ui/mod.rs', 'src/ui/picker.rs', 'src/ui/prompt.rs', 'src/ui/run.rs', 'src/ui/search.rs', 'src/ui/status.rs', 'src/ui/tabs.rs', 'src/ui/tree.rs',
  'src/view/mod.rs', 'src/workspace/mod.rs', 'src/workspace/ops.rs', 'src/workspace/tree.rs', 'src/workspace/walk.rs',
  'tests/find.rs', 'tests/lsp.rs', 'tests/split.rs', 'tests/tabs.rs'];

// [path, line text contains, range text, severity, message]
const DIAGS = [
  ['src/ui/status.rs', 'let width: u16', 'area.width as usize', 'err', 'mismatched types: expected u16, found usize'],
  ['src/ui/status.rs', 'let gap = 2;', 'gap', 'warn', 'unused variable: gap'],
  ['src/ui/status.rs', 'drop_segment(&mut', 'drop_segment', 'err', 'cannot find function drop_segment in this scope'],
  ['src/ui/status.rs', '[left, right].concat()', 'concat', 'info', 'concat allocates a new Vec; extend left instead'],
];

const HOVER = {
  code: ['pub fn render_status(theme: &Theme, status: &Status, area: Rect)', '    -> Line<\'static>'],
  doc: 'Draws the one-row status line: the message on the left, then path, position, language, server and counts on the right, dropping segments from the right as the row narrows.',
};

const COMPLETIONS = [
  ['field', 'accent', 'Color'], ['field', 'acc_ink', 'Color'], ['field', 'bg', 'Color'], ['field', 'border', 'Color'], ['field', 'border_active', 'Color'],
  ['field', 'btn', 'Color'], ['field', 'card', 'Color'], ['field', 'card2', 'Color'], ['field', 'current_line_bg', 'Color'], ['field', 'err', 'Color'],
  ['field', 'fg', 'Color'], ['field', 'find_match_bg', 'Color'], ['fn', 'highlight', 'fn(Color, Color) -> Style'], ['field', 'muted', 'Color'],
  ['field', 'selection_bg', 'Color'], ['field', 'sidebar_bg', 'Color'], ['field', 'syntax', 'Syntax'], ['field', 'working', 'Color'],
];

const RUNS = [
  { name: 'dev', cmd: 'cargo run -- tests/fixtures/project', src: '.glyph.toml' },
  { name: 'test', cmd: 'cargo test', src: '.glyph.toml' },
  { name: 'cargo run', cmd: 'cargo run', src: 'detected' },
];
const L = (...p) => p.map(x => typeof x === 'string' ? [x] : x);
const BUILD = kind => [
  L(['   Compiling', 'ansi.green', 1], ' glyph v0.1.0 (C:\\src\\glyph)'),
  L(['    Finished', 'ansi.green', 1], ' `' + kind + '` profile [unoptimized + debuginfo] target(s) in 3.84s'),
];
const FEED = {
  dev: [...BUILD('dev'), L(['     Running', 'ansi.green', 1], ' `target\\debug\\glyph-demo.exe tests/fixtures/project`')],
  devLoop: [
    ['INFO', 'ansi.green', 'watching 7 files under tests/fixtures/project'],
    ['INFO', 'ansi.green', 'listening on 127.0.0.1:8080'],
    ['WARN', 'ansi.yellow', 'slow frame: 21 ms (budget 16 ms)'],
    ['INFO', 'ansi.green', 'GET /health 200 0.4 ms'],
    ['INFO', 'ansi.green', 'reloaded src/ui/status.rs (2.1 KB)'],
    ['ERROR', 'ansi.red', 'GET /api/files 500: permission denied (os error 5)'],
  ],
  testOk: [...BUILD('test'), L(['     Running', 'ansi.green', 1], ' tests/find.rs (target\\debug\\deps\\find-3f2a9c)'), L(''), L('running 6 tests'),
    ...['next_and_previous_wrap', 'toggles_research', 'typing_finds_the_first_match', 'tab_moves_between_fields', 'a_pasted_tab_moves', 'research_from_picks_the_next'].map(t => L('test ' + t + ' ... ', ['ok', 'ansi.green'])),
    L(''), L('test result: ', ['ok', 'ansi.green'], '. 6 passed; 0 failed; 0 ignored; finished in 0.41s')],
  testFail: [...BUILD('test'), L(['     Running', 'ansi.green', 1], ' tests/find.rs (target\\debug\\deps\\find-3f2a9c)'), L(''), L('running 6 tests'),
    L('test next_and_previous_wrap ... ', ['ok', 'ansi.green']), L('test toggles_research ... ', ['ok', 'ansi.green']),
    L('test typing_finds_the_first_match ... ', ['FAILED', 'ansi.red']), L('test tab_moves_between_fields ... ', ['ok', 'ansi.green']),
    L(''), L('failures:'), L(''), L('---- typing_finds_the_first_match stdout ----'),
    L("thread 'typing_finds_the_first_match' panicked at tests/find.rs:42:5:"), L('assertion `left == right` failed'),
    L(['  left', 'ansi.cyan'], ': Some(0..3)'), L([' right', 'ansi.cyan'], ': Some(8..11)'), L(''),
    L('test result: ', ['FAILED', 'ansi.red'], '. 3 passed; 1 failed; 0 ignored; finished in 0.38s'),
    L(['error', 'ansi.red', 1], ': test failed, to rerun pass `--test find`')],
};
window.GriffinContent = { FILES, PATHS, DIAGS, HOVER, COMPLETIONS, RUNS, FEED };
})();
