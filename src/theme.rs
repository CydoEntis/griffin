//! Tome's 13 themes and `[theme_overrides]`. A theme paints Tome's chrome (tab
//! bar, tree, status line, popups) and the editor ground, and carries the syntax
//! colours highlighting will use. `aurora` and `moonlit` are hand-tuned; the pack
//! palettes are Hydra's, reused under the MIT licence both projects share, with
//! their Aurora roles derived as the design's `fromHydra` does.

use std::collections::BTreeMap;

use ratatui::style::{Color, Modifier, Style};

use crate::highlight::Role;

/// The signature themes, then Hydra's in Hydra's order; `Theme::named` knows
/// each one.
#[cfg(test)]
pub const NAMES: &[&str] = &[
    "aurora",
    "moonlit",
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
    "deep",
    "surface",
    "raised",
    "raised2",
    "line2",
    "guide",
    "accent2",
    "warn",
    "info",
    "err_soft",
    "warn_soft",
    "info_soft",
    "sel",
    "cur_line",
    "gutter",
    "scrim",
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
    // The Aurora roles (design README §4). Where a v1 field above means the same
    // thing (README §6) the two hold the same colour, so screens not yet restyled
    // and restyled ones agree; overrides set both.
    /// Below the editor ground: the deepest panel.
    pub deep: Color,
    /// Chrome ground: tree, status bar, run title. v1 `sidebar_bg`.
    pub surface: Color,
    /// Dialogs and popups. v1 `card`.
    pub raised: Color,
    /// Input rows and the active tab pill. v1 `card2`.
    pub raised2: Color,
    /// Popup borders and rules. v1 `border`.
    pub line2: Color,
    /// Indent guides and the split rule. v1 `line`.
    pub guide: Color,
    /// The second aurora colour; glows run from `accent` to it.
    pub accent2: Color,
    /// Warnings, running, dirty marks. v1 `warning`.
    pub warn: Color,
    /// Info and hint diagnostics.
    pub info: Color,
    /// Severity colours softened towards `bg`, for the inline diagnostic lens.
    pub err_soft: Color,
    pub warn_soft: Color,
    pub info_soft: Color,
    /// Selected text. v1 `selection_bg`.
    pub sel: Color,
    /// The cursor line where the glow has faded.
    pub cur_line: Color,
    /// Line numbers.
    pub gutter: Color,
    /// What dimmed screens are mixed towards behind a dialog.
    pub scrim: Color,
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

impl Syntax {
    /// The style text in `role` is drawn with.
    pub fn style(&self, role: Role) -> Style {
        match role {
            Role::Keyword => self.keyword,
            Role::String => self.string,
            Role::Comment => self.comment,
            Role::Function => self.function,
            Role::Type => self.r#type,
            Role::Number => self.number,
            Role::Constant => self.constant,
            Role::Operator => self.operator,
            Role::Punctuation => self.punctuation,
            Role::Variable => self.variable,
            Role::Property => self.property,
            Role::Tag => self.tag,
            Role::Attribute => self.attribute,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::hydra()
    }
}

/// A role an override can set. A UI role is one or more fields: the role itself
/// first, then the v1 or Aurora fields that mean the same thing.
enum Slot<'a> {
    Ui(Vec<&'a mut Color>),
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

/// `a` moved `t` of the way to `b`, per RGB channel, rounded (design README §1).
/// `t` is clamped to 0..=1. A colour that isn't RGB (a terminal colour) can't be
/// blended, so the nearer end is taken.
pub fn mix(a: Color, b: Color, t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            // f64 like the design's JS, so a .5 lands on the same side. With `t`
            // clamped the value lies between two u8s, so the cast can't wrap.
            let m = |p: u8, q: u8| (f64::from(p) + (f64::from(q) - f64::from(p)) * t).round() as u8;
            Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
        }
        _ if t >= 0.5 => b,
        _ => a,
    }
}

/// A ramp through `stops`, evenly spaced over `t` in 0..=1 (clamped), mixing
/// between neighbours (design README §1). Every Aurora glow is one of these.
pub fn grad(stops: &[Color], t: f64) -> Color {
    match stops {
        [] => Color::Reset,
        [only] => *only,
        _ => {
            let n = stops.len() - 1;
            let pos = t.clamp(0.0, 1.0) * n as f64;
            // `pos` is in 0..=n, so the floor fits; `min` keeps t = 1 on the last
            // segment rather than past it.
            let i = (pos.floor() as usize).min(n - 1);
            mix(stops[i], stops[i + 1], pos - i as f64)
        }
    }
}

/// Perceived lightness of an RGB colour, 0..=1; a non-RGB colour counts as dark.
fn luminance(c: Color) -> f64 {
    match c {
        Color::Rgb(r, g, b) => {
            (0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)) / 255.0
        }
        _ => 0.0,
    }
}

