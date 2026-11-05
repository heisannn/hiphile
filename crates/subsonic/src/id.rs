use std::sync::Arc;

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
        #[serde(transparent)]
        pub struct $name(Arc<str>);

        impl $name {
            pub fn new(id: impl Into<Arc<str>>) -> Self {
                Self(id.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

define_id!(
    /// Server-assigned song (child) ID. Opaque; never parse it.
    SongId
);
define_id!(
    /// Server-assigned album ID. Opaque; never parse it.
    AlbumId
);
define_id!(
    /// Server-assigned artist ID. Opaque; never parse it.
    ArtistId
);
define_id!(
    /// Server-assigned playlist ID. Opaque; never parse it.
    PlaylistId
);
define_id!(
    /// Server-assigned cover art ID. Opaque; never parse it.
    CoverArtId
);

/// A song, an album or an artist, as something the user can mark as a favorite.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StarTarget {
    Song(SongId),
    Album(AlbumId),
    Artist(ArtistId),
}
