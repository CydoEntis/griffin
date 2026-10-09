//! `>keybindings`: the card listing every command with its keys and config
//! name, searchable, in the catalog card's style (glyph-catalog spec C2).

mod harness;

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{Glyph, ROWS};
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(10);

/// At 100x30 the card is the catalog's: 86 wide from column 7, dropping from
/// row 4 with its lit edge, down to a row short of the screen's bottom. The
/// header is row 6, the rule row 7 and the list rows 8 to 26, the text four
/// cells in; the config names start 35 cells after the titles and the keys end
/// four cells in from the card's right edge. A blank, then the footer on row
/// 28, two cells in.
const CARD_X: u16 = 7;
const CARD_Y: u16 = 4;
const HEADER_ROW: u16 = 6;
const RULE_ROW: u16 = 7;
const FIRST_ROW: u16 = 8;
const TEXT_X: u16 = 11;
const NAME_X: u16 = 46;
const RIGHT: u16 = 89;
const FOOTER_ROW: u16 = 28;
const HEADER: &str = "✦ keybindings";
const FOOTER: &str = "⏎ rebind  del reset  esc close";

/// The default theme, hydra: its `accent` and `raised`.
const ACCENT: Color = Color::Rgb(0xc3, 0xf5, 0x3c);
const RAISED: Color = Color::Rgb(0x0f, 0x18, 0x21);

/// `a` moved `t` of the way to `b`, per channel, rounded, as the editor mixes.
fn mix(a: Color, b: Color, t: f64) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (a, b) else {
        panic!("can't mix {a:?} and {b:?}");
    };
    let m = |p: u8, q: u8| (f64::from(p) + (f64::from(q) - f64::from(p)) * t).round() as u8;
    Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
}

fn row(glyph: &Glyph, row: u16) -> String {
    glyph.screen()[usize::from(row)].clone()
}

/// Glyph on an empty project with `config`.
fn glyph_with(config: &str) -> Glyph {
    let project = tempfile::tempdir().expect("create project");
    let glyph = Glyph::spawn_in_with_config(project.path(), config, &["."]);
    glyph.wait_for_text("Open directory", START);
    glyph
}

/// Ctrl+P, `>keybindings`, Enter.
fn open_card(glyph: &mut Glyph) {
    glyph.send_keys("ctrl+p");
    glyph.wait_for_text("cast · files · commands", WAIT);
    glyph.type_text(">keybindings");
    glyph.wait_for_text("cast · commands", WAIT);
    glyph.wait_for_screen("Keybindings listed", WAIT, |screen| {
        // The cast's first command row, under its `COMMANDS` label.
        screen[9].contains("Keybindings")
    });
    glyph.send_keys("enter");
    glyph.wait_for_text(HEADER, WAIT);
}

/// Column where `keys` start on a row: they end at `RIGHT`.
fn keys_x(keys: &str) -> u16 {
    RIGHT - u16::try_from(keys.chars().count()).expect("short keys")
}

/// Whether screen row `y` holds `text` from the list's text column.
fn starts(screen: &[String], y: u16, text: &str) -> bool {
    screen[usize::from(y)]
        .chars()
        .skip(usize::from(TEXT_X))
        .collect::<String>()
        .starts_with(text)
}

/// Types `query` and waits for row `y` to be `title`.
fn filter_at(glyph: &mut Glyph, query: &str, y: u16, title: &str) {
    glyph.type_text(query);
    glyph.wait_for_screen(&format!("{query} filtered"), WAIT, |screen| {
        starts(screen, y, title)
    });
}

/// Types `query` and waits for the first row to be `title`.
fn filter(glyph: &mut Glyph, query: &str, title: &str) {
    filter_at(glyph, query, FIRST_ROW, title);
}

/// Deletes `n` letters of the query.
fn erase(glyph: &mut Glyph, n: usize) {
    for _ in 0..n {
        glyph.send_keys("backspace");
    }
}

