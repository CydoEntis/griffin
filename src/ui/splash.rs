//! The splash Glyph opens on when it's started with nothing to edit
//! (glyph-splash spec S2, as in `design/splash.png`): no card, just one block
//! centred in the editor area: the `glyph` wordmark in half-block letters on a
//! soft glow, a ramp rule, the project path, and three actions to pick from.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::theme::{Theme, grad, mix};
use crate::ui::glow_row;
use crate::ui::tree::{cut, cut_front};

/// The wordmark: `glyph` drawn in half-block pixels, two pixel rows per text
/// row, so the descenders of `g`, `y` and `p` get a row of their own. Every row
/// has `WORD_W` characters.
const WORDMARK: [&str; 5] = [
    concat!(
        "     ", "  ", "▀█ ", "  ", "     ", "  ", "     ", "  ", "█    "
    ),
    concat!(
        "▄▀▀▀█",
        "  ",
        " █ ",
        "  ",
        "█   █",
        "  ",
        "█▀▀▀▄",
        "  ",
        "█▀▀▀▄"
    ),
    concat!(
        "█   █",
        "  ",
        " █ ",
        "  ",
        "█   █",
        "  ",
        "█   █",
        "  ",
        "█   █"
    ),
    concat!(
        "▀▄▄▄█",
        "  ",
        " █▄",
        "  ",
        "▀▄▄▄█",
        "  ",
        "█▄▄▄▀",
        "  ",
        "█   █"
    ),
    concat!(
        "▄▄▄▄▀",
        "  ",
        "   ",
        "  ",
        "▄▄▄▄▀",
        "  ",
        "█    ",
        "  ",
        "     "
    ),
];

/// The wordmark's width in cells.
const WORD_W: u16 = 31;

/// The least width the big wordmark is drawn in: it and a cell of glow on
/// either side.
const WORD_ROOM: u16 = WORD_W + 4;

/// The rule's width: as wide as the glow, so the two end together.
const RULE_W: u16 = 63;

/// How far the glow reaches from the wordmark's centre, in cells across and
/// rows down; cells are about twice as tall as wide, so this is near round.
const GLOW_RX: f64 = 34.0;
const GLOW_RY: f64 = 4.5;

/// How far a cell at the glow's centre is blended toward `accent2`. Kept low:
/// the glow should read as light behind the letters, not a panel.
const GLOW: f64 = 0.14;

/// The action rows' width; the selected row's glow runs across all of it.
const ROW_W: u16 = 56;

/// Where a row's parts start, from its left: the `✦`, the label, the hint.
const MARK_X: u16 = 2;
const LABEL_X: u16 = 4;
const HINT_X: u16 = 22;

/// Where the key sits, from the row's right edge.
const KEY_FROM_RIGHT: u16 = 2;

/// One of the splash's actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    NewFile,
    NewDirectory,
    OpenDirectory,
}

impl Item {
    /// The rows, top to bottom.
    pub const ALL: [Item; 3] = [Item::NewFile, Item::NewDirectory, Item::OpenDirectory];

    pub fn label(self) -> &'static str {
        match self {
            Item::NewFile => "New file",
            Item::NewDirectory => "New directory",
            Item::OpenDirectory => "Open directory",
        }
    }

    /// The letter shown at the row's right; the keymap binds the same one.
    pub fn key(self) -> &'static str {
        match self {
            Item::NewFile => "n",
            Item::NewDirectory => "d",
            Item::OpenDirectory => "o",
        }
    }

    /// What the row does, after its label: where a new file or folder goes.
    fn hint(self, project: &str, room: usize) -> String {
        match self {
            Item::NewFile | Item::NewDirectory => {
                format!("in {}", cut_front(project, room.saturating_sub(3)))
            }
            Item::OpenDirectory => cut("choose a folder", room),
        }
    }
}

/// What one row of the splash holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// One of the wordmark's five rows.
    Word(usize),
    /// `✦ glyph` on one row, for when the wordmark doesn't fit.
    Brand,
    Rule,
    Blank,
    Path,
    Item(Item),
}

use Line::{Blank, Brand, Path, Rule, Word};

/// One size of the splash: its rows, and whether the action rows carry hints.
struct Step {
    lines: &'static [Line],
    hints: bool,
}

