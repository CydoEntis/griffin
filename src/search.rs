//! Finding text in a buffer: plain or regex, with or without case, as char ranges
//! into the rope so the editor can highlight and jump to them.

use std::ops::Range;

use regex::RegexBuilder;
use ropey::Rope;

/// What to look for and how.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub pattern: String,
    pub case_sensitive: bool,
    pub regex: bool,
}

/// Every non-overlapping match of `query` in `rope`, in order, as char ranges. An
/// empty pattern finds nothing; an invalid regex is an error saying why.
pub fn find_all(rope: &Rope, query: &Query) -> Result<Vec<Range<usize>>, String> {
    Ok(replacements(rope, query, "")?
        .into_iter()
        .map(|(range, _)| range)
        .collect())
}

/// Every match of `query` in `rope`, as `find_all` finds them, each with the text
/// that replaces it: `template` as is in plain mode, and in regex mode with `$1`,
/// `${name}` and `$$` expanded from that match's groups.
pub fn replacements(
    rope: &Rope,
    query: &Query,
    template: &str,
) -> Result<Vec<(Range<usize>, String)>, String> {
    if query.pattern.is_empty() {
        return Ok(Vec::new());
    }
    // Plain text goes through the regex engine too, escaped, so case folding is
    // the same Unicode-aware folding either way.
    let pattern = if query.regex {
        query.pattern.clone()
    } else {
        regex::escape(&query.pattern)
    };
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(!query.case_sensitive)
        .multi_line(true)
        .build()
        .map_err(|err| match err {
            regex::Error::Syntax(_) => "invalid regex".to_string(),
            other => other.to_string(),
        })?;
    // The regex crate needs contiguous text; a snapshot of the rope is that.
    let text = rope.to_string();
    Ok(re
        .captures_iter(&text)
        .filter_map(|caps| {
            // Group 0 is always the whole match.
            let whole = caps.get(0)?;
            // An empty match (`a*`, `^`) has nothing to highlight or select.
            if whole.is_empty() {
                return None;
            }
            let range = rope.byte_to_char(whole.start())..rope.byte_to_char(whole.end());
            let with = if query.regex {
                let mut out = String::new();
                caps.expand(template, &mut out);
                out
            } else {
                template.to_string()
            };
            Some((range, with))
        })
        .collect())
}

/// The index of the first match starting at or after `pos`, wrapping to the first
/// match when none does. `None` only when there are no matches.
pub fn next_from(matches: &[Range<usize>], pos: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let i = matches.partition_point(|m| m.start < pos);
    Some(if i == matches.len() { 0 } else { i })
}

#[cfg(test)]
#[expect(
    clippy::single_range_in_vec_init,
    reason = "a list of one match, not a range of numbers"
)]
mod tests {
    use super::*;

    fn query(pattern: &str, case_sensitive: bool, regex: bool) -> Query {
        Query {
            pattern: pattern.to_string(),
            case_sensitive,
            regex,
        }
    }

    fn find(text: &str, q: &Query) -> Vec<Range<usize>> {
        find_all(&Rope::from_str(text), q).expect("valid query")
    }

    #[test]
    fn plain_text_ignores_case_by_default_and_escapes_regex_characters() {
        let text = "foo Foo FOO f.o";
        assert_eq!(find(text, &query("foo", false, false)), [0..3, 4..7, 8..11]);
        assert_eq!(find(text, &query("foo", true, false)), [0..3]);
        assert_eq!(find(text, &query("f.o", false, false)), [12..15]);
    }

    #[test]
    fn regex_mode_matches_patterns_per_line() {
        let text = "foo\nfoooo bar\nfo";
        assert!(find(text, &query("fo{2}", false, false)).is_empty());
        assert_eq!(find(text, &query("fo{2}", false, true)), [0..3, 4..7]);
        assert_eq!(find(text, &query("^fo+$", false, true)), [0..3, 14..16]);
    }

    #[test]
    fn ranges_are_char_indices_not_bytes() {
        assert_eq!(
            find("héllo héllo", &query("llo", false, false)),
            [2..5, 8..11]
        );
    }

    #[test]
    fn empty_pattern_and_empty_matches_find_nothing() {
        assert!(find("abc", &query("", false, false)).is_empty());
        assert!(find("abc", &query("x*", false, true)).is_empty());
    }

    #[test]
    fn invalid_regex_is_an_error() {
        let err = find_all(&Rope::from_str("abc"), &query("(", false, true)).unwrap_err();
        assert_eq!(err, "invalid regex");
        // Plain mode takes the same text literally.
        assert_eq!(find("a(b", &query("(", false, false)), [1..2]);
    }

    fn replace(text: &str, q: &Query, template: &str) -> Vec<(Range<usize>, String)> {
        replacements(&Rope::from_str(text), q, template).expect("valid query")
    }

    #[test]
    fn regex_replacements_expand_groups() {
        let q = query(r"(\w+)=(\d+)", false, true);
        assert_eq!(
            replace("a=1 bb=22", &q, "$2:$1"),
            [(0..3, "1:a".to_string()), (4..9, "22:bb".to_string())]
        );
        // Braces end a group name early; `$$` is a literal dollar.
        assert_eq!(replace("x=7", &q, "${1}_$$"), [(0..3, "x_$".to_string())]);
        let named = query(r"(?P<key>\w+)=", false, true);
        assert_eq!(replace("k=", &named, "<$key>"), [(0..2, "<k>".to_string())]);
    }

    #[test]
    fn plain_replacements_take_dollars_literally() {
        assert_eq!(
            replace("foo Foo", &query("foo", false, false), "$1"),
            [(0..3, "$1".to_string()), (4..7, "$1".to_string())]
        );
    }

    #[test]
    fn next_from_wraps_past_the_last_match() {
        let matches = [2..4, 6..8, 10..12];
        assert_eq!(next_from(&matches, 0), Some(0));
        assert_eq!(next_from(&matches, 2), Some(0));
        assert_eq!(next_from(&matches, 3), Some(1));
        assert_eq!(next_from(&matches, 11), Some(0));
        assert_eq!(next_from(&[], 0), None);
    }
}
