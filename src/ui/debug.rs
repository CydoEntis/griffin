//! The debug panel (glyph-debugger spec D7): while a debug session is on it
//! takes the run panel's slot, with the paused program's call stack on the left
//! and the selected frame's variables, by scope, on the right. While the program
//! runs there is no stack to show, so the body is the program's output.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::dap::{Scope, StackFrame, Variable};
use crate::keymap::Action;
use crate::theme::Theme;
use crate::ui::glow_row;
use crate::ui::run::{RunView, render_output};

const PAUSED_HINTS: &str = "tab stack/variables   F6 focus   F4 hide";
const RUNNING_HINTS: &str = "alt+F6 stop   F4 hide";

/// How deep expanded values nest before the rest is left out: a value that
/// contains itself would otherwise expand without end.
const MAX_DEPTH: usize = 32;

/// The half of the panel that has the keys.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    #[default]
    Stack,
    Variables,
}

/// What a key in the panel asks of the app, which owns the session.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Show this frame's variables and open its file at its line.
    Frame(StackFrame),
    /// Ask the adapter for the children of this reference.
    Fetch(i64),
}

/// One row of the variables side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarRow {
    pub depth: usize,
    pub name: String,
    /// `None` for a scope's heading row.
    pub value: Option<String>,
    pub kind: Option<String>,
    /// Non-zero when the row has children.
    pub reference: i64,
    pub expanded: bool,
}

/// What the panel shows and where its selections are. Reset each time the
/// program stops, since adapters number frames and references afresh then.
#[derive(Debug, Default)]
pub struct DebugPanel {
    frames: Vec<StackFrame>,
    /// The stack row the keys are on.
    stack_row: usize,
    /// The frame whose variables are shown.
    shown: usize,
    scopes: Vec<Scope>,
    /// Children fetched so far, by reference.
    children: HashMap<i64, Vec<Variable>>,
    expanded: HashSet<i64>,
    pane: Pane,
    var_row: usize,
}

impl DebugPanel {
    /// A new stop's stack, the top frame selected. Returns the frame whose
    /// scopes to ask for.
    pub fn set_frames(&mut self, frames: Vec<StackFrame>) -> Option<i64> {
        *self = DebugPanel {
            pane: self.pane,
            frames,
            ..DebugPanel::default()
        };
        self.frames.first().map(|f| f.id)
    }

    /// The program runs on: what was shown no longer holds.
    pub fn clear(&mut self) {
        self.set_frames(Vec::new());
    }

    /// The scopes of frame `frame`, if it's still the one shown. Scopes that
    /// aren't expensive start expanded; returns their references, whose
    /// variables to ask for.
    pub fn set_scopes(&mut self, frame: i64, scopes: Vec<Scope>) -> Vec<i64> {
        if self.frames.get(self.shown).map(|f| f.id) != Some(frame) {
            return Vec::new();
        }
        let open: Vec<i64> = scopes
            .iter()
            .filter(|s| !s.expensive && s.variables_reference != 0)
            .map(|s| s.variables_reference)
            .collect();
        self.expanded.extend(open.iter().copied());
        self.scopes = scopes;
        self.var_row = 0;
        open
    }

    pub fn set_variables(&mut self, reference: i64, variables: Vec<Variable>) {
        self.children.insert(reference, variables);
    }

    /// The variables side as rows: each scope, then, while it's expanded, its
    /// variables, nested the same way.
    pub fn rows(&self) -> Vec<VarRow> {
        let mut rows = Vec::new();
        for scope in &self.scopes {
            let reference = scope.variables_reference;
            let expanded = reference != 0 && self.expanded.contains(&reference);
            rows.push(VarRow {
                depth: 0,
                name: scope.name.clone(),
                value: None,
                kind: None,
                reference,
                expanded,
            });
            if expanded {
                self.push_children(reference, 1, &mut rows);
            }
        }
        rows
    }