impl Step {
    fn big(&self) -> bool {
        self.lines.contains(&Word(0))
    }

    fn fits(&self, area: Rect) -> bool {
        self.lines.len() <= usize::from(area.height)
            && (!self.big() || area.width >= WORD_ROOM)
            && (!self.hints || area.width >= ROW_W)
    }
}

const FULL: &[Line] = &[
    Word(0),
    Word(1),
    Word(2),
    Word(3),
    Word(4),
    Rule,
    Blank,
    Path,
    Blank,
    Line::Item(Item::NewFile),
    Blank,
    Line::Item(Item::NewDirectory),
    Blank,
    Line::Item(Item::OpenDirectory),
];

const SMALL: &[Line] = &[
    Brand,
    Blank,
    Path,
    Blank,
    Line::Item(Item::NewFile),
    Blank,
    Line::Item(Item::NewDirectory),
    Blank,
    Line::Item(Item::OpenDirectory),
];

const NO_PATH: &[Line] = &[
    Brand,
    Blank,
    Line::Item(Item::NewFile),
    Blank,
    Line::Item(Item::NewDirectory),
    Blank,
    Line::Item(Item::OpenDirectory),
];

const TIGHT: &[Line] = &[
    Brand,
    Line::Item(Item::NewFile),
    Line::Item(Item::NewDirectory),
    Line::Item(Item::OpenDirectory),
];

/// The splash's sizes, biggest first: with less room the wordmark becomes
/// `✦ glyph`, then the hints go, then the path (spec S2). The last is the least
/// the splash can be; with fewer rows than that it's cut off at the bottom.
const STEPS: &[Step] = &[
    Step {
        lines: FULL,
        hints: true,
    },
    Step {
        lines: SMALL,
        hints: true,
    },
    Step {
        lines: SMALL,
        hints: false,
    },
    Step {
        lines: NO_PATH,
        hints: false,
    },
    Step {
        lines: TIGHT,
        hints: false,
    },
];

/// Where the splash goes in an editor area.
struct Layout {
    step: &'static Step,
    /// The row the first line is on.
    top: u16,
    /// The action rows' span across the area.
    row_x: u16,
    row_w: u16,
}

impl Layout {
    fn new(area: Rect) -> Layout {
        let step = STEPS
            .iter()
            .find(|step| step.fits(area))
            .unwrap_or(&STEPS[STEPS.len() - 1]);
        let height = u16::try_from(step.lines.len()).unwrap_or(u16::MAX);
        let row_w = ROW_W.min(area.width);
        Layout {
            step,
            top: area.y + area.height.saturating_sub(height) / 2,
            row_x: centred(area, row_w),
            row_w,
        }
    }

    /// Each line with the row it's on, as far as the area goes.
    fn rows(&self, area: Rect) -> impl Iterator<Item = (u16, Line)> + '_ {
        (self.top..area.bottom()).zip(self.step.lines.iter().copied())
    }
}

/// The x that centres something `width` wide in `area`.
fn centred(area: Rect, width: u16) -> u16 {
    area.x + area.width.saturating_sub(width) / 2
}

/// The splash's state: which row Enter runs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Splash {
    selected: usize,
}

impl Splash {
    pub fn selected(&self) -> Item {
        Item::ALL[self.selected % Item::ALL.len()]
    }

    pub fn select(&mut self, item: Item) {
        self.selected = Item::ALL.iter().position(|&i| i == item).unwrap_or(0);
    }

    /// Moves the selection `delta` rows, wrapping at either end.
    pub fn move_by(&mut self, delta: isize) {
        let len = Item::ALL.len() as isize;
        // `rem_euclid` keeps a step up from the first row on the last one.
        let at = (self.selected as isize + delta).rem_euclid(len);
        self.selected = usize::try_from(at).unwrap_or(0);
    }

    /// The action whose row is at `at`, for a click.
    pub fn item_at(area: Rect, at: Position) -> Option<Item> {
        if !area.contains(at) {
            return None;
        }
        let layout = Layout::new(area);
        if at.x < layout.row_x || at.x >= layout.row_x + layout.row_w {
            return None;
        }
        layout.rows(area).find_map(|(y, line)| match line {
            Line::Item(item) if y == at.y => Some(item),
            _ => None,
        })
    }

