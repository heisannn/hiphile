//! The page of the artist on show: their picture and what plays them, their top songs, their
//! albums, and the artists like them. It takes the focus as a whole, and the keys move a selection
//! through the songs and the cards.

use std::rc::Rc;
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    ButtonStyle, MenuItem, indeterminate_circular_progress, styled_button,
};
use hiphile_core::Shelf;
use hiphile_core::artists::{ArtistPage, ArtistSection, album_count_to_show};
use hiphile_core::favorites::Favorite;
use hiphile_core::navigation::Location;
use next_ui::card_grid::{CARD_GAP, Card, card, card_row};
use next_ui::context_menu::{more_button, pressed_at};
use next_ui::cover::{CoverKind, cover};
use next_ui::detail_header::{DetailHeader, detail_header, play_and_shuffle};
use next_ui::details::{format_album_count, format_song_count, join_details};
use next_ui::favorite_button::favorite_button;
use next_ui::keymap::{KeyContext, key_context};
use next_ui::link::{album_link, artist_line, song_artist_link};
use next_ui::load_state::{load_failure, pane_loading};
use next_ui::now_playing::{skippable, song_row_label};
use next_ui::page_row::{PageRow, page_row};
use next_ui::pane::INSET;
use next_ui::row_cover::row_cover;
use next_ui::row_list::is_context_click;
use next_ui::section::{section_failure, section_heading, section_loading, section_note};
use next_ui::song_row::song_with_line;
use next_ui::{AlbumAction, Command};
use subsonic::{Album, Artist, ArtistId, CoverArtId, Song};

use super::{DETAIL_CONTEXT, Item, Part, View};

/// What the covers of the header and the cards are asked for at: the same as the Albums screen's,
/// so that opening an album needs no second image.
const COVER_PIXELS: u32 = 384;
/// About a row of cards past either end of the page in view, which are there before they show.
const OVERDRAW: f32 = 300.0;
/// Below an album's cover: the space, the title, the year and the space at the foot.
const ALBUM_WORDS: f32 = 8.0 + 20.0 + 20.0 + 12.0;
/// What a text button has between its edge and its label, which brings it closer to the edge for
/// its label to line up.
const TEXT_BUTTON_PADDING: f32 = 12.0;
const PROGRESS: f32 = 24.0;

pub(super) fn page_pane(view: &Rc<View>) -> Div {
    let context = {
        let (enabled, run) = (view.clone(), view.clone());
        KeyContext::new(
            DETAIL_CONTEXT,
            move |command| enabled.is_enabled_on_page(*command),
            move |command| run.run_on_page(command),
        )
    };
    let layout = view.layout.clone();
    let (width, height, scrolled) = (layout.width, layout.height, layout.scrolled);
    let focused = view.page_focused;
    let page = div()
        .flex()
        .flex_col()
        .size_full()
        .overflow_y_scroll()
        .focusable()
        .node_ref(view.page_node)
        .access(|a| a.role(Role::ListBox).label("Artist"))
        .on_focus_change(move |on| focused.set(on))
        .on_scrolled(move |at| scrolled.set_if_changed(at.y))
        .on_resized(move |w, h| {
            width.set_if_changed(w);
            height.set_if_changed(h);
        })
        .child(content(view.clone()));
    key_context(page, &context)
}

/// What the page shows of the artist.
#[derive(Clone, Debug, PartialEq)]
enum Shape {
    Nothing,
    /// An artist the list does not have, whom the server has yet to name.
    Waiting,
    Failed(String),
    Named(ArtistId),
}

