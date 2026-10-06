//! Reading one folder's entries the way the tree shows them.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

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
}
