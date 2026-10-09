//! Language servers: one per language per project root, started when a file of
//! that language opens and kept in sync with every open buffer of it. Messages
//! reach the app only as `AppEvent::Lsp` (ADR-0001), and a server that is missing
//! or dies only costs a status line: editing never waits on one.

mod client;
mod position;
pub mod servers;
mod transport;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lsp_types::{
    CompletionItemKind, CompletionResponse, CompletionTextEdit, DiagnosticSeverity,
    GotoDefinitionResponse, Hover, HoverContents, InsertTextFormat, MarkedString, MarkupKind,
    Position, PublishDiagnosticsParams, TextEdit, Uri,
};
use ropey::Rope;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::app::AppEvent;
use crate::buffer::Buffer;
use crate::config::LspServer;
use crate::highlight::languages;
use client::Client;
pub use position::char_index;
use position::{lsp_position, path_to_uri, uri_to_path};

/// Something one server did, tagged with the server it came from.
#[derive(Debug)]
pub struct LspEvent {
    pub server: u64,
    pub event: ServerEvent,
}

#[derive(Debug)]
pub enum ServerEvent {
    /// A JSON-RPC message from the server.
    Message(Value),
    /// The server's output closed and the process exited, with its code if any
    /// and the last lines it wrote to stderr, oldest first.
    Exited {
        code: Option<i32>,
        stderr: Vec<String>,
    },
}

/// Where the server following a buffer is, as the status line shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    /// Spawned; `initialize` isn't answered yet.
    Starting,
    Ready,
    /// The command couldn't be started, so there is no server.
    NotFound,
    /// It ran and then died, or refused to initialize.
    Crashed,
}

/// How bad a diagnostic is. A server that doesn't say is taken to mean an error,
/// as the protocol suggests clients do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

/// One problem a server reported in a buffer, by char indices into the text as
/// it was when the report arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: Range<usize>,
    pub severity: Severity,
    pub message: String,
}

/// Where F8 (`forward`) or Shift+F8 moves `cursor` among `diagnostics` (sorted
/// by start): the start of the next one after it, or the previous one before it,
/// wrapping around at either end. `None` when there are none.
pub fn diagnostic_jump(diagnostics: &[Diagnostic], cursor: usize, forward: bool) -> Option<usize> {
    let mut starts = diagnostics.iter().map(|d| d.range.start);
    if forward {
        let first = diagnostics.first()?.range.start;
        Some(starts.find(|&start| start > cursor).unwrap_or(first))
    } else {
        let last = diagnostics.last()?.range.start;
        Some(starts.rfind(|&start| start < cursor).unwrap_or(last))
    }
}

/// The diagnostic under `cursor`, the most severe when several overlap. A cursor
/// on an empty range's position counts as on it.
pub fn diagnostic_at(diagnostics: &[Diagnostic], cursor: usize) -> Option<&Diagnostic> {
    diagnostics
        .iter()
        .filter(|d| d.range.contains(&cursor) || d.range.start == cursor)
        .min_by_key(|d| d.severity)
}

/// A place a server pointed to: a file and an LSP position in it. The position
/// stays in protocol terms until the file is open and its text known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub position: Position,
}

/// The location a go to definition goes to: the first the server returned. `None`
/// for an empty or unreadable result.
fn first_location(result: &Value) -> Option<Location> {
    let (uri, position) = match serde_json::from_value(result.clone()).ok()? {
        GotoDefinitionResponse::Scalar(location) => (location.uri, location.range.start),
        GotoDefinitionResponse::Array(locations) => {
            let location = locations.into_iter().next()?;
            (location.uri, location.range.start)
        }
        GotoDefinitionResponse::Link(links) => {
            let link = links.into_iter().next()?;
            (link.target_uri, link.target_selection_range.start)
        }
    };
    Some(Location {
        path: uri_to_path(&uri)?,
        position,
    })
}

/// Whether `line` opens or closes a markdown code fence.
fn is_fence(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("```") || line.starts_with("~~~")
}

