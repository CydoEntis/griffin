mod harness;

use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{COLS, Glyph, ROWS, pill_text};
use tempfile::TempDir;
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const PROMPT: &str = "has unsaved changes";
/// The tree's width: the design's `tw`.
const TREE: u16 = 28;
/// The first pill's left cap, two columns right of the tree (README §2.2).
const PILL_X: u16 = TREE + 2;
/// The pills sit on row 1, under a blank row 0; the thread is on row 2.
const PILL_ROW: u16 = 1;
const THREAD_ROW: u16 = 2;
/// The tree rows of `a.txt` and `b.txt`, below its brand row.
const A_ROW: u16 = 3;
const B_ROW: u16 = 4;

// The `aurora` roles the header uses (README §4.1).
const BG: Color = Color::Rgb(0x0b, 0x0a, 0x10);
const RAISED2: Color = Color::Rgb(0x1e, 0x1b, 0x2a);
const MUTED: Color = Color::Rgb(0x67, 0x62, 0x7d);
const STRONG: Color = Color::Rgb(0xf4, 0xf2, 0xfb);
const ACCENT: Color = Color::Rgb(0xb6, 0x9c, 0xff);
const ACCENT2: Color = Color::Rgb(0x6e, 0xe7, 0xd8);
const WARN: Color = Color::Rgb(0xf0, 0xcf, 0x7a);

/// `a` moved `t` of the way to `b`, per channel, rounded, as the editor mixes.
fn mix(a: Color, b: Color, t: f64) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (a, b) else {
        panic!("can't mix {a:?} and {b:?}");
    };
    let m = |p: u8, q: u8| (f64::from(p) + (f64::from(q) - f64::from(p)) * t).round() as u8;
    Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
}

/// The thread's colour in column `x` with the active pill's middle at `pc`, by
/// README §2.3: the hue runs `accent` to `accent2` from the tree's edge to the
/// screen's, and fades into `bg` with distance from `pc`.
fn thread(x: u16, pc: f64) -> Color {
    let hue = mix(
        ACCENT,
        ACCENT2,
        f64::from(x - TREE) / f64::from(COLS - TREE),
    );
    let d = (f64::from(x) - pc).abs();
    mix(hue, BG, (d / 46.0).clamp(0.0, 1.0) * 0.92 + 0.04)
}

/// A temp folder holding `a.txt` ("alpha") and `b.txt` ("bravo"), opened as the
/// project with the tree showing, cwd inside it, under the config `toml`.
fn open_project_with(toml: &str) -> (TempDir, Glyph) {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join("a.txt"), "alpha\n").expect("write a.txt");
    fs::write(dir.path().join("b.txt"), "bravo\n").expect("write b.txt");
    let glyph = Glyph::spawn_in_with_config(dir.path(), toml, &["."]);
    glyph.wait_for_text("b.txt", START);
    (dir, glyph)
}

fn open_project() -> (TempDir, Glyph) {
    open_project_with("")
}

/// Opens both files from the tree, leaving b.txt active.
fn open_both(glyph: &mut Glyph) {
    glyph.click(3, A_ROW);
    glyph.wait_for_text("1  alpha", WAIT);
    glyph.click(3, B_ROW);
    glyph.wait_for_text("1  bravo", WAIT);
    wait_for_tabs(glyph, &["a.txt", "b.txt"], 1);
}

/// Waits for the pill row to show `names` with `active` capped (see
/// `pill_text`).
fn wait_for_tabs(glyph: &Glyph, names: &[&str], active: usize) {
    let text = pill_text(names, active);
    glyph.wait_for_screen(&format!("pills {text:?}"), WAIT, |screen| {
        let row: String = screen[usize::from(PILL_ROW)]
            .chars()
            .skip(usize::from(PILL_X))
            .collect();
        row.trim_end() == text
    });
}

/// Waits for the active tab's name, the only bold text on the pill row beside
/// the tree's `glyph` brand, to be `name`.
fn wait_for_active(glyph: &Glyph, name: &str) {
    glyph.wait_for_bold(PILL_ROW, &format!("glyph{name}"), WAIT);
}

fn status_line(glyph: &Glyph) -> String {
    glyph.screen()[usize::from(ROWS - 1)].clone()
}

fn read(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).expect("read file")
}