    /// Draws the splash centred in `area`, naming `project` (the brand label:
    /// the home folder shown as `~`) under the wordmark and in the hints.
    pub fn render(&self, theme: &Theme, project: &str, frame: &mut Frame, area: Rect) {
        let area = area.intersection(frame.area());
        if area.is_empty() {
            return;
        }
        let buf = frame.buffer_mut();
        // The splash has the editor area to itself, so whatever was drawn there
        // before is wiped to the editor ground.
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].reset();
            }
        }
        buf.set_style(area, Style::new().bg(theme.bg));
        let layout = Layout::new(area);
        if layout.step.big() && theme.ramps() {
            glow(theme, buf, area, layout.top);
        }
        for (y, line) in layout.rows(area) {
            match line {
                Word(row) => wordmark_row(theme, buf, area, row, y),
                Brand => brand(theme, buf, area, y),
                Rule => rule(theme, buf, area, y),
                Blank => {}
                Path => path(theme, buf, area, project, y),
                Line::Item(item) => self.item_row(theme, buf, &layout, project, item, y),
            }
        }
    }

    /// One action row: the label in `text`, the hint in `muted`, the key at the
    /// right in `muted`. The selected row lies on the dialog glow with `✦` in
    /// `accent2`, its label bold `strong` and its key bold `accent`.
    fn item_row(
        &self,
        theme: &Theme,
        buf: &mut Buffer,
        layout: &Layout,
        project: &str,
        item: Item,
        y: u16,
    ) {
        let selected = item == self.selected();
        let (x0, x1) = (layout.row_x, layout.row_x + layout.row_w);
        if selected {
            glow_row(theme, buf, x0, x1, y);
        }
        // `mono` reverses the selected row, which only reads with no colours of
        // its own on it.
        let plain = selected && !theme.ramps();
        let pick = |style: Style| if plain { Style::new() } else { style };
        let bold_if = |style: Style| {
            if selected {
                style.add_modifier(Modifier::BOLD)
            } else {
                style
            }
        };
        let key_x = x1.saturating_sub(KEY_FROM_RIGHT);
        let label_x = x0 + LABEL_X;
        if label_x >= key_x {
            return;
        }
        if selected {
            buf.set_string(x0 + MARK_X, y, "✦", pick(Style::new().fg(theme.accent2)));
        }
        let label_fg = if selected { theme.strong } else { theme.text };
        let room = usize::from(key_x.saturating_sub(label_x + 1));
        let label = cut(item.label(), room);
        buf.set_string(label_x, y, label, pick(bold_if(Style::new().fg(label_fg))));
        let hint_x = x0 + HINT_X;
        if layout.step.hints && hint_x + 2 < key_x {
            let room = usize::from(key_x - hint_x - 2);
            let hint = item.hint(project, room);
            buf.set_string(hint_x, y, hint, pick(Style::new().fg(theme.muted)));
        }
        let key_fg = if selected { theme.accent } else { theme.muted };
        buf.set_string(key_x, y, item.key(), pick(bold_if(Style::new().fg(key_fg))));
    }
}

/// A soft light behind the wordmark: the ground blended toward `accent2`,
/// strongest at the wordmark's centre and fading to nothing at an ellipse
/// around it.
fn glow(theme: &Theme, buf: &mut Buffer, area: Rect, top: u16) {
    let cx = f64::from(centred(area, WORD_W)) + f64::from(WORD_W) / 2.0;
    let cy = f64::from(top) + WORDMARK.len() as f64 / 2.0;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let dx = (f64::from(x) + 0.5 - cx) / GLOW_RX;
            let dy = (f64::from(y) + 0.5 - cy) / GLOW_RY;
            let d = (dx * dx + dy * dy).sqrt();
            if d < 1.0 {
                // Squared, so the light falls off fast near its edge rather
                // than stopping at a visible line.
                let t = GLOW * (1.0 - d).powi(2);
                buf[(x, y)].set_bg(mix(theme.bg, theme.accent2, t));
            }
        }
    }
}

/// The colour of the wordmark's column `col`, along accent → accent2.
fn word_color(theme: &Theme, col: u16) -> Color {
    grad(
        &[theme.accent, theme.accent2],
        f64::from(col) / f64::from(WORD_W - 1),
    )
}

