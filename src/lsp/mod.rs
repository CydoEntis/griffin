//! Language servers: one per language per project root, started when a file of
//! that language opens and kept in sync with every open buffer of it. Messages
//! reach the app only as `AppEvent::Lsp` (ADR-0001), and a server that is missing
//! or dies only costs a status line: editing never waits on one.

mod client;
mod position;
mod transport;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lsp_types::Uri;
use ropey::Rope;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;
use crate::buffer::Buffer;
use crate::config::LspServer;
use crate::highlight::languages;
use client::Client;
use position::path_to_uri;

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

    /// Hands a server's message or exit to its client. Returns a status line when
    /// there's news (a crash, a failed start).
    pub fn handle(&mut self, event: LspEvent) -> Option<String> {
        let client = self.servers.get_mut(&event.server)?;
        match event.event {
            ServerEvent::Message(message) => client.handle(message),
            ServerEvent::Exited(code) => client.exited(code),
        }
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