#[test]
fn keybindings_in_the_cast_lists_every_command_with_its_keys_and_name() {
    let mut glyph = glyph_with("");
    open_card(&mut glyph);

    // The lit edge across the card, then `✦ keybindings` and a rule.
    assert_eq!(
        row(&glyph, CARD_Y).chars().nth(usize::from(CARD_X)),
        Some('▀')
    );
    assert_eq!(glyph.text_col(HEADER_ROW, HEADER), Some(TEXT_X));
    assert_eq!(row(&glyph, RULE_ROW).chars().nth(9), Some('─'));

    for (i, (title, name, keys)) in [("Quit", "quit", "Ctrl+Q"), ("Save", "save", "Ctrl+S")]
        .into_iter()
        .enumerate()
    {
        let y = FIRST_ROW + u16::try_from(i).expect("two rows");
        assert_eq!(glyph.text_col(y, title), Some(TEXT_X), "{title}");
        assert_eq!(glyph.text_col(y, name), Some(NAME_X), "{title}");
        assert_eq!(glyph.text_col(y, keys), Some(keys_x(keys)), "{title}");
    }
    // The config name is `muted`, apart from the title and the keys.
    let title = glyph.fg_at(TEXT_X, FIRST_ROW + 1);
    let name = glyph.fg_at(NAME_X, FIRST_ROW + 1);
    let keys = glyph.fg_at(keys_x("Ctrl+S"), FIRST_ROW + 1);
    assert_ne!(name, title);
    assert_ne!(name, keys);

    // A blank, then the footer.
    assert_eq!(row(&glyph, FOOTER_ROW - 1).trim(), "");
    assert_eq!(glyph.text_col(FOOTER_ROW, FOOTER), Some(CARD_X + 2));

    // A command with no key shows a dash.
    filter(&mut glyph, "keybindings", "Keybindings");
    assert_eq!(glyph.text_col(FIRST_ROW, "keybindings"), Some(NAME_X));
    assert_eq!(glyph.text_col(FIRST_ROW, "—"), Some(keys_x("—")));
}

#[test]
fn scope_only_commands_come_under_a_muted_heading() {
    let mut glyph = glyph_with("");
    open_card(&mut glyph);
    filter(&mut glyph, "tree", "Toggle file tree");

    // The commands that work anywhere, a blank, then the tree's own.
    assert_eq!(
        glyph.text_col(FIRST_ROW + 1, "Focus file tree"),
        Some(TEXT_X)
    );
    assert_eq!(row(&glyph, FIRST_ROW + 2).trim(), "");
    assert_eq!(row(&glyph, FIRST_ROW + 3).trim(), "file tree");
    assert_eq!(glyph.text_col(FIRST_ROW + 3, "file tree"), Some(TEXT_X));
    assert!(glyph.bold_at(TEXT_X, FIRST_ROW + 3));
    assert_ne!(
        glyph.fg_at(TEXT_X, FIRST_ROW + 3),
        glyph.fg_at(TEXT_X, FIRST_ROW + 4)
    );
    assert_eq!(
        glyph.text_col(FIRST_ROW + 4, "New file in tree"),
        Some(TEXT_X)
    );
    assert_eq!(
        glyph.text_col(FIRST_ROW + 5, "Shift+A"),
        Some(keys_x("Shift+A"))
    );

    // The splash's and the debug panel's under theirs, with nothing above.
    erase(&mut glyph, 4);
    filter_at(&mut glyph, "splash: quit", FIRST_ROW + 1, "Splash: quit");
    assert_eq!(row(&glyph, FIRST_ROW).trim(), "splash");
    erase(&mut glyph, 12);
    filter_at(&mut glyph, "debug_up", FIRST_ROW + 1, "Debug panel: up");
    assert_eq!(row(&glyph, FIRST_ROW).trim(), "debug panel");
}

#[test]
fn typing_filters_on_title_key_or_config_name() {
    let mut glyph = glyph_with("");
    open_card(&mut glyph);
    // The header holds what's typed.
    filter(&mut glyph, "save as", "Save as…");
    assert!(row(&glyph, HEADER_ROW).contains("✦ keybindings  save as"));
    assert_eq!(row(&glyph, FIRST_ROW + 1).trim(), "");
    erase(&mut glyph, 7);
    filter(&mut glyph, "f12", "Go to definition");
    erase(&mut glyph, 3);
    filter(&mut glyph, "go_to_line", "Go to line…");
    assert_eq!(glyph.text_col(FIRST_ROW, "Ctrl+G"), Some(keys_x("Ctrl+G")));
    // Nothing matching says so.
    glyph.type_text("zzz");
    glyph.wait_for_text("no matching commands", WAIT);
}