fn content(view: Rc<View>) -> impl IntoView {
    let page = view.page;
    let shape = memo(move || {
        page.with(|page| match page {
            None => Shape::Nothing,
            Some(page) if page.name.is_some() => Shape::Named(page.id.clone()),
            Some(page) => match &page.albums {
                Shelf::Failed(error) => Shape::Failed(error.clone()),
                Shelf::Loading | Shelf::Loaded(_) => Shape::Waiting,
            },
        })
    });
    dyn_view(move || {
        let shape = shape.get();
        let view = view.clone();
        untrack(move || {
            match shape {
                Shape::Nothing => div(),
                Shape::Waiting => pane_loading("Loading the artist"),
                Shape::Failed(reason) => {
                    let retrying = view.clone();
                    load_failure("Couldn't load this artist", reason, move || {
                        retrying.page_node.focus();
                        retrying.detail.reload();
                    })
                }
                Shape::Named(_) => artist_page(&view),
            }
            .flex_1()
            .into_view()
        })
    })
}

fn artist_page(view: &Rc<View>) -> Div {
    let grid_top = view.layout.grid_top;
    let albums = albums_state(view.page);
    div()
        .flex()
        .flex_col()
        .flex_none()
        .w_full()
        .pb(INSET)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .on_resized(move |_, h| grid_top.set_if_changed(h))
                .child(header(view))
                .child(songs(view))
                .child(albums_heading(view, albums)),
        )
        .child(album_grid(view, albums))
        .child(similar(view))
}

/// What the header shows, which it is made again for.
#[derive(Clone, Debug, PartialEq)]
struct Head {
    id: ArtistId,
    name: String,
    picture: Option<CoverArtId>,
    summary: String,
    has_songs: bool,
}

fn header(view: &Rc<View>) -> Div {
    let page = view.page;
    let head = memo(move || {
        page.with(|page| {
            let page = page.as_ref()?;
            Some(Head {
                id: page.id.clone(),
                name: page.name.clone()?,
                picture: page.picture().cloned(),
                summary: join_details([
                    page.album_count().map(format_album_count),
                    page.song_count().map(format_song_count),
                ]),
                has_songs: page.has_songs(),
            })
        })
    });
    let view = view.clone();
    div().flex().flex_col().flex_none().child(dyn_view(move || {
        let head = head.get();
        let view = view.clone();
        untrack(move || match head {
            None => div().into_view(),
            Some(head) => header_of(&view, head).into_view(),
        })
    }))
}

fn header_of(view: &Rc<View>, head: Head) -> Div {
    let Head {
        id,
        name,
        picture,
        summary,
        has_songs,
    } = head;
    let session = view.cx.workspace.session();
    let url = picture.map(|picture| session.cover_art_url(&picture, COVER_PIXELS));
    let fetching = view.fetching;
    let is_fetching = {
        let id = id.clone();
        move || fetching.with(|fetching| fetching.as_ref() == Some(&id))
    };
    let buttons = {
        let (playing, shuffling) = (view.clone(), view.clone());
        let (play, shuffle) = (id.clone(), id.clone());
        let is_fetching = is_fetching.clone();
        play_and_shuffle(
            move || playing.act_with(play.clone(), AlbumAction::Play),
            move || shuffling.shuffle(shuffle.clone()),
            move || !has_songs || is_fetching(),
        )
    };
    let mut header = DetailHeader::new(cover(url, CoverKind::Artist), name, summary)
        .artist_picture()
        .action(buttons);
    if let Some(artist) = view.whole_artist(&id) {
        header = header.action(favorite_button(
            Favorite::Artist(artist),
            &view.cx.commands,
            view.cx.known,
        ));
    }
    let more = {
        let view = view.clone();
        more_button(view.menu, move || {
            let off = |item: MenuItem| item.disabled_when(move || !has_songs);
            vec![
                view.enqueue_items(&id).into_iter().map(off).collect(),
                vec![off(view.add_artist_to_playlist_item(&id))],
            ]
        })
    };
    let waiting = show(is_fetching, || {
        indeterminate_circular_progress()
            .size(PROGRESS)
            .access(|a| a.role(Role::ProgressBar).label("Loading the songs"))
    });
    detail_header(
        header
            .action(more)
            .action(div().flex().flex_none().child(waiting)),
    )
}

