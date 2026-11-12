//! What shows below the bar: what the search found, kind by kind, or why there is nothing.

use std::rc::Rc;
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::Ink;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::search::{Hit, Results, SearchStatus};
use next_ui::ScreenContext;
use next_ui::cover::CoverKind;
use next_ui::details::join_details;
use next_ui::load_state::{centered, no_matches, pane_loading};
use next_ui::now_playing::{skippable, song_row_label};
use next_ui::pane::INSET;
use next_ui::row_cover::row_cover;
use next_ui::row_list::{ROW_HEIGHT, ROW_PADDING};
use next_ui::row_text::row_text;
use next_ui::selection::selection_fill;
use next_ui::type_scale::{error_text, muted_text};
use subsonic::{AlbumId, ArtistId, SongId};

use crate::search::Choose;

/// What the bar opens to at the least, less the bar and the line below it: md3e's sizes, which
/// what stands in for the results fills so that it stands in the middle.
const ROOM: f32 = 240.0 - 56.0 - 1.0;
/// From the edges to the rows, whose corners are rounded.
const EDGE: f32 = INSET - ROW_PADDING;
const ROW_CORNER: f32 = 12.0;
/// Above the first heading and below the last row.
const PADDING: f32 = 8.0;
const HEADING_HEIGHT: f32 = 36.0;

/// What the search has to show for what is typed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Found {
    /// Nothing is typed.
    Nothing,
    /// The library is read for the first time, and there is nothing to search meanwhile.
    Reading,
    Failed(String),
    NoMatches,
    Hits(Arc<Results>),
}

impl Found {
    pub(crate) fn new(status: &SearchStatus, query: &str) -> Self {
        if let Some(error) = status.failure() {
            return Self::Failed(error.to_owned());
        }
        if status.is_waiting() {
            return Self::Reading;
        }
        if query.trim().is_empty() {
            return Self::Nothing;
        }
        let results = status.results();
        if results.sections.is_empty() {
            Self::NoMatches
        } else {
            Self::Hits(results.clone())
        }
    }

    fn kind(&self) -> Kind {
        match self {
            Self::Nothing => Kind::Nothing,
            Self::Reading => Kind::Reading,
            Self::Failed(error) => Kind::Failed(error.clone()),
            Self::NoMatches => Kind::NoMatches,
            Self::Hits(_) => Kind::Hits,
        }
    }
}

/// [`Found`] without the hits, which change apart from it.
#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Nothing,
    Reading,
    Failed(String),
    NoMatches,
    Hits,
}

/// Which hit a row shows, the same in each new search that finds it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum HitKey {
    Artist(ArtistId),
    Album(AlbumId),
    Song(SongId),
}

impl HitKey {
    pub(crate) fn of(hit: &Hit) -> Self {
        match hit {
            Hit::Artist(artist) => Self::Artist(artist.id.clone()),
            Hit::Album(album) => Self::Album(album.id().clone()),
            Hit::Song(song) => Self::Song(song.id().clone()),
        }
    }
}

/// What `found` shows, with the row of `highlighted` highlighted. Choosing a row gives its hit
/// to `choose`.
pub(crate) fn results(
    found: Memo<Found>,
    highlighted: Memo<Option<HitKey>>,
    choose: Choose,
    cx: &ScreenContext,
) -> Div {
    let kind = memo(move || found.with(Found::kind));
    let results = memo(move || match found.get() {
        Found::Hits(results) => results,
        _ => Arc::default(),
    });
    let cx = cx.clone();
    div()
        .flex()
        .flex_col()
        .child(dyn_view(move || match kind.get() {
            Kind::Nothing => untrack(|| div().into_view()),
            Kind::Reading => untrack(|| standing_in(pane_loading("Loading Library")).into_view()),
            Kind::Failed(error) => untrack(|| standing_in(centered(error_text(error))).into_view()),
            Kind::NoMatches => untrack(|| {
                let note = cx.workspace.session().missing_readings_note();
                standing_in(no_matches("Nothing matches.", note)).into_view()
            }),
            Kind::Hits => {
                untrack(|| sections(results, highlighted, choose.clone(), &cx).into_view())
            }
        }))
}

/// `part` in the room there is below the bar while there is no result.
fn standing_in(part: Div) -> Div {
    div().flex().flex_col().min_h(ROOM).child(part)
}

fn sections(
    results: Memo<Arc<Results>>,
    highlighted: Memo<Option<HitKey>>,
    choose: Choose,
    cx: &ScreenContext,
) -> Div {
    let cx = cx.clone();
    div()
        .flex()
        .flex_col()
        .py(PADDING)
        .access(|a| a.role(Role::ListBox).label("Results"))
        .child(for_each(
            move || {
                results.with(|results| results.sections.iter().map(|(kind, _)| *kind).collect())
            },
            |kind: &&'static str| *kind,
            move |kind| section(kind, results, highlighted, choose.clone(), &cx),
        ))
}

