//! A split's tab header (design README §2.2–2.3): a blank row, then one pill per
//! open buffer, then the aurora thread, a `─` rule that glows brightest under the
//! active pill and fades away from it.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::{Theme, grad, mix};

/// Rows a split's header takes above its editor: blank, pills, thread.
pub const HEADER_HEIGHT: u16 = 3;

/// How far either side of the active pill the thread fades out, in cells.
const THREAD_REACH: f64 = 46.0;

/// What one tab shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabLabel {
    pub name: String,
    pub dirty: bool,
}

impl TabLabel {
    /// The text between the pill's caps.
    fn text(&self) -> String {
        if self.dirty {
            format!(" {} • ", self.name)
        } else {
            format!(" {} ", self.name)
        }
    }

    /// A pill's cells: the label plus a cap either side.
    fn width(&self) -> usize {
        self.text().width() + 2
    }
}

/// The row of a header that holds the pills.
fn pill_row(header: Rect) -> Rect {
    Rect {
        y: header.y + 1,
        height: 1.min(header.height.saturating_sub(1)),
        ..header
    }
}

/// The row of a header that holds the thread.
fn thread_row(header: Rect) -> Rect {
    Rect {
        y: header.y + 2,
        height: 1.min(header.height.saturating_sub(2)),
        ..header
    }
}

/// Where each pill lands in a row `width` cells wide, as (column offset of its
/// left cap, width); `None` for pills that don't fit. Pills start one cell in and
/// have one blank cell between them (README §2.2: step `len(label) + 3`). When they
/// don't all fit, pills drop off the left until the active one does, so the tab
/// being edited is always on screen.
pub fn layout(labels: &[TabLabel], active: usize, width: u16) -> Vec<Option<(u16, u16)>> {
    let widths: Vec<usize> = labels.iter().map(TabLabel::width).collect();
    let width = usize::from(width);
    // Where the pills from `first` through the active one end: one cell in, each
    // pill plus its gap, less the gap after the last.
    let end = |first: usize| widths[first..=active].iter().map(|w| w + 1).sum::<usize>();
    let mut first = 0;
    while first < active && end(first) > width {
        first += 1;
    }
    let mut x = 1;
    widths
        .iter()
        .enumerate()
        .map(|(index, &w)| {
            if index < first || x >= width {
                return None;
            }
            let at = x;
            x += w + 1;
            // Both fit in u16: `at` is below `width`, which came from a u16, and the
            // width shown is cut to what's left of it.
            let shown = w.min(width - at);
            Some((
                u16::try_from(at).unwrap_or(u16::MAX),
                u16::try_from(shown).unwrap_or(u16::MAX),
            ))
        })
        .collect()
}

/// The tab under screen cell (`col`, `row`) of a header drawn in `header`.
pub fn tab_at(
    labels: &[TabLabel],
    active: usize,
    header: Rect,
    col: u16,
    row: u16,
) -> Option<usize> {
    let pills = pill_row(header);
    if row != pills.y || pills.height == 0 {
        return None;
    }
    let offset = col.checked_sub(pills.x)?;
    layout(labels, active, pills.width)
        .iter()
        .position(|place| place.is_some_and(|(x, w)| (x..x + w).contains(&offset)))
}

