//! The queue beside the screens: what plays after what, which the user plays from, reorders and
//! takes from.

use std::cell::Cell;
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{
    IconButtonStyle, MenuItem, menu_item, styled_icon_button, tooltip,
};
use anti_webui_md3e::icons;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::favorites::Favorite;
use hiphile_core::navigation::Location;
use next_host::follow;
use next_ui::context_menu::ContextMenu;
use next_ui::details::format_song_count;
use next_ui::keymap::{KeyBinding, KeyContext, Keystroke, bind_keys, key_context, keystroke_for};
use next_ui::link::{artist_line, song_artist_link};
use next_ui::list_pane::{ListHeader, list_header};
use next_ui::load_state::empty_state;
use next_ui::menus::{
    Songs, add_to_playlist_item, favorite_item, go_to_album_item, go_to_artist_item,
};
use next_ui::now_playing::{PlayState, song_row_label};
use next_ui::pane::{GAP, INSET, RADIUS};
use next_ui::pane_handle::{Resizes, pane_handle};
use next_ui::row_list::{
    ROW_HEIGHT, RowList, RowSource, RowState, Selection, row_list, scroll_to_row,
};
use next_ui::song_row::{SongRow, song_row, song_words};
use next_ui::type_scale::styled_text;
use next_ui::window::moves_window;
use next_ui::{APP_BAR_HEIGHT, Command, Commands, Known, ScreenContext};
use playback::{Entry, EntryId, EntryState, PlayerSnapshot};
use subsonic::Song;

use crate::panels::{Panels, QUEUE_MIN_WIDTH};

const QUEUE_CONTEXT: &str = "Queue";

/// What the keys do in the queue, past moving through it and Enter, which plays.
#[derive(Clone, Debug, PartialEq)]
enum QueueKey {
    PlayNext,
    MoveUp,
    MoveDown,
    Remove,
    AddToPlaylist,
    ToggleFavorite,
    GoToArtist,
    TogglePlayback,
}

thread_local! {
    static BOUND: Cell<bool> = const { Cell::new(false) };
}

/// Once for each thread, as its keymap is.
fn bind_queue_keys() {
    if BOUND.replace(true) {
        return;
    }
    let named = Keystroke::named;
    bind_keys(
        [
            (named(NamedKey::Enter).with_alt(), QueueKey::PlayNext),
            (named(NamedKey::ArrowUp).with_alt(), QueueKey::MoveUp),
            (named(NamedKey::ArrowDown).with_alt(), QueueKey::MoveDown),
            (named(NamedKey::Delete), QueueKey::Remove),
            // After Delete, as a menu shows the key bound last
            (named(NamedKey::Backspace), QueueKey::Remove),
            (Keystroke::primary('p'), QueueKey::AddToPlaylist),
            (Keystroke::primary('d'), QueueKey::ToggleFavorite),
            (Keystroke::primary('a').with_shift(), QueueKey::GoToArtist),
            (named(NamedKey::Space), QueueKey::TogglePlayback),
        ]
        .into_iter()
        .map(|(stroke, key)| KeyBinding::new(stroke, key, QUEUE_CONTEXT)),
    );
}

/// The entries as the player has them. Equal while they are the same, which the player keeps
/// until they change.
#[derive(Clone)]
struct Entries(Arc<Vec<Entry<Song>>>);

impl PartialEq for Entries {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl RowSource for Entries {
    type Item = Entry<Song>;

    fn len(&self) -> usize {
        self.0.len()
    }

    fn get(&self, position: usize) -> Option<&Entry<Song>> {
        self.0.get(position)
    }
}

/// The queue of the workspace's player beside the screens: its title in line with theirs, and
/// below it a pane as wide as `panels` says, with a handle at its start that makes it wider or
/// narrower. The selection stays in `panels` while it is closed.
pub fn queue(cx: &ScreenContext, panels: Panels) -> Div {
    bind_queue_keys();
    let shown = follow(cx.workspace.player().subscribe());
    let width = memo(move || panels.queue_width());
    let title = moves_window(
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(APP_BAR_HEIGHT)
            .pl(GAP + INSET)
            .select_none()
            .child(
                styled_text(TextStyle::TITLE_LARGE, "Queue")
                    .access(|a| a.role(Role::Heading).level(2)),
            ),
    );
    let handle = pane_handle(
        "Queue Width",
        Resizes::After,
        move || width.get(),
        QUEUE_MIN_WIDTH,
        move || panels.queue_most(),
        move |wanted| panels.set_queue_width(wanted),
    );
    let pane = div()
        .flex()
        .flex_col()
        .flex_none()
        .min_h(0)
        .rounded(RADIUS)
        .overflow_hidden()
        .dyn_style(move |s| s.w(width.get()).bg(color_scheme().surface_container_low))
        .child(entries_view(cx, shown, panels.queue_selection));
    div()
        .flex()
        .flex_col()
        .flex_none()
        .min_h(0)
        .child(title)
        .child(
            div()
                .flex_1()
                .min_h(0)
                .flex()
                .flex_row()
                .child(handle)
                .child(pane),
        )
}

/// What acts on the queue for its keys, its menus and its rows. Cheap to clone.
#[derive(Clone)]
struct Actions {
    player: playback::Player,
    commands: Commands,
    known: Known,
    selection: Selection<EntryId>,
    list: NodeRef,
}

impl Actions {
    /// The selected entries, in the order they are in now.
    fn selected(&self) -> Vec<EntryId> {
        let shown = self.player.snapshot();
        let ids = shown.queue().entries().iter().map(Entry::id);
        untrack(|| self.selection.in_order(ids))
    }

