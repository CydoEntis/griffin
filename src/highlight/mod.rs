//! Syntax highlighting with tree-sitter. Each highlighted buffer owns a
//! `Highlighter`: a parser and the buffer's current tree. Edits are fed in as
//! `InputEdit`s so the next parse reuses the old tree, and the renderer asks only
//! for the roles in the bytes it draws.
//!
//! Queries are compiled by `tree-sitter-highlight` (see `languages`), but run
//! here on the buffer's own tree: its `Highlighter` parses from scratch on every
//! call and can't take an edited tree, which incremental reparsing needs.
//!
//! Embedded languages (CSS in `<style>`, JavaScript in `<script>`) are found
//! with the host language's injections query after each parse. Each region is
//! parsed on its own in its language's grammar, limited to the region's bytes,
//! and its roles paint over the host's.
//!
//! Depends on nothing else in the crate (only `ropey` and the tree-sitter crates),
//! so the highlight tests can compile it on its own.

pub mod languages;

use std::fmt;
use std::ops::Range;
use std::path::Path;

use ropey::Rope;
use streaming_iterator::StreamingIterator;
use tree_sitter::{InputEdit, Node, Parser, Point, QueryCursor, Tree};

pub use languages::Language;

/// A theme syntax role a span of text is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Keyword,
    String,
    Comment,
    Function,
    Type,
    Number,
    Constant,
    Operator,
    Punctuation,
    Variable,
    Property,
    Tag,
    Attribute,
}

impl Role {
    /// The role named like the theme's syntax override key.
    pub fn from_name(name: &str) -> Option<Role> {
        Some(match name {
            "keyword" => Role::Keyword,
            "string" => Role::String,
            "comment" => Role::Comment,
            "function" => Role::Function,
            "type" => Role::Type,
            "number" => Role::Number,
            "constant" => Role::Constant,
            "operator" => Role::Operator,
            "punctuation" => Role::Punctuation,
            "variable" => Role::Variable,
            "property" => Role::Property,
            "tag" => Role::Tag,
            "attribute" => Role::Attribute,
            _ => return None,
        })
    }
}

/// The parser and tree for one buffer's text.
pub struct Highlighter {
    language: &'static Language,
    parser: Parser,
    tree: Option<Tree>,
    /// Edits have reached the tree since the last parse.
    stale: bool,
    /// Parses embedded regions; shared by every injected language.
    inner: Parser,
    /// The embedded regions found by the last parse.
    injections: Vec<Injection>,
}

/// One embedded region: its language and its own tree, whose nodes carry byte
/// offsets into the whole buffer.
#[derive(Clone)]
struct Injection {
    language: &'static Language,
    tree: Tree,
    injections: Vec<Injection>,
}

/// How deep embedded regions may nest. HTML's embedded languages embed nothing,
/// so this only guards against a future registry entry that loops.
const MAX_DEPTH: usize = 4;

impl fmt::Debug for Highlighter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Highlighter")
            .field("language", &self.language.name)
            .field("stale", &self.stale)
            .finish_non_exhaustive()
    }
}

impl Clone for Highlighter {
    /// Parsers can't be cloned; the copy gets its own. Trees share their nodes, so
    /// this is cheap enough to do for a split's copy of a buffer each frame.
    fn clone(&self) -> Self {
        let mut copy = Self::new(self.language);
        copy.tree = self.tree.clone();
        copy.stale = self.stale;
        copy.injections = self.injections.clone();
        copy
    }
}

impl Highlighter {
    /// A highlighter for `path`, if its language is registered. Nothing is parsed
    /// until `parse`.
    pub fn for_path(path: &Path) -> Option<Self> {
        let language = languages::for_path(path)?;
        language.compiled()?;
        Some(Self::new(language))
    }

    fn new(language: &'static Language) -> Self {
        let mut parser = Parser::new();
        // A grammar that fails to load leaves the parser without a language, and
        // `parse` then returns no tree: the file just shows uncoloured.
        let _ = parser.set_language(&(language.grammar)());
        Self {
            language,
            parser,
            tree: None,
            stale: true,
            inner: Parser::new(),
            injections: Vec::new(),
        }
    }

    /// Tells the tree about an edit, so the next `parse` can reuse it.
    pub fn edit(&mut self, edit: &InputEdit) {
        if let Some(tree) = &mut self.tree {
            tree.edit(edit);
        }
        self.stale = true;
    }

