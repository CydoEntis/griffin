//! The project: its root folder, walking it, and the tree model the sidebar shows.

pub mod tree;
pub mod walk;

use std::path::{Path, PathBuf};

pub use tree::Tree;

/// What the command line asked to open: the project root, and the file to open in
/// the editor, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub root: PathBuf,
    pub file: Option<PathBuf>,
    /// A folder was named, so the tree starts open.
    pub show_tree: bool,
}

impl Launch {
    /// A folder becomes the root with the tree open; a file opens in the editor with
    /// its folder as the root and the tree closed, as before trees existed; nothing
    /// at all means the current folder.
    pub fn from_arg(path: Option<&Path>) -> Self {
        match path {
            Some(dir) if dir.is_dir() => Launch {
                root: dir.to_path_buf(),
                file: None,
                show_tree: true,
            },
            Some(file) => {
                let root = match file.parent() {
                    Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
                    _ => PathBuf::from("."),
                };
                Launch {
                    root,
                    file: Some(file.to_path_buf()),
                    show_tree: false,
                }
            }
            None => Launch {
                root: PathBuf::from("."),
                file: None,
                show_tree: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_the_root_with_the_tree_open() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let launch = Launch::from_arg(Some(dir.path()));
        assert_eq!(launch.root, dir.path());
        assert_eq!(launch.file, None);
        assert!(launch.show_tree);
        Ok(())
    }

    #[test]
    fn a_file_opens_with_its_folder_as_root() {
        let launch = Launch::from_arg(Some(Path::new("some/dir/notes.txt")));
        assert_eq!(launch.root, Path::new("some/dir"));
        assert_eq!(
            launch.file.as_deref(),
            Some(Path::new("some/dir/notes.txt"))
        );
        assert!(!launch.show_tree);

        let bare = Launch::from_arg(Some(Path::new("notes.txt")));
        assert_eq!(bare.root, Path::new("."));
    }

    #[test]
    fn nothing_means_the_current_folder() {
        let launch = Launch::from_arg(None);
        assert_eq!(launch.root, Path::new("."));
        assert_eq!(launch.file, None);
        assert!(!launch.show_tree);
    }
}
