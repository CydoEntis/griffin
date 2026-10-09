mod harness;

// The engine depends only on `ropey` and the tree-sitter crates, so the role tests
// compile it straight from the source instead of going through the binary.
#[allow(dead_code)]
#[path = "../src/highlight/mod.rs"]
mod highlight;

use std::fs;
use std::ops::Range;
use std::path::Path;
use std::time::{Duration, Instant};

use harness::Tome;
use highlight::{Highlighter, Role, input_edit};
use ropey::Rope;
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);

const SAMPLE: &str = "tests/fixtures/highlight/sample.rs";
const SAMPLE_TS: &str = "tests/fixtures/highlight/sample.ts";
const SAMPLE_TSX: &str = "tests/fixtures/highlight/sample.tsx";
const SAMPLE_JS: &str = "tests/fixtures/highlight/sample.js";
const SAMPLE_HTML: &str = "tests/fixtures/highlight/sample.html";
const SAMPLE_CSS: &str = "tests/fixtures/highlight/sample.css";
const SAMPLE_GO: &str = "tests/fixtures/highlight/sample.go";
const SAMPLE_PY: &str = "tests/fixtures/highlight/sample.py";
const SAMPLE_SQL: &str = "tests/fixtures/highlight/sample.sql";

/// hydra's syntax and text colours.
const KEYWORD: Color = Color::Rgb(0xa5, 0x93, 0xff);
const STRING: Color = Color::Rgb(0x7f, 0xd9, 0x62);
const COMMENT: Color = Color::Rgb(0x71, 0x80, 0x8f);
const TAG: Color = Color::Rgb(0xff, 0x7a, 0xb6);
const PROPERTY: Color = Color::Rgb(0xe8, 0xc5, 0x65);
const FG: Color = Color::Rgb(0xc9, 0xd1, 0xd9);

/// The file at `path` highlighted as Tome would: sorted byte ranges with the
/// role each is drawn in.
fn highlight_roles(path: &Path) -> Vec<(Range<usize>, Role)> {
    let text = fs::read_to_string(path).expect("read fixture");
    let rope = Rope::from_str(&text);
    let mut highlighter = Highlighter::for_path(path).expect("a registered language");
    highlighter.parse(&rope);
    highlighter.spans(&rope, 0..rope.len_bytes())
}

/// The role at 1-based `line` and `col` (chars) of `path`, if any.
fn role_at(path: &Path, roles: &[(Range<usize>, Role)], line: usize, col: usize) -> Option<Role> {
    let rope = Rope::from_str(&fs::read_to_string(path).expect("read fixture"));
    let byte = rope.char_to_byte(rope.line_to_char(line - 1) + col - 1);
    roles
        .iter()
        .find(|(range, _)| range.contains(&byte))
        .map(|&(_, role)| role)
}

#[test]
fn rust_roles() {
    let path = Path::new(SAMPLE);
    let roles = highlight_roles(path);
    let at = |line, col| role_at(path, &roles, line, col);
    // `// A sample ...`, and the doc comment.
    assert_eq!(at(1, 1), Some(Role::Comment));
    assert_eq!(at(1, 20), Some(Role::Comment));
    assert_eq!(at(4, 5), Some(Role::Comment));
    // `use`, `struct`, `fn`, `let`, `as`.
    assert_eq!(at(2, 1), Some(Role::Keyword));
    assert_eq!(at(5, 1), Some(Role::Keyword));
    assert_eq!(at(10, 1), Some(Role::Keyword));
    assert_eq!(at(11, 5), Some(Role::Keyword));
    assert_eq!(at(11, 26), Some(Role::Keyword));
    // `Point` where defined and used; `i32` and `f64`.
    assert_eq!(at(5, 8), Some(Role::Type));
    assert_eq!(at(10, 17), Some(Role::Type));
    assert_eq!(at(6, 8), Some(Role::Type));
    assert_eq!(at(10, 39), Some(Role::Type));
    // `distance` defined and called; `.sqrt()`; `println!`.
    assert_eq!(at(10, 4), Some(Role::Function));
    assert_eq!(at(20, 13), Some(Role::Function));
    assert_eq!(at(13, 25), Some(Role::Function));
    assert_eq!(at(21, 5), Some(Role::Function));
    // `"origin"`, first quote to last.
    assert_eq!(at(18, 17), Some(Role::String));
    assert_eq!(at(18, 24), Some(Role::String));
    // `0` and `2.5`.
    assert_eq!(at(17, 29), Some(Role::Number));
    assert_eq!(at(19, 17), Some(Role::Number));
    // `x:` is a field; plain names are uncoloured.
    assert_eq!(at(6, 5), Some(Role::Property));
    assert_eq!(at(11, 9), None);
}