    /// Brings the tree up to date with `rope`, reusing the edited old tree.
    /// Does nothing when no edit came in since the last parse.
    pub fn parse(&mut self, rope: &Rope) {
        if !self.stale {
            return;
        }
        self.tree = self
            .parser
            .parse_with_options(&mut read_rope(rope), self.tree.as_ref(), None);
        self.stale = false;
        // Embedded regions are small next to their file, so they're parsed afresh
        // instead of being kept in step with each edit.
        self.injections = match &self.tree {
            Some(tree) => find_injections(&mut self.inner, self.language, tree, rope, 0),
            None => Vec::new(),
        };
    }

    /// The roles in `bytes` of `rope`, as sorted, non-overlapping byte ranges.
    /// Uncoloured text has no span. Where captures nest, the innermost wins; where
    /// several patterns capture one node, the earliest in the query wins, as
    /// tree-sitter's own highlighter does.
    pub fn spans(&self, rope: &Rope, bytes: Range<usize>) -> Vec<(Range<usize>, Role)> {
        let bytes = bytes.start.min(rope.len_bytes())..bytes.end.min(rope.len_bytes());
        let (Some(tree), Some(compiled)) = (&self.tree, self.language.compiled()) else {
            return Vec::new();
        };
        if bytes.is_empty() {
            return Vec::new();
        }

        let mut found = Vec::new();
        collect(compiled, tree, rope, &bytes, 0, &mut found);
        collect_injections(&self.injections, rope, &bytes, 1, &mut found);
        // Host roles first, then each embedded layer over them. Within a layer,
        // outer ranges before the ones nested in them, so inner ones paint last;
        // an unclosed string paints over everything it runs into.
        found.sort_by_key(|p| {
            (
                p.unclosed,
                p.layer,
                p.range.start,
                std::cmp::Reverse(p.range.end),
                p.pattern,
            )
        });

        let mut painted: Vec<Option<Role>> = vec![None; bytes.len()];
        let mut last: Option<(usize, Range<usize>)> = None;
        for Paint {
            layer, range, role, ..
        } in found
        {
            if last.as_ref() == Some(&(layer, range.clone())) {
                continue;
            }
            let from = range.start.max(bytes.start) - bytes.start;
            let to = range.end.min(bytes.end).saturating_sub(bytes.start);
            if from < to {
                painted[from..to].fill(Some(role));
            }
            last = Some((layer, range));
        }

        let mut spans: Vec<(Range<usize>, Role)> = Vec::new();
        for (offset, role) in painted.into_iter().enumerate() {
            let Some(role) = role else { continue };
            let at = bytes.start + offset;
            match spans.last_mut() {
                Some((range, prev)) if range.end == at && *prev == role => range.end = at + 1,
                _ => spans.push((at..at + 1, role)),
            }
        }
        spans
    }
}

/// A role found for a range, before overlaps are resolved.
struct Paint {
    unclosed: bool,
    /// 0 for the file's own language, one more for each level of embedding.
    layer: usize,
    range: Range<usize>,
    pattern: usize,
    role: Role,
}

/// The bytes of `node` in `rope`, as the query cursor reads them.
fn node_text<'r>(rope: &'r Rope) -> impl FnMut(Node) -> std::vec::IntoIter<&'r [u8]> + 'r {
    move |node: Node| {
        rope.get_byte_slice(node.byte_range())
            .into_iter()
            .flat_map(|slice| slice.chunks())
            .map(str::as_bytes)
            .collect::<Vec<_>>()
            .into_iter()
    }
}

/// The source the parser reads `rope` through.
fn read_rope<'r>(rope: &'r Rope) -> impl FnMut(usize, Point) -> &'r [u8] + 'r {
    move |byte: usize, _: Point| -> &'r [u8] {
        if byte >= rope.len_bytes() {
            return &[];
        }
        let (chunk, start, _, _) = rope.chunk_at_byte(byte);
        &chunk.as_bytes()[byte - start..]
    }
}