    fn push_children(&self, reference: i64, depth: usize, rows: &mut Vec<VarRow>) {
        if depth > MAX_DEPTH {
            return;
        }
        for variable in self.children.get(&reference).into_iter().flatten() {
            let child = variable.variables_reference;
            let expanded = child != 0 && self.expanded.contains(&child);
            rows.push(VarRow {
                depth,
                name: variable.name.clone(),
                value: Some(variable.value.clone()),
                kind: variable.kind.clone(),
                reference: child,
                expanded,
            });
            if expanded {
                self.push_children(child, depth + 1, rows);
            }
        }
    }

    /// Acts on one of the panel's keys.
    pub fn handle(&mut self, action: Action) -> Option<Step> {
        match (action, self.pane) {
            (Action::DebugSwitchPane, Pane::Stack) => self.pane = Pane::Variables,
            (Action::DebugSwitchPane, Pane::Variables) => self.pane = Pane::Stack,
            (Action::DebugUp, Pane::Stack) => self.stack_row = self.stack_row.saturating_sub(1),
            (Action::DebugDown, Pane::Stack) => {
                self.stack_row = (self.stack_row + 1).min(self.frames.len().saturating_sub(1));
            }
            (Action::DebugUp, Pane::Variables) => self.var_row = self.var_row.saturating_sub(1),
            (Action::DebugDown, Pane::Variables) => {
                self.var_row = (self.var_row + 1).min(self.rows().len().saturating_sub(1));
            }
            (Action::DebugActivate, Pane::Stack) => {
                let frame = self.frames.get(self.stack_row)?.clone();
                self.shown = self.stack_row;
                self.scopes.clear();
                self.children.clear();
                self.expanded.clear();
                self.var_row = 0;
                return Some(Step::Frame(frame));
            }
            (Action::DebugActivate, Pane::Variables) => {
                let row = self.rows().into_iter().nth(self.var_row)?;
                return if row.expanded {
                    self.collapse(&row);
                    None
                } else {
                    self.expand(&row)
                };
            }
            (Action::DebugExpand, Pane::Variables) => {
                let row = self.rows().into_iter().nth(self.var_row)?;
                return self.expand(&row);
            }
            (Action::DebugCollapse, Pane::Variables) => {
                let rows = self.rows();
                let row = rows.get(self.var_row)?;
                if row.expanded {
                    self.collapse(row);
                } else if let Some(parent) = rows[..self.var_row]
                    .iter()
                    .rposition(|r| r.depth < row.depth)
                {
                    self.var_row = parent;
                }
            }
            _ => {}
        }
        None
    }

    fn expand(&mut self, row: &VarRow) -> Option<Step> {
        if row.reference == 0 || row.expanded {
            return None;
        }
        self.expanded.insert(row.reference);
        (!self.children.contains_key(&row.reference)).then_some(Step::Fetch(row.reference))
    }

    fn collapse(&mut self, row: &VarRow) {
        self.expanded.remove(&row.reference);
        self.var_row = self.var_row.min(self.rows().len().saturating_sub(1));
    }
}

/// Everything the panel is drawn from.
pub struct DebugView<'a> {
    pub panel: &'a DebugPanel,
    /// The program's output, and the name it runs under.
    pub run: Option<&'a RunView>,
    pub paused: bool,
    /// The panel has the keys.
    pub focused: bool,
}

/// Draws the panel in `area`: a title row on `surface`, then the stack and the
/// variables while paused, or the program's output while it runs. Returns
/// where the terminal cursor goes while the panel has focus.
pub fn render_debug_panel(
    theme: &Theme,
    view: &DebugView,
    area: Rect,
    frame: &mut Frame,
) -> Option<(u16, u16)> {
    if area.height == 0 || area.width == 0 {
        return None;
    }
    let out = frame.buffer_mut();
    out.set_style(area, Style::new().bg(theme.bg).fg(theme.fg));
    let title = Rect { height: 1, ..area };
    render_title(theme, view, title, out);
    let body = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };
    if body.height == 0 {
        return None;
    }
    if !view.paused || view.panel.frames.is_empty() {
        if let Some(run) = view.run {
            render_output(theme, run, body, frame);
        }
        return None;
    }
    let stack_width = (body.width * 2 / 5).max(20).min(body.width);
    let stack = Rect {
        width: stack_width,
        ..body
    };
    let divider = stack.right();
    let vars = Rect {
        x: divider.saturating_add(1),
        width: body.right().saturating_sub(divider.saturating_add(1)),
        ..body
    };
    let out = frame.buffer_mut();
    if divider < body.right() {
        for y in body.top()..body.bottom() {
            out.set_string(divider, y, "│", Style::new().fg(theme.guide));
        }
    }
    let stack_cursor = render_stack(theme, view, stack, out);
    let vars_cursor = render_variables(theme, view, vars, out);
    if !view.focused {
        return None;
    }
    match view.panel.pane {
        Pane::Stack => stack_cursor,
        Pane::Variables => vars_cursor,
    }
}