/// Draws the header into `header`: row 0 left as the editor ground, the pills on
/// row 1 and the thread on row 2. In the split without focus the active pill is
/// dimmed to `muted` on `raised` and the thread drawn fainter, so the bright pill
/// always names where keys go (README §5.6).
pub fn render_tabs(
    theme: &Theme,
    labels: &[TabLabel],
    active: usize,
    focused: bool,
    header: Rect,
    frame: &mut Frame,
) {
    let out = frame.buffer_mut();
    out.set_style(header, Style::new().bg(theme.bg).fg(theme.muted));
    let pills = pill_row(header);
    if pills.height == 0 {
        return;
    }
    let mut centre = None;
    for (index, place) in layout(labels, active, pills.width).into_iter().enumerate() {
        let Some((x, w)) = place else {
            continue;
        };
        let pill = Rect::new(pills.x + x, pills.y, w, 1);
        let label = &labels[index];
        if index == active {
            draw_active(out, theme, label, focused, pill);
            // The pill's middle, between the caps (README §2.2 `pc`).
            centre = Some(f64::from(pill.x) + 1.0 + label.text().width() as f64 / 2.0);
        } else {
            put(out, pill, 1, &label.text(), Style::new().fg(theme.muted));
            if label.dirty {
                put_dot(out, pill, label, Style::new().fg(theme.warn));
            }
        }
    }
    draw_thread(out, theme, focused, centre, thread_row(header));
}

/// The active pill: caps `▐ ▌` in the pill's colour on the ground, the label
/// filled between them, the name bold and lit from `strong` to `accent` letter by
/// letter. `mono` has no ramps, so it fills with reverse video instead.
fn draw_active(out: &mut Buffer, theme: &Theme, label: &TabLabel, focused: bool, pill: Rect) {
    let inner = label.text().width();
    let flat = theme.flat();
    let fill = match (flat, focused) {
        (true, true) => {
            Theme::highlight(theme.tab_active_bg, theme.tab_active_fg).add_modifier(Modifier::BOLD)
        }
        // Only one pill is filled on screen, the focused split's.
        (true, false) => Style::new(),
        (false, true) => Style::new().bg(theme.raised2).fg(theme.strong),
        (false, false) => Style::new().bg(theme.raised).fg(theme.muted),
    };
    let caps = match (flat, focused) {
        // Reverse video shows the cap colour as the fill, so the caps round it off.
        (true, true) => Some(theme.tab_active_bg),
        (true, false) => None,
        (false, true) => Some(theme.raised2),
        (false, false) => Some(theme.raised),
    };
    if let Some(colour) = caps {
        let cap = Style::new().fg(colour).bg(theme.bg);
        put(out, pill, 0, "▐", cap);
        put(out, pill, 1 + inner, "▌", cap);
    }
    put(out, pill, 1, &" ".repeat(inner), fill);
    let n = label.name.chars().count();
    let mut x = 2;
    for (i, c) in label.name.chars().enumerate() {
        let t = if n > 1 {
            i as f64 / (n - 1) as f64
        } else {
            0.0
        };
        let name = match (flat, focused) {
            (true, true) => fill,
            // Not bold: in mono, bold marks only the focused split's name.
            (true, false) => Style::new()
                .fg(theme.strong)
                .add_modifier(Modifier::UNDERLINED),
            (false, true) => Style::new()
                .fg(grad(&[theme.strong, theme.accent], t))
                .add_modifier(Modifier::BOLD),
            (false, false) => Style::new().fg(theme.muted),
        };
        put(out, pill, x, &c.to_string(), name);
        x += c.width().unwrap_or(0);
    }
    if label.dirty {
        // Setting only a colour keeps the cell's REVERSED, so a `warn` dot on the
        // mono fill would show as a coloured block; there it takes the fill's
        // style (SPEC_V1_LAYOUT §2).
        let dot = if flat && focused {
            fill
        } else {
            Style::new().fg(theme.warn)
        };
        put_dot(out, pill, label, dot);
    }
}

/// The dirty `•`, one space after the name.
fn put_dot(out: &mut Buffer, pill: Rect, label: &TabLabel, style: Style) {
    put(out, pill, 3 + label.name.width(), "•", style);
}