#[test]
fn typescript_roles() {
    let path = Path::new(SAMPLE_TS);
    let roles = highlight_roles(path);
    let at = |line, col| role_at(path, &roles, line, col);
    assert_eq!(at(1, 1), Some(Role::Comment));
    // `interface`, `function`, `const`, `return`.
    assert_eq!(at(2, 1), Some(Role::Keyword));
    assert_eq!(at(7, 1), Some(Role::Keyword));
    assert_eq!(at(8, 3), Some(Role::Keyword));
    assert_eq!(at(9, 3), Some(Role::Keyword));
    // `Point` declared and used; `number` and `string`.
    assert_eq!(at(2, 11), Some(Role::Type));
    assert_eq!(at(7, 22), Some(Role::Type));
    assert_eq!(at(3, 6), Some(Role::Type));
    assert_eq!(at(4, 10), Some(Role::Type));
    // `distance` defined and called; `.sqrt()`.
    assert_eq!(at(7, 10), Some(Role::Function));
    assert_eq!(at(13, 13), Some(Role::Function));
    assert_eq!(at(9, 15), Some(Role::Function));
    // `"origin"`, first quote to last; `0`.
    assert_eq!(at(12, 38), Some(Role::String));
    assert_eq!(at(12, 45), Some(Role::String));
    assert_eq!(at(12, 28), Some(Role::Number));
}

#[test]
fn tsx_roles() {
    let path = Path::new(SAMPLE_TSX);
    let roles = highlight_roles(path);
    let at = |line, col| role_at(path, &roles, line, col);
    // `import`, `type`, `export`, `return`.
    assert_eq!(at(2, 1), Some(Role::Keyword));
    assert_eq!(at(4, 1), Some(Role::Keyword));
    assert_eq!(at(6, 1), Some(Role::Keyword));
    assert_eq!(at(8, 3), Some(Role::Keyword));
    // `"./button"`.
    assert_eq!(at(2, 24), Some(Role::String));
    // `Props` declared; `string`.
    assert_eq!(at(4, 6), Some(Role::Type));
    assert_eq!(at(4, 23), Some(Role::Type));
    // `App`, `go = () =>`, `alert(...)`.
    assert_eq!(at(6, 17), Some(Role::Function));
    assert_eq!(at(7, 9), Some(Role::Function));
    assert_eq!(at(7, 20), Some(Role::Function));
    // `<Button onClick={go}>`: the component is a tag, its prop an attribute;
    // and the closing `</Button>`.
    assert_eq!(at(8, 11), Some(Role::Tag));
    assert_eq!(at(8, 16), Some(Role::Tag));
    assert_eq!(at(8, 18), Some(Role::Attribute));
    assert_eq!(at(8, 24), Some(Role::Attribute));
    assert_eq!(at(8, 40), Some(Role::Tag));
}

#[test]
fn javascript_roles() {
    let path = Path::new(SAMPLE_JS);
    let roles = highlight_roles(path);
    let at = |line, col| role_at(path, &roles, line, col);
    assert_eq!(at(1, 1), Some(Role::Comment));
    // `import`, `function`, `return`, `const`.
    assert_eq!(at(2, 1), Some(Role::Keyword));
    assert_eq!(at(4, 1), Some(Role::Keyword));
    assert_eq!(at(5, 3), Some(Role::Keyword));
    assert_eq!(at(8, 1), Some(Role::Keyword));
    // `"hello "`.
    assert_eq!(at(5, 10), Some(Role::String));
    // `greet` defined and called; `render(...)`.
    assert_eq!(at(4, 10), Some(Role::Function));
    assert_eq!(at(8, 41), Some(Role::Function));
    assert_eq!(at(9, 1), Some(Role::Function));
    // `<div className="greeting">`: tag, attribute, string value.
    assert_eq!(at(8, 15), Some(Role::Tag));
    assert_eq!(at(8, 19), Some(Role::Attribute));
    assert_eq!(at(8, 29), Some(Role::String));
    assert_eq!(at(9, 14), Some(Role::Number));
}

