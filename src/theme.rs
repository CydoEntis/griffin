//! Hydra's 11 themes and `[theme_overrides]`. A theme paints Griffin's chrome (tab
//! bar, tree, status line, popups) and the editor ground, and carries the syntax
//! colours highlighting will use. Palettes are Hydra's, reused under the MIT
//! licence both projects share.

use std::collections::BTreeMap;

use ratatui::style::{Color, Modifier, Style};

/// Hydra's theme names, in Hydra's order; `Theme::named` knows each one.
#[cfg(test)]
pub const NAMES: &[&str] = &[
    "hydra",
    "papercolor-dark",
    "tango-dark",
    "monokai",
    "tokyo-night",
    "catppuccin-mocha",
    "catppuccin-latte",
    "gruvbox",
    "nord",
    "dracula",
    "mono",
];

/// The UI roles by their canonical names. `[theme_overrides]` also takes Hydra's
/// other names for them (see `Theme::slot`).
#[cfg(test)]
pub const UI_ROLES: &[&str] = &[
    "bg",
    "fg",
    "muted",
    "accent",
    "border",
    "border_active",
    "sidebar_bg",
    "selection_bg",
    "tab_active_bg",
    "tab_active_fg",
    "warning",
    "ok",
    "err",
    "card",
    "card2",
    "btn",
    "hov",
    "line",
    "text",
    "strong",
    "acc_ink",
];

/// The syntax roles by their override names.
#[cfg(test)]
pub const SYNTAX_ROLES: &[&str] = &[
    "keyword",
    "string",
    "comment",
    "function",
    "type",
    "number",
    "constant",
    "operator",
    "punctuation",
    "variable",
    "property",
    "tag",
    "attribute",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Editor ground.
    pub bg: Color,
    /// Editor text.
    pub fg: Color,
    /// Line numbers, hints, anything quieter than text.
    pub muted: Color,
    /// Focus and matched letters.
    pub accent: Color,
    pub border: Color,
    pub border_active: Color,
    /// Chrome: tab bar, tree, status line.
    pub sidebar_bg: Color,
    /// Selected text in the editor.
    pub selection_bg: Color,
    pub tab_active_bg: Color,
    pub tab_active_fg: Color,
    /// Hydra's `working`; diagnostics warnings later.
    pub warning: Color,
    /// Hydra's `done`.
    pub ok: Color,
    /// Hydra's `err` and `blocked`: failures and diagnostics errors.
    pub err: Color,
    /// Popup cards.
    pub card: Color,
    /// Input rows: the prompt bar and the picker's query.
    pub card2: Color,
    pub btn: Color,
    /// The keyboard cursor row in a list.
    pub hov: Color,
    /// Dividers.
    pub line: Color,
    /// Chrome text.
    pub text: Color,
    /// Names and titles.
    pub strong: Color,
    /// Text on the accent colour.
    pub acc_ink: Color,
    pub syntax: Syntax,
}

/// Highlight styles. Each is a foreground colour, plus bold or dim in `mono`,
/// which has no hues to spend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Syntax {
    pub keyword: Style,
    pub string: Style,
    pub comment: Style,
    pub function: Style,
    pub r#type: Style,
    pub number: Style,
    pub constant: Style,
    pub operator: Style,
    pub punctuation: Style,
    pub variable: Style,
    pub property: Style,
    pub tag: Style,
    pub attribute: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::hydra()
    }
}

/// A role an override can set.
enum Slot<'a> {
    Ui(&'a mut Color),
    Syntax(&'a mut Style),
}

/// `#rrggbb`, a 0-255 palette index, or a colour name (`red`, `light-blue`,
/// `reset`).
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(hex, 16).ok()?;
        let [_, r, g, b] = v.to_be_bytes();
        return Some(Color::Rgb(r, g, b));
    }
    if let Ok(n) = s.parse::<u8>() {
        return Some(Color::Indexed(n));
    }
    s.parse::<Color>().ok()
}

/// A colour written in the palettes below. They're literals checked by the unit
/// tests, so a typo shows up there as `Reset` rather than at runtime.
fn hex(s: &str) -> Color {
    parse_color(s).unwrap_or(Color::Reset)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            // Stays within 0..=255: it's a point between two u8 values.
            let m = |p: u8, q: u8| (f32::from(p) + (f32::from(q) - f32::from(p)) * t).round() as u8;
            Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
        }
        _ if t >= 0.5 => b,
        _ => a,
    }
}

