//! Which playlist is chosen, which the window's location says, and what the list, the detail and
//! the dialogs of the screen share.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::core::reactive::effect_with;
use anti_webui::prelude::*;
use anti_webui_md3e::components::Snackbar;
use hiphile_core::navigation::{Destination, Location};
use hiphile_core::playlists::{Entry, EntryId, OpenPlaylist, PlaylistsSnapshot, PlaylistsState};
use hiphile_core::selection::after_removal;
use next_host::{Pending, Scope, follow};
use next_ui::context_menu::ContextMenu;
use next_ui::list_detail::{Split, list_detail};
use next_ui::now_playing::{NowPlaying, follow_now_playing};
use next_ui::row_list::{ROW_HEIGHT, Selection, reveal};
use next_ui::{AlbumAction, Command, Screen, ScreenContext};
use subsonic::{Playlist, PlaylistId, Song};

mod detail;
mod dialogs;
mod keys;
mod list;
mod song_row;

use dialogs::Dialogs;

thread_local! {
    /// How many songs have been taken out of playlists. Only the last can be put back, so an
    /// older snackbar's Undo does nothing.
    static REMOVALS: Cell<u64> = const { Cell::new(0) };
}

/// The playlists beside the songs of the one `location` names. While it names none, or one that
/// has gone, it is the one open before or else the first listed. Choosing another with the pointer
/// sends [`Command::Show`], and moving to it with the keys [`Command::ShowInPlace`]. The list is as
/// wide as `split` says.
pub fn playlists(location: Signal<Location>, split: Split, cx: &ScreenContext) -> Screen {
    keys::bind();
    let screen = Playlists::new(location, cx);
    screen.follow_location();
    screen.follow_songs();
    let body = list_detail(split, list::pane(&screen), detail::pane(&screen))
        .child(screen.menu.view())
        .child(dialogs::rename_dialog(&screen))
        .child(dialogs::delete_dialog(&screen));
    Screen::new(Destination::Playlists.label(), body).home(screen.list_node)
}

/// What the parts of the screen share. Cheap to clone.
#[derive(Clone)]
struct Playlists {
    cx: ScreenContext,
    state: PlaylistsState,
    /// What `state` last published, which the views follow. What the screen does reads `state`
    /// itself, which has its own changes before they arrive here.
    shown: Signal<PlaylistsSnapshot>,
    /// The playlist the location names, while the location is this screen's.
    wanted: Memo<Option<Option<PlaylistId>>>,
    username: Arc<str>,
    filter: Signal<String>,
    list_selection: Selection<PlaylistId>,
    songs_selection: Selection<EntryId>,
    list_node: NodeRef,
    songs_node: NodeRef,
    /// Whether the songs had the focus, which they take back when they are made again.
    songs_focused: Signal<bool>,
    /// Whether the list has taken the focus once, as it does when it first shows.
    list_focused: Rc<Cell<bool>>,
    menu: ContextMenu,
    dialogs: Dialogs,
    now_playing: Memo<Option<NowPlaying>>,
    scope: Scope,
    /// The playlist last opened, which is opened again only to try it again.
    opened: Rc<RefCell<Option<PlaylistId>>>,
    /// The songs of a playlist being had to act on, which the next such action calls off.
    acting: Rc<RefCell<Option<Pending>>>,
}

/// What the location should name.
enum Kept {
    Wanted,
    Instead(Option<PlaylistId>),
}

impl Playlists {
    fn new(location: Signal<Location>, cx: &ScreenContext) -> Self {
        let state = cx.workspace.playlists().clone();
        let shown = follow(state.subscribe());
        let wanted = memo(move || {
            location.with(|location| match location {
                Location::Playlists(playlist) => Some(playlist.clone()),
                _ => None,
            })
        });
        let now_playing = follow_now_playing(&cx.workspace);
        let filter = signal(state.query());
        {
            let state = state.clone();
            effect(move || {
                let query = filter.get();
                untrack(|| state.set_query(&query));
            });
        }
        Self {
            username: Arc::from(cx.workspace.session().username()),
            cx: cx.clone(),
            state,
            shown,
            wanted,
            filter,
            list_selection: Selection::new(),
            songs_selection: Selection::new(),
            list_node: node_ref(),
            songs_node: node_ref(),
            songs_focused: signal(false),
            list_focused: Rc::default(),
            menu: ContextMenu::new(cx.overlays),
            dialogs: Dialogs::new(cx.overlays),
            now_playing,
            scope: Scope::here(),
            opened: Rc::default(),
            acting: Rc::default(),
        }
    }

