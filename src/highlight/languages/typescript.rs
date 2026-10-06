use std::sync::OnceLock;

use super::Language;
use super::javascript::{FUNCTIONS, JSX, ROLES};

// The TypeScript query only adds to the JavaScript one, as the grammar's README
// says: its patterns go first so types win, then JavaScript's.

pub static TYPESCRIPT: Language = Language {
    name: "typescript",
    extensions: &["ts", "mts", "cts"],
    grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    highlights: &[
        FUNCTIONS,
        tree_sitter_typescript::HIGHLIGHTS_QUERY,
        tree_sitter_javascript::HIGHLIGHT_QUERY,
    ],
    injections: tree_sitter_javascript::INJECTIONS_QUERY,
    roles: ROLES,
    compiled: OnceLock::new(),
};

pub static TSX: Language = Language {
    name: "tsx",
    extensions: &["tsx"],
    grammar: || tree_sitter_typescript::LANGUAGE_TSX.into(),
    highlights: &[
        JSX,
        FUNCTIONS,
        tree_sitter_typescript::HIGHLIGHTS_QUERY,
        tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        tree_sitter_javascript::HIGHLIGHT_QUERY,
    ],
    injections: tree_sitter_javascript::INJECTIONS_QUERY,
    roles: ROLES,
    compiled: OnceLock::new(),
};
