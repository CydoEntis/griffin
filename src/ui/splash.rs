//! The splash Glyph opens on when it's started with nothing to edit
//! (glyph-splash spec S2): a card centred in the editor area with the `✦ glyph`
//! wordmark, the project folder, and three actions to pick from.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;

use crate::theme::{Theme, grad};
use crate::ui::tree::{cut, cut_front};
use crate::ui::{dialog_card, glow_row};

/// The card's width when the editor area has room for it.
const CARD_WIDTH: u16 = 52;

/// Where the card's text starts, from its left edge, as in the cast palette.
const INSET: u16 = 4;

/// The hints on the card's last text row.
pub const FOOTER: &str = "ctrl+p go to file · ctrl+q quit";

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
}

/// What one row of the card holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// The lit `▀` top edge every Aurora dialog card has.
    Edge,
    Blank,
    Brand,
    Path,
    Item(Item),
    Footer,
}

use Line::{Blank, Brand, Edge, Footer, Path};

/// The card's rows when everything fits, then with less and less room: the
/// footer goes first, then the path (spec Design, "Small editor areas"). The
/// last is the least the splash can be; with fewer rows than that it's cut off
/// at the bottom.
const LAYOUTS: &[&[Line]] = &[
    &[
        Edge,
        Blank,
        Brand,
        Path,
        Blank,
        Line::Item(Item::NewFile),
        Line::Item(Item::NewDirectory),
        Line::Item(Item::OpenDirectory),
        Blank,
        Footer,
        Blank,
    ],
    &[
        Edge,
        Blank,
        Brand,
        Path,
        Blank,
        Line::Item(Item::NewFile),
        Line::Item(Item::NewDirectory),
        Line::Item(Item::OpenDirectory),
        Blank,
    ],
    &[
        Edge,
        Blank,
        Brand,
        Blank,
        Line::Item(Item::NewFile),
        Line::Item(Item::NewDirectory),
        Line::Item(Item::OpenDirectory),
        Blank,
    ],
    &[
        Brand,
        Line::Item(Item::NewFile),
        Line::Item(Item::NewDirectory),
        Line::Item(Item::OpenDirectory),
    ],
];

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

    /// The card's place in `area` and the rows it shows there.
    fn layout(area: Rect) -> (Rect, &'static [Line]) {
        let lines = LAYOUTS
            .iter()
            .copied()
            .find(|lines| lines.len() <= usize::from(area.height))
            .unwrap_or(LAYOUTS[LAYOUTS.len() - 1]);
        let width = if area.width >= CARD_WIDTH + 4 {
            CARD_WIDTH
        } else {
            area.width.saturating_sub(4).max(area.width.min(24))
        };
        let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let card = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        }
        .intersection(area);
        (card, lines)
    }

    /// The action whose row is at `at`, for a click.
    pub fn item_at(area: Rect, at: Position) -> Option<Item> {
        let (card, lines) = Self::layout(area);
        if !card.contains(at) {
            return None;
        }
        match lines.get(usize::from(at.y - card.y)) {
            Some(Line::Item(item)) => Some(*item),
            _ => None,
        }
    }

    /// Draws the splash centred in `area`, naming `project` (an absolute path)
    /// under the wordmark.
    pub fn render(&self, theme: &Theme, project: &str, frame: &mut Frame, area: Rect) {
        let (card, lines) = Self::layout(area);
        if card.is_empty() {
            return;
        }
        if lines.first() == Some(&Edge) {
            dialog_card(theme, frame, card);
        } else {
            frame.render_widget(Clear, card);
            frame
                .buffer_mut()
                .set_style(card, Style::new().bg(theme.raised).fg(theme.text));
        }
        let buf = frame.buffer_mut();
        let x = card.x + INSET;
        let room = usize::from(card.width.saturating_sub(2 * INSET));
        for (y, line) in (card.y..card.bottom()).zip(lines.iter().copied()) {
            match line {
                Edge | Blank => {}
                Brand => brand(theme, buf, card, x, y),
                Path => {
                    let path = cut_front(project, room);
                    buf.set_string(x, y, path, Style::new().fg(theme.muted));
                }
                Line::Item(item) => self.item_row(theme, buf, card, item, y),
                Footer => {
                    let hints = cut(FOOTER, room);
                    buf.set_string(x, y, hints, Style::new().fg(theme.muted));
                }
            }
        }
    }

    /// One action row: on the dialog glow with `✦` in accent2 while selected,
    /// else `▸` in `muted`; the label, then its letter right-aligned in `muted`
    /// where the cast palette puts its `⏎`.
    fn item_row(&self, theme: &Theme, buf: &mut Buffer, card: Rect, item: Item, y: u16) {
        let selected = item == self.selected();
        if selected {
            glow_row(theme, buf, card.x, card.right(), y);
        }
        // `mono` reverses the selected row, which only reads with no colours of
        // its own on it.
        let plain = selected && !theme.ramps();
        let pick = |style: Style| if plain { Style::new() } else { style };
        let (marker, marker_style) = if selected {
            ("✦", Style::new().fg(theme.accent2))
        } else {
            ("▸", Style::new().fg(theme.muted))
        };
        let x = card.x + INSET;
        let key_x = card.right().saturating_sub(INSET + 1);
        if x >= key_x {
            return;
        }
        buf.set_string(x, y, marker, pick(marker_style));
        let label_x = x + 2;
        let room = usize::from(key_x.saturating_sub(label_x + 1));
        let label_style = Style::new().fg(if selected { theme.strong } else { theme.text });
        buf.set_string(label_x, y, cut(item.label(), room), pick(label_style));
        buf.set_string(key_x, y, item.key(), pick(Style::new().fg(theme.muted)));
    }
}

