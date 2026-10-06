//! Where Griffin's char indices meet the protocol: LSP positions count UTF-16 code
//! units within a line, and documents are named by `file://` URIs.

use std::path::Path;
use std::str::FromStr;

use lsp_types::{Position, Range, Uri};
use ropey::Rope;

/// The LSP position of char index `char_idx` in `rope`. The one place char indices
/// become UTF-16 offsets; an index past the end is clamped to it.
pub fn lsp_position(rope: &Rope, char_idx: usize) -> Position {
    let char_idx = char_idx.min(rope.len_chars());
    let line = rope.char_to_line(char_idx);
    let line_start = rope.line_to_char(line);
    let character = rope.char_to_utf16_cu(char_idx) - rope.char_to_utf16_cu(line_start);
    Position {
        line: to_u32(line),
        character: to_u32(character),
    }
}

/// The char index of LSP position `position` in `rope`: the inverse of
/// `lsp_position`. A line past the end clamps to the end of the text, a column
/// past the line's end to that end (before its line break), and a column inside
/// a surrogate pair to the character it belongs to.
pub fn char_index(rope: &Rope, position: Position) -> usize {
    let line = usize::try_from(position.line).unwrap_or(usize::MAX);
    if line >= rope.len_lines() {
        return rope.len_chars();
    }
    let slice = rope.line(line);
    let mut content = slice.len_chars();
    // The line break isn't a column a position can name.
    while content > 0 && matches!(slice.char(content - 1), '\n' | '\r') {
        content -= 1;
    }
    let wanted = usize::try_from(position.character).unwrap_or(usize::MAX);
    let mut units = 0;
    let mut col = 0;
    for ch in slice.chars().take(content) {
        let width = ch.len_utf16();
        if units + width > wanted {
            break;
        }
        units += width;
        col += 1;
    }
    rope.line_to_char(line) + col
}

/// Protocol positions are `u32`; a document that long can't be described anyway.
fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The one region that differs between two versions of a text: its char range in
/// `old`, and the text that replaced it. `None` when they're the same. Several
/// edits since the last sync come out as one change covering all of them.
pub fn changed_region(old: &Rope, new: &Rope) -> Option<(Range, String)> {
    let (old_len, new_len) = (old.len_chars(), new.len_chars());
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .count();
    if prefix == old_len && prefix == new_len {
        return None;
    }
    let room = old_len.min(new_len) - prefix;
    let suffix = old
        .chars_at(old_len)
        .reversed()
        .zip(new.chars_at(new_len).reversed())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    let range = Range {
        start: lsp_position(old, prefix),
        end: lsp_position(old, old_len - suffix),
    };
    let text = new.slice(prefix..new_len - suffix).to_string();
    Some((range, text))
}

/// The `file://` URI for `path`, made absolute first. Bytes outside the URI's safe
/// set are percent-encoded, so spaces and non-ASCII names survive the trip.
pub fn path_to_uri(path: &Path) -> Option<Uri> {
    let absolute = std::path::absolute(path).ok()?;
    let text = absolute.to_str()?;
    #[cfg(windows)]
    let text = {
        // A verbatim prefix has no meaning inside a URI.
        let text = text.strip_prefix(r"\\?\").unwrap_or(text);
        format!("/{}", text.replace('\\', "/"))
    };
    let mut uri = String::from("file://");
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/:".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    Uri::from_str(&uri).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[test]
    fn ascii_positions_are_line_and_column() {
        let rope = Rope::from_str("ab\ncd\n");
        assert_eq!(lsp_position(&rope, 0), pos(0, 0));
        assert_eq!(lsp_position(&rope, 2), pos(0, 2));
        assert_eq!(lsp_position(&rope, 4), pos(1, 1));
        assert_eq!(lsp_position(&rope, 6), pos(2, 0));
        assert_eq!(lsp_position(&rope, 99), pos(2, 0));
    }

    #[test]
    fn characters_outside_the_bmp_count_as_two_utf16_units() {
        // `é` is one unit, the crab two.
        let rope = Rope::from_str("é🦀x\n🦀");
        assert_eq!(lsp_position(&rope, 1), pos(0, 1));
        assert_eq!(lsp_position(&rope, 2), pos(0, 3));
        assert_eq!(lsp_position(&rope, 3), pos(0, 4));
        assert_eq!(lsp_position(&rope, 4), pos(1, 0));
        assert_eq!(lsp_position(&rope, 5), pos(1, 2));
    }

    #[test]
    fn char_index_inverts_lsp_position_and_clamps() {
        let rope = Rope::from_str("é🦀x\r\nab\n");
        for i in 0..rope.len_chars() {
            let ch = rope.char(i);
            if ch == '\r' || ch == '\n' {
                continue;
            }
            assert_eq!(char_index(&rope, lsp_position(&rope, i)), i, "char {i}");
        }
        // Inside the crab's surrogate pair: the crab.
        assert_eq!(char_index(&rope, pos(0, 2)), 1);
        // Past the line's end: before its line break.
        assert_eq!(char_index(&rope, pos(0, 99)), 3);
        assert_eq!(char_index(&rope, pos(1, 99)), 7);
        // Past the last line: the end.
        assert_eq!(char_index(&rope, pos(9, 0)), rope.len_chars());
    }

    #[test]
    fn the_changed_region_is_what_differs_between_prefix_and_suffix() {
        let old = Rope::from_str("fn main() {}\n");
        let new = Rope::from_str("fn main() { x }\n");
        let (range, text) = changed_region(&old, &new).unwrap();
        assert_eq!(range.start, pos(0, 11));
        assert_eq!(range.end, pos(0, 11));
        assert_eq!(text, " x ");

        assert_eq!(changed_region(&old, &old.clone()), None);
    }

    #[test]
    fn a_deletion_has_an_empty_replacement() {
        let old = Rope::from_str("one\ntwo\nsix");
        let new = Rope::from_str("one\nsix");
        let (range, text) = changed_region(&old, &new).unwrap();
        assert_eq!((range.start, range.end), (pos(1, 0), pos(2, 0)));
        assert_eq!(text, "");
    }

    #[test]
    fn repeated_characters_never_overlap_prefix_and_suffix() {
        let old = Rope::from_str("aa");
        let new = Rope::from_str("aaa");
        let (range, text) = changed_region(&old, &new).unwrap();
        assert_eq!((range.start, range.end), (pos(0, 2), pos(0, 2)));
        assert_eq!(text, "a");
    }

    #[test]
    fn regions_use_utf16_columns() {
        let old = Rope::from_str("🦀b");
        let new = Rope::from_str("🦀cb");
        let (range, text) = changed_region(&old, &new).unwrap();
        assert_eq!(range.start, pos(0, 2));
        assert_eq!(text, "c");
    }

    #[test]
    fn uris_are_absolute_file_uris_with_unsafe_bytes_encoded() {
        let dir = tempfile::tempdir().unwrap();
        let uri = path_to_uri(&dir.path().join("my file.rs")).unwrap();
        let text = uri.as_str();
        assert!(text.starts_with("file:///"), "{text}");
        assert!(text.ends_with("/my%20file.rs"), "{text}");
        assert!(!text.contains('\\'), "{text}");
    }
}
