//! Cards of albums and artists in rows as wide as the pane, such as Home's shelves, and the keys
//! that move a selection through them.

use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{Ink, filled_card};
use anti_webui_md3e::typography::TextStyle;

use crate::keymap::Keystroke;
use crate::pane::INSET;
use crate::row_text::faded_end;
use crate::selection::selection_fill;

pub const CARD_MIN_WIDTH: f32 = 160.0;
/// Between cards, across and down.
pub const CARD_GAP: f32 = 16.0;
/// From a card's edge to its words, and to a round picture.
const CARD_PADDING: f32 = 12.0;
/// Rows of songs and of cards differ too much in height for a page to be a number of them. This
/// many stays in view.
const PAGE_ROWS: usize = 4;
/// More than half of any picture, which corners are cut down to: a circle.
const ROUND: f32 = 10_000.0;
/// A card's corners. A cover that fills the top of one is rounded as much: a picture cuts what is
/// in it square, as if its card did not round it.
const CORNER: f32 = 12.0;
/// How many cards fit across a pane `width` wide.
pub fn columns_for(width: f32) -> usize {
    let room = width - INSET * 2.0 + CARD_GAP;
    ((room / (CARD_MIN_WIDTH + CARD_GAP)) as usize).max(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridMotion {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    First,
    Last,
}

impl GridMotion {
    /// What asks for each motion: an arrow, Page Up, Page Down, Home or End, without modifiers.
    pub fn strokes() -> [(Keystroke, Self); 8] {
        [
            (NamedKey::ArrowUp, Self::Up),
            (NamedKey::ArrowDown, Self::Down),
            (NamedKey::ArrowLeft, Self::Left),
            (NamedKey::ArrowRight, Self::Right),
            (NamedKey::PageUp, Self::PageUp),
            (NamedKey::PageDown, Self::PageDown),
            (NamedKey::Home, Self::First),
            (NamedKey::End, Self::Last),
        ]
        .map(|(key, motion)| (Keystroke::named(key), motion))
    }
}

/// The item to select after `motion` from `current`, in `rows` as they show from top to bottom.
/// Without a current item that is on the screen, a move starts at the first item, or the last for
/// [`GridMotion::Last`].
pub fn step<T: Clone + PartialEq>(
    rows: &[Vec<T>],
    current: Option<&T>,
    motion: GridMotion,
) -> Option<T> {
    let first = || rows.iter().flatten().next().cloned();
    let last = || rows.iter().flatten().next_back().cloned();
    let position = current.and_then(|current| {
        rows.iter().enumerate().find_map(|(row_ix, row)| {
            let column = row.iter().position(|item| item == current)?;
            Some((row_ix, column))
        })
    });
    let Some((row_ix, column)) = position else {
        return if motion == GridMotion::Last {
            last()
        } else {
            first()
        };
    };
    let in_row = |row_ix: usize, column: usize| {
        let row = rows.get(row_ix)?;
        row.get(column.min(row.len().checked_sub(1)?)).cloned()
    };
    match motion {
        GridMotion::Up => in_row(row_ix.saturating_sub(1), column),
        GridMotion::Down => in_row(row_ix + 1, column).or_else(|| in_row(row_ix, column)),
        GridMotion::Left => in_row(row_ix, column.saturating_sub(1)),
        GridMotion::Right => in_row(row_ix, column + 1),
        GridMotion::PageUp => in_row(row_ix.saturating_sub(PAGE_ROWS), column),
        GridMotion::PageDown => in_row((row_ix + PAGE_ROWS).min(rows.len() - 1), column),
        GridMotion::First => first(),
        GridMotion::Last => last(),
    }
}

/// `cards` side by side as wide as `columns` of them share the pane: a row of fewer leaves the rest
/// empty.
pub fn card_row(columns: usize, cards: Vec<Div>) -> Div {
    let empty = columns.saturating_sub(cards.len());
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_start()
        .gap(CARD_GAP)
        .px(INSET)
        .pb(CARD_GAP)
        .children(
            cards
                .into_iter()
                .chain((0..empty).map(|_| div()))
                .map(|card| card.flex_1().min_w(0))
                .collect::<Vec<_>>(),
        )
}

type When = Rc<dyn Fn() -> bool>;

/// What [`card`] shows.
pub struct Card {
    cover: Div,
    is_round: bool,
    title: String,
    details: Option<String>,
    line: Option<Div>,
    selected: When,
    focused: When,
}

impl Card {
    /// `cover` fills the top of the card: [`crate::cover::cover`] of what the card is.
    pub fn new(cover: Div, title: impl Into<String>) -> Self {
        Self {
            cover,
            is_round: false,
            title: title.into(),
            details: None,
            line: None,
            selected: Rc::new(|| false),
            focused: Rc::new(|| false),
        }
    }

    /// A line below the title, such as the artist or the year.
    pub fn details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// A line below the title that is more than words, such as an artist that is a link
    /// ([`crate::link::artist_line`]). It keeps its own colors.
    pub fn line(mut self, line: Div) -> Self {
        self.line = Some(line);
        self
    }

    /// The cover is a picture of someone, which is round.
    pub fn artist_picture(mut self) -> Self {
        self.is_round = true;
        self
    }

    pub fn selected_when(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.selected = Rc::new(f);
        self
    }

    /// Whether what the cards are in has the focus, which makes the selection stronger.
    pub fn focused_when(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.focused = Rc::new(f);
        self
    }
}

/// A filled card that reacts to the pointer as Material's do, but takes no focus of its own: what
/// the cards are in has it and moves the selection with the keys, as a row list does. Selected, it
/// is filled in the selection's color, less strongly without the focus. Its clicks are the
/// caller's.
pub fn card(card: Card) -> Div {
    let Card {
        cover,
        is_round,
        title,
        details,
        line,
        selected,
        focused,
    } = card;
    let selected = memo(move || selected());
    let focused = memo(move || focused());
    let cover = if is_round {
        div()
            .flex()
            .flex_none()
            .p(CARD_PADDING)
            .child(cover.w_full().aspect_ratio(1.0_f32).rounded(ROUND))
    } else {
        cover
            .flex_none()
            .w_full()
            .aspect_ratio(1.0_f32)
            .rounded_t(CORNER)
    };
    let words = div()
        .flex()
        .flex_col()
        .min_w(0)
        .px(CARD_PADDING)
        .pt(if is_round { 0.0_f32 } else { 8.0 })
        .pb(CARD_PADDING)
        .child(faded_end(
            TextStyle::TITLE_SMALL
                .apply(div())
                .child(text(title.clone())),
        ))
        .children(details.map(|details| {
            faded_end(
                TextStyle::BODY_MEDIUM
                    .apply(div())
                    .dyn_style(|s| s.text_color(color_scheme().on_surface_variant))
                    .child(text(details)),
            )
        }))
        .children(line);
    let ink = Ink::new(|c| c.on_surface, || false);
    let card = filled_card()
        .min_w(0)
        .cursor(CursorIcon::Pointer)
        .select_none()
        .access(move |a| {
            a.role(Role::ListOption)
                .label(title.clone())
                .selected(selected.get())
        })
        .dyn_style(move |s| {
            let c = color_scheme();
            match selection_fill(&c, selected.get(), focused.get()) {
                Some(fill) => s.bg(fill),
                None => s,
            }
        })
        .child(cover)
        .child(words);
    ink.paint(ink.listen(card))
}

#[cfg(test)]
mod tests;
