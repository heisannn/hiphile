//! OpenSubsonic protocol.
//!
//! This crate builds request URLs and parses responses; it never performs IO.
//! The caller executes the request with any HTTP stack, which keeps the
//! protocol usable from the GUI, the CLI, and tests alike.

mod album;
mod artist;
mod auth;
mod endpoint;
mod error;
mod id;
mod playlist;
mod response;
mod song;

pub use album::{Album, AlbumWithSongs};
pub use artist::{Artist, ArtistWithAlbums};
pub use auth::Credentials;
pub use endpoint::{CLIENT_NAME, CoverArtRequest, Endpoint, PROTOCOL_VERSION};
pub use error::{Error, ServerErrorCode};
pub use id::{AlbumId, ArtistId, CoverArtId, PlaylistId, SongId};
pub use playlist::{Playlist, PlaylistWithSongs};
pub use response::{
    Envelope, GetAlbum, GetAlbumList2, GetArtist, GetArtistInfo2, GetArtists, GetIndexes,
    GetPlaylist, GetPlaylists, GetScanStatus, GetSimilarSongs2, GetSong, GetTopSongs, Ping,
    ScanStatus, Search3, ServerInfo, parse,
};
pub use song::Song;
