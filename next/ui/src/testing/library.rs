//! A saved library, as a sync would have left it, for the views that read what is saved.

use serde_json::json;
use subsonic::{Album, Artist, ScanStatus, Song};

use super::{TestCore, core};

/// What a sync would have saved.
#[derive(Clone, Debug, Default)]
pub struct SavedLibrary {
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub songs: Vec<Song>,
}

impl catalog::Source for SavedLibrary {
    async fn albums_page(&self, _: u32, _: u32) -> Result<Vec<Album>, client::Error> {
        Ok(self.albums.clone())
    }

    async fn artists(&self) -> Result<Vec<Artist>, client::Error> {
        Ok(self.artists.clone())
    }

    async fn songs_page(&self, _: u32, _: u32) -> Result<Vec<Song>, client::Error> {
        Ok(self.songs.clone())
    }

    async fn scan_status(&self) -> Result<ScanStatus, client::Error> {
        Ok(serde_json::from_value(json!({ "scanning": false })).unwrap())
    }

    async fn library_modified(&self) -> Result<Option<u64>, client::Error> {
        Ok(None)
    }
}

/// A core signed in as alice on `server`, as [`super::signed_in_core`] makes, whose saved library
/// holds `library`, in the test's own folder. Saving into the catalog it gives stands for another
/// sync, which the workspace does not hear of by itself.
pub fn signed_in_core_with_library(
    server: &str,
    library: &SavedLibrary,
) -> (TestCore, hiphile_core::Hiphile, catalog::Catalog) {
    let (mut core, hiphile) = core("");
    let catalog = catalog::Catalog::open(core._dir.path().join("library")).unwrap();
    save_library(&catalog, library);
    let session = hiphile_core::test_support::session_saving_into(Some(catalog.clone()), server);
    core.nets.push(session.client().net().clone());
    hiphile.open_workspace(session, config::Profile::new(server, "alice"));
    (core, hiphile, catalog)
}

/// Puts `library` in `catalog` in place of what it held.
pub fn save_library(catalog: &catalog::Catalog, library: &SavedLibrary) {
    let mode = catalog::Mode::Always;
    futures::executor::block_on(async {
        catalog::sync_albums(catalog, library, mode).await.unwrap();
        catalog::sync_artists(catalog, library, mode).await.unwrap();
        catalog::sync_songs(catalog, library, mode).await.unwrap();
    });
}