#[test]
fn pills_show_each_buffer_with_the_active_one_and_dirty_marks() {
    let (_dir, mut glyph) = open_project();
    // An untitled tab before anything is opened.
    wait_for_tabs(&glyph, &["untitled"], 0);
    // Opening a file replaces that untouched untitled tab; a second gets its own.
    open_both(&mut glyph);
    wait_for_active(&glyph, "b.txt");

    // The visual check: both names, the active one capped, the dirty one marked.
    glyph.type_text("x");
    wait_for_tabs(&glyph, &["a.txt", "b.txt •"], 1);
    wait_for_active(&glyph, "b.txt");
    // From x = 30, each pill steps on by its label plus three: ` a.txt ` is 7.
    assert_eq!(glyph.text_col(PILL_ROW, "a.txt"), Some(PILL_X + 2));
    assert_eq!(glyph.text_col(PILL_ROW, "▐ b.txt"), Some(PILL_X + 10));
    // Row 0 is blank right of the tree, and row 2 is the thread.
    let screen = glyph.screen();
    let blank: String = screen[0].chars().skip(usize::from(TREE)).collect();
    assert_eq!(blank.trim(), "", "{screen:#?}");
    let rule: String = screen[usize::from(THREAD_ROW)]
        .chars()
        .skip(usize::from(TREE) + 1)
        .collect();
    assert_eq!(
        rule,
        "─".repeat(usize::from(COLS - TREE - 1)),
        "{screen:#?}"
    );

    // Opening an already-open file switches to its tab instead of opening another.
    glyph.click(3, A_ROW);
    wait_for_active(&glyph, "a.txt");
    glyph.wait_for_text("1  alpha", WAIT);
    wait_for_tabs(&glyph, &["a.txt", "b.txt •"], 0);
    glyph.click(3, B_ROW);
    glyph.wait_for_text("1  xbravo", WAIT);
    wait_for_tabs(&glyph, &["a.txt", "b.txt •"], 1);
}

#[test]
fn aurora_pills_and_a_thread_that_follows_the_active_tab() {
    let (_dir, mut glyph) = open_project_with("theme = \"aurora\"\n");
    open_both(&mut glyph);
    glyph.type_text("x");
    wait_for_tabs(&glyph, &["a.txt", "b.txt •"], 1);

    // Inactive: the label in `muted`, no fill.
    let a = PILL_X;
    assert_eq!(glyph.fg_at(a + 2, PILL_ROW), MUTED);
    assert_eq!(glyph.bg_at(a + 2, PILL_ROW), BG);
    // Active: caps in `raised2` on the ground, the label on `raised2`, the name
    // bold from `strong` to `accent`, the dirty dot in `warn`.
    let b = PILL_X + 10;
    assert_eq!(glyph.fg_at(b, PILL_ROW), RAISED2);
    assert_eq!(glyph.bg_at(b, PILL_ROW), BG);
    assert_eq!(glyph.bg_at(b + 1, PILL_ROW), RAISED2);
    assert_eq!(glyph.fg_at(b + 2, PILL_ROW), STRONG);
    assert_eq!(glyph.fg_at(b + 6, PILL_ROW), ACCENT);
    assert_eq!(glyph.fg_at(b + 8, PILL_ROW), WARN);
    assert_eq!(glyph.bg_at(b + 8, PILL_ROW), RAISED2);
    // ` b.txt • ` is 9 wide, so the right cap is 10 cells on.
    assert_eq!(glyph.fg_at(b + 10, PILL_ROW), RAISED2);
    assert_eq!(glyph.text_col(PILL_ROW, "▌"), Some(b + 10));

    // The thread is brightest under the pill's middle and faint far from it.
    let pc = f64::from(b) + 1.0 + 9.0 / 2.0;
    for x in [b + 5, b + 6, TREE + 1, COLS - 1] {
        assert_eq!(glyph.fg_at(x, THREAD_ROW), thread(x, pc), "column {x}");
    }

    // Switching tab moves it, straight away.
    glyph.send_keys("alt+,");
    wait_for_tabs(&glyph, &["a.txt", "b.txt •"], 0);
    let pc = f64::from(a) + 1.0 + 7.0 / 2.0;
    for x in [a + 4, a + 5, b + 5, COLS - 1] {
        glyph.wait_for_fg_at(x, THREAD_ROW, thread(x, pc), WAIT);
    }
}

