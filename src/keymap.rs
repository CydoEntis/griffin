//! The one place that looks at `KeyCode`: turns key events into an `Action` or typed
//! text, using the spec's default bindings overridden by `[keys]` in `config.toml`.

use std::collections::HashMap;
use std::fmt;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::buffer::movement::Motion;
use crate::config::KeysConfig;

/// Something a key can trigger. Each feature adds its variant here, a name in
/// `Action::name` and a line in `DEFAULT_BINDINGS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Quit,
    Save,
    /// Esc: closes whatever prompt or popup is open.
    Cancel,
    Move(Motion),
    Newline,
    Backspace,
    Delete,
    Tab,
    Undo,
    Redo,
    /// Shift+movement: moves the cursor and extends the selection.
    Select(Motion),
    SelectAll,
    Copy,
    Cut,
    Paste,
}

impl Action {
    const ALL: &[Action] = &[
        Action::Quit,
        Action::Save,
        Action::Cancel,
        Action::Move(Motion::Left),
        Action::Move(Motion::Right),
        Action::Move(Motion::Up),
        Action::Move(Motion::Down),
        Action::Move(Motion::LineStart),
        Action::Move(Motion::LineEnd),
        Action::Move(Motion::PageUp),
        Action::Move(Motion::PageDown),
        Action::Move(Motion::WordLeft),
        Action::Move(Motion::WordRight),
        Action::Move(Motion::DocStart),
        Action::Move(Motion::DocEnd),
        Action::Newline,
        Action::Backspace,
        Action::Delete,
        Action::Tab,
        Action::Undo,
        Action::Redo,
        Action::Select(Motion::Left),
        Action::Select(Motion::Right),
        Action::Select(Motion::Up),
        Action::Select(Motion::Down),
        Action::Select(Motion::LineStart),
        Action::Select(Motion::LineEnd),
        Action::Select(Motion::PageUp),
        Action::Select(Motion::PageDown),
        Action::Select(Motion::WordLeft),
        Action::Select(Motion::WordRight),
        Action::Select(Motion::DocStart),
        Action::Select(Motion::DocEnd),
        Action::SelectAll,
        Action::Copy,
        Action::Cut,
        Action::Paste,
    ];

    /// The name used on the left of `[keys]`.
    pub fn name(self) -> &'static str {
        match self {
            Action::Quit => "quit",
            Action::Save => "save",
            Action::Cancel => "cancel",
            Action::Move(motion) => match motion {
                Motion::Left => "move_left",
                Motion::Right => "move_right",
                Motion::Up => "move_up",
                Motion::Down => "move_down",
                Motion::LineStart => "line_start",
                Motion::LineEnd => "line_end",
                Motion::PageUp => "page_up",
                Motion::PageDown => "page_down",
                Motion::WordLeft => "word_left",
                Motion::WordRight => "word_right",
                Motion::DocStart => "doc_start",
                Motion::DocEnd => "doc_end",
            },
            Action::Newline => "newline",
            Action::Backspace => "backspace",
            Action::Delete => "delete",
            Action::Tab => "tab",
            Action::Undo => "undo",
            Action::Redo => "redo",
            Action::Select(motion) => match motion {
                Motion::Left => "select_left",
                Motion::Right => "select_right",
                Motion::Up => "select_up",
                Motion::Down => "select_down",
                Motion::LineStart => "select_line_start",
                Motion::LineEnd => "select_line_end",
                Motion::PageUp => "select_page_up",
                Motion::PageDown => "select_page_down",
                Motion::WordLeft => "select_word_left",
                Motion::WordRight => "select_word_right",
                Motion::DocStart => "select_doc_start",
                Motion::DocEnd => "select_doc_end",
            },
            Action::SelectAll => "select_all",
            Action::Copy => "copy",
            Action::Cut => "cut",
            Action::Paste => "paste",
        }
    }

    fn from_name(name: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.name() == name)
    }
}

