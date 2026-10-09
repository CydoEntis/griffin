//! Commenting lines out and back in (Ctrl+/), on the cursor's line or every line
//! a selection touches.

use std::ops::Range;

use super::Buffer;
use super::edit::Change;

impl Buffer {
    /// Comments out the lines in range with `marker` and a space, or, when every
    /// non-blank one is commented already, takes the marker (and one space after
    /// it) off again. Markers go at the least-indented line's column so a block
    /// stays lined up; blank lines are left alone. One undo step.
    pub fn toggle_comment(&mut self, marker: &str) {
        let lines = self.non_blank_lines();
        if lines.is_empty() {
            return;
        }
        let commented = lines
            .iter()
            .all(|(_, text)| unindented(text).starts_with(marker));
        let edits: Vec<_> = if commented {
            lines
                .iter()
                .map(|(start, text)| {
                    let at = start + indent(text);
                    let after = &unindented(text)[marker.len()..];
                    let len = marker.chars().count() + usize::from(after.starts_with(' '));
                    (at..at + len, String::new())
                })
                .collect()
        } else {
            let col = lines
                .iter()
                .map(|(_, text)| indent(text))
                .min()
                .unwrap_or(0);
            lines
                .iter()
                .map(|(start, _)| (start + col..start + col, format!("{marker} ")))
                .collect()
        };
        self.edit_lines(&edits);
    }

    /// `toggle_comment` for a language with only block comments (HTML, CSS): wraps
    /// each non-blank line in `open` and `close`, as `open line close`, or, when
    /// every one is wrapped already, unwraps them, taking one space inside each
    /// marker along. One undo step.
    pub fn toggle_wrap_comment(&mut self, open: &str, close: &str) {
        let lines = self.non_blank_lines();
        if lines.is_empty() {
            return;
        }
        let wrapped = lines
            .iter()
            .all(|(_, text)| wrapped_body(text, open, close).is_some());
        let mut edits = Vec::new();
        if wrapped {
            for (start, text) in &lines {
                let Some(inner) = wrapped_body(text, open, close) else {
                    continue;
                };
                let lead = inner.starts_with(' ');
                // A lone space between the markers is the opening one's.
                let trail = inner.ends_with(' ') && inner.len() > usize::from(lead);
                let at = start + indent(text);
                let opened = at + open.chars().count();
                let closed = opened + inner.chars().count();
                edits.push((at..opened + usize::from(lead), String::new()));
                edits.push((
                    closed - usize::from(trail)..closed + close.chars().count(),
                    String::new(),
                ));
            }
        } else {
            let col = lines
                .iter()
                .map(|(_, text)| indent(text))
                .min()
                .unwrap_or(0);
            // Known limit: a line already holding `close` ends the new comment
            // early there, as neither HTML nor CSS comments nest.
            for (start, text) in &lines {
                let end = start + text.chars().count();
                edits.push((start + col..start + col, format!("{open} ")));
                edits.push((end..end, format!(" {close}")));
            }
        }
        self.edit_lines(&edits);
    }

    /// The lines Ctrl+/ acts on: the cursor's, or every line the selection
    /// touches. A selection ending at a line's very start doesn't take that line,
    /// since selecting whole lines with Shift+Down ends there.
    fn comment_lines(&self) -> Range<usize> {
        let Some(range) = self.selection() else {
            let line = self.rope.char_to_line(self.cursor);
            return line..line + 1;
        };
        let first = self.rope.char_to_line(range.start);
        let mut last = self.rope.char_to_line(range.end);
        if last > first && self.rope.line_to_char(last) == range.end {
            last -= 1;
        }
        first..last + 1
    }

    /// Each non-blank line in `comment_lines`, as its first char index and its
    /// text without the line break.
    fn non_blank_lines(&self) -> Vec<(usize, String)> {
        self.comment_lines()
            .map(|line| {
                let text = self.rope.line(line).to_string();
                let text = text.trim_end_matches(['\n', '\r']).to_string();
                (self.rope.line_to_char(line), text)
            })
            .filter(|(_, text)| !text.trim().is_empty())
            .collect()
    }

