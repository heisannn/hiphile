//! OpenSubsonic protocol.
//!
//! This crate builds request URLs and parses responses; it never performs IO.
//! The caller executes the request with any HTTP stack, which keeps the
//! protocol usable from the GUI, the CLI, and tests alike.

mod album;
mod auth;
mod endpoint;
mod error;
mod id;
mod response;
mod song;

pub use album::{Album, AlbumWithSongs};
pub use auth::Credentials;
pub use endpoint::{CLIENT_NAME, Endpoint, PROTOCOL_VERSION};
pub use error::{Error, ServerErrorCode};
pub use id::{AlbumId, ArtistId, CoverArtId, PlaylistId, SongId};
pub use response::{Envelope, GetAlbum, GetAlbumList2, GetSong, Ping, Search3, ServerInfo, parse};
pub use song::Song;
