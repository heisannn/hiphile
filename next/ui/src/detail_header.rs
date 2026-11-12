//! The head of a detail pane: the cover large, what it is called, by whom, and the buttons that
//! play it.

use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    ButtonSize, FabColor, FabStyle, IconButtonStyle, button_group, styled_extended_fab,
    styled_icon_button,
};
use anti_webui_md3e::icons;
use anti_webui_md3e::typography::TextStyle;

use crate::pane::INSET;
use crate::row_text::{faded_after, faded_end};
use crate::type_scale::{muted_text, styled_text};

const COVER: f32 = 192.0;
/// A square cover's corners, which follow the pane's around it.
const COVER_CORNER: f32 = 12.0;
/// The cover in a header narrower than [`WIDE`], which leaves the words and buttons their room.
const NARROW_COVER: f32 = 128.0;
const WIDE: f32 = 560.0;
/// Between the cover and the words.
const COVER_GAP: f32 = 24.0;
/// A comment is cut after this many lines: the cover sets the header's height, which a long one
/// would take from the songs.
const COMMENT_LINES: usize = 2;
/// What a disabled button's content is drawn at in Material.
const DISABLED_OPACITY: f32 = 0.38;

/// What [`detail_header`] shows.
pub struct DetailHeader {
    cover: Div,
    is_round: bool,
    title: String,
    subtitle: Option<Div>,
    comment: Option<String>,
    summary: String,
    actions: Vec<Div>,
}

impl DetailHeader {
    /// `cover` is sized by the header: [`crate::cover::cover`] of what is shown. `summary` is the
    /// line of details, such as the year, the number of songs and how long they last.
    pub fn new(cover: Div, title: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            cover,
            is_round: false,
            title: title.into(),
            subtitle: None,
            comment: None,
            summary: summary.into(),
            actions: Vec::new(),
        }
    }

    /// Below the title: whose it is, such as the links to the artists of an album, or a playlist's
    /// owner.
    pub fn subtitle(mut self, subtitle: Div) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    pub fn comment(mut self, comment: Option<String>) -> Self {
        self.comment = comment.filter(|comment| !comment.trim().is_empty());
        self
    }

    /// The cover is a picture of someone, which is round.
    pub fn artist_picture(mut self) -> Self {
        self.is_round = true;
        self
    }

    /// A button after the others below the summary, such as [`play_and_shuffle`] first.
    pub fn action(mut self, action: Div) -> Self {
        self.actions.push(action);
        self
    }
}

/// The cover and, beside it at its foot, the title, the subtitle, the comment, the summary and
/// the buttons. In a narrow pane the cover is smaller.
pub fn detail_header(header: DetailHeader) -> Div {
    let DetailHeader {
        cover,
        is_round,
        title,
        subtitle,
        comment,
        summary,
        actions,
    } = header;
    let width = signal(0.0f32);
    let side = memo(move || {
        let width = width.get();
        if width == 0.0 || width >= WIDE {
            COVER
        } else {
            NARROW_COVER
        }
    });
    let comment = comment.map(|comment| {
        let style = TextStyle::BODY_MEDIUM;
        faded_after(muted_text(style, comment), COMMENT_LINES, style.line_height)
    });
    let words = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(0)
        .gap(4.0_f32)
        .child(faded_end(
            styled_text(TextStyle::HEADLINE_SMALL, title.clone())
                .access(move |a| a.role(Role::Heading).level(2).label(title.clone())),
        ))
        .children(subtitle.map(|subtitle| {
            TextStyle::TITLE_MEDIUM
                .apply(div())
                .flex()
                .flex_row()
                .min_w(0)
                .child(subtitle)
        }))
        .children(comment)
        .child(muted_text(TextStyle::BODY_MEDIUM, summary))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(8.0_f32)
                .mt(12)
                .children(actions),
        );
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_end()
        .gap(COVER_GAP)
        .p(INSET)
        .on_resized(move |w, _| width.set_if_changed(w))
        .child(cover.flex_none().dyn_style(move |s| {
            let side = side.get();
            s.size(side)
                .rounded(if is_round { side / 2.0 } else { COVER_CORNER })
        }))
        .child(words)
}

/// Play and Shuffle, side by side, which run `play` and `shuffle`. Play is the screen's one
/// floating action. Both are disabled while `disabled` says so, as when there is nothing to play.
pub fn play_and_shuffle(
    play: impl Fn() + 'static,
    shuffle: impl Fn() + 'static,
    disabled: impl Fn() -> bool + 'static,
) -> Div {
    let disabled = Rc::new(disabled);
    let (play_off, look_off) = (disabled.clone(), disabled.clone());
    // A floating action button has no disabled look of its own: it fades as Material's buttons do
    let play = styled_extended_fab(
        icons::PLAY_ARROW,
        "Play",
        FabStyle::new().color(FabColor::Primary),
    )
    .access(|a| a.label("Play"))
    .disabled(move || play_off())
    .dyn_style(move |s| {
        if look_off() {
            s.opacity(DISABLED_OPACITY)
        } else {
            s
        }
    })
    .on_click(move |_| play());
    let shuffle = styled_icon_button(
        icons::SHUFFLE,
        IconButtonStyle::tonal()
            .size(ButtonSize::Medium)
            .square()
            .disabled_when(move || disabled()),
    )
    .access(|a| a.label("Shuffle"))
    .on_click(move |_| shuffle());
    button_group([play, shuffle])
}

#[cfg(test)]
mod tests;