/// How a part of the page that loads stands.
#[derive(Clone, Debug, PartialEq)]
enum Load {
    Loading,
    Failed(String),
    Loaded,
}

fn load_of<T>(shelf: &Shelf<T>) -> Load {
    match shelf {
        Shelf::Loading => Load::Loading,
        Shelf::Failed(error) => Load::Failed(error.clone()),
        Shelf::Loaded(_) => Load::Loaded,
    }
}

fn songs(view: &Rc<View>) -> Div {
    let page = view.page;
    let section = memo(move || {
        page.with(|page| {
            let sections = page.as_ref()?.sections();
            sections
                .into_iter()
                .find(|section| matches!(section, ArtistSection::TopSongs | ArtistSection::Songs))
        })
    });
    let view = view.clone();
    div().flex().flex_col().flex_none().child(show(
        move || section.get().is_some(),
        move || songs_part(&view, section),
    ))
}

fn songs_part(view: &Rc<View>, section: Memo<Option<ArtistSection>>) -> Div {
    let page = view.page;
    let heading = view.layout.heading(Part::Songs);
    let expanded = view.top_songs_expanded;
    let title = memo(move || match section.get() {
        Some(ArtistSection::Songs) => "Songs",
        _ => "Top songs",
    });
    let load = memo(move || {
        page.with(|page| {
            page.as_ref()
                .map_or(Load::Loading, |page| load_of(&page.top_songs))
        })
    });
    let on_show = memo(move || {
        page.with(|page| {
            page.as_ref()
                .map(|page| page.top_songs_on_show(expanded.get()).to_vec())
                .unwrap_or_default()
        })
    });
    let has_more =
        memo(move || page.with(|page| page.as_ref().is_some_and(ArtistPage::has_more_top_songs)));
    let detail = view.detail.clone();
    let view = view.clone();
    div()
        .flex()
        .flex_col()
        .flex_none()
        .child(dyn_view(move || {
            let title = title.get();
            untrack(|| section_heading(title).node_ref(heading).into_view())
        }))
        .child(dyn_view(move || {
            let (view, detail) = (view.clone(), detail.clone());
            match load.get() {
                Load::Loading => untrack(|| section_loading("Loading top songs").into_view()),
                Load::Failed(reason) => untrack(|| {
                    section_failure(reason, move || detail.reload_top_songs()).into_view()
                }),
                Load::Loaded => untrack(move || song_rows(&view, on_show).into_view()),
            }
        }))
        .child(show(
            move || has_more.get() && load.get() == Load::Loaded,
            move || see_more(expanded),
        ))
}

fn song_rows(view: &Rc<View>, on_show: Memo<Vec<Song>>) -> Div {
    let view = view.clone();
    div().flex().flex_col().flex_none().child(for_each(
        move || on_show.get().into_iter().enumerate().collect::<Vec<_>>(),
        |(position, song)| (*position, song.id().clone()),
        move |(position, song)| song_row(&view, position, song),
    ))
}

