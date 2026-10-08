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
/// in from the card's right edge. A blank, the `debuggers` heading on row 16
/// and the adapters on rows 17 to 19; a blank, then the footer two cells in.
const CARD_X: u16 = 7;
const CARD_Y: u16 = 4;
const HEADER_ROW: u16 = 6;
const RULE_ROW: u16 = 7;
const FIRST_ROW: u16 = 8;
const DEBUGGERS_ROW: u16 = 16;
const LAST_ROW: u16 = 19;
const TEXT_X: u16 = 11;
const COMMAND_X: u16 = 36;
const RIGHT: u16 = 89;
const FOOTER_ROW: u16 = 21;
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
    glyph.wait_for_bg(CARD_X, LAST_ROW, lit, WAIT);
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

/// `command` as the platform shell runs it, so the install's first word (the
/// tool the catalog looks for) is a program on PATH: `cmd` or `sh`.
fn shell(command: &str) -> String {
    if cfg!(windows) {
        format!("cmd /C {command}")
    } else {
        format!("sh -c '{command}'")
    }
}

/// `config.toml` giving Rust the server `command` and the install `install`.
/// Triple-quoted literals, so backslashes and quotes stay as they are. The
/// missing server is called `nope` so the messages naming it fit the 100-cell
/// status line beside the cursor position.
fn rust_install(command: &str, install: &str) -> String {
    format!("[lsp.rust]\ncommand = '''{command}'''\ninstall = '''{install}'''\n")
}

/// Glyph on an empty project with `config`, the PATH it inherits, and its
/// clipboard in the file `clipboard`.
fn with_config(project: &Path, config: &str, clipboard: &Path) -> Glyph {
    let env = [("GLYPH_CLIPBOARD_FILE", OsString::from(clipboard))];
    let glyph = Glyph::spawn_in_with_config_and_env(project, config, &env, &["."]);
    glyph.wait_for_text("Open directory", START);
    glyph
}

/// At 30 rows the run panel's title is row 20 and its output starts on row 21.
const RUN_TITLE_ROW: u16 = 20;
const RUN_OUTPUT_ROW: u16 = 21;

/// Waits for the first server row to say `missing`.
fn wait_missing(glyph: &Glyph) {
    glyph.wait_for_screen("Rust missing", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("missing")
    });
}

#[test]
fn enter_on_a_missing_row_runs_its_install_in_the_run_panel() {
    let project = tempfile::tempdir().expect("create project");
    let clip = tempfile::tempdir().expect("create clipboard dir");
    let config = rust_install("nope", &shell("echo installing rust"));
    let mut glyph = with_config(project.path(), &config, &clip.path().join("clip"));
    open_catalog(&mut glyph);
    wait_missing(&glyph);
    glyph.send_keys("enter");

    // The card closes and the panel, hidden until now, shows the install.
    glyph.wait_for_text_gone(HEADER, WAIT);
    glyph.wait_for_screen("the install's title and output", WAIT, |screen| {
        screen[usize::from(RUN_TITLE_ROW)].contains("install Rust")
            && screen[usize::from(RUN_OUTPUT_ROW)].contains("installing rust")
    });
    // It exited 0, but the server is still nowhere on PATH.
    glyph.wait_for_text(
        "installed, but nope isn't on PATH; restart your terminal",
        WAIT,
    );
}

#[test]
fn a_failed_install_says_its_exit_code() {
    let project = tempfile::tempdir().expect("create project");
    let clip = tempfile::tempdir().expect("create clipboard dir");
    let config = rust_install("nope", &shell("exit 3"));
    let mut glyph = with_config(project.path(), &config, &clip.path().join("clip"));
    open_catalog(&mut glyph);
    wait_missing(&glyph);
    glyph.send_keys("enter");
    glyph.wait_for_text("install failed (exit 3); see the run panel", WAIT);
    assert!(row(&glyph, RUN_TITLE_ROW).contains("install Rust"));
}

