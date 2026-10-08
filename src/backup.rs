//! Crash backups (R12): a dirty buffer's text is copied to `<data dir>/backups/`
//! shortly after the last edit, so a crash loses at most a second or two of work.
//! Reopening the file offers the backup back; saving or closing cleanly removes it.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use tokio::time::sleep_until;

use crate::app::AppEvent;
use crate::save::save_atomic;

/// Quiet time after the last edit before a backup is written. Well under the 2 s
/// R12 allows, leaving room for the write itself.
pub const DEBOUNCE: Duration = Duration::from_millis(750);

/// When the next backup is due: `DEBOUNCE` after the latest edit. Time is passed
/// in, so the schedule is testable without waiting.
#[derive(Debug, Default, Clone, Copy)]
pub struct Debounce {
    due: Option<Instant>,
}

impl Debounce {
    /// An edit at `now` pushes the deadline back.
    pub fn edit(&mut self, now: Instant) {
        self.due = Some(now + DEBOUNCE);
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.due
    }

    /// Whether the backup is due at `now`; true at most once per run of edits.
    pub fn fire(&mut self, now: Instant) -> bool {
        match self.due {
            Some(due) if now >= due => {
                self.due = None;
                true
            }
            _ => false,
        }
    }
}

/// Starts the debounce timer task. Send `()` on the returned channel after every
/// edit; once edits stop for `DEBOUNCE`, the task sends `AppEvent::BackupDue`. The
/// task never sees `App` (ADR-0001); it only says when.
pub fn spawn_timer(app: mpsc::UnboundedSender<AppEvent>) -> mpsc::UnboundedSender<()> {
    let (tx, mut edits) = mpsc::unbounded_channel::<()>();
    tokio::spawn(async move {
        let mut debounce = Debounce::default();
        loop {
            match debounce.deadline() {
                Some(due) => {
                    tokio::select! {
                        edit = edits.recv() => match edit {
                            Some(()) => debounce.edit(Instant::now()),
                            None => return,
                        },
                        () = sleep_until(due.into()) => {
                            if debounce.fire(Instant::now()) && app.send(AppEvent::BackupDue).is_err() {
                                return;
                            }
                        }
                    }
                }
                None => match edits.recv().await {
                    Some(()) => debounce.edit(Instant::now()),
                    None => return,
                },
            }
        }
    });
    tx
}

/// Where backups live and which one belongs to which buffer.
#[derive(Debug, Clone)]
pub struct Backups {
    /// `<data dir>/backups`; `None` when the OS has no data dir, which turns
    /// backups off.
    dir: Option<PathBuf>,
    /// Names this run's untitled buffers, which have no path to hash; each one adds
    /// its tab's number.
    session: String,
    /// Serialises writes against deletes, so a write already in flight can't
    /// bring a backup back after saving or closing removed it. The number is bumped
    /// by every delete; a write started before it is dropped.
    epoch: Arc<Mutex<u64>>,
}

impl Default for Backups {
    /// Backups switched off; unit tests that don't care about them get this.
    fn default() -> Self {
        Self::new(None)
    }
}