fn song_row(view: &Rc<View>, position: usize, song: Song) -> Div {
    let item = Item::Song(song.id().clone());
    let node = node_ref();
    view.layout.keep(item.clone(), node);
    let commands = &view.cx.commands;
    // The artist on show goes without saying
    let by_another = view
        .page_on_show()
        .is_none_or(|page| page.is_by_another_artist(&song));
    let artist = song_artist_link(&song, commands).filter(|_| by_another);
    let album = song
        .album()
        .map(|name| album_link(name, song.album_id().cloned(), commands));
    let (now_playing, known) = (view.now_playing, view.cx.known);
    let state = {
        let id = song.id().clone();
        move || now_playing.with(|now| now.as_ref()?.state_of(&id))
    };
    let cover = row_cover(&view.cx.workspace, song.cover_art(), CoverKind::Album);
    let content = skippable(
        song_with_line(&song, cover, artist_line(artist, album), state),
        {
            let song = song.clone();
            move || known.is_skipped(&song)
        },
    );
    let label = {
        let song = song.clone();
        move || {
            let state = now_playing.with(|now| now.as_ref()?.state_of(song.id()));
            song_row_label(song.title(), state, known.is_skipped(&song))
        }
    };
    let (selected, focused) = (is_selected(view, &item), view.page_focused);
    let (clicked, pressed) = (view.clone(), view.clone());
    let clicked_item = item.clone();
    page_row(
        PageRow::new(content)
            .label(label)
            .selected_when(selected)
            .focused_when(move || focused.get()),
    )
    .node_ref(node)
    .on_click(move |e| {
        if is_context_click(e) {
            return;
        }
        clicked.select_item(clicked_item.clone());
        if e.click_count >= 2
            && let Some(play) = clicked.play_top_songs_from(position)
        {
            clicked.send(play);
        }
    })
    .on_pointer_down(move |e| {
        if is_context_click(e) {
            pressed.select_item(item.clone());
            let groups = pressed.song_menu(&song);
            pressed.menu.open_at(pressed_at(e), groups);
        }
    })
}

fn see_more(expanded: Signal<bool>) -> Div {
    let label = move || {
        if expanded.get() {
            "Show less".to_owned()
        } else {
            "See more".to_owned()
        }
    };
    div()
        .flex()
        .flex_row()
        .flex_none()
        .px(INSET - TEXT_BUTTON_PADDING)
        .pt(4.0_f32)
        .child(
            styled_button(label, ButtonStyle::text())
                .on_click(move |_| expanded.update(|expanded| *expanded = !*expanded)),
        )
}

/// How the albums stand, `None` when the page has no place for them.
#[derive(Clone, Debug, PartialEq)]
enum Albums {
    Absent,
    Loading,
    Failed(String),
    Empty,
    Loaded(Arc<[Album]>),
}

fn albums_state(page: Signal<Option<ArtistPage>>) -> Memo<Albums> {
    memo(move || {
        page.with(|page| {
            let Some(page) = page else {
                return Albums::Absent;
            };
            if !page.sections().contains(&ArtistSection::Albums) {
                return Albums::Absent;
            }
            match &page.albums {
                Shelf::Loading => Albums::Loading,
                Shelf::Failed(error) => Albums::Failed(error.clone()),
                Shelf::Loaded(albums) if albums.is_empty() => Albums::Empty,
                Shelf::Loaded(albums) => Albums::Loaded(albums.clone()),
            }
        })
    })
}

/// The heading of the albums, and what stands in for them while there are none to show.
fn albums_heading(view: &Rc<View>, albums: Memo<Albums>) -> Div {
    let load = memo(move || match albums.get() {
        Albums::Absent => None,
        Albums::Loading => Some(Load::Loading),
        Albums::Failed(error) => Some(Load::Failed(error)),
        Albums::Empty | Albums::Loaded(_) => Some(Load::Loaded),
    });
    let is_empty = memo(move || albums.get() == Albums::Empty);
    let heading = view.layout.heading(Part::Albums);
    let detail = view.detail.clone();
    div().flex().flex_col().flex_none().child(dyn_view(move || {
        let load = load.get();
        let detail = detail.clone();
        untrack(move || {
            let Some(load) = load else {
                return div().into_view();
            };
            let status = match load {
                Load::Loading => section_loading("Loading albums"),
                Load::Failed(reason) => section_failure(reason, move || detail.reload_albums()),
                Load::Loaded => div().flex().flex_col().child(show(
                    move || is_empty.get(),
                    || section_note("This artist has no albums."),
                )),
            };
            div()
                .flex()
                .flex_col()
                .flex_none()
                .child(section_heading("Albums").node_ref(heading))
                .child(status)
                .into_view()
        })
    }))
}