/// One row of the big wordmark. Blank pixels are left alone so the glow shows
/// through them; in `mono` the letters take the terminal's own colour.
fn wordmark_row(theme: &Theme, buf: &mut Buffer, area: Rect, row: usize, y: u16) {
    let x0 = centred(area, WORD_W);
    for (col, c) in (0u16..).zip(WORDMARK[row].chars()) {
        let x = x0 + col;
        if c == ' ' || x >= area.right() {
            continue;
        }
        let style = if theme.ramps() {
            Style::new().fg(word_color(theme, col))
        } else {
            Style::new()
        };
        buf[(x, y)]
            .set_symbol(c.encode_utf8(&mut [0; 4]))
            .set_style(style);
    }
}

/// The rule under the wordmark: `─` along the same ramp, fading into the
/// ground at both ends; plain in `mono`.
fn rule(theme: &Theme, buf: &mut Buffer, area: Rect, y: u16) {
    let width = RULE_W.min(area.width);
    let x0 = centred(area, width);
    let span = f64::from(width.saturating_sub(1).max(1));
    for i in 0..width {
        let t = f64::from(i) / span;
        let cell = &mut buf[(x0 + i, y)];
        cell.set_symbol("─");
        if theme.ramps() {
            // Full strength over the middle half, fading over each outer
            // quarter to the cell's own ground.
            let strength = (t.min(1.0 - t) * 4.0).min(1.0);
            let ramp = grad(&[theme.accent, theme.accent2], t);
            cell.set_fg(mix(cell.bg, ramp, strength));
        }
    }
}

/// `✦ glyph`: the tree brand row's wordmark, its letters ramping accent →
/// accent2 in bold, for when the big one doesn't fit.
fn brand(theme: &Theme, buf: &mut Buffer, area: Rect, y: u16) {
    if area.width < 7 {
        return;
    }
    let x = centred(area, 7);
    buf.set_string(x, y, "✦", Style::new().fg(theme.accent2));
    for (i, c) in (0u16..).zip(["g", "l", "y", "p", "h"]) {
        let fg = grad(&[theme.accent, theme.accent2], f64::from(i) / 4.0);
        let style = Style::new().fg(fg).add_modifier(Modifier::BOLD);
        buf.set_string(x + 2 + i, y, c, style);
    }
}

