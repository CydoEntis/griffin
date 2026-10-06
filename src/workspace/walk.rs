//! Reading the project from disk: one folder's entries the way the tree shows
//! them, and every file under the root for the go-to-file picker.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ignore::{WalkBuilder, WalkState};

/// One file or folder directly inside a listed folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// The entries directly inside `dir`, folders first then files, each alphabetical
/// ignoring case. `.gitignore` rules (the folder's own and its parents') hide
/// entries, as does `.git/`; other dotfiles stay visible. Entries that can't be
/// read are left out rather than failing the whole listing.
pub fn list_dir(dir: &Path) -> Vec<Entry> {
    let mut entries: Vec<Entry> = WalkBuilder::new(dir)
        .max_depth(Some(1))
        // Dotfiles such as `.gitignore` are part of the project; only ignore rules hide.
        .hidden(false)
        // A folder that isn't a git checkout still has its `.gitignore` honoured.
        .require_git(false)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.depth() == 1)
        .filter(|entry| entry.file_name() != ".git")
        .map(|entry| Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            // `Path::is_dir` follows symlinks, so a linked folder still expands.
            is_dir: entry.path().is_dir(),
            path: entry.into_path(),
        })
        .collect();
    entries.sort_by(compare);
    entries
}

fn compare(a: &Entry, b: &Entry) -> Ordering {
    b.is_dir
        .cmp(&a.is_dir)
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.name.cmp(&b.name))
}

/// Every file under `root`, as paths relative to it with `/` between parts, sorted.
/// The same rules as `list_dir` hide entries. The walk runs on several threads, as
/// a large project must list in well under a second.
pub fn list_files(root: &Path) -> Vec<String> {
    let found: Mutex<Vec<String>> = Mutex::new(Vec::new());
    WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build_parallel()
        .run(|| {
            let found = &found;
            let mut batch = Batch {
                files: Vec::new(),
                found,
            };
            Box::new(move |entry| {
                if let Ok(entry) = entry
                    && entry.file_type().is_some_and(|kind| !kind.is_dir())
                    && let Ok(relative) = entry.path().strip_prefix(root)
                {
                    batch.files.push(relative_name(relative));
                }
                WalkState::Continue
            })
        });
    let mut files = found
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    files.sort_unstable();
    files
}

/// One walker thread's finds, handed over when the thread finishes so threads
/// don't fight over the lock once per file.
struct Batch<'a> {
    files: Vec<String>,
    found: &'a Mutex<Vec<String>>,
}

impl Drop for Batch<'_> {
    fn drop(&mut self) {
        // A poisoned lock means another walker thread panicked; its files are lost
        // either way, so keep this thread's.
        let mut found = self
            .found
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        found.append(&mut self.files);
    }
}

fn relative_name(relative: &Path) -> String {
    let parts: Vec<_> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect();
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn folders_come_first_then_files_alphabetically() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        for file in ["b.txt", "A.txt", "c.txt"] {
            fs::write(dir.path().join(file), "")?;
        }
        for folder in ["zeta", "Alpha"] {
            fs::create_dir(dir.path().join(folder))?;
        }
        fs::write(dir.path().join("zeta").join("inner.txt"), "")?;
        let entries = list_dir(dir.path());
        assert_eq!(
            names(&entries),
            ["Alpha", "zeta", "A.txt", "b.txt", "c.txt"]
        );
        assert!(entries[0].is_dir && !entries[2].is_dir);
        assert_eq!(entries[1].path, dir.path().join("zeta"));
        Ok(())
    }

    #[test]
    fn gitignored_entries_and_dot_git_are_hidden() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join(".gitignore"), "*.log\nbuild/\n")?;
        fs::write(dir.path().join("debug.log"), "")?;
        fs::write(dir.path().join("keep.txt"), "")?;
        fs::create_dir(dir.path().join("build"))?;
        fs::create_dir(dir.path().join(".git"))?;
        fs::create_dir(dir.path().join("src"))?;
        fs::write(dir.path().join("src").join("trace.log"), "")?;
        fs::write(dir.path().join("src").join("main.rs"), "")?;

        assert_eq!(
            names(&list_dir(dir.path())),
            ["src", ".gitignore", "keep.txt"]
        );
        // The root's rules still apply one folder down.
        assert_eq!(names(&list_dir(&dir.path().join("src"))), ["main.rs"]);
        Ok(())
    }

    #[test]
    fn list_files_walks_the_whole_tree_relative_to_the_root() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(
            dir.path().join(".gitignore"),
            "*.log
build/
",
        )?;
        fs::write(dir.path().join("debug.log"), "")?;
        fs::create_dir_all(dir.path().join("build"))?;
        fs::write(dir.path().join("build").join("out.txt"), "")?;
        fs::create_dir_all(dir.path().join(".git"))?;
        fs::write(dir.path().join(".git").join("HEAD"), "")?;
        fs::create_dir_all(dir.path().join("src").join("util"))?;
        fs::write(dir.path().join("src").join("main.rs"), "")?;
        fs::write(dir.path().join("src").join("util").join("helpers.rs"), "")?;
        fs::write(dir.path().join("README.md"), "")?;

        assert_eq!(
            list_files(dir.path()),
            [
                ".gitignore",
                "README.md",
                "src/main.rs",
                "src/util/helpers.rs"
            ]
        );
        Ok(())
    }

    /// R18: a 50k-file project lists in under a second. Slow to set up, so run it
    /// with `cargo test --release -- --ignored`.
    #[test]
    #[ignore = "bench: creates 50,000 files"]
    fn list_files_lists_fifty_thousand_files_in_under_a_second() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        for folder in 0..500 {
            let folder = dir.path().join(format!("pkg{folder:03}"));
            fs::create_dir(&folder)?;
            for file in 0..100 {
                fs::write(folder.join(format!("file{file:03}.rs")), "")?;
            }
        }
        let start = std::time::Instant::now();
        let files = list_files(dir.path());
        let took = start.elapsed();
        assert_eq!(files.len(), 50_000);
        assert!(took < std::time::Duration::from_secs(1), "took {took:?}");
        Ok(())
    }
}