/// Straight-line distance between two RGB colours; non-RGB counts as black.
fn rgb_distance(a: Color, b: Color) -> f64 {
    let rgb = |c: Color| match c {
        Color::Rgb(r, g, b) => [f64::from(r), f64::from(g), f64::from(b)],
        _ => [0.0; 3],
    };
    let (a, b) = (rgb(a), rgb(b));
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// A pack theme's v1 roles, before the Aurora roles are derived from them.
struct Hydra {
    bg: Color,
    fg: Color,
    muted: Color,
    accent: Color,
    border: Color,
    border_active: Color,
    sidebar_bg: Color,
    selection_bg: Color,
    tab_active_bg: Color,
    tab_active_fg: Color,
    warning: Color,
    ok: Color,
    err: Color,
    card: Color,
    card2: Color,
    btn: Color,
    hov: Color,
    line: Color,
    text: Color,
    strong: Color,
    acc_ink: Color,
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

    /// Whether the theme's colours can be blended into ramps and dimmed towards
    /// its scrim. `mono` uses the terminal's own colours, which can't be mixed, so
    /// it draws glows as reverse video and the scrim as the DIM modifier instead.
    pub fn ramps(&self) -> bool {
        matches!(self.scrim, Color::Rgb(..))
    }

    /// Whether the glows can't be drawn: `mono` uses terminal colours, which
    /// `mix` can't blend, so it draws flat with bold and reverse video instead
    /// (README §4).
    pub fn flat(&self) -> bool {
        !matches!(
            (self.surface, self.accent, self.accent2),
            (Color::Rgb(..), Color::Rgb(..), Color::Rgb(..))
        )
    }

    /// The theme called `name`, if there is one.
    pub fn named(name: &str) -> Option<Theme> {
        let theme = match name {
            "aurora" => Theme::signature(
                [
                    "#0b0a10", "#07060b", "#0f0e16", "#16141f", "#1e1b2a", "#2e2a40", "#1f1c2b",
                    "#67627d", "#a6a2bb", "#d2cfe2", "#f4f2fb", "#b69cff", "#6ee7d8", "#120a24",
                    "#ff6f91", "#f0cf7a", "#7fe3c9", "#8fd0ff", "#b05a76", "#a38d58", "#6a8db0",
                    "#221c35", "#13111c", "#3a3550", "#000000",
                ],
                syntax([
                    "#b69cff", "#7fe3c9", "#5c5773", "#ece4ff", "#8fd0ff", "#f0cf7a", "#f0cf7a",
                    "#7d7896", "#5d5873", "#d2cfe2", "#bab5d1", "#b69cff", "#7d7896",
                ]),
            ),
            "moonlit" => Theme::signature(
                [
                    "#090b10", "#06080c", "#0d1017", "#141822", "#1b2030", "#2a3246", "#1a1f2b",
                    "#5f6a82", "#a2abbf", "#d0d6e4", "#f2f5fb", "#a9c6ff", "#f0cf7a", "#0b1426",
                    "#ff6f91", "#f0cf7a", "#7fe3c9", "#a9c6ff", "#b05a76", "#a38d58", "#6a8db0",
                    "#1c2538", "#10131b", "#343c50", "#000000",
                ],
                syntax([
                    "#a9c6ff", "#e6d29c", "#56607a", "#eef3ff", "#8fd8ff", "#f0cf7a", "#f0cf7a",
                    "#77819a", "#596379", "#d0d6e4", "#c0c9dc", "#a9c6ff", "#77819a",
                ]),
            ),
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
        Theme::pack(
            Hydra {
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
            },
            syntax,
        )
    }

    /// A theme from Hydra's classic colours: bg fg muted accent border
    /// border_active sidebar_bg selection_bg tab_active_bg tab_active_fg working
    /// blocked done. The newer roles are mixed from them, as Hydra does.
    fn classic(c: [&str; 13], syntax: Syntax) -> Theme {
        let (bg, fg, muted, border) = (hex(c[0]), hex(c[1]), hex(c[2]), hex(c[4]));
        Theme::pack(
            Hydra {
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
            },
            syntax,
        )
    }

    /// A pack theme: its v1 roles plus the Aurora roles derived from them as the
    /// design's `fromHydra` does (README §4). The v1 roles the palettes never had
    /// (`current_line_bg`, `gutter_fg`, `info`, `scrim`) are derived first, as
    /// SPEC_V1_LAYOUT §13 says.
    fn pack(h: Hydra, syntax: Syntax) -> Theme {
        let dark = luminance(h.bg) < 0.5;
        let fg_of = |s: Style| s.fg.unwrap_or(Color::Reset);
        // The aurora needs a second colour well apart from the accent; the
        // palette's own syntax hues are where it comes from. Only a strictly
        // farther colour wins, so a tie keeps the earlier one, as the design's
        // stable sort does.
        let mut accent2 = fg_of(syntax.r#type);
        for c in [syntax.function, syntax.string, syntax.keyword].map(fg_of) {
            if rgb_distance(c, h.accent) > rgb_distance(accent2, h.accent) {
                accent2 = c;
            }
        }
        let info = fg_of(syntax.function);
        let black = Color::Rgb(0, 0, 0);
        Theme {
            deep: mix(h.bg, if dark { black } else { h.fg }, 0.15),
            surface: h.sidebar_bg,
            raised: h.card,
            raised2: h.card2,
            line2: h.border,
            guide: mix(h.sidebar_bg, h.line, 0.6),
            accent2,
            warn: h.warning,
            info,
            err_soft: mix(h.err, h.bg, 0.35),
            warn_soft: mix(h.warning, h.bg, 0.35),
            info_soft: mix(info, h.bg, 0.35),
            sel: h.selection_bg,
            cur_line: mix(h.bg, h.fg, 0.06),
            gutter: mix(h.muted, h.bg, 0.3),
            scrim: if dark {
                mix(h.bg, black, 0.5)
            } else {
                mix(h.bg, h.fg, 0.3)
            },
            bg: h.bg,
            fg: h.fg,
            muted: h.muted,
            accent: h.accent,
            border: h.border,
            border_active: h.border_active,
            sidebar_bg: h.sidebar_bg,
            selection_bg: h.selection_bg,
            tab_active_bg: h.tab_active_bg,
            tab_active_fg: h.tab_active_fg,
            warning: h.warning,
            ok: h.ok,
            err: h.err,
            card: h.card,
            card2: h.card2,
            btn: h.btn,
            hov: h.hov,
            line: h.line,
            text: h.text,
            strong: h.strong,
            acc_ink: h.acc_ink,
            syntax,
        }
    }

    /// A hand-tuned Aurora theme, in README §4.1's row order: bg deep surface
    /// raised raised2 line2 guide muted text fg strong accent accent2 acc_ink err
    /// warn ok info err_soft warn_soft info_soft sel cur_line gutter scrim. Each
    /// v1 role takes the Aurora role README §6 maps it to; `tab_active_*` keep the
    /// v1 accent fill, which only `mono`'s reversed pill still reads.
    fn signature(c: [&str; 25], syntax: Syntax) -> Theme {
        let h = |i: usize| hex(c[i]);
        Theme {
            bg: h(0),
            fg: h(9),
            muted: h(7),
            accent: h(11),
            border: h(5),
            border_active: h(11),
            sidebar_bg: h(2),
            selection_bg: h(21),
            tab_active_bg: h(11),
            tab_active_fg: h(13),
            warning: h(15),
            ok: h(16),
            err: h(14),
            card: h(3),
            card2: h(4),
            btn: h(4),
            hov: h(21),
            line: h(6),
            text: h(8),
            strong: h(10),
            acc_ink: h(13),
            deep: h(1),
            surface: h(2),
            raised: h(3),
            raised2: h(4),
            line2: h(5),
            guide: h(6),
            accent2: h(12),
            warn: h(15),
            info: h(17),
            err_soft: h(18),
            warn_soft: h(19),
            info_soft: h(20),
            sel: h(21),
            cur_line: h(22),
            gutter: h(23),
            scrim: h(24),
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
            // SPEC_V1_LAYOUT §13's mono column. Mono has no ramps, so the second
            // aurora colour is the accent and the soft severities are the plain
            // ones; the cursor line number's weight stands in for `cur_line`, and
            // dialogs dim with the DIM modifier rather than towards a scrim.
            deep: Color::Reset,
            surface: Color::Reset,
            raised: Color::Black,
            raised2: Color::DarkGray,
            line2: Color::DarkGray,
            guide: Color::DarkGray,
            accent2: Color::White,
            warn: Color::Yellow,
            info: Color::Blue,
            err_soft: Color::Red,
            warn_soft: Color::Yellow,
            info_soft: Color::Blue,
            sel: Color::DarkGray,
            cur_line: Color::Reset,
            gutter: Color::DarkGray,
            scrim: Color::Reset,
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
    /// Hydra's names for the roles Tome renamed or folded together, and the v1
    /// and Aurora names for one another (README §6): overriding either sets both,
    /// so screens drawn with the v1 fields and restyled ones agree.
    fn slot(&mut self, key: &str) -> Option<Slot<'_>> {
        let ui = match key.replace('-', "_").as_str() {
            "bg" => vec![&mut self.bg],
            "fg" => vec![&mut self.fg],
            "muted" | "dim" | "idle" => vec![&mut self.muted],
            "accent" | "acc" => vec![&mut self.accent],
            "accent2" => vec![&mut self.accent2],
            "border" => vec![&mut self.border, &mut self.line2],
            "line2" => vec![&mut self.line2, &mut self.border],
            "border_active" => vec![&mut self.border_active],
            "sidebar_bg" | "surf" => vec![&mut self.sidebar_bg, &mut self.surface],
            "surface" => vec![&mut self.surface, &mut self.sidebar_bg],
            "selection_bg" => vec![&mut self.selection_bg, &mut self.sel],
            "hov" => vec![&mut self.hov, &mut self.sel],
            "sel" => vec![&mut self.sel, &mut self.selection_bg, &mut self.hov],
            "tab_active_bg" => vec![&mut self.tab_active_bg, &mut self.raised2],
            "tab_active_fg" => vec![&mut self.tab_active_fg],
            "warning" | "working" => vec![&mut self.warning, &mut self.warn],
            "warn" => vec![&mut self.warn, &mut self.warning],
            "ok" | "done" => vec![&mut self.ok],
            "err" | "blocked" | "needs" => vec![&mut self.err],
            "info" => vec![&mut self.info],
            "err_soft" => vec![&mut self.err_soft],
            "warn_soft" => vec![&mut self.warn_soft],
            "info_soft" => vec![&mut self.info_soft],
            "card" => vec![&mut self.card, &mut self.raised],
            "raised" => vec![&mut self.raised, &mut self.card],
            "card2" => vec![&mut self.card2, &mut self.raised2],
            "raised2" => vec![&mut self.raised2, &mut self.card2],
            "btn" => vec![&mut self.btn],
            "line" => vec![&mut self.line, &mut self.guide],
            "guide" => vec![&mut self.guide, &mut self.line],
            "deep" => vec![&mut self.deep],
            "cur_line" | "current_line_bg" => vec![&mut self.cur_line],
            "gutter" | "gutter_fg" => vec![&mut self.gutter],
            "scrim" => vec![&mut self.scrim],
            "text" => vec![&mut self.text],
            "strong" => vec![&mut self.strong],
            "acc_ink" => vec![&mut self.acc_ink],
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
                Some(Slot::Ui(slots)) => {
                    for slot in slots {
                        *slot = color;
                    }
                }
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
    match resolve(name, overrides) {
        Ok(theme) => (theme, None),
        Err(err) => (Theme::hydra(), Some(format!("{err}, using hydra"))),
    }
}

/// The theme `config.toml` asks for, or why it can't be had. Applying a saved
/// config keeps the theme in use on an error, so it mustn't fall back itself.
pub fn resolve(
    name: Option<&str>,
    overrides: &BTreeMap<String, toml::Value>,
) -> Result<Theme, String> {
    let name = name.unwrap_or("hydra");
    let mut theme = Theme::named(name).ok_or_else(|| format!("theme: unknown theme \"{name}\""))?;
    theme
        .apply(overrides)
        .map_err(|err| format!("theme: {err}"))?;
    Ok(theme)
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
            // The first field is the role the key names.
            Slot::Ui(colors) => Some(Style::new().fg(*colors[0])),
            Slot::Syntax(style) => Some(*style),
        }
    }

    #[test]
    fn every_name_loads() {
        assert_eq!(NAMES.len(), 13);
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
        // README §6: each v1 name sets the Aurora role it became, and the other
        // way round, so old and restyled screens agree.
        for (v1, aurora) in [
            ("sidebar_bg", "surface"),
            ("surf", "surface"),
            ("card", "raised"),
            ("card2", "raised2"),
            ("tab_active_bg", "raised2"),
            ("border", "line2"),
            ("line", "guide"),
            ("hov", "sel"),
            ("selection_bg", "sel"),
            ("working", "warn"),
            ("warning", "warn"),
            ("done", "ok"),
            ("current_line_bg", "cur_line"),
            ("gutter_fg", "gutter"),
        ] {
            let (theme, message) = load(None, &overrides(&format!("{v1} = \"#010203\"")));
            assert_eq!(message, None, "{v1}");
            let got = get(&theme, aurora).and_then(|style| style.fg);
            assert_eq!(got, Some(Color::Rgb(1, 2, 3)), "{v1} -> {aurora}");
        }
        for (aurora, v1) in [
            ("surface", "sidebar_bg"),
            ("raised", "card"),
            ("raised2", "card2"),
            ("line2", "border"),
            ("guide", "line"),
            ("sel", "selection_bg"),
            ("sel", "hov"),
            ("warn", "warning"),
        ] {
            let (theme, _) = load(None, &overrides(&format!("{aurora} = \"#010203\"")));
            let got = get(&theme, v1).and_then(|style| style.fg);
            assert_eq!(got, Some(Color::Rgb(1, 2, 3)), "{aurora} -> {v1}");
        }
    }

    #[test]
    fn signature_themes_load_and_hydra_stays_the_default() {
        assert!(Theme::named("aurora").is_some());
        assert!(Theme::named("moonlit").is_some());
        assert_eq!(
            Theme::default(),
            Theme::named("hydra").expect("hydra exists")
        );
        let (theme, message) = load(None, &BTreeMap::new());
        assert_eq!(message, None);
        assert_eq!(theme, Theme::default());
    }

    /// README §4.1 and §4.2 as (theme, role, colour) triples, read from the
    /// design doc itself so the test can't drift from it.
    fn readme_tables() -> Vec<(String, String, Color)> {
        const README: &str = include_str!("../docs/features/tome-aurora/design/README.md");
        let start = README
            .find("### 4.1")
            .expect("README has the chrome roles table");
        let end = README
            .find("## 5.")
            .expect("README has a section after the tables");
        let cells = |line: &str| -> Vec<String> {
            line.trim()
                .trim_matches('|')
                .split('|')
                .map(|c| c.trim().trim_matches('`').to_string())
                .collect()
        };
        let mut themes = Vec::new();
        let mut out = Vec::new();
        for line in README[start..end].lines() {
            if !line.starts_with('|') || line.starts_with("|---") {
                continue;
            }
            let row = cells(line);
            if row[0] == "role" {
                themes = row[1..].to_vec();
                continue;
            }
            let role = row[0].trim_start_matches("syn.").to_string();
            for (theme, value) in themes.iter().zip(&row[1..]) {
                let color =
                    parse_color(value).unwrap_or_else(|| panic!("{theme} {role}: bad hex {value}"));
                out.push((theme.clone(), role.clone(), color));
            }
        }
        out
    }

    #[test]
    fn every_theme_matches_the_readme_tables() {
        let rows = readme_tables();
        // 25 chrome roles and 13 syntax roles for 12 themes.
        assert_eq!(rows.len(), (25 + 13) * 12);
        for (name, role, want) in rows {
            let theme = Theme::named(&name).unwrap_or_else(|| panic!("no theme {name}"));
            let got = get(&theme, &role)
                .unwrap_or_else(|| panic!("{name}: no role {role}"))
                .fg;
            assert_eq!(got, Some(want), "{name}: {role}");
        }
    }

    #[test]
    fn accent2_is_the_syntax_hue_farthest_from_accent() {
        for name in NAMES
            .iter()
            .filter(|n| !["aurora", "moonlit", "mono"].contains(n))
        {
            let theme = Theme::named(name).expect("listed themes exist");
            let s = &theme.syntax;
            let candidates = [s.r#type, s.function, s.string, s.keyword].map(|st| st.fg);
            let best = candidates
                .iter()
                .flatten()
                .map(|c| rgb_distance(*c, theme.accent))
                .fold(0.0, f64::max);
            assert_eq!(rgb_distance(theme.accent2, theme.accent), best, "{name}");
            assert!(candidates.contains(&Some(theme.accent2)), "{name}");
        }
    }

    #[test]
    fn mono_aurora_roles_are_terminal_colours() {
        let mono = Theme::named("mono").expect("mono exists");
        assert_eq!(mono.accent2, mono.accent);
        assert_eq!(mono.gutter, Color::DarkGray);
        assert_eq!(mono.cur_line, Color::Reset);
        assert_eq!(mono.info, Color::Blue);
    }

    #[test]
    fn only_mono_is_flat() {
        for name in NAMES {
            let theme = Theme::named(name).expect("listed themes exist");
            assert_eq!(theme.flat(), *name == "mono", "{name}");
        }
    }

    #[test]
    fn mix_is_a_rounded_clamped_lerp() {
        let a = Color::Rgb(0, 100, 255);
        let b = Color::Rgb(255, 0, 0);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        // 127.5 rounds up, 50 exactly, 127.5 rounds up.
        assert_eq!(mix(a, b, 0.5), Color::Rgb(128, 50, 128));
        assert_eq!(mix(a, b, -1.0), a);
        assert_eq!(mix(a, b, 2.0), b);
        // Terminal colours can't blend: the nearer end wins.
        assert_eq!(mix(Color::Red, b, 0.4), Color::Red);
        assert_eq!(mix(Color::Red, b, 0.6), b);
    }

    #[test]
    fn grad_spaces_stops_evenly_and_clamps() {
        let (r, g, b) = (
            Color::Rgb(255, 0, 0),
            Color::Rgb(0, 255, 0),
            Color::Rgb(0, 0, 255),
        );
        let stops = [r, g, b];
        assert_eq!(grad(&stops, 0.0), r);
        assert_eq!(grad(&stops, 0.5), g);
        assert_eq!(grad(&stops, 1.0), b);
        assert_eq!(grad(&stops, 0.25), Color::Rgb(128, 128, 0));
        assert_eq!(grad(&stops, 0.75), Color::Rgb(0, 128, 128));
        assert_eq!(grad(&stops, -0.5), r);
        assert_eq!(grad(&stops, 1.5), b);
        assert_eq!(grad(&[r, b], 0.5), Color::Rgb(128, 0, 128));
        assert_eq!(grad(&[g], 0.3), g);
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
