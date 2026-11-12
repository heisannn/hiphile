//! The playlists, on the list's pane.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    ButtonStyle, IconButtonStyle, MenuItem, menu_item, styled_button, styled_icon_button,
};
use anti_webui_md3e::icons;
use hiphile_core::playlists::PlaylistList;
use next_ui::details::{format_duration, format_playlist_count, format_song_count, join_details};
use next_ui::keymap::{KeyContext, key_context};
use next_ui::list_pane::{ListHeader, list_filter, list_header, refresh_failure};
use next_ui::load_state::{empty_state, load_failure, no_matches, pane_loading};
use next_ui::row_list::{ROW_PADDING, RowList, row_list};
use next_ui::row_text::row_text;
use next_ui::{AlbumAction, Command, Commands};
use playback::Placement;
use subsonic::{Playlist, PlaylistId};

use super::Playlists;
use super::keys::{self, KeyCommand};

/// The filter, how many playlists there are with the button that makes one, a refresh that
/// failed, and the list.
pub(super) fn pane(screen: &Playlists) -> Div {
    let shown = screen.shown;
    let count = memo(move || shown.with(|shown| shown.loaded_list().map(|list| list.len())));
    let refresh_error = memo(move || shown.with(|shown| shown.refresh_error().map(str::to_owned)));
    let commands = screen.cx.commands.clone();
    let new = styled_icon_button(icons::ADD, IconButtonStyle::standard())
        .access(|a| a.label("New Playlist…"))
        .on_click(move |_| commands.send(Command::NewPlaylist(Arc::from([]))));
    let filter = screen.filter;
    let state = screen.state.clone();
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(show(
            move || count.get().is_some_and(|count| count > 0),
            move || list_filter(filter, "Filter Playlists"),
        ))
        .child(list_header(
            ListHeader::new(move || {
                // The note in the empty list says there are none
                count
                    .get()
                    .filter(|count| *count > 0)
                    .map(format_playlist_count)
            })
            .action(new),
        ))
        .child(dyn_view(move || {
            let error = refresh_error.get();
            let state = state.clone();
            untrack(move || {
                error.map(|error| {
                    refresh_failure("Couldn't refresh playlists", error, move || {
                        state.refresh();
                    })
                })
            })
        }))
        .child(content(screen))
}

/// What the list shows in place of rows, or that it shows them.
#[derive(Clone, Debug, PartialEq)]
enum Content {
    Loading,
    Failed(String),
    Empty,
    NoMatches(String),
    Rows,
}

fn content(screen: &Playlists) -> Div {
    let (shown, filter) = (screen.shown, screen.filter);
    let content = memo(move || {
        shown.with(|shown| match shown.list() {
            PlaylistList::Loading => Content::Loading,
            PlaylistList::Failed(error) => Content::Failed(error.clone()),
            PlaylistList::Loaded(list) if list.is_empty() => Content::Empty,
            PlaylistList::Loaded(_) if shown.visible().is_empty() => {
                Content::NoMatches(filter.with(|filter| filter.trim().to_owned()))
            }
            PlaylistList::Loaded(_) => Content::Rows,
        })
    });
    let context = list_context(screen);
    let screen = screen.clone();
    let shown = div()
        .flex_1()
        .min_h(0)
        .flex()
        .flex_col()
        .child(dyn_view(move || {
            let content = content.get();
            let screen = screen.clone();
            untrack(move || match content {
                Content::Loading => pane_loading("Loading Playlists"),
                Content::Failed(error) => {
                    let state = screen.state.clone();
                    load_failure("Couldn't load playlists", error, move || state.retry_list())
                }
                Content::Empty => empty_state(
                    "No playlists yet",
                    None,
                    Some(new_playlist_button(&screen.cx.commands)),
                ),
                Content::NoMatches(query) => no_matches(
                    &format!("No playlists match “{query}”"),
                    screen.cx.workspace.session().missing_readings_note(),
                ),
                Content::Rows => rows(&screen),
            })
        }));
    key_context(shown, &context)
}

fn new_playlist_button(commands: &Commands) -> Div {
    let commands = commands.clone();
    styled_button("New Playlist…", ButtonStyle::tonal())
        .on_click(move |_| commands.send(Command::NewPlaylist(Arc::from([]))))
}