    fn song_of(&self, id: EntryId) -> Option<Song> {
        let shown = self.player.snapshot();
        let queue = shown.queue();
        Some(queue.entries()[queue.position(id)?].item().clone())
    }

    fn cursor_song(&self) -> Option<Song> {
        self.song_of(untrack(|| self.selection.cursor())?)
    }

    fn selected_songs(&self) -> Arc<[Song]> {
        self.selected()
            .into_iter()
            .filter_map(|id| self.song_of(id))
            .collect()
    }

    /// Where Move Up or Move Down takes the selected entries: the position they go before.
    fn moved_to(&self, up: bool) -> Option<usize> {
        let shown = self.player.snapshot();
        let queue = shown.queue();
        let positions: Vec<usize> = self
            .selected()
            .into_iter()
            .filter_map(|id| queue.position(id))
            .collect();
        let (first, last) = (*positions.first()?, *positions.last()?);
        if up {
            first.checked_sub(1)
        } else {
            (last + 1 < queue.len()).then_some(last + 2)
        }
    }

    /// Whether any of the selected entries can be changed so.
    fn any_selected(&self, can: impl Fn(playback::EntryCommands) -> bool) -> bool {
        let shown = self.player.snapshot();
        self.selected()
            .into_iter()
            .filter_map(|id| shown.queue().commands_for(id))
            .any(can)
    }

    fn is_enabled(&self, key: &QueueKey) -> bool {
        match key {
            QueueKey::PlayNext => self.any_selected(|can| can.play_next),
            QueueKey::MoveUp => self.moved_to(true).is_some(),
            QueueKey::MoveDown => self.moved_to(false).is_some(),
            QueueKey::Remove => self.any_selected(|can| can.remove),
            QueueKey::AddToPlaylist => !self.selected().is_empty(),
            QueueKey::ToggleFavorite => self.cursor_song().is_some_and(|song| {
                let target = Favorite::Song(song).target();
                untrack(|| self.known.is_favorite(&target)).is_some()
            }),
            QueueKey::GoToArtist => self
                .cursor_song()
                .is_some_and(|song| song.artist_id().is_some()),
            QueueKey::TogglePlayback => self.player.snapshot().has_song(),
        }
    }

    fn run(&self, key: QueueKey) {
        match key {
            QueueKey::PlayNext => self.play_next(),
            QueueKey::MoveUp | QueueKey::MoveDown => {
                if let Some(to) = self.moved_to(key == QueueKey::MoveUp) {
                    move_entries(&self.player, &self.selected(), to);
                }
            }
            QueueKey::Remove => self.remove(),
            QueueKey::AddToPlaylist => self
                .commands
                .send(Command::ChoosePlaylist(self.selected_songs())),
            QueueKey::ToggleFavorite => {
                if let Some(song) = self.cursor_song() {
                    self.commands
                        .send(Command::ToggleFavorite(Favorite::Song(song)));
                }
            }
            QueueKey::GoToArtist => {
                if let Some(artist) = self
                    .cursor_song()
                    .and_then(|song| song.artist_id().cloned())
                {
                    self.commands
                        .send(Command::Show(Location::Artists(Some(artist))));
                }
            }
            QueueKey::TogglePlayback => self.commands.send(Command::TogglePlayback),
        }
    }

    fn play(&self, id: EntryId) {
        self.commands.send(Command::PlayEntry(id));
    }

    /// In the order they are in, after the one that plays.
    fn play_next(&self) {
        let shown = self.player.snapshot();
        let ids: Vec<EntryId> = self
            .selected()
            .into_iter()
            .filter(|id| {
                shown
                    .queue()
                    .commands_for(*id)
                    .is_some_and(|can| can.play_next)
            })
            .collect();
        for id in ids.into_iter().rev() {
            self.player.move_entry_to_next(id);
        }
    }

