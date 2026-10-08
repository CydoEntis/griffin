//! A crossterm backend that draws diagnostic underlines curly (SGR `4:3`), which
//! ratatui has no modifier for.

use std::io::{self, Write};

use ratatui::backend::{Backend, ClearType, CrosstermBackend, IntoCrossterm, WindowSize};
use ratatui::buffer::Cell;
use ratatui::crossterm::cursor::MoveTo;
use ratatui::crossterm::queue;
use ratatui::crossterm::style::{
    Attribute, Colors, ContentStyle, SetAttribute, SetAttributes, SetColors,
};
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Modifier};

/// [`CrosstermBackend`], except that an underlined cell with an underline colour
/// is drawn with a curly underline. Only diagnostics set an underline colour
/// (`view::render_buffer`), so that is what marks a cell as one.
pub struct UndercurlBackend<W: Write> {
    inner: CrosstermBackend<W>,
}

impl<W: Write> UndercurlBackend<W> {
    pub fn new(writer: W) -> Self {
        Self {
            inner: CrosstermBackend::new(writer),
        }
    }

    /// Draws one curly-underlined cell with all of its own style, so it doesn't
    /// depend on what the cells before it left set.
    fn draw_curly(&mut self, x: u16, y: u16, cell: &Cell) -> io::Result<()> {
        let mut style = cell.style();
        // Written below in the colon form, which crossterm doesn't produce.
        style.underline_color = None;
        let style: ContentStyle = style.into_crossterm();
        // The attributes include `UNDERLINED` (see `is_curly`), so a plain `4`
        // goes out before the `4:3`: a terminal that doesn't know curly
        // underlines keeps the straight one, as SPEC_V1_LAYOUT allows.
        queue!(
            self.inner,
            MoveTo(x, y),
            SetAttribute(Attribute::Reset),
            SetColors(Colors {
                foreground: style.foreground_color,
                background: style.background_color,
            }),
            SetAttributes(style.attributes),
        )?;
        if let Some(color) = underline_color_sgr(cell.underline_color) {
            write!(self.inner, "\x1b[{color}m")?;
        }
        write!(self.inner, "\x1b[4:3m{}\x1b[0m", cell.symbol())
    }
}

/// SGR 58 for `color`, with colons between its parts (ITU T.416). A terminal
/// that doesn't know 58 then skips it whole; with semicolons it reads the
/// colour's numbers as attributes of their own, and the Windows console drops
/// the cell's colours.
fn underline_color_sgr(color: Color) -> Option<String> {
    let index = match color {
        Color::Reset => return None,
        Color::Rgb(r, g, b) => return Some(format!("58:2::{r}:{g}:{b}")),
        Color::Indexed(i) => i,
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
    };
    Some(format!("58:5:{index}"))
}

/// Whether `cell` is a diagnostic's underline.
fn is_curly(cell: &Cell) -> bool {
    cell.modifier.contains(Modifier::UNDERLINED) && cell.underline_color != Color::Reset
}

impl<W: Write> Backend for UndercurlBackend<W> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        // Runs of ordinary cells go to crossterm's own drawing, which ends each
        // run by resetting every attribute, so a curly cell starts from a clean
        // state and leaves one.
        let mut plain: Vec<(u16, u16, &Cell)> = Vec::new();
        for (x, y, cell) in content {
            if is_curly(cell) {
                if !plain.is_empty() {
                    self.inner.draw(plain.drain(..))?;
                }
                self.draw_curly(x, y, cell)?;
            } else {
                plain.push((x, y, cell));
            }
        }
        if plain.is_empty() {
            return Ok(());
        }
        self.inner.draw(plain.into_iter())
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        Backend::flush(&mut self.inner)
    }
}

impl<W: Write> Write for UndercurlBackend<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Write::flush(&mut self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    use std::sync::{Arc, Mutex};

    /// A writer the test keeps a handle on after the backend takes it.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("output lock").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn drawn(cells: &[(u16, u16, Cell)]) -> String {
        let out = Shared::default();
        let mut backend = UndercurlBackend::new(out.clone());
        backend
            .draw(cells.iter().map(|(x, y, c)| (*x, *y, c)))
            .expect("draw into memory");
        Write::flush(&mut backend).expect("flush");
        let bytes = out.0.lock().expect("output lock").clone();
        String::from_utf8(bytes).expect("utf-8 output")
    }

    fn cell(symbol: &'static str, style: Style) -> Cell {
        let mut cell = Cell::new(symbol);
        cell.set_style(style);
        cell
    }

    #[test]
    fn a_diagnostic_cell_is_curly_in_its_colour() {
        let style = Style::new()
            .fg(Color::Rgb(1, 2, 3))
            .underline_color(Color::Rgb(255, 0, 0))
            .add_modifier(Modifier::UNDERLINED | Modifier::ITALIC);
        let out = drawn(&[(0, 0, cell("a", Style::new())), (1, 0, cell("y", style))]);
        let y = out.find('y').expect("y drawn");
        let before = &out[..y];
        let curly = before.rfind("\x1b[4:3m").expect("curly underline");
        assert!(before[..curly].contains("\x1b[4m"), "{out:?}");
        assert!(before.contains("\x1b[58:2::255:0:0m"), "{out:?}");
        assert!(before.contains("\x1b[38;2;1;2;3"), "{out:?}");
        assert!(before.contains("\x1b[3m"), "{out:?}");
        // The `a` before it is drawn plainly.
        assert!(!out[..out.find('a').expect("a drawn")].contains("4:3"));
    }

    #[test]
    fn other_underlines_stay_straight() {
        let style = Style::new().add_modifier(Modifier::UNDERLINED);
        let out = drawn(&[(0, 0, cell("k", style))]);
        assert!(out.contains("\x1b[4m"), "{out:?}");
        assert!(!out.contains("4:3"), "{out:?}");
    }
}
