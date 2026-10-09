//! Creating, renaming and trashing files and folders from the tree. Each operation
//! checks the name and refuses to touch an existing entry before calling the OS, so
//! a failure leaves the disk as it was.

use std::fmt::Debug;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// Moving to the OS trash, behind a trait so tests never fill the real one.
pub trait Trash: Debug {
    fn delete(&mut self, path: &Path) -> Result<()>;
}

/// The real trash, through the `trash` crate.
#[derive(Debug, Default)]
pub struct OsTrash;

impl Trash for OsTrash {
    fn delete(&mut self, path: &Path) -> Result<()> {
        Ok(trash::delete(path)?)
    }
}

impl Default for Box<dyn Trash> {
    fn default() -> Self {
        Box::new(OsTrash)
    }
}

/// Characters no file name may hold on at least one of the OSes Tome runs on;
/// refusing them everywhere keeps a project portable.
const FORBIDDEN: &[char] = &['/', '\\', '<', '>', ':', '"', '|', '?', '*'];

/// Why `name` can't name a file or folder, if it can't.
pub fn invalid_name(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("empty name")
    } else if name == "." || name == ".." {
        Some("reserved name")
    } else if name.contains(FORBIDDEN) || name.chars().any(char::is_control) {
        Some("name holds a character file names can't")
    } else if name.ends_with(['.', ' ']) || name.starts_with(' ') {
        // Windows silently drops a trailing dot or space, so the file made would
        // not be the one named.
        Some("name starts or ends with a space or ends with a dot")
    } else {
        None
    }
}

fn checked_target(dir: &Path, name: &str) -> Result<PathBuf> {
    if let Some(why) = invalid_name(name) {
        bail!("invalid name {name:?}: {why}");
    }
    let target = dir.join(name);
    if fs::symlink_metadata(&target).is_ok() {
        bail!("{name} already exists");
    }
    Ok(target)
}

/// Creates the empty file `name` in `dir` and returns its path.
pub fn create_file(dir: &Path, name: &str) -> Result<PathBuf> {
    let target = checked_target(dir, name)?;
    // `create_new` also refuses a file that appeared since the check.
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|err| os_error("create", name, err))?;
    Ok(target)
}

/// Creates the folder `name` in `dir` and returns its path.
pub fn create_folder(dir: &Path, name: &str) -> Result<PathBuf> {
    let target = checked_target(dir, name)?;
    fs::create_dir(&target).map_err(|err| os_error("create", name, err))?;
    Ok(target)
}

/// Renames `path` to `name` in the same folder and returns the new path.
pub fn rename(path: &Path, name: &str) -> Result<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let old = path.file_name().map(|n| n.to_string_lossy().into_owned());
    if old.as_deref() == Some(name) {
        return Ok(path.to_path_buf());
    }
    // On case-insensitive file systems `Notes.txt` "exists" while renaming
    // `notes.txt` to it; that's the same entry, not a clash.
    let case_only = cfg!(any(windows, target_os = "macos"))
        && old
            .as_deref()
            .is_some_and(|old| old.eq_ignore_ascii_case(name));
    let target = if case_only {
        match invalid_name(name) {
            Some(why) => bail!("invalid name {name:?}: {why}"),
            None => dir.join(name),
        }
    } else {
        // `fs::rename` replaces an existing file on Unix, so this check is what
        // keeps a rename from destroying one.
        checked_target(dir, name)?
    };
    fs::rename(path, &target).map_err(|err| os_error("rename to", name, err))?;
    Ok(target)
}

fn os_error(verb: &str, name: &str, err: io::Error) -> anyhow::Error {
    anyhow::anyhow!("cannot {verb} {name}: {err}")
}

/// `path` with its `from` prefix swapped for `to`, when it lies at or under `from`:
/// where an open file ends up after renaming it or a folder holding it.
pub fn rebase(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    path.strip_prefix(from).ok().map(|rest| {
        if rest.as_os_str().is_empty() {
            to.to_path_buf()
        } else {
            to.join(rest)
        }
    })
}

/// A trash for tests: records each path and removes it from disk, as the real one
/// would, or fails every call when `fail` is set.
#[cfg(test)]
#[derive(Debug, Default, Clone)]
pub struct FakeTrash {
    pub trashed: std::rc::Rc<std::cell::RefCell<Vec<PathBuf>>>,
    pub fail: bool,
}

#[cfg(test)]
impl Trash for FakeTrash {
    fn delete(&mut self, path: &Path) -> Result<()> {
        if self.fail {
            bail!("trash unavailable");
        }
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
        self.trashed.borrow_mut().push(path.to_path_buf());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_files_and_folders_but_never_over_an_existing_name() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = create_file(dir.path(), "notes.md")?;
        assert_eq!(file, dir.path().join("notes.md"));
        assert_eq!(fs::read(&file)?, b"");
        fs::write(&file, "keep")?;

        let err = create_file(dir.path(), "notes.md").unwrap_err();
        assert_eq!(err.to_string(), "notes.md already exists");
        let err = create_folder(dir.path(), "notes.md").unwrap_err();
        assert_eq!(err.to_string(), "notes.md already exists");
        assert_eq!(fs::read_to_string(&file)?, "keep");

        let folder = create_folder(dir.path(), "lib")?;
        assert!(folder.is_dir());
        Ok(())
    }

    #[test]
    fn invalid_names_are_refused_before_touching_the_disk() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for bad in [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "what?",
            "tab\there",
            "dot.",
            " lead",
        ] {
            let err = create_file(dir.path(), bad).unwrap_err();
            assert!(
                err.to_string().starts_with("invalid name"),
                "{bad:?}: {err}"
            );
        }
        assert_eq!(fs::read_dir(dir.path())?.count(), 0);
        assert_eq!(invalid_name(".gitignore"), None);
        assert_eq!(invalid_name("my notes.txt"), None);
        Ok(())
    }

    #[test]
    fn rename_moves_the_entry_and_refuses_to_replace_another() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        fs::write(&a, "a")?;
        fs::write(&b, "b")?;

        let err = rename(&a, "b.txt").unwrap_err();
        assert_eq!(err.to_string(), "b.txt already exists");
        assert_eq!(fs::read_to_string(&b)?, "b");
        assert!(a.exists());

        let c = rename(&a, "c.txt")?;
        assert_eq!(c, dir.path().join("c.txt"));
        assert_eq!(fs::read_to_string(&c)?, "a");
        assert!(!a.exists());
        // The same name is a no-op, not a clash with itself.
        assert_eq!(rename(&c, "c.txt")?, c);
        assert!(rename(&c, "x/y").is_err());
        Ok(())
    }

    #[test]
    fn rebase_follows_a_renamed_file_or_folder() {
        let from = Path::new("p/src");
        let to = Path::new("p/lib");
        assert_eq!(
            rebase(Path::new("p/src/main.rs"), from, to),
            Some(PathBuf::from("p/lib/main.rs"))
        );
        assert_eq!(rebase(from, from, to), Some(to.to_path_buf()));
        assert_eq!(rebase(Path::new("p/srcx/a.rs"), from, to), None);
    }
}