#[test]
fn arrows_move_the_selection_and_esc_or_a_click_outside_close() {
    let mut glyph = glyph_with("");
    open_card(&mut glyph);
    let lit = mix(RAISED, ACCENT, 0.3);

    // The first row starts selected, on the glow.
    glyph.wait_for_bg(CARD_X, FIRST_ROW, lit, WAIT);
    assert_ne!(glyph.bg_at(CARD_X, FIRST_ROW + 1), lit);
    glyph.send_keys("down");
    glyph.wait_for_bg(CARD_X, FIRST_ROW + 1, lit, WAIT);
    assert_ne!(glyph.bg_at(CARD_X, FIRST_ROW), lit);
    glyph.send_keys("up");
    glyph.send_keys("up");
    glyph.wait_for_bg(CARD_X, FIRST_ROW, lit, WAIT);

    // Past the bottom the list scrolls, keeping the selection in view.
    for _ in 0..20 {
        glyph.send_keys("down");
    }
    glyph.wait_for_screen("the list scrolled", WAIT, |screen| {
        !screen[usize::from(FIRST_ROW)].contains("Quit")
    });
    assert_eq!(glyph.bg_at(CARD_X, FIRST_ROW + 18), lit);

    glyph.send_keys("esc");
    glyph.wait_for_text_gone(HEADER, WAIT);

    // A click outside the card closes it too; one inside doesn't.
    open_card(&mut glyph);
    glyph.click(CARD_X + 10, FIRST_ROW + 2);
    // Events are handled in order, so the card filtering what's typed after
    // the click shows the click left it open.
    filter(&mut glyph, "save as", "Save as…");
    assert_eq!(glyph.text_col(HEADER_ROW, HEADER), Some(TEXT_X));
    glyph.click(2, ROWS - 3);
    glyph.wait_for_text_gone(HEADER, WAIT);
}

#[test]
fn a_paste_goes_into_the_filter() {
    let mut glyph = glyph_with("");
    open_card(&mut glyph);
    // A bracketed paste with a line break, which the filter leaves out.
    glyph.write(b"\x1b[200~toggle tree\r\x1b[201~");
    glyph.wait_for_screen("the paste filtered", WAIT, |screen| {
        starts(screen, FIRST_ROW, "Toggle file tree")
    });
    assert!(row(&glyph, HEADER_ROW).contains("✦ keybindings  toggle tree"));
    assert_eq!(row(&glyph, FIRST_ROW + 1).trim(), "");
}

/// What the selected row shows while it waits for a new key.
const WAITING: &str = "press a key… (esc cancels)";

/// Glyph on `a.txt` (`alpha`) in `project`, with `GLYPH_CONFIG` pointing at
/// `config`.
fn glyph_on_file(project: &Path, config: &Path) -> Glyph {
    fs::write(project.join("a.txt"), "alpha\n").expect("write a.txt");
    let env = [("GLYPH_CONFIG", OsString::from(config))];
    let glyph = Glyph::spawn_in_with_env(project, &env, &["a.txt"]);
    glyph.wait_for_text("alpha", START);
    glyph
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn enter_then_a_key_rebinds_save_and_the_new_key_saves() {
    let project = tempfile::tempdir().expect("create project");
    let home = tempfile::tempdir().expect("create config home");
    let config = home.path().join("config.toml");
    fs::write(&config, "# mine\ntheme = \"hydra\"\n").expect("write config");
    let mut glyph = glyph_on_file(project.path(), &config);
    open_card(&mut glyph);
    filter(&mut glyph, "save", "Save");

    glyph.send_keys("enter");
    glyph.wait_for_text(WAITING, WAIT);
    assert_eq!(glyph.text_col(FIRST_ROW, WAITING), Some(keys_x(WAITING)));
    glyph.send_keys("alt+w");
    glyph.wait_for_text("save bound to Alt+W", WAIT);
    glyph.wait_for_screen("the row shows Alt+W", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].trim_end().ends_with("Alt+W")
    });
    assert_eq!(glyph.text_col(FIRST_ROW, "Alt+W"), Some(keys_x("Alt+W")));
    assert_eq!(
        read(&config),
        "# mine\ntheme = \"hydra\"\n\n[keys]\nsave = \"alt+w\"\n"
    );

    // Applied at once: Alt+W saves a.txt, with no restart.
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(HEADER, WAIT);
    glyph.type_text("x");
    glyph.wait_for_text("xalpha", WAIT);
    glyph.send_keys("alt+w");
    let file = project.path().join("a.txt");
    glyph.wait_for_files("a.txt saved by Alt+W", WAIT, || read(&file) == "xalpha\n");
}

