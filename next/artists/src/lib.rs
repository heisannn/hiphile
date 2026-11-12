//! The artists on AntiWebUI: the list `hiphile_core::artists::ArtistList` keeps, and the page of
//! the artist chosen in it, which `hiphile_core::artists::ArtistDetail` keeps.

mod artists_view;

pub use artists_view::{ARTISTS_CONTEXT, ArtistsCommand, DETAIL_CONTEXT, artists};
