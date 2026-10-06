//! Finding text in a buffer: plain or regex, with or without case, as char ranges
//! into the rope so the editor can highlight and jump to them. Also finding it in
//! every file of the project, line by line, for project search.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use ignore::WalkState;
use regex::{Regex, RegexBuilder};
use ropey::Rope;

use crate::workspace::walk::{project_walk, relative_name};

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
    let re = compile(query)?;
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

/// `query` as a regex, `^` and `$` matching at line breaks. An invalid regex is an
/// error saying why.
pub fn compile(query: &Query) -> Result<Regex, String> {
    // Plain text goes through the regex engine too, escaped, so case folding is
    // the same Unicode-aware folding either way.
    let pattern = if query.regex {
        query.pattern.clone()
    } else {
        regex::escape(&query.pattern)
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!query.case_sensitive)
        .multi_line(true)
        .build()
        .map_err(|err| match err {
            regex::Error::Syntax(_) => "invalid regex".to_string(),
            other => other.to_string(),
        })
}

/// One line of a file that `re` matches somewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineHit {
    /// Counting from 0.
    pub line: usize,
    /// Char column of the first match in the whole line, for the cursor.
    pub col: usize,
    /// The line as a list shows it: leading whitespace and the line break gone,
    /// other control characters as spaces, cut short when very long.
    pub text: String,
    /// Each match as a char range into `text`; a match past the cut is left out.
    pub matches: Vec<Range<usize>>,
}

/// Longest `LineHit::text` kept, in chars; no list row is wider than this.
const MAX_HIT_CHARS: usize = 300;

/// Every line of `text` that `re` matches, in order. Lines are searched one at a
/// time, so `^` and `$` match at each line's ends and nothing spans a line break.
pub fn line_hits(text: &str, re: &Regex) -> Vec<LineHit> {
    let mut hits = Vec::new();
    for (line, content) in text.lines().enumerate() {
        let mut found = re.find_iter(content).filter(|m| !m.is_empty()).peekable();
        let Some(first) = found.peek() else {
            continue;
        };
        let col = content[..first.start()].chars().count();
        let trimmed = content.trim_start();
        let skipped = content.len() - trimmed.len();
        let skipped_chars = content[..skipped].chars().count();
        let shown: String = trimmed
            .chars()
            .take(MAX_HIT_CHARS)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let shown_len = shown.chars().count();
        let matches = found
            .filter_map(|m| {
                let start = content[..m.start()]
                    .chars()
                    .count()
                    .checked_sub(skipped_chars)?;
                let end = (content[..m.end()].chars().count() - skipped_chars).min(shown_len);
                (start < end).then_some(start..end)
            })
            .collect();
        hits.push(LineHit {
            line,
            col,
            text: shown,
            matches,
        });
    }
    hits
}

/// A line project search found, in the file at `path`: relative to the project
/// root, `/`-separated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: String,
    pub line: LineHit,
}

/// Every line of `text` that `re` matches, as hits in the file at `path`.
pub fn hits_in(path: &str, text: &str, re: &Regex) -> Vec<Hit> {
    line_hits(text, re)
        .into_iter()
        .map(|line| Hit {
            path: path.to_string(),
            line,
        })
        .collect()
}

/// Hits one walker thread collects before handing them on, so a large result
/// streams in without a message per line.
const BATCH: usize = 64;

/// Bytes looked at to decide a file is binary: one with a NUL in them is, as git
/// decides it.
const BINARY_SNIFF: usize = 8000;

/// Searches every file under `root` that the project shows (`.gitignore`d files
/// and `.git` left out), skipping binary files and the files in `skip`, given as
/// `std::path::absolute` makes them (open buffers, searched from memory instead).
/// Hits go to `send` in batches as the walker threads find them, so in no
/// particular order. Stops soon after `cancel` is set.
pub fn search_project(
    root: &Path,
    re: &Regex,
    skip: &HashSet<PathBuf>,
    cancel: &AtomicBool,
    send: &(dyn Fn(Vec<Hit>) + Sync),
) {
    project_walk(root).build_parallel().run(|| {
        let mut batch = HitBatch {
            hits: Vec::new(),
            send,
        };
        Box::new(move |entry| {
            if cancel.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            let path = entry.path();
            if entry.file_type().is_none_or(|kind| kind.is_dir())
                || std::path::absolute(path).is_ok_and(|path| skip.contains(&path))
            {
                return WalkState::Continue;
            }
            let (Ok(relative), Ok(bytes)) = (path.strip_prefix(root), std::fs::read(path)) else {
                return WalkState::Continue;
            };
            if bytes[..bytes.len().min(BINARY_SNIFF)].contains(&0) {
                return WalkState::Continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            batch
                .hits
                .extend(hits_in(&relative_name(relative), &text, re));
            if batch.hits.len() >= BATCH {
                batch.flush();
            }
            WalkState::Continue
        })
    });
}

/// One walker thread's hits not sent yet; the rest go when the thread finishes.
struct HitBatch<'a> {
    hits: Vec<Hit>,
    send: &'a (dyn Fn(Vec<Hit>) + Sync),
}