#[test]
fn mono_reverses_the_active_pill_and_draws_the_thread_in_accent() {
    let (_dir, mut glyph) = open_project_with("theme = \"mono\"\n");
    open_both(&mut glyph);
    glyph.wait_for_reversed(PILL_ROW, " b.txt ", WAIT);
    // The whole reversed label is bold, beside the tree's brand.
    glyph.wait_for_bold(PILL_ROW, "glyph b.txt ", WAIT);
    // `accent` is the terminal's white.
    let white = glyph.fg_at(TREE + 1, THREAD_ROW);
    assert_ne!(white, Color::Default);
    for x in TREE + 1..COLS {
        assert_eq!(glyph.fg_at(x, THREAD_ROW), white, "column {x}");
    }
    glyph.send_keys("alt+,");
    glyph.wait_for_reversed(PILL_ROW, " a.txt ", WAIT);

    // A dirty active pill keeps its dot inside the fill, in the pill's own colour,
    // not a `warn` block cut into it.
    glyph.type_text("y");
    glyph.wait_for_reversed(PILL_ROW, " a.txt • ", WAIT);
    let name = glyph.text_col(PILL_ROW, "a.txt").expect("a.txt on screen");
    let dot = glyph.text_col(PILL_ROW, "•").expect("dirty dot on screen");
    assert_eq!(glyph.fg_at(dot, PILL_ROW), glyph.fg_at(name, PILL_ROW));
    assert_eq!(glyph.bg_at(dot, PILL_ROW), glyph.bg_at(name, PILL_ROW));
}

#[test]
fn alt_keys_move_between_tabs_and_ctrl_w_closes_with_the_guard() {
    let (dir, mut glyph) = open_project();
    open_both(&mut glyph);
    glyph.send_keys("ctrl+n");
    wait_for_tabs(&glyph, &["a.txt", "b.txt", "untitled"], 2);
    wait_for_active(&glyph, "untitled");

    glyph.send_keys("alt+,");
    wait_for_active(&glyph, "b.txt");
    glyph.send_keys("alt+.");
    wait_for_active(&glyph, "untitled");
    // Past the end wraps round to the first.
    glyph.send_keys("alt+.");
    wait_for_active(&glyph, "a.txt");
    glyph.wait_for_text("1  alpha", WAIT);
    glyph.send_keys("alt+3");
    wait_for_active(&glyph, "untitled");
    glyph.send_keys("alt+2");
    wait_for_active(&glyph, "b.txt");

    // A dirty tab asks first; cancel keeps it.
    glyph.type_text("y");
    wait_for_tabs(&glyph, &["a.txt", "b.txt •", "untitled"], 1);
    glyph.send_keys("ctrl+w");
    glyph.wait_for_text(PROMPT, WAIT);
    glyph.type_text("c");
    glyph.wait_for_text_gone(PROMPT, WAIT);
    wait_for_tabs(&glyph, &["a.txt", "b.txt •", "untitled"], 1);
    // Discard closes it without touching the file; the next tab takes over.
    glyph.send_keys("ctrl+w");
    glyph.wait_for_text(PROMPT, WAIT);
    glyph.type_text("d");
    wait_for_tabs(&glyph, &["a.txt", "untitled"], 1);
    wait_for_active(&glyph, "untitled");
    assert_eq!(read(dir.path(), "b.txt"), "bravo\n");

    // Clean tabs close straight away.
    glyph.send_keys("ctrl+w");
    wait_for_tabs(&glyph, &["a.txt"], 0);
    glyph.send_keys("ctrl+w");
    wait_for_tabs(&glyph, &["untitled"], 0);
    glyph.send_keys("ctrl+q");
    assert!(glyph.wait_exit(WAIT).success());
}

#[test]
fn ctrl_n_then_alt_s_saves_relative_to_the_project_root() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let root = parent.path().join("proj");
    fs::create_dir(&root).expect("create proj");
    fs::write(root.join("a.txt"), "alpha\n").expect("write a.txt");
    // cwd is the parent, so a path relative to the root is not one relative to cwd.
    let mut glyph = Glyph::spawn_in(parent.path(), &["proj"]);
    glyph.wait_for_text("a.txt", START);

    glyph.send_keys("ctrl+n");
    wait_for_active(&glyph, "untitled");
    glyph.type_text("hello");
    glyph.wait_for_text("1  hello", WAIT);
    wait_for_tabs(&glyph, &["untitled", "untitled •"], 1);

    // A name another file already has changes nothing.
    glyph.send_keys("alt+s");
    glyph.wait_for_text("Save as:", WAIT);
    glyph.type_text("a.txt");
    glyph.send_keys("enter");
    glyph.wait_for_text("a.txt already exists", WAIT);
    assert_eq!(read(&root, "a.txt"), "alpha\n");

    glyph.send_keys("alt+s");
    glyph.wait_for_text("Save as:", WAIT);
    glyph.type_text("new.txt");
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("Save as:", WAIT);
    wait_for_tabs(&glyph, &["untitled", "new.txt"], 1);
    assert_eq!(read(&root, "new.txt"), "hello");
    assert!(!parent.path().join("new.txt").exists());
    assert!(!status_line(&glyph).contains('•'));
    // The tree shows the new file.
    glyph.wait_for_text("  new.txt", WAIT);

    // Save as again offers the current path, relative to the root, and moves the
    // tab to the new name; the old file stays as it was.
    glyph.type_text("!");
    glyph.send_keys("alt+s");
    glyph.wait_for_text("Save as: new.txt", WAIT);
    for _ in 0.."new.txt".len() {
        glyph.send_keys("backspace");
    }
    glyph.type_text("renamed.txt");
    glyph.send_keys("enter");
    wait_for_tabs(&glyph, &["untitled", "renamed.txt"], 1);
    assert_eq!(read(&root, "renamed.txt"), "hello!");
    assert_eq!(read(&root, "new.txt"), "hello");
    glyph.send_keys("ctrl+q");
    assert!(glyph.wait_exit(WAIT).success());
}