/// `markdown` with its code fence lines dropped, since the popup shows plain
/// text: the code inside them stays, the fence markers go.
fn strip_fences(markdown: &str) -> String {
    markdown
        .lines()
        .filter(|line| !is_fence(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether `line` is a markdown thematic break (`---`, `***`, `___`).
fn is_thematic_break(line: &str) -> bool {
    let marks: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3 && matches!(marks[0], '-' | '*' | '_') && marks.iter().all(|&c| c == marks[0])
}

/// `markdown` as (code, docs): the code inside the fences it opens with, then
/// the rest with its fences stripped. Servers put the signature in those leading
/// fences (rust-analyzer sends the module path and the signature as two, split
/// by blank lines), and the popup draws a rule under them. A thematic break
/// opening the docs goes: it is the server's own rule, and the popup has one.
fn split_fence(markdown: &str) -> (String, String) {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut at = 0;
    let mut blocks = Vec::new();
    loop {
        while lines.get(at).is_some_and(|line| line.trim().is_empty()) {
            at += 1;
        }
        if !lines.get(at).is_some_and(|line| is_fence(line)) {
            break;
        }
        let start = at + 1;
        let close = lines[start..]
            .iter()
            .position(|line| is_fence(line))
            .map_or(lines.len(), |offset| start + offset);
        blocks.push(lines[start..close].join("\n"));
        at = (close + 1).min(lines.len());
    }
    let mut rest = &lines[at..];
    if !blocks.is_empty() && rest.first().is_some_and(|line| is_thematic_break(line)) {
        rest = &rest[1..];
    }
    (blocks.join("\n\n"), strip_fences(&rest.join("\n")))
}

/// A hover reply as the popup shows it: the code it opens with (the signature)
/// and the docs after it, both plain text with fences stripped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HoverText {
    /// Empty when the reply doesn't open with code.
    pub code: String,
    /// Empty when the reply is only code.
    pub docs: String,
}

/// A hover reply split into code and docs, blank edges trimmed. `None` for an
/// empty or unreadable result, which shows nothing.
fn hover_text(result: &Value) -> Option<HoverText> {
    let hover: Hover = serde_json::from_value(result.clone()).ok()?;
    let marked = |marked: MarkedString| match marked {
        MarkedString::String(text) => split_fence(&text),
        MarkedString::LanguageString(code) => (code.value, String::new()),
    };
    let parts = match hover.contents {
        HoverContents::Scalar(one) => vec![marked(one)],
        HoverContents::Array(many) => many.into_iter().map(marked).collect(),
        HoverContents::Markup(markup) if markup.kind == MarkupKind::Markdown => {
            vec![split_fence(&markup.value)]
        }
        HoverContents::Markup(markup) => vec![(String::new(), markup.value)],
    };
    let mut code = Vec::new();
    let mut docs: Vec<String> = Vec::new();
    for (part_code, part_docs) in parts {
        // Code once the docs have begun is an example in them, not the signature.
        if docs.iter().any(|part| !part.trim().is_empty()) {
            docs.push(part_code);
        } else {
            code.push(part_code);
        }
        docs.push(part_docs);
    }
    let tidy = |parts: Vec<String>| {
        parts
            .iter()
            .map(|part| part.trim_matches(['\n', '\r']).trim_end())
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    let text = HoverText {
        code: tidy(code),
        docs: tidy(docs),
    };
    (!text.code.is_empty() || !text.docs.is_empty()).then_some(text)
}

/// One completion a server offered, with positions already in char indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    /// A short name for what it is (`fn`, `field`), or empty when the server
    /// didn't say.
    pub kind: &'static str,
    /// The server's one-line `detail` (often a type or signature), or empty.
    pub detail: String,
    /// What typing is matched against: the server's `filterText`, else the label.
    pub filter: String,
    /// What accepting it inserts, snippet syntax already flattened.
    pub text: String,
    /// The range `text` replaces, as char indices into the text the request was
    /// made against, when the server gave a `textEdit`. Without one, the word
    /// before the cursor is replaced.
    pub edit: Option<Range<usize>>,
}

/// The short name the popup shows for a completion's kind.
fn kind_label(kind: Option<CompletionItemKind>) -> &'static str {
    match kind {
        Some(CompletionItemKind::TEXT) => "text",
        Some(CompletionItemKind::METHOD) => "method",
        Some(CompletionItemKind::FUNCTION) => "fn",
        Some(CompletionItemKind::CONSTRUCTOR) => "ctor",
        Some(CompletionItemKind::FIELD) => "field",
        Some(CompletionItemKind::VARIABLE) => "var",
        Some(CompletionItemKind::CLASS) => "class",
        Some(CompletionItemKind::INTERFACE) => "iface",
        Some(CompletionItemKind::MODULE) => "mod",
        Some(CompletionItemKind::PROPERTY) => "prop",
        Some(CompletionItemKind::UNIT) => "unit",
        Some(CompletionItemKind::VALUE) => "value",
        Some(CompletionItemKind::ENUM) => "enum",
        Some(CompletionItemKind::KEYWORD) => "keyword",
        Some(CompletionItemKind::SNIPPET) => "snippet",
        Some(CompletionItemKind::COLOR) => "color",
        Some(CompletionItemKind::FILE) => "file",
        Some(CompletionItemKind::REFERENCE) => "ref",
        Some(CompletionItemKind::FOLDER) => "folder",
        Some(CompletionItemKind::ENUM_MEMBER) => "variant",
        Some(CompletionItemKind::CONSTANT) => "const",
        Some(CompletionItemKind::STRUCT) => "struct",
        Some(CompletionItemKind::EVENT) => "event",
        Some(CompletionItemKind::OPERATOR) => "op",
        Some(CompletionItemKind::TYPE_PARAMETER) => "type",
        _ => "",
    }
}

/// `snippet` as plain text: tab stops (`$1`, `$0`) and variables (`$TM_FILENAME`)
/// vanish, placeholders (`${1:name}`) keep their text, choices (`${1|a,b|}`) their
/// first option, and escaped `$`, `}` and `\` lose the backslash.
pub fn strip_snippet(snippet: &str) -> String {
    let chars: Vec<char> = snippet.chars().collect();
    let mut at = 0;
    let mut out = String::new();
    strip_into(&chars, &mut at, false, &mut out);
    out
}

/// Flattens `chars` from `at` into `out`, stopping after the `}` that closes the
/// placeholder when `nested`.
fn strip_into(chars: &[char], at: &mut usize, nested: bool, out: &mut String) {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    while let Some(&c) = chars.get(*at) {
        match c {
            '\\' if chars.get(*at + 1).is_some_and(|n| "$}\\,|".contains(*n)) => {
                out.push(chars[*at + 1]);
                *at += 2;
            }
            '}' if nested => {
                *at += 1;
                return;
            }
            '$' if chars.get(*at + 1).is_some_and(|&n| ident(n)) => {
                *at += 1;
                while chars.get(*at).is_some_and(|&n| ident(n)) {
                    *at += 1;
                }
            }
            '$' if chars.get(*at + 1) == Some(&'{') => {
                *at += 2;
                while chars.get(*at).is_some_and(|&n| ident(n)) {
                    *at += 1;
                }
                match chars.get(*at) {
                    Some(':') => {
                        *at += 1;
                        strip_into(chars, at, true, out);
                    }
                    Some('|') => {
                        *at += 1;
                        strip_choice(chars, at, out);
                    }
                    Some('}') => *at += 1,
                    // Not snippet syntax after all; what follows stays as text.
                    _ => {}
                }
            }
            _ => {
                out.push(c);
                *at += 1;
            }
        }
    }
}

/// The first option of a choice (`a,b|}`, after its opening `|`) into `out`,
/// moving `at` past the closing `|}`.
fn strip_choice(chars: &[char], at: &mut usize, out: &mut String) {
    let mut first = true;
    while let Some(&c) = chars.get(*at) {
        *at += 1;
        match c {
            '\\' => {
                if let Some(&next) = chars.get(*at) {
                    *at += 1;
                    if first {
                        out.push(next);
                    }
                }
            }
            ',' => first = false,
            '|' => break,
            _ if first => out.push(c),
            _ => {}
        }
    }
    if chars.get(*at) == Some(&'}') {
        *at += 1;
    }
}

/// A completion reply as items, in the server's `sortText` order (by label when
/// it gives none), with `textEdit` ranges read against `rope`, the text the
/// request was made against. An unreadable or empty reply gives no items.
fn completion_items(result: &Value, rope: &Rope) -> Vec<CompletionItem> {
    let items = match serde_json::from_value::<Option<CompletionResponse>>(result.clone()) {
        Ok(Some(CompletionResponse::Array(items))) => items,
        Ok(Some(CompletionResponse::List(list))) => list.items,
        Ok(None) | Err(_) => return Vec::new(),
    };
    let mut items: Vec<_> = items
        .into_iter()
        .map(|item| {
            let (edit, new_text) = match &item.text_edit {
                Some(CompletionTextEdit::Edit(edit)) => (Some(edit.range), Some(&edit.new_text)),
                Some(CompletionTextEdit::InsertAndReplace(edit)) => {
                    (Some(edit.insert), Some(&edit.new_text))
                }
                None => (None, None),
            };
            let raw = new_text
                .or(item.insert_text.as_ref())
                .unwrap_or(&item.label);
            let text = if item.insert_text_format == Some(InsertTextFormat::SNIPPET) {
                strip_snippet(raw)
            } else {
                raw.clone()
            };
            let edit = edit.map(|range| {
                let start = char_index(rope, range.start);
                start..char_index(rope, range.end).max(start)
            });
            let sort = item.sort_text.clone().unwrap_or_else(|| item.label.clone());
            let completion = CompletionItem {
                filter: item
                    .filter_text
                    .clone()
                    .unwrap_or_else(|| item.label.clone()),
                kind: kind_label(item.kind),
                // A row has one line for it, so a multi-line detail keeps its first.
                detail: item
                    .detail
                    .as_deref()
                    .and_then(|detail| detail.lines().next())
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                label: item.label,
                text,
                edit,
            };
            (sort, completion)
        })
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    items.into_iter().map(|(_, item)| item).collect()
}

/// What a server's message means for the app.
#[derive(Debug, PartialEq, Eq)]
pub enum LspNews {
    /// A line for the status line (a crash, a failed start).
    Message(String),
    /// The full set of diagnostics now standing for buffer `doc`, sorted by
    /// start; empty clears them.
    Diagnostics {
        doc: u64,
        diagnostics: Vec<Diagnostic>,
    },
    /// The answer to the latest go to definition: where to go, or `None` when
    /// the server found nothing.
    Definition(Option<Location>),
    /// The answer to the latest hover, or `None` when the server had nothing to
    /// say.
    Hover(Option<HoverText>),
    /// The answer to the latest completion request for buffer `doc`; empty when
    /// the server offered nothing.
    Completion {
        doc: u64,
        items: Vec<CompletionItem>,
    },
    /// The answer to the formatting request for buffer `doc`: edits as char
    /// ranges into the text the request was made against, sorted and not
    /// overlapping, or why there are none to apply.
    Formatted {
        doc: u64,
        edits: Result<Vec<(Range<usize>, String)>, String>,
    },
}

/// What asking to format a buffer before saving it did.
#[derive(Debug, PartialEq, Eq)]
pub enum FormatRequest {
    /// Its language doesn't have `format_on_save`, or it has no server: just save.
    Off,
    /// The request is out; the reply comes back from `handle` as
    /// `LspNews::Formatted`.
    Sent,
    /// It should be formatted but can't be now, and why.
    Unavailable(String),
}

/// A formatting reply as edits against `rope`, the text the request was made
/// against, in document order. `null` means nothing to change. Edits that
/// overlap can't be applied one after another, so they count as an error.
fn formatting_edits(message: &Value, rope: &Rope) -> Result<Vec<(Range<usize>, String)>, String> {
    if let Some(error) = message.get("error") {
        return Err(error["message"].as_str().unwrap_or("error").to_string());
    }
    let edits = serde_json::from_value::<Option<Vec<TextEdit>>>(message["result"].clone())
        .map_err(|_| "unreadable reply".to_string())?
        .unwrap_or_default();
    let mut edits: Vec<_> = edits
        .into_iter()
        .map(|edit| {
            let start = char_index(rope, edit.range.start);
            (
                start..char_index(rope, edit.range.end).max(start),
                edit.new_text,
            )
        })
        .collect();
    // Stable, so inserts at one position keep the order the server gave them.
    edits.sort_by_key(|(range, _)| range.start);
    if edits.windows(2).any(|pair| pair[0].0.end > pair[1].0.start) {
        return Err("overlapping edits".into());
    }
    Ok(edits)
}

/// A completion request awaiting its reply, with the text it was made against
/// so the reply's ranges read right even after more typing.
#[derive(Debug)]
struct PendingCompletion {
    server: u64,
    id: i64,
    doc: u64,
    rope: Rope,
}

/// A formatting request awaiting its reply, with the text it was made against.
#[derive(Debug)]
struct PendingFormat {
    server: u64,
    id: i64,
    doc: u64,
    rope: Rope,
}

/// The `[lsp.<lang>]` key for `path`: the highlight registry's language name, except
/// that `.jsx` files have their own key, as the spec's default servers list them.
pub fn language_for(path: &Path) -> Option<&'static str> {
    let name = languages::for_path(path)?.name;
    let jsx = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jsx"));
    Some(if name == "javascript" && jsx {
        "jsx"
    } else {
        name
    })
}

/// What the status line calls a server: its program's file name without
/// directory or Windows executable extension, so `C:\bin\rust-analyzer.exe`
/// reads `rust-analyzer`.
fn server_name(command: &str) -> String {
    // Either separator, since a config written on Windows may use backslashes
    // that `Path` on Unix wouldn't split.
    let file = command.rsplit(['/', '\\']).next().unwrap_or(command);
    let executable = |ext: &str| {
        ["exe", "cmd", "bat"]
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known))
    };
    match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && executable(ext) => stem,
        _ => file,
    }
    .to_string()
}

/// The protocol's language identifier for a config key; React flavours have their
/// own in the LSP spec.
fn protocol_language_id(lang: &str) -> &str {
    match lang {
        "tsx" => "typescriptreact",
        "jsx" => "javascriptreact",
        other => other,
    }
}

/// How long a server asked to stop gets to exit before it's killed.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// Waits up to `SHUTDOWN_GRACE` for stopping servers' tasks to end, then aborts
/// what's left: the reader owns the child process, and dropping it kills the
/// process (`kill_on_drop`), so a server that ignores `exit` doesn't linger.
async fn reap(tasks: Vec<JoinHandle<()>>) {
    let aborts: Vec<_> = tasks.iter().map(JoinHandle::abort_handle).collect();
    let _ = tokio::time::timeout(SHUTDOWN_GRACE, async {
        for task in tasks {
            let _ = task.await;
        }
    })
    .await;
    for abort in aborts {
        abort.abort();
    }
}

/// One buffer's link to its server, and what the server has been told about it.
#[derive(Debug)]
struct Attached {
    server: u64,
    uri: Uri,
    lang: &'static str,
    /// `didOpen` went out; until the server is ready it waits.
    opened: bool,
    version: i32,
    /// The text as the server last saw it, to work out the next change.
    rope: Rope,
    revision: u64,
    saves: u64,
}

/// Whether a language's server must restart for its table to go from `old` to
/// `new`: only what the process is started with counts.
fn needs_restart(old: Option<&LspServer>, new: Option<&LspServer>) -> bool {
    let start = |table: Option<&LspServer>| table.map(|t| (t.command.clone(), t.args.clone()));
    start(old) != start(new)
}

