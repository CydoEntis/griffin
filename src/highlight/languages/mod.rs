//! The language registry: which grammar and queries highlight a file, by its
//! extension. A new language is one file here plus one line in `LANGUAGES`.
//!
//! Versions known to load together (checked when Rust was added, #22):
//! `tree-sitter = "0.27"` with `tree-sitter-rust = "0.24"` (0.24.2 ships grammar
//! ABI 15, which 0.27 loads). Query iteration needs `streaming-iterator = "0.1"`.
//! Grammars are compiled in (ADR-0001); none load at runtime.

mod rust;

use std::path::Path;
use std::sync::OnceLock;

use tree_sitter::Query;

use super::Role;

/// Everything the engine needs to highlight one language.
pub struct Language {
    pub name: &'static str,
    /// File extensions, without the dot, compared case-insensitively.
    pub extensions: &'static [&'static str],
    pub grammar: fn() -> tree_sitter::Language,
    /// Highlight query sources, joined in order. On one node the earliest pattern
    /// wins, so a language puts its own patterns before the grammar's to take
    /// precedence.
    pub highlights: &'static [&'static str],
    /// Kept with the language so embedded-language support can use it; the Rust
    /// grammar's only injections are macro bodies, which stay uncoloured for now.
    #[allow(dead_code)] // read by the query test until injections are drawn
    pub injections: &'static str,
    /// A capture whose name ends in `.unclosed` runs to the end of its line: the
    /// grammar leaves a quote with no closing partner as an error token, and this
    /// is how the rest of the line still reads as a string while it's typed.
    ///
    /// Capture names this language maps to a role differently from the default
    /// (the capture's first dotted segment read as a role name).
    pub roles: &'static [(&'static str, Role)],
    pub(super) compiled: OnceLock<Option<Compiled>>,
}

/// A language's query, compiled once, with each capture's role by capture index.
pub(super) struct Compiled {
    pub query: Query,
    pub roles: Vec<Option<Role>>,
    /// Which captures are `.unclosed`, by capture index.
    pub unclosed: Vec<bool>,
}

/// Every highlighted language.
static LANGUAGES: &[&Language] = &[&rust::RUST];

/// The language for `path`'s extension, if Griffin highlights it.
pub fn for_path(path: &Path) -> Option<&'static Language> {
    let ext = path.extension()?.to_str()?;
    LANGUAGES
        .iter()
        .copied()
        .find(|lang| lang.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}

impl Language {
    /// The compiled query and role table, built on first use. `None` if the query
    /// doesn't compile against the grammar, which the unit tests rule out for
    /// every registered language; highlighting is then just skipped.
    pub(super) fn compiled(&self) -> Option<&Compiled> {
        self.compiled
            .get_or_init(|| {
                let source = self.highlights.concat();
                let query = Query::new(&(self.grammar)(), &source).ok()?;
                let roles = query
                    .capture_names()
                    .iter()
                    .map(|name| self.role_for(name))
                    .collect();
                let unclosed = query
                    .capture_names()
                    .iter()
                    .map(|name| name.ends_with(".unclosed"))
                    .collect();
                Some(Compiled {
                    query,
                    roles,
                    unclosed,
                })
            })
            .as_ref()
    }

    /// The role for capture `name`: this language's table first, then the name
    /// itself, then its first dotted segment (`function.method` is a function).
    pub fn role_for(&self, name: &str) -> Option<Role> {
        let listed = |n: &str| {
            self.roles
                .iter()
                .find(|(capture, _)| *capture == n)
                .map(|&(_, role)| role)
        };
        listed(name).or_else(|| Role::from_name(name)).or_else(|| {
            let first = name.split('.').next()?;
            listed(first).or_else(|| Role::from_name(first))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rs_extension_resolves() {
        let lang = for_path(Path::new("src/main.rs")).expect("rust is registered");
        assert_eq!(lang.name, "rust");
        assert!(for_path(Path::new("LIB.RS")).is_some());
        assert!(for_path(Path::new("notes.txt")).is_none());
        assert!(for_path(Path::new("Makefile")).is_none());
    }

    #[test]
    fn every_language_query_compiles() {
        for lang in LANGUAGES {
            let compiled = lang.compiled();
            assert!(compiled.is_some(), "{} query fails to compile", lang.name);
            if let Err(err) = Query::new(&(lang.grammar)(), lang.injections) {
                panic!("{} injections query: {err}", lang.name);
            }
        }
    }

    #[test]
    fn dotted_captures_fall_back_to_their_first_segment() {
        let lang = &rust::RUST;
        assert_eq!(lang.role_for("function.method"), Some(Role::Function));
        assert_eq!(
            lang.role_for("punctuation.bracket"),
            Some(Role::Punctuation)
        );
        assert_eq!(lang.role_for("constant.builtin"), Some(Role::Constant));
        assert_eq!(lang.role_for("constructor"), Some(Role::Type));
        assert_eq!(lang.role_for("nonsense"), None);
    }
}
