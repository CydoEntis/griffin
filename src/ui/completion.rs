//! The completion popup: what the language server offered for the word at the
//! cursor, filtered by what has been typed of it since, in a small card anchored
//! at the cursor like the hover's.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;
use ropey::Rope;
use unicode_width::UnicodeWidthStr;

use super::hover::anchor;
use crate::lsp::CompletionItem;
use crate::theme::Theme;

/// Most items the popup shows at once.
pub const MAX_ITEMS: usize = 10;
/// Widest a row's text gets inside the card.
const MAX_TEXT_WIDTH: usize = 60;
/// Border plus one cell of padding on each side.
const SIDE: u16 = 2;

/// Whether `c` belongs to the word being completed.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Where the word ending at char index `cursor` starts: back over word chars.
pub fn word_start(rope: &Rope, cursor: usize) -> usize {
    let mut start = cursor.min(rope.len_chars());
    while start > 0 && is_word_char(rope.char(start - 1)) {
        start -= 1;
    }
    start
}

/// The text typed of the word since `start`, when the cursor at `cursor` is
/// still on it: at or after `start` with only word chars between. `None` once it
/// has moved off.
pub fn typed_word(rope: &Rope, start: usize, cursor: usize) -> Option<String> {
    if cursor < start || cursor > rope.len_chars() {
        return None;
    }
    let typed = rope.slice(start..cursor).to_string();
    typed.chars().all(is_word_char).then_some(typed)
}

#[derive(Debug, Clone)]
pub struct Completion {
    /// The buffer it completes in.
    pub doc: u64,
    /// Where the word being completed starts, as a char index.
    pub start: usize,
    /// The cursor when the request went out, which the items' edit ranges are
    /// relative to.
    asked_at: usize,
    items: Vec<CompletionItem>,
    /// Index into the shown items.
    selected: usize,
}

impl Completion {
    pub fn new(doc: u64, start: usize, asked_at: usize, items: Vec<CompletionItem>) -> Self {
        Self {
            doc,
            start,
            asked_at,
            items,
            selected: 0,
        }
    }

    /// The items whose filter text starts with `typed`, ignoring case, at most
    /// `MAX_ITEMS`.
    pub fn shown(&self, typed: &str) -> Vec<&CompletionItem> {
        let typed = typed.to_lowercase();
        self.items
            .iter()
            .filter(|item| item.filter.to_lowercase().starts_with(&typed))
            .take(MAX_ITEMS)
            .collect()
    }

    pub fn selected(&self, typed: &str) -> Option<&CompletionItem> {
        let shown = self.shown(typed);
        shown
            .get(self.selected.min(shown.len().saturating_sub(1)))
            .copied()
    }

    /// Moves the selection one row, stopping at either end.
    pub fn step(&mut self, typed: &str, down: bool) {
        let last = self.shown(typed).len().saturating_sub(1);
        self.selected = if down {
            (self.selected + 1).min(last)
        } else {
            self.selected.min(last).saturating_sub(1)
        };
    }

    /// Typing changed what's shown, so the best match is selected again.
    pub fn reset(&mut self) {
        self.selected = 0;
    }

    /// The range accepting `item` replaces with the cursor at `cursor`: its
    /// `textEdit` range stretched over whatever was typed since the request, or
    /// else the word typed so far.
    pub fn replaced(&self, item: &CompletionItem, cursor: usize) -> Range<usize> {
        match &item.edit {
            Some(edit) => {
                // Text after the request's cursor that the edit covers is still
                // just after the cursor; typing only ever added before it.
                let after = edit.end.saturating_sub(self.asked_at);
                edit.start.min(cursor)..cursor + after
            }
            None => self.start.min(cursor)..cursor,
        }
    }

