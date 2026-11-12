//! Names that go to what they name in the app: an album, an artist.

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::Ink;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::navigation::Location;
use subsonic::{Album, AlbumId, ArtistId, Song};

use crate::row_list::{is_context_click, row_control};
use crate::row_text::faded_end;
use crate::{Command, Commands};

/// How far the hover of a name reaches past it, which its margins take back so that the name
/// stays in line with the text around it.
pub const REACH: f32 = 4.0;

/// `name`, which shows `to` when it is clicked, in the type and color of the text around it. A
/// name with nowhere to go is plain text.
///
/// It is quiet until the pointer is on it, as a row's details are read far more often than they
/// are clicked, and it is no stop of Tab: from the keyboard, Go to Album and Go to Artist of the
/// menus go there. Clicking it does nothing to the row or card it is in.
pub fn link(name: impl Into<String>, to: Option<Location>, commands: &Commands) -> Div {
    let name = name.into();
    let Some(to) = to else {
        return div().flex_none().child(text(name));
    };
    let commands = commands.clone();
    let ink = Ink::new(|c| c.on_surface, || false);
    let hovered = ink.hovered;
    let label = name.clone();
    let named = div()
        .flex_none()
        .px(REACH)
        .mx(-REACH)
        .py(REACH / 2.0)
        .my(-REACH / 2.0)
        .rounded(4.0_f32)
        .cursor(CursorIcon::Pointer)
        .access(move |a| a.role(Role::Link).label(label.clone()))
        .dyn_style(move |s| {
            if hovered.get() {
                s.text_color(color_scheme().on_surface)
            } else {
                s
            }
        })
        .on_click(move |e| {
            if !is_context_click(e) {
                commands.send(Command::Show(to.clone()));
            }
        })
        .child(text(name));
    ink.paint(ink.listen(row_control(named)))
}

/// A link to `album`, named `name`.
pub fn album_link(name: impl Into<String>, album: Option<AlbumId>, commands: &Commands) -> Div {
    link(
        name,
        album.map(|album| Location::Albums(Some(album))),
        commands,
    )
}

/// A link to `artist`, named `name`.
pub fn artist_link(name: impl Into<String>, artist: Option<ArtistId>, commands: &Commands) -> Div {
    link(
        name,
        artist.map(|artist| Location::Artists(Some(artist))),
        commands,
    )
}

/// The artist of `song`, as a link.
pub fn song_artist_link(song: &Song, commands: &Commands) -> Option<Div> {
    let name = song.artist()?;
    Some(artist_link(name, song.artist_id().cloned(), commands))
}

/// The artist of `album`, as a link.
pub fn album_artist_link(album: &Album, commands: &Commands) -> Option<Div> {
    let name = album.artist()?;
    Some(artist_link(name, album.artist_id().cloned(), commands))
}

/// The line under a row's title that says who it is by: `artist`, then `rest` after a dot, in the
/// details' type and color. What does not fit is cut and fades at its end.
pub fn artist_line(artist: Option<Div>, rest: Option<Div>) -> Div {
    let dot = (artist.is_some() && rest.is_some()).then(|| div().flex_none().child(text("·")));
    let line = TextStyle::BODY_MEDIUM
        .apply(div())
        .flex()
        .flex_row()
        .items_center()
        .gap(4.0_f32)
        // Room for the hover of a name at either end, which the margins of the line take back
        .px(REACH)
        .dyn_style(|s| s.text_color(color_scheme().on_surface_variant))
        .children(artist)
        .children(dot)
        .children(rest);
    faded_end(line).mx(-REACH)
}

/// The artist of `song` and its album, each a link.
pub fn song_line(song: &Song, commands: &Commands) -> Div {
    let album = song
        .album()
        .map(|name| album_link(name, song.album_id().cloned(), commands));
    artist_line(song_artist_link(song, commands), album)
}

/// The artist of `album`, as a link, and its year.
pub fn album_line(album: &Album, commands: &Commands) -> Div {
    let year = album
        .year()
        .map(|year| div().flex_none().child(text(year.to_string())));
    artist_line(album_artist_link(album, commands), year)
}

#[cfg(test)]
mod tests;
