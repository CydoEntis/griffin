pub mod catalog;
pub mod completion;
pub mod confirm;
pub mod debug;
pub mod dirpicker;
pub mod find;
pub mod hover;
pub mod keybindings;
pub mod nofile;
pub mod picker;
pub mod prompt;
pub mod run;
pub mod search;
pub mod splash;
pub mod status;
pub mod tabs;
pub mod tree;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Clear};

use crate::theme::{Theme, grad, mix};
use crate::ui::prompt::PromptBar;

/// The frame the popups anchored at the cursor (hover, completion) are drawn in:
/// a rounded `line2` border around `raised` (design README §5.3). They don't dim
/// the screen, so unlike dialogs (`dialog_card`) they need the border to stand
/// apart from the text around them.
pub fn card_block(theme: &Theme) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.line2))
        .style(Style::new().bg(theme.raised).fg(theme.text))
}

/// How far each cell moves towards `scrim` behind a dialog (design README §3).
const DIM: f64 = 0.6;

/// Dims everything already drawn in `area`, so a dialog drawn after it stands
/// out: each fg, bg and underline colour mixed `DIM` towards `scrim`, or the DIM modifier where the
/// theme's colours can't be blended (`mono`).
pub fn dim(theme: &Theme, buf: &mut Buffer, area: Rect) {
    let area = area.intersection(buf.area);
    let ramps = theme.ramps();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            if ramps {
                cell.fg = mix(cell.fg, theme.scrim, DIM);
                cell.bg = mix(cell.bg, theme.scrim, DIM);
                // A diagnostic's colour lives in its curly underline, so leaving
                // it alone would draw full-strength squiggles through the dim.
                // `Reset` stays: it means "no underline colour", not a colour.
                if cell.underline_color != Color::Reset {
                    cell.underline_color = mix(cell.underline_color, theme.scrim, DIM);
                }
            } else {
                cell.modifier.insert(Modifier::DIM);
            }
        }
    }
}

/// Draws an empty dialog card over `card`: `text` on `raised`, no border, and a
/// lit top edge of `▀` running accent → accent2 → accent (design README §3).
/// Content goes from the card's second row.
pub fn dialog_card(theme: &Theme, frame: &mut Frame, card: Rect) {
    let card = card.intersection(frame.area());
    if card.is_empty() {
        return;
    }
    frame.render_widget(Clear, card);
    let buf = frame.buffer_mut();
    buf.set_style(card, Style::new().bg(theme.raised).fg(theme.text));
    let stops = [theme.accent, theme.accent2, theme.accent];
    let span = f64::from(card.width.saturating_sub(1).max(1));
    for x in card.left()..card.right() {
        let fg = grad(&stops, f64::from(x - card.x) / span);
        buf[(x, card.y)]
            .set_symbol("▀")
            .set_style(Style::new().fg(fg).bg(theme.raised));
    }
}

/// Lights a selected list row from `x0` up to `x1`: a ramp from accent through
/// accent2 into `raised` (design README §3), or reverse video in `mono`, which
/// has no ramps. Draw it before the row's text, and in `mono` give that text no
/// colours of its own so the reverse video reads.
pub fn glow_row(theme: &Theme, buf: &mut Buffer, x0: u16, x1: u16, y: u16) {
    if theme.ramps() {
        let stops = [
            mix(theme.raised, theme.accent, 0.3),
            mix(theme.raised, theme.accent2, 0.12),
            theme.raised,
        ];
        let span = f64::from(x1.saturating_sub(x0).max(1));
        for x in x0..x1 {
            buf[(x, y)].set_bg(grad(&stops, f64::from(x - x0) / span));
        }
    } else {
        buf.set_style(
            Rect::new(x0, y, x1.saturating_sub(x0), 1),
            Theme::highlight(theme.hov, theme.strong),
        );
    }
}

/// Marks the selected row as the one Enter acts on: `⏎` ending at `right`, in
/// the accent (plain in `mono`, where the row is already reversed).
pub fn enter_mark(theme: &Theme, buf: &mut Buffer, right: u16, y: u16) {
    let style = if theme.ramps() {
        Style::new().fg(theme.accent)
    } else {
        Style::new()
    };
    buf.set_string(right.saturating_sub(1), y, "⏎", style);
}