/// What stopping servers left behind.
#[derive(Debug, Default)]
pub struct Stopped {
    /// Buffers the stopped servers followed: what those servers said about
    /// them, such as diagnostics, no longer holds.
    pub docs: Vec<u64>,
    /// Ends once the servers have exited, killed if they had to be; `None`
    /// when there was nothing to wait for.
    pub exit: Option<JoinHandle<()>>,
}

/// Every language server Glyph has started, and which buffers each one follows.
#[derive(Debug, Default)]
pub struct Lsp {
    config: BTreeMap<String, LspServer>,
    events: Option<UnboundedSender<AppEvent>>,
    servers: HashMap<u64, Client>,
    by_root: HashMap<(&'static str, PathBuf), u64>,
    docs: HashMap<u64, Attached>,
    next_server: u64,
    /// The go to definition awaiting its reply, as (server, request id). A newer
    /// request replaces it, so a slow reply to an older one is ignored.
    definition: Option<(u64, i64)>,
    /// The hover awaiting its reply, likewise.
    hover: Option<(u64, i64)>,
    /// The completion awaiting its reply, likewise.
    completion: Option<PendingCompletion>,
    /// The formatting request a save is waiting on.
    format: Option<PendingFormat>,
    /// Why each language's server last failed after it was spawned, by language
    /// id. Kept past forgetting the server, so the reason outlives a retry until
    /// one starts.
    failures: HashMap<String, String>,
}

impl Lsp {
    pub fn new(config: BTreeMap<String, LspServer>) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    /// Servers can only start once there's an app channel for them to report on;
    /// until then (and in unit tests without one) nothing is started.
    pub fn connect(&mut self, events: UnboundedSender<AppEvent>) {
        self.events = Some(events);
    }

    /// Brings every server up to date with the open buffers, `docs` by id: starts
    /// servers for newly opened files, and sends opens, changes, saves and closes.
    /// Called after each event, so whatever changed a buffer, the server hears of
    /// it. Returns status lines for servers that couldn't start.
    pub fn sync(&mut self, root: &Path, docs: &[(u64, &Buffer)]) -> Vec<String> {
        let mut messages = Vec::new();
        if self.events.is_none() {
            return messages;
        }
        let live: HashSet<u64> = docs.iter().map(|(id, _)| *id).collect();
        let gone: Vec<u64> = self
            .docs
            .keys()
            .filter(|id| !live.contains(id))
            .copied()
            .collect();
        for id in gone {
            self.detach(id);
        }
        for &(id, buffer) in docs {
            let wanted = self.wanted(buffer);
            let same = matches!((&wanted, self.docs.get(&id)),
                (Some((lang, uri)), Some(doc)) if doc.lang == *lang && doc.uri == *uri);
            if !same {
                self.detach(id);
                if let Some((lang, uri)) = wanted
                    && let Some(server) = self.server_for(lang, root, &mut messages)
                {
                    self.docs.insert(
                        id,
                        Attached {
                            server,
                            uri,
                            lang,
                            opened: false,
                            version: 0,
                            rope: Rope::new(),
                            revision: 0,
                            saves: 0,
                        },
                    );
                }
            }
            self.update(id, buffer);
        }
        messages
    }

    /// The language and URI a buffer syncs as, when a server is configured for it.
    fn wanted(&self, buffer: &Buffer) -> Option<(&'static str, Uri)> {
        let path = buffer.path.as_deref()?;
        let lang = language_for(path)?;
        self.config.get(lang)?.command.as_ref()?;
        Some((lang, path_to_uri(path)?))
    }

    /// Sends what changed in `buffer` since its server last heard, once that server
    /// is ready.
    fn update(&mut self, id: u64, buffer: &Buffer) {
        let Some(doc) = self.docs.get_mut(&id) else {
            return;
        };
        let Some(client) = self.servers.get(&doc.server).filter(|c| c.is_ready()) else {
            return;
        };
        if !doc.opened {
            client.did_open(
                &doc.uri,
                protocol_language_id(doc.lang),
                doc.version,
                buffer.rope.to_string(),
            );
            doc.opened = true;
        } else {
            // A change made and then saved within one event still goes out first.
            if doc.revision != buffer.revision {
                doc.version += 1;
                client.did_change(&doc.uri, doc.version, &doc.rope, &buffer.rope);
            }
            if doc.saves != buffer.saves {
                client.did_save(&doc.uri);
            }
        }
        doc.rope = buffer.rope.clone();
        doc.revision = buffer.revision;
        doc.saves = buffer.saves;
    }

    /// Stops syncing buffer `id`, telling its server when it had been told of it.
    fn detach(&mut self, id: u64) {
        if let Some(doc) = self.docs.remove(&id)
            && doc.opened
            && let Some(client) = self.servers.get(&doc.server).filter(|c| c.is_ready())
        {
            client.did_close(&doc.uri);
        }
    }

    /// The state and display name of the server following buffer `doc`. `None`
    /// when no server is configured for its language, which the status line
    /// leaves out rather than calling missing.
    pub fn server_state(&self, doc: u64) -> Option<(ServerState, String)> {
        let attached = self.docs.get(&doc)?;
        let client = self.servers.get(&attached.server)?;
        let state = match client.state {
            client::State::Starting => ServerState::Starting,
            client::State::Ready { .. } => ServerState::Ready,
            client::State::Failed if client.spawned => ServerState::Crashed,
            client::State::Failed => ServerState::NotFound,
        };
        let command = self.config.get(attached.lang)?.command.as_deref()?;
        Some((state, server_name(command)))
    }

    /// Why the server for `lang` last failed: its `initialize` error, the last
    /// line it wrote to stderr before crashing, or its exit code. `None` when it
    /// hasn't failed, or has started since.
    pub fn failure(&self, lang: &str) -> Option<&str> {
        self.failures.get(lang).map(String::as_str)
    }

    /// Drops the remembered reason `lang`'s server failed, for a retry that has
    /// nothing to start yet: the next file opened tries it afresh.
    pub fn clear_failure(&mut self, lang: &str) {
        self.failures.remove(lang);
    }

    /// The server table for every language, as configured.
    pub fn config(&self) -> &BTreeMap<String, LspServer> {
        &self.config
    }

    /// Whether a server for `lang` is up (starting or ready) under any root.
    pub fn is_running(&self, lang: &str) -> bool {
        self.by_root.iter().any(|((l, _), id)| {
            *l == lang
                && self
                    .servers
                    .get(id)
                    .is_some_and(|c| c.state != client::State::Failed)
        })
    }

    /// The server for `lang` under `root`, starting it the first time. One that
    /// failed to start is remembered as failed, so it's reported once.
    fn server_for(
        &mut self,
        lang: &'static str,
        root: &Path,
        messages: &mut Vec<String>,
    ) -> Option<u64> {
        let key = (lang, root.to_path_buf());
        if let Some(&id) = self.by_root.get(&key) {
            return Some(id);
        }
        let config = self.config.get(lang)?;
        let command = config.command.clone()?;
        let events = self.events.clone()?;
        let id = self.next_server;
        self.next_server += 1;
        let program = servers::program(&command);
        let client = match transport::spawn(id, &program, &config.args, root, events) {
            Ok(connection) => {
                let name = root.file_name().map_or_else(
                    || root.display().to_string(),
                    |n| n.to_string_lossy().into(),
                );
                Client::start(
                    lang,
                    connection.outgoing,
                    connection.tasks,
                    path_to_uri(root),
                    &name,
                )
            }
            Err(err) => {
                messages.push(if err.kind() == io::ErrorKind::NotFound {
                    // Said where it's noticed, so the fix is one step away.
                    format!("{lang}: server not found ({command}) · >language servers installs it")
                } else {
                    format!("{lang}: cannot start server ({command}): {err}")
                });
                Client::failed(lang)
            }
        };
        self.servers.insert(id, client);
        self.by_root.insert(key, id);
        Some(id)
    }

    /// Asks the server following buffer `doc` where the symbol at char index
    /// `cursor` is defined; the answer comes back from `handle` as
    /// `LspNews::Definition`. `false` when no ready server follows the buffer.
    pub fn definition(&mut self, doc: u64, cursor: usize) -> bool {
        let sent = self.request_at(doc, cursor, Client::definition);
        self.definition = sent.or(self.definition);
        sent.is_some()
    }

    /// Asks the server following buffer `doc` about the symbol at char index
    /// `cursor`; the answer comes back from `handle` as `LspNews::Hover`. `false`
    /// when no ready server follows the buffer.
    pub fn hover(&mut self, doc: u64, cursor: usize) -> bool {
        let sent = self.request_at(doc, cursor, Client::hover);
        self.hover = sent.or(self.hover);
        sent.is_some()
    }

    /// Asks the server following buffer `doc` for completions at char index
    /// `cursor`; the answer comes back from `handle` as `LspNews::Completion`.
    /// `false` when no ready server follows the buffer.
    pub fn completion(&mut self, doc: u64, cursor: usize) -> bool {
        let Some((server, id)) = self.request_at(doc, cursor, Client::completion) else {
            return false;
        };
        let rope = self
            .docs
            .get(&doc)
            .map(|d| d.rope.clone())
            .unwrap_or_default();
        self.completion = Some(PendingCompletion {
            server,
            id,
            doc,
            rope,
        });
        true
    }

    /// Whether typing `ch` in buffer `doc` should open completion: it's one of
    /// the trigger characters of the ready server following the buffer.
    pub fn is_trigger(&self, doc: u64, ch: char) -> bool {
        let Some(attached) = self.docs.get(&doc).filter(|d| d.opened) else {
            return false;
        };
        let mut buf = [0; 4];
        let ch = &*ch.encode_utf8(&mut buf);
        self.servers
            .get(&attached.server)
            .filter(|c| c.is_ready())
            .is_some_and(|c| c.triggers.iter().any(|t| t == ch))
    }