/// The 13 syntax colours in `SYNTAX_ROLES` order.
fn syntax(c: [&str; 13]) -> Syntax {
    let s = |i: usize| Style::new().fg(hex(c[i]));
    Syntax {
        keyword: s(0),
        string: s(1),
        comment: s(2),
        function: s(3),
        r#type: s(4),
        number: s(5),
        constant: s(6),
        operator: s(7),
        punctuation: s(8),
        variable: s(9),
        property: s(10),
        tag: s(11),
        attribute: s(12),
    }
}

impl Theme {
    /// A filled highlight (active tab, selected row, selected text) as reverse
    /// video over swapped colours: it reads as `bg`/`fg` where the theme has
    /// colours, still shows where they are terminal defaults (`mono`), and is
    /// what the PTY tests look for as "selected".
    pub fn highlight(bg: Color, fg: Color) -> Style {
        Style::new().fg(bg).bg(fg).add_modifier(Modifier::REVERSED)
    }

    /// The theme called `name`, if there is one.
    pub fn named(name: &str) -> Option<Theme> {
        let theme = match name {
            "hydra" => Theme::hydra(),
            "papercolor-dark" => Theme::design(
                [
                    "#1c1c1c", "#d0d0d0", "#262626", "#303030", "#3a3a3a", "#444444", "#808080",
                    "#b2b2b2", "#eeeeee", "#00afaf", "#1c1c1c", "#ffaf00", "#5faf00", "#ff5f87",
                    "#3a3a3a", "#5f5faf",
                ],
                syntax([
                    "#ff5faf", "#d7af5f", "#808080", "#5fafd7", "#af87d7", "#ffaf00", "#ffaf00",
                    "#00afaf", "#b2b2b2", "#d0d0d0", "#5f8787", "#ff5f87", "#d7af5f",
                ]),
            ),
            "tango-dark" => Theme::design(
                [
                    "#2e3436", "#eeeeec", "#252a2b", "#363c3e", "#41474a", "#555753", "#9a9c97",
                    "#d3d7cf", "#eeeeec", "#8ae234", "#2e3436", "#fcaf3e", "#73d216", "#ff5c5c",
                    "#4a5052", "#204a87",
                ],
                syntax([
                    "#ad7fa8", "#8ae234", "#888a85", "#729fcf", "#34e2e2", "#fcaf3e", "#fcaf3e",
                    "#fce94f", "#d3d7cf", "#eeeeec", "#e9b96e", "#ef2929", "#fce94f",
                ]),
            ),
            "monokai" => Theme::design(
                [
                    "#272822", "#f8f8f2", "#1e1f1c", "#2f302a", "#3e3d32", "#49483e", "#8f8a72",
                    "#cfcfc2", "#f8f8f2", "#a6e22e", "#272822", "#fd971f", "#a6e22e", "#f92672",
                    "#3e3d32", "#55544a",
                ],
                syntax([
                    "#f92672", "#e6db74", "#75715e", "#a6e22e", "#66d9ef", "#ae81ff", "#ae81ff",
                    "#f92672", "#cfcfc2", "#f8f8f2", "#fd971f", "#f92672", "#a6e22e",
                ]),
            ),
            "tokyo-night" => Theme::design(
                [
                    "#1a1b26", "#c0caf5", "#16161e", "#1f2335", "#292e42", "#292e42", "#7a82ad",
                    "#a9b1d6", "#e0e6ff", "#7aa2f7", "#1a1b26", "#e0af68", "#9ece6a", "#f7768e",
                    "#292e42", "#3b4261",
                ],
                syntax([
                    "#bb9af7", "#9ece6a", "#565f89", "#7aa2f7", "#7dcfff", "#ff9e64", "#ff9e64",
                    "#89ddff", "#a9b1d6", "#c0caf5", "#73daca", "#f7768e", "#e0af68",
                ]),
            ),
            "catppuccin-mocha" => Theme::classic(
                [
                    "#1e1e2e", "#cdd6f4", "#6c7086", "#89b4fa", "#45475a", "#89b4fa", "#181825",
                    "#313244", "#89b4fa", "#1e1e2e", "#f9e2af", "#f38ba8", "#a6e3a1",
                ],
                syntax([
                    "#cba6f7", "#a6e3a1", "#9399b2", "#89b4fa", "#f9e2af", "#fab387", "#fab387",
                    "#89dceb", "#9399b2", "#cdd6f4", "#b4befe", "#f38ba8", "#f9e2af",
                ]),
            ),
            "catppuccin-latte" => Theme::classic(
                [
                    "#eff1f5", "#303446", "#6c6f85", "#7a2fd8", "#bcc0cc", "#7a2fd8", "#e6e9ef",
                    "#ccd0da", "#7a2fd8", "#eff1f5", "#8a5a00", "#d20f39", "#2f8a1f",
                ],
                syntax([
                    "#8839ef", "#40a02b", "#7c7f93", "#1e66f5", "#df8e1d", "#fe640b", "#fe640b",
                    "#04a5e5", "#7c7f93", "#303446", "#7287fd", "#d20f39", "#df8e1d",
                ]),
            ),
            "gruvbox" => Theme::classic(
                [
                    "#282828", "#ebdbb2", "#928374", "#fabd2f", "#504945", "#fabd2f", "#1d2021",
                    "#3c3836", "#fabd2f", "#282828", "#fe8019", "#fb4934", "#b8bb26",
                ],
                syntax([
                    "#fb4934", "#b8bb26", "#928374", "#8ec07c", "#fabd2f", "#d3869b", "#d3869b",
                    "#fe8019", "#a89984", "#ebdbb2", "#83a598", "#fe8019", "#fabd2f",
                ]),
            ),
            "nord" => Theme::classic(
                [
                    "#2e3440", "#eceff4", "#8390a8", "#a3be8c", "#434c5e", "#a3be8c", "#272c36",
                    "#3b4252", "#a3be8c", "#2e3440", "#ebcb8b", "#e0707a", "#a3be8c",
                ],
                syntax([
                    "#81a1c1", "#a3be8c", "#8390a8", "#88c0d0", "#8fbcbb", "#b48ead", "#b48ead",
                    "#81a1c1", "#d8dee9", "#d8dee9", "#88c0d0", "#81a1c1", "#8fbcbb",
                ]),
            ),
            "dracula" => Theme::classic(
                [
                    "#282a36", "#f8f8f2", "#7a8ac0", "#bd93f9", "#44475a", "#bd93f9", "#21222c",
                    "#44475a", "#bd93f9", "#282a36", "#f1fa8c", "#ff5555", "#50fa7b",
                ],
                syntax([
                    "#ff79c6", "#f1fa8c", "#7a8ac0", "#50fa7b", "#8be9fd", "#bd93f9", "#bd93f9",
                    "#ff79c6", "#f8f8f2", "#f8f8f2", "#ffb86c", "#ff79c6", "#50fa7b",
                ]),
            ),
            "mono" => Theme::mono(),
            _ => return None,
        };
        Some(theme)
    }