    /// Draws the items matching `typed` in a card anchored at the cursor cell
    /// `cursor`, inside `bounds`: each row its kind in `muted`, its label in `fg`
    /// with the typed prefix in `accent` bold, and its detail right-aligned in
    /// `muted`. The selected row is the glow row with its label in `strong`.
    /// Nothing when nothing matches.
    pub fn render(
        &self,
        theme: &Theme,
        typed: &str,
        cursor: (u16, u16),
        bounds: Rect,
        frame: &mut Frame,
    ) {
        let shown = self.shown(typed);
        if shown.is_empty() {
            return;
        }
        let selected = self.selected.min(shown.len() - 1);
        let kind_width = shown.iter().map(|i| i.kind.width()).max().unwrap_or(0);
        let gap = usize::from(kind_width > 0);
        let label_width = shown.iter().map(|i| i.label.width()).max().unwrap_or(0);
        let detail_width = shown.iter().map(|i| i.detail.width()).max().unwrap_or(0);
        // Two blanks keep the detail apart from the longest label.
        let detail_room = if detail_width > 0 {
            detail_width + 2
        } else {
            0
        };
        let text_width = (kind_width + gap + label_width + detail_room).clamp(1, MAX_TEXT_WIDTH);
        let rows = u16::try_from(shown.len()).unwrap_or(u16::MAX);
        let width = u16::try_from(text_width).unwrap_or(u16::MAX);
        let card = anchor(cursor, width + SIDE * 2, rows + 2, bounds);
        if card.width <= SIDE * 2 || card.height <= 2 {
            return;
        }
        frame.render_widget(Clear, card);
        let block = super::card_block(theme);
        let inner = block.inner(card);
        frame.render_widget(block, card);
        let room = usize::from(inner.width.saturating_sub(2));
        let typed_chars = typed.chars().count();
        // `mono` draws the glow as reverse video, which only reads when the
        // row's text brings no colours of its own.
        let plain = !theme.ramps();
        let out = frame.buffer_mut();
        for (index, (y, item)) in (inner.y..inner.bottom()).zip(&shown).enumerate() {
            let chosen = index == selected;
            if chosen {
                super::glow_row(theme, out, inner.x, inner.right(), y);
            }
            let fg = |color| {
                if chosen && plain {
                    Style::new()
                } else {
                    Style::new().fg(color)
                }
            };
            let x = inner.x + 1;
            out.set_stringn(x, y, item.kind, room, fg(theme.muted));
            let offset = kind_width + gap;
            if offset >= room {
                continue;
            }
            let label_x = x + u16::try_from(offset).unwrap_or(u16::MAX);
            let label_room = room - offset;
            let label = fg(if chosen { theme.strong } else { theme.fg });
            let (end, _) = out.set_stringn(label_x, y, &item.label, label_room, label);
            // The typed prefix: the label's first chars, when they are what was
            // typed (the match is on the filter text, which can differ).
            let prefix_len = item
                .label
                .char_indices()
                .nth(typed_chars)
                .map_or(item.label.len(), |(at, _)| at);
            let prefix = &item.label[..prefix_len];
            if typed_chars > 0
                && prefix.chars().count() == typed_chars
                && prefix.to_lowercase() == typed.to_lowercase()
            {
                let style = fg(theme.accent).add_modifier(Modifier::BOLD);
                out.set_stringn(label_x, y, prefix, label_room, style);
            }
            let detail = item.detail.width();
            let right = usize::from(inner.right().saturating_sub(1));
            let free = right.saturating_sub(usize::from(end) + 2);
            if detail > 0 && free > 0 {
                let shown_width = detail.min(free);
                let detail_x = u16::try_from(right - shown_width).unwrap_or(u16::MAX);
                out.set_stringn(detail_x, y, &item.detail, shown_width, fg(theme.muted));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, edit: Option<Range<usize>>) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            kind: "fn",
            detail: String::new(),
            filter: label.into(),
            text: label.into(),
            edit,
        }
    }

    #[test]
    fn typing_filters_by_prefix_ignoring_case_and_at_most_ten_show() {
        let names = [
            "Push", "push_str", "pop", "len", "a", "b", "c", "d", "e", "f", "g", "h",
        ];
        let completion = Completion::new(1, 0, 0, names.iter().map(|n| item(n, None)).collect());
        assert_eq!(completion.shown("").len(), MAX_ITEMS);
        let labels = |typed| -> Vec<String> {
            completion
                .shown(typed)
                .iter()
                .map(|i| i.label.clone())
                .collect()
        };
        assert_eq!(labels("pu"), ["Push", "push_str"]);
        assert_eq!(labels("PUSH_"), ["push_str"]);
        assert!(labels("x").is_empty());
    }

    #[test]
    fn the_selection_stops_at_either_end() {
        let items = ["a1", "a2", "a3"].iter().map(|n| item(n, None)).collect();
        let mut completion = Completion::new(1, 0, 0, items);
        completion.step("", false);
        assert_eq!(
            completion.selected("").map(|i| i.label.as_str()),
            Some("a1")
        );
        for _ in 0..5 {
            completion.step("", true);
        }
        assert_eq!(
            completion.selected("").map(|i| i.label.as_str()),
            Some("a3")
        );
        completion.step("", false);
        assert_eq!(
            completion.selected("").map(|i| i.label.as_str()),
            Some("a2")
        );
        completion.reset();
        assert_eq!(
            completion.selected("").map(|i| i.label.as_str()),
            Some("a1")
        );
    }

    #[test]
    fn accepting_replaces_the_edit_range_or_the_typed_word() {
        // Asked at 6 with the word starting at 6; "le" typed since.
        let completion = Completion::new(1, 6, 6, Vec::new());
        assert_eq!(completion.replaced(&item("len", Some(6..6)), 8), 6..8);
        // An edit reaching two chars past the request's cursor still covers them.
        assert_eq!(completion.replaced(&item("len", Some(4..8)), 8), 4..10);
        assert_eq!(completion.replaced(&item("len", None), 8), 6..8);
    }

    #[test]
    fn the_word_is_found_and_left() {
        let rope = Rope::from_str("    s.le x\n");
        assert_eq!(word_start(&rope, 8), 6);
        assert_eq!(word_start(&rope, 6), 6);
        assert_eq!(word_start(&rope, 5), 4);
        assert_eq!(typed_word(&rope, 6, 8).as_deref(), Some("le"));
        assert_eq!(typed_word(&rope, 6, 6).as_deref(), Some(""));
        assert_eq!(typed_word(&rope, 6, 5), None);
        assert_eq!(typed_word(&rope, 6, 10), None);
    }
}
