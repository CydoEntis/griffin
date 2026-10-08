//! `>language servers`: the catalog card listing each server and whether it's
//! installed (glyph-catalog spec C1, C2).

mod harness;

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{Glyph, ROWS};
use tempfile::TempDir;
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(10);

/// At 100x30 the catalog is the cast's card: 86 wide from column 7, dropping
/// from row 4 with its lit edge. The header is row 6, the rule row 7 and the
/// servers rows 8 to 14, their text four cells in; the states end four cells
/// in from the card's right edge. A blank, then the footer two cells in.
const CARD_X: u16 = 7;
const CARD_Y: u16 = 4;
const HEADER_ROW: u16 = 6;
const RULE_ROW: u16 = 7;
const FIRST_ROW: u16 = 8;
const TEXT_X: u16 = 11;
const COMMAND_X: u16 = 36;
const RIGHT: u16 = 89;
const FOOTER_ROW: u16 = 16;
const HEADER: &str = "✦ language servers";
const FOOTER: &str = "⏎ install  c copy command  esc close";

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

/// A folder of empty programs named `names`, runnable as the PATH lookup
/// counts them: with `.exe` on Windows, executable elsewhere.
fn programs(names: &[&str]) -> TempDir {
    let dir = tempfile::tempdir().expect("create PATH folder");
    for name in names {
        let file = if cfg!(windows) {
            format!("{name}.exe")
        } else {
            (*name).to_string()
        };
        let path = dir.path().join(file);
        fs::write(&path, "").expect("write fake program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .expect("make fake program executable");
        }
    }
    dir
}

/// Glyph on an empty project with only `path` on its PATH.
fn with_path(project: &Path, path: &Path) -> Glyph {
    let env = [("PATH", OsString::from(path))];
    let glyph = Glyph::spawn_in_with_env(project, &env, &["."]);
    glyph.wait_for_text("Open directory", START);
    glyph
}

/// Ctrl+P, `>language servers`, Enter.
fn open_catalog(glyph: &mut Glyph) {
    glyph.send_keys("ctrl+p");
    glyph.wait_for_text("cast · files · commands", WAIT);
    glyph.type_text(">language servers");
    glyph.wait_for_text("cast · commands", WAIT);
    glyph.wait_for_screen("Language servers listed", WAIT, |screen| {
        // The cast's first command row, under its `COMMANDS` label.
        screen[9].contains("Language servers")
    });
    glyph.send_keys("enter");
    glyph.wait_for_text(HEADER, WAIT);
}

/// Column where `state` starts on a server row: it ends at `RIGHT`.
fn state_x(state: &str) -> u16 {
    RIGHT - u16::try_from(state.chars().count()).expect("short state")
}

#[test]
fn language_servers_in_the_cast_lists_each_server_and_its_state() {
    let project = tempfile::tempdir().expect("create project");
    let path = programs(&["rust-analyzer", "npm", "vscode-css-language-server"]);
    let mut glyph = with_path(project.path(), path.path());
    open_catalog(&mut glyph);

    // The lit edge across the cast's card, then `✦ language servers` and a rule.
    assert_eq!(
        row(&glyph, CARD_Y).chars().nth(usize::from(CARD_X)),
        Some('▀')
    );
    assert_eq!(glyph.text_col(HEADER_ROW, HEADER), Some(TEXT_X));
    assert_eq!(row(&glyph, RULE_ROW).chars().nth(9), Some('─'));

    let expect = [
        ("Rust", "rust-analyzer", "installed"),
        ("Go", "gopls", "needs go"),
        (
            "TypeScript / JavaScript",
            "typescript-language-server",
            "missing",
        ),
        ("Python", "pyright-langserver", "missing"),
        ("HTML", "vscode-html-language-server", "missing"),
        ("CSS", "vscode-css-language-server", "installed"),
        ("SQL", "sqls", "needs go"),
    ];
    for (i, (name, command, state)) in expect.into_iter().enumerate() {
        let y = FIRST_ROW + u16::try_from(i).expect("seven rows");
        assert_eq!(glyph.text_col(y, name), Some(TEXT_X), "{name}");
        assert_eq!(glyph.text_col(y, command), Some(COMMAND_X), "{name}");
        assert_eq!(glyph.text_col(y, state), Some(state_x(state)), "{name}");
    }
    // Each state has its own colour: `ok`, `warn` and `muted`.
    let installed = glyph.fg_at(state_x("installed"), FIRST_ROW);
    let needs = glyph.fg_at(state_x("needs go"), FIRST_ROW + 1);
    let missing = glyph.fg_at(state_x("missing"), FIRST_ROW + 2);
    assert_ne!(installed, needs);
    assert_ne!(installed, missing);
    assert_ne!(needs, missing);
    assert_eq!(glyph.fg_at(state_x("installed"), FIRST_ROW + 5), installed);

    // A blank, then the footer.
    assert_eq!(row(&glyph, FOOTER_ROW - 1).trim(), "");
    assert_eq!(glyph.text_col(FOOTER_ROW, FOOTER), Some(CARD_X + 2));
}