#[test]
fn css_roles() {
    let path = Path::new(SAMPLE_CSS);
    let roles = highlight_roles(path);
    let at = |line, col| role_at(path, &roles, line, col);
    assert_eq!(at(1, 1), Some(Role::Comment));
    // Selectors: `body` and `h1` are tags, the class in `.card` a property.
    assert_eq!(at(2, 1), Some(Role::Tag));
    assert_eq!(at(7, 2), Some(Role::Property));
    assert_eq!(at(7, 9), Some(Role::Tag));
    // `margin`, `font-family`, `padding`.
    assert_eq!(at(3, 3), Some(Role::Property));
    assert_eq!(at(4, 3), Some(Role::Property));
    assert_eq!(at(4, 13), Some(Role::Property));
    assert_eq!(at(8, 3), Some(Role::Property));
    // `"Helvetica"`, first quote to last.
    assert_eq!(at(4, 16), Some(Role::String));
    assert_eq!(at(4, 26), Some(Role::String));
    // `0`, `12` and `1.5`; `px` is a unit.
    assert_eq!(at(3, 11), Some(Role::Number));
    assert_eq!(at(8, 12), Some(Role::Number));
    assert_eq!(at(8, 17), Some(Role::Number));
    assert_eq!(at(8, 14), Some(Role::Type));
}

#[test]
fn html_roles() {
    let path = Path::new(SAMPLE_HTML);
    let roles = highlight_roles(path);
    let at = |line, col| role_at(path, &roles, line, col);
    // `<html lang="en">`: tag, attribute, value.
    assert_eq!(at(2, 2), Some(Role::Tag));
    assert_eq!(at(2, 7), Some(Role::Attribute));
    assert_eq!(at(2, 13), Some(Role::String));
    // `<p class="note">` and its closing `</p>`.
    assert_eq!(at(9, 4), Some(Role::Tag));
    assert_eq!(at(9, 6), Some(Role::Attribute));
    assert_eq!(at(9, 13), Some(Role::String));
    assert_eq!(at(9, 26), Some(Role::Tag));
    // The text between tags is uncoloured.
    assert_eq!(at(9, 19), None);
    // Inside `<style>`: `p { color: red; }` in the CSS grammar.
    assert_eq!(at(5, 5), Some(Role::Tag));
    assert_eq!(at(5, 9), Some(Role::Property));
    // Inside `<script>`: `const greeting = "hi";` in the JavaScript grammar.
    assert_eq!(at(11, 5), Some(Role::Keyword));
    assert_eq!(at(11, 22), Some(Role::String));
}

#[test]
fn go_roles() {
    let path = Path::new(SAMPLE_GO);
    let roles = highlight_roles(path);
    let at = |line, col| role_at(path, &roles, line, col);
    assert_eq!(at(1, 1), Some(Role::Comment));
    assert_eq!(at(1, 20), Some(Role::Comment));
    // `package`, `import`, `type`, `struct`, `func`, `return`.
    assert_eq!(at(2, 1), Some(Role::Keyword));
    assert_eq!(at(4, 1), Some(Role::Keyword));
    assert_eq!(at(6, 1), Some(Role::Keyword));
    assert_eq!(at(6, 12), Some(Role::Keyword));
    assert_eq!(at(11, 1), Some(Role::Keyword));
    assert_eq!(at(13, 2), Some(Role::Keyword));
    // `"fmt"`, first quote to last; `"origin"`.
    assert_eq!(at(4, 8), Some(Role::String));
    assert_eq!(at(4, 12), Some(Role::String));
    assert_eq!(at(18, 14), Some(Role::String));
    // `Point` declared and used; `int` and `float64`.
    assert_eq!(at(6, 6), Some(Role::Type));
    assert_eq!(at(11, 17), Some(Role::Type));
    assert_eq!(at(7, 4), Some(Role::Type));
    assert_eq!(at(11, 33), Some(Role::Type));
    // `distance` defined and called; `fmt.Println`.
    assert_eq!(at(11, 6), Some(Role::Function));
    assert_eq!(at(18, 24), Some(Role::Function));
    assert_eq!(at(18, 6), Some(Role::Function));
    // The field `X`; `2.5`.
    assert_eq!(at(7, 2), Some(Role::Property));
    assert_eq!(at(13, 14), Some(Role::Number));
}