    /// Applies `edits` (sorted, not overlapping) as one undo step, then puts the
    /// cursor and any selection back on the same text, so a selection still covers
    /// the lines it did. Text put in exactly where a position sits goes after it:
    /// a selection from a line's start then takes in the new marker too.
    fn edit_lines(&mut self, edits: &[(Range<usize>, String)]) {
        if edits.is_empty() {
            return;
        }
        let map = |pos: usize| {
            let mut moved = pos;
            for (range, inserted) in edits {
                if range.start >= pos {
                    break;
                }
                let kept = range.end.min(pos);
                moved = moved - (kept - range.start) + inserted.chars().count();
            }
            moved
        };
        let anchor = self.selection().and(self.anchor);
        let (cursor, new_anchor) = (map(self.cursor), anchor.map(map));
        self.begin_group();
        // The last first, so the earlier ranges still point at their text.
        for (range, inserted) in edits.iter().rev() {
            let removed = self.rope.slice(range.clone()).to_string();
            self.apply(Change {
                at: range.start,
                removed,
                inserted: inserted.clone(),
            });
        }
        self.end_group();
        if let Some(anchor) = anchor {
            self.history.select_on_undo(anchor);
        }
        self.cursor = cursor;
        self.anchor = new_anchor;
        self.goal_col = None;
    }
}

/// How many chars of leading spaces and tabs `text` has.
fn indent(text: &str) -> usize {
    text.chars().take_while(|c| matches!(c, ' ' | '\t')).count()
}

fn unindented(text: &str) -> &str {
    text.trim_start_matches([' ', '\t'])
}