/// The roles a highlights query gives nodes of `tree` in `bytes`.
fn collect(
    compiled: &languages::Compiled,
    tree: &Tree,
    rope: &Rope,
    bytes: &Range<usize>,
    layer: usize,
    found: &mut Vec<Paint>,
) {
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(bytes.clone());
    let mut captures = cursor.captures(compiled.query(), tree.root_node(), node_text(rope));
    while let Some((m, index)) = captures.next() {
        if m.pattern_index < compiled.highlights_start {
            continue;
        }
        let capture = m.captures()[*index];
        let i = capture.index as usize;
        let Some(Some(role)) = compiled.roles.get(i) else {
            continue;
        };
        let mut range = capture.node.byte_range();
        let unclosed = compiled.unclosed.get(i).copied().unwrap_or(false);
        if unclosed {
            range.end = line_end(rope, range.start);
        }
        found.push(Paint {
            unclosed,
            layer,
            range,
            pattern: m.pattern_index,
            role: *role,
        });
    }
}

/// The roles of every embedded region that reaches into `bytes`, and of the
/// regions embedded in those.
fn collect_injections(
    injections: &[Injection],
    rope: &Rope,
    bytes: &Range<usize>,
    layer: usize,
    found: &mut Vec<Paint>,
) {
    for injection in injections {
        let root = injection.tree.root_node().byte_range();
        if root.end <= bytes.start || root.start >= bytes.end {
            continue;
        }
        let Some(compiled) = injection.language.compiled() else {
            continue;
        };
        collect(compiled, &injection.tree, rope, bytes, layer, found);
        collect_injections(&injection.injections, rope, bytes, layer + 1, found);
    }
}

