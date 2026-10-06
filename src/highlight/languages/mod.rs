//! The language registry: which grammar and queries highlight a file, by its
//! extension. A new language is one file here plus one line in `LANGUAGES`.
//!
//! Versions known to load together (checked when Rust was added, #22):
//! `tree-sitter = "0.27"`, `tree-sitter-highlight = "0.27"` and
//! `tree-sitter-rust = "0.24"` (0.24.2 ships grammar ABI 15, which 0.27 loads).
//! Query iteration needs `streaming-iterator = "0.1"`. Added with TypeScript and
//! JavaScript (#23): `tree-sitter-typescript = "0.23"` and
//! `tree-sitter-javascript = "0.25"`. Grammars are compiled in
//! (ADR-0001); none load at runtime.
//!
//! Each language's queries are built into a `tree_sitter_highlight`
//! `HighlightConfiguration`, which lays out the injections, locals and highlights
//! queries as one query the way tree-sitter's highlighter expects.

mod javascript;
mod rust;
mod typescript;

use std::path::Path;
use std::sync::OnceLock;

use tree_sitter::Query;
use tree_sitter_highlight::HighlightConfiguration;

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
    /// Built into the language's configuration so embedded-language support can
    /// use it; the Rust grammar's only injections are macro bodies, which stay
    /// uncoloured for now.
    pub injections: &'static str,
    /// Capture names this language maps to a role differently from the default
    /// (the capture's first dotted segment read as a role name).
    pub roles: &'static [(&'static str, Role)],
    pub(super) compiled: OnceLock<Option<Compiled>>,
}

/// A language's queries, compiled once, with each capture's role by capture index.
pub(super) struct Compiled {
    /// The injections and highlights queries joined into one, in `config.query`.
    pub config: HighlightConfiguration,
    /// Patterns before this index come from the injections query and colour
    /// nothing themselves.
    pub highlights_start: usize,
    pub roles: Vec<Option<Role>>,
    /// Which captures are `.unclosed`, by capture index. Such a capture runs to the
    /// end of its line: the grammar leaves a quote with no closing partner as an
    /// error token, and this is how the rest of the line still reads as a string
    /// while it's typed.
    pub unclosed: Vec<bool>,
}

impl Compiled {
    pub fn query(&self) -> &Query {
        &self.config.query
    }
}

/// Every highlighted language.
static LANGUAGES: &[&Language] = &[
    &rust::RUST,
    &typescript::TYPESCRIPT,
    &typescript::TSX,
    &javascript::JAVASCRIPT,
];

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
                let config = HighlightConfiguration::new(
                    (self.grammar)(),
                    self.name,
                    &self.highlights.concat(),
                    self.injections,
                    "",
                )
                .ok()?;
                let query = &config.query;
                // The injections query comes first in the joined source.
                let highlights_start = (0..query.pattern_count())
                    .take_while(|&i| query.start_byte_for_pattern(i) < self.injections.len())
                    .count();
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
                    config,
                    highlights_start,
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
    fn ts_js_extensions_resolve() {
        let name = |file: &str| for_path(Path::new(file)).map(|lang| lang.name);
        for file in ["a.ts", "a.mts", "a.cts", "A.TS"] {
            assert_eq!(name(file), Some("typescript"), "{file}");
        }
        assert_eq!(name("App.tsx"), Some("tsx"));
        for file in ["a.js", "a.mjs", "a.cjs", "App.jsx"] {
            assert_eq!(name(file), Some("javascript"), "{file}");
        }
        assert_eq!(name("package.json"), None);
        assert_eq!(name("App.vue"), None);
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