/// The aurora thread (README §2.3): `─` across the row, its hue running from
/// `accent` to `accent2`, faded towards the ground with distance from `centre`.
/// The split without focus fades it further (factor .97 for .92). `mono` can't
/// blend, so its thread is flat `accent`.
fn draw_thread(out: &mut Buffer, theme: &Theme, focused: bool, centre: Option<f64>, row: Rect) {
    if row.height == 0 {
        return;
    }
    let reach = if focused { 0.92 } else { 0.97 };
    // The design measures the ramp from the tree's edge `tw`, one column left of
    // the header (the blank column after the tree), out to the screen's edge.
    let span = f64::from(row.width) + 1.0;
    let centre = centre.unwrap_or(0.0);
    for x in row.left()..row.right() {
        let fg = if theme.flat() {
            theme.accent
        } else {
            let hue = grad(
                &[theme.accent, theme.accent2],
                (f64::from(x - row.x) + 1.0) / span,
            );
            let d = (f64::from(x) - centre).abs();
            mix(
                hue,
                theme.bg,
                (d / THREAD_REACH).clamp(0.0, 1.0) * reach + 0.04,
            )
        };
        out.set_string(x, row.y, "─", Style::new().fg(fg).bg(theme.bg));
    }
}

/// Writes `text` at column `x` of the one-row `area`, clipped to it. Styles
/// without a background keep the cell's, so a pill's fill stays under its text.
fn put(out: &mut Buffer, area: Rect, x: usize, text: &str, style: Style) {
    let Ok(x) = u16::try_from(x) else {
        return;
    };
    if x >= area.width {
        return;
    }
    out.set_stringn(area.x + x, area.y, text, usize::from(area.width - x), style);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    fn label(name: &str, dirty: bool) -> TabLabel {
        TabLabel {
            name: name.into(),
            dirty,
        }
    }

    fn rgb(c: Color) -> (u8, u8, u8) {
        match c {
            Color::Rgb(r, g, b) => (r, g, b),
            other => panic!("not RGB: {other:?}"),
        }
    }

    #[test]
    fn pills_step_by_their_label_plus_three() {
        let labels = [label("a.txt", false), label("b.txt", true)];
        // " a.txt " is 7 wide, 9 with caps; the next starts 10 on.
        assert_eq!(layout(&labels, 0, 40), vec![Some((1, 9)), Some((11, 11))]);
        let header = Rect::new(29, 0, 40, 3);
        assert_eq!(tab_at(&labels, 0, header, 30, 1), Some(0));
        assert_eq!(tab_at(&labels, 0, header, 38, 1), Some(0));
        assert_eq!(tab_at(&labels, 0, header, 39, 1), None);
        assert_eq!(tab_at(&labels, 0, header, 40, 1), Some(1));
        assert_eq!(tab_at(&labels, 0, header, 50, 1), Some(1));
        assert_eq!(tab_at(&labels, 0, header, 51, 1), None);
        assert_eq!(tab_at(&labels, 0, header, 5, 1), None);
        // Only the pill row is clickable.
        assert_eq!(tab_at(&labels, 0, header, 32, 0), None);
        assert_eq!(tab_at(&labels, 0, header, 32, 2), None);
    }

    #[test]
    fn pills_drop_off_the_left_so_the_active_one_shows() {
        let labels: Vec<TabLabel> = (0..5).map(|n| label(&format!("f{n}"), false)).collect();
        // Each pill is 6 cells and a gap; 16 fit two and a bit.
        assert_eq!(
            layout(&labels, 0, 16),
            vec![Some((1, 6)), Some((8, 6)), Some((15, 1)), None, None]
        );
        assert_eq!(
            layout(&labels, 4, 16),
            vec![None, None, None, Some((1, 6)), Some((8, 6))]
        );
    }

    fn draw(theme: &Theme, labels: &[TabLabel], active: usize, focused: bool) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(40, 3)).expect("test terminal");
        terminal
            .draw(|frame| {
                render_tabs(
                    theme,
                    labels,
                    active,
                    focused,
                    Rect::new(0, 0, 40, 3),
                    frame,
                )
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..40).map(|x| buffer[(x, y)].symbol()).collect()
    }

    #[test]
    fn the_active_pill_is_capped_filled_and_lit() {
        let theme = Theme::named("aurora").expect("aurora exists");
        let labels = [label("a.txt", false), label("b.txt", true)];
        let buffer = draw(&theme, &labels, 1, true);
        assert_eq!(row(&buffer, 0).trim(), "");
        assert_eq!(row(&buffer, 1).trim_end(), "   a.txt   ▐ b.txt • ▌");
        assert_eq!(buffer[(2, 1)].fg, theme.muted);
        assert_eq!(buffer[(11, 1)].fg, theme.raised2);
        assert_eq!(buffer[(11, 1)].bg, theme.bg);
        assert_eq!(buffer[(12, 1)].bg, theme.raised2);
        assert_eq!(buffer[(13, 1)].fg, theme.strong);
        assert_eq!(buffer[(17, 1)].fg, theme.accent);
        assert!(buffer[(13, 1)].modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(19, 1)].fg, theme.warn);
        assert_eq!(buffer[(19, 1)].bg, theme.raised2);
        assert_eq!(buffer[(21, 1)].symbol(), "▌");
        // Brightest under the pill's middle (11 + 1 + 9/2 = 16.5), faint far off.
        let bright = rgb(buffer[(16, 2)].fg);
        let far = rgb(buffer[(39, 2)].fg);
        assert!(bright.0 > far.0 && bright.1 > far.1, "{bright:?} {far:?}");
        assert!(row(&buffer, 2).chars().all(|c| c == '─'));
    }

    #[test]
    fn the_unfocused_split_dims_its_pill_and_thread() {
        let theme = Theme::named("aurora").expect("aurora exists");
        let labels = [label("a.txt", false)];
        let focused = draw(&theme, &labels, 0, true);
        let unfocused = draw(&theme, &labels, 0, false);
        assert_eq!(unfocused[(3, 1)].fg, theme.muted);
        assert_eq!(unfocused[(3, 1)].bg, theme.raised);
        assert_eq!(unfocused[(1, 1)].fg, theme.raised);
        // Under the pill the two agree (both .04); away from it the unfocused one
        // has faded further.
        assert!(rgb(focused[(25, 2)].fg).0 > rgb(unfocused[(25, 2)].fg).0);
    }

    #[test]
    fn mono_reverses_the_active_pill_and_draws_the_thread_flat() {
        let theme = Theme::named("mono").expect("mono exists");
        let labels = [label("a.txt", false), label("b.txt", false)];
        let buffer = draw(&theme, &labels, 1, true);
        let reversed: String = (0..40)
            .filter(|&x| buffer[(x, 1)].modifier.contains(Modifier::REVERSED))
            .map(|x| buffer[(x, 1)].symbol())
            .collect();
        assert_eq!(reversed, " b.txt ");
        assert!(buffer[(13, 1)].modifier.contains(Modifier::BOLD));
        assert!((0..40).all(|x| buffer[(x, 2)].fg == theme.accent));
    }

    #[test]
    fn mono_draws_a_dirty_active_pills_dot_in_the_fill() {
        let theme = Theme::named("mono").expect("mono exists");
        let labels = [label("a.txt", false), label("b.txt", true)];
        let buffer = draw(&theme, &labels, 1, true);
        assert_eq!(buffer[(19, 1)].symbol(), "•");
        assert_eq!(buffer[(19, 1)].style(), buffer[(13, 1)].style());
        assert!(buffer[(19, 1)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn mono_underlines_the_unfocused_active_name_without_bold() {
        let theme = Theme::named("mono").expect("mono exists");
        let labels = [label("a.txt", false)];
        let buffer = draw(&theme, &labels, 0, false);
        let name = &buffer[(3, 1)];
        assert_eq!(name.symbol(), "a");
        assert_eq!(name.fg, theme.strong);
        assert!(name.modifier.contains(Modifier::UNDERLINED));
        assert!(!name.modifier.contains(Modifier::BOLD));
    }
}
