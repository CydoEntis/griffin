//! One language server, seen from Griffin: its lifecycle, request ids, and the
//! document notifications it is sent. Nothing here waits on the server; replies
//! arrive later through `handle` (ADR-0001).

use std::collections::HashMap;

use lsp_types::{
    ClientCapabilities, ClientInfo, CompletionClientCapabilities, CompletionItemCapability,
    CompletionParams, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, GotoCapability, GotoDefinitionParams,
    HoverClientCapabilities, HoverParams, InitializeParams, MarkupKind, PartialResultParams,
    Position, TextDocumentClientCapabilities, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams,
    TextDocumentSyncCapability, TextDocumentSyncClientCapabilities, TextDocumentSyncKind, Uri,
    VersionedTextDocumentIdentifier, WorkDoneProgressParams, WorkspaceFolder,
};
use ropey::Rope;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use super::position::changed_region;

/// Where a server is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// `initialize` is out; documents wait until it's answered.
    Starting,
    /// Initialized; documents sync the way the server asked.
    Ready { sync: TextDocumentSyncKind },
    /// Never started, crashed, or shut down. Editing goes on without it.
    Failed,
}

#[derive(Debug)]
pub struct Client {
    /// The language id from config, for status messages.
    pub lang: String,
    pub state: State,
    outgoing: Option<UnboundedSender<Value>>,
    /// The connection's writer and reader, waited on briefly at quit.
    tasks: Vec<JoinHandle<()>>,
    next_id: i64,
    /// Requests sent and not yet answered, by id, with their method.
    pending: HashMap<i64, String>,
    /// Griffin asked it to stop, so its exit is no news.
    shutting_down: bool,
    /// The characters that open completion when typed, from the server's
    /// `completionProvider`; empty until it's initialized, or when it has none.
    pub triggers: Vec<String>,
}

