//! The key list the editor area shows whenever no file is open and the splash
//! isn't up (tome-splash spec S11, as in `design/no-file-open.png`): the
//! project folder and `no file open`, then a few keys to get going.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::keymap::{Action, Keymap};
use crate::theme::Theme;

/// The rows under the header, top to bottom: what runs and what it's called.
pub const ROWS: [(Action, &str); 4] = [
    (Action::GoToFile, "cast · files and commands"),
    (Action::NewFile, "new file"),
    (Action::ToggleTree, "toggle tree"),
    (Action::Quit, "quit"),
];

/// Where a row's label starts, from its key (SPEC_V1_LAYOUT §4: "label … at
/// +10"); further right when a rebound key is too long to leave a gap.
const LABEL_X: u16 = 10;

/// The gap between a key and its label when the key pushes the label right.
const KEY_GAP: u16 = 2;

/// The gap between the folder and `no file open`.
const HEADER_GAP: u16 = 2;

const NO_FILE: &str = "no file open";

/// One row of the list: the key, named as the keymap names it, and its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    pub key: String,
    pub label: &'static str,
}

/// The rows as `keymap` binds them, so a rebound key shows its new name. An
/// action left with no key has nothing to show, so its row goes.
pub fn rows(keymap: &Keymap) -> Vec<KeyRow> {
    ROWS.iter()
        .filter_map(|&(action, label)| {
            Some(KeyRow {
                key: keymap.key_label(action)?,
                label,
            })
        })
        .collect()
}

fn width(text: &str) -> u16 {
    u16::try_from(text.width()).unwrap_or(u16::MAX)
}

/// Where the list goes in `area`, as (left, top, label column from left). The
/// block is centred across, so it sits where the design has it at any width,
/// with its lines left-aligned inside; its header starts a third of the way
/// down, keeping it in the upper part of the pane as the design does.
fn place(area: Rect, folder: &str, rows: &[KeyRow]) -> (u16, u16, u16) {
    let label_x = rows
        .iter()
        .map(|row| width(&row.key).saturating_add(KEY_GAP))
        .fold(LABEL_X, u16::max);
    let header = width(folder)
        .saturating_add(1)
        .saturating_add(HEADER_GAP)
        .saturating_add(width(NO_FILE));
    let block_w = rows
        .iter()
        .map(|row| label_x.saturating_add(width(row.label)))
        .fold(header, u16::max);
    let block_h = u16::try_from(rows.len() * 2 + 1).unwrap_or(u16::MAX);
    let x = area.x + area.width.saturating_sub(block_w) / 2;
    let y = area.y + (area.height / 3).min(area.height.saturating_sub(block_h));
    (x, y, label_x)
}

