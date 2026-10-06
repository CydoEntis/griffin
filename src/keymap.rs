//! The one place that looks at `KeyCode`: turns key events into an `Action` or typed
//! text, using the spec's default bindings overridden by `[keys]` in `config.toml`.

use std::collections::HashMap;
use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::config::KeysConfig;

/// Something a key can trigger. Each feature adds its variant here, a name in
/// `Action::name` and a line in `DEFAULT_BINDINGS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Quit,
}

impl Action {
    const ALL: &[Action] = &[Action::Quit];

    /// The name used on the left of `[keys]`.
    pub fn name(self) -> &'static str {
        match self {
            Action::Quit => "quit",
        }
    }

    fn from_name(name: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.name() == name)
    }
}

/// The spec's "Default keymap" table, one line per binding.
const DEFAULT_BINDINGS: &[(Action, &str)] = &[(Action::Quit, "ctrl+q")];

/// What a key event means to the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Action(Action),
    /// A printable character typed without Ctrl or Alt.
    Text(char),
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapError(String);

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KeymapError {}

/// A key with its modifiers, normalised so that what the terminal reports and what
/// the config names compare equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    code: KeyCode,
    mods: KeyModifiers,
}

#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: HashMap<Key, Action>,
}

impl Default for Keymap {
    fn default() -> Self {
        // Every entry in DEFAULT_BINDINGS is a valid key, which the
        // `defaults_build` test checks, so an empty `[keys]` can't fail.
        Keymap::new(&KeysConfig::new()).expect("default bindings parse")
    }
}

impl Keymap {
    /// Defaults, with every action named in `keys` taking exactly the keys given
    /// there instead of its defaults.
    pub fn new(keys: &KeysConfig) -> Result<Self, KeymapError> {
        let mut overrides = HashMap::new();
        for (name, binding) in keys {
            let action = Action::from_name(name)
                .ok_or_else(|| KeymapError(format!("[keys]: unknown action {name:?}")))?;
            let parsed = binding
                .keys()
                .iter()
                .map(|k| parse_key(k).map_err(|e| KeymapError(format!("[keys] {name}: {e}"))))
                .collect::<Result<Vec<_>, _>>()?;
            overrides.insert(action, parsed);
        }

        let mut bindings = HashMap::new();
        for &(action, notation) in DEFAULT_BINDINGS {
            if overrides.contains_key(&action) {
                continue;
            }
            let key = parse_key(notation)
                .map_err(|e| KeymapError(format!("default binding for {}: {e}", action.name())))?;
            bindings.insert(key, action);
        }
        // Inserted last so a user's key wins over another action's default.
        for (action, keys) in overrides {
            for key in keys {
                bindings.insert(key, action);
            }
        }
        Ok(Keymap { bindings })
    }

    pub fn resolve(&self, event: &KeyEvent) -> Input {
        if event.kind == KeyEventKind::Release {
            return Input::Ignored;
        }
        if let Some(&action) = self.bindings.get(&normalize(event.code, event.modifiers)) {
            return Input::Action(action);
        }
        match event.code {
            KeyCode::Char(c)
                if !c.is_control()
                    && !event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                Input::Text(c)
            }
            _ => Input::Ignored,
        }
    }
}

/// Terminals report Shift+letter as the uppercase letter (sometimes with SHIFT,
/// sometimes without) and Shift+Tab as BackTab; fold those into one form. For other
/// characters the shifted character itself is the key, so SHIFT is dropped.
fn normalize(code: KeyCode, modifiers: KeyModifiers) -> Key {
    let mut mods = modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
    let code = match code {
        KeyCode::BackTab => {
            mods |= KeyModifiers::SHIFT;
            KeyCode::Tab
        }
        KeyCode::Char(c) if c.is_ascii_uppercase() => {
            mods |= KeyModifiers::SHIFT;
            KeyCode::Char(c.to_ascii_lowercase())
        }
        KeyCode::Char(c) if c != ' ' && !c.is_ascii_lowercase() => {
            mods -= KeyModifiers::SHIFT;
            KeyCode::Char(c)
        }
        other => other,
    };
    Key { code, mods }
}

