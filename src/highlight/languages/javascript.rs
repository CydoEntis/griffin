use std::sync::OnceLock;

use super::{Language, Role};

/// Ahead of the grammar's queries, so these win. The JavaScript query opens with
/// the catch-alls `(identifier) @variable` and `(property_identifier) @property`,
/// which would beat its own function patterns further down; here they come first.
/// Shared with TypeScript, whose grammar has the same nodes.
pub(super) const FUNCTIONS: &str = r#"
(function_expression name: (identifier) @function)
(function_declaration name: (identifier) @function)
(generator_function_declaration name: (identifier) @function)
(method_definition name: (property_identifier) @function.method)
(pair
  key: (property_identifier) @function.method
  value: [(function_expression) (arrow_function)])
(variable_declarator
  name: (identifier) @function
  value: [(function_expression) (arrow_function)])
(call_expression function: (identifier) @function)
(call_expression
  function: (member_expression
    property: (property_identifier) @function.method))
"#;

/// The grammar's JSX query tags only lowercase (DOM) element names, and a
/// capitalised name would otherwise read as a type; a component is a tag too.
/// Shared with TSX.
pub(super) const JSX: &str = r#"
(jsx_opening_element name: (identifier) @tag)
(jsx_closing_element name: (identifier) @tag)
(jsx_self_closing_element name: (identifier) @tag)
(jsx_attribute (property_identifier) @attribute)
"#;

/// Capture names the JavaScript-family queries use that aren't role names.
pub(super) const ROLES: &[(&str, Role)] = &[("constructor", Role::Type)];

pub static JAVASCRIPT: Language = Language {
    name: "javascript",
    extensions: &["js", "mjs", "cjs", "jsx"],
    grammar: || tree_sitter_javascript::LANGUAGE.into(),
    highlights: &[
        JSX,
        FUNCTIONS,
        tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        tree_sitter_javascript::HIGHLIGHT_QUERY,
    ],
    injections: tree_sitter_javascript::INJECTIONS_QUERY,
    embeds: &[],
    roles: ROLES,
    line_comment: Some("//"),
    wrap_comment: None,
    compiled: OnceLock::new(),
};
