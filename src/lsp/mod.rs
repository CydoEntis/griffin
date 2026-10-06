//! Language servers: one per language per project root, started when a file of
//! that language opens and kept in sync with every open buffer of it. Messages
//! reach the app only as `AppEvent::Lsp` (ADR-0001), and a server that is missing
//! or dies only costs a status line: editing never waits on one.

mod client;
mod position;
mod transport;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lsp_types::{
    DiagnosticSeverity, GotoDefinitionResponse, Hover, HoverContents, MarkedString, MarkupKind,
    Position, PublishDiagnosticsParams, Uri,
};
use ropey::Rope;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

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
    /// The server's output closed and the process exited, with its code if any.
    Exited(Option<i32>),
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

/// `markdown` with its code fence lines dropped, since the popup shows plain
/// text: the code inside them stays, the fence markers go.
fn strip_fences(markdown: &str) -> String {
    markdown
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            !line.starts_with("```") && !line.starts_with("~~~")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A hover reply as plain text, fences stripped and blank edges trimmed. `None`
/// for an empty or unreadable result, which shows nothing.
fn hover_text(result: &Value) -> Option<String> {
    let hover: Hover = serde_json::from_value(result.clone()).ok()?;
    let marked = |marked: MarkedString| match marked {
        MarkedString::String(text) => strip_fences(&text),
        MarkedString::LanguageString(code) => code.value,
    };
    let text = match hover.contents {
        HoverContents::Scalar(one) => marked(one),
        HoverContents::Array(many) => many
            .into_iter()
            .map(marked)
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        HoverContents::Markup(markup) if markup.kind == MarkupKind::Markdown => {
            strip_fences(&markup.value)
        }
        HoverContents::Markup(markup) => markup.value,
    };
    let text = text.trim_matches(['\n', '\r']).trim_end();
    (!text.trim().is_empty()).then(|| text.to_string())
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
    /// The answer to the latest hover, as plain text, or `None` when the server
    /// had nothing to say.
    Hover(Option<String>),
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

/// The protocol's language identifier for a config key; React flavours have their
/// own in the LSP spec.
fn protocol_language_id(lang: &str) -> &str {
    match lang {
        "tsx" => "typescriptreact",
        "jsx" => "javascriptreact",
        other => other,
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

/// Every language server Griffin has started, and which buffers each one follows.
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
        let client = match transport::spawn(id, &command, &config.args, root, events) {
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
                    format!("{lang}: server not found ({command})")
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
    /// answers to the latest go to definition and hover.
    pub fn handle(&mut self, event: LspEvent) -> Vec<LspNews> {
        let reply = match &event.event {
            ServerEvent::Message(message) if message.get("method").is_none() => message["id"]
                .as_i64()
                .map(|id| (event.server, id))
                .filter(|reply| self.definition == Some(*reply) || self.hover == Some(*reply)),
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
            } else {
                self.hover = None;
                LspNews::Hover(hover_text(&message["result"]))
            };
            client.handle(message);
            return vec![news];
        }
        let message = match event.event {
            ServerEvent::Message(message)
                if message["method"] == "textDocument/publishDiagnostics"
                    && message.get("id").is_none() =>
            {
                return self.diagnostics(event.server, message);
            }
            ServerEvent::Message(message) => client.handle(message),
            ServerEvent::Exited(code) => client.exited(code),
        };
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

    /// Asks every server to stop and gives them a moment to go. Whatever is still
    /// running after that is killed as the runtime drops it.
    pub async fn finish(&mut self) {
        let tasks: Vec<_> = self
            .servers
            .values_mut()
            .flat_map(Client::shutdown)
            .collect();
        let _ = tokio::time::timeout(Duration::from_millis(500), async {
            for task in tasks {
                let _ = task.await;
            }
        })
        .await;
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
                args: Vec::new(),
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
        let mut lsp = Lsp::new(config("griffin-no-such-server"));
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
        assert_eq!(first, ["rust: server not found (griffin-no-such-server)"]);
        assert!(
            lsp.sync(dir.path(), &[(1, &a), (2, &notes), (3, &b)])
                .is_empty()
        );
        assert_eq!(lsp.servers.len(), 1);
        // Plain text has no server and isn't followed.
        assert!(!lsp.docs.contains_key(&2));
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
        let markdown = serde_json::json!({"contents": {"kind": "markdown",
            "value": "```rust\nfn greet()\n```\n\nSays hello."}});
        assert_eq!(
            hover_text(&markdown).as_deref(),
            Some("fn greet()\n\nSays hello.")
        );
        let plain = serde_json::json!({"contents": {"kind": "plaintext", "value": "```x```"}});
        assert_eq!(hover_text(&plain).as_deref(), Some("```x```"));
        let marked = serde_json::json!({"contents": [
            {"language": "rust", "value": "fn a()"}, "", "docs"
        ]});
        assert_eq!(hover_text(&marked).as_deref(), Some("fn a()\n\ndocs"));
        let scalar = serde_json::json!({"contents": "a"});
        assert_eq!(hover_text(&scalar).as_deref(), Some("a"));
        assert_eq!(hover_text(&Value::Null), None);
        assert_eq!(hover_text(&serde_json::json!({"contents": ""})), None);
        assert_eq!(hover_text(&serde_json::json!({"contents": []})), None);
        let fences = serde_json::json!({"contents": {"kind": "markdown", "value": "```\n```"}});
        assert_eq!(hover_text(&fences), None);
    }

    #[test]
    fn without_an_app_channel_nothing_starts() {
        let dir = tempfile::tempdir().unwrap();
        let mut lsp = Lsp::new(config("griffin-no-such-server"));
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
                args: Vec::new(),
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
}