#[test]
fn python_roles() {
    let path = Path::new(SAMPLE_PY);
    let roles = highlight_roles(path);
    let at = |line, col| role_at(path, &roles, line, col);
    assert_eq!(at(1, 1), Some(Role::Comment));
    assert_eq!(at(1, 20), Some(Role::Comment));
    // `import`, `class`, `def`, `return`.
    assert_eq!(at(2, 1), Some(Role::Keyword));
    assert_eq!(at(5, 1), Some(Role::Keyword));
    assert_eq!(at(6, 5), Some(Role::Keyword));
    assert_eq!(at(12, 1), Some(Role::Keyword));
    assert_eq!(at(13, 5), Some(Role::Keyword));
    // `Point` declared and annotated; `int`.
    assert_eq!(at(5, 7), Some(Role::Type));
    assert_eq!(at(12, 17), Some(Role::Type));
    assert_eq!(at(6, 27), Some(Role::Type));
    // `__init__` and `distance` defined; `abs(...)` and `print(...)` called.
    assert_eq!(at(6, 9), Some(Role::Function));
    assert_eq!(at(12, 5), Some(Role::Function));
    assert_eq!(at(13, 12), Some(Role::Function));
    assert_eq!(at(17, 1), Some(Role::Function));
    assert_eq!(at(13, 29), Some(Role::Number));
    // The decorator `@functools.cache`, the `@` and both names.
    assert_eq!(at(11, 1), Some(Role::Attribute));
    assert_eq!(at(11, 2), Some(Role::Attribute));
    assert_eq!(at(11, 12), Some(Role::Attribute));
    // The f-string: prefix, quotes and text are a string; the call inside the
    // braces is a function.
    assert_eq!(at(17, 7), Some(Role::String));
    assert_eq!(at(17, 8), Some(Role::String));
    assert_eq!(at(17, 10), Some(Role::String));
    assert_eq!(at(17, 46), Some(Role::String));
    assert_eq!(at(17, 50), Some(Role::String));
    assert_eq!(at(17, 20), Some(Role::Function));
}

#[test]
fn sql_roles() {
    let path = Path::new(SAMPLE_SQL);
    let roles = highlight_roles(path);
    let at = |line, col| role_at(path, &roles, line, col);
    // `--` line comment and `/* */` block comment.
    assert_eq!(at(1, 1), Some(Role::Comment));
    assert_eq!(at(1, 20), Some(Role::Comment));
    assert_eq!(at(14, 1), Some(Role::Comment));
    assert_eq!(at(14, 30), Some(Role::Comment));
    // `SELECT`, `FROM`, `WHERE`, first letter to last; `CREATE TABLE`.
    assert_eq!(at(8, 1), Some(Role::Keyword));
    assert_eq!(at(8, 6), Some(Role::Keyword));
    assert_eq!(at(9, 1), Some(Role::Keyword));
    assert_eq!(at(9, 4), Some(Role::Keyword));
    assert_eq!(at(10, 1), Some(Role::Keyword));
    assert_eq!(at(10, 5), Some(Role::Keyword));
    assert_eq!(at(2, 1), Some(Role::Keyword));
    assert_eq!(at(2, 8), Some(Role::Keyword));
    // `'origin'` and `'skip'`, quote to quote.
    assert_eq!(at(4, 33), Some(Role::String));
    assert_eq!(at(4, 40), Some(Role::String));
    assert_eq!(at(10, 28), Some(Role::String));
    assert_eq!(at(10, 33), Some(Role::String));
    // `10`, `2.5` and `42` are numbers, not strings.
    assert_eq!(at(5, 15), Some(Role::Number));
    assert_eq!(at(8, 19), Some(Role::Number));
    assert_eq!(at(8, 21), Some(Role::Number));
    assert_eq!(at(10, 12), Some(Role::Number));
    // Line 12 doesn't parse. Its brackets and words aren't read as the start of
    // a string or comment that swallows what follows, and the statement after
    // it is still coloured.
    for col in [1, 13, 21, 32, 33] {
        let role = at(12, col);
        assert!(
            !matches!(role, Some(Role::String | Role::Comment)),
            "line 12 col {col} is {role:?}"
        );
    }
    assert_eq!(at(15, 1), Some(Role::Keyword));
    assert_eq!(at(15, 8), Some(Role::Keyword));
    assert_eq!(at(15, 31), Some(Role::Keyword));
    assert_eq!(at(15, 39), Some(Role::String));
    assert_eq!(at(15, 46), Some(Role::Number));
}