    /// Removes what can be of the selection, and selects the entry that took the place of the
    /// first, so that removing again goes on down the queue.
    fn remove(&self) {
        let shown = self.player.snapshot();
        let queue = shown.queue();
        let removed: Vec<(usize, EntryId)> = self
            .selected()
            .into_iter()
            .filter(|id| queue.commands_for(*id).is_some_and(|can| can.remove))
            .filter_map(|id| Some((queue.position(id)?, id)))
            .collect();
        let Some(&(first, _)) = removed.first() else {
            return;
        };
        for (_, id) in removed {
            self.player.remove_entry(id);
        }
        let shown = self.player.snapshot();
        let entries = shown.queue().entries();
        match entries.get(first.min(entries.len().saturating_sub(1))) {
            Some(entry) => self.selection.select(entry.id()),
            None => self.selection.clear(),
        }
    }

    /// The entries dragged onto `to`, which the one dragged becomes the selection of.
    fn drop(&self, ids: Vec<EntryId>, to: usize) {
        move_entries(&self.player, &ids, to);
        if let [only] = ids[..] {
            self.selection.select(only);
        }
        self.list.focus();
    }

    /// An item that does what `key` does, and shows that key.
    fn key_item(&self, label: &str, key: QueueKey) -> MenuItem {
        let item = with_keystroke(menu_item(label), &key);
        if self.is_enabled(&key) {
            let this = self.clone();
            item.on_click(move || this.run(key.clone()))
        } else {
            item.disabled_when(|| true)
        }
    }

