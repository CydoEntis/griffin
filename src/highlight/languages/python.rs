use std::sync::OnceLock;

use super::{Language, Role};

/// Ahead of the grammar's query, so these win. It opens with the catch-all
/// `(identifier) @variable`, which would beat its own function, type and
/// constant patterns further down on the same identifier. A decorator is drawn
/// as an attribute, names and all: the grammar's `(decorator) @function` would
/// lose its inner names to `@variable` and `@property`.
const OWN: &str = r#"
(decorator) @attribute
(decorator (identifier) @attribute)
(decorator (attribute (identifier) @attribute))
(decorator (call function: (identifier) @attribute))
(decorator (call function: (attribute (identifier) @attribute)))
(function_definition name: (identifier) @function)
(class_definition name: (identifier) @type)
(call function: (identifier) @function)
(call function: (attribute attribute: (identifier) @function.method))
(type (identifier) @type)
((identifier) @constant
 (#match? @constant "^[A-Z][A-Z_0-9]*$"))
((identifier) @constructor
 (#match? @constructor "^[A-Z]"))
"#;

pub static PYTHON: Language = Language {
    name: "python",
    extensions: &["py", "pyi"],
    grammar: || tree_sitter_python::LANGUAGE.into(),
    highlights: &[OWN, tree_sitter_python::HIGHLIGHTS_QUERY],
    injections: "",
    embeds: &[],
    roles: &[("constructor", Role::Type), ("escape", Role::Constant)],
    line_comment: Some("#"),
    wrap_comment: None,
    compiled: OnceLock::new(),
};