/// An input row of a dialog across `area`, on `raised2`: `✦` in accent2 two
/// cells in, the label (`text` while the row takes the keys, else `muted`)
/// padded to `label_width`, two blanks, then the value in `strong`. Returns
/// where the cursor goes.
pub fn input_row(
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
    label: &str,
    label_width: u16,
    field: &PromptBar,
    focused: bool,
) -> (u16, u16) {
    let buf = frame.buffer_mut();
    buf.set_style(area, Style::new().bg(theme.raised2).fg(theme.strong));
    let prompt = area.x + 2;
    buf.set_string(prompt, area.y, "✦", Style::new().fg(theme.accent2));
    let label_x = prompt + 2;
    let label_style = Style::new().fg(if focused { theme.text } else { theme.muted });
    buf.set_stringn(
        label_x,
        area.y,
        label,
        usize::from(area.right().saturating_sub(label_x)),
        label_style,
    );
    let x = (label_x + label_width + 2).min(area.right());
    let value = Rect {
        x,
        width: area.right() - x,
        ..area
    };
    field.render_value(buf, value, Style::new().fg(theme.strong))
}

/// The hints on a dialog's last row, in `muted`, two cells in.
pub fn footer(theme: &Theme, buf: &mut Buffer, card: Rect, hints: &str) {
    let y = card.bottom().saturating_sub(1);
    buf.set_stringn(
        card.x + 2,
        y,
        hints,
        usize::from(card.width.saturating_sub(4)),
        Style::new().fg(theme.muted),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dim_mixes_towards_the_scrim_or_sets_dim_in_mono() {
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        let area = buf.area;
        buf.set_style(area, Style::new().fg(theme.fg).bg(theme.bg));
        dim(&theme, &mut buf, area);
        assert_eq!(buf[(0, 0)].fg, mix(theme.fg, theme.scrim, 0.6));
        assert_eq!(buf[(1, 0)].bg, mix(theme.bg, theme.scrim, 0.6));

        let mono = Theme::named("mono").expect("mono exists");
        let mut buf = Buffer::empty(area);
        buf.set_style(area, Style::new().fg(Color::Gray));
        dim(&mono, &mut buf, area);
        assert!(buf[(1, 0)].modifier.contains(Modifier::DIM));
        assert_eq!(buf[(1, 0)].fg, Color::Gray);
    }

    #[test]
    fn dim_mixes_a_diagnostic_underline_towards_the_scrim_too() {
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        let area = buf.area;
        buf.set_style(area, Style::new().fg(theme.fg).bg(theme.bg));
        buf[(0, 0)].set_style(
            Style::new()
                .underline_color(theme.err)
                .add_modifier(Modifier::UNDERLINED),
        );
        dim(&theme, &mut buf, area);
        assert_eq!(
            buf[(0, 0)].underline_color,
            mix(theme.err, theme.scrim, 0.6)
        );
        assert_ne!(buf[(0, 0)].underline_color, theme.err);
        // A cell without an underline colour keeps none.
        assert_eq!(buf[(1, 0)].underline_color, Color::Reset);
    }

    #[test]
    fn the_glow_row_starts_lit_and_fades_into_raised() {
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        glow_row(&theme, &mut buf, 0, 10, 0);
        let stops = [
            mix(theme.raised, theme.accent, 0.3),
            mix(theme.raised, theme.accent2, 0.12),
            theme.raised,
        ];
        assert_eq!(buf[(0, 0)].bg, stops[0]);
        assert_eq!(buf[(5, 0)].bg, stops[1]);
        assert_eq!(buf[(9, 0)].bg, grad(&stops, 0.9));

        let mono = Theme::named("mono").expect("mono exists");
        glow_row(&mono, &mut buf, 0, 10, 0);
        assert!(buf[(3, 0)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn the_lit_edge_runs_accent_to_accent2_and_back() -> anyhow::Result<()> {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(21, 3))?;
        terminal.draw(|frame| dialog_card(&theme, frame, Rect::new(0, 0, 21, 3)))?;
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "▀");
        assert_eq!(buffer[(0, 0)].fg, theme.accent);
        assert_eq!(buffer[(10, 0)].fg, theme.accent2);
        assert_eq!(buffer[(20, 0)].fg, theme.accent);
        assert_eq!(buffer[(5, 1)].bg, theme.raised);
        Ok(())
    }
}
