use std::sync::OnceLock;

use super::Language;

/// Its injections query puts JavaScript in `<script>` and CSS in `<style>`; both
/// are registered, so those bodies are coloured in their own grammars.
pub static HTML: Language = Language {
    name: "html",
    extensions: &["html", "htm"],
    grammar: || tree_sitter_html::LANGUAGE.into(),
    highlights: &[tree_sitter_html::HIGHLIGHTS_QUERY],
    injections: tree_sitter_html::INJECTIONS_QUERY,
    embeds: &["css", "javascript"],
    roles: &[],
    line_comment: None,
    compiled: OnceLock::new(),
};