    /// Asks the server following buffer `doc` to format it with the editor's
    /// indentation, when its language has `format_on_save`.
    pub fn format_on_save(
        &mut self,
        doc: u64,
        tab_width: usize,
        insert_spaces: bool,
    ) -> FormatRequest {
        let Some(attached) = self.docs.get(&doc) else {
            return FormatRequest::Off;
        };
        let lang = attached.lang;
        if !self.config.get(lang).is_some_and(|c| c.format_on_save) {
            return FormatRequest::Off;
        }
        let Some(client) = self
            .servers
            .get_mut(&attached.server)
            .filter(|c| c.is_ready() && attached.opened)
        else {
            return FormatRequest::Unavailable(format!("{lang} server not ready"));
        };
        if !client.formats {
            return FormatRequest::Unavailable(format!("{lang} server can't format"));
        }
        let tab_size = u32::try_from(tab_width).unwrap_or(u32::MAX);
        let id = client.formatting(&attached.uri, tab_size, insert_spaces);
        // The server's copy is the buffer's text: every event is followed by a
        // sync, so the reply's positions read against this.
        self.format = Some(PendingFormat {
            server: attached.server,
            id,
            doc,
            rope: attached.rope.clone(),
        });
        FormatRequest::Sent
    }

    /// Stops waiting for the formatting reply; one arriving later is ignored.
    pub fn cancel_format(&mut self) {
        self.format = None;
    }

    /// Sends the position request `send` makes for char index `cursor` in buffer
    /// `doc`. Returns (server, request id), or `None` with no ready server.
    fn request_at(
        &mut self,
        doc: u64,
        cursor: usize,
        send: fn(&mut Client, &Uri, Position) -> i64,
    ) -> Option<(u64, i64)> {
        let attached = self.docs.get(&doc).filter(|d| d.opened)?;
        let client = self
            .servers
            .get_mut(&attached.server)
            .filter(|c| c.is_ready())?;
        // The server's copy of the text is the buffer's: every event is followed
        // by a sync, so the position means the same to both.
        let position = lsp_position(&attached.rope, cursor);
        let id = send(client, &attached.uri, position);
        Some((attached.server, id))
    }

    /// Hands a server's message or exit to its client, turns published
    /// diagnostics into char ranges for each buffer they name, and passes on the
    /// answers to the latest go to definition, hover, completion and formatting.
    pub fn handle(&mut self, event: LspEvent) -> Vec<LspNews> {
        let completion = self.completion.as_ref().map(|p| (p.server, p.id));
        let format = self.format.as_ref().map(|p| (p.server, p.id));
        let reply = match &event.event {
            ServerEvent::Message(message) if message.get("method").is_none() => message["id"]
                .as_i64()
                .map(|id| (event.server, id))
                .filter(|reply| {
                    self.definition == Some(*reply)
                        || self.hover == Some(*reply)
                        || completion == Some(*reply)
                        || format == Some(*reply)
                }),
            _ => None,
        };
        let Some(client) = self.servers.get_mut(&event.server) else {
            return Vec::new();
        };
        if let Some(reply) = reply
            && let ServerEvent::Message(message) = event.event
        {
            let news = if self.definition == Some(reply) {
                self.definition = None;
                LspNews::Definition(first_location(&message["result"]))
            } else if format == Some(reply)
                && let Some(pending) = self.format.take()
            {
                LspNews::Formatted {
                    doc: pending.doc,
                    edits: formatting_edits(&message, &pending.rope),
                }
            } else if completion == Some(reply)
                && let Some(pending) = self.completion.take()
            {
                LspNews::Completion {
                    doc: pending.doc,
                    items: completion_items(&message["result"], &pending.rope),
                }
            } else {
                self.hover = None;
                LspNews::Hover(hover_text(&message["result"]))
            };
            client.handle(message);
            return vec![news];
        }
        let before = client.state;
        let message = match event.event {
            ServerEvent::Message(message)
                if message["method"] == "textDocument/publishDiagnostics"
                    && message.get("id").is_none() =>
            {
                return self.diagnostics(event.server, message);
            }
            ServerEvent::Message(message) => client.handle(message),
            ServerEvent::Exited { code, stderr } => client.exited(code, &stderr),
        };
        match client.state {
            client::State::Ready { .. } if !matches!(before, client::State::Ready { .. }) => {
                self.failures.remove(&client.lang);
            }
            client::State::Failed if before != client::State::Failed => {
                // No reason means Glyph stopped it: not a failure.
                if let Some(reason) = &client.reason {
                    self.failures.insert(client.lang.clone(), reason.clone());
                }
            }
            _ => {}
        }
        message.map(LspNews::Message).into_iter().collect()
    }

    /// The buffers `server` follows under the published URI, each with the
    /// diagnostics mapped onto its text. Positions are read against the text the
    /// server was last sent, which is the buffer's text: every event is followed
    /// by a sync before the next one is handled.
    fn diagnostics(&self, server: u64, message: Value) -> Vec<LspNews> {
        let Ok(params) =
            serde_json::from_value::<PublishDiagnosticsParams>(message["params"].clone())
        else {
            return Vec::new();
        };
        let mut news = Vec::new();
        for (&doc, attached) in &self.docs {
            if attached.server != server || attached.uri != params.uri {
                continue;
            }
            let mut diagnostics: Vec<Diagnostic> = params
                .diagnostics
                .iter()
                .map(|d| {
                    let start = char_index(&attached.rope, d.range.start);
                    let end = char_index(&attached.rope, d.range.end).max(start);
                    Diagnostic {
                        range: start..end,
                        severity: match d.severity {
                            Some(DiagnosticSeverity::WARNING) => Severity::Warning,
                            Some(DiagnosticSeverity::INFORMATION) => Severity::Information,
                            Some(DiagnosticSeverity::HINT) => Severity::Hint,
                            _ => Severity::Error,
                        },
                        message: d.message.clone(),
                    }
                })
                .collect();
            diagnostics.sort_by_key(|d| (d.range.start, d.range.end));
            news.push(LspNews::Diagnostics { doc, diagnostics });
        }
        news
    }

    /// Stops every server started for project folder `root` and forgets it, so
    /// the next file opened under that folder starts a fresh one. Buffers they
    /// follow are closed with them first. The wait for the processes to exit
    /// runs on a task of its own, so editing never waits on a server; the
    /// returned handle ends once they're gone, killed if they had to be. `None`
    /// when there was nothing to wait for.
    pub fn stop_root(&mut self, root: &Path) -> Option<JoinHandle<()>> {
        let stopped: HashSet<u64> = self
            .by_root
            .iter()
            .filter(|((_, r), _)| r == root)
            .map(|(_, &id)| id)
            .collect();
        self.stop(&stopped).exit
    }

    /// Takes `config` as the server tables from now on. Each language whose
    /// `command` or `args` changed has its servers stopped, in every root, as
    /// `stop_root` stops them; the next `sync` starts the new one for its open
    /// files. The rest keep their servers running: `format_on_save` and
    /// `install` are read from the config as they're needed, so they take
    /// effect without a restart.
    pub fn reconfigure(&mut self, config: BTreeMap<String, LspServer>) -> Stopped {
        let changed: HashSet<&str> = self
            .config
            .keys()
            .chain(config.keys())
            .filter(|lang| needs_restart(self.config.get(*lang), config.get(*lang)))
            .map(String::as_str)
            .collect();
        let stopped: HashSet<u64> = self
            .by_root
            .iter()
            .filter(|((lang, _), _)| changed.contains(lang))
            .map(|(_, &id)| id)
            .collect();
        self.config = config;
        self.stop(&stopped)
    }

    /// Stops servers `stopped` and forgets them, closing the buffers they
    /// follow first.
    fn stop(&mut self, stopped: &HashSet<u64>) -> Stopped {
        if stopped.is_empty() {
            return Stopped::default();
        }
        self.by_root.retain(|_, id| !stopped.contains(id));
        let docs: Vec<u64> = self
            .docs
            .iter()
            .filter(|(_, doc)| stopped.contains(&doc.server))
            .map(|(&id, _)| id)
            .collect();
        // Before the shutdown, so the server hears every close on the same queue.
        for &id in &docs {
            self.detach(id);
        }
        // A reply from a stopped server would name a buffer it no longer follows.
        let from_stopped =
            |pending: Option<(u64, i64)>| pending.filter(|(server, _)| !stopped.contains(server));
        self.definition = from_stopped(self.definition);
        self.hover = from_stopped(self.hover);
        if self
            .completion
            .as_ref()
            .is_some_and(|p| stopped.contains(&p.server))
        {
            self.completion = None;
        }
        if self
            .format
            .as_ref()
            .is_some_and(|p| stopped.contains(&p.server))
        {
            self.format = None;
        }
        let tasks: Vec<JoinHandle<()>> = stopped
            .iter()
            .filter_map(|id| self.servers.remove(id))
            .flat_map(|mut client| client.shutdown())
            .collect();
        let exit = (!tasks.is_empty()).then(|| tokio::spawn(reap(tasks)));
        Stopped { docs, exit }
    }

    /// Forgets every server for `lang` that is failed (never found, or crashed),
    /// in every root, and detaches the buffers that followed them, so the next
    /// `sync` tries starting it again: a server installed since then starts
    /// without reopening the project. Running servers are left alone. Returns
    /// whether anything was forgotten.
    pub fn forget_failed(&mut self, lang: &str) -> bool {
        let failed: HashSet<u64> = self
            .by_root
            .iter()
            .filter(|((l, _), id)| {
                *l == lang
                    && self
                        .servers
                        .get(id)
                        .is_some_and(|c| c.state == client::State::Failed)
            })
            .map(|(_, &id)| id)
            .collect();
        if failed.is_empty() {
            return false;
        }
        self.by_root.retain(|_, id| !failed.contains(id));
        // `sync` only re-attaches buffers that aren't attached, so these must go
        // for the retry to happen. A failed server isn't ready, so no close is sent.
        let docs: Vec<u64> = self
            .docs
            .iter()
            .filter(|(_, doc)| failed.contains(&doc.server))
            .map(|(&id, _)| id)
            .collect();
        for id in docs {
            self.detach(id);
        }
        // A crashed server may still have had a request out; its reply can't come.
        let live = |pending: Option<(u64, i64)>| pending.filter(|(s, _)| !failed.contains(s));
        self.definition = live(self.definition);
        self.hover = live(self.hover);
        if self
            .completion
            .as_ref()
            .is_some_and(|p| failed.contains(&p.server))
        {
            self.completion = None;
        }
        if self
            .format
            .as_ref()
            .is_some_and(|p| failed.contains(&p.server))
        {
            self.format = None;
        }
        for id in &failed {
            // One that refused to initialize may still be running; aborting the
            // reader drops the child, which kills it (`kill_on_drop`). Nothing is
            // waited on, so editing never stalls here.
            if let Some(mut client) = self.servers.remove(id) {
                for task in client.shutdown() {
                    task.abort();
                }
            }
        }
        true
    }

