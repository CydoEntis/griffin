mod harness;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use harness::{ROWS, Tome};
use tempfile::TempDir;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const QUESTION: &str = "Recover unsaved changes to a.txt?";
/// At 100x30 the card is 59 wide (the 51-cell explanation + 8) and 7 tall,
/// centred (SPEC_V1_LAYOUT §7.4): from column 20 and row 11, with its text at
/// column 23 and the buttons on row 15, ` Recover ` then ` Discard ` at 34.
const CARD_X: u16 = 20;
const CARD_Y: u16 = 11;
const TEXT_X: u16 = 23;
const BUTTON_ROW: u16 = 15;
const DISCARD_X: u16 = 34;
/// The default theme's `accent`, where the focused button starts.
const ACCENT: vt100::Color = vt100::Color::Rgb(0xc3, 0xf5, 0x3c);

/// A project folder holding `a.txt` and a separate data dir that survives relaunches.
struct Setup {
    files: TempDir,
    data: TempDir,
}

impl Setup {
    fn new(contents: &str) -> Self {
        let files = tempfile::tempdir().expect("create temp dir");
        let data = tempfile::tempdir().expect("create temp data dir");
        fs::write(files.path().join("a.txt"), contents).expect("write a.txt");
        Self { files, data }
    }

    fn launch(&self) -> Tome {
        let tome = Tome::spawn_in_with_data(self.files.path(), self.data.path(), &["a.txt"]);
        tome.wait_for_text("a.txt", START);
        tome
    }

    fn backups(&self) -> PathBuf {
        self.data.path().join("backups")
    }

    fn backup_files(&self) -> Vec<PathBuf> {
        backup_files(&self.backups())
    }

    fn read_a(&self) -> String {
        fs::read_to_string(self.files.path().join("a.txt")).expect("read a.txt")
    }
}

/// Finished backups only: a write in progress shows up as a dot-prefixed temp file.
fn backup_files(dir: &Path) -> Vec<PathBuf> {
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .map(|e| e.expect("read backups entry").path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "bak"))
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn status_line(tome: &Tome) -> String {
    tome.screen()[usize::from(ROWS - 1)].clone()
}

