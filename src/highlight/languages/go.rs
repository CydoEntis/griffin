use std::sync::OnceLock;

use super::{Language, Role};

// The grammar's query puts its function patterns ahead of the catch-all
// `(identifier) @variable`, so it's used as is.

pub static GO: Language = Language {
    name: "go",
    extensions: &["go"],
    grammar: || tree_sitter_go::LANGUAGE.into(),
    highlights: &[tree_sitter_go::HIGHLIGHTS_QUERY],
    injections: "",
    embeds: &[],
    roles: &[("escape", Role::Constant)],
    compiled: OnceLock::new(),
};
