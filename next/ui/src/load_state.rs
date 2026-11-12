//! What a pane shows in place of what it is for: while that loads, when there is none of it, and
//! when it could not be had, in the words every screen says these with.

use anti_webui::prelude::*;
use anti_webui_md3e::components::{ButtonStyle, loading_indicator, styled_button};
use anti_webui_md3e::typography::TextStyle;

use crate::type_scale::{error_text, muted_text, styled_text};

/// Around what stands in the middle of a pane.
const MARGIN: f32 = 32.0;
/// Text in the middle of a pane wraps at this width.
const TEXT_WIDTH: f32 = 360.0;

/// What a pane or a section is for, as the words for its states name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subject {
    title: &'static str,
    plain: &'static str,
}

impl Subject {
    /// `title` is the name in title case, as in `Loading Songs`, and `plain` the name in a
    /// sentence, as in `Couldn't load songs` or `Couldn't load this album`.
    pub const fn new(title: &'static str, plain: &'static str) -> Self {
        Self { title, plain }
    }

    /// The name of the wait while it loads.
    pub fn loading(self) -> String {
        format!("Loading {}", self.title)
    }

    /// The name of the wait while what is on show of it is brought up to date.
    pub fn updating(self) -> String {
        format!("Updating {}", self.title)
    }

    /// The heading of a failure to load it.
    pub fn load_failed(self) -> String {
        format!("Couldn't load {}", self.plain)
    }

    /// The heading of a failure to bring what is on show of it up to date.
    pub fn refresh_failed(self) -> String {
        format!("Couldn't refresh {}", self.plain)
    }

    /// What stands in for none of it, of a subject in the plural.
    pub fn none(self) -> String {
        format!("No {}", self.plain)
    }

    /// What stands in for none of it in the server's library, of a subject in the plural.
    pub fn none_on_server(self) -> String {
        format!("No {} on this server", self.plain)
    }
}

pub const SONGS: Subject = Subject::new("Songs", "songs");
pub const ALBUMS: Subject = Subject::new("Albums", "albums");
pub const ARTISTS: Subject = Subject::new("Artists", "artists");
pub const PLAYLISTS: Subject = Subject::new("Playlists", "playlists");
pub const FAVORITES: Subject = Subject::new("Favorites", "favorites");
pub const LIBRARY: Subject = Subject::new("Library", "the library");
pub const HOME: Subject = Subject::new("Home", "Home");
pub const ALBUM: Subject = Subject::new("Album", "this album");
pub const ARTIST: Subject = Subject::new("Artist", "this artist");
pub const PLAYLIST: Subject = Subject::new("Playlist", "this playlist");
pub const TOP_SONGS: Subject = Subject::new("Top Songs", "top songs");
pub const SIMILAR_ARTISTS: Subject = Subject::new("Similar Artists", "similar artists");
pub const RECOMMENDATIONS: Subject = Subject::new("Recommendations", "recommendations");
pub const RECENTLY_PLAYED: Subject = Subject::new("Recently Played Songs", "recently played songs");
pub const MOST_PLAYED: Subject = Subject::new("Most Played Albums", "most played albums");
pub const NEW_ALBUMS: Subject = Subject::new("New Albums", "new albums");
pub const FAVORITE_SONGS: Subject = Subject::new("Favorite Songs", "favorite songs");
pub const FAVORITE_ALBUMS: Subject = Subject::new("Favorite Albums", "favorite albums");
pub const FAVORITE_ARTISTS: Subject = Subject::new("Favorite Artists", "favorite artists");

/// What a section has in place of what its heading names when the server has none of it.
pub const NONE_ON_SERVER: &str = "None on this server";

/// What a list says when the filter `query` leaves nothing of it.
pub fn no_matches_for(query: &str) -> String {
    format!("No matches for “{}”", query.trim())
}

/// `child` in the middle of the room it is given.
pub fn centered(child: impl IntoView) -> Div {
    div()
        .flex_1()
        .size_full()
        .min_h(0)
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .p(MARGIN)
        .child(child)
}

/// What a pane shows while what it is for loads. `label` names the wait for assistive technology.
pub fn pane_loading(label: impl Into<String>) -> Div {
    let label = label.into();
    centered(loading_indicator().access(move |a| a.role(Role::ProgressBar).label(label.clone())))
}

/// What a pane shows when there is nothing in it: what is missing, why, if that helps, and what to
/// do about it, if there is something.
pub fn empty_state(message: &str, note: Option<&str>, action: Option<Div>) -> Div {
    standing(message.to_owned(), note, action)
}

/// What a list shows when its filter, which `query` reads, leaves nothing. `note` is what
/// `Session::missing_readings_note` gives.
pub fn no_matches(query: impl Fn() -> String + 'static, note: Option<&str>) -> Div {
    standing(move || no_matches_for(&query()), note, None)
}

fn standing(message: impl IntoTextSource, note: Option<&str>, action: Option<Div>) -> Div {
    centered(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(4.0_f32)
            .max_w(TEXT_WIDTH)
            .text_align(TextAlign::Center)
            .child(muted_text(TextStyle::BODY_LARGE, message))
            .children(note.map(|note| muted_text(TextStyle::BODY_MEDIUM, note.to_owned())))
            .children(action.map(|action| action.mt(12))),
    )
}

/// What a pane shows when what it is for could not be loaded: `title` says what, `reason` why,
/// and Retry runs `retry`.
pub fn load_failure(
    title: impl Into<String>,
    reason: impl Into<String>,
    retry: impl Fn() + 'static,
) -> Div {
    let title = title.into();
    centered(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(4.0_f32)
            .max_w(TEXT_WIDTH)
            .text_align(TextAlign::Center)
            .child(
                styled_text(TextStyle::TITLE_MEDIUM, title.clone())
                    .access(move |a| a.role(Role::Heading).level(2).label(title.clone())),
            )
            .child(error_text(reason.into()))
            .child(retry_button(retry).mt(12)),
    )
}

/// The button that tries again what failed.
pub fn retry_button(retry: impl Fn() + 'static) -> Div {
    styled_button("Retry", ButtonStyle::tonal()).on_click(move |_| retry())
}

#[cfg(test)]
mod tests;