#[test]
fn an_install_waits_for_the_running_command() {
    let project = tempfile::tempdir().expect("create project");
    let clip = tempfile::tempdir().expect("create clipboard dir");
    // Long enough to still be running when Enter is pressed again; it's
    // stopped below, never waited out.
    let long = if cfg!(windows) {
        shell("\"echo started & ping -n 60 127.0.0.1 >nul\"")
    } else {
        shell("echo started; sleep 60")
    };
    let config = rust_install("nope", &long);
    let mut glyph = with_config(project.path(), &config, &clip.path().join("clip"));
    open_catalog(&mut glyph);
    wait_missing(&glyph);
    glyph.send_keys("enter");
    glyph.wait_for_screen("the install running", WAIT, |screen| {
        screen[usize::from(RUN_OUTPUT_ROW)].contains("started")
    });

    open_catalog(&mut glyph);
    wait_missing(&glyph);
    glyph.send_keys("enter");
    glyph.wait_for_text("install Rust is running; stop it first", WAIT);
    // The catalog stays open under the message.
    assert_eq!(glyph.text_col(HEADER_ROW, HEADER), Some(TEXT_X));

    glyph.send_keys("esc");
    glyph.wait_for_text_gone(HEADER, WAIT);
    glyph.send_keys("shift+f5");
    glyph.wait_for_screen("the install stopped", WAIT, |screen| {
        screen[usize::from(RUN_TITLE_ROW)].contains("stopped")
    });
}

#[test]
fn enter_copies_a_command_it_cant_run() {
    let project = tempfile::tempdir().expect("create project");
    let clip = tempfile::tempdir().expect("create clipboard dir");
    let clipboard = clip.path().join("clip");

    // `needs rustup`: the PATH holds nothing.
    let path = programs(&[]);
    let env = [
        ("PATH", OsString::from(path.path())),
        ("GLYPH_CLIPBOARD_FILE", OsString::from(&clipboard)),
    ];
    let mut glyph = Glyph::spawn_in_with_env(project.path(), &env, &["."]);
    glyph.wait_for_text("Open directory", START);
    open_catalog(&mut glyph);
    glyph.send_keys("enter");
    glyph.wait_for_text("copied: rustup component add rust-analyzer", WAIT);
    assert_eq!(
        fs::read_to_string(&clipboard).expect("read clipboard"),
        "rustup component add rust-analyzer"
    );
    // The catalog stays, and nothing ran.
    assert_eq!(glyph.text_col(HEADER_ROW, HEADER), Some(TEXT_X));
    assert!(!glyph.screen().iter().any(|l| l.contains("install Rust")));
    drop(glyph);

    // `sudo` would ask for a password the run panel can't take: copied too.
    let sudo = format!("sudo {}", shell("echo hi"));
    let config = rust_install("nope", &sudo);
    let mut glyph = with_config(project.path(), &config, &clipboard);
    open_catalog(&mut glyph);
    wait_missing(&glyph);
    glyph.send_keys("enter");
    glyph.wait_for_text(&format!("copied: {sudo}"), WAIT);
    assert_eq!(
        fs::read_to_string(&clipboard).expect("read clipboard"),
        sudo
    );
}

/// Presses Enter on the selected row, then ↓: keys are handled in order, so
/// once the selection has moved, Enter has been handled too.
fn enter_then_down(glyph: &mut Glyph) {
    let lit = mix(RAISED, ACCENT, 0.3);
    glyph.wait_for_bg(CARD_X, FIRST_ROW, lit, WAIT);
    glyph.send_keys("enter");
    glyph.send_keys("down");
    glyph.wait_for_bg(CARD_X, FIRST_ROW + 1, lit, WAIT);
}

#[test]
fn enter_on_an_installed_row_does_nothing() {
    let project = tempfile::tempdir().expect("create project");
    let path = programs(&["rust-analyzer"]);
    let mut glyph = with_path(project.path(), path.path());
    open_catalog(&mut glyph);
    enter_then_down(&mut glyph);
    assert_eq!(glyph.text_col(HEADER_ROW, HEADER), Some(TEXT_X));
    let screen = glyph.screen();
    assert!(
        !screen.iter().any(|l| l.contains("install Rust")),
        "{screen:#?}"
    );
    assert!(!screen.iter().any(|l| l.contains("copied")), "{screen:#?}");
}

#[test]
fn enter_on_a_running_row_does_nothing() {
    let project = tempfile::tempdir().expect("create project");
    fs::write(project.path().join("a.rs"), "fn a() {}\n").expect("write a.rs");
    let log = tempfile::tempdir().expect("create log dir");
    let config = rust_install(env!("CARGO_BIN_EXE_fake_lsp"), &shell("echo again"));
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
    enter_then_down(&mut glyph);
    assert_eq!(glyph.text_col(HEADER_ROW, HEADER), Some(TEXT_X));
    assert!(!glyph.screen().iter().any(|l| l.contains("install Rust")));
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(HEADER, WAIT);
    // Quit cleanly so the fake server exits too.
    glyph.send_keys("ctrl+q");
    glyph.wait_exit(WAIT);
}