/// An lsp-types value as JSON. These types serialize infallibly; `Null` stands in
/// should one ever not.
fn to_json(value: impl Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

impl Client {
    /// A server that never started: it stays failed, so it isn't tried again for
    /// every file and the status line says why only once.
    pub fn failed(lang: &str) -> Self {
        Self::new(lang, None, Vec::new())
    }

    fn new(
        lang: &str,
        outgoing: Option<UnboundedSender<Value>>,
        tasks: Vec<JoinHandle<()>>,
    ) -> Self {
        Self {
            lang: lang.to_string(),
            state: if outgoing.is_some() {
                State::Starting
            } else {
                State::Failed
            },
            outgoing,
            tasks,
            next_id: 1,
            pending: HashMap::new(),
            shutting_down: false,
            triggers: Vec::new(),
        }
    }

    /// A freshly spawned server: sends `initialize` for the project at `root`.
    pub fn start(
        lang: &str,
        outgoing: UnboundedSender<Value>,
        tasks: Vec<JoinHandle<()>>,
        root: Option<Uri>,
        root_name: &str,
    ) -> Self {
        let mut client = Self::new(lang, Some(outgoing), tasks);
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            capabilities: ClientCapabilities {
                text_document: Some(TextDocumentClientCapabilities {
                    synchronization: Some(TextDocumentSyncClientCapabilities {
                        did_save: Some(true),
                        ..Default::default()
                    }),
                    definition: Some(GotoCapability {
                        dynamic_registration: None,
                        link_support: Some(true),
                    }),
                    // Plain text first: the popup shows text, not markdown styles.
                    hover: Some(HoverClientCapabilities {
                        dynamic_registration: None,
                        content_format: Some(vec![MarkupKind::PlainText, MarkupKind::Markdown]),
                    }),
                    // No snippets: tab stops aren't supported, so plain insert
                    // text is what Griffin wants (snippets sent anyway are
                    // flattened).
                    completion: Some(CompletionClientCapabilities {
                        completion_item: Some(CompletionItemCapability {
                            snippet_support: Some(false),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            workspace_folders: root.clone().map(|uri| {
                vec![WorkspaceFolder {
                    uri,
                    name: root_name.to_string(),
                }]
            }),
            client_info: Some(ClientInfo {
                name: "griffin".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            ..Default::default()
        };
        let mut params = to_json(params);
        // Older servers only read `rootUri`; lsp-types marks the field deprecated,
        // so it is set on the JSON instead.
        if let Some(uri) = root {
            params["rootUri"] = json!(uri.as_str());
        }
        client.request("initialize", params);
        client
    }

    fn send(&self, message: Value) {
        if let Some(outgoing) = &self.outgoing {
            // A closed queue means the writer saw the server go; its exit event
            // says so.
            let _ = outgoing.send(message);
        }
    }

    fn request(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, method.to_string());
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    fn notify(&self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.state, State::Ready { .. })
    }

    /// Reacts to one message from the server. Returns a line for the status line
    /// when there's news worth one.
    pub fn handle(&mut self, message: Value) -> Option<String> {
        let id = message.get("id").cloned();
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned);
        match (id, method) {
            (Some(id), Some(method)) => {
                self.answer(id, &method, &message);
                None
            }
            (Some(id), None) => {
                let method = self.pending.remove(&id.as_i64()?)?;
                if method == "initialize" {
                    return self.initialized(&message);
                }
                None
            }
            // Notifications (logs, progress) mean nothing here; diagnostics never
            // reach a client, `Lsp` maps them onto buffers.
            (None, _) => None,
        }
    }

    /// A request from the server. Each gets an empty answer, since some servers
    /// stall until theirs is answered; `workspace/configuration` expects one entry
    /// per item asked for.
    fn answer(&self, id: Value, method: &str, message: &Value) {
        let result = if method == "workspace/configuration" {
            let items = message["params"]["items"].as_array().map_or(0, Vec::len);
            Value::Array(vec![Value::Null; items])
        } else {
            Value::Null
        };
        self.send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    fn initialized(&mut self, response: &Value) -> Option<String> {
        if let Some(error) = response.get("error") {
            self.state = State::Failed;
            let reason = error["message"].as_str().unwrap_or("error");
            return Some(format!("{}: server failed to start ({reason})", self.lang));
        }
        let sync = serde_json::from_value::<TextDocumentSyncCapability>(
            response["result"]["capabilities"]["textDocumentSync"].clone(),
        )
        .ok()
        .map_or(TextDocumentSyncKind::NONE, |capability| match capability {
            TextDocumentSyncCapability::Kind(kind) => kind,
            TextDocumentSyncCapability::Options(options) => {
                options.change.unwrap_or(TextDocumentSyncKind::NONE)
            }
        });
        self.state = State::Ready { sync };
        self.triggers =
            response["result"]["capabilities"]["completionProvider"]["triggerCharacters"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .filter(|t| !t.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
        self.notify("initialized", json!({}));
        None
    }

    /// The process is gone. Unless Griffin stopped it, that's a crash worth a
    /// status line.
    pub fn exited(&mut self, code: Option<i32>) -> Option<String> {
        let was_failed = self.state == State::Failed;
        self.state = State::Failed;
        self.outgoing = None;
        if self.shutting_down || was_failed {
            return None;
        }
        Some(match code {
            Some(code) => format!("{}: server crashed (exit code {code})", self.lang),
            None => format!("{}: server crashed", self.lang),
        })
    }

    /// Asks the server to stop. The replies aren't waited for; the writer task
    /// ends after sending `exit`, the reader once the process is gone. Returns
    /// both, for a bounded wait at quit.
    pub fn shutdown(&mut self) -> Vec<JoinHandle<()>> {
        if self.state != State::Failed && !self.shutting_down {
            self.shutting_down = true;
            self.request("shutdown", Value::Null);
            self.notify("exit", Value::Null);
        }
        self.outgoing = None;
        std::mem::take(&mut self.tasks)
    }

    pub fn did_open(&self, uri: &Uri, language_id: &str, version: i32, text: String) {
        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: language_id.to_string(),
                version,
                text,
            },
        };
        self.notify("textDocument/didOpen", to_json(params));
    }

    /// Sends the change from `old` to `new` as the server asked: the whole text,
    /// or just the region that differs. A server that wants no changes gets none.
    pub fn did_change(&self, uri: &Uri, version: i32, old: &Rope, new: &Rope) {
        let State::Ready { sync } = self.state else {
            return;
        };
        let change = if sync == TextDocumentSyncKind::FULL {
            TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: new.to_string(),
            }
        } else if sync == TextDocumentSyncKind::INCREMENTAL {
            let Some((range, text)) = changed_region(old, new) else {
                return;
            };
            TextDocumentContentChangeEvent {
                range: Some(range),
                range_length: None,
                text,
            }
        } else {
            return;
        };
        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version,
            },
            content_changes: vec![change],
        };
        self.notify("textDocument/didChange", to_json(params));
    }

    pub fn did_save(&self, uri: &Uri) {
        let params = DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            text: None,
        };
        self.notify("textDocument/didSave", to_json(params));
    }

    /// Asks where the symbol at `position` in `uri` is defined. Returns the
    /// request's id, which the reply will carry.
    pub fn definition(&mut self, uri: &Uri, position: Position) -> i64 {
        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };
        self.request("textDocument/definition", to_json(params))
    }

    /// Asks for hover information on the symbol at `position` in `uri`. Returns
    /// the request's id, which the reply will carry.
    pub fn hover(&mut self, uri: &Uri, position: Position) -> i64 {
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };
        self.request("textDocument/hover", to_json(params))
    }

    /// Asks for completions at `position` in `uri`. Returns the request's id,
    /// which the reply will carry.
    pub fn completion(&mut self, uri: &Uri, position: Position) -> i64 {
        let params = CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            context: None,
        };
        self.request("textDocument/completion", to_json(params))
    }

    pub fn did_close(&self, uri: &Uri) {
        let params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
        };
        self.notify("textDocument/didClose", to_json(params));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use tokio::sync::mpsc::{self, UnboundedReceiver};

    fn started() -> (Client, UnboundedReceiver<Value>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let root = Uri::from_str("file:///project").ok();
        (Client::start("rust", tx, Vec::new(), root, "project"), rx)
    }

    fn drain(rx: &mut UnboundedReceiver<Value>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(message) = rx.try_recv() {
            out.push(message);
        }
        out
    }

    fn ready(sync: Value) -> (Client, UnboundedReceiver<Value>) {
        let (mut client, mut rx) = started();
        let init = drain(&mut rx);
        let id = init[0]["id"].clone();
        let reply = json!({"jsonrpc": "2.0", "id": id,
            "result": {"capabilities": {"textDocumentSync": sync}}});
        assert_eq!(client.handle(reply), None);
        assert_eq!(drain(&mut rx)[0]["method"], "initialized");
        (client, rx)
    }

    fn uri() -> Uri {
        Uri::from_str("file:///project/main.rs").unwrap()
    }

    #[test]
    fn start_sends_initialize_with_the_root_and_waits() {
        let (client, mut rx) = started();
        let sent = drain(&mut rx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["method"], "initialize");
        assert_eq!(sent[0]["id"], 1);
        assert_eq!(sent[0]["params"]["rootUri"], "file:///project");
        assert_eq!(
            sent[0]["params"]["workspaceFolders"][0]["uri"],
            "file:///project"
        );
        assert_eq!(client.state, State::Starting);
    }

    #[test]
    fn the_initialize_reply_sets_the_sync_kind_from_a_number_or_options() {
        let (client, _) = ready(json!(1));
        assert_eq!(
            client.state,
            State::Ready {
                sync: TextDocumentSyncKind::FULL
            }
        );
        let (client, _) = ready(json!({"openClose": true, "change": 2}));
        assert_eq!(
            client.state,
            State::Ready {
                sync: TextDocumentSyncKind::INCREMENTAL
            }
        );
        let (client, _) = ready(Value::Null);
        assert_eq!(
            client.state,
            State::Ready {
                sync: TextDocumentSyncKind::NONE
            }
        );
    }

    #[test]
    fn changes_are_full_or_incremental_as_advertised() {
        let old = Rope::from_str("let a = 1;\n");
        let new = Rope::from_str("let ab = 1;\n");

        let (client, mut rx) = ready(json!(1));
        client.did_change(&uri(), 2, &old, &new);
        let sent = drain(&mut rx);
        let change = &sent[0]["params"]["contentChanges"][0];
        assert_eq!(change["text"], "let ab = 1;\n");
        assert!(change.get("range").is_none(), "{change}");
        assert_eq!(sent[0]["params"]["textDocument"]["version"], 2);

        let (client, mut rx) = ready(json!(2));
        client.did_change(&uri(), 2, &old, &new);
        let sent = drain(&mut rx);
        let change = &sent[0]["params"]["contentChanges"][0];
        assert_eq!(change["text"], "b");
        assert_eq!(change["range"]["start"], json!({"line": 0, "character": 5}));
        assert_eq!(change["range"]["end"], json!({"line": 0, "character": 5}));

        let (client, mut rx) = ready(json!(0));
        client.did_change(&uri(), 2, &old, &new);
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn server_requests_get_an_answer() {
        let (mut client, mut rx) = ready(json!(1));
        let request = json!({"jsonrpc": "2.0", "id": "c1", "method": "workspace/configuration",
            "params": {"items": [{}, {}]}});
        assert_eq!(client.handle(request), None);
        let sent = drain(&mut rx);
        assert_eq!(sent[0]["id"], "c1");
        assert_eq!(sent[0]["result"], json!([null, null]));
    }

    #[test]
    fn an_unexpected_exit_is_a_crash_and_a_requested_one_is_not() {
        let (mut client, _) = ready(json!(1));
        assert_eq!(
            client.exited(Some(3)).as_deref(),
            Some("rust: server crashed (exit code 3)")
        );
        assert_eq!(client.state, State::Failed);
        // Nothing reaches a dead server, and it isn't reported twice.
        assert_eq!(client.exited(None), None);

        let (mut client, mut rx) = ready(json!(1));
        client.shutdown();
        let methods: Vec<Value> = drain(&mut rx).iter().map(|m| m["method"].clone()).collect();
        assert_eq!(methods, [json!("shutdown"), json!("exit")]);
        assert_eq!(client.exited(Some(0)), None);
    }

    #[test]
    fn definition_sends_the_position_and_its_reply_is_consumed() {
        let (mut client, mut rx) = ready(json!(2));
        let id = client.definition(&uri(), Position::new(3, 7));
        let sent = drain(&mut rx);
        assert_eq!(sent[0]["method"], "textDocument/definition");
        assert_eq!(sent[0]["id"], id);
        assert_eq!(sent[0]["params"]["textDocument"]["uri"], uri().as_str());
        assert_eq!(
            sent[0]["params"]["position"],
            json!({"line": 3, "character": 7})
        );
        let reply = json!({"jsonrpc": "2.0", "id": id, "result": null});
        assert_eq!(client.handle(reply), None);
        assert!(client.pending.is_empty());
    }

    #[test]
    fn hover_sends_the_position_and_advertises_plain_text() {
        let (_client, mut rx) = started();
        let init = drain(&mut rx);
        assert_eq!(
            init[0]["params"]["capabilities"]["textDocument"]["hover"]["contentFormat"],
            json!(["plaintext", "markdown"])
        );
        let (mut client, mut rx) = ready(json!(2));
        let id = client.hover(&uri(), Position::new(1, 4));
        let sent = drain(&mut rx);
        assert_eq!(sent[0]["method"], "textDocument/hover");
        assert_eq!(sent[0]["id"], id);
        assert_eq!(
            sent[0]["params"]["position"],
            json!({"line": 1, "character": 4})
        );
    }

    #[test]
    fn completion_sends_the_position_and_reads_the_trigger_characters() {
        let (mut client, mut rx) = started();
        let init = drain(&mut rx);
        assert_eq!(
            init[0]["params"]["capabilities"]["textDocument"]["completion"]["completionItem"]["snippetSupport"],
            json!(false)
        );
        let reply = json!({"jsonrpc": "2.0", "id": init[0]["id"], "result": {"capabilities": {
            "textDocumentSync": 2, "completionProvider": {"triggerCharacters": [".", ":", ""]}}}});
        client.handle(reply);
        assert_eq!(client.triggers, [".", ":"]);
        drain(&mut rx);
        let id = client.completion(&uri(), Position::new(2, 5));
        let sent = drain(&mut rx);
        assert_eq!(sent[0]["method"], "textDocument/completion");
        assert_eq!(sent[0]["id"], id);
        assert_eq!(
            sent[0]["params"]["position"],
            json!({"line": 2, "character": 5})
        );
        // A server without a completion provider has no triggers.
        let (client, _) = ready(json!(2));
        assert!(client.triggers.is_empty());
    }

    #[test]
    fn a_failed_initialize_says_why() {
        let (mut client, mut rx) = started();
        drain(&mut rx);
        let reply = json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -1, "message": "nope"}});
        assert_eq!(
            client.handle(reply).as_deref(),
            Some("rust: server failed to start (nope)")
        );
        assert_eq!(client.state, State::Failed);
    }
}