/// Parses one key in the spec's notation: `ctrl+s`, `alt+,`, `shift+f5`, `ctrl++`.
fn parse_key(notation: &str) -> Result<Key, String> {
    let lower = notation.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return Err("empty key".into());
    }
    // A trailing "+" is the plus key itself (`ctrl++`), not a separator.
    let (prefix, name) = match lower.strip_suffix("++") {
        Some(rest) => (rest, "+"),
        None if lower == "+" => ("", "+"),
        None => lower.rsplit_once('+').unwrap_or(("", lower.as_str())),
    };

    let mut mods = KeyModifiers::NONE;
    for part in prefix.split('+') {
        mods |= match part {
            "ctrl" | "control" => KeyModifiers::CONTROL,
            "alt" | "meta" => KeyModifiers::ALT,
            "shift" => KeyModifiers::SHIFT,
            "" if prefix.is_empty() => KeyModifiers::NONE,
            other => return Err(format!("unknown modifier {other:?} in {notation:?}")),
        };
    }

    let code = match name {
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "insert" | "ins" => KeyCode::Insert,
        "delete" | "del" => KeyCode::Delete,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        _ => match function_key(name) {
            Some(n) => KeyCode::F(n),
            None => {
                let mut chars = name.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if !c.is_control() => {
                        if mods.contains(KeyModifiers::SHIFT) && !c.is_ascii_alphabetic() {
                            return Err(format!(
                                "{notation:?}: write the shifted character instead of shift+{c}"
                            ));
                        }
                        KeyCode::Char(c)
                    }
                    _ => return Err(format!("unknown key {name:?} in {notation:?}")),
                }
            }
        },
    };
    Ok(normalize(code, mods))
}

fn function_key(name: &str) -> Option<u8> {
    let n: u8 = name.strip_prefix('f')?.parse().ok()?;
    (1..=24).contains(&n).then_some(n)
}

