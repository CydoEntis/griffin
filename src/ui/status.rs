//! The status bar on the last row (design README §2.5): the glyph block, the
//! path slot at x = 20, and the cursor position and diagnostic counts on the
//! right.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::theme::{Theme, grad, mix};

/// Cells the glyph block covers: ten on the accent ramp, then eight fading into
/// the bar.
const BLOCK_WIDTH: u16 = 18;
/// Where the path (or a message in its place) starts.
const PATH_X: u16 = 20;

/// How a status message went, which picks its glyph and colour.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Ok,
    Warn,
    Err,
}

impl Tone {
    fn glyph(self) -> &'static str {
        match self {
            Tone::Ok => "✓",
            Tone::Warn => "⚠",
            Tone::Err => "✕",
        }
    }

    fn color(self, theme: &Theme) -> Color {
        match self {
            Tone::Ok => theme.ok,
            Tone::Warn => theme.warn,
            Tone::Err => theme.err,
        }
    }
}

/// What the status line shows about the active buffer.
#[derive(Debug, Default, Clone, Copy)]
pub struct Status<'a> {
    /// The buffer's path as the user knows it (relative to the project), or
    /// `untitled`.
    pub path: &'a str,
    /// Unsaved changes, shown as ` •` after the name.
    pub dirty: bool,
    /// Takes the path's place while set.
    pub message: Option<(Tone, &'a str)>,
    /// The cursor as 0-based (line, char column).
    pub position: (usize, usize),
    /// The language server's warnings and errors for the buffer.
    pub warnings: usize,
    pub errors: usize,
}

/// A piece of text and how to draw it.
type Run = (String, Style);

/// Draws the one-row status bar.
pub fn render_status(frame: &mut Frame, theme: &Theme, area: Rect, status: &Status) {
    let buf = frame.buffer_mut();
    let bar = Style::new().fg(theme.text).bg(theme.surface);
    buf.set_style(area, bar);
    let right_edge = area.x + area.width;

    let block = BLOCK_WIDTH.min(area.width);
    for i in 0..block {
        buf[(area.x + i, area.y)].set_bg(block_bg(theme, i));
    }
    put(
        buf,
        area.x + 1,
        area.y,
        right_edge,
        &[(
            "✦ glyph".to_string(),
            Style::new().fg(theme.acc_ink).add_modifier(Modifier::BOLD),
        )],
    );

    let right = right_runs(theme, status);
    let right_width: u16 = right.iter().map(|(s, _)| width(s)).sum();
    // Right-aligned to W − 2 like the design's `putRight`, which ends just before it.
    let right_x = right_edge
        .saturating_sub(2)
        .saturating_sub(right_width)
        .max(area.x);
    // The left slot stops a cell short of the right side so they never touch.
    let left_end = right_x.saturating_sub(1);
    let left_x = area.x + PATH_X;
    let left = left_runs(theme, status, left_end.saturating_sub(left_x));
    put(buf, left_x, area.y, left_end, &left);
    put(buf, right_x, area.y, right_edge, &right);
}

/// The block's bg at cell `i` (README §2.5). Themes without RGB colours (`mono`)
/// can't ramp, so the block is flat `accent` there.
fn block_bg(theme: &Theme, i: u16) -> Color {
    let rgb = |c: Color| matches!(c, Color::Rgb(..));
    if !(rgb(theme.accent) && rgb(theme.accent2) && rgb(theme.surface)) {
        return theme.accent;
    }
    let i = f64::from(i);
    if i < 10.0 {
        grad(&[theme.accent, theme.accent2], i / 9.0)
    } else {
        mix(theme.accent2, theme.surface, (i - 9.0) / 9.0)
    }
}

/// The path slot: a message with its glyph while there is one, else the path.
/// A path wider than `room` drops its directory (README §2.5), since the file
/// name and the dirty mark are what the user needs when space runs out.
fn left_runs(theme: &Theme, status: &Status, room: u16) -> Vec<Run> {
    if let Some((tone, text)) = status.message {
        let style = Style::new().fg(tone.color(theme));
        return vec![
            (format!("{} ", tone.glyph()), style),
            (text.to_string(), Style::new().fg(theme.text)),
        ];
    }
    // Either separator, so a Windows path splits the same as a Unix one.
    let split = status.path.rfind(['/', '\\']).map_or(0, |i| i + 1);
    let (dir, name) = status.path.split_at(split);
    let mut runs = vec![
        (dir.to_string(), Style::new().fg(theme.muted)),
        (name.to_string(), Style::new().fg(theme.strong)),
    ];
    if status.dirty {
        runs.push((" •".to_string(), Style::new().fg(theme.warn)));
    }
    let full: u16 = runs.iter().map(|(s, _)| width(s)).sum();
    if full > room {
        runs.remove(0);
    }
    runs
}

/// The position, then the counts while the server reports anything.
fn right_runs(theme: &Theme, status: &Status) -> Vec<Run> {
    let (line, col) = status.position;
    let mut runs = vec![(
        format!("Ln {}, Col {}", line + 1, col + 1),
        Style::new().fg(theme.text),
    )];
    if status.warnings + status.errors > 0 {
        runs.push(("    ".to_string(), Style::new()));
        runs.push((format!("✕ {}", status.errors), Style::new().fg(theme.err)));
        runs.push(("  ".to_string(), Style::new()));
        runs.push((
            format!("⚠ {}", status.warnings),
            Style::new().fg(theme.warn),
        ));
    }
    runs
}

/// Writes `runs` from `x`, cutting at `end`. Styles patch the bar's, so a run
/// without a bg keeps the cell's.
fn put(buf: &mut Buffer, mut x: u16, y: u16, end: u16, runs: &[Run]) {
    for (text, style) in runs {
        if x >= end {
            return;
        }
        let (next, _) = buf.set_stringn(x, y, text, usize::from(end - x), *style);
        x = next;
    }
}

fn width(s: &str) -> u16 {
    u16::try_from(s.width()).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_ramps_from_accent_to_accent2_then_fades_into_the_bar() {
        let theme = Theme::named("aurora").expect("aurora exists");
        assert_eq!(block_bg(&theme, 0), theme.accent);
        assert_eq!(block_bg(&theme, 9), theme.accent2);
        assert_eq!(
            block_bg(&theme, 10),
            mix(theme.accent2, theme.surface, 1.0 / 9.0)
        );
        assert_eq!(
            block_bg(&theme, 17),
            mix(theme.accent2, theme.surface, 8.0 / 9.0)
        );
    }

    fn text(runs: &[Run]) -> String {
        runs.iter().map(|(s, _)| s.as_str()).collect()
    }

    #[test]
    fn a_path_too_wide_for_the_slot_keeps_only_its_name() {
        let theme = Theme::named("aurora").expect("aurora exists");
        let status = Status {
            path: "src/workspace/walker/long_file_name.rs",
            dirty: true,
            ..Status::default()
        };
        // 38 cells of path plus 2 for the dirty mark.
        assert_eq!(
            text(&left_runs(&theme, &status, 40)),
            "src/workspace/walker/long_file_name.rs •"
        );
        let short = left_runs(&theme, &status, 39);
        assert_eq!(text(&short), "long_file_name.rs •");
        assert_eq!(short[0].1.fg, Some(theme.strong));
    }

    #[test]
    fn mono_has_a_flat_accent_block() {
        let theme = Theme::named("mono").expect("mono exists");
        for i in 0..BLOCK_WIDTH {
            assert_eq!(block_bg(&theme, i), theme.accent);
        }
    }
}
