use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml_edit::{Item, TableLike};

use crate::keymap::Keymap;
use crate::theme::{self, Theme};

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
    /// The shell command that installs the server. `servers::with_defaults`
    /// fills in the default for a built-in server; set here, it replaces it.
    pub install: Option<String>,
}

/// `[editor]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    /// Display columns between tab stops.
    pub tab_width: usize,
    /// Tab inserts spaces up to the next stop instead of a tab character.
    pub insert_spaces: bool,
    /// Typed brackets bring their closers along (see `Buffer::type_char`).
    pub auto_pairs: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            tab_width: 4,
            insert_spaces: true,
            auto_pairs: true,
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

/// What a `config.toml` gives the editor, built from it whole. Startup and
/// saving the file inside Glyph both come through here, so the two can't read
/// the same file differently.
#[derive(Debug, Default)]
pub struct Settings {
    pub keymap: Keymap,
    pub editor: EditorConfig,
    pub theme: Theme,
    /// `[lsp.<lang>]` as written; `lsp::servers::with_defaults` fills the rest.
    pub lsp: BTreeMap<String, LspServer>,
    pub debug: BTreeMap<String, DebugAdapter>,
}

impl Settings {
    /// Startup's reading: never fails. A malformed file means every default; a
    /// bad `[keys]` costs only the keymap and a bad theme only the theme, each
    /// with a message for the status line.
    pub fn startup(loaded: Loaded) -> (Self, Option<String>) {
        if let Some(err) = loaded.error {
            return (Self::default(), Some(format!("config error: {err}")));
        }
        let config = loaded.config;
        let (theme, theme_error) = theme::load(config.theme.as_deref(), &config.theme_overrides);
        let (keymap, keys_error) = match Keymap::new(&config.keys) {
            Ok(keymap) => (keymap, None),
            Err(err) => (Keymap::default(), Some(format!("config error: {err}"))),
        };
        let errors: Vec<String> = keys_error.into_iter().chain(theme_error).collect();
        let message = (!errors.is_empty()).then(|| errors.join(" · "));
        let settings = Self {
            keymap,
            editor: config.editor,
            theme,
            lsp: config.lsp,
            debug: config.debug,
        };
        (settings, message)
    }

    /// A saved file's reading: all or nothing, so a typo while editing never
    /// swaps working settings for defaults. The error is the status line's
    /// `config error: …`.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let config = toml::from_str::<Config>(text)
            .map_err(|err| format!("config error: {}", describe(&err, text)))?;
        let keymap = Keymap::new(&config.keys).map_err(|err| err.to_string());
        let theme = theme::resolve(config.theme.as_deref(), &config.theme_overrides);
        match (keymap, theme) {
            (Ok(keymap), Ok(theme)) => Ok(Self {
                keymap,
                editor: config.editor,
                theme,
                lsp: config.lsp,
                debug: config.debug,
            }),
            (keymap, theme) => {
                let errors: Vec<String> = keymap.err().into_iter().chain(theme.err()).collect();
                Err(format!("config error: {}", errors.join(" · ")))
            }
        }
    }
}

/// What `>settings` writes when there is no `config.toml` yet. Every line is
/// commented out, so the new file changes nothing until the user says so, and
/// each `[editor]` line shows the default it would keep.
pub const TEMPLATE: &str = r#"# Glyph's settings. Remove the `#` in front of a line to change it.

# One of aurora, moonlit, hydra, papercolor-dark, tango-dark, monokai,
# tokyo-night, catppuccin-mocha, catppuccin-latte, gruvbox, nord, dracula, mono.
# theme = "hydra"

[editor]
# tab_width = 4
# insert_spaces = true
# auto_pairs = true

[keys]
# action = "key" or ["key", "key"]
# save = "ctrl+s"
# go_to_file = ["ctrl+p", "alt+p"]
"#;

/// Writes `TEMPLATE` to `path`, making its folders first, unless a file is
/// already there: `create_new` keeps a file that appeared meanwhile.
pub fn create_template(path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => std::io::Write::write_all(&mut file, TEMPLATE.as_bytes()),
        Err(err) if err.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err),
    }
}

/// `text`, a whole `config.toml`, with `name` under `[keys]` bound to exactly
/// `keys`, or with its entry gone when `keys` is `None` so the command's
/// defaults come back. Everything else, comments and order included, is kept
/// as written; `[keys]` is added when there's none. One key is written as a
/// string, any other number as a list. The `Err` is the status line's
/// `config error: …`.
pub fn with_keys(text: &str, name: &str, keys: Option<&[String]>) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|_| unreadable(text))?;
    let root = doc.as_table_mut();
    if !root.contains_key("keys") {
        if keys.is_none() {
            return Ok(text.to_string());
        }
        root.insert("keys", Item::Table(toml_edit::Table::new()));
    }
    let table = root
        .get_mut("keys")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| "config error: keys is not a table".to_string())?;
    match keys {
        None => {
            table.remove(name);
        }
        Some(keys) => {
            let mut value = match keys {
                [one] => toml_edit::Value::from(one.as_str()),
                many => toml_edit::Value::Array(many.iter().map(String::as_str).collect()),
            };
            // A comment after the old value stays with the new one.
            if let Some(old) = table.get(name).and_then(Item::as_value) {
                *value.decor_mut() = old.decor().clone();
            }
            table.insert(name, Item::Value(value));
        }
    }
    Ok(doc.to_string())
}