/// Builds the event a terminal reports for a key in `[keys]` notation, so tests
/// outside this module can send keys without naming `KeyCode`.
#[cfg(test)]
pub fn key_event(notation: &str) -> KeyEvent {
    let key = parse_key(notation).unwrap_or_else(|e| panic!("{e}"));
    let code = match key.code {
        KeyCode::Char(c) if key.mods.contains(KeyModifiers::SHIFT) => {
            KeyCode::Char(c.to_ascii_uppercase())
        }
        other => other,
    };
    KeyEvent::new(code, key.mods)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::KeyBinding;

    fn keys(entries: &[(&str, KeyBinding)]) -> KeysConfig {
        entries
            .iter()
            .map(|(name, binding)| (name.to_string(), binding.clone()))
            .collect()
    }

    fn one(key: &str) -> KeyBinding {
        KeyBinding::One(key.to_string())
    }

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    /// Every key in the spec's "Default keymap" table, with the event crossterm
    /// reports for it.
    fn spec_keys() -> Vec<(&'static str, KeyEvent)> {
        let ctrl = |c| ev(KeyCode::Char(c), KeyModifiers::CONTROL);
        let alt = |c| ev(KeyCode::Char(c), KeyModifiers::ALT);
        let mut all = vec![
            ("ctrl+q", ctrl('q')),
            ("ctrl+s", ctrl('s')),
            ("alt+s", alt('s')),
            ("ctrl+n", ctrl('n')),
            ("ctrl+w", ctrl('w')),
            ("ctrl+z", ctrl('z')),
            ("ctrl+y", ctrl('y')),
            ("ctrl+c", ctrl('c')),
            ("ctrl+x", ctrl('x')),
            ("ctrl+v", ctrl('v')),
            ("ctrl+a", ctrl('a')),
            ("alt+,", alt(',')),
            ("alt+.", alt('.')),
            ("f12", ev(KeyCode::F(12), KeyModifiers::NONE)),
            ("alt+left", ev(KeyCode::Left, KeyModifiers::ALT)),
            ("alt+k", alt('k')),
            ("ctrl+f", ctrl('f')),
            ("ctrl+r", ctrl('r')),
            ("alt+f", alt('f')),
            ("ctrl+p", ctrl('p')),
            ("ctrl+g", ctrl('g')),
            ("ctrl+b", ctrl('b')),
            ("ctrl+e", ctrl('e')),
            ("alt+v", alt('v')),
            ("f6", ev(KeyCode::F(6), KeyModifiers::NONE)),
            ("f5", ev(KeyCode::F(5), KeyModifiers::NONE)),
            ("shift+f5", ev(KeyCode::F(5), KeyModifiers::SHIFT)),
            ("ctrl+f5", ev(KeyCode::F(5), KeyModifiers::CONTROL)),
            ("f4", ev(KeyCode::F(4), KeyModifiers::NONE)),
            ("f8", ev(KeyCode::F(8), KeyModifiers::NONE)),
            ("shift+f8", ev(KeyCode::F(8), KeyModifiers::SHIFT)),
            ("alt+/", alt('/')),
        ];
        const DIGITS: [(&str, char); 9] = [
            ("alt+1", '1'),
            ("alt+2", '2'),
            ("alt+3", '3'),
            ("alt+4", '4'),
            ("alt+5", '5'),
            ("alt+6", '6'),
            ("alt+7", '7'),
            ("alt+8", '8'),
            ("alt+9", '9'),
        ];
        all.extend(DIGITS.iter().map(|&(n, c)| (n, alt(c))));
        all
    }

    #[test]
    fn defaults_build() {
        let map = Keymap::default();
        assert_eq!(map.bindings.len(), DEFAULT_BINDINGS.len());
    }

    #[test]
    fn every_key_in_the_spec_table_parses_and_matches_its_event() {
        for (notation, event) in spec_keys() {
            let config = keys(&[("quit", one(notation))]);
            let map = Keymap::new(&config).unwrap_or_else(|e| panic!("{notation}: {e}"));
            assert_eq!(
                map.resolve(&event),
                Input::Action(Action::Quit),
                "{notation} should match {event:?}"
            );
        }
    }

    #[test]
    fn ctrl_q_quits_by_default() {
        let map = Keymap::default();
        let event = ev(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert_eq!(map.resolve(&event), Input::Action(Action::Quit));
    }

    #[test]
    fn remapping_replaces_the_default() {
        let map = Keymap::new(&keys(&[("quit", one("alt+q"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('q'), KeyModifiers::ALT)),
            Input::Action(Action::Quit)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Input::Ignored
        );
    }

    #[test]
    fn a_list_binds_every_key() {
        let binding = KeyBinding::Many(vec!["alt+q".into(), "shift+f5".into()]);
        let map = Keymap::new(&keys(&[("quit", binding)])).unwrap();
        for event in [
            ev(KeyCode::Char('q'), KeyModifiers::ALT),
            ev(KeyCode::F(5), KeyModifiers::SHIFT),
        ] {
            assert_eq!(map.resolve(&event), Input::Action(Action::Quit));
        }
    }

    #[test]
    fn unknown_action_is_an_error() {
        let err = Keymap::new(&keys(&[("fly", one("ctrl+q"))])).unwrap_err();
        assert!(err.to_string().contains("unknown action \"fly\""), "{err}");
    }

    #[test]
    fn unknown_key_or_modifier_is_an_error() {
        for bad in ["ctrl+nope", "hyper+q", "", "ctrl+", "f99", "shift+,"] {
            let result = Keymap::new(&keys(&[("quit", one(bad))]));
            assert!(result.is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn notation_is_case_insensitive_and_allows_plus_key() {
        assert_eq!(parse_key("Ctrl+Q"), parse_key("ctrl+q"));
        let plus = parse_key("ctrl++").unwrap();
        assert_eq!(plus.code, KeyCode::Char('+'));
        assert_eq!(plus.mods, KeyModifiers::CONTROL);
    }

    #[test]
    fn shift_letters_and_backtab_match_however_the_terminal_reports_them() {
        let map = Keymap::new(&keys(&[("quit", one("ctrl+shift+q"))])).unwrap();
        for event in [
            ev(KeyCode::Char('Q'), KeyModifiers::CONTROL),
            ev(
                KeyCode::Char('Q'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
        ] {
            assert_eq!(map.resolve(&event), Input::Action(Action::Quit));
        }
        let map = Keymap::new(&keys(&[("quit", one("shift+tab"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Input::Action(Action::Quit)
        );
    }

    #[test]
    fn plain_characters_are_text() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('q'), KeyModifiers::NONE)),
            Input::Text('q')
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('Q'), KeyModifiers::SHIFT)),
            Input::Text('Q')
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('é'), KeyModifiers::NONE)),
            Input::Text('é')
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            Input::Ignored
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('x'), KeyModifiers::ALT)),
            Input::Ignored
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Enter, KeyModifiers::NONE)),
            Input::Ignored
        );
    }

    #[test]
    fn key_release_is_ignored() {
        let map = Keymap::default();
        let mut release = ev(KeyCode::Char('q'), KeyModifiers::CONTROL);
        release.kind = KeyEventKind::Release;
        assert_eq!(map.resolve(&release), Input::Ignored);
    }

    #[test]
    fn key_event_helper_round_trips() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&key_event("ctrl+q")),
            Input::Action(Action::Quit)
        );
        assert_eq!(map.resolve(&key_event("q")), Input::Text('q'));
    }
}