/// The screen row of buffer line `n` (1-based) with the view at the top: the
/// editor starts below the three rows of the tab header.
fn line(n: u16) -> u16 {
    n + 2
}

#[test]
fn go_shows_keyword_colour_on_screen() {
    let tome = Tome::spawn(&[SAMPLE_GO]);
    tome.wait_for_text("func distance", START);
    let row = line(11);
    let col = tome
        .text_col(row, "func distance")
        .expect("line 11 is on screen");
    tome.wait_for_fg_at(col, row, KEYWORD, WAIT);
    assert_eq!(tome.fg_at(col + 3, row), KEYWORD);
    // `distance` isn't a keyword.
    assert_ne!(tome.fg_at(col + 5, row), KEYWORD);
}

#[test]
fn sql_shows_keyword_colour_on_screen() {
    let tome = Tome::spawn(&[SAMPLE_SQL]);
    tome.wait_for_text("SELECT label", START);
    let row = line(8);
    let col = tome
        .text_col(row, "SELECT label")
        .expect("line 8 is on screen");
    tome.wait_for_fg_at(col, row, KEYWORD, WAIT);
    assert_eq!(tome.fg_at(col + 5, row), KEYWORD);
    // `label` is a column, not a keyword.
    assert_ne!(tome.fg_at(col + 7, row), KEYWORD);
    // The line that doesn't parse still shows its text.
    tome.wait_for_text("this is not ) valid sql at all (;", WAIT);
}

#[test]
fn html_injections_show_on_screen() {
    let tome = Tome::spawn(&[SAMPLE_HTML]);
    tome.wait_for_text("const greeting", START);
    let property = tome
        .text_col(line(5), "color: red")
        .expect("line 5 on screen");
    tome.wait_for_fg_at(property, line(5), PROPERTY, WAIT);
    assert_eq!(tome.fg_at(property + 4, line(5)), PROPERTY);
    let keyword = tome
        .text_col(line(11), "const greeting")
        .expect("line 11 on screen");
    tome.wait_for_fg_at(keyword, line(11), KEYWORD, WAIT);
    assert_eq!(tome.fg_at(keyword + 4, line(11)), KEYWORD);
    // `greeting` is a plain name, not a keyword.
    assert_ne!(tome.fg_at(keyword + 6, line(11)), KEYWORD);
}

#[test]
fn tsx_shows_tag_colour_on_screen() {
    let tome = Tome::spawn(&[SAMPLE_TSX]);
    tome.wait_for_text("<Button onClick", START);
    let row = line(8);
    let tag = tome
        .text_col(row, "Button onClick")
        .expect("line 8 on screen");
    tome.wait_for_fg_at(tag, row, TAG, WAIT);
    assert_eq!(tome.fg_at(tag + 5, row), TAG);
    // `onClick` is an attribute, not a tag.
    assert_ne!(tome.fg_at(tag + 7, row), TAG);
}

