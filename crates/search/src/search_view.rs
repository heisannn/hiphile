use gpui_kit::component::{
    ActiveTheme as _, IndexPath, StyledExt as _, WindowExt as _,
    command::{Command, CommandGroup, CommandItem, CommandState},
    h_flex, v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Task, Window, div, prelude::FluentBuilder as _, rems,
};
use hiphile_core::search::{Hit, SearchIndex, SearchStatus};
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

pub struct Search {
    session: session::Session,
    index: SearchIndex,
    /// What the index showed last.
    status: SearchStatus,
    state: Entity<CommandState>,
    /// So an empty palette does not say that nothing matched.
    has_query: bool,
    _index_changes: Task<()>,
}

impl EventEmitter<SearchEvent> for Search {}

impl Search {
    /// `window` is where the palette opens.
    pub fn new(
        session: session::Session,
        index: SearchIndex,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let index_changes =
            gpui_host::observe_in(index.subscribe(), window, cx, Self::follow_index);
        Self {
            session,
            status: index.status(),
            index,
            state: cx.new(|cx| CommandState::new(window, cx)),
            has_query: false,
            _index_changes: index_changes,
        }
    }

    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.index.open();
        self.state
            .update(cx, |state, cx| state.set_query("", window, cx));
        self.search("", cx);
        // At once, so that it does not open on what it found when last open
        self.follow_index(self.index.status(), window, cx);
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

    /// A first reading shows in the palette as a spinner: there is nothing to search meanwhile.
    fn follow_index(&mut self, status: SearchStatus, window: &mut Window, cx: &mut Context<Self>) {
        if status.is_waiting() != self.state.read(cx).is_loading() {
            self.state.update(cx, |state, cx| {
                state.set_loading(status.is_waiting(), window, cx);
            });
        }
        self.status = status;
        cx.notify();
    }

    fn search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.has_query = !query.trim().is_empty();
        self.index.search(query);
        cx.notify();
    }

    fn confirm(&mut self, path: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.status.results().hit(path.section, path.row).cloned() else {
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
        if let Some(error) = self.status.failure() {
            return div()
                .p_4()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(SharedString::from(error.to_owned()))
                .into_any_element();
        }
        if !self.has_query || self.status.is_waiting() {
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
        let groups = self.status.results().sections.iter().map(|(label, hits)| {
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