    /// The playlist chosen. Tracks it.
    fn selected(&self) -> Option<PlaylistId> {
        self.wanted.get().flatten()
    }

    /// Shows `playlist`, chosen with the pointer, which leaves the playlist that was on show to go
    /// back to.
    fn choose(&self, playlist: PlaylistId) {
        self.cx
            .commands
            .send(Command::Show(Location::Playlists(Some(playlist))));
    }

    fn show_in_place(&self, playlist: Option<PlaylistId>) {
        self.cx
            .commands
            .send(Command::ShowInPlace(Location::Playlists(playlist)));
    }

    /// Keeps the location, the list's selection and the open playlist one.
    ///
    /// An effect that writes what it reads is left out of every later change, so none of these
    /// does: the location is changed from outside them, and the selection put right by another.
    fn follow_location(&self) {
        let shown = self.shown;
        let listed = memo(move || shown.with(|shown| shown.loaded_list().cloned()));
        let this = self.clone();
        let settle = ui_channel(move |()| this.settle());
        let this = self.clone();
        effect(move || {
            let Some(wanted) = this.wanted.get() else {
                return;
            };
            listed.with(|_| ());
            untrack(|| match this.kept(&wanted) {
                Kept::Wanted => {
                    if let Some(playlist) = wanted {
                        this.open(playlist);
                    }
                }
                Kept::Instead(_) => settle.send(()),
            });
        });

        let reselect = signal(());
        let this = self.clone();
        effect(move || {
            let Some(wanted) = this.wanted.get() else {
                return;
            };
            reselect.with(|_| ());
            untrack(|| this.select_in_list(wanted));
        });

        // One playlist is chosen at a time, so the rows a selection could take in besides go
        let this = self.clone();
        effect(move || {
            let listed = untrack(|| this.listed_ids());
            let only = match this.list_selection.in_order(listed).as_slice() {
                [only] => Some(only.clone()),
                _ => None,
            };
            untrack(|| {
                let Some(wanted) = this.wanted.get() else {
                    return;
                };
                match only {
                    Some(chosen) if wanted.as_ref() != Some(&chosen) => {
                        this.show_in_place(Some(chosen));
                    }
                    Some(_) => {}
                    None => reselect.set(()),
                }
            });
        });

        let this = self.clone();
        effect_with(move |was: Option<Option<Option<PlaylistId>>>| {
            let now = this.wanted.get();
            if was.is_some_and(|was| was != now)
                && let Some(Some(playlist)) = &now
            {
                untrack(|| this.reveal(playlist));
            }
            now
        });
    }

    /// What the location should name in place of `wanted`. The playlist wanted stays while it is
    /// listed, or while there is no list to tell. Else it is the one open, or the first.
    fn kept(&self, wanted: &Option<PlaylistId>) -> Kept {
        let shown = self.state.snapshot();
        let Some(list) = shown.loaded_list() else {
            return Kept::Wanted;
        };
        let is_listed = |id: &PlaylistId| list.iter().any(|listed| listed.id() == id);
        if wanted.as_ref().is_some_and(is_listed) {
            return Kept::Wanted;
        }
        let open = shown
            .open_playlist()
            .map(|open| open.playlist().id())
            .filter(|id| is_listed(id));
        let kept = open
            .or_else(|| shown.visible_playlist(0).map(Playlist::id))
            .cloned();
        if kept == *wanted {
            Kept::Wanted
        } else {
            Kept::Instead(kept)
        }
    }

    /// Shows the playlist the location should name, if it names another.
    fn settle(&self) {
        let Some(wanted) = untrack(|| self.wanted.get()) else {
            return;
        };
        if let Kept::Instead(kept) = self.kept(&wanted) {
            self.show_in_place(kept);
        }
    }

    fn select_in_list(&self, wanted: Option<PlaylistId>) {
        match wanted {
            Some(playlist) => self.list_selection.select(playlist),
            None => self.list_selection.clear(),
        }
    }

    /// The playlists the list shows, in its order.
    fn listed_ids(&self) -> Vec<PlaylistId> {
        self.shown.with(|shown| {
            (0..shown.visible().len())
                .filter_map(|at| {
                    shown
                        .visible_playlist(at)
                        .map(|playlist| playlist.id().clone())
                })
                .collect()
        })
    }