/// The title row, as the run panel's: the state glyph and word, the session's
/// name in `strong` bold, and the keys ending one cell from the right.
fn render_title(theme: &Theme, view: &DebugView, row: Rect, out: &mut Buffer) {
    out.set_style(row, Style::new().bg(theme.surface).fg(theme.text));
    let (glyph, word, color, hints) = if view.paused {
        ("‖", "paused", theme.accent2, PAUSED_HINTS)
    } else {
        ("●", "running", theme.warn, RUNNING_HINTS)
    };
    let name = view.run.map_or("debug", |run| run.name.as_str());
    let state = Style::new().fg(color);
    let right = row.right();
    let mut x = row.x + 1;
    for (text, style) in [
        (glyph, state),
        (" ", Style::new()),
        (
            name,
            Style::new().fg(theme.strong).add_modifier(Modifier::BOLD),
        ),
        ("  ", Style::new()),
        (word, state),
    ] {
        let room = usize::from(right.saturating_sub(x));
        x = out.set_stringn(x, row.y, text, room, style).0;
    }
    let width = u16::try_from(hints.width()).unwrap_or(u16::MAX);
    let hints_x = right.saturating_sub(width.saturating_add(1));
    if hints_x >= x + 3 {
        out.set_string(hints_x, row.y, hints, Style::new().fg(theme.muted));
    }
}

/// A side's label row: `muted`, or `accent` while that side has the keys.
fn label(theme: &Theme, text: &str, lit: bool, area: Rect, out: &mut Buffer) {
    let color = if lit { theme.accent } else { theme.muted };
    out.set_stringn(
        area.x + 1,
        area.y,
        text,
        usize::from(area.width.saturating_sub(1)),
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    );
}

/// The first of `count` rows to draw so that `selected` shows in `height`.
fn first_row(selected: usize, height: usize) -> usize {
    (selected + 1).saturating_sub(height.max(1))
}

/// Where a row's text goes, and in what: the theme's colours, or none on a
/// `mono` selection, whose reverse video would otherwise be undone.
fn painter(theme: &Theme, selected: bool) -> impl Fn(Style) -> Style {
    let plain = selected && !theme.ramps();
    move |style| if plain { Style::new() } else { style }
}

fn render_stack(
    theme: &Theme,
    view: &DebugView,
    area: Rect,
    out: &mut Buffer,
) -> Option<(u16, u16)> {
    let panel = view.panel;
    let lit = view.focused && panel.pane == Pane::Stack;
    label(theme, "call stack", lit, area, out);
    let list = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    let height = usize::from(list.height);
    let first = first_row(panel.stack_row, height);
    let mut cursor = None;
    for (index, stack_frame) in panel.frames.iter().enumerate().skip(first).take(height) {
        let y = list.y + u16::try_from(index - first).unwrap_or(u16::MAX);
        let selected = index == panel.stack_row;
        if selected {
            glow_row(theme, out, list.x, list.right(), y);
            cursor = Some((list.x + 1, y));
        }
        let paint = painter(theme, selected);
        let right = list.right();
        let mut x = list.x + 1;
        let mut put = |text: &str, style: Style| {
            let room = usize::from(right.saturating_sub(x));
            x = out.set_stringn(x, y, text, room, paint(style)).0;
        };
        // The frame whose variables are shown, marked as the paused line is.
        if index == panel.shown {
            put("▶ ", Style::new().fg(theme.accent2));
        } else {
            put("  ", Style::new());
        }
        let name = if selected {
            Style::new().fg(theme.strong).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.text)
        };
        put(&stack_frame.name, name);
        put("  ", Style::new());
        put(&location(stack_frame), Style::new().fg(theme.muted));
    }
    cursor
}

