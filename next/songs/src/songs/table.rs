use anti_webui::prelude::*;
use hiphile_core::Shown;
use hiphile_core::songs::{SongList, SongListStatus, Sort};
use next_host::follow;
use next_ui::ScreenContext;
use next_ui::context_menu::ContextMenu;
use next_ui::keymap::key_context;
use next_ui::now_playing::{NowPlaying, PlayState, follow_now_playing, song_row_label};
use next_ui::row_list::{RowList, RowSource, Selection, row_list};
use subsonic::{Song, SongId};

use super::columns::{ROW_HEIGHT, headings};
use super::song_row::song_row;

/// What the parts of the table share: the songs as the core arranges them, which are selected,
/// the song playing, and where the commands on them go. Cheap to clone.
#[derive(Clone)]
pub(super) struct Table {
    pub(super) cx: ScreenContext,
    pub(super) song_list: SongList,
    pub(super) status: Signal<SongListStatus>,
    pub(super) shown: Memo<Option<Shown<Song>>>,
    /// The order the songs are in, or `None` while they go by how well they match the filter.
    pub(super) sort: Memo<Option<Sort>>,
    pub(super) selection: Selection<SongId>,
    /// Whether a row was pressed, which chooses what is selected.
    pub(super) chose: Signal<bool>,
    pub(super) now_playing: Memo<Option<NowPlaying>>,
    pub(super) menu: ContextMenu,
    /// The rows, which take the focus.
    pub(super) list: NodeRef,
}

impl Table {
    /// Follows the songs of `cx` and its player for as long as the current owner lives.
    pub(super) fn new(cx: &ScreenContext) -> Self {
        let song_list = cx.workspace.songs().clone();
        let status = follow(song_list.subscribe());
        Self {
            cx: cx.clone(),
            song_list,
            status,
            shown: memo(move || status.with(|status| status.shown().cloned())),
            sort: memo(move || status.with(|status| (!status.is_ranked()).then(|| status.sort()))),
            selection: Selection::new(),
            chose: signal(false),
            now_playing: follow_now_playing(&cx.workspace),
            menu: ContextMenu::new(cx.overlays),
            list: node_ref(),
        }
    }

    /// Tracks it.
    pub(super) fn state_of(&self, song: &SongId) -> Option<PlayState> {
        self.now_playing
            .with(|now| now.as_ref().and_then(|now| now.state_of(song)))
    }

    /// The headings, and the rows below them, where the keys of the songs work.
    pub(super) fn view(&self) -> Div {
        let shown = self.shown;
        let (known, row, label, open, menu) = (
            self.cx.known,
            self.clone(),
            self.clone(),
            self.clone(),
            self.clone(),
        );
        let list = RowList::new(
            "Songs",
            move || Rows(shown.get()),
            |song: &Song| song.id().clone(),
            move |song, state| song_row(&row, song, state),
        )
        .row_height(ROW_HEIGHT)
        .selection(self.selection)
        .node_ref(self.list)
        .row_label(move |song| {
            song_row_label(
                song.title(),
                label.state_of(song.id()),
                known.is_skipped(song),
            )
        })
        .on_open(move |song| open.open(&song))
        .on_context_menu(move |song, at| {
            let songs = menu.songs_for(&song);
            menu.menu.open_at(at, menu.menu_groups(songs));
        });
        let chose = self.chose;
        let rows = div()
            .flex_1()
            .min_h(0)
            .flex()
            .flex_col()
            .on_pointer_down(move |_| chose.set(true))
            .child(row_list(list).flex_1().min_h(0));
        div()
            .flex_1()
            .min_h(0)
            .flex()
            .flex_col()
            .child(headings(self))
            .child(key_context(rows, &self.key_context()))
    }
}

/// The songs on show, of which there are none until the first are arranged.
struct Rows(Option<Shown<Song>>);

impl RowSource for Rows {
    type Item = Song;

    fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |shown| shown.visible().len())
    }

    fn get(&self, position: usize) -> Option<&Song> {
        self.0.as_ref()?.get(position)
    }
}