/// The regions of `tree` that `language`'s injections query marks as a language
/// `language` embeds, each parsed in that language's grammar.
fn find_injections(
    parser: &mut Parser,
    language: &Language,
    tree: &Tree,
    rope: &Rope,
    depth: usize,
) -> Vec<Injection> {
    if language.embeds.is_empty() || depth >= MAX_DEPTH {
        return Vec::new();
    }
    let Some(compiled) = language.compiled() else {
        return Vec::new();
    };
    let query = compiled.query();
    let content = query.capture_index_for_name("injection.content");
    let named = query.capture_index_for_name("injection.language");

    let mut regions: Vec<(&'static Language, Vec<tree_sitter::Range>)> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), node_text(rope));
    while let Some(m) = matches.next() {
        if m.pattern_index >= compiled.highlights_start {
            continue;
        }
        let settings = query.property_settings(m.pattern_index);
        let setting = |key: &str| settings.iter().find(|p| &*p.key == key);
        // `#set! injection.language "css"`, or the text of an
        // `@injection.language` capture.
        let name = match setting("injection.language").and_then(|p| p.value.as_deref()) {
            Some(name) => name.to_string(),
            None => match m.captures().iter().find(|c| Some(c.index) == named) {
                Some(c) => rope
                    .get_byte_slice(c.node.byte_range())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                None => continue,
            },
        };
        if !language
            .embeds
            .iter()
            .any(|e| e.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        let Some(embedded) = languages::for_name(&name) else {
            continue;
        };
        let include_children = setting("injection.include-children").is_some();
        for capture in m.captures().iter().filter(|c| Some(c.index) == content) {
            let ranges = region_ranges(capture.node, include_children);
            if !ranges.is_empty() {
                regions.push((embedded, ranges));
            }
        }
    }

    let mut injections = Vec::new();
    for (embedded, ranges) in regions {
        if parser.set_language(&(embedded.grammar)()).is_err()
            || parser.set_included_ranges(&ranges).is_err()
        {
            continue;
        }
        let Some(inner) = parser.parse_with_options(&mut read_rope(rope), None, None) else {
            continue;
        };
        let nested = find_injections(parser, embedded, &inner, rope, depth + 1);
        injections.push(Injection {
            language: embedded,
            tree: inner,
            injections: nested,
        });
    }
    injections
}

/// The text of an injected `node`: all of it, or, as tree-sitter's highlighter
/// does by default, only the gaps between its children.
fn region_ranges(node: Node, include_children: bool) -> Vec<tree_sitter::Range> {
    let whole = node.range();
    if include_children || node.child_count() == 0 {
        return vec![whole];
    }
    let mut ranges = Vec::new();
    let mut from = (whole.start_byte, whole.start_point);
    let mut walk = node.walk();
    for child in node.children(&mut walk) {
        if child.start_byte() > from.0 {
            ranges.push(tree_sitter::Range {
                start_byte: from.0,
                start_point: from.1,
                end_byte: child.start_byte(),
                end_point: child.start_position(),
            });
        }
        from = (child.end_byte(), child.end_position());
    }
    if whole.end_byte > from.0 {
        ranges.push(tree_sitter::Range {
            start_byte: from.0,
            start_point: from.1,
            end_byte: whole.end_byte,
            end_point: whole.end_point,
        });
    }
    ranges
}

/// The byte where the line holding `byte` ends, before its line break.
fn line_end(rope: &Rope, byte: usize) -> usize {
    let line = rope.byte_to_line(byte);
    let next = rope.line_to_byte(line + 1);
    if next > byte && rope.byte(next - 1) == b'\n' {
        next - 1
    } else {
        next
    }
}

/// The edit replacing chars `at..at + removed` of `rope` with `inserted`, as
/// tree-sitter wants it (bytes and row/byte-column points). Call before the rope
/// changes.
pub fn input_edit(rope: &Rope, at: usize, removed: usize, inserted: &str) -> InputEdit {
    let point = |byte: usize| {
        let row = rope.byte_to_line(byte);
        Point::new(row, byte - rope.line_to_byte(row))
    };
    let start_byte = rope.char_to_byte(at);
    let old_end_byte = rope.char_to_byte(at + removed);
    let start_position = point(start_byte);
    let new_end_position = match inserted.rfind('\n') {
        Some(nl) => Point::new(
            start_position.row + inserted.matches('\n').count(),
            inserted.len() - nl - 1,
        ),
        None => Point::new(start_position.row, start_position.column + inserted.len()),
    };
    InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte: start_byte + inserted.len(),
        start_position,
        old_end_position: point(old_end_byte),
        new_end_position,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles_of(text: &str) -> Vec<(String, Role)> {
        let rope = Rope::from_str(text);
        let mut h = Highlighter::for_path(Path::new("x.rs")).expect("rust");
        h.parse(&rope);
        h.spans(&rope, 0..rope.len_bytes())
            .into_iter()
            .map(|(r, role)| (text[r].to_string(), role))
            .collect()
    }

    #[test]
    fn rust_tokens_get_their_roles() {
        let roles = roles_of("fn main() { let x = 42; // hi\n}");
        assert!(roles.contains(&("fn".into(), Role::Keyword)), "{roles:?}");
        assert!(
            roles.contains(&("main".into(), Role::Function)),
            "{roles:?}"
        );
        assert!(roles.contains(&("42".into(), Role::Number)), "{roles:?}");
        assert!(
            roles.contains(&("// hi".into(), Role::Comment)),
            "{roles:?}"
        );
    }

    #[test]
    fn spans_are_sorted_and_disjoint() {
        let text = "#[derive(Debug)]\nstruct A { b: u8 }\n";
        let rope = Rope::from_str(text);
        let mut h = Highlighter::for_path(Path::new("x.rs")).expect("rust");
        h.parse(&rope);
        let spans = h.spans(&rope, 0..rope.len_bytes());
        assert!(spans.windows(2).all(|w| w[0].0.end <= w[1].0.start));
        // Only the asked-for bytes come back.
        let line2 = h.spans(&rope, 17..rope.len_bytes());
        assert!(line2.iter().all(|(r, _)| r.start >= 17));
    }

    #[test]
    fn an_edit_reparses_incrementally() {
        let mut rope = Rope::from_str("let a = b;\n");
        let mut h = Highlighter::for_path(Path::new("x.rs")).expect("rust");
        h.parse(&rope);
        let edit = input_edit(&rope, 8, 0, "\"");
        rope.insert(8, "\"");
        h.edit(&edit);
        h.parse(&rope);
        let spans = h.spans(&rope, 0..rope.len_bytes());
        // The quote, `b` and `;`: the rest of the line, though nothing closes it.
        for byte in [8, 9, 10] {
            let role = spans.iter().find(|(r, _)| r.contains(&byte)).map(|s| s.1);
            assert_eq!(role, Some(Role::String), "byte {byte}: {spans:?}");
        }
        assert!(
            spans.iter().all(|(r, _)| r.end <= 11),
            "past the line: {spans:?}"
        );
    }

    #[test]
    fn input_edit_counts_bytes_and_rows() {
        let rope = Rope::from_str("aé\nbc");
        let edit = input_edit(&rope, 4, 1, "x\nyz");
        assert_eq!(edit.start_byte, 5);
        assert_eq!(edit.old_end_byte, 6);
        assert_eq!(edit.new_end_byte, 9);
        assert_eq!(edit.start_position, Point::new(1, 1));
        assert_eq!(edit.old_end_position, Point::new(1, 2));
        assert_eq!(edit.new_end_position, Point::new(2, 2));
    }
}