/// The hits of one kind, under a heading that names it.
fn section(
    kind: &'static str,
    results: Memo<Arc<Results>>,
    highlighted: Memo<Option<HitKey>>,
    choose: Choose,
    cx: &ScreenContext,
) -> Div {
    let hits = memo(move || {
        results.with(|results| {
            let section = results.sections.iter().find(|(named, _)| *named == kind);
            section.map(|(_, hits)| hits.clone()).unwrap_or_default()
        })
    });
    let first = memo(move || hits.with(|hits| hits.first().map(HitKey::of)));
    let heading = node_ref();
    let cx = cx.clone();
    div()
        .flex()
        .flex_col()
        .child(
            muted_text(TextStyle::TITLE_SMALL, kind)
                .flex()
                .flex_none()
                .items_center()
                .h(HEADING_HEIGHT)
                .px(INSET)
                .node_ref(heading)
                .access(move |a| a.role(Role::Heading).level(2).label(kind)),
        )
        .child(for_each(
            move || hits.get(),
            HitKey::of,
            move |hit| {
                let place = Place {
                    highlighted,
                    first,
                    heading,
                };
                row(hit, place, choose.clone(), &cx)
            },
        ))
}

/// Where a row is among the others.
struct Place {
    highlighted: Memo<Option<HitKey>>,
    /// The first of its section, which shows the heading too when it comes into view.
    first: Memo<Option<HitKey>>,
    heading: NodeRef,
}

fn row(hit: Hit, place: Place, choose: Choose, cx: &ScreenContext) -> Div {
    let Place {
        highlighted,
        first,
        heading,
    } = place;
    let key = HitKey::of(&hit);
    let is_highlighted = {
        let key = key.clone();
        memo(move || highlighted.with(|on| on.as_ref() == Some(&key)))
    };
    let node = node_ref();
    effect(move || {
        if is_highlighted.get() {
            untrack(|| {
                if first.with(|first| first.as_ref() == Some(&key)) {
                    heading.scroll_into_view();
                }
                node.scroll_into_view();
            });
        }
    });
    let (content, label) = look(&hit, cx);
    let ink = Ink::new(|c| c.on_surface, || false);
    let frame = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .h(ROW_HEIGHT)
        .mx(EDGE)
        .px(ROW_PADDING)
        .rounded(ROW_CORNER)
        .node_ref(node)
        .access(move |a| {
            a.role(Role::ListOption)
                .selected(is_highlighted.get())
                .label(label())
        })
        .dyn_style(move |s| {
            let c = color_scheme();
            let s = s.text_color(c.on_surface);
            match selection_fill(&c, is_highlighted.get(), true) {
                Some(fill) => s.bg(fill),
                None => s,
            }
        })
        .on_click(move |_| choose(&hit))
        .child(content.flex_1().min_w(0));
    ink.paint(ink.listen(frame))
}

/// What a row shows of `hit`, and what it is called for assistive technology, which may change.
fn look(hit: &Hit, cx: &ScreenContext) -> (Div, Rc<dyn Fn() -> String>) {
    let cover_of =
        |cover: Option<&subsonic::CoverArtId>, kind| row_cover(&cx.workspace, cover, kind);
    let line = |cover: Div, title: &str, details: Option<String>| {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(12.0_f32)
            .child(cover)
            .child(row_text(title.to_owned(), details).flex_1())
    };
    match hit {
        Hit::Artist(artist) => {
            let name = artist.name.clone();
            let content = line(cover_of(None, CoverKind::Artist), &name, None);
            (content, Rc::new(move || name.clone()))
        }
        Hit::Album(album) => {
            let name = album.name().to_owned();
            let content = line(
                cover_of(album.cover_art(), CoverKind::Album),
                &name,
                album.artist().map(str::to_owned),
            );
            (content, Rc::new(move || name.clone()))
        }
        Hit::Song(song) => {
            let title = song.title().to_owned();
            let details = join_details([song.artist(), song.album()]);
            let content = line(
                cover_of(song.cover_art(), CoverKind::Album),
                &title,
                (!details.is_empty()).then_some(details),
            );
            let known = cx.known;
            let skipped = {
                let song = song.clone();
                move || known.is_skipped(&song)
            };
            let label = {
                let skipped = skipped.clone();
                move || song_row_label(&title, None, skipped())
            };
            (skippable(content, skipped), Rc::new(label))
        }
    }
}
