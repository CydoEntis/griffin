//! Atomic saves: the new contents go to a temp file in the target's folder, which is
//! then renamed over the target, so a crash or a failed write never leaves a
//! half-written file behind.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `text` to `path` atomically. A file that doesn't exist yet is created.
pub fn save_atomic(path: &Path, text: &str) -> io::Result<()> {
    save_with(path, text.as_bytes(), |file, bytes| file.write_all(bytes))
}

/// `save_atomic` with the write step injected, so tests can make it fail.
fn save_with(
    path: &Path,
    bytes: &[u8],
    write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let (temp_path, mut file) = create_temp(path)?;
    let result = (|| {
        copy_permissions(path, &file)?;
        write(&mut file, bytes)?;
        file.flush()?;
        file.sync_all()?;
        // Windows can't rename a file that is still open.
        drop(file);
        fs::rename(&temp_path, path)
    })();
    if result.is_err() {
        // Best effort: the original is untouched either way, and the write error is
        // the one worth reporting.
        let _ = fs::remove_file(&temp_path);
    }
    result?;
    sync_dir(path);
    Ok(())
}

/// Creates a new temp file next to `path`. Same folder means same filesystem, so
/// the rename is atomic.
fn create_temp(path: &Path) -> io::Result<(PathBuf, File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?
        .to_string_lossy();
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!(".{name}.tome-{}-{n}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => return Ok((temp, file)),
            // Left over from a crashed run with the same pid; try the next name.
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
}

/// The renamed file would otherwise get default permissions, e.g. losing a
/// script's execute bit.
#[cfg(unix)]
fn copy_permissions(target: &Path, temp: &File) -> io::Result<()> {
    match fs::metadata(target) {
        Ok(meta) => temp.set_permissions(meta.permissions()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(not(unix))]
fn copy_permissions(_target: &Path, _temp: &File) -> io::Result<()> {
    Ok(())
}

/// Makes the rename itself durable. Best effort: the data is already synced and
/// in place, so a failure here isn't worth failing the save over.
#[cfg(unix)]
fn sync_dir(path: &Path) {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn replaces_an_existing_file_and_leaves_no_temp() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("a.txt");
        fs::write(&path, "old contents\n")?;
        save_atomic(&path, "new\n")?;
        assert_eq!(fs::read_to_string(&path)?, "new\n");
        assert_eq!(entries(dir.path()), ["a.txt"]);
        Ok(())
    }

    #[test]
    fn creates_a_missing_file() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("new.txt");
        save_atomic(&path, "hi")?;
        assert_eq!(fs::read_to_string(&path)?, "hi");
        assert_eq!(entries(dir.path()), ["new.txt"]);
        Ok(())
    }

    #[test]
    fn writes_the_temp_file_in_the_target_folder() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("a.txt");
        fs::write(&path, "old")?;
        let mut seen = Vec::new();
        save_with(&path, b"new", |file, bytes| {
            seen = entries(dir.path());
            file.write_all(bytes)
        })?;
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert!(seen[0].starts_with(".a.txt.tome-"), "{seen:?}");
        assert!(seen[0].ends_with(".tmp"), "{seen:?}");
        assert_eq!(fs::read_to_string(&path)?, "new");
        Ok(())
    }

    #[test]
    fn failed_write_leaves_the_original_and_no_temp() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("a.txt");
        let original = b"line one\r\nline two\r\n\xe2\x9c\x93";
        fs::write(&path, original)?;
        let result = save_with(&path, b"replacement", |file, bytes| {
            // Part of the data lands before the failure, as with a full disk.
            file.write_all(&bytes[..4])?;
            Err(io::Error::other("disk full"))
        });
        assert_eq!(result.unwrap_err().to_string(), "disk full");
        assert_eq!(fs::read(&path)?, original);
        assert_eq!(entries(dir.path()), ["a.txt"]);
        Ok(())
    }

    #[test]
    fn missing_folder_is_an_error() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("nope").join("a.txt");
        assert!(save_atomic(&path, "x").is_err());
        assert!(entries(dir.path()).is_empty());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn keeps_the_original_permissions() -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("run.sh");
        fs::write(&path, "echo hi\n")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751))?;
        save_atomic(&path, "echo bye\n")?;
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o751);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn permissions_are_on_the_temp_file_before_the_rename() -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("run.sh");
        fs::write(&path, "x")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        let mut mode = 0;
        save_with(&path, b"y", |file, bytes| {
            mode = file.metadata()?.permissions().mode() & 0o777;
            file.write_all(bytes)
        })?;
        assert_eq!(mode, 0o700);
        Ok(())
    }
}