#[test]
fn delete_on_a_row_brings_back_its_default_and_backspace_edits_the_query() {
    let project = tempfile::tempdir().expect("create project");
    let home = tempfile::tempdir().expect("create config home");
    let config = home.path().join("config.toml");
    fs::write(&config, "[keys]\nsave = \"alt+w\"\nclose_tab = \"alt+q\"\n").expect("write config");
    let mut glyph = glyph_on_file(project.path(), &config);
    open_card(&mut glyph);
    filter(&mut glyph, "save", "Save");
    assert_eq!(glyph.text_col(FIRST_ROW, "Alt+W"), Some(keys_x("Alt+W")));

    glyph.send_keys("delete");
    glyph.wait_for_text("save reset to its default keys", WAIT);
    glyph.wait_for_screen("the row shows Ctrl+S", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)]
            .trim_end()
            .ends_with("Ctrl+S")
    });
    assert_eq!(read(&config), "[keys]\nclose_tab = \"alt+q\"\n");

    // Backspace still corrects the query rather than resetting anything.
    glyph.send_keys("backspace");
    glyph.wait_for_screen("the query lost its e", WAIT, |screen| {
        screen[usize::from(HEADER_ROW)]
            .trim_end()
            .ends_with("✦ keybindings  sav")
    });
    assert_eq!(read(&config), "[keys]\nclose_tab = \"alt+q\"\n");

    // Ctrl+S saves again.
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(HEADER, WAIT);
    glyph.type_text("y");
    glyph.wait_for_text("yalpha", WAIT);
    glyph.send_keys("ctrl+s");
    let file = project.path().join("a.txt");
    glyph.wait_for_files("a.txt saved by Ctrl+S", WAIT, || read(&file) == "yalpha\n");
}

