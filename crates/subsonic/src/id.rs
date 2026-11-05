use std::sync::Arc;

// IDs are server-assigned and opaque, so never parse them
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
    /// A "child" in the Subsonic API.
    SongId
);
define_id!(AlbumId);
define_id!(ArtistId);
define_id!(PlaylistId);
define_id!(CoverArtId);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StarTarget {
    Song(SongId),
    Album(AlbumId),
    Artist(ArtistId),
}