fn rows(screen: &Playlists) -> Div {
    let shown = screen.shown;
    let visible = memo(move || {
        shown.with(|shown| {
            (0..shown.visible().len())
                .filter_map(|at| shown.visible_playlist(at).cloned())
                .collect::<Rc<[Playlist]>>()
        })
    });
    let username = screen.username.clone();
    let (opening, menu_of) = (screen.clone(), screen.clone());
    let list = row_list(
        RowList::new(
            "Playlists",
            move || visible.get(),
            |playlist: &Playlist| playlist.id().clone(),
            move |_, row| playlist_row(visible, row.position(), row.id().clone(), username.clone()),
        )
        .row_label(|playlist| playlist.name().to_owned())
        .selection(screen.list_selection)
        .node_ref(screen.list_node)
        .on_open(move |_| opening.focus_songs())
        .on_context_menu(move |playlist, at| {
            let has_songs = menu_of
                .playlist(&playlist)
                .is_none_or(|listed| listed.song_count() != Some(0));
            let groups = playlist_menu(&menu_of, &playlist, has_songs, true);
            menu_of.menu.open_at(at, groups);
        }),
    );
    // The list is laid out only now, so it could not be scrolled to the playlist chosen before
    let first = Cell::new(true);
    let screen = screen.clone();
    list.flex_1().min_h(0).on_resized(move |_, _| {
        if !first.replace(false) {
            return;
        }
        screen.reveal_selected();
        if !screen.list_focused.replace(true) {
            screen.list_node.focus();
        }
    })
}

/// The name of the playlist at `position` and, below, whose it is when it is not the user's,
/// how many songs it has and how long they last. It follows the playlist while it stays there.
fn playlist_row(
    visible: Memo<Rc<[Playlist]>>,
    position: usize,
    id: PlaylistId,
    username: Arc<str>,
) -> Div {
    let playlist = memo(move || {
        visible.with(|visible| {
            visible
                .get(position)
                .filter(|playlist| playlist.id() == &id)
                .cloned()
        })
    });
    div()
        .flex()
        .flex_row()
        .items_center()
        .px(ROW_PADDING)
        .child(dyn_view(move || {
            let playlist = playlist.get();
            let username = username.clone();
            untrack(move || {
                playlist.map(|playlist| {
                    let details = details(&playlist, &username);
                    row_text(
                        playlist.name().to_owned(),
                        Some(details).filter(|details| !details.is_empty()),
                    )
                    .flex_1()
                })
            })
        }))
}

fn details(playlist: &Playlist, username: &str) -> String {
    let owner = playlist
        .owner()
        .filter(|_| !playlist.is_owned_by(username))
        .map(str::to_owned);
    let count = playlist
        .song_count()
        .map(|count| format_song_count(count as usize));
    let duration = playlist
        .duration()
        .filter(|_| playlist.song_count() != Some(0))
        .map(format_duration);
    join_details([owner, count, duration])
}

/// What can be done to the whole of `playlist`: queue it, and rename and delete it when the user
/// may. Play is for where nothing else plays it.
pub(super) fn playlist_menu(
    screen: &Playlists,
    playlist: &PlaylistId,
    has_songs: bool,
    with_play: bool,
) -> Vec<Vec<MenuItem>> {
    let queue = [
        ("Play", AlbumAction::Play),
        ("Play Next", AlbumAction::Enqueue(Placement::Next)),
        ("Add to Queue", AlbumAction::Enqueue(Placement::Last)),
    ];
    let queue = queue
        .into_iter()
        .filter(|(label, _)| with_play || *label != "Play")
        .map(|(label, action)| {
            let (screen, playlist) = (screen.clone(), playlist.clone());
            menu_item(label)
                .disabled_when(move || !has_songs)
                .on_click(move || screen.act_on(&playlist, action.clone()))
        })
        .collect();
    let mut groups = vec![queue];
    if screen.state.snapshot().is_editable(playlist) {
        let (renaming, deleting) = (screen.clone(), screen.clone());
        let (rename, delete) = (playlist.clone(), playlist.clone());
        groups.push(vec![
            menu_item("Rename…").on_click(move || renaming.dialogs.rename(rename.clone())),
            menu_item("Delete…").on_click(move || deleting.dialogs.delete(delete.clone())),
        ]);
    }
    groups
}

fn list_context(screen: &Playlists) -> KeyContext<KeyCommand> {
    let (asked, running) = (screen.clone(), screen.clone());
    KeyContext::new(
        keys::LIST,
        move |command| {
            let Some(playlist) = asked.selected() else {
                return false;
            };
            match command {
                KeyCommand::OpenSongs
                | KeyCommand::Play
                | KeyCommand::PlayNext
                | KeyCommand::AddToQueue => true,
                KeyCommand::Rename | KeyCommand::Delete => {
                    asked.shown.with(|shown| shown.is_editable(&playlist))
                }
                _ => false,
            }
        },
        move |command| {
            let screen = &running;
            let Some(playlist) = untrack(|| screen.selected()) else {
                return;
            };
            match command {
                KeyCommand::OpenSongs => screen.focus_songs(),
                KeyCommand::Play => screen.act_on(&playlist, AlbumAction::Play),
                KeyCommand::PlayNext => {
                    screen.act_on(&playlist, AlbumAction::Enqueue(Placement::Next));
                }
                KeyCommand::AddToQueue => {
                    screen.act_on(&playlist, AlbumAction::Enqueue(Placement::Last));
                }
                KeyCommand::Rename => screen.dialogs.rename(playlist),
                KeyCommand::Delete => screen.dialogs.delete(playlist),
                _ => {}
            }
        },
    )
}