#[test]
fn a_server_found_after_its_install_starts_for_the_open_files() {
    let project = tempfile::tempdir().expect("create project");
    fs::write(project.path().join("a.rs"), "fn a() {}\n").expect("write a.rs");
    let files = tempfile::tempdir().expect("create server dir");
    let log = files.path().join("log.jsonl");
    // The fake server, copied into place by the install: absent until then.
    let server = files.path().join(if cfg!(windows) {
        "glyph-fake-server.exe"
    } else {
        "glyph-fake-server"
    });
    let fake = env!("CARGO_BIN_EXE_fake_lsp");
    let install = if cfg!(windows) {
        shell(&format!("copy /Y \"{fake}\" \"{}\"", server.display()))
    } else {
        format!("cp '{fake}' '{}'", server.display())
    };
    let config = rust_install(&server.display().to_string(), &install);
    let env = [("FAKE_LSP_LOG", log.clone().into_os_string())];
    let mut glyph = Glyph::spawn_in_with_config_and_env(project.path(), &config, &env, &["a.rs"]);
    glyph.wait_for_text("rust: server not found", START);

    open_catalog(&mut glyph);
    wait_missing(&glyph);
    glyph.send_keys("enter");
    glyph.wait_for_text("Rust installed", WAIT);
    // Without restarting Glyph, the server now follows a.rs.
    glyph.wait_for_files("the server to open a.rs", WAIT, || {
        fs::read_to_string(&log)
            .is_ok_and(|log| log.contains("textDocument/didOpen") && log.contains("a.rs"))
    });
    // Quit cleanly so the server exits and its folder can go.
    glyph.send_keys("ctrl+q");
    glyph.wait_exit(WAIT);
}

#[test]
fn server_not_found_says_the_catalog_installs_it() {
    let project = tempfile::tempdir().expect("create project");
    fs::write(project.path().join("a.rs"), "fn a() {}\n").expect("write a.rs");
    let config = "[lsp.rust]\ncommand = 'nope'\n";
    let glyph = Glyph::spawn_in_with_config(project.path(), config, &["a.rs"]);
    glyph.wait_for_text(
        "rust: server not found (nope) · >language servers installs it",
        START,
    );
}

/// The tool `lldb-dap`'s install runs on this platform.
const LLDB_TOOL: &str = if cfg!(windows) { "winget" } else { "apt" };
const LLDB_INSTALL: &str = if cfg!(windows) {
    "winget install --id LLVM.LLVM -e --accept-source-agreements --accept-package-agreements"
} else {
    "sudo apt install lldb"
};

/// Writes the script `name` into `dir`: `windows` as `name.cmd`, `unix` as an
/// executable `name`. The catalog finds either as the program `name`.
fn script(dir: &Path, name: &str, windows: &str, unix: &str) {
    if cfg!(windows) {
        fs::write(dir.join(format!("{name}.cmd")), windows).expect("write fake script");
    } else {
        let path = dir.join(name);
        fs::write(&path, unix).expect("write fake script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .expect("make fake script executable");
        }
    }
}

/// `dir` on PATH ahead of the folders the platform shell and the scripts'
/// commands come from, but none of the user's own programs.
fn path_with_shell(dir: &Path) -> OsString {
    let system: Vec<std::path::PathBuf> = if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        vec![Path::new(&root).join("System32")]
    } else {
        vec!["/usr/bin".into(), "/bin".into()]
    };
    std::env::join_paths(std::iter::once(dir.to_path_buf()).chain(system)).expect("join PATH")
}

/// Glyph on an empty project with `path` as its PATH, an empty LLVM folder (so
/// an LLVM installed on this machine doesn't count), and its clipboard in the
/// file `clipboard`.
fn with_adapters_path(project: &Path, path: OsString, clipboard: &Path) -> (Glyph, TempDir) {
    let program_files = tempfile::tempdir().expect("create Program Files");
    let env = [
        ("PATH", path),
        ("ProgramFiles", program_files.path().as_os_str().to_owned()),
        ("GLYPH_CLIPBOARD_FILE", clipboard.as_os_str().to_owned()),
    ];
    let glyph = Glyph::spawn_in_with_env(project, &env, &["."]);
    glyph.wait_for_text("Open directory", START);
    (glyph, program_files)
}

/// Moves the selection from the first row down to row `y`.
fn select(glyph: &mut Glyph, y: u16) {
    let lit = mix(RAISED, ACCENT, 0.3);
    glyph.wait_for_bg(CARD_X, FIRST_ROW, lit, WAIT);
    // The heading isn't a row: the adapters follow SQL a ↓ each, so this
    // stops short of `y` by the heading and its blank.
    for _ in FIRST_ROW..y - 2 {
        glyph.send_keys("down");
    }
    glyph.wait_for_bg(CARD_X, y, lit, WAIT);
}

