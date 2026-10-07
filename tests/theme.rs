mod harness;

use std::time::Duration;

use harness::{COLS, Glyph, ROWS};
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const STATUS_ROW: u16 = ROWS - 1;

/// nord's `sidebar_bg`.
const NORD_SIDEBAR: Color = Color::Rgb(0x27, 0x2c, 0x36);
/// hydra's `sidebar_bg` (`surf`).
const HYDRA_SIDEBAR: Color = Color::Rgb(0x0c, 0x13, 0x1b);
/// nord's `bg`, the editor ground.
const NORD_BG: Color = Color::Rgb(0x2e, 0x34, 0x40);

/// Every cell of the status line has background `color`.
fn assert_status_bg(glyph: &Glyph, color: Color) {
    glyph.wait_for_bg(0, STATUS_ROW, color, WAIT);
    for col in 0..COLS {
        assert_eq!(glyph.bg_at(col, STATUS_ROW), color, "status column {col}");
    }
}

#[test]
fn nord_paints_the_status_line_with_its_sidebar_colour() {
    let glyph = Glyph::spawn_with_config("theme = \"nord\"\n", &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    assert_status_bg(&glyph, NORD_SIDEBAR);
    // The editor ground below the first line is nord's background.
    glyph.wait_for_bg(50, 10, NORD_BG, WAIT);
}

#[test]
fn an_override_changes_the_colour() {
    let glyph = Glyph::spawn_with_config(
        "theme = \"nord\"\n[theme_overrides]\nsidebar_bg = \"#123456\"\n",
        &[],
    );
    glyph.wait_for_text("Ln 1, Col 1", START);
    assert_status_bg(&glyph, Color::Rgb(0x12, 0x34, 0x56));
    glyph.wait_for_bg(50, 10, NORD_BG, WAIT);
}

#[test]
fn an_unknown_theme_falls_back_to_hydra_and_says_so() {
    let glyph = Glyph::spawn_with_config("theme = \"solarized\"\n", &[]);
    glyph.wait_for_text("theme: unknown theme \"solarized\", using hydra", START);
    assert_status_bg(&glyph, HYDRA_SIDEBAR);
}

#[test]
fn a_bad_override_falls_back_to_hydra_and_says_so() {
    let glyph = Glyph::spawn_with_config(
        "theme = \"nord\"\n[theme_overrides]\nkeyword = \"blurple\"\n",
        &[],
    );
    glyph.wait_for_text(
        "theme: bad colour \"blurple\" for keyword, using hydra",
        START,
    );
    assert_status_bg(&glyph, HYDRA_SIDEBAR);
    let screen = glyph.screen();
    assert_eq!(
        screen[usize::from(STATUS_ROW)].matches("theme:").count(),
        1,
        "one message: {screen:#?}"
    );
}
