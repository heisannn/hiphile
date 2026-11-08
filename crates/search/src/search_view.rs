use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, IndexPath, StyledExt as _, WindowExt as _,
    command::{Command, CommandGroup, CommandItem, CommandState},
    h_flex, v_flex,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Context, Entity, EventEmitter, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Task, Window, div,
    prelude::FluentBuilder as _, rems,
};
use hiphile_core::search::{Hit, Results, SearchLibrary};
use subsonic::{AlbumId, ArtistId, Song};
use ui::no_matches;

#[cfg(test)]
mod tests;

/// In rem.
const WIDTH: f32 = 40.;
/// In rem.
const MAX_HEIGHT: f32 = 26.;

/// For the app shell to carry out: the palette floats above the workspace, so actions from it
/// would not reach there.
#[derive(Clone, Debug)]
pub enum SearchEvent {
    ShowArtist(ArtistId),
    ShowAlbum(AlbumId),
    PlaySong(Song),
}

enum SavedLibrary {
    /// Read on the palette's first opening, since a library that may never be
    /// searched is not worth reading.
    Unread,
    Read(Arc<SearchLibrary>),
    Failed(SharedString),
}

pub struct Search {
    session: session::Session,
    state: Entity<CommandState>,
    /// Stays in use while a newer reading is under way.
    library: SavedLibrary,
    is_reading: bool,
    is_reading_out_of_date: bool,
    /// The window it opened in, to show a first reading in. `None` until then.
    window: Option<AnyWindowHandle>,
    results: Arc<Results>,
    /// So an empty palette does not say that nothing matched.
    has_query: bool,
    _load_task: Task<()>,
    _search_task: Task<()>,
    _readings_changes: Option<Task<()>>,
}

impl EventEmitter<SearchEvent> for Search {}

impl Search {
    pub fn new(session: session::Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let readings_changes = session.readings().map(|readings| {
            ui::on_readings_changed(readings, cx, |this: &mut Self, cx| this.library_changed(cx))
        });
        Self {
            session,
            state: cx.new(|cx| CommandState::new(window, cx)),
            library: SavedLibrary::Unread,
            is_reading: false,
            is_reading_out_of_date: false,
            window: None,
            results: Arc::default(),
            has_query: false,
            _load_task: Task::ready(()),
            _search_task: Task::ready(()),
            _readings_changes: readings_changes,
        }
    }

    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.window = Some(window.window_handle());
        if matches!(self.library, SavedLibrary::Unread | SavedLibrary::Failed(_)) {
            self.read_library(cx);
        }
        self.state
            .update(cx, |state, cx| state.set_query("", window, cx));
        self.results = Arc::default();
        self.has_query = false;
        let this = cx.entity();
        window.open_dialog(cx, move |dialog, window, _| {
            dialog
                .w(window.rem_size() * WIDTH)
                .p_0()
                .close_button(false)
                .child(this.clone())
        });
        self.state.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Reads the library again, unless it was never read. What is found in the meantime comes from
    /// what was read before, and the palette may be open on it, so that stays until the new reading
    /// is done.
    pub fn library_changed(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.library, SavedLibrary::Unread) {
            self.read_library(cx);
        }
    }

    fn read_library(&mut self, cx: &mut Context<Self>) {
        if self.is_reading {
            self.is_reading_out_of_date = true;
            return;
        }
        let Some(catalog) = self.session.catalog().cloned() else {
            self.library = SavedLibrary::Failed("There's no saved library to search.".into());
            return;
        };
        self.is_reading = true;
        let book = self.session.reading_book();
        let state = self.state.clone();
        let window = self.window;
        let shows_spinner = !matches!(self.library, SavedLibrary::Read(_));
        self._load_task = cx.spawn(async move |this, cx| {
            if shows_spinner {
                show_spinner(&state, window, true, cx);
            }
            let read = cx
                .background_spawn(async move {
                    let albums = catalog.albums()?;
                    let songs = catalog.songs()?;
                    Ok::<_, catalog::Error>(match book {
                        Some(book) => {
                            book.read(|speller| SearchLibrary::new(albums, songs, Some(speller)))
                        }
                        None => SearchLibrary::new(albums, songs, None),
                    })
                })
                .await;
            if shows_spinner {
                show_spinner(&state, window, false, cx);
            }
            this.update(cx, |this, cx| this.finish_reading(read, cx))
                .ok();
        });
        cx.notify();
    }

    fn finish_reading(
        &mut self,
        read: Result<SearchLibrary, catalog::Error>,
        cx: &mut Context<Self>,
    ) {
        self.is_reading = false;
        self.library = match read {
            Ok(library) => SavedLibrary::Read(Arc::new(library)),
            Err(error) => {
                self.results = Arc::default();
                SavedLibrary::Failed(session::describe_catalog(&error).into())
            }
        };
        if std::mem::take(&mut self.is_reading_out_of_date) {
            self.read_library(cx);
        }
        let query = self.state.read(cx).query(cx);
        self.search(&query, cx);
        cx.notify();
    }

    fn search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.has_query = !query.trim().is_empty();
        let SavedLibrary::Read(library) = &self.library else {
            cx.notify();
            return;
        };
        let library = library.clone();
        let query = query.to_owned();
        self._search_task = cx.spawn(async move |this, cx| {
            let results = cx
                .background_spawn(async move { library.search(&query) })
                .await;
            this.update(cx, |this, cx| {
                this.results = Arc::new(results);
                cx.notify();
            })
            .ok();
        });
    }

    fn confirm(&mut self, path: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.results.hit(path.section, path.row).cloned() else {
            return;
        };
        window.close_dialog(cx);
        cx.emit(match hit {
            Hit::Artist(artist) => SearchEvent::ShowArtist(artist.id),
            Hit::Album(album) => SearchEvent::ShowAlbum(album.id().clone()),
            Hit::Song(song) => SearchEvent::PlaySong(song),
        });
    }

    fn render_empty(&self, cx: &App) -> gpui_kit::AnyElement {
        let is_waiting = self.is_reading && !matches!(self.library, SavedLibrary::Read(_));
        if let (SavedLibrary::Failed(error), false) = (&self.library, is_waiting) {
            return div()
                .p_4()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(error.clone())
                .into_any_element();
        }
        if !self.has_query || is_waiting {
            return div().into_any_element();
        }
        div()
            .p_4()
            .child(no_matches(
                "Nothing matches.",
                self.session.missing_readings_note(),
                cx,
            ))
            .into_any_element()
    }
}