    /// The menu of the entry `id`, on which the pointer was pressed, as the queue is now. What
    /// changes the queue's order changes the selection's.
    fn menu(&self, id: EntryId) -> Vec<Vec<MenuItem>> {
        let Some(song) = self.song_of(id) else {
            return Vec::new();
        };
        let play = {
            let this = self.clone();
            let item = menu_item("Play").on_click(move || this.play(id));
            item.trailing(Keystroke::named(NamedKey::Enter).to_string())
        };
        let songs = Songs::List(self.selected_songs());
        let mut others = vec![
            add_to_playlist_item(&self.commands, self.known, &songs),
            with_keystroke(
                favorite_item(&self.commands, self.known, Favorite::Song(song.clone())),
                &QueueKey::ToggleFavorite,
            ),
        ];
        others.extend(go_to_album_item(&self.commands, song.album_id()));
        others.extend(
            go_to_artist_item(&self.commands, song.artist_id())
                .map(|item| with_keystroke(item, &QueueKey::GoToArtist)),
        );
        vec![
            vec![play, self.key_item("Play Next", QueueKey::PlayNext)],
            vec![
                self.key_item("Move Up", QueueKey::MoveUp),
                self.key_item("Move Down", QueueKey::MoveDown),
            ],
            others,
            vec![self.key_item("Remove", QueueKey::Remove)],
        ]
    }
}

fn with_keystroke(item: MenuItem, key: &QueueKey) -> MenuItem {
    match keystroke_for(key, QUEUE_CONTEXT) {
        Some(stroke) => item.trailing(stroke.to_string()),
        None => item,
    }
}

/// Moves `ids`, in their order, to go before the position `to` of the queue as it is now, or after
/// the last for its length. Each goes before the one after it, last first, as one move shifts what
/// is after where it was.
fn move_entries(player: &playback::Player, ids: &[EntryId], to: usize) {
    let shown = player.snapshot();
    let entries = shown.queue().entries();
    let mut before = entries[to.min(entries.len())..]
        .iter()
        .map(Entry::id)
        .find(|id| !ids.contains(id));
    for &id in ids.iter().rev() {
        let shown = player.snapshot();
        let queue = shown.queue();
        let Some(from) = queue.position(id) else {
            continue;
        };
        let at = before
            .and_then(|before| queue.position(before))
            .unwrap_or(queue.len());
        player.move_entry(id, if from < at { at - 1 } else { at });
        before = Some(id);
    }
}

/// The count and what acts on the whole queue, above its entries.
fn entries_view(
    cx: &ScreenContext,
    shown: Signal<PlayerSnapshot>,
    selection: Selection<EntryId>,
) -> Div {
    let player = cx.workspace.player().clone();
    let entries = memo(move || shown.with(|shown| Entries(shown.queue().snapshot())));
    let current = memo(move || shown.with(|shown| shown.queue().current_id()));
    let current_ix = memo(move || shown.with(|shown| shown.queue().current_ix()));
    let is_playing = memo(move || shown.with(PlayerSnapshot::is_playing));
    let is_shuffled = memo(move || shown.with(PlayerSnapshot::is_shuffled));
    let is_clearable = memo(move || shown.with(|shown| shown.queue().is_clearable()));
    let is_empty = move || entries.with(|entries| entries.0.is_empty());
    let list = node_ref();
    let actions = Actions {
        player: player.clone(),
        commands: cx.commands.clone(),
        known: cx.known,
        selection,
        list,
    };

    let save = {
        let commands = cx.commands.clone();
        tooltip(
            styled_icon_button(
                icons::PLAYLIST_ADD,
                IconButtonStyle::standard().disabled_when(is_empty),
            )
            .access(|a| a.label("Save as Playlist…"))
            .on_click(move |_| commands.send(Command::SaveQueue)),
            "Save as Playlist…",
        )
    };
    let clear = tooltip(
        styled_icon_button(
            icons::CLEAR_ALL,
            IconButtonStyle::standard().disabled_when(move || !is_clearable.get()),
        )
        .access(|a| a.label("Clear"))
        .on_click(move |_| player.clear_queue()),
        "Clear",
    );
    let header = list_header(
        ListHeader::new(move || {
            let count = entries.with(|entries| entries.0.len());
            (count > 0).then(|| format_song_count(count))
        })
        .action(save)
        .action(clear),
    )
    .mt(8);

    let menu = ContextMenu::new(cx.covers);
    let rows = {
        let commands = cx.commands.clone();
        RowList::new(
            "Queue",
            move || entries.get(),
            |entry: &Entry<Song>| entry.id(),
            move |entry, row| entry_row(entry, row, shown, &commands),
        )
    }
    .selection(selection)
    .node_ref(list)
    .row_label(move |entry| {
        let title = entry.item().title();
        let at = entries.with(|entries| entries.0.iter().position(|e| e.id() == entry.id()));
        match (at, current_ix.get()) {
            (Some(at), Some(current)) if at == current => {
                song_row_label(title, Some(PlayState::new(is_playing.get())), false)
            }
            (Some(at), Some(current)) if at > current => title.to_owned(),
            _ => format!("{title}, played"),
        }
    })
    .on_open({
        let actions = actions.clone();
        move |id| actions.play(id)
    })
    .on_context_menu({
        let actions = actions.clone();
        move |id, at| menu.open_at(at, actions.menu(id))
    })
    .on_reorder({
        let actions = actions.clone();
        move |ids, to| actions.drop(ids, to)
    })
    .keys_start_at(move || current.get());

    // The current entry comes into view as the queue opens, as another becomes current, and as
    // the order changes with the shuffle. Not as it moves with what is before it, which would
    // take the rows in view away from where the user is
    let laid_out = signal(false);
    effect(move || {
        let (_, _, laid_out) = (current.get(), is_shuffled.get(), laid_out.get());
        if let (true, Some(at)) = (laid_out, untrack(|| current_ix.get())) {
            untrack(|| scroll_to_row(list, at, ROW_HEIGHT));
        }
    });

    let keys = KeyContext::new(
        QUEUE_CONTEXT,
        {
            let actions = actions.clone();
            move |key| actions.is_enabled(key)
        },
        move |key| actions.run(key),
    );
    // The list stays while the queue is empty, so that the focus has somewhere to stay
    let body = div()
        .relative()
        .flex_1()
        .min_h(0)
        .flex()
        .flex_col()
        .on_resized(move |_, _| laid_out.set_if_changed(true))
        .child(row_list(rows).flex_1().min_h(0))
        .child(show(is_empty, || {
            empty_state("Add songs with Play Next or Add to Queue.", None, None)
                .absolute()
                .inset(0)
                .pointer_events_none()
        }));
    key_context(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(0)
            .child(header)
            .child(body)
            .child(menu.view()),
        &keys,
    )
}

/// The entry's title and artist, whether it plays, and how long it is, in a row of the queue. The
/// ones played are quieter. It has no buttons: the queue is narrow, and its keys and its menu do
/// what they would.
fn entry_row(
    entry: &Entry<Song>,
    row: RowState<EntryId>,
    shown: Signal<PlayerSnapshot>,
    commands: &Commands,
) -> Div {
    let position = row.position();
    let song = entry.item();
    let state = memo(move || shown.with(|shown| shown.queue().entry_state(position)));
    let is_playing = memo(move || shown.with(PlayerSnapshot::is_playing));
    let play_state =
        move || (state.get() == EntryState::Current).then(|| PlayState::new(is_playing.get()));
    let line = artist_line(song_artist_link(song, commands), None);
    let words = song_words(song, Some(line)).dyn_style(move |s| {
        if state.get() == EntryState::Played {
            s.text_color(color_scheme().on_surface_variant)
        } else {
            s
        }
    });
    song_row(SongRow::new(song, words, play_state))
}

#[cfg(test)]
mod tests;