/// Draws the list over `area`, which it has to itself. `folder` is the project
/// folder's name. `mono` gets plain text: its colours are the terminal's own,
/// and bold `strong` beside plain `muted` would be all there is to tell apart.
pub fn render(theme: &Theme, folder: &str, rows: &[KeyRow], frame: &mut Frame, area: Rect) {
    let area = area.intersection(frame.area());
    if area.is_empty() {
        return;
    }
    let buf = frame.buffer_mut();
    buf.set_style(area, Style::new().bg(theme.bg).fg(theme.fg));
    let plain = !theme.ramps();
    let style = |fg, bold: bool| {
        if plain {
            Style::new()
        } else if bold {
            Style::new().fg(fg).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(fg)
        }
    };
    let (x, y, label_x) = place(area, folder, rows);
    // Text never runs past the pane into the tree or the other split.
    let room = |from: u16| usize::from(area.right().saturating_sub(from));
    let folder = format!("{folder}/");
    buf.set_stringn(x, y, &folder, room(x), style(theme.strong, true));
    let note_x = x.saturating_add(width(&folder)).saturating_add(HEADER_GAP);
    buf.set_stringn(note_x, y, NO_FILE, room(note_x), style(theme.muted, false));
    for (i, row) in (0u16..).zip(rows) {
        let row_y = y.saturating_add(2 + 2 * i);
        if row_y >= area.bottom() {
            break;
        }
        buf.set_stringn(x, row_y, &row.key, room(x), style(theme.accent, true));
        let at = x.saturating_add(label_x);
        buf.set_stringn(at, row_y, row.label, room(at), style(theme.text, false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyBinding, KeysConfig};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    fn draw(theme: &Theme, keymap: &Keymap, w: u16, h: u16) -> anyhow::Result<Buffer> {
        let mut terminal = Terminal::new(TestBackend::new(w, h))?;
        let rows = rows(keymap);
        terminal.draw(|frame| render(theme, "tome", &rows, frame, Rect::new(0, 0, w, h)))?;
        Ok(terminal.backend().buffer().clone())
    }

    fn line(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    /// The column `text` starts at on row `y`.
    fn col(buf: &Buffer, y: u16, text: &str) -> Option<u16> {
        let line = line(buf, y);
        let at = line.find(text)?;
        u16::try_from(line[..at].chars().count()).ok()
    }

    // In 100 × 30 the 35-wide block (10 + `cast · files and commands`) starts
    // at (100 − 35) / 2 = 32, and its header on row 30 / 3 = 10.
    const X: u16 = 32;
    const TOP: u16 = 10;

    #[test]
    fn the_list_is_left_aligned_rows_two_apart_with_labels_at_plus_ten() -> anyhow::Result<()> {
        let theme = Theme::default();
        let buf = draw(&theme, &Keymap::default(), 100, 30)?;
        assert_eq!(col(&buf, TOP, "tome/  no file open"), Some(X));
        for (i, (key, label)) in [
            ("Ctrl+P", "cast · files and commands"),
            ("Ctrl+N", "new file"),
            ("Ctrl+B", "toggle tree"),
            ("Ctrl+Q", "quit"),
        ]
        .into_iter()
        .enumerate()
        {
            let y = TOP + 2 + 2 * u16::try_from(i)?;
            assert_eq!(col(&buf, y, key), Some(X), "{:?}", line(&buf, y));
            assert_eq!(col(&buf, y, label), Some(X + 10), "{:?}", line(&buf, y));
            // The rows between stay empty.
            assert_eq!(line(&buf, y - 1).trim(), "");
        }
        Ok(())
    }

    #[test]
    fn the_folder_is_bold_strong_the_note_muted_keys_bold_accent_labels_text() -> anyhow::Result<()>
    {
        let theme = Theme::default();
        let buf = draw(&theme, &Keymap::default(), 100, 30)?;
        let folder = &buf[(X, TOP)];
        assert_eq!(folder.fg, theme.strong);
        assert!(folder.modifier.contains(Modifier::BOLD));
        let slash = &buf[(X + 4, TOP)];
        assert_eq!((slash.symbol(), slash.fg), ("/", theme.strong));
        let note = &buf[(X + 7, TOP)];
        assert_eq!(note.fg, theme.muted);
        assert!(!note.modifier.contains(Modifier::BOLD));
        for y in [TOP + 2, TOP + 4, TOP + 6, TOP + 8] {
            let key = &buf[(X, y)];
            assert_eq!(key.fg, theme.accent);
            assert!(key.modifier.contains(Modifier::BOLD));
            let label = &buf[(X + 10, y)];
            assert_eq!(label.fg, theme.text);
            assert!(!label.modifier.contains(Modifier::BOLD));
        }
        Ok(())
    }

    #[test]
    fn a_rebound_key_shows_its_new_name() -> anyhow::Result<()> {
        let mut keys = KeysConfig::new();
        keys.insert("go_to_file".into(), KeyBinding::One("alt+o".into()));
        keys.insert("new_file".into(), KeyBinding::One("f2".into()));
        let keymap = Keymap::new(&keys)?;
        let shown = rows(&keymap);
        assert_eq!(shown[0].key, "Alt+O");
        assert_eq!(shown[1].key, "F2");
        assert_eq!(shown[2].key, "Ctrl+B");
        let buf = draw(&Theme::default(), &keymap, 100, 30)?;
        assert_eq!(col(&buf, TOP + 2, "Alt+O"), Some(X));
        assert_eq!(col(&buf, TOP + 2, "cast"), Some(X + 10));
        assert_eq!(col(&buf, TOP + 4, "F2"), Some(X));
        assert_eq!(col(&buf, TOP + 4, "new file"), Some(X + 10));

        // A key too long for the label column pushes every label right, two
        // clear of it: the block is then 12 + 25 wide, from (100 − 37) / 2.
        let mut keys = KeysConfig::new();
        keys.insert("go_to_file".into(), KeyBinding::One("ctrl+alt+o".into()));
        let buf = draw(&Theme::default(), &Keymap::new(&keys)?, 100, 30)?;
        assert_eq!(col(&buf, TOP + 2, "Ctrl+Alt+O  cast"), Some(31));
        assert_eq!(col(&buf, TOP + 4, "new file"), Some(31 + 12));
        Ok(())
    }

    #[test]
    fn mono_is_plain_text() -> anyhow::Result<()> {
        let mono = Theme::named("mono").expect("mono exists");
        let buf = draw(&mono, &Keymap::default(), 100, 30)?;
        assert_eq!(col(&buf, TOP, "tome/  no file open"), Some(X));
        for y in [TOP, TOP + 2, TOP + 4, TOP + 6, TOP + 8] {
            for x in 0..100 {
                let cell = &buf[(x, y)];
                assert_eq!(cell.fg, Color::Reset, "({x}, {y})");
                assert!(cell.modifier.is_empty(), "({x}, {y})");
            }
        }
        Ok(())
    }

    #[test]
    fn a_short_pane_moves_the_list_up_and_a_narrow_one_cuts_it() -> anyhow::Result<()> {
        let theme = Theme::default();
        // 9 rows high: a third down would push the last row out, so it starts
        // at the top.
        let buf = draw(&theme, &Keymap::default(), 100, 9)?;
        assert_eq!(col(&buf, 0, "tome/"), Some(X));
        assert_eq!(col(&buf, 8, "Ctrl+Q"), Some(X));
        let buf = draw(&theme, &Keymap::default(), 20, 30)?;
        assert_eq!(line(&buf, TOP + 2), "Ctrl+P    cast · fil");
        Ok(())
    }
}
