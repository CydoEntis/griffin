use std::sync::OnceLock;

use super::Language;

// The grammar's query captures class and id selectors as `@property`, like
// property names, and tag selectors as `@tag`; those are the roles they get.

pub static CSS: Language = Language {
    name: "css",
    extensions: &["css"],
    grammar: || tree_sitter_css::LANGUAGE.into(),
    highlights: &[tree_sitter_css::HIGHLIGHTS_QUERY],
    injections: "",
    embeds: &[],
    roles: &[],
    line_comment: None,
    compiled: OnceLock::new(),
};