impl HitBatch<'_> {
    fn flush(&mut self) {
        if !self.hits.is_empty() {
            (self.send)(std::mem::take(&mut self.hits));
        }
    }
}

impl Drop for HitBatch<'_> {
    fn drop(&mut self) {
        self.flush();
    }
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

    fn hits(text: &str, q: &Query) -> Vec<LineHit> {
        line_hits(text, &compile(q).expect("valid query"))
    }

    #[test]
    fn line_hits_lists_matching_lines_with_their_matches() {
        let text = "fn a() {}\r\n    // TODO one, todo two\r\nnone\n\tTODO";
        let found = hits(text, &query("todo", false, false));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].line, 1);
        // The column counts the indent the shown text leaves out.
        assert_eq!(found[0].col, 7);
        assert_eq!(found[0].text, "// TODO one, todo two");
        assert_eq!(found[0].matches, [3..7, 13..17]);
        assert_eq!((found[1].line, found[1].col), (3, 1));
        assert_eq!(found[1].text, "TODO");
        // Case and regex behave as in the find bar; `$` ends each line.
        assert_eq!(hits(text, &query("todo", true, false)).len(), 1);
        let ends = hits(text, &query("two$", false, true));
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0].matches, [18..21]);
    }

    #[test]
    fn line_hits_cuts_long_lines_and_counts_chars_not_bytes() {
        let text = format!("é{}x", "-".repeat(400));
        let found = hits(&text, &query("é", false, false));
        assert_eq!(found[0].text.chars().count(), MAX_HIT_CHARS);
        assert_eq!(found[0].matches, [0..1]);
        // A match past the cut still finds the line, but has nothing to highlight.
        let late = hits(&text, &query("x", false, false));
        assert_eq!(late[0].col, 401);
        assert!(late[0].matches.is_empty());
    }

    /// Every batch `search_project` sends, for `pattern` under `root`.
    fn search_dir(root: &Path, pattern: &str, skip: &HashSet<PathBuf>) -> Vec<Vec<Hit>> {
        let batches = std::sync::Mutex::new(Vec::new());
        let re = compile(&query(pattern, false, false)).expect("valid query");
        let send = |hits| batches.lock().expect("no panics").push(hits);
        search_project(root, &re, skip, &AtomicBool::new(false), &send);
        batches.into_inner().expect("no panics")
    }

    #[test]
    fn project_search_respects_gitignore_and_skips_binary_and_open_files() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "*.log\n")?;
        std::fs::write(root.join("debug.log"), "TODO ignored\n")?;
        std::fs::write(root.join("data.bin"), b"TODO\0binary")?;
        std::fs::write(root.join("open.txt"), "TODO on disk\n")?;
        std::fs::create_dir_all(root.join("src"))?;
        std::fs::write(root.join("src").join("lib.rs"), "a\nb\n  // TODO here\n")?;
        std::fs::create_dir_all(root.join(".git"))?;
        std::fs::write(root.join(".git").join("TODO"), "TODO\n")?;
        let skip = HashSet::from([std::path::absolute(root.join("open.txt"))?]);

        let hits: Vec<Hit> = search_dir(root, "todo", &skip).concat();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].path, "src/lib.rs");
        assert_eq!((hits[0].line.line, hits[0].line.col), (2, 5));
        assert_eq!(hits[0].line.text, "// TODO here");
        Ok(())
    }

    #[test]
    fn project_search_streams_many_files_in_batches_and_stops_when_cancelled() -> anyhow::Result<()>
    {
        let dir = tempfile::tempdir()?;
        for n in 0..1000 {
            std::fs::write(dir.path().join(format!("f{n:04}.txt")), "x\nneedle\n")?;
        }
        let batches = search_dir(dir.path(), "needle", &HashSet::new());
        assert!(batches.len() > 1, "one batch of {}", batches[0].len());
        assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), 1000);

        let sent = std::sync::Mutex::new(0);
        let re = compile(&query("needle", false, false)).expect("valid query");
        let send = |hits: Vec<Hit>| *sent.lock().expect("no panics") += hits.len();
        search_project(
            dir.path(),
            &re,
            &HashSet::new(),
            &AtomicBool::new(true),
            &send,
        );
        assert_eq!(sent.into_inner().expect("no panics"), 0);
        Ok(())
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