#[test]
fn rust_shows_theme_colours_on_screen() {
    let tome = Tome::spawn(&[SAMPLE]);
    tome.wait_for_text("fn distance", START);
    let row = line(10);
    let col = tome
        .text_col(row, "fn distance")
        .expect("line 10 is on screen");
    tome.wait_for_fg_at(col, row, KEYWORD, WAIT);
    assert_eq!(tome.fg_at(col + 1, row), KEYWORD);
    // `distance` itself isn't a keyword.
    assert_ne!(tome.fg_at(col + 3, row), KEYWORD);
    let comment = tome
        .text_col(line(1), "// A sample")
        .expect("comment on screen");
    assert_eq!(tome.fg_at(comment, line(1)), COMMENT);
    assert_eq!(tome.fg_at(comment + 5, line(1)), COMMENT);
    let string = tome
        .text_col(line(18), "\"origin\"")
        .expect("string on screen");
    assert_eq!(tome.fg_at(string, line(18)), STRING);
}

#[test]
fn files_without_a_grammar_are_uncoloured() {
    let tome = Tome::spawn(&["tests/fixtures/plain.txt"]);
    tome.wait_for_text("fn plain text", START);
    tome.wait_for_text("// slashes", WAIT);
    for (row, text) in [
        (line(1), "fn plain text is never coloured"),
        (line(2), "\"even with quotes\" and // slashes"),
    ] {
        let start = tome.text_col(row, text).expect("line on its row");
        let end = start + u16::try_from(text.len()).expect("short line");
        for col in start..end {
            assert_eq!(tome.fg_at(col, row), FG, "row {row}, column {col}");
        }
    }
}

#[test]
fn rust_retypes_on_edit() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("edit.rs");
    fs::write(&path, "fn main() {\n    let a = b + c;\n}\n").expect("write file");
    let mut tome = Tome::spawn_in(dir.path(), &["edit.rs"]);
    tome.wait_for_text("let a = b + c;", START);

    let row = line(2);
    let b = tome.text_col(row, "b + c").expect("line 2 on screen");
    let c = b + 4;
    let semicolon = b + 5;
    let keyword = tome.text_col(row, "let").expect("`let` on screen");
    tome.wait_for_fg_at(keyword, row, KEYWORD, WAIT);
    assert_eq!(tome.fg_at(c, row), FG);

    tome.click(b, row);
    tome.type_text("\"");
    tome.wait_for_text("let a = \"b + c;", WAIT);
    // Everything from the quote to the line's end now reads as a string.
    for col in b..=semicolon + 1 {
        tome.wait_for_fg_at(col, row, STRING, WAIT);
    }
    assert_eq!(tome.fg_at(keyword, row), KEYWORD);
}

#[test]
#[ignore = "timing bench; run with cargo test -- --ignored"]
fn bench_typing_10k_lines() {
    let sample = fs::read_to_string(SAMPLE).expect("read fixture");
    let mut text = String::new();
    while text.lines().count() < 10_000 {
        text.push_str(&sample);
    }
    let mut rope = Rope::from_str(&text);
    let mut highlighter = Highlighter::for_path(Path::new("big.rs")).expect("rust");
    highlighter.parse(&rope);

    // Type in the middle of the file and colour a 30-line screen there: the tree
    // edit, the reparse and the visible spans. `app::tests::
    // bench_typing_10k_lines_full_frame` times the same keystroke through `App`
    // and a whole drawn frame, which this test can't reach from outside the crate.
    let line = 5_000;
    let mut worst = Duration::ZERO;
    for i in 0..20 {
        let at = rope.line_to_char(line) + 4;
        let start = Instant::now();
        let edit = input_edit(&rope, at, 0, "x");
        rope.insert(at, "x");
        highlighter.edit(&edit);
        highlighter.parse(&rope);
        let bytes = rope.line_to_byte(line - 15)..rope.line_to_byte(line + 15);
        let spans = highlighter.spans(&rope, bytes);
        let took = start.elapsed();
        assert!(!spans.is_empty(), "keystroke {i} lost the colours");
        worst = worst.max(took);
    }
    println!("worst keystroke: {worst:?}");
    assert!(
        worst < Duration::from_millis(16),
        "a keystroke took {worst:?}"
    );
}