#[test]
fn an_open_config_tab_shows_the_rebind_and_saving_it_keeps_it() {
    let project = tempfile::tempdir().expect("create project");
    let home = tempfile::tempdir().expect("create config home");
    let config = home.path().join("config.toml");
    let mut glyph = glyph_on_file(project.path(), &config);
    // `>settings` opens config.toml, written from the template.
    glyph.send_keys("ctrl+p");
    glyph.wait_for_text("cast · files · commands", WAIT);
    glyph.type_text(">settings");
    glyph.wait_for_screen("Settings listed", WAIT, |screen| {
        screen[9].contains("Settings")
    });
    glyph.send_keys("enter");
    glyph.wait_for_text("# tab_width = 4", WAIT);

    open_card(&mut glyph);
    filter(&mut glyph, "save", "Save");
    glyph.send_keys("enter");
    glyph.wait_for_text(WAITING, WAIT);
    glyph.send_keys("alt+w");
    glyph.wait_for_text("save bound to Alt+W", WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(HEADER, WAIT);
    // The tab shows the new entry, and saving it with the new key keeps it.
    glyph.wait_for_text("save = \"alt+w\"", WAIT);
    glyph.send_keys("alt+w");
    glyph.wait_for_text("settings applied", WAIT);
    // A stale tab would have written the template back, Ctrl+S with it.
    assert!(read(&config).contains("[keys]\nsave = \"alt+w\"\n"));
}

#[test]
fn esc_while_waiting_cancels_and_writes_nothing() {
    let project = tempfile::tempdir().expect("create project");
    let home = tempfile::tempdir().expect("create config home");
    let config = home.path().join("config.toml");
    let mut glyph = glyph_on_file(project.path(), &config);
    open_card(&mut glyph);
    filter(&mut glyph, "save", "Save");

    glyph.send_keys("enter");
    glyph.wait_for_text(WAITING, WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(WAITING, WAIT);
    // The card is still open, the row as it was.
    assert_eq!(glyph.text_col(HEADER_ROW, HEADER), Some(TEXT_X));
    assert_eq!(glyph.text_col(FIRST_ROW, "Ctrl+S"), Some(keys_x("Ctrl+S")));
    assert!(!config.exists());
    // A second Esc closes it.
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(HEADER, WAIT);
    assert!(!config.exists());
}

/// What the move prompt asks when Ctrl+K, Hover's key, is pressed for Save.
const MOVE_CTRL_K: &str = "Ctrl+K is used by Show hover — move it here?";

/// Glyph on a.txt with Hover on Ctrl+K, the card filtered to Save, Enter and
/// Ctrl+K pressed, and the move prompt showing.
fn ask_to_move_ctrl_k(project: &Path, config: &Path) -> Glyph {
    fs::write(config, "[keys]\nhover = \"ctrl+k\"\n").expect("write config");
    let mut glyph = glyph_on_file(project, config);
    open_card(&mut glyph);
    filter(&mut glyph, "save", "Save");
    glyph.send_keys("enter");
    glyph.wait_for_text(WAITING, WAIT);
    glyph.send_keys("ctrl+k");
    glyph.wait_for_text(MOVE_CTRL_K, WAIT);
    glyph.wait_for_text("Show hover is left with no key.", WAIT);
    glyph
}

#[test]
fn yes_moves_a_key_another_command_has() {
    let project = tempfile::tempdir().expect("create project");
    let home = tempfile::tempdir().expect("create config home");
    let config = home.path().join("config.toml");
    let mut glyph = ask_to_move_ctrl_k(project.path(), &config);
    // Nothing is written until it's answered.
    assert_eq!(read(&config), "[keys]\nhover = \"ctrl+k\"\n");

    glyph.type_text("y");
    glyph.wait_for_text("Ctrl+K moved from hover to save", WAIT);
    glyph.wait_for_text_gone(MOVE_CTRL_K, WAIT);
    assert_eq!(read(&config), "[keys]\nhover = []\nsave = \"ctrl+k\"\n");
    // Save's row shows its new key; Hover's has none.
    glyph.wait_for_screen("Save shows Ctrl+K", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)]
            .trim_end()
            .ends_with("Ctrl+K")
    });
    erase(&mut glyph, 4);
    filter(&mut glyph, "hover", "Show hover");
    assert_eq!(glyph.text_col(FIRST_ROW, "—"), Some(keys_x("—")));

    // Ctrl+K saves now.
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(HEADER, WAIT);
    glyph.type_text("x");
    glyph.wait_for_text("xalpha", WAIT);
    glyph.send_keys("ctrl+k");
    let file = project.path().join("a.txt");
    glyph.wait_for_files("a.txt saved by Ctrl+K", WAIT, || read(&file) == "xalpha\n");
}

#[test]
fn no_or_esc_on_the_move_prompt_writes_nothing() {
    let project = tempfile::tempdir().expect("create project");
    let home = tempfile::tempdir().expect("create config home");
    let config = home.path().join("config.toml");
    let mut glyph = ask_to_move_ctrl_k(project.path(), &config);
    glyph.type_text("n");
    glyph.wait_for_text_gone(MOVE_CTRL_K, WAIT);
    // Back on the card, Save's row as it was.
    assert_eq!(glyph.text_col(HEADER_ROW, HEADER), Some(TEXT_X));
    assert_eq!(glyph.text_col(FIRST_ROW, "Ctrl+S"), Some(keys_x("Ctrl+S")));
    assert_eq!(read(&config), "[keys]\nhover = \"ctrl+k\"\n");

    glyph.send_keys("enter");
    glyph.wait_for_text(WAITING, WAIT);
    glyph.send_keys("ctrl+k");
    glyph.wait_for_text(MOVE_CTRL_K, WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(MOVE_CTRL_K, WAIT);
    assert_eq!(glyph.text_col(HEADER_ROW, HEADER), Some(TEXT_X));
    assert_eq!(read(&config), "[keys]\nhover = \"ctrl+k\"\n");
}

#[test]
fn mono_reverses_the_selected_row() {
    let mut glyph = glyph_with("theme = \"mono\"\n");
    open_card(&mut glyph);
    // The whole card's width, the keys four cells in from its right edge.
    let quit = format!("    {:<35}{:<37}Ctrl+Q    ", "Quit", "quit");
    glyph.wait_for_reversed(FIRST_ROW, &quit, WAIT);
    glyph.send_keys("down");
    let save = format!("    {:<35}{:<37}Ctrl+S    ", "Save", "save");
    glyph.wait_for_reversed(FIRST_ROW + 1, &save, WAIT);
    assert_eq!(glyph.reversed_text(FIRST_ROW), "");
}