    /// The default: Hydra's own palette.
    fn hydra() -> Theme {
        Theme::design(
            [
                "#070b10", "#c9d1d9", "#0c131b", "#0f1821", "#18242f", "#1f2c3a", "#71808f",
                "#a7b4c2", "#f2f6f8", "#c3f53c", "#0a1204", "#ffb547", "#7fd962", "#ff6b6b",
                "#1d2a37", "#2a3a4c",
            ],
            syntax([
                "#a593ff", "#7fd962", "#71808f", "#5aa9ff", "#3dd6c0", "#ffb547", "#ffb547",
                "#ff7ab6", "#a7b4c2", "#c9d1d9", "#e8c565", "#ff7ab6", "#e8c565",
            ]),
        )
    }

    /// A theme from Hydra's design roles, in Hydra's order:
    /// bg fg surf card card2 line dim text strong acc accInk needs ok err btn hov.
    /// `needs` (amber) becomes `warning`; `err` is Hydra's red.
    fn design(c: [&str; 16], syntax: Syntax) -> Theme {
        let h = |i: usize| hex(c[i]);
        Theme {
            bg: h(0),
            fg: h(1),
            muted: h(6),
            accent: h(9),
            border: h(5),
            border_active: h(9),
            sidebar_bg: h(2),
            selection_bg: h(15),
            tab_active_bg: h(9),
            tab_active_fg: h(10),
            warning: h(11),
            ok: h(12),
            err: h(13),
            card: h(3),
            card2: h(4),
            btn: h(14),
            hov: h(15),
            line: h(5),
            text: h(7),
            strong: h(8),
            acc_ink: h(10),
            syntax,
        }
    }

