mod harness;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use harness::{COLS, Griffin, ROWS};
use tempfile::TempDir;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const QUESTION: &str = "Recover unsaved changes?";
const CARD: &str = "Recover unsaved changes? Recover / Discard";

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

    fn launch(&self) -> Griffin {
        let griffin = Griffin::spawn_in_with_data(self.files.path(), self.data.path(), &["a.txt"]);
        griffin.wait_for_text("a.txt", START);
        griffin
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

fn status_line(griffin: &Griffin) -> String {
    griffin.screen()[usize::from(ROWS - 1)].clone()
}

/// Edits `a.txt`, waits for its backup to land, then kills griffin without
/// letting it clean up.
fn edit_and_crash(setup: &Setup) {
    let mut griffin = setup.launch();
    griffin.wait_for_text("Ln 1, Col 1", START);
    griffin.type_text("xy");
    griffin.wait_for_text("xyhello", WAIT);
    let typed = Instant::now();
    griffin.wait_for_files("a backup file", WAIT, || {
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
    griffin.kill();
    assert_eq!(setup.read_a(), "hello\n", "nothing was saved");
}

/// Waits for the recover card and checks it sits in the middle of the screen.
fn wait_for_card(griffin: &Griffin) {
    griffin.wait_for_text(QUESTION, START);
    let screen = griffin.screen();
    let row = screen
        .iter()
        .position(|line| line.contains(CARD))
        .unwrap_or_else(|| panic!("{screen:#?}"));
    assert!((13..=16).contains(&row), "card on row {row}: {screen:#?}");
    let line = &screen[row];
    assert!(
        line.contains("Recover") && line.contains("Discard"),
        "{line}"
    );
    let col = griffin
        .text_col(row as u16, CARD)
        .unwrap_or_else(|| panic!("{screen:#?}"));
    let right = COLS - (col + CARD.len() as u16);
    assert!(col.abs_diff(right) <= 1, "card not centred: {screen:#?}");
}

#[test]
fn recover_after_a_crash_loads_the_backup() {
    let setup = Setup::new("hello\n");
    edit_and_crash(&setup);

    let mut griffin = setup.launch();
    wait_for_card(&griffin);
    griffin.type_text("r");
    griffin.wait_for_text_gone(QUESTION, WAIT);
    griffin.wait_for_text("xyhello", WAIT);
    griffin.wait_for_text("a.txt ●", WAIT);
    assert!(status_line(&griffin).contains("a.txt ●"));
    // The backup stays until the recovered text is saved.
    assert_eq!(setup.backup_files().len(), 1);

    griffin.send_keys("ctrl+s");
    griffin.wait_for_text("saved a.txt", WAIT);
    assert_eq!(setup.read_a(), "xyhello\n");
    griffin.wait_for_files("the backup to go", WAIT, || setup.backup_files().is_empty());
    griffin.send_keys("ctrl+q");
    assert!(griffin.wait_exit(WAIT).success());
}

#[test]
fn discard_after_a_crash_deletes_the_backup() {
    let setup = Setup::new("hello\n");
    edit_and_crash(&setup);

    let mut griffin = setup.launch();
    wait_for_card(&griffin);
    griffin.type_text("d");
    griffin.wait_for_text_gone(QUESTION, WAIT);
    griffin.wait_for_text("hello", WAIT);
    let status = status_line(&griffin);
    assert!(!status.contains('●'), "dirty after discard: {status:?}");
    assert!(!griffin.screen().iter().any(|l| l.contains("xyhello")));
    griffin.wait_for_files("the backup to go", WAIT, || setup.backup_files().is_empty());

    // Nothing is offered the next time.
    griffin.send_keys("ctrl+q");
    assert!(griffin.wait_exit(WAIT).success());
    let mut griffin = setup.launch();
    griffin.wait_for_text("Ln 1, Col 1", START);
    griffin.assert_running_for(Duration::from_millis(300));
    assert!(!griffin.screen().iter().any(|l| l.contains(QUESTION)));
    griffin.send_keys("ctrl+q");
    assert!(griffin.wait_exit(WAIT).success());
}

#[test]
fn quitting_cleanly_deletes_the_backup() {
    let setup = Setup::new("hello\n");
    let mut griffin = setup.launch();
    griffin.wait_for_text("Ln 1, Col 1", START);
    griffin.type_text("x");
    griffin.wait_for_text("a.txt ●", WAIT);
    griffin.wait_for_files("a backup file", WAIT, || !setup.backup_files().is_empty());
    griffin.send_keys("ctrl+q");
    griffin.wait_for_text("Unsaved changes", WAIT);
    griffin.type_text("d");
    assert!(griffin.wait_exit(WAIT).success());
    assert!(setup.backup_files().is_empty());
    assert_eq!(setup.read_a(), "hello\n");
}