/// Why `text` isn't TOML, in the words `Settings::from_text` would use.
fn unreadable(text: &str) -> String {
    match toml::from_str::<toml::Table>(text) {
        Err(err) => format!("config error: {}", describe(&err, text)),
        Ok(_) => "config error: config.toml is not valid TOML".to_string(),
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

    fn save_key(keymap: &Keymap) -> Option<String> {
        keymap.key_label(crate::keymap::Action::Save)
    }

    #[test]
    fn startup_reads_keys_editor_theme_and_tables() {
        let text = "theme = \"nord\"\n[editor]\ntab_width = 2\n[keys]\nsave = \"f2\"\n\
                    [lsp.rust]\ncommand = \"ra\"\n[debug.python]\nadapter = \"dbg\"\n";
        let (settings, message) = Settings::startup(parse(text));
        assert_eq!(message, None);
        assert_eq!(settings.editor.tab_width, 2);
        assert_eq!(settings.theme, Theme::named("nord").unwrap());
        assert_eq!(save_key(&settings.keymap).as_deref(), Some("F2"));
        assert_eq!(settings.lsp["rust"].command.as_deref(), Some("ra"));
        assert_eq!(settings.debug["python"].adapter.as_deref(), Some("dbg"));
    }

    #[test]
    fn startup_falls_back_to_defaults_on_a_malformed_file() {
        let (settings, message) = Settings::startup(parse("[editor\ntab_width = 2\n"));
        assert!(message.unwrap().starts_with("config error: line 1"));
        assert_eq!(settings.editor, EditorConfig::default());
        assert_eq!(settings.theme, Theme::default());
        assert!(settings.lsp.is_empty());
    }

    #[test]
    fn startup_drops_only_the_part_that_is_bad() {
        let text = "theme = \"nord\"\n[editor]\ntab_width = 2\n[keys]\nnope = \"f2\"\n";
        let (settings, message) = Settings::startup(parse(text));
        assert_eq!(
            message.as_deref(),
            Some("config error: [keys]: unknown action \"nope\"")
        );
        assert_eq!(settings.theme, Theme::named("nord").unwrap());
        assert_eq!(settings.editor.tab_width, 2);
        assert_eq!(save_key(&settings.keymap).as_deref(), Some("Ctrl+S"));

        let (settings, message) = Settings::startup(parse("theme = \"nope\"\n"));
        assert_eq!(
            message.as_deref(),
            Some("theme: unknown theme \"nope\", using hydra")
        );
        assert_eq!(settings.theme, Theme::default());
    }

    #[test]
    fn from_text_takes_a_good_file_whole() {
        let settings = Settings::from_text("theme = \"nord\"\n[editor]\ntab_width = 2\n").unwrap();
        assert_eq!(settings.editor.tab_width, 2);
        assert_eq!(settings.theme, Theme::named("nord").unwrap());
    }

    #[test]
    fn from_text_refuses_a_file_with_any_error() {
        let err = Settings::from_text("[editor\n").unwrap_err();
        assert!(err.starts_with("config error: line 1"), "{err}");
        let err = Settings::from_text("[editor]\ntab_width = 2\n[keys]\nnope = \"f2\"\n");
        assert_eq!(
            err.unwrap_err(),
            "config error: [keys]: unknown action \"nope\""
        );
        let err = Settings::from_text("theme = \"nope\"\n").unwrap_err();
        assert_eq!(err, "config error: theme: unknown theme \"nope\"");
    }

    #[test]
    fn template_parses_to_the_defaults() {
        let loaded = parse(TEMPLATE);
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(loaded.config.editor, EditorConfig::default());
        assert_eq!(loaded.config.theme, None);
        assert!(loaded.config.keys.is_empty());
    }

    #[test]
    fn template_names_each_editor_option_with_its_default() {
        // Uncommenting every line must change nothing: the values shown are
        // the defaults, and the examples are valid.
        let uncommented: String = TEMPLATE
            .lines()
            .filter_map(|line| line.strip_prefix("# "))
            .filter(|line| line.contains(" = "))
            .filter(|line| !line.starts_with("action"))
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        let sections = uncommented
            .replace(
                "tab_width",
                "[editor]
tab_width",
            )
            .replace(
                "save =",
                "[keys]
save =",
            );
        let loaded = parse(&sections);
        assert!(
            loaded.error.is_none(),
            "{:?}
{sections}",
            loaded.error
        );
        assert_eq!(loaded.config.editor, EditorConfig::default());
        assert_eq!(loaded.config.theme.as_deref(), Some("hydra"));
        assert!(crate::keymap::Keymap::new(&loaded.config.keys).is_ok());
        for name in ["tab_width", "insert_spaces", "auto_pairs"] {
            assert!(TEMPLATE.contains(&format!("# {name} = ")), "{name}");
        }
        for theme in crate::theme::NAMES {
            assert!(TEMPLATE.contains(theme), "{theme}");
        }
    }

    #[test]
    fn create_template_makes_the_folders_and_keeps_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glyph").join("nested").join("config.toml");
        create_template(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), TEMPLATE);
        fs::write(
            &path,
            "theme = \"nord\"
",
        )
        .unwrap();
        create_template(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "theme = \"nord\"
"
        );
    }

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
    fn auto_pairs_defaults_to_true_and_can_be_turned_off() {
        assert!(parse("").config.editor.auto_pairs);
        let loaded = parse("[editor]\nauto_pairs = false\n");
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert!(!loaded.config.editor.auto_pairs);
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
                install: None,
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
    fn an_lsp_table_reads_its_install_command() {
        let loaded = parse("[lsp.python]\ninstall = \"pip install pyright\"\n");
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(
            loaded.config.lsp["python"].install.as_deref(),
            Some("pip install pyright")
        );
        assert_eq!(parse("[lsp.go]\n").config.lsp["go"].install, None);
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

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn with_keys_writes_one_entry_and_keeps_the_rest_of_the_file() {
        let text = "# my settings\ntheme = \"nord\" # dark\n\n[editor]\ntab_width = 2\n\n\
                    [keys]\n# the palette\ngo_to_file = \"alt+p\"\nsave = \"f2\" # mine\n\n\
                    [lsp.rust]\ncommand = \"ra\"\n";
        let out = with_keys(text, "save", Some(&keys(&["alt+w"]))).unwrap();
        assert_eq!(
            out,
            "# my settings\ntheme = \"nord\" # dark\n\n[editor]\ntab_width = 2\n\n\
             [keys]\n# the palette\ngo_to_file = \"alt+p\"\nsave = \"alt+w\" # mine\n\n\
             [lsp.rust]\ncommand = \"ra\"\n"
        );
        // A new entry goes at the end of `[keys]`, before the next table.
        let out = with_keys(&out, "close_tab", Some(&keys(&["ctrl+k", "alt+q"]))).unwrap();
        assert!(
            out.contains(
                "save = \"alt+w\" # mine\nclose_tab = [\"ctrl+k\", \"alt+q\"]\n\n[lsp.rust]"
            ),
            "{out}"
        );
        // No keys at all is an empty list, which takes the defaults away too.
        let out = with_keys(&out, "close_tab", Some(&[])).unwrap();
        assert!(out.contains("close_tab = []\n"), "{out}");
        let config: Config = toml::from_str(&out).unwrap();
        assert_eq!(config.keys["close_tab"], KeyBinding::Many(Vec::new()));
        assert_eq!(config.lsp["rust"].command.as_deref(), Some("ra"));
    }

    #[test]
    fn with_keys_adds_keys_to_a_file_without_them() {
        let out = with_keys("theme = \"nord\"\n", "save", Some(&keys(&["alt+w"]))).unwrap();
        assert_eq!(out, "theme = \"nord\"\n\n[keys]\nsave = \"alt+w\"\n");
        let out = with_keys("", "save", Some(&keys(&["alt+w"]))).unwrap();
        assert_eq!(out, "[keys]\nsave = \"alt+w\"\n");
        // The template's commented-out lines stay, under the new entry.
        let out = with_keys(TEMPLATE, "save", Some(&keys(&["alt+w"]))).unwrap();
        assert!(out.starts_with("# Glyph's settings."), "{out}");
        assert!(out.contains("[keys]\nsave = \"alt+w\"\n"), "{out}");
        assert!(out.contains("# save = \"ctrl+s\"\n"), "{out}");
        assert!(out.contains("# tab_width = 4\n"), "{out}");
    }

    #[test]
    fn with_keys_none_removes_the_entry_only() {
        let text = "[keys]\nsave = \"alt+w\"\n# the palette\ngo_to_file = \"alt+p\"\n";
        let out = with_keys(text, "save", None).unwrap();
        assert_eq!(out, "[keys]\n# the palette\ngo_to_file = \"alt+p\"\n");
        // Nothing to remove changes nothing, and adds no `[keys]`.
        assert_eq!(with_keys(&out, "save", None).unwrap(), out);
        assert_eq!(
            with_keys("theme = \"nord\"\n", "save", None).unwrap(),
            "theme = \"nord\"\n"
        );
    }

    #[test]
    fn with_keys_refuses_a_file_that_isnt_toml() {
        let err = with_keys("theme = \"nord\"\n[keys\n", "save", Some(&keys(&["f2"]))).unwrap_err();
        assert!(err.starts_with("config error: line 2"), "{err}");
        let err = with_keys("keys = 3\n", "save", Some(&keys(&["f2"]))).unwrap_err();
        assert_eq!(err, "config error: keys is not a table");
    }
}