    /// A theme from Hydra's classic colours: bg fg muted accent border
    /// border_active sidebar_bg selection_bg tab_active_bg tab_active_fg working
    /// blocked done. The newer roles are mixed from them, as Hydra does.
    fn classic(c: [&str; 13], syntax: Syntax) -> Theme {
        let (bg, fg, muted, border) = (hex(c[0]), hex(c[1]), hex(c[2]), hex(c[4]));
        Theme {
            bg,
            fg,
            muted,
            accent: hex(c[3]),
            border,
            border_active: hex(c[5]),
            sidebar_bg: hex(c[6]),
            selection_bg: hex(c[7]),
            tab_active_bg: hex(c[8]),
            tab_active_fg: hex(c[9]),
            warning: hex(c[10]),
            err: hex(c[11]),
            ok: hex(c[12]),
            card: mix(bg, fg, 0.05),
            card2: mix(bg, fg, 0.11),
            btn: mix(bg, fg, 0.14),
            hov: hex(c[7]),
            line: border,
            text: mix(muted, fg, 0.5),
            strong: fg,
            acc_ink: bg,
            syntax,
        }
    }

    /// The terminal's own colours plus greys; syntax is told apart by weight.
    fn mono() -> Theme {
        let plain = Style::new().fg(Color::Reset);
        let bold = plain.add_modifier(Modifier::BOLD);
        let dim = plain.add_modifier(Modifier::DIM);
        Theme {
            bg: Color::Reset,
            fg: Color::Reset,
            muted: Color::DarkGray,
            accent: Color::White,
            border: Color::DarkGray,
            border_active: Color::White,
            sidebar_bg: Color::Reset,
            selection_bg: Color::DarkGray,
            tab_active_bg: Color::White,
            tab_active_fg: Color::Black,
            warning: Color::Yellow,
            ok: Color::Green,
            err: Color::Red,
            card: Color::Black,
            card2: Color::DarkGray,
            btn: Color::DarkGray,
            hov: Color::DarkGray,
            line: Color::DarkGray,
            text: Color::Gray,
            strong: Color::White,
            acc_ink: Color::Black,
            syntax: Syntax {
                keyword: bold,
                string: plain,
                comment: dim,
                function: bold,
                r#type: bold,
                number: plain,
                constant: bold,
                operator: plain,
                punctuation: dim,
                variable: plain,
                property: plain,
                tag: bold,
                attribute: dim,
            },
        }
    }

    /// The role an override key names. Takes the canonical names, `-` for `_`,
    /// and Hydra's names for the roles Griffin renamed or folded together.
    fn slot(&mut self, key: &str) -> Option<Slot<'_>> {
        let ui = match key.replace('-', "_").as_str() {
            "bg" => &mut self.bg,
            "fg" => &mut self.fg,
            "muted" | "dim" | "idle" => &mut self.muted,
            "accent" | "acc" => &mut self.accent,
            "border" => &mut self.border,
            "border_active" => &mut self.border_active,
            "sidebar_bg" | "surf" => &mut self.sidebar_bg,
            "selection_bg" => &mut self.selection_bg,
            "tab_active_bg" => &mut self.tab_active_bg,
            "tab_active_fg" => &mut self.tab_active_fg,
            "warning" | "working" => &mut self.warning,
            "ok" | "done" => &mut self.ok,
            "err" | "blocked" | "needs" => &mut self.err,
            "card" => &mut self.card,
            "card2" => &mut self.card2,
            "btn" => &mut self.btn,
            "hov" => &mut self.hov,
            "line" => &mut self.line,
            "text" => &mut self.text,
            "strong" => &mut self.strong,
            "acc_ink" => &mut self.acc_ink,
            other => {
                let s = &mut self.syntax;
                let style = match other {
                    "keyword" => &mut s.keyword,
                    "string" => &mut s.string,
                    "comment" => &mut s.comment,
                    "function" => &mut s.function,
                    "type" => &mut s.r#type,
                    "number" => &mut s.number,
                    "constant" => &mut s.constant,
                    "operator" => &mut s.operator,
                    "punctuation" => &mut s.punctuation,
                    "variable" => &mut s.variable,
                    "property" => &mut s.property,
                    "tag" => &mut s.tag,
                    "attribute" => &mut s.attribute,
                    _ => return None,
                };
                return Some(Slot::Syntax(style));
            }
        };
        Some(Slot::Ui(ui))
    }

    /// Sets each overridden role, or says what's wrong with the first bad entry.
    fn apply(&mut self, overrides: &BTreeMap<String, toml::Value>) -> Result<(), String> {
        for (key, value) in overrides {
            let color = match value {
                toml::Value::String(s) => parse_color(s),
                toml::Value::Integer(n) => u8::try_from(*n).ok().map(Color::Indexed),
                _ => None,
            }
            .ok_or_else(|| format!("bad colour {value} for {key}"))?;
            match self.slot(key) {
                Some(Slot::Ui(slot)) => *slot = color,
                // Keeps mono's bold/dim; only the colour is overridden.
                Some(Slot::Syntax(style)) => *style = style.fg(color),
                None => return Err(format!("unknown colour {key}")),
            }
        }
        Ok(())
    }
}