    /// Opening a playlist reads its songs again, so the one open stays as it is.
    fn open(&self, playlist: PlaylistId) {
        if self.opened.borrow().as_ref() == Some(&playlist) {
            return;
        }
        self.songs_selection.clear();
        let is_open = self
            .state
            .snapshot()
            .open_playlist()
            .is_some_and(|open| open.playlist().id() == &playlist);
        self.opened.replace(Some(playlist.clone()));
        if !is_open {
            self.state.open(playlist);
        }
    }

    /// Reads the chosen playlist's songs again, after they failed to load.
    fn reopen(&self) {
        let Some(playlist) = untrack(|| self.selected()) else {
            return;
        };
        self.songs_selection.clear();
        self.opened.replace(Some(playlist.clone()));
        self.state.open(playlist);
    }

    /// Brings `playlist` into view in the list and focuses the list, emptying a filter that
    /// hides it. One not listed is left alone.
    fn reveal(&self, playlist: &PlaylistId) {
        let mut shown = self.state.snapshot();
        let Some(listed_at) = shown
            .loaded_list()
            .and_then(|list| list.iter().position(|listed| listed.id() == playlist))
        else {
            return;
        };
        if !shown.visible().contains(&listed_at) {
            // At once, before the filter's own effect runs
            self.state.set_query("");
            self.filter.set(String::new());
            shown = self.state.snapshot();
        }
        if let Some(at) = shown
            .visible()
            .iter()
            .position(|visible| *visible == listed_at)
        {
            reveal(self.list_node, at, ROW_HEIGHT);
        }
        self.list_node.focus();
    }

    /// Scrolls the list to the playlist chosen, which may not be laid out before.
    fn reveal_selected(&self) {
        let Some(playlist) = untrack(|| self.selected()) else {
            return;
        };
        let shown = self.state.snapshot();
        let at = (0..shown.visible().len()).find(|at| {
            shown
                .visible_playlist(*at)
                .is_some_and(|listed| listed.id() == &playlist)
        });
        if let Some(at) = at {
            reveal(self.list_node, at, ROW_HEIGHT);
        }
    }

    /// The playlist as the open detail or the list has it.
    fn playlist(&self, id: &PlaylistId) -> Option<Playlist> {
        let shown = self.state.snapshot();
        shown
            .open_playlist()
            .map(OpenPlaylist::playlist)
            .filter(|open| open.id() == id)
            .or_else(|| shown.loaded_list()?.iter().find(|listed| listed.id() == id))
            .cloned()
    }

    /// Does `action` to the whole of `playlist`, whose songs may still be on their way.
    fn act_on(&self, playlist: &PlaylistId, action: AlbumAction) {
        if let Some(before) = self.acting.take() {
            before.cancel();
        }
        let songs = self.state.songs_of(playlist);
        let (commands, snackbars) = (self.cx.commands.clone(), self.cx.snackbars);
        let pending = self.scope.on_ui(songs, move |songs| match songs {
            Ok(songs) => {
                if let Some(command) = action.command(songs) {
                    commands.send(command);
                }
            }
            Err(error) => snackbars.show(Snackbar::new(error).long()),
        });
        self.acting.replace(Some(pending));
    }

    /// Takes `playlist` off the list. When it was the one chosen, the one that takes its place
    /// in the list is chosen instead.
    fn delete(&self, playlist: PlaylistId) {
        let before = self.state.snapshot();
        let position = (0..before.visible().len()).find(|at| {
            before
                .visible_playlist(*at)
                .is_some_and(|listed| listed.id() == &playlist)
        });
        if let Err(error) = self.state.delete(playlist.clone()) {
            self.cx.snackbars.show(Snackbar::new(error).long());
            return;
        }
        if untrack(|| self.selected()).as_ref() == Some(&playlist) {
            let after = self.state.snapshot();
            let next = position
                .and_then(|position| after_removal(position, after.visible().len()))
                .and_then(|position| after.visible_playlist(position))
                .map(|listed| listed.id().clone());
            self.show_in_place(next);
        }
        self.list_node.focus();
    }

    /// The songs of the playlist open, when it has any.
    fn open_songs(&self) -> Option<Arc<[Song]>> {
        let shown = self.state.snapshot();
        let open = shown.open_playlist()?;
        (!open.entries().is_empty()).then(|| open.entries().songs())
    }

    /// Moves the focus to the songs, selecting the first when none is.
    fn focus_songs(&self) {
        let shown = self.state.snapshot();
        let Some(entries) = shown.open_playlist().map(OpenPlaylist::entries) else {
            return;
        };
        let Some(first) = entries.get(0) else {
            return;
        };
        let is_listed = self
            .songs_selection
            .cursor()
            .is_some_and(|cursor| entries.position(cursor).is_some());
        if !is_listed {
            self.songs_selection.select(first.id());
        }
        self.songs_node.focus();
    }