/// `✦ glyph`: the tree brand row's wordmark, its letters ramping accent →
/// accent2 in bold.
fn brand(theme: &Theme, buf: &mut Buffer, card: Rect, x: u16, y: u16) {
    if x + 7 > card.right() {
        return;
    }
    buf.set_string(x, y, "✦", Style::new().fg(theme.accent2));
    for (i, c) in (0u16..).zip(["g", "l", "y", "p", "h"]) {
        let fg = grad(&[theme.accent, theme.accent2], f64::from(i) / 4.0);
        let style = Style::new().fg(fg).add_modifier(Modifier::BOLD);
        buf.set_string(x + 2 + i, y, c, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn draw(splash: &Splash, theme: &Theme, w: u16, h: u16) -> anyhow::Result<Buffer> {
        let mut terminal = Terminal::new(TestBackend::new(w, h))?;
        terminal
            .draw(|frame| splash.render(theme, "/home/me/project", frame, Rect::new(0, 0, w, h)))?;
        Ok(terminal.backend().buffer().clone())
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
            .filter(|text| !text.is_empty() && !text.starts_with('▀'))
            .collect()
    }

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
    fn the_card_is_centred_with_every_row_when_there_is_room() -> anyhow::Result<()> {
        let buf = draw(&Splash::default(), &Theme::default(), 80, 21)?;
        assert_eq!(
            rows(&buf),
            [
                "✦ glyph",
                "/home/me/project",
                // The letters sit in column 61, five in from the card's right.
                &format!("{:<43}n", "✦ New file"),
                &format!("{:<43}d", "▸ New directory"),
                &format!("{:<43}o", "▸ Open directory"),
                FOOTER,
            ]
        );
        // 52 wide from x 14; 11 tall from y 5.
        assert_eq!(buf[(14, 5)].symbol(), "▀");
        assert_eq!(buf[(65, 5)].symbol(), "▀");
        assert_eq!(buf[(18, 7)].symbol(), "✦");
        assert_eq!(buf[(61, 10)].symbol(), "n");
        Ok(())
    }

    #[test]
    fn small_areas_drop_the_footer_then_the_path() -> anyhow::Result<()> {
        let theme = Theme::default();
        let buf = draw(&Splash::default(), &theme, 80, 10)?;
        let shown = rows(&buf);
        assert!(shown.contains(&"/home/me/project".to_string()), "{shown:?}");
        assert!(!shown.contains(&FOOTER.to_string()), "{shown:?}");

        let buf = draw(&Splash::default(), &theme, 80, 8)?;
        let shown = rows(&buf);
        assert_eq!(shown.len(), 4, "{shown:?}");
        assert_eq!(shown[0], "✦ glyph");
        Ok(())
    }

    #[test]
    fn a_click_finds_the_row_under_it() {
        let area = Rect::new(0, 0, 80, 21);
        assert_eq!(
            Splash::item_at(area, Position::new(30, 11)),
            Some(Item::NewDirectory)
        );
        assert_eq!(Splash::item_at(area, Position::new(30, 7)), None);
        assert_eq!(Splash::item_at(area, Position::new(2, 11)), None);
    }

    #[test]
    fn mono_reverses_the_selected_row_and_leaves_it_uncoloured() -> anyhow::Result<()> {
        let mono = Theme::named("mono").expect("mono exists");
        let buf = draw(&Splash::default(), &mono, 80, 21)?;
        assert!(buf[(20, 10)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(20, 11)].modifier.contains(Modifier::REVERSED));
        Ok(())
    }
}