/// The theme `config.toml` asks for. An unknown name or a bad override gives plain
/// `hydra` instead, with one message for the status line, so a typo is visible
/// rather than half-applied.
pub fn load(
    name: Option<&str>,
    overrides: &BTreeMap<String, toml::Value>,
) -> (Theme, Option<String>) {
    let name = name.unwrap_or("hydra");
    let Some(mut theme) = Theme::named(name) else {
        return (
            Theme::hydra(),
            Some(format!("theme: unknown theme \"{name}\", using hydra")),
        );
    };
    match theme.apply(overrides) {
        Ok(()) => (theme, None),
        Err(err) => (Theme::hydra(), Some(format!("theme: {err}, using hydra"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overrides(toml: &str) -> BTreeMap<String, toml::Value> {
        toml::from_str(toml).expect("test overrides are valid toml")
    }

    fn get(theme: &Theme, role: &str) -> Option<Style> {
        let mut theme = theme.clone();
        match theme.slot(role)? {
            Slot::Ui(color) => Some(Style::new().fg(*color)),
            Slot::Syntax(style) => Some(*style),
        }
    }

    #[test]
    fn every_name_loads() {
        assert_eq!(NAMES.len(), 11);
        for name in NAMES {
            assert!(Theme::named(name).is_some(), "{name}");
            let (_, message) = load(Some(name), &BTreeMap::new());
            assert_eq!(message, None, "{name}");
        }
    }

    #[test]
    fn themes_differ() {
        for (i, a) in NAMES.iter().enumerate() {
            for b in &NAMES[i + 1..] {
                assert_ne!(Theme::named(a), Theme::named(b), "{a} and {b}");
            }
        }
    }

    #[test]
    fn no_role_is_unset_in_any_theme() {
        for name in NAMES {
            let theme = Theme::named(name).expect("listed themes exist");
            for role in UI_ROLES {
                let color = get(&theme, role)
                    .and_then(|style| style.fg)
                    .unwrap_or_else(|| panic!("{name}: no {role}"));
                // `Reset` is how a mistyped palette literal shows up; only mono
                // means it, for the terminal's own colours.
                if *name != "mono" {
                    assert_ne!(color, Color::Reset, "{name}: {role}");
                }
            }
            for role in SYNTAX_ROLES {
                let style = get(&theme, role).unwrap_or_else(|| panic!("{name}: no {role}"));
                assert!(style.fg.is_some(), "{name}: {role} has no colour");
                if *name != "mono" {
                    assert!(
                        matches!(style.fg, Some(Color::Rgb(..))),
                        "{name}: {role} is {style:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn mono_tells_syntax_apart_by_weight_not_hue() {
        let mono = Theme::named("mono").expect("mono exists");
        assert!(mono.syntax.keyword.add_modifier.contains(Modifier::BOLD));
        assert!(mono.syntax.comment.add_modifier.contains(Modifier::DIM));
        for role in SYNTAX_ROLES {
            let style = get(&mono, role).expect("every role exists");
            assert_eq!(style.fg, Some(Color::Reset), "{role}");
        }
    }

    #[test]
    fn overrides_take_hex_index_and_names_for_any_role() {
        let (theme, message) = load(
            Some("nord"),
            &overrides(
                r##"
                sidebar_bg = "#123456"
                keyword = 141
                comment = "203"
                accent = "light-red"
                border-active = "Blue"
                surf = "#000001"
                "##,
            ),
        );
        assert_eq!(message, None);
        // `surf` is Hydra's name for `sidebar_bg`; map order puts it last.
        assert_eq!(theme.sidebar_bg, Color::Rgb(0, 0, 1));
        assert_eq!(theme.syntax.keyword.fg, Some(Color::Indexed(141)));
        assert_eq!(theme.syntax.comment.fg, Some(Color::Indexed(203)));
        assert_eq!(theme.accent, Color::LightRed);
        assert_eq!(theme.border_active, Color::Blue);
        // Untouched roles keep nord's values.
        let nord = Theme::named("nord").expect("nord exists");
        assert_eq!(theme.bg, nord.bg);
        assert_eq!(theme.syntax.string, nord.syntax.string);
    }

    #[test]
    fn every_role_can_be_overridden() {
        for role in UI_ROLES.iter().chain(SYNTAX_ROLES) {
            let (theme, message) = load(None, &overrides(&format!("{role} = \"#010203\"")));
            assert_eq!(message, None, "{role}");
            let style = get(&theme, role).expect("every role exists");
            assert_eq!(style.fg, Some(Color::Rgb(1, 2, 3)), "{role}");
        }
    }

    #[test]
    fn hydra_role_names_are_accepted() {
        for key in ["working", "blocked", "needs", "done", "idle", "dim", "acc"] {
            let (_, message) = load(None, &overrides(&format!("{key} = \"red\"")));
            assert_eq!(message, None, "{key}");
        }
    }

    #[test]
    fn overriding_a_mono_syntax_colour_keeps_its_weight() {
        let (theme, _) = load(Some("mono"), &overrides("keyword = \"red\""));
        assert_eq!(theme.syntax.keyword.fg, Some(Color::Red));
        assert!(theme.syntax.keyword.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn unknown_theme_falls_back_to_hydra_with_one_message() {
        let (theme, message) = load(Some("solarized"), &BTreeMap::new());
        assert_eq!(theme, Theme::default());
        assert_eq!(
            message.as_deref(),
            Some("theme: unknown theme \"solarized\", using hydra")
        );
    }

    #[test]
    fn bad_values_fall_back_to_hydra_with_one_message() {
        for (toml, expected) in [
            (
                r##"bg = "#12345""##,
                r##"theme: bad colour "#12345" for bg, using hydra"##,
            ),
            (
                r#"bg = "blurple""#,
                r#"theme: bad colour "blurple" for bg, using hydra"#,
            ),
            (
                "keyword = 256",
                "theme: bad colour 256 for keyword, using hydra",
            ),
            (
                "keyword = -1",
                "theme: bad colour -1 for keyword, using hydra",
            ),
            (
                "keyword = true",
                "theme: bad colour true for keyword, using hydra",
            ),
            (
                r#"backgrnd = "red""#,
                "theme: unknown colour backgrnd, using hydra",
            ),
        ] {
            let (theme, message) = load(Some("nord"), &overrides(&format!("{toml}\nfg = \"red\"")));
            assert_eq!(theme, Theme::default(), "{toml}");
            assert_eq!(message.as_deref(), Some(expected), "{toml}");
        }
    }

    #[test]
    fn parse_color_forms() {
        assert_eq!(parse_color("#c3f53c"), Some(Color::Rgb(0xc3, 0xf5, 0x3c)));
        assert_eq!(parse_color(" 7 "), Some(Color::Indexed(7)));
        assert_eq!(parse_color("dark-gray"), Some(Color::DarkGray));
        assert_eq!(parse_color("#fff"), None);
        assert_eq!(parse_color("#gggggg"), None);
        assert_eq!(parse_color("nope"), None);
    }

    #[test]
    fn highlight_swaps_colours_under_reverse_video() {
        let style = Theme::highlight(Color::Indexed(1), Color::Indexed(2));
        assert_eq!(style.fg, Some(Color::Indexed(1)));
        assert_eq!(style.bg, Some(Color::Indexed(2)));
        assert!(style.add_modifier.contains(Modifier::REVERSED));
    }
}