/// The spec's "Default keymap" table plus R4's movement, R6's editing and R10's
/// selection keys, one line per binding.
const DEFAULT_BINDINGS: &[(Action, &str)] = &[
    (Action::Quit, "ctrl+q"),
    (Action::Save, "ctrl+s"),
    (Action::Cancel, "esc"),
    (Action::Move(Motion::Left), "left"),
    (Action::Move(Motion::Right), "right"),
    (Action::Move(Motion::Up), "up"),
    (Action::Move(Motion::Down), "down"),
    (Action::Move(Motion::LineStart), "home"),
    (Action::Move(Motion::LineEnd), "end"),
    (Action::Move(Motion::PageUp), "pageup"),
    (Action::Move(Motion::PageDown), "pagedown"),
    (Action::Move(Motion::WordLeft), "ctrl+left"),
    (Action::Move(Motion::WordRight), "ctrl+right"),
    (Action::Move(Motion::DocStart), "ctrl+home"),
    (Action::Move(Motion::DocEnd), "ctrl+end"),
    (Action::Newline, "enter"),
    (Action::Backspace, "backspace"),
    (Action::Delete, "delete"),
    (Action::Tab, "tab"),
    (Action::Undo, "ctrl+z"),
    (Action::Redo, "ctrl+y"),
    (Action::Select(Motion::Left), "shift+left"),
    (Action::Select(Motion::Right), "shift+right"),
    (Action::Select(Motion::Up), "shift+up"),
    (Action::Select(Motion::Down), "shift+down"),
    (Action::Select(Motion::LineStart), "shift+home"),
    (Action::Select(Motion::LineEnd), "shift+end"),
    (Action::Select(Motion::PageUp), "shift+pageup"),
    (Action::Select(Motion::PageDown), "shift+pagedown"),
    (Action::Select(Motion::WordLeft), "ctrl+shift+left"),
    (Action::Select(Motion::WordRight), "ctrl+shift+right"),
    (Action::Select(Motion::DocStart), "ctrl+shift+home"),
    (Action::Select(Motion::DocEnd), "ctrl+shift+end"),
    (Action::SelectAll, "ctrl+a"),
    (Action::Copy, "ctrl+c"),
    (Action::Cut, "ctrl+x"),
    (Action::Paste, "ctrl+v"),
];

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