/// The cards of the albums, of which only the rows near the page in view are made: an artist can
/// have hundreds of albums, whose covers should not all load.
fn album_grid(view: &Rc<View>, albums: Memo<Albums>) -> Div {
    let loaded = memo(move || match albums.get() {
        Albums::Loaded(albums) => Some(albums),
        _ => None,
    });
    let view = view.clone();
    div().flex().flex_col().flex_none().child(dyn_view(move || {
        let loaded = loaded.get();
        let view = view.clone();
        untrack(move || match loaded {
            Some(albums) => grid(&view, albums).into_view(),
            None => div().into_view(),
        })
    }))
}

/// How high a row of album cards is reckoned to be in a page `width` wide, before one is laid out.
pub(super) fn reckoned_row(width: f32, columns: usize) -> f32 {
    let columns = columns.max(1) as f32;
    let room = width - INSET * 2.0 - (columns - 1.0) * CARD_GAP;
    (room / columns).max(0.0) + ALBUM_WORDS + CARD_GAP
}

fn grid(view: &Rc<View>, albums: Arc<[Album]>) -> Div {
    let layout = view.layout.clone();
    let (width, height, scrolled, grid_top, columns) = (
        layout.width,
        layout.height,
        layout.scrolled,
        layout.grid_top,
        layout.columns,
    );
    // The tallest row laid out at the width it was laid out at
    let measured = signal(None::<(f32, f32)>);
    let row_height = memo(move || {
        let width = width.get();
        match measured.get() {
            Some((at, height)) if at == width => height,
            _ => reckoned_row(width, columns.get()),
        }
        .max(1.0)
    });
    let album_row = layout.album_row;
    effect(move || album_row.set(row_height.get()));
    let len = albums.len();
    let rows = memo(move || len.div_ceil(columns.get().max(1)));
    let near = memo(move || {
        let row = row_height.get();
        let top = scrolled.get() - grid_top.get();
        let first = ((top - OVERDRAW) / row).floor().max(0.0) as usize;
        let last = ((top + height.get() + OVERDRAW) / row).ceil().max(0.0) as usize;
        let last = last.min(rows.get());
        (first.min(last), last)
    });
    let view = view.clone();
    div()
        .relative()
        .w_full()
        .flex_none()
        .dyn_style(move |s| s.h(rows.get() as f32 * row_height.get()))
        .child(for_each(
            move || {
                let (first, last) = near.get();
                let columns = columns.get().max(1);
                (first..last).map(|row| (row, columns)).collect::<Vec<_>>()
            },
            |row| *row,
            move |(row, columns)| {
                let start = row * columns;
                let end = (start + columns).min(albums.len());
                let cards = albums[start..end]
                    .iter()
                    .map(|album| album_card(&view, album))
                    .collect();
                div()
                    .absolute()
                    .left(0)
                    .right(0)
                    .dyn_style(move |s| {
                        let height = row_height.get();
                        s.top(row as f32 * height).h(height)
                    })
                    .child(card_row(columns, cards).on_resized(move |_, h| {
                        let at = width.get_untracked();
                        measured.update(|measured| match measured {
                            Some((width, tallest)) if *width == at && *tallest >= h => {}
                            _ => *measured = Some((at, h)),
                        });
                    }))
            },
        ))
}

fn album_card(view: &Rc<View>, album: &Album) -> Div {
    let session = view.cx.workspace.session();
    let url = album
        .cover_art()
        .map(|cover_art| session.cover_art_url(cover_art, COVER_PIXELS));
    let item = Item::Album(album.id().clone());
    let mut shown = Card::new(cover(url, CoverKind::Album), album.name());
    if let Some(year) = album.year() {
        shown = shown.details(year.to_string());
    }
    let to = Command::Show(Location::Albums(Some(album.id().clone())));
    let (menu, album) = (view.clone(), album.clone());
    on_card(view, shown, item, to, move || menu.album_menu(&album))
}