#[test]
fn click_selects_a_pill_and_middle_click_closes_it() {
    let (_dir, mut glyph) = open_project();
    open_both(&mut glyph);
    // The blank row above the pills and the thread below them are not tabs.
    let a = glyph.text_col(PILL_ROW, "a.txt").expect("a.txt pill");
    glyph.click(a, 0);
    glyph.click(a, THREAD_ROW);
    glyph.assert_running_for(Duration::from_millis(200));
    wait_for_active(&glyph, "b.txt");
    // Any cell of a pill selects it, from cap to cap.
    glyph.click(PILL_X, PILL_ROW);
    wait_for_active(&glyph, "a.txt");
    glyph.wait_for_text("1  alpha", WAIT);
    glyph.click(PILL_X + 10, PILL_ROW);
    wait_for_active(&glyph, "b.txt");
    glyph.click(a, PILL_ROW);
    wait_for_active(&glyph, "a.txt");
    // Clicking a tab gives the editor focus: typing edits a.txt.
    glyph.type_text("z");
    glyph.wait_for_text("1  zalpha", WAIT);
    wait_for_tabs(&glyph, &["a.txt •", "b.txt"], 0);

    // Middle click on a clean tab closes it.
    let b = glyph.text_col(PILL_ROW, "b.txt").expect("b.txt pill");
    glyph.middle_click(b, PILL_ROW);
    wait_for_tabs(&glyph, &["a.txt •"], 0);

    // On a dirty one it asks first.
    let a = glyph.text_col(PILL_ROW, "a.txt").expect("a.txt pill");
    glyph.middle_click(a, PILL_ROW);
    glyph.wait_for_text(PROMPT, WAIT);
    glyph.type_text("c");
    glyph.wait_for_text_gone(PROMPT, WAIT);
    wait_for_tabs(&glyph, &["a.txt •"], 0);
    glyph.middle_click(a, PILL_ROW);
    glyph.wait_for_text(PROMPT, WAIT);
    glyph.type_text("d");
    wait_for_tabs(&glyph, &["untitled"], 0);
}

#[test]
fn ctrl_q_asks_about_each_dirty_tab_in_turn() {
    let (dir, mut glyph) = open_project();
    open_both(&mut glyph);
    glyph.type_text("2");
    glyph.click(3, A_ROW);
    glyph.wait_for_text("1  alpha", WAIT);
    glyph.type_text("1");
    // A clean untitled tab in between is never asked about.
    glyph.send_keys("ctrl+n");
    wait_for_tabs(&glyph, &["a.txt •", "b.txt •", "untitled"], 2);

    glyph.send_keys("ctrl+q");
    glyph.wait_for_text("a.txt has unsaved changes", WAIT);
    // The question is about a.txt, shown as the active tab.
    wait_for_tabs(&glyph, &["a.txt •", "b.txt •", "untitled"], 0);
    glyph.wait_for_text("1  1alpha", WAIT);
    glyph.type_text("s");
    // Then b.txt.
    wait_for_tabs(&glyph, &["a.txt", "b.txt •", "untitled"], 1);
    glyph.wait_for_text("b.txt has unsaved changes", WAIT);
    glyph.wait_for_text("1  2bravo", WAIT);
    assert_eq!(read(dir.path(), "a.txt"), "1alpha\n");

    // Cancel stops the quit with b.txt still unsaved.
    glyph.type_text("c");
    glyph.wait_for_text_gone(PROMPT, WAIT);
    glyph.assert_running_for(Duration::from_millis(300));
    wait_for_tabs(&glyph, &["a.txt", "b.txt •", "untitled"], 1);

    // Asked again, discard: nothing is left to ask about, so glyph quits.
    glyph.send_keys("ctrl+q");
    glyph.wait_for_text("b.txt has unsaved changes", WAIT);
    wait_for_active(&glyph, "b.txt");
    glyph.type_text("d");
    assert!(glyph.wait_exit(WAIT).success());
    assert_eq!(read(dir.path(), "b.txt"), "bravo\n");
}
