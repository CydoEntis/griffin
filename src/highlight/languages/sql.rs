use std::sync::OnceLock;

use super::{Language, Role};

/// Ahead of the grammar's query, so these win. It captures every `(literal)` as
/// a string before its number patterns, and those patterns use Lua's `%d`, which
/// tree-sitter's regex engine reads as a literal `d`; so a numeric literal is
/// matched here instead.
const OWN: &str = r#"
((literal) @number
 (#match? @number "^[-+]?([0-9]+(\.[0-9]*)?|\.[0-9]+)([eE][-+]?[0-9]+)?$"))
"#;

pub static SQL: Language = Language {
    name: "sql",
    extensions: &["sql"],
    grammar: || tree_sitter_sequel::LANGUAGE.into(),
    highlights: &[OWN, tree_sitter_sequel::HIGHLIGHTS_QUERY],
    injections: "",
    embeds: &[],
    roles: &[
        ("field", Role::Property),
        ("float", Role::Number),
        ("boolean", Role::Constant),
        ("conditional", Role::Keyword),
        ("storageclass", Role::Keyword),
        ("parameter", Role::Variable),
    ],
    compiled: OnceLock::new(),
};