    /// Asks every server to stop and gives them a moment to go; whatever is
    /// still running after that is killed.
    pub async fn finish(&mut self) {
        let tasks: Vec<_> = self
            .servers
            .values_mut()
            .flat_map(Client::shutdown)
            .collect();
        reap(tasks).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn config(command: &str) -> BTreeMap<String, LspServer> {
        BTreeMap::from([(
            "rust".to_string(),
            LspServer {
                command: Some(command.into()),
                ..LspServer::default()
            },
        )])
    }

    fn rust_buffer(dir: &Path, name: &str) -> Buffer {
        Buffer {
            path: Some(dir.join(name)),
            ..Buffer::empty()
        }
    }

    #[test]
    fn clearing_a_failure_forgets_only_that_languages_reason() {
        let mut lsp = Lsp::new(config("glyph-no-such-server"));
        lsp.failures.insert("rust".into(), "exit code 3".into());
        lsp.failures.insert("go".into(), "no gopls".into());
        lsp.clear_failure("rust");
        assert_eq!(lsp.failure("rust"), None);
        assert_eq!(lsp.failure("go"), Some("no gopls"));
    }

    #[test]
    fn languages_are_keyed_by_registry_name_with_jsx_apart() {
        assert_eq!(language_for(Path::new("a.rs")), Some("rust"));
        assert_eq!(language_for(Path::new("a.js")), Some("javascript"));
        assert_eq!(language_for(Path::new("a.JSX")), Some("jsx"));
        assert_eq!(language_for(Path::new("a.tsx")), Some("tsx"));
        assert_eq!(language_for(Path::new("a.txt")), None);
        assert_eq!(protocol_language_id("tsx"), "typescriptreact");
        assert_eq!(protocol_language_id("rust"), "rust");
    }

    #[tokio::test]
    async fn a_missing_server_is_reported_once_for_every_file_of_its_language() {
        let dir = tempfile::tempdir().unwrap();
        let mut lsp = Lsp::new(config("glyph-no-such-server"));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        lsp.connect(tx);
        let (a, b) = (
            rust_buffer(dir.path(), "a.rs"),
            rust_buffer(dir.path(), "b.rs"),
        );
        let notes = Buffer {
            path: Some(dir.path().join("notes.txt")),
            ..Buffer::empty()
        };

        let first = lsp.sync(dir.path(), &[(1, &a), (2, &notes)]);
        assert_eq!(
            first,
            ["rust: server not found (glyph-no-such-server) · >language servers installs it"]
        );
        assert!(
            lsp.sync(dir.path(), &[(1, &a), (2, &notes), (3, &b)])
                .is_empty()
        );
        assert_eq!(lsp.servers.len(), 1);
        // Plain text has no server and isn't followed.
        assert!(!lsp.docs.contains_key(&2));
        assert_eq!(
            lsp.server_state(3),
            Some((ServerState::NotFound, "glyph-no-such-server".to_string()))
        );
        assert_eq!(lsp.server_state(2), None);
    }

    #[test]
    fn a_server_is_named_by_its_program_without_directory_or_exe() {
        assert_eq!(server_name("rust-analyzer"), "rust-analyzer");
        assert_eq!(server_name(r"C:\tools\rust-analyzer.EXE"), "rust-analyzer");
        assert_eq!(server_name("/usr/bin/gopls"), "gopls");
        assert_eq!(
            server_name("bin/pyright-langserver.cmd"),
            "pyright-langserver"
        );
        assert_eq!(
            server_name("vscode-css-language-server"),
            "vscode-css-language-server"
        );
    }

    fn diag(range: Range<usize>, severity: Severity) -> Diagnostic {
        Diagnostic {
            range,
            severity,
            message: String::new(),
        }
    }

    #[test]
    fn jumps_go_to_the_next_or_previous_start_and_wrap() {
        let list = [diag(2..4, Severity::Warning), diag(10..12, Severity::Error)];
        assert_eq!(diagnostic_jump(&list, 0, true), Some(2));
        assert_eq!(diagnostic_jump(&list, 2, true), Some(10));
        assert_eq!(diagnostic_jump(&list, 3, true), Some(10));
        assert_eq!(diagnostic_jump(&list, 10, true), Some(2));
        assert_eq!(diagnostic_jump(&list, 11, false), Some(10));
        assert_eq!(diagnostic_jump(&list, 10, false), Some(2));
        assert_eq!(diagnostic_jump(&list, 2, false), Some(10));
        assert_eq!(diagnostic_jump(&[], 5, true), None);
        assert_eq!(diagnostic_jump(&[], 5, false), None);
    }

    #[test]
    fn the_diagnostic_at_the_cursor_is_the_worst_one_covering_it() {
        let list = [
            diag(2..8, Severity::Warning),
            diag(4..6, Severity::Error),
            diag(9..9, Severity::Hint),
        ];
        assert_eq!(diagnostic_at(&list, 1), None);
        assert_eq!(
            diagnostic_at(&list, 2).map(|d| d.severity),
            Some(Severity::Warning)
        );
        assert_eq!(
            diagnostic_at(&list, 5).map(|d| d.severity),
            Some(Severity::Error)
        );
        assert_eq!(diagnostic_at(&list, 8), None);
        assert_eq!(
            diagnostic_at(&list, 9).map(|d| d.severity),
            Some(Severity::Hint)
        );
    }

    #[test]
    fn a_publish_becomes_sorted_char_ranges_for_the_buffer_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut lsp = Lsp::new(config("ra"));
        let uri = path_to_uri(&dir.path().join("a.rs")).unwrap();
        lsp.docs.insert(
            7,
            Attached {
                server: 1,
                uri: uri.clone(),
                lang: "rust",
                opened: true,
                version: 0,
                rope: Rope::from_str("é🦀 x\nyy\n"),
                revision: 0,
                saves: 0,
            },
        );
        let message = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {"uri": uri.as_str(), "diagnostics": [
                {"range": {"start": {"line": 1, "character": 0},
                           "end": {"line": 1, "character": 2}},
                 "message": "no severity"},
                {"range": {"start": {"line": 0, "character": 4},
                           "end": {"line": 0, "character": 5}},
                 "severity": 2, "message": "after the crab"}
            ]}
        });
        let news = lsp.diagnostics(1, message.clone());
        assert_eq!(
            news,
            [LspNews::Diagnostics {
                doc: 7,
                diagnostics: vec![
                    Diagnostic {
                        range: 3..4,
                        severity: Severity::Warning,
                        message: "after the crab".into(),
                    },
                    Diagnostic {
                        range: 5..7,
                        severity: Severity::Error,
                        message: "no severity".into(),
                    },
                ],
            }]
        );
        // Another server's report on the same file isn't this one's.
        assert!(lsp.diagnostics(2, message).is_empty());
    }

    #[test]
    fn the_first_definition_location_wins_in_any_result_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::path::absolute(dir.path().join("b.rs")).unwrap();
        let uri = path_to_uri(&path).unwrap();
        let range = serde_json::json!({"start": {"line": 2, "character": 4},
                                       "end": {"line": 2, "character": 9}});
        let expected = Some(Location {
            path: path.clone(),
            position: Position::new(2, 4),
        });
        let location = serde_json::json!({"uri": uri.as_str(), "range": range});
        assert_eq!(first_location(&location), expected);
        let other = serde_json::json!({"uri": "file:///elsewhere.rs", "range": range});
        assert_eq!(
            first_location(&serde_json::json!([location, other])),
            expected
        );
        let link = serde_json::json!([{
            "targetUri": uri.as_str(),
            "targetRange": {"start": {"line": 0, "character": 0},
                            "end": {"line": 5, "character": 0}},
            "targetSelectionRange": range,
        }]);
        assert_eq!(first_location(&link), expected);
        assert_eq!(first_location(&Value::Null), None);
        assert_eq!(first_location(&serde_json::json!([])), None);
    }

    #[test]
    fn hover_text_is_plain_with_fences_stripped_in_any_result_shape() {
        let text = |code: &str, docs: &str| {
            Some(HoverText {
                code: code.into(),
                docs: docs.into(),
            })
        };
        let markdown = serde_json::json!({"contents": {"kind": "markdown",
            "value": "```rust\nfn greet()\n```\n\nSays hello."}});
        assert_eq!(hover_text(&markdown), text("fn greet()", "Says hello."));
        let plain = serde_json::json!({"contents": {"kind": "plaintext", "value": "```x```"}});
        assert_eq!(hover_text(&plain), text("", "```x```"));
        let marked = serde_json::json!({"contents": [
            {"language": "rust", "value": "fn a()"}, "", "docs"
        ]});
        assert_eq!(hover_text(&marked), text("fn a()", "docs"));
        let scalar = serde_json::json!({"contents": "a"});
        assert_eq!(hover_text(&scalar), text("", "a"));
        // Code after the docs is an example in them; a fence not first is docs.
        let example = serde_json::json!({"contents": {"kind": "markdown",
            "value": "Adds.\n\n```rust\nadd(1)\n```"}});
        assert_eq!(hover_text(&example), text("", "Adds.\n\nadd(1)"));
        let late = serde_json::json!({"contents": ["docs", {"language": "rust", "value": "f()"}]});
        assert_eq!(hover_text(&late), text("", "docs\n\nf()"));
        assert_eq!(hover_text(&Value::Null), None);
        assert_eq!(hover_text(&serde_json::json!({"contents": ""})), None);
        assert_eq!(hover_text(&serde_json::json!({"contents": []})), None);
        let fences = serde_json::json!({"contents": {"kind": "markdown", "value": "```\n```"}});
        assert_eq!(hover_text(&fences), None);
        // rust-analyzer: module path and signature as two fences, then `---`.
        let rust_analyzer = serde_json::json!({"contents": {"kind": "markdown",
            "value": "```rust\nmy_crate\n```\n\n```rust\npub fn greet()\n```\n\n---\n\nSays hello."}});
        assert_eq!(
            hover_text(&rust_analyzer),
            text("my_crate\n\npub fn greet()", "Says hello.")
        );
        // pyright: the break straight under the fence; a later break stays.
        let pyright = serde_json::json!({"contents": {"kind": "markdown",
            "value": "```python\ndef f()\n```\n---\nDoes.\n\n***\n\nMore."}});
        assert_eq!(
            hover_text(&pyright),
            text("def f()", "Does.\n\n***\n\nMore.")
        );
    }

    #[test]
    fn snippets_flatten_to_their_placeholder_text() {
        assert_eq!(strip_snippet("push(${1:ch})$0"), "push(ch)");
        assert_eq!(
            strip_snippet("fn ${1:name}(${2:args}) {\n\t$0\n}"),
            "fn name(args) {\n\t\n}"
        );
        assert_eq!(strip_snippet("${1:outer ${2:inner}}"), "outer inner");
        assert_eq!(strip_snippet("${1|one,two|}"), "one");
        assert_eq!(strip_snippet("$TM_FILENAME ${TM_SELECTED_TEXT:x}"), " x");
        assert_eq!(strip_snippet(r"a \$1 \} \\ b"), r"a $1 } \ b");
        assert_eq!(strip_snippet("${}"), "");
        assert_eq!(strip_snippet("cost $ 5"), "cost $ 5");
        assert_eq!(strip_snippet("plain"), "plain");
    }

    #[test]
    fn completion_items_insert_text_edit_then_insert_text_then_label() {
        let rope = Rope::from_str("fn x() {\n    s.le\n}\n");
        let result = serde_json::json!({"isIncomplete": false, "items": [
            {"label": "len", "kind": 2, "sortText": "b",
             "textEdit": {"range": {"start": {"line": 1, "character": 6},
                                    "end": {"line": 1, "character": 8}}, "newText": "len()"},
             "insertText": "ignored"},
            {"label": "push", "kind": 3, "sortText": "a", "insertTextFormat": 2,
             "insertText": "push(${1:ch})$0", "filterText": "pu"},
            {"label": "trim", "sortText": "c"}
        ]});
        let items = completion_items(&result, &rope);
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["push", "len", "trim"]);
        assert_eq!(items[0].text, "push(ch)");
        assert_eq!((items[0].kind, items[0].filter.as_str()), ("fn", "pu"));
        assert_eq!(items[0].edit, None);
        assert_eq!(items[1].text, "len()");
        assert_eq!(items[1].kind, "method");
        assert_eq!(items[1].edit, Some(15..17));
        assert_eq!((items[2].text.as_str(), items[2].kind), ("trim", ""));
        // A bare array works too; null and junk give nothing.
        let array = serde_json::json!([{"label": "a"}]);
        assert_eq!(completion_items(&array, &rope).len(), 1);
        assert!(completion_items(&Value::Null, &rope).is_empty());
        assert!(completion_items(&serde_json::json!(7), &rope).is_empty());
    }

    #[test]
    fn formatting_edits_are_sorted_char_ranges_and_errors_say_why() {
        let rope = Rope::from_str("é x\nfn  a(){}\n");
        let edit = |l1, c1, l2, c2, text: &str| {
            serde_json::json!({"range": {"start": {"line": l1, "character": c1},
                                         "end": {"line": l2, "character": c2}},
                               "newText": text})
        };
        let reply = serde_json::json!({"id": 1, "result": [
            edit(1, 7, 1, 7, " "), edit(0, 1, 0, 2, ""), edit(1, 2, 1, 4, " ")
        ]});
        assert_eq!(
            formatting_edits(&reply, &rope),
            Ok(vec![
                (1..2, String::new()),
                (6..8, " ".into()),
                (11..11, " ".into())
            ])
        );
        let none = serde_json::json!({"id": 1, "result": null});
        assert_eq!(formatting_edits(&none, &rope), Ok(Vec::new()));
        let error = serde_json::json!({"id": 1, "error": {"code": -32603, "message": "boom"}});
        assert_eq!(formatting_edits(&error, &rope), Err("boom".into()));
        let overlap = serde_json::json!({"id": 1, "result": [
            edit(1, 0, 1, 4, ""), edit(1, 2, 1, 6, "")
        ]});
        assert!(formatting_edits(&overlap, &rope).is_err());
    }

    #[test]
    fn format_on_save_is_off_unless_the_language_asks() {
        let dir = tempfile::tempdir().unwrap();
        let mut lsp = Lsp::new(config("ra"));
        lsp.docs.insert(
            1,
            Attached {
                server: 0,
                uri: path_to_uri(&dir.path().join("a.rs")).unwrap(),
                lang: "rust",
                opened: true,
                version: 0,
                rope: Rope::new(),
                revision: 0,
                saves: 0,
            },
        );
        assert_eq!(lsp.format_on_save(1, 4, true), FormatRequest::Off);
        assert_eq!(lsp.format_on_save(2, 4, true), FormatRequest::Off);
        if let Some(rust) = lsp.config.get_mut("rust") {
            rust.format_on_save = true;
        }
        assert_eq!(
            lsp.format_on_save(1, 4, true),
            FormatRequest::Unavailable("rust server not ready".into())
        );
    }

    #[test]
    fn without_an_app_channel_nothing_starts() {
        let dir = tempfile::tempdir().unwrap();
        let mut lsp = Lsp::new(config("glyph-no-such-server"));
        let a = rust_buffer(dir.path(), "a.rs");
        assert!(lsp.sync(dir.path(), &[(1, &a)]).is_empty());
        assert!(lsp.servers.is_empty());
    }

    #[test]
    fn a_table_without_a_command_selects_no_server() {
        let lsp = Lsp::new(BTreeMap::from([(
            "rust".to_string(),
            LspServer {
                command: None,
                ..LspServer::default()
            },
        )]));
        let dir = tempfile::tempdir().unwrap();
        assert!(lsp.wanted(&rust_buffer(dir.path(), "a.rs")).is_none());
        let lsp = Lsp::new(config("ra"));
        let (lang, uri) = lsp.wanted(&rust_buffer(dir.path(), "a.rs")).unwrap();
        assert_eq!(lang, "rust");
        assert!(uri.as_str().ends_with("/a.rs"));
        assert!(Uri::from_str(uri.as_str()).is_ok());
    }

    /// The scripted fake server. Cargo builds it beside this binary's `deps`
    /// folder whenever it builds the integration tests, which `cargo test` does.
    fn fake_lsp() -> PathBuf {
        let exe = std::env::current_exe().unwrap();
        let dir = exe.parent().and_then(Path::parent).unwrap();
        let path = dir.join(format!("fake_lsp{}", std::env::consts::EXE_SUFFIX));
        assert!(
            path.exists(),
            "no {}: run `cargo build --bins`",
            path.display()
        );
        path
    }

    /// An `Lsp` whose Rust server is the fake, logging every message it gets to
    /// `log.jsonl` and following `script` when there is one. The fake takes both
    /// as arguments because a server config can't set its environment.
    struct Fake {
        lsp: Lsp,
        events: tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
        files: tempfile::TempDir,
    }

    impl Fake {
        fn new(script: Option<&str>) -> Self {
            let files = tempfile::tempdir().unwrap();
            let log = files.path().join("log.jsonl");
            let mut args = vec!["--log".to_string(), log.display().to_string()];
            if let Some(script) = script {
                let path = files.path().join("script.json");
                std::fs::write(&path, script).unwrap();
                args.extend(["--script".to_string(), path.display().to_string()]);
            }
            let mut lsp = Lsp::new(BTreeMap::from([(
                "rust".to_string(),
                LspServer {
                    command: Some(fake_lsp().display().to_string()),
                    args,
                    ..LspServer::default()
                },
            )]));
            let (tx, events) = tokio::sync::mpsc::unbounded_channel();
            lsp.connect(tx);
            Self { lsp, events, files }
        }

        /// Syncs `docs` and handles server events, as the app loop does, until
        /// `done` holds.
        async fn run_until(
            &mut self,
            root: &Path,
            docs: &[(u64, &Buffer)],
            done: impl Fn(&Lsp) -> bool,
        ) {
            let waited = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    self.lsp.sync(root, docs);
                    if done(&self.lsp) {
                        return;
                    }
                    match self.events.recv().await {
                        Some(AppEvent::Lsp(event)) => {
                            self.lsp.handle(event);
                        }
                        Some(_) => {}
                        None => panic!("the app channel closed"),
                    }
                }
            })
            .await;
            assert!(waited.is_ok(), "timed out waiting on the fake server");
        }

        /// Every logged message, as (pid, message).
        fn log(&self) -> Vec<(u64, Value)> {
            let text =
                std::fs::read_to_string(self.files.path().join("log.jsonl")).unwrap_or_default();
            text.lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .map(|e| (e["pid"].as_u64().unwrap_or(0), e["message"].clone()))
                .collect()
        }

        /// The exit codes reported for `server` and not yet handled.
        fn exits(&mut self, server: u64) -> Vec<Option<i32>> {
            let mut codes = Vec::new();
            while let Ok(event) = self.events.try_recv() {
                if let AppEvent::Lsp(LspEvent {
                    server: from,
                    event: ServerEvent::Exited { code, .. },
                }) = event
                    && from == server
                {
                    codes.push(code);
                }
            }
            codes
        }
    }

    /// Spawns the fake server straight through the transport with `script`,
    /// sends `initialize`, and waits for its exit report: (code, stderr lines),
    /// with the connection.
    async fn spawn_until_exit(script: Value) -> (Option<i32>, Vec<String>, transport::Connection) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("script.json");
        std::fs::write(&path, script.to_string()).unwrap();
        let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let args = ["--script".to_string(), path.display().to_string()];
        let connection = transport::spawn(7, &fake_lsp(), &args, root.path(), tx).unwrap();
        connection
            .outgoing
            .send(serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
            .unwrap();
        let (code, stderr) = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Some(AppEvent::Lsp(LspEvent {
                        server: 7,
                        event: ServerEvent::Exited { code, stderr },
                    })) => return (code, stderr),
                    Some(_) => {}
                    None => panic!("the app channel closed"),
                }
            }
        })
        .await
        .expect("the fake server to exit");
        (code, stderr, connection)
    }

    #[tokio::test]
    async fn spawn_keeps_the_last_lines_a_server_writes_to_stderr() {
        let lines: Vec<String> = (1..=25).map(|n| format!("line {n}")).collect();
        let script = serde_json::json!({"stderr": {"initialize": lines}, "exit_on": "initialize"});
        let (code, stderr, _) = spawn_until_exit(script).await;
        assert_eq!(code, Some(3));
        assert_eq!(stderr, lines[5..]);
        assert_eq!(stderr.len(), transport::STDERR_LINES);
    }

    // Unix only: a Windows child inherits every inheritable handle, the
    // server's stdout pipe included, so the exit itself would never be seen.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stderr_held_open_by_a_child_is_let_go_after_the_exit() {
        // The child holds stderr for 20 s; the reading task must let go long
        // before.
        let script = serde_json::json!({"hold_stderr": 20000,
            "stderr": {"initialize": ["giving up"]}, "exit_on": "initialize"});
        let (code, stderr, mut connection) = spawn_until_exit(script).await;
        assert_eq!(code, Some(3));
        assert_eq!(stderr, ["giving up"]);
        // The stderr task is the connection's last, so stopping the server
        // reaches it too; by now it's been aborted rather than left reading.
        assert_eq!(connection.tasks.len(), 3);
        let reading = connection.tasks.pop().unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(5), reading).await;
        assert!(
            ended
                .expect("the stderr task to end")
                .is_err_and(|e| e.is_cancelled()),
            "it ended by being aborted"
        );
    }

    #[tokio::test]
    async fn a_failure_reason_is_remembered_per_language_until_a_start_succeeds() {
        let root = tempfile::tempdir().unwrap();
        let crashed = |lsp: &Lsp| {
            lsp.server_state(1)
                .is_some_and(|(state, _)| state == ServerState::Crashed)
        };

        // Crashed, saying why on stderr.
        let mut fake = Fake::new(Some(
            r#"{"stderr": {"initialize": ["warming up", "tsserver not found", ""]},
                "exit_on": "initialize"}"#,
        ));
        let a = rust_buffer(root.path(), "a.rs");
        assert_eq!(fake.lsp.failure("rust"), None);
        fake.run_until(root.path(), &[(1, &a)], crashed).await;
        assert_eq!(fake.lsp.failure("rust"), Some("tsserver not found"));
        assert_eq!(fake.lsp.failure("python"), None);

        // Crashed in silence: its exit code.
        let script = r#"{"exit_on": "initialize"}"#;
        set_server(&mut fake, "rust", &fake_lsp(), Some(script));
        assert!(fake.lsp.forget_failed("rust"));
        // Forgetting a server to retry it keeps the reason until a start works.
        assert_eq!(fake.lsp.failure("rust"), Some("tsserver not found"));
        fake.run_until(root.path(), &[(1, &a)], crashed).await;
        assert_eq!(fake.lsp.failure("rust"), Some("exit code 3"));

        // Refused to initialize: its error's message.
        let script = r#"{"errors": {"initialize": {"code": -32603, "message": "no tsserver"}}}"#;
        set_server(&mut fake, "rust", &fake_lsp(), Some(script));
        assert!(fake.lsp.forget_failed("rust"));
        fake.run_until(root.path(), &[(1, &a)], crashed).await;
        assert_eq!(fake.lsp.failure("rust"), Some("no tsserver"));

        // Started: the reason is gone.
        set_server(&mut fake, "rust", &fake_lsp(), None);
        assert!(fake.lsp.forget_failed("rust"));
        fake.run_until(root.path(), &[(1, &a)], opened(1)).await;
        assert_eq!(fake.lsp.failure("rust"), None);

        // Stopping it is no failure.
        reaped(fake.lsp.stop_root(root.path())).await;
        assert_eq!(fake.lsp.failure("rust"), None);
    }

    async fn reaped(handle: Option<JoinHandle<()>>) {
        let handle = handle.expect("servers to wait for");
        let done = tokio::time::timeout(Duration::from_secs(5), handle).await;
        assert!(done.is_ok(), "stopping servers took too long");
    }

    fn opened(doc: u64) -> impl Fn(&Lsp) -> bool {
        move |lsp| lsp.docs.get(&doc).is_some_and(|d| d.opened)
    }

    fn methods(log: &[(u64, Value)], pid: u64) -> Vec<String> {
        log.iter()
            .filter(|(p, _)| *p == pid)
            .filter_map(|(_, m)| m["method"].as_str().map(String::from))
            .collect()
    }

    fn pid_of(log: &[(u64, Value)], file: &str) -> u64 {
        log.iter()
            .find(|(_, m)| {
                m["method"] == "textDocument/didOpen"
                    && m["params"]["textDocument"]["uri"]
                        .as_str()
                        .is_some_and(|uri| uri.ends_with(&format!("/{file}")))
            })
            .map(|(pid, _)| *pid)
            .unwrap_or_else(|| panic!("no didOpen for {file}"))
    }

    #[tokio::test]
    async fn stopping_a_root_closes_its_files_shuts_its_server_down_and_forgets_it() {
        let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mut fake = Fake::new(None);
        let a = rust_buffer(one.path(), "a.rs");
        fake.run_until(one.path(), &[(1, &a)], opened(1)).await;
        let first = fake.lsp.docs[&1].server;

        reaped(fake.lsp.stop_root(one.path())).await;
        assert!(fake.lsp.by_root.is_empty());
        assert!(fake.lsp.servers.is_empty());
        assert!(fake.lsp.docs.is_empty());
        // It went because it was told to, not because it was killed.
        assert_eq!(fake.exits(first), [Some(0)]);
        let log = fake.log();
        let sent = methods(&log, pid_of(&log, "a.rs"));
        assert_eq!(
            sent[sent.len() - 3..],
            ["textDocument/didClose", "shutdown", "exit"]
        );

        // The same language in another folder gets a server of its own.
        let b = rust_buffer(two.path(), "b.rs");
        fake.run_until(two.path(), &[(2, &b)], opened(2)).await;
        let second = fake.lsp.docs[&2].server;
        assert_ne!(second, first);
        assert_eq!(
            fake.lsp.by_root[&("rust", two.path().to_path_buf())],
            second
        );
        reaped(fake.lsp.stop_root(two.path())).await;
        let log = fake.log();
        assert_ne!(pid_of(&log, "b.rs"), pid_of(&log, "a.rs"));
    }

    #[tokio::test]
    async fn stopping_one_root_leaves_another_running() {
        let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mut fake = Fake::new(None);
        let (a, b) = (
            rust_buffer(one.path(), "a.rs"),
            rust_buffer(two.path(), "b.rs"),
        );
        fake.run_until(one.path(), &[(1, &a)], opened(1)).await;
        fake.run_until(two.path(), &[(1, &a), (2, &b)], opened(2))
            .await;

        reaped(fake.lsp.stop_root(one.path())).await;
        assert!(!fake.lsp.docs.contains_key(&1));
        let other = fake.lsp.docs[&2].server;
        assert_eq!(fake.lsp.servers.len(), 1);
        assert!(fake.lsp.servers[&other].is_ready());
        assert!(fake.exits(other).is_empty());
        reaped(fake.lsp.stop_root(two.path())).await;
    }

    #[tokio::test]
    async fn a_server_that_ignores_exit_is_killed() {
        let root = tempfile::tempdir().unwrap();
        // Busy for a minute on `shutdown`, so it never reads `exit`.
        let mut fake = Fake::new(Some(r#"{"delay": {"shutdown": 60000}}"#));
        let a = rust_buffer(root.path(), "a.rs");
        fake.run_until(root.path(), &[(1, &a)], opened(1)).await;
        let server = fake.lsp.docs[&1].server;

        reaped(fake.lsp.stop_root(root.path())).await;
        assert!(fake.lsp.servers.is_empty());
        // Killed with its reader, so no exit was ever reported.
        assert!(fake.exits(server).is_empty());
    }

    #[test]
    fn stopping_a_root_without_servers_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut lsp = Lsp::new(config("glyph-no-such-server"));
        // No runtime here: nothing is spawned for a root with no servers.
        assert!(lsp.stop_root(dir.path()).is_none());
    }

    #[tokio::test]
    async fn stopping_a_missing_or_crashed_server_never_blocks_editing() {
        let root = tempfile::tempdir().unwrap();
        let mut lsp = Lsp::new(config("glyph-no-such-server"));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        lsp.connect(tx);
        let a = rust_buffer(root.path(), "a.rs");
        lsp.sync(root.path(), &[(1, &a)]);
        // A server that never started has no process to wait on.
        assert!(lsp.stop_root(root.path()).is_none());
        assert!(lsp.by_root.is_empty() && lsp.docs.is_empty());

        let mut fake = Fake::new(Some(r#"{"exit_on": "initialize"}"#));
        let mut a = rust_buffer(root.path(), "a.rs");
        fake.run_until(root.path(), &[(1, &a)], |lsp| {
            lsp.server_state(1)
                .is_some_and(|(state, _)| state == ServerState::Crashed)
        })
        .await;
        let crashed = fake.lsp.stop_root(root.path());
        assert!(fake.lsp.by_root.is_empty() && fake.lsp.servers.is_empty());
        // Editing carries on at once; any wait on the dead server is elsewhere.
        a.type_text("x");
        assert!(fake.lsp.sync(root.path(), &[]).is_empty());
        if let Some(handle) = crashed {
            reaped(Some(handle)).await;
        }
    }

    /// Points `fake`'s server for `lang` at `command`, keeping its log and adding
    /// `script` when given, as a config change between syncs would.
    fn set_server(fake: &mut Fake, lang: &str, command: &Path, script: Option<&str>) {
        let mut args = vec![
            "--log".to_string(),
            fake.files.path().join("log.jsonl").display().to_string(),
        ];
        if let Some(script) = script {
            let path = fake.files.path().join(format!("{lang}-script.json"));
            std::fs::write(&path, script).unwrap();
            args.extend(["--script".to_string(), path.display().to_string()]);
        }
        fake.lsp.config.insert(
            lang.to_string(),
            LspServer {
                command: Some(command.display().to_string()),
                args,
                ..LspServer::default()
            },
        );
    }

    fn sorted(ids: impl Iterator<Item = u64>) -> Vec<u64> {
        let mut ids: Vec<u64> = ids.collect();
        ids.sort_unstable();
        ids
    }

    #[tokio::test]
    async fn forgetting_failed_servers_drops_every_failed_one_of_the_language_only() {
        let roots: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
        let (one, two, three) = (roots[0].path(), roots[1].path(), roots[2].path());
        let missing = one.join("glyph-no-such-server");
        let mut fake = Fake::new(None);
        let a = rust_buffer(one, "a.rs");
        fake.run_until(one, &[(1, &a)], opened(1)).await;
        let running = fake.lsp.docs[&1].server;

        // Not found under `two`, for Rust and for Python.
        let b = rust_buffer(two, "b.rs");
        let py = Buffer {
            path: Some(two.join("d.py")),
            ..Buffer::empty()
        };
        set_server(&mut fake, "rust", &missing, None);
        set_server(&mut fake, "python", &missing, None);
        fake.lsp.sync(two, &[(1, &a), (2, &b), (4, &py)]);
        assert_eq!(fake.lsp.server_state(2).unwrap().0, ServerState::NotFound);
        assert_eq!(fake.lsp.server_state(4).unwrap().0, ServerState::NotFound);
        let python = fake.lsp.docs[&4].server;

        // Crashed under `three`.
        let c = rust_buffer(three, "c.rs");
        set_server(
            &mut fake,
            "rust",
            &fake_lsp(),
            Some(r#"{"exit_on": "initialize"}"#),
        );
        let docs = [(1, &a), (2, &b), (3, &c), (4, &py)];
        fake.run_until(three, &docs, |lsp| {
            lsp.server_state(3)
                .is_some_and(|(state, _)| state == ServerState::Crashed)
        })
        .await;

        assert!(fake.lsp.forget_failed("rust"));
        assert_eq!(
            sorted(fake.lsp.by_root.values().copied()),
            [running, python]
        );
        assert_eq!(sorted(fake.lsp.servers.keys().copied()), [running, python]);
        assert_eq!(sorted(fake.lsp.docs.keys().copied()), [1, 4]);
        // The running server never heard a thing about it.
        assert_eq!(fake.lsp.docs[&1].server, running);
        assert!(fake.lsp.servers[&running].is_ready());
        assert!(fake.exits(running).is_empty());
        reaped(fake.lsp.stop_root(one)).await;
        let log = fake.log();
        let sent = methods(&log, pid_of(&log, "a.rs"));
        // Its only close is the one stopping the root sent.
        assert_eq!(
            sent.iter()
                .filter(|m| *m == "textDocument/didClose")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn reconfiguring_restarts_only_for_a_new_command_or_args() {
        let root = tempfile::tempdir().unwrap();
        let mut fake = Fake::new(None);
        let a = rust_buffer(root.path(), "a.rs");
        fake.run_until(root.path(), &[(1, &a)], opened(1)).await;
        let server = fake.lsp.docs[&1].server;

        // Format on save and install are taken as they are, server and all.
        let mut config = fake.lsp.config.clone();
        let rust = config.get_mut("rust").unwrap();
        rust.format_on_save = true;
        rust.install = Some("cargo install it".to_string());
        let stopped = fake.lsp.reconfigure(config.clone());
        assert!(stopped.docs.is_empty() && stopped.exit.is_none());
        assert_eq!(fake.lsp.config, config);
        assert_eq!(fake.lsp.docs[&1].server, server);
        assert!(fake.lsp.servers[&server].is_ready());

        // New args stop it, and the buffer it followed is handed back.
        config
            .get_mut("rust")
            .unwrap()
            .args
            .push("--new".to_string());
        let stopped = fake.lsp.reconfigure(config.clone());
        assert_eq!(stopped.docs, [1]);
        reaped(stopped.exit).await;
        assert!(!fake.lsp.servers.contains_key(&server));
        assert!(!fake.lsp.docs.contains_key(&1));
        fake.run_until(root.path(), &[(1, &a)], opened(1)).await;
        assert_ne!(fake.lsp.docs[&1].server, server);
        reaped(fake.lsp.stop_root(root.path())).await;
    }

    #[test]
    fn only_the_command_and_args_need_a_restart() {
        let table = |command: &str, args: &[&str]| LspServer {
            command: Some(command.to_string()),
            args: args.iter().map(|a| a.to_string()).collect(),
            ..LspServer::default()
        };
        let old = table("ra", &["--a"]);
        let mut same = old.clone();
        same.format_on_save = true;
        same.install = Some("x".to_string());
        assert!(!needs_restart(Some(&old), Some(&same)));
        assert!(!needs_restart(None, None));
        assert!(needs_restart(Some(&old), Some(&table("ra", &["--b"]))));
        assert!(needs_restart(Some(&old), Some(&table("rb", &["--a"]))));
        assert!(needs_restart(Some(&old), None));
        assert!(needs_restart(None, Some(&old)));
    }

    #[tokio::test]
    async fn forgetting_with_nothing_failed_does_nothing() {
        let root = tempfile::tempdir().unwrap();
        let mut empty = Lsp::new(config("glyph-no-such-server"));
        assert!(!empty.forget_failed("rust"));

        let mut fake = Fake::new(None);
        let a = rust_buffer(root.path(), "a.rs");
        fake.run_until(root.path(), &[(1, &a)], opened(1)).await;
        let server = fake.lsp.docs[&1].server;
        assert!(!fake.lsp.forget_failed("rust"));
        assert!(!fake.lsp.forget_failed("python"));
        assert_eq!(fake.lsp.by_root.len(), 1);
        assert_eq!(sorted(fake.lsp.servers.keys().copied()), [server]);
        assert!(fake.lsp.docs[&1].opened && fake.lsp.docs[&1].server == server);
        assert!(fake.lsp.servers[&server].is_ready());
        reaped(fake.lsp.stop_root(root.path())).await;
    }

    #[tokio::test]
    async fn only_a_server_that_is_up_counts_as_running() {
        let dir = tempfile::tempdir().unwrap();
        let mut missing = Lsp::new(config("glyph-no-such-server"));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        missing.connect(tx);
        assert!(!missing.is_running("rust"));
        let a = rust_buffer(dir.path(), "a.rs");
        missing.sync(dir.path(), &[(1, &a)]);
        // It was tried and failed: not running.
        assert!(!missing.is_running("rust"));

        let mut fake = Fake::new(None);
        fake.run_until(dir.path(), &[(1, &a)], opened(1)).await;
        assert!(fake.lsp.is_running("rust"));
        assert!(!fake.lsp.is_running("python"));
        reaped(fake.lsp.stop_root(dir.path())).await;
        assert!(!fake.lsp.is_running("rust"));
    }

    #[tokio::test]
    async fn a_server_installed_after_it_was_missing_starts_once_forgotten() {
        let root = tempfile::tempdir().unwrap();
        let mut fake = Fake::new(None);
        // Where the server will be installed; nothing is there yet.
        let installed = fake
            .files
            .path()
            .join(format!("late-server{}", std::env::consts::EXE_SUFFIX));
        set_server(&mut fake, "rust", &installed, None);
        let a = rust_buffer(root.path(), "a.rs");
        let messages = fake.lsp.sync(root.path(), &[(1, &a)]);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].starts_with("rust: server not found"));

        // On Linux a copy is a file open for writing, and a child another test
        // thread forks meanwhile inherits that descriptor until it execs; running
        // the copy then fails with ETXTBSY. A symlink is never open for writing.
        #[cfg(unix)]
        std::os::unix::fs::symlink(fake_lsp(), &installed).unwrap();
        #[cfg(not(unix))]
        std::fs::copy(fake_lsp(), &installed).unwrap();
        // Still remembered as missing until it's forgotten.
        assert!(fake.lsp.sync(root.path(), &[(1, &a)]).is_empty());
        assert_eq!(fake.lsp.server_state(1).unwrap().0, ServerState::NotFound);

        assert!(fake.lsp.forget_failed("rust"));
        fake.run_until(root.path(), &[(1, &a)], opened(1)).await;
        assert_eq!(fake.lsp.server_state(1).unwrap().0, ServerState::Ready);
        // Once it has exited, everything it was sent is in the log.
        reaped(fake.lsp.stop_root(root.path())).await;
        let log = fake.log();
        let sent = methods(&log, pid_of(&log, "a.rs"));
        assert!(sent.iter().any(|m| m == "textDocument/didOpen"));
    }
}
