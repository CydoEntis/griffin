mod harness;

use std::time::Duration;

use harness::{COLS, ROWS, Tome};
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

/// The status bar's glyph block covers columns 0..18; the bar is past it.
const BLOCK_END: u16 = 18;

/// Every cell of the status line past the glyph block has background `color`.
fn assert_status_bg(tome: &Tome, color: Color) {
    tome.wait_for_bg(BLOCK_END, STATUS_ROW, color, WAIT);
    for col in BLOCK_END..COLS {
        assert_eq!(tome.bg_at(col, STATUS_ROW), color, "status column {col}");
    }
}

#[test]
fn nord_paints_the_status_line_with_its_sidebar_colour() {
    let mut tome = Tome::spawn_with_config("theme = \"nord\"\n", &[]);
    tome.wait_for_text("Open directory", START);
    assert_status_bg(&tome, NORD_SIDEBAR);
    // Leave the splash, whose card covers the middle of the editor area.
    tome.send_keys("ctrl+n");
    // The editor ground below the first line is nord's background.
    tome.wait_for_bg(50, 10, NORD_BG, WAIT);
}

#[test]
fn an_override_changes_the_colour() {
    let mut tome = Tome::spawn_with_config(
        "theme = \"nord\"\n[theme_overrides]\nsidebar_bg = \"#123456\"\n",
        &[],
    );
    tome.wait_for_text("Open directory", START);
    assert_status_bg(&tome, Color::Rgb(0x12, 0x34, 0x56));
    tome.send_keys("ctrl+n");
    tome.wait_for_bg(50, 10, NORD_BG, WAIT);
}

#[test]
fn an_unknown_theme_falls_back_to_hydra_and_says_so() {
    let tome = Tome::spawn_with_config("theme = \"solarized\"\n", &[]);
    tome.wait_for_text("theme: unknown theme \"solarized\", using hydra", START);
    assert_status_bg(&tome, HYDRA_SIDEBAR);
}

#[test]
fn a_bad_override_falls_back_to_hydra_and_says_so() {
    let tome = Tome::spawn_with_config(
        "theme = \"nord\"\n[theme_overrides]\nkeyword = \"blurple\"\n",
        &[],
    );
    tome.wait_for_text(
        "theme: bad colour \"blurple\" for keyword, using hydra",
        START,
    );
    assert_status_bg(&tome, HYDRA_SIDEBAR);
    let screen = tome.screen();
    assert_eq!(
        screen[usize::from(STATUS_ROW)].matches("theme:").count(),
        1,
        "one message: {screen:#?}"
    );
}

#[test]
fn mono_draws_the_glyph_block_on_flat_accent() {
    let tome = Tome::spawn_with_config("theme = \"mono\"\n", &[]);
    tome.wait_for_text("Open directory", START);
    // mono has no ramps: every block cell is its accent, the terminal's white.
    let accent = tome.bg_at(0, STATUS_ROW);
    assert_ne!(accent, Color::Default);
    for col in 0..BLOCK_END {
        assert_eq!(tome.bg_at(col, STATUS_ROW), accent, "block column {col}");
    }
    // Past the block the bar is on the terminal's own background.
    assert_eq!(tome.bg_at(BLOCK_END, STATUS_ROW), Color::Default);
    assert_eq!(tome.text_col(STATUS_ROW, "✦ tome"), Some(1));
}
