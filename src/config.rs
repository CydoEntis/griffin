use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The user's `config.toml`. Sections Glyph doesn't know yet are ignored rather
/// than rejected, so later tickets can add theirs without breaking old files.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// One of `theme::NAMES`; `None` means hydra. Checked by `theme::load`.
    pub theme: Option<String>,
    /// Role name to colour. Values stay raw TOML so a bad one is reported by
    /// `theme::load` and costs only the theme, not the whole config.
    pub theme_overrides: BTreeMap<String, toml::Value>,
    pub editor: EditorConfig,
    pub keys: KeysConfig,
    /// `[lsp.<lang>]`: the language server for each language id.
    pub lsp: BTreeMap<String, LspServer>,
    /// `[debug.<lang>]`: the debug adapter for each language id.
    pub debug: BTreeMap<String, DebugAdapter>,
}

/// One `[debug.<lang>]` table, laid over the language's default adapter by
/// `dap::adapters::adapter_for`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct DebugAdapter {
    /// The adapter program, found on PATH or given as a path.
    pub adapter: Option<String>,
    pub args: Vec<String>,
}

/// One `[lsp.<lang>]` table. Every field is optional so a table that only sets
/// options for a later feature still loads.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct LspServer {
    /// The program to start, found on PATH or given as a path.
    pub command: Option<String>,
    pub args: Vec<String>,
    /// Ctrl+S formats the buffer through this server before writing it. Off
    /// unless asked for, since a formatter rewrites the user's text.
    pub format_on_save: bool,
}

/// `[editor]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    /// Display columns between tab stops.
    pub tab_width: usize,
    /// Tab inserts spaces up to the next stop instead of a tab character.
    pub insert_spaces: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            tab_width: 4,
            insert_spaces: true,
        }
    }
}

/// `[keys]`: action name to one key or a list of keys, in the spec's notation.
/// Names and keys are checked by `Keymap::new`, which owns the notation.
pub type KeysConfig = BTreeMap<String, KeyBinding>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum KeyBinding {
    One(String),
    Many(Vec<String>),
}

impl KeyBinding {
    pub fn keys(&self) -> &[String] {
        match self {
            KeyBinding::One(key) => std::slice::from_ref(key),
            KeyBinding::Many(keys) => keys,
        }
    }
}

/// What loading produced: always a usable config, plus the reason it fell back to
/// defaults, if it did.
#[derive(Debug, Default)]
pub struct Loaded {
    pub config: Config,
    pub error: Option<String>,
}

/// Loads `config.toml` from `GLYPH_CONFIG` or the OS config dir.
pub fn load() -> Loaded {
    match config_path(std::env::var_os("GLYPH_CONFIG")) {
        Some(path) => load_from(&path),
        None => Loaded::default(),
    }
}

/// `GLYPH_CONFIG` wins when set and non-empty; otherwise `%APPDATA%\glyph\` or
/// `~/.config/glyph/`. `None` only when the OS has no config dir at all.
pub fn config_path(env_override: Option<OsString>) -> Option<PathBuf> {
    if let Some(path) = env_override.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    directories::BaseDirs::new().map(|dirs| dirs.config_dir().join("glyph").join("config.toml"))
}

/// A missing file means defaults; an unreadable or malformed one means defaults
/// plus an error for the status line.
pub fn load_from(path: &Path) -> Loaded {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(err) if err.kind() == ErrorKind::NotFound => Loaded::default(),
        Err(err) => Loaded {
            config: Config::default(),
            error: Some(format!("cannot read {}: {err}", path.display())),
        },
    }
}

pub fn parse(text: &str) -> Loaded {
    match toml::from_str::<Config>(text) {
        Ok(config) => Loaded {
            config,
            error: None,
        },
        Err(err) => Loaded {
            config: Config::default(),
            error: Some(describe(&err, text)),
        },
    }
}

/// One line for the status line: toml's own message is multi-line with a snippet.
fn describe(err: &toml::de::Error, text: &str) -> String {
    let message = err.message().trim();
    match err.span() {
        Some(span) => {
            let end = span.start.min(text.len());
            let line = text.as_bytes()[..end]
                .iter()
                .filter(|&&b| b == b'\n')
                .count()
                + 1;
            format!("line {line}: {message}")
        }
        None => message.to_string(),
    }
}

/// The project's `.glyph.toml`, at the project root.
pub const PROJECT_FILE: &str = ".glyph.toml";

/// `.glyph.toml`. Like `config.toml`, unknown sections are ignored.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    pub run: Vec<RunEntry>,
    pub debug: DebugLaunch,
}

/// `.glyph.toml` `[debug]`: how this project's program is launched under the
/// debugger. Each field set replaces the language's default for it.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct DebugLaunch {
    /// Relative to the project root unless absolute.
    pub program: Option<String>,
    /// The program's arguments; `None` keeps the default (none), so an empty
    /// list and a missing one mean the same thing.
    pub args: Option<Vec<String>>,
    /// Relative to the project root; the root itself when absent.
    pub cwd: Option<String>,
    /// Run before the adapter starts; an empty string means no build at all.
    pub build: Option<String>,
}

