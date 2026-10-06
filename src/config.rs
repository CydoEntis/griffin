use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The user's `config.toml`. Sections Griffin doesn't know yet are ignored rather
/// than rejected, so later tickets can add theirs without breaking old files.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub keys: KeysConfig,
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

/// Loads `config.toml` from `GRIFFIN_CONFIG` or the OS config dir.
pub fn load() -> Loaded {
    match config_path(std::env::var_os("GRIFFIN_CONFIG")) {
        Some(path) => load_from(&path),
        None => Loaded::default(),
    }
}

/// `GRIFFIN_CONFIG` wins when set and non-empty; otherwise `%APPDATA%\griffin\` or
/// `~/.config/griffin/`. `None` only when the OS has no config dir at all.
pub fn config_path(env_override: Option<OsString>) -> Option<PathBuf> {
    if let Some(path) = env_override.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    directories::BaseDirs::new().map(|dirs| dirs.config_dir().join("griffin").join("config.toml"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn empty_file_is_defaults() {
        let loaded = parse("");
        assert!(loaded.error.is_none());
        assert!(loaded.config.keys.is_empty());
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
    fn default_path_is_griffin_config_toml_in_the_os_config_dir() {
        let path = config_path(None).expect("test machines have a config dir");
        assert!(path.ends_with(Path::new("griffin").join("config.toml")));
        let base = directories::BaseDirs::new().unwrap();
        assert!(path.starts_with(base.config_dir()));
        assert!(config_path(Some(OsString::new())) == Some(path));
    }
}