    /// The entry of the open playlist, and where it is.
    fn entry(&self, id: EntryId) -> Option<(usize, Entry)> {
        let shown = self.state.snapshot();
        let entries = shown.open_playlist()?.entries();
        let ix = entries.position(id)?;
        Some((ix, entries.get(ix)?.clone()))
    }

    /// The song the cursor of the songs is on. Tracks the cursor and the songs.
    fn cursor_song(&self) -> Option<Song> {
        let cursor = self.songs_selection.cursor()?;
        self.shown.with(|shown| {
            let entries = shown.open_playlist()?.entries();
            Some(entries.get(entries.position(cursor)?)?.song().clone())
        })
    }

    fn play_from(&self, entry: EntryId) {
        let shown = self.state.snapshot();
        let Some(entries) = shown.open_playlist().map(OpenPlaylist::entries) else {
            return;
        };
        if let Some(start) = entries.position(entry) {
            self.cx.commands.send(Command::Play {
                songs: entries.songs(),
                start: Some(start),
            });
        }
    }

    /// Moves the entry up for less than 0 and down for more. The top stays the top.
    fn move_entry(&self, entry: EntryId, by: isize) {
        let Some((ix, _)) = self.entry(entry) else {
            return;
        };
        if let Some(to) = ix.checked_add_signed(by)
            && self.state.move_entry(entry, to)
        {
            self.songs_selection.select(entry);
        }
    }

    /// Puts `ids`, in their order, before the entry at `before`, as a drag drops them.
    fn reorder(&self, ids: Vec<EntryId>, mut before: usize) {
        for id in &ids {
            let Some((from, _)) = self.entry(*id) else {
                continue;
            };
            let to = if from < before { before - 1 } else { before };
            self.state.move_entry(*id, to);
            before = to + 1;
        }
        if let [id] = ids[..] {
            self.songs_selection.select(id);
        }
        self.songs_node.focus();
    }

    /// Takes the entry out of the open playlist, selecting the one in its place, and offers to
    /// put it back.
    fn remove(&self, entry: EntryId) {
        let Some(removal) = self.state.remove_entry(entry) else {
            return;
        };
        let shown = self.state.snapshot();
        let next = shown.open_playlist().and_then(|open| {
            let at = after_removal(removal.ix(), open.entries().len())?;
            open.entries().get(at).map(Entry::id)
        });
        match next {
            Some(next) => self.songs_selection.select(next),
            None => self.songs_selection.clear(),
        }
        let removed = REMOVALS.with(|removals| {
            removals.set(removals.get() + 1);
            removals.get()
        });
        let this = self.clone();
        let undo = move || {
            if REMOVALS.with(Cell::get) != removed {
                return;
            }
            let put_back = this.state.undo_removal();
            if let Some(put_back) = put_back
                && this.shown.is_alive()
            {
                this.songs_selection.select(put_back);
            }
        };
        self.cx
            .snackbars
            .show(Snackbar::new(removal.message()).action("Undo", undo).long());
    }

    /// Follows the songs' selection to where its song is after the playlist changed, and keeps
    /// it to one song, which is what their commands act on.
    fn follow_songs(&self) {
        let (shown, selection) = (self.shown, self.songs_selection);
        effect_with(move |was: Option<Option<OpenPlaylist>>| {
            let now = shown.with(|shown| shown.open_playlist().cloned());
            if let (Some(Some(was)), Some(now)) = (&was, &now) {
                untrack(|| {
                    let Some(cursor) = selection.cursor() else {
                        return;
                    };
                    match now.follow(was, cursor) {
                        Some(kept) if kept != cursor => selection.select(kept),
                        Some(_) => {}
                        None => selection.clear(),
                    }
                });
            }
            now
        });
        let narrowed = signal(None::<EntryId>);
        effect(move || {
            let several = selection.with(|selected| {
                let listed = shown.with_untracked(|shown| {
                    shown.open_playlist().map_or(0, |open| {
                        let entries = open.entries();
                        selected.count_in(
                            (0..entries.len()).filter_map(|ix| Some(entries.get(ix)?.id())),
                        )
                    })
                });
                (listed > 1).then(|| selected.cursor().copied()).flatten()
            });
            if several.is_some() {
                untrack(|| narrowed.set(several));
            }
        });
        effect(move || {
            if let Some(cursor) = narrowed.get() {
                untrack(|| selection.select(cursor));
            }
        });
    }
}

#[cfg(test)]
mod tests;