fn similar(view: &Rc<View>) -> Div {
    let page = view.page;
    let state = memo(move || {
        page.with(|page| {
            let page = page.as_ref()?;
            if !page.sections().contains(&ArtistSection::Similar) {
                return None;
            }
            Some(match &page.similar {
                Shelf::Loaded(artists) => Ok(artists.clone()),
                shelf => Err(load_of(shelf)),
            })
        })
    });
    let heading = view.layout.heading(Part::Similar);
    let view = view.clone();
    div().flex().flex_col().flex_none().child(dyn_view(move || {
        let state = state.get();
        let view = view.clone();
        untrack(move || {
            let Some(state) = state else {
                return div().into_view();
            };
            let detail = view.detail.clone();
            let shown = match state {
                Ok(artists) => similar_cards(&view, artists),
                Err(Load::Failed(reason)) => {
                    section_failure(reason, move || detail.reload_similar())
                }
                Err(Load::Loading | Load::Loaded) => section_loading("Loading similar artists"),
            };
            div()
                .flex()
                .flex_col()
                .flex_none()
                .child(section_heading("Similar artists").node_ref(heading))
                .child(shown)
                .into_view()
        })
    }))
}

fn similar_cards(view: &Rc<View>, artists: Arc<[Artist]>) -> Div {
    let columns = view.layout.columns;
    let view = view.clone();
    div().flex().flex_col().flex_none().child(dyn_view(move || {
        let columns = columns.get().max(1);
        let (view, artists) = (view.clone(), artists.clone());
        untrack(move || {
            div()
                .flex()
                .flex_col()
                .flex_none()
                .children(
                    artists
                        .chunks(columns)
                        .map(|row| {
                            let cards = row.iter().map(|artist| similar_card(&view, artist));
                            card_row(columns, cards.collect())
                        })
                        .collect::<Vec<_>>(),
                )
                .into_view()
        })
    }))
}

fn similar_card(view: &Rc<View>, artist: &Artist) -> Div {
    let session = view.cx.workspace.session();
    let url = artist
        .cover_art()
        .map(|cover_art| session.cover_art_url(cover_art, COVER_PIXELS));
    let item = Item::Artist(artist.id().clone());
    let node = node_ref();
    view.layout.keep(item.clone(), node);
    let mut shown = Card::new(cover(url, CoverKind::Artist), artist.name()).artist_picture();
    if let Some(count) = album_count_to_show(artist) {
        shown = shown.details(format_album_count(count as usize));
    }
    let to = Command::Show(Location::Artists(Some(artist.id().clone())));
    let (menu, artist) = (view.clone(), artist.clone());
    on_card(view, shown, item, to, move || {
        menu.similar_artist_menu(&artist)
    })
    .node_ref(node)
}

/// `shown` as a card of the page: selected while `item` is, clicking it selects it and sends `to`,
/// and the secondary button selects it and opens what `menu` makes.
fn on_card(
    view: &Rc<View>,
    shown: Card,
    item: Item,
    to: Command,
    menu: impl Fn() -> Vec<Vec<MenuItem>> + 'static,
) -> Div {
    let focused = view.page_focused;
    let shown = shown
        .selected_when(is_selected(view, &item))
        .focused_when(move || focused.get());
    let (clicked, pressed) = (view.clone(), view.clone());
    let clicked_item = item.clone();
    card(shown)
        .on_click(move |e| {
            if !is_context_click(e) {
                clicked.select_item(clicked_item.clone());
                clicked.send(to.clone());
            }
        })
        .on_pointer_down(move |e| {
            if is_context_click(e) {
                pressed.select_item(item.clone());
                pressed.menu.open_at(pressed_at(e), menu());
            }
        })
}

fn is_selected(view: &Rc<View>, item: &Item) -> impl Fn() -> bool + 'static {
    let (selected, item) = (view.item, item.clone());
    move || selected.with(|selected| selected.as_ref() == Some(&item))
}