/// `file:line` for a frame, by its file's name.
fn location(frame: &StackFrame) -> String {
    let source = frame.source.as_ref();
    let file = source
        .and_then(|s| s.path.as_deref())
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .or_else(|| source.and_then(|s| s.name.clone()))
        .unwrap_or_else(|| "?".to_string());
    format!("{file}:{}", frame.line)
}

fn render_variables(
    theme: &Theme,
    view: &DebugView,
    area: Rect,
    out: &mut Buffer,
) -> Option<(u16, u16)> {
    if area.width == 0 {
        return None;
    }
    let panel = view.panel;
    let lit = view.focused && panel.pane == Pane::Variables;
    label(theme, "variables", lit, area, out);
    let list = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    let rows = panel.rows();
    let height = usize::from(list.height);
    let first = first_row(panel.var_row, height);
    let mut cursor = None;
    for (index, row) in rows.iter().enumerate().skip(first).take(height) {
        let y = list.y + u16::try_from(index - first).unwrap_or(u16::MAX);
        // The variables side only shows a selection while it has the keys,
        // so one glow says which side does.
        let selected = lit && index == panel.var_row;
        if selected {
            glow_row(theme, out, list.x, list.right(), y);
        }
        let paint = painter(theme, selected);
        let right = list.right();
        let indent = u16::try_from(row.depth * 2).unwrap_or(u16::MAX);
        let mut x = list.x.saturating_add(1).saturating_add(indent).min(right);
        if index == panel.var_row {
            cursor = Some((x, y));
        }
        let mut put = |text: &str, style: Style| {
            let room = usize::from(right.saturating_sub(x));
            x = out.set_stringn(x, y, text, room, paint(style)).0;
        };
        let chevron = match (row.reference != 0, row.expanded) {
            (false, _) => "  ",
            (true, false) => "▸ ",
            (true, true) => "▾ ",
        };
        put(chevron, Style::new().fg(theme.muted));
        let Some(value) = &row.value else {
            put(
                &row.name,
                Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
            );
            continue;
        };
        put(&row.name, Style::new().fg(theme.strong));
        put(" = ", Style::new().fg(theme.muted));
        put(value, Style::new().fg(theme.text));
        if let Some(kind) = row.kind.as_deref().filter(|k| !k.is_empty()) {
            put("  ", Style::new());
            put(kind, Style::new().fg(theme.muted));
        }
    }
    cursor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dap::Source;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn frame(id: i64, name: &str, file: &str, line: usize) -> StackFrame {
        StackFrame {
            id,
            name: name.into(),
            source: Some(Source {
                name: None,
                path: Some(PathBuf::from(file)),
            }),
            line,
            column: 1,
        }
    }

    fn variable(name: &str, value: &str, kind: &str, reference: i64) -> Variable {
        Variable {
            name: name.into(),
            value: value.into(),
            kind: Some(kind.into()),
            variables_reference: reference,
        }
    }

    fn scope(name: &str, reference: i64) -> Scope {
        Scope {
            name: name.into(),
            variables_reference: reference,
            expensive: false,
        }
    }

    /// A panel paused in `inner` called from `main`, its locals loaded.
    fn paused() -> DebugPanel {
        let mut panel = DebugPanel::default();
        let top = panel.set_frames(vec![
            frame(1, "inner", "/p/main.py", 3),
            frame(2, "main", "/p/main.py", 9),
        ]);
        assert_eq!(top, Some(1));
        assert_eq!(panel.set_scopes(1, vec![scope("Locals", 10)]), vec![10]);
        panel.set_variables(
            10,
            vec![
                variable("n", "3", "int", 0),
                variable("items", "[1, 2]", "list", 11),
            ],
        );
        panel
    }

    fn names(panel: &DebugPanel) -> Vec<String> {
        panel.rows().iter().map(|r| r.name.clone()).collect()
    }

    #[test]
    fn scopes_start_expanded_and_values_expand_on_demand() {
        let mut panel = paused();
        assert_eq!(names(&panel), ["Locals", "n", "items"]);
        panel.handle(Action::DebugSwitchPane);
        panel.handle(Action::DebugDown);
        panel.handle(Action::DebugDown);
        assert_eq!(panel.handle(Action::DebugExpand), Some(Step::Fetch(11)));
        panel.set_variables(11, vec![variable("0", "1", "int", 0)]);
        assert_eq!(names(&panel), ["Locals", "n", "items", "0"]);
        assert_eq!(panel.rows()[3].depth, 2);
        // Loaded once, so expanding again asks for nothing.
        panel.handle(Action::DebugCollapse);
        assert_eq!(names(&panel), ["Locals", "n", "items"]);
        assert_eq!(panel.handle(Action::DebugActivate), None);
        assert_eq!(names(&panel), ["Locals", "n", "items", "0"]);
        // Left on a leaf goes to its parent.
        panel.handle(Action::DebugDown);
        panel.handle(Action::DebugCollapse);
        assert_eq!(panel.var_row, 2);
    }

    #[test]
    fn enter_on_a_frame_shows_it_and_drops_the_old_variables() {
        let mut panel = paused();
        panel.handle(Action::DebugDown);
        let step = panel.handle(Action::DebugActivate);
        assert_eq!(step, Some(Step::Frame(frame(2, "main", "/p/main.py", 9))));
        assert!(panel.rows().is_empty());
        // A late answer for the frame no longer shown is dropped.
        assert!(panel.set_scopes(1, vec![scope("Locals", 10)]).is_empty());
        assert_eq!(panel.set_scopes(2, vec![scope("Locals", 20)]), vec![20]);
    }

    fn draw(theme: &Theme, panel: &DebugPanel, focused: bool) -> anyhow::Result<Buffer> {
        let mut terminal = Terminal::new(TestBackend::new(80, 6))?;
        let view = DebugView {
            panel,
            run: None,
            paused: true,
            focused,
        };
        terminal.draw(|f| {
            render_debug_panel(theme, &view, f.area(), f);
        })?;
        Ok(terminal.backend().buffer().clone())
    }

    fn row_text(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn frames_on_the_left_and_variables_on_the_right() -> anyhow::Result<()> {
        let theme = Theme::default();
        let buffer = draw(&theme, &paused(), false)?;
        assert!(row_text(&buffer, 0).starts_with(" ‖ debug  paused"));
        assert!(row_text(&buffer, 1).contains("call stack"));
        assert!(row_text(&buffer, 1).contains("variables"));
        let top = row_text(&buffer, 2);
        assert!(top.contains("▶ inner  main.py:3"), "{top:?}");
        assert!(top.contains("▾ Locals"), "{top:?}");
        assert!(row_text(&buffer, 3).contains("main  main.py:9"));
        assert!(row_text(&buffer, 3).contains("n = 3  int"));
        assert!(row_text(&buffer, 4).contains("▸ items = [1, 2]  list"));
        // The selected frame glows.
        assert_ne!(buffer[(0, 2)].bg, theme.bg);
        Ok(())
    }

    #[test]
    fn mono_selects_in_reverse_video() -> anyhow::Result<()> {
        let mono = Theme::named("mono").expect("mono exists");
        let mut panel = paused();
        let buffer = draw(&mono, &panel, true)?;
        assert!(buffer[(3, 2)].modifier.contains(Modifier::REVERSED));
        assert!(!buffer[(3, 3)].modifier.contains(Modifier::REVERSED));

        panel.handle(Action::DebugSwitchPane);
        panel.handle(Action::DebugDown);
        let buffer = draw(&mono, &panel, true)?;
        // Past the stack side, where `main` also ends in an `n`.
        let x = (33..80)
            .find(|&x| buffer[(x, 3)].symbol() == "n" && buffer[(x + 1, 3)].symbol() == " ")
            .expect("n = 3 on row 3");
        assert!(buffer[(x, 3)].modifier.contains(Modifier::REVERSED));
        // Its text keeps no colours of its own, so the reversal reads.
        assert_eq!(buffer[(x, 3)].fg, mono.hov);
        Ok(())
    }
}