/// The project path, centred: its last folder in bold `strong`, the rest in
/// `muted`, its start cut with `…` when it doesn't fit.
fn path(theme: &Theme, buf: &mut Buffer, area: Rect, project: &str, y: u16) {
    let room = usize::from(ROW_W.min(area.width));
    let shown = cut_front(project, room);
    // The last folder starts after the last separator; a path cut inside its
    // last folder is all last folder.
    let split = shown.rfind(['/', '\\']).map_or(0, |i| i + 1);
    let (head, last) = shown.split_at(split);
    let width = u16::try_from(shown.width()).unwrap_or(u16::MAX);
    let x = centred(area, width);
    buf.set_string(x, y, head, Style::new().fg(theme.muted));
    let last_x = x + u16::try_from(head.width()).unwrap_or(u16::MAX);
    let strong = Style::new().fg(theme.strong).add_modifier(Modifier::BOLD);
    buf.set_string(last_x, y, last, strong);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const PROJECT: &str = "~/src";

    fn draw_with(
        splash: &Splash,
        theme: &Theme,
        project: &str,
        w: u16,
        h: u16,
    ) -> anyhow::Result<Buffer> {
        let mut terminal = Terminal::new(TestBackend::new(w, h))?;
        terminal.draw(|frame| splash.render(theme, project, frame, Rect::new(0, 0, w, h)))?;
        Ok(terminal.backend().buffer().clone())
    }

    fn draw(splash: &Splash, theme: &Theme, w: u16, h: u16) -> anyhow::Result<Buffer> {
        draw_with(splash, theme, PROJECT, w, h)
    }

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .trim()
            .to_string()
    }

    fn rows(buf: &Buffer) -> Vec<String> {
        (0..buf.area.height)
            .map(|y| row(buf, y))
            .filter(|text| !text.is_empty())
            .collect()
    }

    /// The column `text` starts at on row `y`.
    fn col(buf: &Buffer, y: u16, text: &str) -> Option<u16> {
        let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
        let at = line.find(text)?;
        u16::try_from(line[..at].chars().count()).ok()
    }

    /// RGB distance, for "closer to" checks on blended colours.
    fn distance(a: Color, b: Color) -> f64 {
        let rgb = |c: Color| match c {
            Color::Rgb(r, g, b) => [f64::from(r), f64::from(g), f64::from(b)],
            _ => [0.0; 3],
        };
        let (a, b) = (rgb(a), rgb(b));
        (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
    }

    // In 100 × 26 the 14-row splash starts on row 6: the wordmark on rows 6–10,
    // the rule on 11, the path on 13 and the actions on 15, 17 and 19.
    const TOP: u16 = 6;
    const RULE_Y: u16 = 11;
    const PATH_Y: u16 = 13;
    const ITEM_YS: [u16; 3] = [15, 17, 19];
    /// The wordmark is centred: (100 − 31) / 2.
    const WORD_X: u16 = 34;
    /// The action rows are centred too: (100 − 56) / 2.
    const ROW_X: u16 = 22;

    #[test]
    fn selection_wraps_both_ways() {
        let mut splash = Splash::default();
        assert_eq!(splash.selected(), Item::NewFile);
        splash.move_by(-1);
        assert_eq!(splash.selected(), Item::OpenDirectory);
        splash.move_by(1);
        assert_eq!(splash.selected(), Item::NewFile);
        splash.move_by(2);
        assert_eq!(splash.selected(), Item::OpenDirectory);
    }

    #[test]
    fn every_wordmark_row_is_as_wide_as_the_wordmark() {
        for row in WORDMARK {
            assert_eq!(row.chars().count(), usize::from(WORD_W), "{row:?}");
            assert_eq!(row.width(), usize::from(WORD_W), "{row:?}");
        }
    }

    #[test]
    fn the_wordmark_is_the_bitmap_with_each_column_on_the_ramp() -> anyhow::Result<()> {
        let theme = Theme::default();
        let buf = draw(&Splash::default(), &theme, 100, 26)?;
        for (y, bitmap) in (TOP..).zip(WORDMARK) {
            let drawn: String = (WORD_X..WORD_X + WORD_W)
                .map(|x| buf[(x, y)].symbol())
                .collect();
            assert_eq!(drawn, bitmap, "row {y}");
            for (col, c) in (0u16..).zip(bitmap.chars()) {
                if c != ' ' {
                    let cell = &buf[(WORD_X + col, y)];
                    assert_eq!(cell.fg, word_color(&theme, col), "column {col}, row {y}");
                }
            }
        }
        // The first column is accent, the last accent2.
        assert_eq!(buf[(WORD_X, TOP + 1)].fg, theme.accent);
        assert_eq!(buf[(WORD_X + WORD_W - 1, TOP + 1)].fg, theme.accent2);
        Ok(())
    }

    #[test]
    fn the_glow_leans_toward_accent2_and_fades_with_distance() -> anyhow::Result<()> {
        let theme = Theme::default();
        let buf = draw(&Splash::default(), &theme, 100, 26)?;
        // Inside the `g`'s bowl, near the centre: blended, toward accent2.
        let near = buf[(WORD_X + 15, TOP + 2)].bg;
        let mid = buf[(WORD_X - 8, TOP + 2)].bg;
        let far = buf[(2, TOP + 2)].bg;
        assert_ne!(near, theme.bg);
        assert!(distance(near, theme.accent2) < distance(theme.bg, theme.accent2));
        assert!(distance(near, theme.bg) > distance(mid, theme.bg));
        assert_ne!(mid, theme.bg);
        assert_eq!(far, theme.bg);
        // Nor does it reach the action rows.
        assert_eq!(buf[(ROW_X + 30, ITEM_YS[2])].bg, theme.bg);

        let mono = Theme::named("mono").expect("mono exists");
        let buf = draw(&Splash::default(), &mono, 100, 26)?;
        for y in 0..ITEM_YS[0] {
            for x in 0..100 {
                assert_eq!(buf[(x, y)].bg, mono.bg, "({x}, {y})");
            }
        }
        Ok(())
    }

    #[test]
    fn the_rule_runs_the_ramp_and_fades_at_both_ends() -> anyhow::Result<()> {
        let theme = Theme::default();
        let buf = draw(&Splash::default(), &theme, 100, 26)?;
        // 63 wide, centred: from 18 to 80.
        assert_eq!(row(&buf, RULE_Y), "─".repeat(63));
        assert_eq!(col(&buf, RULE_Y, "─"), Some(18));
        let middle = buf[(49, RULE_Y)].fg;
        assert_eq!(middle, grad(&[theme.accent, theme.accent2], 0.5));
        for end in [18, 80] {
            let cell = &buf[(end, RULE_Y)];
            assert_eq!(cell.fg, cell.bg, "column {end} fades out");
        }
        let near_end = &buf[(22, RULE_Y)];
        assert!(distance(near_end.fg, near_end.bg) < distance(middle, buf[(49, RULE_Y)].bg));
        Ok(())
    }

    #[test]
    fn the_path_dims_all_but_its_last_folder_and_is_cut_at_its_start() -> anyhow::Result<()> {
        let theme = Theme::default();
        let buf = draw(&Splash::default(), &theme, 100, 26)?;
        assert_eq!(row(&buf, PATH_Y), "~/src");
        let x = col(&buf, PATH_Y, "~/src").expect("the path is shown");
        assert_eq!(x, 47);
        for at in [x, x + 1] {
            assert_eq!(buf[(at, PATH_Y)].fg, theme.muted);
        }
        for at in x + 2..x + 5 {
            assert_eq!(buf[(at, PATH_Y)].fg, theme.strong);
            assert!(buf[(at, PATH_Y)].modifier.contains(Modifier::BOLD));
        }

        let long = format!("~/{}last", "deep/".repeat(20));
        let buf = draw_with(&Splash::default(), &theme, &long, 100, 26)?;
        let shown = row(&buf, PATH_Y);
        assert!(shown.starts_with('…'), "{shown}");
        assert!(shown.ends_with("/deep/last"), "{shown}");
        assert_eq!(shown.chars().count(), 56);
        let x = col(&buf, PATH_Y, "last").expect("the last folder is shown");
        assert_eq!(buf[(x, PATH_Y)].fg, theme.strong);
        assert_eq!(buf[(x - 2, PATH_Y)].fg, theme.muted);
        Ok(())
    }

    #[test]
    fn the_actions_have_hints_and_keys_and_the_selected_one_glows() -> anyhow::Result<()> {
        let theme = Theme::default();
        let buf = draw(&Splash::default(), &theme, 100, 26)?;
        let line = |label: &str, hint: &str, key: &str| {
            format!("{:<18}{:<32}{key}", label, hint).trim().to_string()
        };
        assert_eq!(
            row(&buf, ITEM_YS[0]),
            format!("✦ {}", line("New file", "in ~/src", "n"))
        );
        assert_eq!(
            row(&buf, ITEM_YS[1]),
            line("New directory", "in ~/src", "d")
        );
        assert_eq!(
            row(&buf, ITEM_YS[2]),
            line("Open directory", "choose a folder", "o")
        );
        let (y, other) = (ITEM_YS[0], ITEM_YS[1]);
        // The selected row: dialog glow, `✦` in accent2, bold strong label,
        // bold accent key.
        let stops = [
            mix(theme.raised, theme.accent, 0.3),
            mix(theme.raised, theme.accent2, 0.12),
            theme.raised,
        ];
        assert_eq!(buf[(ROW_X, y)].bg, stops[0]);
        assert_eq!(buf[(ROW_X + MARK_X, y)].symbol(), "✦");
        assert_eq!(buf[(ROW_X + MARK_X, y)].fg, theme.accent2);
        let label = &buf[(ROW_X + LABEL_X, y)];
        assert_eq!(label.fg, theme.strong);
        assert!(label.modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(ROW_X + HINT_X, y)].fg, theme.muted);
        let key = &buf[(ROW_X + ROW_W - 2, y)];
        assert_eq!(key.symbol(), "n");
        assert_eq!(key.fg, theme.accent);
        assert!(key.modifier.contains(Modifier::BOLD));
        // The others: on the ground, label in text, hint and key in muted.
        assert_eq!(buf[(ROW_X, other)].bg, theme.bg);
        assert_eq!(buf[(ROW_X + LABEL_X, other)].fg, theme.text);
        assert!(
            !buf[(ROW_X + LABEL_X, other)]
                .modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(buf[(ROW_X + HINT_X, other)].fg, theme.muted);
        assert_eq!(buf[(ROW_X + ROW_W - 2, other)].fg, theme.muted);
        Ok(())
    }

    #[test]
    fn small_areas_drop_the_wordmark_then_the_hints_then_the_path() -> anyhow::Result<()> {
        let theme = Theme::default();
        let splash = Splash::default();

        // Too short for the wordmark: `✦ glyph`, the path, rows with hints.
        let shown = rows(&draw(&splash, &theme, 100, 13)?);
        assert_eq!(shown[0], "✦ glyph");
        assert_eq!(shown[1], "~/src");
        assert!(shown[2].contains("in ~/src"), "{shown:?}");
        assert_eq!(shown.len(), 5, "{shown:?}");

        // Too narrow for the hints too.
        let shown = rows(&draw(&splash, &theme, 50, 13)?);
        assert_eq!(shown[0], "✦ glyph");
        assert_eq!(shown[1], "~/src");
        assert_eq!(shown[2], format!("✦ {:<44}n", "New file"));
        assert_eq!(shown.len(), 5, "{shown:?}");

        // Too short for the path.
        let shown = rows(&draw(&splash, &theme, 100, 8)?);
        assert_eq!(shown.len(), 4, "{shown:?}");
        assert_eq!(shown[0], "✦ glyph");
        assert!(shown[1].starts_with("✦ New file"), "{shown:?}");
        assert!(!shown.iter().any(|r| r.contains("~/src")), "{shown:?}");

        // Shorter still: the rows close up.
        let shown = rows(&draw(&splash, &theme, 100, 4)?);
        assert_eq!(shown.len(), 4, "{shown:?}");
        assert!(shown[3].starts_with("Open directory"), "{shown:?}");
        Ok(())
    }

    #[test]
    fn a_click_finds_the_row_under_it_at_every_size() -> anyhow::Result<()> {
        let theme = Theme::default();
        for (w, h) in [(100, 26), (100, 13), (50, 13), (100, 8), (100, 4), (30, 3)] {
            let area = Rect::new(0, 0, w, h);
            let buf = draw(&Splash::default(), &theme, w, h)?;
            for item in Item::ALL {
                let y = (0..h).find(|&y| col(&buf, y, item.label()).is_some());
                let Some(y) = y else {
                    // Cut off at the bottom: nothing there to click.
                    continue;
                };
                let x = col(&buf, y, item.label()).expect("found above");
                assert_eq!(
                    Splash::item_at(area, Position::new(x, y)),
                    Some(item),
                    "{item:?} at {w}×{h}"
                );
            }
            // The brand row isn't an action.
            let brand =
                (0..h).find(|&y| row(&buf, y).contains("glyph") || row(&buf, y).contains('▀'));
            if let Some(y) = brand {
                assert_eq!(Splash::item_at(area, Position::new(w / 2, y)), None);
            }
        }
        // Beside a row, off its span, nothing.
        let area = Rect::new(0, 0, 100, 26);
        assert_eq!(
            Splash::item_at(area, Position::new(ROW_X + 4, ITEM_YS[1])),
            Some(Item::NewDirectory)
        );
        assert_eq!(
            Splash::item_at(area, Position::new(ROW_X - 1, ITEM_YS[1])),
            None
        );
        assert_eq!(
            Splash::item_at(area, Position::new(ROW_X + 4, ITEM_YS[1] + 1)),
            None
        );
        Ok(())
    }

    #[test]
    fn mono_has_no_glow_a_plain_wordmark_and_rule_and_a_reversed_row() -> anyhow::Result<()> {
        let mono = Theme::named("mono").expect("mono exists");
        let buf = draw(&Splash::default(), &mono, 100, 26)?;
        for (y, bitmap) in (TOP..).zip(WORDMARK) {
            for (col, c) in (0u16..).zip(bitmap.chars()) {
                let cell = &buf[(WORD_X + col, y)];
                assert_eq!(cell.symbol(), c.to_string());
                assert_eq!(cell.fg, Color::Reset);
            }
        }
        for x in 18..81 {
            assert_eq!(buf[(x, RULE_Y)].fg, Color::Reset);
        }
        assert!(
            buf[(ROW_X + 10, ITEM_YS[0])]
                .modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            !buf[(ROW_X + 10, ITEM_YS[1])]
                .modifier
                .contains(Modifier::REVERSED)
        );
        Ok(())
    }
}