#[test]
fn debuggers_are_listed_under_their_heading_and_copy_their_install() {
    let project = tempfile::tempdir().expect("create project");
    let clip = tempfile::tempdir().expect("create clipboard dir");
    let clipboard = clip.path().join("clip");
    // lldb-dap's install tool and a Python that can't import anything, but
    // no Go.
    let path = programs(&[LLDB_TOOL]);
    script(
        path.path(),
        "python",
        "@exit /b 1\r\n",
        "#!/bin/sh\nexit 1\n",
    );
    let (mut glyph, _llvm) =
        with_adapters_path(project.path(), OsString::from(path.path()), &clipboard);
    open_catalog(&mut glyph);

    assert_eq!(row(&glyph, DEBUGGERS_ROW - 1).trim(), "");
    assert_eq!(glyph.text_col(DEBUGGERS_ROW, "debuggers"), Some(TEXT_X));
    assert!(glyph.bold_at(TEXT_X, DEBUGGERS_ROW));
    let expect = [
        ("lldb-dap", "lldb-dap", "missing"),
        ("debugpy", "python -m debugpy.adapter", "missing"),
        ("dlv", "dlv dap", "needs go"),
    ];
    for (i, (name, command, state)) in expect.into_iter().enumerate() {
        let y = DEBUGGERS_ROW + 1 + u16::try_from(i).expect("three rows");
        assert_eq!(glyph.text_col(y, name), Some(TEXT_X), "{name}");
        // lldb-dap's command is its name, so look from the command column.
        let from_command: String = row(&glyph, y)
            .chars()
            .skip(usize::from(COMMAND_X))
            .collect();
        assert!(from_command.starts_with(command), "{from_command:?}");
        assert_eq!(glyph.text_col(y, state), Some(state_x(state)), "{name}");
    }
    // The states take the servers' colours.
    let missing = glyph.fg_at(state_x("missing"), DEBUGGERS_ROW + 2);
    let needs = glyph.fg_at(state_x("needs go"), LAST_ROW);
    assert_ne!(missing, needs);
    assert_eq!(glyph.fg_at(state_x("needs go"), FIRST_ROW + 6), needs);
    assert_eq!(row(&glyph, FOOTER_ROW - 1).trim(), "");
    assert_eq!(glyph.text_col(FOOTER_ROW, FOOTER), Some(CARD_X + 2));

    // `c` on lldb-dap copies its install command.
    select(&mut glyph, DEBUGGERS_ROW + 1);
    glyph.send_keys("c");
    glyph.wait_for_text("copied: ", WAIT);
    assert_eq!(
        fs::read_to_string(&clipboard).expect("read clipboard"),
        LLDB_INSTALL
    );
    // Enter on dlv, whose install needs Go, copies too.
    glyph.send_keys("down");
    glyph.send_keys("down");
    glyph.wait_for_bg(CARD_X, LAST_ROW, mix(RAISED, ACCENT, 0.3), WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text(
        "copied: go install github.com/go-delve/delve/cmd/dlv@latest",
        WAIT,
    );
    // Off Windows lldb-dap's install needs `sudo`: Enter copies it rather
    // than running it.
    if !cfg!(windows) {
        fs::remove_file(&clipboard).expect("clear clipboard");
        glyph.send_keys("up");
        glyph.send_keys("up");
        glyph.wait_for_bg(CARD_X, DEBUGGERS_ROW + 1, mix(RAISED, ACCENT, 0.3), WAIT);
        glyph.send_keys("enter");
        glyph.wait_for_text("copied: sudo apt install lldb", WAIT);
        assert_eq!(
            fs::read_to_string(&clipboard).expect("read clipboard"),
            LLDB_INSTALL
        );
    }
    assert!(!glyph.screen().iter().any(|l| l.contains("install dlv")));
}

/// A fake `go` that says what it was asked and, when `creates`, leaves an
/// empty `dlv` beside itself, as `go install` would in `~/go/bin`.
fn fake_go(dir: &Path, creates: bool) {
    let (windows, unix) = if creates {
        (
            "@echo off\r\necho fake go %*\r\ntype nul > \"%~dp0dlv.exe\"\r\n",
            "#!/bin/sh\necho \"fake go $*\"\nd=$(dirname \"$0\")\n: > \"$d/dlv\"\nchmod +x \"$d/dlv\"\n",
        )
    } else {
        (
            "@echo off\r\necho fake go %*\r\n",
            "#!/bin/sh\necho \"fake go $*\"\n",
        )
    };
    script(dir, "go", windows, unix);
}

#[test]
fn enter_on_dlv_runs_its_install_and_finds_it_after() {
    let project = tempfile::tempdir().expect("create project");
    let clip = tempfile::tempdir().expect("create clipboard dir");
    let bin = tempfile::tempdir().expect("create PATH folder");
    fake_go(bin.path(), true);
    let (mut glyph, _llvm) = with_adapters_path(
        project.path(),
        path_with_shell(bin.path()),
        &clip.path().join("clip"),
    );
    open_catalog(&mut glyph);
    glyph.wait_for_screen("dlv missing", WAIT, |screen| {
        screen[usize::from(LAST_ROW)].contains("missing")
    });
    select(&mut glyph, LAST_ROW);
    glyph.send_keys("enter");

    glyph.wait_for_text_gone(HEADER, WAIT);
    glyph.wait_for_screen("the install's title and output", WAIT, |screen| {
        screen[usize::from(RUN_TITLE_ROW)].contains("install dlv")
            && screen[usize::from(RUN_OUTPUT_ROW)]
                .contains("fake go install github.com/go-delve/delve/cmd/dlv@latest")
    });
    glyph.wait_for_text("dlv installed", WAIT);

    // Opened again, the catalog finds it.
    open_catalog(&mut glyph);
    glyph.wait_for_screen("dlv installed", WAIT, |screen| {
        screen[usize::from(LAST_ROW)].contains("installed")
    });
    assert_eq!(
        glyph.text_col(LAST_ROW, "installed"),
        Some(state_x("installed"))
    );
}

#[test]
fn an_adapter_still_missing_after_its_install_says_so() {
    let project = tempfile::tempdir().expect("create project");
    let clip = tempfile::tempdir().expect("create clipboard dir");
    let bin = tempfile::tempdir().expect("create PATH folder");
    fake_go(bin.path(), false);
    let (mut glyph, _llvm) = with_adapters_path(
        project.path(),
        path_with_shell(bin.path()),
        &clip.path().join("clip"),
    );
    open_catalog(&mut glyph);
    glyph.wait_for_screen("dlv missing", WAIT, |screen| {
        screen[usize::from(LAST_ROW)].contains("missing")
    });
    select(&mut glyph, LAST_ROW);
    glyph.send_keys("enter");
    glyph.wait_for_text(
        "installed, but dlv isn't on PATH; restart your terminal",
        WAIT,
    );
}

#[test]
fn debugpy_is_installed_once_python_can_import_it() {
    let project = tempfile::tempdir().expect("create project");
    let clip = tempfile::tempdir().expect("create clipboard dir");
    let bin = tempfile::tempdir().expect("create PATH folder");
    // A fake Python: `-c` (the import check) succeeds once its pip has run.
    script(
        bin.path(),
        "python",
        "@echo off\r\n\
         if \"%~1\"==\"-c\" goto import\r\n\
         echo fake pip %*\r\n\
         type nul > \"%~dp0debugpy-installed\"\r\n\
         exit /b 0\r\n\
         :import\r\n\
         if exist \"%~dp0debugpy-installed\" exit /b 0\r\n\
         exit /b 1\r\n",
        "#!/bin/sh\n\
         d=$(dirname \"$0\")\n\
         if [ \"$1\" = \"-c\" ]; then\n\
         test -f \"$d/debugpy-installed\"\n\
         exit $?\n\
         fi\n\
         echo \"fake pip $*\"\n\
         : > \"$d/debugpy-installed\"\n",
    );
    let (mut glyph, _llvm) = with_adapters_path(
        project.path(),
        path_with_shell(bin.path()),
        &clip.path().join("clip"),
    );
    open_catalog(&mut glyph);
    // Python is there, but it can't import debugpy yet.
    let debugpy = DEBUGGERS_ROW + 2;
    glyph.wait_for_screen("debugpy missing", WAIT, |screen| {
        screen[usize::from(debugpy)].contains("missing")
    });
    select(&mut glyph, debugpy);
    glyph.send_keys("enter");
    glyph.wait_for_screen("the install's title and output", WAIT, |screen| {
        screen[usize::from(RUN_TITLE_ROW)].contains("install debugpy")
            && screen[usize::from(RUN_OUTPUT_ROW)].contains("fake pip -m pip install debugpy")
    });
    glyph.wait_for_text("debugpy installed", WAIT);

    open_catalog(&mut glyph);
    glyph.wait_for_screen("debugpy installed", WAIT, |screen| {
        screen[usize::from(debugpy)].contains("installed")
    });
}