#[test]
fn a_running_server_says_so() {
    let project = tempfile::tempdir().expect("create project");
    fs::write(project.path().join("a.rs"), "fn a() {}\n").expect("write a.rs");
    let log = tempfile::tempdir().expect("create log dir");
    let config = format!(
        "[lsp.rust]\ncommand = '{}'\n",
        env!("CARGO_BIN_EXE_fake_lsp")
    );
    let env = [(
        "FAKE_LSP_LOG",
        log.path().join("log.jsonl").into_os_string(),
    )];
    let mut glyph = Glyph::spawn_in_with_config_and_env(project.path(), &config, &env, &["a.rs"]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    open_catalog(&mut glyph);
    glyph.wait_for_screen("Rust running", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("running")
    });
    assert_eq!(
        glyph.text_col(FIRST_ROW, "running"),
        Some(state_x("running"))
    );
}

#[test]
fn arrows_move_the_selection_and_esc_or_a_click_outside_close() {
    let project = tempfile::tempdir().expect("create project");
    let path = programs(&[]);
    let mut glyph = with_path(project.path(), path.path());
    open_catalog(&mut glyph);
    let lit = mix(RAISED, ACCENT, 0.3);

    // The first row starts selected, on the glow.
    glyph.wait_for_bg(CARD_X, FIRST_ROW, lit, WAIT);
    assert_ne!(glyph.bg_at(CARD_X, FIRST_ROW + 1), lit);
    glyph.send_keys("down");
    glyph.wait_for_bg(CARD_X, FIRST_ROW + 1, lit, WAIT);
    assert_ne!(glyph.bg_at(CARD_X, FIRST_ROW), lit);
    // ↓ stops at the last row, ↑ at the first.
    for _ in 0..10 {
        glyph.send_keys("down");
    }
    glyph.wait_for_bg(CARD_X, FIRST_ROW + 6, lit, WAIT);
    for _ in 0..10 {
        glyph.send_keys("up");
    }
    glyph.wait_for_bg(CARD_X, FIRST_ROW, lit, WAIT);

    // Typed letters go nowhere; the catalog stays.
    glyph.type_text("x");
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(HEADER, WAIT);

    // A click outside the card closes it too.
    open_catalog(&mut glyph);
    glyph.click(2, ROWS - 3);
    glyph.wait_for_text_gone(HEADER, WAIT);
}

#[test]
fn c_copies_the_install_command_and_says_so() {
    let project = tempfile::tempdir().expect("create project");
    let path = programs(&[]);
    let mut glyph = with_path(project.path(), path.path());
    open_catalog(&mut glyph);
    glyph.send_keys("down");
    glyph.send_keys("c");
    glyph.wait_for_screen("the copy in the status line", WAIT, |screen| {
        screen[usize::from(ROWS - 1)].contains("copied: go install golang.org/x/tools/gopls@latest")
    });
    assert!(row(&glyph, HEADER_ROW).contains(HEADER));
}

#[test]
fn mono_reverses_the_selected_row() {
    let project = tempfile::tempdir().expect("create project");
    let path = programs(&[]);
    let env = [("PATH", OsString::from(path.path()))];
    let mut glyph =
        Glyph::spawn_in_with_config_and_env(project.path(), "theme = \"mono\"\n", &env, &["."]);
    glyph.wait_for_text("Open directory", START);
    open_catalog(&mut glyph);
    // The whole card's width, the state four cells in from its right edge.
    let rust = format!("    {:<25}{:<41}needs rustup    ", "Rust", "rust-analyzer");
    glyph.wait_for_reversed(FIRST_ROW, &rust, WAIT);
    glyph.send_keys("down");
    let go = format!("    {:<25}{:<45}needs go    ", "Go", "gopls");
    glyph.wait_for_reversed(FIRST_ROW + 1, &go, WAIT);
    assert_eq!(glyph.reversed_text(FIRST_ROW), "");
}