/// Edits `a.txt`, waits for its backup to land, then kills tome without
/// letting it clean up.
fn edit_and_crash(setup: &Setup) {
    let mut tome = setup.launch();
    tome.wait_for_text("Ln 1, Col 1", START);
    tome.type_text("xy");
    tome.wait_for_text("xyhello", WAIT);
    let typed = Instant::now();
    tome.wait_for_files("a backup file", WAIT, || {
        setup
            .backup_files()
            .iter()
            .any(|p| fs::read_to_string(p).is_ok_and(|t| t == "xyhello\n"))
    });
    // R12 allows 2 s; a slow CI box gets a little slack on top.
    let took = typed.elapsed();
    assert!(took < Duration::from_secs(3), "backup took {took:?}");
    let backups = setup.backup_files();
    assert_eq!(backups.len(), 1, "{backups:?}");
    let name = backups[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    // Named by a hash of the path: 16 hex digits.
    assert!(
        name.len() == 20 && name[..16].chars().all(|c| c.is_ascii_hexdigit()),
        "{name}"
    );
    tome.kill();
    assert_eq!(setup.read_a(), "hello\n", "nothing was saved");
}

/// Waits for the recover card and checks its copy, with the backup's time, and
/// buttons sit where §7.4 puts them.
fn wait_for_card(setup: &Setup, tome: &Tome) {
    tome.wait_for_text(QUESTION, START);
    let backups = setup.backup_files();
    let written = fs::metadata(&backups[0])
        .and_then(|m| m.modified())
        .expect("backup time");
    let explanation = format!(
        "A backup from {} is newer than the file on disk.",
        chrono::DateTime::<chrono::Local>::from(written).format("%H:%M")
    );
    let screen = tome.screen();
    assert_eq!(
        tome.text_col(CARD_Y, &"▀".repeat(59)),
        Some(CARD_X),
        "{screen:#?}"
    );
    assert_eq!(
        tome.text_col(CARD_Y + 1, QUESTION),
        Some(TEXT_X),
        "{screen:#?}"
    );
    assert_eq!(
        tome.text_col(CARD_Y + 2, &explanation),
        Some(TEXT_X),
        "{screen:#?}"
    );
    assert_eq!(
        tome.text_col(BUTTON_ROW, "Recover    Discard"),
        Some(TEXT_X + 1),
        "{screen:#?}"
    );
    assert_eq!(tome.bg_at(TEXT_X, BUTTON_ROW), ACCENT);
    assert_eq!(tome.underlined_text(BUTTON_ROW), "RD");
}

#[test]
fn recover_after_a_crash_loads_the_backup() {
    let setup = Setup::new("hello\n");
    edit_and_crash(&setup);

    let mut tome = setup.launch();
    wait_for_card(&setup, &tome);
    tome.type_text("r");
    tome.wait_for_text_gone(QUESTION, WAIT);
    tome.wait_for_text("xyhello", WAIT);
    tome.wait_for_text("a.txt •", WAIT);
    assert!(status_line(&tome).contains("a.txt •"));
    // The backup stays until the recovered text is saved.
    assert_eq!(setup.backup_files().len(), 1);

    tome.send_keys("ctrl+s");
    tome.wait_for_text("saved a.txt", WAIT);
    assert_eq!(setup.read_a(), "xyhello\n");
    tome.wait_for_files("the backup to go", WAIT, || setup.backup_files().is_empty());
    tome.send_keys("ctrl+q");
    assert!(tome.wait_exit(WAIT).success());
}

#[test]
fn discard_after_a_crash_deletes_the_backup() {
    let setup = Setup::new("hello\n");
    edit_and_crash(&setup);

    let mut tome = setup.launch();
    wait_for_card(&setup, &tome);
    tome.type_text("d");
    tome.wait_for_text_gone(QUESTION, WAIT);
    tome.wait_for_text("hello", WAIT);
    let status = status_line(&tome);
    assert!(!status.contains('●'), "dirty after discard: {status:?}");
    assert!(!tome.screen().iter().any(|l| l.contains("xyhello")));
    tome.wait_for_files("the backup to go", WAIT, || setup.backup_files().is_empty());

    // Nothing is offered the next time.
    tome.send_keys("ctrl+q");
    assert!(tome.wait_exit(WAIT).success());
    let mut tome = setup.launch();
    tome.wait_for_text("Ln 1, Col 1", START);
    tome.assert_running_for(Duration::from_millis(300));
    assert!(!tome.screen().iter().any(|l| l.contains(QUESTION)));
    tome.send_keys("ctrl+q");
    assert!(tome.wait_exit(WAIT).success());
}

#[test]
fn quitting_cleanly_deletes_the_backup() {
    let setup = Setup::new("hello\n");
    let mut tome = setup.launch();
    tome.wait_for_text("Ln 1, Col 1", START);
    tome.type_text("x");
    tome.wait_for_text("a.txt •", WAIT);
    tome.wait_for_files("a backup file", WAIT, || !setup.backup_files().is_empty());
    tome.send_keys("ctrl+q");
    tome.wait_for_text("has unsaved changes", WAIT);
    tome.type_text("d");
    assert!(tome.wait_exit(WAIT).success());
    assert!(setup.backup_files().is_empty());
    assert_eq!(setup.read_a(), "hello\n");
}

#[test]
fn enter_recovers_and_a_click_on_discard_discards() {
    let setup = Setup::new("hello\n");
    edit_and_crash(&setup);

    // Recover is the default: Enter presses it.
    let mut tome = setup.launch();
    wait_for_card(&setup, &tome);
    tome.send_keys("enter");
    tome.wait_for_text_gone(QUESTION, WAIT);
    tome.wait_for_text("xyhello", WAIT);
    tome.kill();

    // The backup is still there, so it's offered again; a click discards it.
    let mut tome = setup.launch();
    wait_for_card(&setup, &tome);
    tome.click(DISCARD_X + 2, BUTTON_ROW);
    tome.wait_for_text_gone(QUESTION, WAIT);
    tome.wait_for_files("the backup to go", WAIT, || setup.backup_files().is_empty());
    assert!(!tome.screen().iter().any(|l| l.contains("xyhello")));
    tome.send_keys("ctrl+q");
    assert!(tome.wait_exit(WAIT).success());
}