/// One `[[run]]` entry: a command F5 can run.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RunEntry {
    pub name: String,
    pub command: String,
    /// Relative to the project root; the root itself when absent.
    #[serde(default)]
    pub cwd: Option<String>,
}

/// What loading `.glyph.toml` produced: always a usable (maybe empty) config,
/// plus why it fell back to empty, if it did.
#[derive(Debug, Default)]
pub struct LoadedProject {
    pub config: ProjectConfig,
    pub error: Option<String>,
}

/// Reads `.glyph.toml` from `root`. A missing file means no entries; an
/// unreadable or malformed one means no entries plus a one-line error.
pub fn load_project(root: &Path) -> LoadedProject {
    let path = root.join(PROJECT_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => parse_project(&text),
        Err(err) if err.kind() == ErrorKind::NotFound => LoadedProject::default(),
        Err(err) => LoadedProject {
            config: ProjectConfig::default(),
            error: Some(format!("cannot read {PROJECT_FILE}: {err}")),
        },
    }
}

pub fn parse_project(text: &str) -> LoadedProject {
    match toml::from_str::<ProjectConfig>(text) {
        Ok(config) => LoadedProject {
            config,
            error: None,
        },
        Err(err) => LoadedProject {
            config: ProjectConfig::default(),
            error: Some(format!("{PROJECT_FILE} {}", describe(&err, text))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn empty_file_is_defaults() {
        let loaded = parse("");
        assert!(loaded.error.is_none());
        assert!(loaded.config.keys.is_empty());
        assert_eq!(loaded.config.editor.tab_width, 4);
    }

    #[test]
    fn tab_width_is_read_from_editor() {
        let loaded = parse("[editor]\ntab_width = 8\n");
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(loaded.config.editor.tab_width, 8);
        assert!(loaded.config.editor.insert_spaces);
    }

    #[test]
    fn insert_spaces_defaults_to_true_and_can_be_turned_off() {
        assert!(parse("").config.editor.insert_spaces);
        let loaded = parse("[editor]\ninsert_spaces = false\n");
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert!(!loaded.config.editor.insert_spaces);
        assert_eq!(loaded.config.editor.tab_width, 4);
    }

    #[test]
    fn keys_accept_a_string_or_a_list() {
        let loaded = parse(
            r#"
            [keys]
            quit = "alt+q"
            save = ["ctrl+s", "f2"]
            "#,
        );
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        let keys = &loaded.config.keys;
        assert_eq!(keys["quit"].keys(), ["alt+q"]);
        assert_eq!(keys["save"].keys(), ["ctrl+s", "f2"]);
    }

    #[test]
    fn unknown_sections_are_ignored() {
        let loaded = parse(
            r##"
            theme = "hydra"
            [theme_overrides]
            background = "#000000"
            [editor]
            tab_width = 4
            [lsp.python]
            command = "pyright-langserver"
            [keys]
            quit = "ctrl+q"
            "##,
        );
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(loaded.config.keys.len(), 1);
    }

    #[test]
    fn lsp_tables_select_a_command_and_args_per_language() {
        let loaded = parse(
            r#"
            [lsp.python]
            command = "pyright-langserver"
            args = ["--stdio"]
            format_on_save = false
            [lsp.rust]
            command = 'C:\tools\rust-analyzer.exe'
            "#,
        );
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        let lsp = &loaded.config.lsp;
        assert_eq!(
            lsp["python"],
            LspServer {
                command: Some("pyright-langserver".into()),
                args: vec!["--stdio".into()],
                format_on_save: false,
            }
        );
        assert_eq!(
            lsp["rust"].command.as_deref(),
            Some(r"C:\tools\rust-analyzer.exe")
        );
        assert!(lsp["rust"].args.is_empty());
        assert!(!lsp["rust"].format_on_save, "off by default");
        let on = parse("[lsp.rust]\nformat_on_save = true\n");
        assert!(on.config.lsp["rust"].format_on_save);
        assert!(parse("").config.lsp.is_empty());
    }

    #[test]
    fn an_lsp_table_without_a_command_loads_without_one() {
        let loaded = parse("[lsp.go]\nargs = [\"serve\"]\n");
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(loaded.config.lsp["go"].command, None);
        assert_eq!(loaded.config.lsp["go"].args, ["serve"]);
    }

    #[test]
    fn lsp_args_must_be_a_list_of_strings() {
        let loaded = parse("[lsp.go]\ncommand = \"gopls\"\nargs = \"serve\"\n");
        assert!(loaded.error.is_some());
        assert!(loaded.config.lsp.is_empty());
    }

    #[test]
    fn theme_and_overrides_are_read() {
        let loaded = parse(
            r##"
            theme = "nord"
            [theme_overrides]
            sidebar_bg = "#123456"
            keyword = 141
            "##,
        );
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        let config = loaded.config;
        assert_eq!(config.theme.as_deref(), Some("nord"));
        assert_eq!(
            config.theme_overrides["sidebar_bg"].as_str(),
            Some("#123456")
        );
        assert_eq!(config.theme_overrides["keyword"].as_integer(), Some(141));
        assert_eq!(parse("").config.theme, None);
    }

    #[test]
    fn malformed_file_falls_back_to_defaults_with_an_error() {
        let loaded = parse("[keys]\nquit = = \"alt+q\"\n");
        assert!(loaded.config.keys.is_empty());
        let error = loaded.error.expect("malformed toml is an error");
        assert!(error.starts_with("line 2:"), "{error}");
        assert!(!error.contains('\n'), "{error}");
    }

    #[test]
    fn wrong_value_type_is_an_error() {
        let loaded = parse("[keys]\nquit = 5\n");
        assert!(loaded.error.is_some());
        assert!(loaded.config.keys.is_empty());
    }

    #[test]
    fn missing_file_is_defaults_without_error() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_from(&dir.path().join("nope.toml"));
        assert!(loaded.error.is_none());
        assert!(loaded.config.keys.is_empty());
    }

    #[test]
    fn reads_the_file_at_the_given_path() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "[keys]\nquit = \"alt+q\"").unwrap();
        let loaded = load_from(file.path());
        assert!(loaded.error.is_none());
        assert_eq!(loaded.config.keys["quit"].keys(), ["alt+q"]);
    }

    #[test]
    fn env_override_wins_over_the_os_dir() {
        let path = config_path(Some(OsString::from("/tmp/custom.toml")));
        assert_eq!(path, Some(PathBuf::from("/tmp/custom.toml")));
    }

    #[test]
    fn default_path_is_glyph_config_toml_in_the_os_config_dir() {
        let path = config_path(None).expect("test machines have a config dir");
        assert!(path.ends_with(Path::new("glyph").join("config.toml")));
        let base = directories::BaseDirs::new().unwrap();
        assert!(path.starts_with(base.config_dir()));
        assert!(config_path(Some(OsString::new())) == Some(path));
    }
    #[test]
    fn run_entries_are_read_with_an_optional_cwd() {
        let loaded = parse_project(
            r#"
            [[run]]
            name = "dev"
            command = "npm run dev"
            cwd = "web"

            [[run]]
            name = "test"
            command = "cargo test"
            "#,
        );
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(
            loaded.config.run,
            [
                RunEntry {
                    name: "dev".into(),
                    command: "npm run dev".into(),
                    cwd: Some("web".into()),
                },
                RunEntry {
                    name: "test".into(),
                    command: "cargo test".into(),
                    cwd: None,
                },
            ]
        );
    }

    #[test]
    fn a_missing_project_file_is_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_project(dir.path());
        assert!(loaded.error.is_none());
        assert!(loaded.config.run.is_empty());
    }

    #[test]
    fn the_project_file_is_read_from_the_root() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(PROJECT_FILE),
            "[[run]]\nname = \"dev\"\ncommand = \"echo hi\"\n",
        )
        .unwrap();
        let loaded = load_project(dir.path());
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(loaded.config.run[0].name, "dev");
    }

    #[test]
    fn a_malformed_project_file_is_an_empty_list_with_a_one_line_error() {
        let loaded = parse_project("[[run]]\nname = \"dev\"\ncommand = = 1\n");
        assert!(loaded.config.run.is_empty());
        let error = loaded.error.expect("malformed toml is an error");
        assert!(error.starts_with(".glyph.toml line 3:"), "{error}");
        assert!(!error.contains('\n'), "{error}");
    }

    #[test]
    fn debug_tables_select_an_adapter_and_args_per_language() {
        let loaded = parse(
            r#"
            [debug.rust]
            adapter = "codelldb"
            args = ["--port", "0"]
            [debug.python]
            args = ["-X"]
            "#,
        );
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        let debug = &loaded.config.debug;
        assert_eq!(
            debug["rust"],
            DebugAdapter {
                adapter: Some("codelldb".into()),
                args: vec!["--port".into(), "0".into()],
            }
        );
        assert_eq!(debug["python"].adapter, None);
        assert_eq!(debug["python"].args, ["-X"]);
        assert!(parse("").config.debug.is_empty());
    }

    #[test]
    fn the_project_debug_table_reads_each_launch_field() {
        let loaded = parse_project(
            r#"
            [debug]
            program = "bin/app"
            args = ["--verbose"]
            cwd = "work"
            build = "make"
            "#,
        );
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(
            loaded.config.debug,
            DebugLaunch {
                program: Some("bin/app".into()),
                args: Some(vec!["--verbose".into()]),
                cwd: Some("work".into()),
                build: Some("make".into()),
            }
        );
        assert_eq!(parse_project("").config.debug, DebugLaunch::default());
    }

    #[test]
    fn a_run_entry_without_a_command_is_an_error() {
        let loaded = parse_project("[[run]]\nname = \"dev\"\n");
        assert!(loaded.config.run.is_empty());
        assert!(loaded.error.is_some());
    }
}
