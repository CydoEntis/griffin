use std::sync::OnceLock;

use super::{Language, Role};

/// Ahead of the grammar's query, so these win. The grammar marks numbers
/// `@constant.builtin`, like `true`; here they get their own role. Its
/// `(field_identifier) @property` comes before its method-call pattern, so
/// `.sqrt()` would read as a field; called fields are functions here. A quote
/// typed with no closing partner is an error token in the grammar; marked
/// `.unclosed` it colours the rest of its line as a string.
const OWN: &str = r#"
(integer_literal) @number
(float_literal) @number
(call_expression
  function: (field_expression
    field: (field_identifier) @function.method))
(generic_function
  function: (field_expression
    field: (field_identifier) @function.method))
(ERROR "\"" @string.unclosed)
"#;

pub static RUST: Language = Language {
    name: "rust",
    extensions: &["rs"],
    grammar: || tree_sitter_rust::LANGUAGE.into(),
    highlights: &[OWN, tree_sitter_rust::HIGHLIGHTS_QUERY],
    injections: tree_sitter_rust::INJECTIONS_QUERY,
    roles: &[
        ("constructor", Role::Type),
        ("label", Role::Constant),
        ("escape", Role::Constant),
    ],
    compiled: OnceLock::new(),
};