fn show_spinner(
    state: &Entity<CommandState>,
    window: Option<AnyWindowHandle>,
    is_loading: bool,
    cx: &mut AsyncApp,
) {
    if let Some(window) = window {
        window
            .update(cx, |_, window, cx| {
                state.update(cx, |state, cx| state.set_loading(is_loading, window, cx));
            })
            .ok();
    }
}

fn item(name: impl Into<SharedString>, detail: Option<String>, is_skipped: bool) -> CommandItem {
    let name = name.into();
    let detail: Option<SharedString> = detail.map(Into::into);
    let label: SharedString = if is_skipped {
        format!("{name}, skipped").into()
    } else {
        name.clone()
    };
    CommandItem::new().label(label).child(move |_, cx| {
        h_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .when(is_skipped, |this| this.opacity(ui::SKIPPED_OPACITY))
            .child(
                div()
                    .min_w_0()
                    .flex_shrink_0()
                    .max_w_1_2()
                    .truncate()
                    .child(name.clone()),
            )
            .children(detail.clone().map(|detail| {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(detail)
            }))
    })
}

fn item_of(hit: &Hit, cx: &App) -> CommandItem {
    match hit {
        Hit::Artist(artist) => item(artist.name.clone(), None, false),
        Hit::Album(album) => item(
            album.name().to_owned(),
            album.artist().map(str::to_owned),
            false,
        ),
        Hit::Song(song) => {
            let detail = [song.artist(), song.album()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            item(
                song.title().to_owned(),
                Some(detail),
                actions::is_skipped(song, cx),
            )
        }
    }
}

impl Render for Search {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let on_query = cx.entity().downgrade();
        let on_confirm = cx.entity().downgrade();
        let empty = cx.entity().downgrade();
        let groups = self.results.sections.iter().map(|(label, hits)| {
            CommandGroup::new()
                .label(*label)
                .items(hits.iter().map(|hit| item_of(hit, cx)))
        });
        let mut palette = Command::new(&self.state)
            .bordered(false)
            .filterable(false)
            .placeholder("Search artists, albums and songs")
            .max_h(rems(MAX_HEIGHT))
            .on_query(move |query, _, cx| {
                on_query.update(cx, |this, cx| this.search(query, cx)).ok();
            })
            .on_confirm(move |path, window, cx| {
                on_confirm
                    .update(cx, |this, cx| this.confirm(path, window, cx))
                    .ok();
            })
            .empty(move |_, _, cx| {
                empty.upgrade().map_or_else(
                    || div().into_any_element(),
                    |this| this.read(cx).render_empty(cx),
                )
            });
        for group in groups {
            palette = palette.group(group);
        }
        v_flex().w_full().font_normal().child(palette)
    }
}