/// The text of a burst of key events that arrived together, when they look like a
/// paste rather than typing: crossterm can't report bracketed paste on Windows, so
/// a paste there (Windows Terminal's own Ctrl+V included) arrives as a run of key
/// presses instead. A burst counts as pasted when every press is plain text, Enter
/// or Tab and it holds a line break or tab; nobody types a newline in the same
/// instant as the text before it, while a burst of plain letters is left as typing
/// so fast typists keep their undo steps.
#[cfg_attr(
    not(any(windows, test)),
    expect(dead_code, reason = "only Windows needs to recover pastes from keys")
)]
pub fn burst_as_paste(events: &[Event]) -> Option<String> {
    let mut text = String::new();
    let mut presses = 0;
    for event in events {
        let Event::Key(key) = event else {
            return None;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        match key.code {
            KeyCode::Char(c) if !c.is_control() => text.push(c),
            KeyCode::Enter => text.push('\n'),
            KeyCode::Tab => text.push('\t'),
            _ => return None,
        }
        presses += 1;
    }
    (presses >= 2 && text.contains(['\n', '\t'])).then_some(text)
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
    fn default_bindings_cover_every_action_once() {
        let mut bound: Vec<Action> = DEFAULT_BINDINGS.iter().map(|&(a, _)| a).collect();
        bound.sort_by_key(|a| a.name());
        let mut all = Action::ALL.to_vec();
        all.sort_by_key(|a| a.name());
        assert_eq!(bound, all);
        for &motion in Motion::ALL {
            assert!(Action::ALL.contains(&Action::Move(motion)), "{motion:?}");
            assert!(Action::ALL.contains(&Action::Select(motion)), "{motion:?}");
        }
    }

    #[test]
    fn movement_keys_resolve_to_their_motions() {
        let map = Keymap::default();
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let expected = [
            (KeyCode::Left, none, Motion::Left),
            (KeyCode::Right, none, Motion::Right),
            (KeyCode::Up, none, Motion::Up),
            (KeyCode::Down, none, Motion::Down),
            (KeyCode::Home, none, Motion::LineStart),
            (KeyCode::End, none, Motion::LineEnd),
            (KeyCode::PageUp, none, Motion::PageUp),
            (KeyCode::PageDown, none, Motion::PageDown),
            (KeyCode::Left, ctrl, Motion::WordLeft),
            (KeyCode::Right, ctrl, Motion::WordRight),
            (KeyCode::Home, ctrl, Motion::DocStart),
            (KeyCode::End, ctrl, Motion::DocEnd),
        ];
        assert_eq!(expected.len(), Motion::ALL.len());
        for (code, mods, motion) in expected {
            assert_eq!(
                map.resolve(&ev(code, mods)),
                Input::Action(Action::Move(motion)),
                "{mods:?} {code:?}"
            );
        }
    }

    #[test]
    fn movement_can_be_remapped_by_name() {
        let map = Keymap::new(&keys(&[("word_right", one("alt+right"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::Right, KeyModifiers::ALT)),
            Input::Action(Action::Move(Motion::WordRight))
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Right, KeyModifiers::CONTROL)),
            Input::Ignored
        );
    }

    #[test]
    fn ctrl_q_quits_by_default() {
        let map = Keymap::default();
        let event = ev(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert_eq!(map.resolve(&event), Input::Action(Action::Quit));
    }

    #[test]
    fn ctrl_s_saves_and_esc_cancels_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Input::Action(Action::Save)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Esc, KeyModifiers::NONE)),
            Input::Action(Action::Cancel)
        );
        // Plain letters stay text, so prompts can read their answers from it.
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('s'), KeyModifiers::NONE)),
            Input::Text('s')
        );
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
            map.resolve(&ev(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            Input::Ignored
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('x'), KeyModifiers::ALT)),
            Input::Ignored
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::F(9), KeyModifiers::NONE)),
            Input::Ignored
        );
    }

    #[test]
    fn editing_keys_resolve_to_their_actions() {
        let map = Keymap::default();
        let none = KeyModifiers::NONE;
        for (code, action) in [
            (KeyCode::Enter, Action::Newline),
            (KeyCode::Backspace, Action::Backspace),
            (KeyCode::Delete, Action::Delete),
            (KeyCode::Tab, Action::Tab),
        ] {
            assert_eq!(
                map.resolve(&ev(code, none)),
                Input::Action(action),
                "{code:?}"
            );
        }
    }

    #[test]
    fn ctrl_z_undoes_and_ctrl_y_redoes_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            Input::Action(Action::Undo)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('y'), KeyModifiers::CONTROL)),
            Input::Action(Action::Redo)
        );
    }

    #[test]
    fn shift_movement_selects_and_clipboard_keys_resolve() {
        let map = Keymap::default();
        let shift = KeyModifiers::SHIFT;
        let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        let expected = [
            (KeyCode::Left, shift, Motion::Left),
            (KeyCode::Right, shift, Motion::Right),
            (KeyCode::Up, shift, Motion::Up),
            (KeyCode::Down, shift, Motion::Down),
            (KeyCode::Home, shift, Motion::LineStart),
            (KeyCode::End, shift, Motion::LineEnd),
            (KeyCode::PageUp, shift, Motion::PageUp),
            (KeyCode::PageDown, shift, Motion::PageDown),
            (KeyCode::Left, ctrl_shift, Motion::WordLeft),
            (KeyCode::Right, ctrl_shift, Motion::WordRight),
            (KeyCode::Home, ctrl_shift, Motion::DocStart),
            (KeyCode::End, ctrl_shift, Motion::DocEnd),
        ];
        assert_eq!(expected.len(), Motion::ALL.len());
        for (code, mods, motion) in expected {
            assert_eq!(
                map.resolve(&ev(code, mods)),
                Input::Action(Action::Select(motion)),
                "{mods:?} {code:?}"
            );
        }
        let ctrl = KeyModifiers::CONTROL;
        for (c, action) in [
            ('a', Action::SelectAll),
            ('c', Action::Copy),
            ('x', Action::Cut),
            ('v', Action::Paste),
        ] {
            assert_eq!(
                map.resolve(&ev(KeyCode::Char(c), ctrl)),
                Input::Action(action),
                "ctrl+{c}"
            );
        }
    }

    #[test]
    fn a_burst_with_a_line_break_is_a_paste() {
        let key = |code| Event::Key(ev(code, KeyModifiers::NONE));
        let mut release = ev(KeyCode::Char('a'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        let pasted = [
            key(KeyCode::Char('a')),
            Event::Key(release),
            key(KeyCode::Enter),
            key(KeyCode::Tab),
            Event::Key(ev(KeyCode::Char('B'), KeyModifiers::SHIFT)),
        ];
        assert_eq!(burst_as_paste(&pasted).as_deref(), Some("a\n\tB"));

        // Plain letters stay typing, and so does a lone Enter.
        let typed = [key(KeyCode::Char('a')), key(KeyCode::Char('b'))];
        assert_eq!(burst_as_paste(&typed), None);
        assert_eq!(burst_as_paste(&[key(KeyCode::Enter)]), None);
        // A shortcut or a non-text key in the burst means it wasn't a paste.
        let shortcut = [
            key(KeyCode::Char('a')),
            key(KeyCode::Enter),
            Event::Key(ev(KeyCode::Char('z'), KeyModifiers::CONTROL)),
        ];
        assert_eq!(burst_as_paste(&shortcut), None);
        let arrow = [key(KeyCode::Enter), key(KeyCode::Left)];
        assert_eq!(burst_as_paste(&arrow), None);
        assert_eq!(
            burst_as_paste(&[key(KeyCode::Enter), Event::FocusLost]),
            None
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