/// What sits between `open` and `close` when `text`, past its indent and before
/// any trailing whitespace, is wrapped in them; `None` when it isn't. A line
/// that only starts and ends with comments (`/* a */ b; /* c */`) has a marker
/// inside and isn't one comment: unwrapping it would uncomment its middle.
fn wrapped_body<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let body = unindented(text).trim_end_matches([' ', '\t']);
    let inner = body.strip_prefix(open)?.strip_suffix(close)?;
    (!inner.contains(open) && !inner.contains(close)).then_some(inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    /// A buffer holding `text` with the cursor at `|` and, if present, the anchor
    /// at `^`.
    fn buf(text: &str) -> Buffer {
        let (mut plain, mut cursor, mut anchor) = (String::new(), None, None);
        for ch in text.chars() {
            let at = plain.chars().count();
            match ch {
                '|' => cursor = Some(at),
                '^' => anchor = Some(at),
                _ => plain.push(ch),
            }
        }
        Buffer {
            rope: Rope::from_str(&plain),
            cursor: cursor.expect("cursor marker"),
            anchor,
            ..Buffer::empty()
        }
    }

    /// The text with `|` at the cursor and `^` at the anchor.
    fn show(b: &Buffer) -> String {
        let mut marks = vec![(b.cursor, '|')];
        if let Some(anchor) = b.anchor {
            marks.push((anchor, '^'));
        }
        // From the end, so the earlier position still points at its char.
        marks.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut text = b.rope.to_string();
        for (pos, mark) in marks {
            text.insert(b.rope.char_to_byte(pos), mark);
        }
        text
    }

    #[test]
    fn comments_and_uncomments_the_cursor_line() {
        let mut b = buf("let |x = 1;\nlet y = 2;\n");
        b.toggle_comment("//");
        assert_eq!(show(&b), "// let |x = 1;\nlet y = 2;\n");
        b.toggle_comment("//");
        assert_eq!(show(&b), "let |x = 1;\nlet y = 2;\n");
    }

    #[test]
    fn a_selection_takes_every_line_it_touches_but_one_it_ends_at_the_start_of() {
        let mut b = buf("^a\nb\nc\n|d\n");
        b.toggle_comment("#");
        assert_eq!(show(&b), "^# a\n# b\n# c\n|d\n");
        let mut b = buf("a\nb^b\nc|c\nd\n");
        b.toggle_comment("#");
        assert_eq!(show(&b), "a\n# b^b\n# c|c\nd\n");
    }

    #[test]
    fn mixed_lines_are_all_commented() {
        let mut b = buf("^// a\nb\n|");
        b.toggle_comment("//");
        assert_eq!(b.rope.to_string(), "// // a\n// b\n");
    }

    #[test]
    fn markers_go_at_the_least_indented_column() {
        let mut b = buf("^    if x {\n        y();\n    }|\n");
        b.toggle_comment("//");
        assert_eq!(
            b.rope.to_string(),
            "    // if x {\n    //     y();\n    // }\n"
        );
        b.toggle_comment("//");
        assert_eq!(b.rope.to_string(), "    if x {\n        y();\n    }\n");
    }

    #[test]
    fn tab_indents_are_kept() {
        let mut b = buf("^\tif x {\n\t\ty()\n\t}|");
        b.toggle_comment("--");
        assert_eq!(b.rope.to_string(), "\t-- if x {\n\t-- \ty()\n\t-- }");
        b.toggle_comment("--");
        assert_eq!(b.rope.to_string(), "\tif x {\n\t\ty()\n\t}");
    }

    #[test]
    fn blank_lines_are_left_alone() {
        let mut b = buf("^a\n\n   \nb|");
        b.toggle_comment("#");
        assert_eq!(b.rope.to_string(), "# a\n\n   \n# b");
        b.toggle_comment("#");
        assert_eq!(b.rope.to_string(), "a\n\n   \nb");
        let mut blank = buf("  |\n");
        blank.toggle_comment("#");
        assert_eq!(blank.rope.to_string(), "  \n");
        assert!(
            !blank.history.can_undo(),
            "nothing to comment, no undo step"
        );
    }

    #[test]
    fn a_marker_without_a_space_after_it_still_uncomments() {
        let mut b = buf("^//a\n// b\n  //  c|");
        b.toggle_comment("//");
        assert_eq!(b.rope.to_string(), "a\nb\n   c");
    }

    #[test]
    fn one_undo_step_brings_back_the_text_and_selection() {
        let mut b = buf("^a\nb|\n");
        b.toggle_comment("//");
        assert_eq!(show(&b), "^// a\n// b|\n");
        assert!(b.undo());
        assert_eq!(show(&b), "^a\nb|\n");
        assert!(!b.undo(), "the whole toggle was one step");
    }

    #[test]
    fn the_selection_covers_the_same_lines_either_way_round() {
        let mut b = buf("  a|b\n  c^d\n");
        b.toggle_comment("#");
        assert_eq!(show(&b), "  # a|b\n  # c^d\n");
        b.toggle_comment("#");
        assert_eq!(show(&b), "  a|b\n  c^d\n");
    }

    #[test]
    fn html_lines_are_wrapped_and_unwrapped() {
        let mut b = buf("<p>|hi</p>
");
        b.toggle_wrap_comment("<!--", "-->");
        assert_eq!(
            show(&b),
            "<!-- <p>|hi</p> -->
"
        );
        b.toggle_wrap_comment("<!--", "-->");
        assert_eq!(
            show(&b),
            "<p>|hi</p>
"
        );
    }

    #[test]
    fn wrapping_starts_at_the_least_indented_column_and_keeps_the_text() {
        let mut b = buf("^  a {

    color: red;
  }|");
        b.toggle_wrap_comment("/*", "*/");
        assert_eq!(
            b.rope.to_string(),
            "  /* a { */

  /*   color: red; */
  /* } */"
        );
        b.toggle_wrap_comment("/*", "*/");
        assert_eq!(
            b.rope.to_string(),
            "  a {

    color: red;
  }"
        );
    }

    #[test]
    fn a_line_not_wrapped_gets_wrapped_with_the_rest() {
        let mut b = buf("^/* a */
b|");
        b.toggle_wrap_comment("/*", "*/");
        assert_eq!(
            b.rope.to_string(),
            "/* /* a */ */
/* b */"
        );
    }

    #[test]
    fn markers_without_spaces_inside_still_unwrap() {
        let mut b = buf("^<!--a-->
<!-- -->
<!---->  |");
        b.toggle_wrap_comment("<!--", "-->");
        assert_eq!(
            b.rope.to_string(),
            "a

  "
        );
    }

    #[test]
    fn a_wrap_is_one_undo_step_and_keeps_the_selection() {
        let mut b = buf("^<p>a</p>
<p>b</p>|
");
        b.toggle_wrap_comment("<!--", "-->");
        assert_eq!(
            show(&b),
            "^<!-- <p>a</p> -->
<!-- <p>b</p>| -->
"
        );
        assert!(b.undo());
        assert_eq!(
            show(&b),
            "^<p>a</p>
<p>b</p>|
"
        );
        assert!(!b.undo(), "the whole wrap was one step");
    }

    #[test]
    fn a_line_with_comments_only_at_its_ends_gets_wrapped() {
        let mut b = buf("/* a */ color: red; /* b */|");
        b.toggle_wrap_comment("/*", "*/");
        assert_eq!(b.rope.to_string(), "/* /* a */ color: red; /* b */ */");
        let mut b = buf("<!-- x --><p>hi</p><!-- y -->|");
        b.toggle_wrap_comment("<!--", "-->");
        assert_eq!(b.rope.to_string(), "<!-- <!-- x --><p>hi</p><!-- y --> -->");
    }
}