impl Backups {
    /// `data_dir` is glyph's own data folder; backups go in `backups/` under it.
    pub fn new(data_dir: Option<PathBuf>) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        Self {
            dir: data_dir.map(|dir| dir.join("backups")),
            session: format!("untitled-{}-{nanos}", std::process::id()),
            epoch: Arc::default(),
        }
    }

    /// The backup file for a buffer with `path`, or for the untitled buffer in tab
    /// `untitled` when `path` is `None`. Several untitled tabs can be open at once,
    /// so the session alone can't tell their backups apart.
    pub fn path_for(&self, path: Option<&Path>, untitled: u64) -> Option<PathBuf> {
        let dir = self.dir.as_ref()?;
        let name = match path {
            Some(path) => {
                let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
                format!(
                    "{:016x}.bak",
                    fnv1a(absolute.as_os_str().as_encoded_bytes())
                )
            }
            None => format!("{}-{untitled}.bak", self.session),
        };
        Some(dir.join(name))
    }

    /// A write of `text` for the buffer at `path`, to run off the main task.
    /// `None` when backups are off.
    pub fn job(&self, path: Option<&Path>, untitled: u64, text: String) -> Option<Job> {
        Some(Job {
            target: self.path_for(path, untitled)?,
            text,
            epoch: Arc::clone(&self.epoch),
            started: *self.lock(),
        })
    }

    /// The backup's text if it should be offered when opening `path`: it is newer
    /// than the file (or the file is gone) and says something different. A backup
    /// identical to the file is useless and is removed.
    pub fn recoverable(&self, path: &Path) -> Option<String> {
        let backup = self.path_for(Some(path), 0)?;
        let backup_time = fs::metadata(&backup).and_then(|m| m.modified()).ok()?;
        let file = fs::read(path).ok();
        let file_time = fs::metadata(path).and_then(|m| m.modified()).ok();
        if file_time.is_some_and(|file_time| backup_time <= file_time) {
            return None;
        }
        let text = fs::read_to_string(&backup).ok()?;
        if file.as_deref() == Some(text.as_bytes()) {
            let _ = self.delete(Some(path), 0);
            return None;
        }
        Some(text)
    }

    /// When `path`'s backup was last written, if it has one.
    pub fn written(&self, path: &Path) -> Option<SystemTime> {
        let backup = self.path_for(Some(path), 0)?;
        fs::metadata(backup).and_then(|m| m.modified()).ok()
    }

    /// Removes the buffer's backup, if it has one. Waits for a write in flight to
    /// finish first, and stops any that hasn't started from landing afterwards.
    pub fn delete(&self, path: Option<&Path>, untitled: u64) -> io::Result<()> {
        let Some(backup) = self.path_for(path, untitled) else {
            return Ok(());
        };
        let mut epoch = self.lock();
        *epoch += 1;
        match fs::remove_file(backup) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, u64> {
        // The guarded value is a plain counter; a panic elsewhere can't leave it
        // half-updated, so a poisoned lock is still fine to use.
        self.epoch.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One backup write, carrying a snapshot of the text so the task needs nothing
/// from `App`.
#[derive(Debug)]
pub struct Job {
    target: PathBuf,
    text: String,
    epoch: Arc<Mutex<u64>>,
    started: u64,
}

impl Job {
    /// Writes the backup atomically, so a crash mid-write leaves the previous one.
    /// Skipped if the backup was deleted since the job was made.
    pub fn run(self) -> io::Result<()> {
        let epoch = self.epoch.lock().unwrap_or_else(PoisonError::into_inner);
        if *epoch != self.started {
            return Ok(());
        }
        if let Some(dir) = self.target.parent() {
            fs::create_dir_all(dir)?;
        }
        save_atomic(&self.target, &self.text)
    }
}

/// Glyph's data folder: `GLYPH_DATA_DIR` when set and non-empty, otherwise the
/// OS data dir's `glyph` folder (next to the config dir's, which #3 put under
/// `BaseDirs`).
pub fn data_dir(env_override: Option<OsString>) -> Option<PathBuf> {
    if let Some(dir) = env_override.filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    directories::BaseDirs::new().map(|dirs| dirs.data_dir().join("glyph"))
}

/// 64-bit FNV-1a. Backup names must stay the same across runs and Rust versions,
/// which `DefaultHasher` doesn't promise.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backups_in(dir: &Path) -> Vec<String> {
        match fs::read_dir(dir.join("backups")) {
            Ok(entries) => {
                let mut names: Vec<String> = entries
                    .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect();
                names.sort();
                names
            }
            Err(_) => Vec::new(),
        }
    }

    #[test]
    fn backup_lands_within_two_seconds_of_the_last_edit() -> io::Result<()> {
        let data = tempfile::tempdir()?;
        let files = tempfile::tempdir()?;
        let file = files.path().join("a.txt");
        let backups = Backups::new(Some(data.path().to_path_buf()));
        let mut debounce = Debounce::default();

        let t0 = Instant::now();
        debounce.edit(t0);
        // Typing keeps pushing the backup back...
        let last = t0 + Duration::from_millis(500);
        assert!(!debounce.fire(last));
        debounce.edit(last);
        assert!(!debounce.fire(t0 + DEBOUNCE));
        // ...until it stops; then it is due within 2 s of the last keystroke.
        let due = debounce.deadline().expect("an edit schedules a backup");
        assert!(due > last && due <= last + Duration::from_secs(2));
        assert!(debounce.fire(due));
        assert!(
            !debounce.fire(due + DEBOUNCE),
            "fires once per run of edits"
        );

        backups
            .job(Some(&file), 0, "edited\n".into())
            .expect("backups are on")
            .run()?;
        let absolute = std::path::absolute(&file)?;
        let name = format!(
            "{:016x}.bak",
            fnv1a(absolute.as_os_str().as_encoded_bytes())
        );
        assert_eq!(backups_in(data.path()), std::slice::from_ref(&name));
        assert_eq!(
            fs::read_to_string(data.path().join("backups").join(name))?,
            "edited\n"
        );
        Ok(())
    }

    #[test]
    fn names_hash_the_absolute_path_and_untitled_uses_the_session() {
        let backups = Backups::new(Some(PathBuf::from("data")));
        let relative = backups.path_for(Some(Path::new("a.txt")), 0);
        let absolute = std::path::absolute("a.txt").unwrap();
        assert_eq!(relative, backups.path_for(Some(&absolute), 0));
        assert_ne!(relative, backups.path_for(Some(Path::new("b.txt")), 0));

        let untitled = backups.path_for(None, 1).unwrap();
        let name = untitled.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("untitled-"), "{name}");
        assert_eq!(
            untitled.parent(),
            Some(Path::new("data").join("backups").as_path())
        );
        // Another session's untitled buffer gets its own file.
        let other = Backups {
            session: "untitled-other".into(),
            ..backups.clone()
        };
        assert_ne!(other.path_for(None, 1), Some(untitled.clone()));
        // So does another untitled tab of this session.
        assert_ne!(backups.path_for(None, 2), Some(untitled));
    }

    #[test]
    fn hash_is_stable() {
        // Changing this would orphan every backup written by an older glyph.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn only_a_newer_different_backup_is_recoverable() -> io::Result<()> {
        let data = tempfile::tempdir()?;
        let files = tempfile::tempdir()?;
        let file = files.path().join("a.txt");
        let backups = Backups::new(Some(data.path().to_path_buf()));
        assert_eq!(backups.recoverable(&file), None, "no backup at all");

        fs::write(&file, "on disk\n")?;
        backups
            .job(Some(&file), 0, "backed up\n".into())
            .unwrap()
            .run()?;
        let backup = backups.path_for(Some(&file), 0).unwrap();
        let file_time = fs::metadata(&file)?.modified()?;
        let set_backup_time = |time: SystemTime| {
            fs::File::options()
                .write(true)
                .open(&backup)
                .and_then(|f| f.set_modified(time))
        };

        set_backup_time(file_time + Duration::from_secs(5))?;
        assert_eq!(backups.recoverable(&file).as_deref(), Some("backed up\n"));

        // Older than the file: the file was saved since.
        set_backup_time(file_time - Duration::from_secs(5))?;
        assert_eq!(backups.recoverable(&file), None);

        // The file is gone: the backup is all there is.
        fs::remove_file(&file)?;
        assert_eq!(backups.recoverable(&file).as_deref(), Some("backed up\n"));

        // Newer but the same as the file: nothing to recover, and it is cleaned up.
        fs::write(&file, "backed up\n")?;
        set_backup_time(fs::metadata(&file)?.modified()? + Duration::from_secs(5))?;
        assert_eq!(backups.recoverable(&file), None);
        assert!(!backup.exists());
        Ok(())
    }

    #[test]
    fn delete_removes_the_backup_and_cancels_a_pending_write() -> io::Result<()> {
        let data = tempfile::tempdir()?;
        let backups = Backups::new(Some(data.path().to_path_buf()));
        backups.job(None, 0, "one".into()).unwrap().run()?;
        assert_eq!(backups_in(data.path()).len(), 1);

        let pending = backups.job(None, 0, "two".into()).unwrap();
        backups.delete(None, 0)?;
        assert!(backups_in(data.path()).is_empty());
        pending.run()?;
        assert!(
            backups_in(data.path()).is_empty(),
            "a stale write came back"
        );

        // Deleting what isn't there is fine; a fresh job writes again.
        backups.delete(None, 0)?;
        backups.job(None, 0, "three".into()).unwrap().run()?;
        assert_eq!(backups_in(data.path()).len(), 1);
        Ok(())
    }

    #[test]
    fn unwritable_data_dir_is_an_error_not_a_panic() -> io::Result<()> {
        let parent = tempfile::tempdir()?;
        // A file where the data dir should be: `backups/` can't be created under it.
        let data = parent.path().join("data");
        fs::write(&data, "not a folder")?;
        let backups = Backups::new(Some(data));
        assert!(backups.job(None, 0, "x".into()).unwrap().run().is_err());
        Ok(())
    }

    #[test]
    fn data_dir_env_override_wins_when_non_empty() {
        assert_eq!(
            data_dir(Some("custom".into())),
            Some(PathBuf::from("custom"))
        );
        assert_ne!(data_dir(Some("".into())), Some(PathBuf::from("")));
    }

    #[test]
    fn no_data_dir_turns_backups_off() {
        let backups = Backups::default();
        assert_eq!(backups.path_for(None, 0), None);
        assert!(backups.job(None, 0, "x".into()).is_none());
        assert!(backups.delete(None, 0).is_ok());
    }
}
