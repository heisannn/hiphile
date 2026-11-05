use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use subsonic::{Album, Artist, ScanStatus, Song};

use crate::store::Kind;
use crate::{Catalog, Error};

/// How old a sync may get before [`Mode::IfStale`] runs another, even when
/// the server reports no change. Ratings, play counts and servers that do
/// not report changes are caught up this way.
pub const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// Items per request. `getAlbumList2` sends at most 500.
const PAGE_SIZE: u32 = 500;

pub trait Source: Sync {
    /// One page of albums in the server's name order.
    fn albums_page(
        &self,
        size: u32,
        offset: u32,
    ) -> impl Future<Output = Result<Vec<Album>, client::Error>> + Send;

    /// Every artist, in the order to list them.
    fn artists(&self) -> impl Future<Output = Result<Vec<Artist>, client::Error>> + Send;

    /// One page of every song, in an order that holds between pages.
    fn songs_page(
        &self,
        size: u32,
        offset: u32,
    ) -> impl Future<Output = Result<Vec<Song>, client::Error>> + Send;

    fn scan_status(&self) -> impl Future<Output = Result<ScanStatus, client::Error>> + Send;

    /// When the library last changed, in milliseconds since the Unix epoch.
    fn library_modified(&self) -> impl Future<Output = Result<Option<u64>, client::Error>> + Send;
}

impl Source for client::Client {
    fn albums_page(
        &self,
        size: u32,
        offset: u32,
    ) -> impl Future<Output = Result<Vec<Album>, client::Error>> + Send {
        client::Client::albums_page(self, size, offset)
    }

    fn artists(&self) -> impl Future<Output = Result<Vec<Artist>, client::Error>> + Send {
        client::Client::artists(self)
    }

    fn songs_page(
        &self,
        size: u32,
        offset: u32,
    ) -> impl Future<Output = Result<Vec<Song>, client::Error>> + Send {
        client::Client::songs_page(self, size, offset)
    }

    fn scan_status(&self) -> impl Future<Output = Result<ScanStatus, client::Error>> + Send {
        client::Client::scan_status(self)
    }

    fn library_modified(&self) -> impl Future<Output = Result<Option<u64>, client::Error>> + Send {
        client::Client::library_modified(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Syncs only when the copy never completed a sync, the server reports
    /// a change since the last one, or it is older than [`STALE_AFTER`].
    IfStale,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The copy was recent and the server reported no change since.
    UpToDate,
    Synced {
        count: usize,
    },
    /// The library changed while the pages arrived, so they may have
    /// skipped items: the server scanned, or sent an item twice. What
    /// arrived was kept, nothing was removed, and the copy stays stale for
    /// the next sync.
    Unsettled {
        count: usize,
    },
    /// Another sync of this library is running, in this process or another.
    Busy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SyncProgress {
    pub saved: usize,
    /// How many there are likely to be in all. A guess: more may arrive.
    pub total: Option<usize>,
}

/// Each page is kept as it arrives, so a sync that fails partway leaves the
/// copy with what it had plus what arrived; only a complete sync removes
/// albums. Blocks on the disk between requests: run it where blocking is
/// allowed, such as GPUI's background executor or the CLI.
pub async fn sync_albums(
    catalog: &Catalog,
    source: &impl Source,
    mode: Mode,
) -> Result<Outcome, Error> {
    run(catalog, source, Kind::Albums, mode, &|_| {}).await
}

/// Says nothing when the albums are up to date.
pub async fn sync_albums_watched(
    catalog: &Catalog,
    source: &impl Source,
    mode: Mode,
    on_progress: impl Fn(SyncProgress) + Send + Sync,
) -> Result<Outcome, Error> {
    run(catalog, source, Kind::Albums, mode, &on_progress).await
}

pub async fn sync_artists(
    catalog: &Catalog,
    source: &impl Source,
    mode: Mode,
) -> Result<Outcome, Error> {
    run(catalog, source, Kind::Artists, mode, &|_| {}).await
}

pub async fn sync_artists_watched(
    catalog: &Catalog,
    source: &impl Source,
    mode: Mode,
    on_progress: impl Fn(SyncProgress) + Send + Sync,
) -> Result<Outcome, Error> {
    run(catalog, source, Kind::Artists, mode, &on_progress).await
}

/// Sync albums first: songs are listed under the albums known.
pub async fn sync_songs(
    catalog: &Catalog,
    source: &impl Source,
    mode: Mode,
) -> Result<Outcome, Error> {
    run(catalog, source, Kind::Songs, mode, &|_| {}).await
}

pub async fn sync_songs_watched(
    catalog: &Catalog,
    source: &impl Source,
    mode: Mode,
    on_progress: impl Fn(SyncProgress) + Send + Sync,
) -> Result<Outcome, Error> {
    run(catalog, source, Kind::Songs, mode, &on_progress).await
}

/// How many items are likely to arrive, for showing how far a sync has got:
/// the last sync's number, or the server's count of songs when that is more.
/// Artists arrive all at once, which leaves nothing to show.
fn expected_total(
    catalog: &Catalog,
    kind: Kind,
    before: Option<&ScanStatus>,
) -> Result<Option<usize>, Error> {
    let scanned = match kind {
        Kind::Artists => return Ok(None),
        Kind::Albums => 0,
        Kind::Songs => before
            .and_then(ScanStatus::count)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default(),
    };
    Ok(Some(catalog.count(kind)?.max(scanned)).filter(|total| *total > 0))
}

async fn run(
    catalog: &Catalog,
    source: &impl Source,
    kind: Kind,
    mode: Mode,
    on_progress: &(impl Fn(SyncProgress) + Sync),
) -> Result<Outcome, Error> {
    let Some(_lock) = catalog.try_lock_sync()? else {
        return Ok(Outcome::Busy);
    };
    let started = SystemTime::now();
    // Not every server reports it, and the age of the copy decides then
    let modified = source.library_modified().await.ok().flatten();
    if mode == Mode::IfStale && !catalog.is_stale(kind, modified, started, STALE_AFTER)? {
        return Ok(Outcome::UpToDate);
    }

    let before = source.scan_status().await.ok();
    let total = expected_total(catalog, kind, before.as_ref())?;
    let generation = catalog.begin(kind)?;
    let mut count = 0;
    on_progress(SyncProgress { saved: 0, total });
    // Pages that shift under the sync send one item twice where they skip
    // another, so an item seen again means one may be missing.
    let mut seen = HashSet::new();
    let mut repeated = false;
    loop {
        let offset = u32::try_from(count).unwrap_or(u32::MAX);
        let received = match kind {
            Kind::Albums => {
                let page = source.albums_page(PAGE_SIZE, offset).await?;
                catalog.store_albums(generation, offset, &page)?;
                for album in &page {
                    repeated |= !seen.insert(album.id().as_str().to_owned());
                }
                page.len()
            }
            // They all come at once, which leaves no next page to ask for
            Kind::Artists => {
                let artists = source.artists().await?;
                catalog.store_artists(generation, &artists)?;
                for artist in &artists {
                    repeated |= !seen.insert(artist.id().as_str().to_owned());
                }
                break;
            }
            Kind::Songs => {
                let page = source.songs_page(PAGE_SIZE, offset).await?;
                catalog.store_songs(generation, &page)?;
                for song in &page {
                    repeated |= !seen.insert(song.id().as_str().to_owned());
                }
                page.len()
            }
        };
        count += received;
        on_progress(SyncProgress {
            saved: count,
            total,
        });
        if received < PAGE_SIZE as usize {
            break;
        }
    }
    let after = source.scan_status().await.ok();

    let count = seen.len();
    if repeated || !settled(before.as_ref(), after.as_ref()) {
        return Ok(Outcome::Unsettled { count });
    }
    catalog.finish(kind, generation, modified, started)?;
    Ok(Outcome::Synced { count })
}

/// Whether the library held still while the pages arrived, as far as the
/// server tells. Offset pages of a library that changes underneath them
/// can skip items, which a sync must not then remove. A scan that started
/// and ended in between shows as a new last scan, or failing that, a new
/// count.
fn settled(before: Option<&ScanStatus>, after: Option<&ScanStatus>) -> bool {
    if before.into_iter().chain(after).any(ScanStatus::is_scanning) {
        return false;
    }
    match (before, after) {
        (Some(before), Some(after)) => {
            before.count() == after.count() && before.last_scan() == after.last_scan()
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures::executor::block_on;
    use serde_json::json;
    use subsonic::AlbumId;

    use super::*;

    #[derive(Default)]
    struct Fake {
        albums: Vec<Album>,
        artists: Vec<Artist>,
        artists_fail: bool,
        songs: Vec<Song>,
        modified: Option<u64>,
        scanning: bool,
        fail_at: Option<u32>,
        /// Reports a new last scan after the first status, as if a scan
        /// ran during the sync.
        rescans: bool,
        album_requests: Mutex<u32>,
        status_requests: Mutex<u32>,
    }

    impl Source for Fake {
        async fn albums_page(&self, size: u32, offset: u32) -> Result<Vec<Album>, client::Error> {
            *self.album_requests.lock().unwrap() += 1;
            if self.fail_at == Some(offset) {
                return Err(client::Error::HttpStatus(502));
            }
            Ok(page(&self.albums, size, offset))
        }

        async fn artists(&self) -> Result<Vec<Artist>, client::Error> {
            if self.artists_fail {
                return Err(client::Error::HttpStatus(502));
            }
            Ok(self.artists.clone())
        }

        async fn songs_page(&self, size: u32, offset: u32) -> Result<Vec<Song>, client::Error> {
            Ok(page(&self.songs, size, offset))
        }

        async fn scan_status(&self) -> Result<ScanStatus, client::Error> {
            let mut requests = self.status_requests.lock().unwrap();
            let scan = if self.rescans { *requests } else { 0 };
            *requests += 1;
            Ok(ScanStatus::new(
                self.scanning,
                Some(10),
                Some(format!("scan {scan}")),
            ))
        }

        async fn library_modified(&self) -> Result<Option<u64>, client::Error> {
            Ok(self.modified)
        }
    }

    fn page<T: Clone>(items: &[T], size: u32, offset: u32) -> Vec<T> {
        items
            .iter()
            .skip(offset as usize)
            .take(size as usize)
            .cloned()
            .collect()
    }

    fn album(id: &str, name: &str) -> Album {
        serde_json::from_value(json!({ "id": id, "name": name })).unwrap()
    }

    fn albums(count: usize) -> Vec<Album> {
        (0..count)
            .map(|ix| album(&format!("a{ix}"), &format!("Album {ix:04}")))
            .collect()
    }

    fn song(id: &str, album: &str, disc: u32, track: u32) -> Song {
        serde_json::from_value(json!({
            "id": id, "isDir": false, "title": id, "albumId": album,
            "discNumber": disc, "track": track,
        }))
        .unwrap()
    }

    fn ids(albums: &[Album]) -> Vec<&str> {
        albums.iter().map(|album| album.id().as_str()).collect()
    }

    fn song_ids(album: &subsonic::AlbumWithSongs) -> Vec<&str> {
        album
            .songs()
            .iter()
            .map(|song| song.id().as_str())
            .collect()
    }

    fn catalog() -> (tempfile::TempDir, Catalog) {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        (dir, catalog)
    }

    #[test]
    fn keeps_every_page_in_the_server_order() {
        let (_dir, catalog) = catalog();
        // Names out of order: the server's order wins over sorting by name
        let mut source = Fake {
            albums: albums(1203),
            ..Fake::default()
        };
        source.albums.swap(0, 1);
        let outcome = block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        assert_eq!(outcome, Outcome::Synced { count: 1203 });
        assert_eq!(*source.album_requests.lock().unwrap(), 3);
        let stored = catalog.albums().unwrap();
        assert_eq!(stored.len(), 1203);
        assert_eq!(ids(&stored[..3]), ["a1", "a0", "a2"]);
        assert_eq!(stored[1202].id().as_str(), "a1202");
    }

    fn artist(id: &str) -> Artist {
        serde_json::from_value(json!({ "id": id, "name": id })).unwrap()
    }

    fn artist_ids(catalog: &Catalog) -> Vec<String> {
        let artists = catalog.artists().unwrap();
        artists.iter().map(|it| it.id().to_string()).collect()
    }

    #[test]
    fn artists_are_kept_in_the_server_order_and_go_when_the_server_drops_them() {
        let (_dir, catalog) = catalog();
        let mut source = Fake {
            artists: vec![artist("z"), artist("a"), artist("m")],
            ..Fake::default()
        };
        let outcome = block_on(sync_artists(&catalog, &source, Mode::Always)).unwrap();
        assert_eq!(outcome, Outcome::Synced { count: 3 });
        assert_eq!(artist_ids(&catalog), ["z", "a", "m"]);

        source.artists.remove(1);
        block_on(sync_artists(&catalog, &source, Mode::Always)).unwrap();
        assert_eq!(artist_ids(&catalog), ["z", "m"]);
    }

    #[test]
    fn a_failed_sync_of_artists_keeps_them_and_stays_stale() {
        let (_dir, catalog) = catalog();
        let mut source = Fake {
            artists: vec![artist("a")],
            ..Fake::default()
        };
        block_on(sync_artists(&catalog, &source, Mode::Always)).unwrap();

        source.artists_fail = true;
        assert!(block_on(sync_artists(&catalog, &source, Mode::Always)).is_err());
        assert_eq!(artist_ids(&catalog), ["a"]);

        source.artists_fail = false;
        let outcome = block_on(sync_artists(&catalog, &source, Mode::IfStale)).unwrap();
        assert_eq!(outcome, Outcome::Synced { count: 1 });
    }

    fn watched(catalog: &Catalog, source: &Fake, mode: Mode) -> Vec<SyncProgress> {
        let told = Mutex::new(Vec::new());
        block_on(sync_albums_watched(catalog, source, mode, |progress| {
            told.lock().unwrap().push(progress);
        }))
        .unwrap();
        told.into_inner().unwrap()
    }

    fn saved(told: &[SyncProgress]) -> Vec<usize> {
        told.iter().map(|progress| progress.saved).collect()
    }

    #[test]
    fn a_sync_tells_what_it_saved_page_by_page() {
        let (_dir, catalog) = catalog();
        let source = Fake {
            albums: albums(1203),
            ..Fake::default()
        };
        let told = watched(&catalog, &source, Mode::Always);
        assert_eq!(saved(&told), [0, 500, 1000, 1203]);
        assert!(told.iter().all(|progress| progress.total.is_none()));
    }

    #[test]
    fn a_sync_expects_as_many_as_the_last_one_saved() {
        let (_dir, catalog) = catalog();
        let source = Fake {
            albums: albums(3),
            ..Fake::default()
        };
        block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();

        let told = watched(&catalog, &source, Mode::Always);

        assert_eq!(told[0].total, Some(3));
    }

    #[test]
    fn a_sync_expects_as_many_songs_as_the_server_counts() {
        let (_dir, catalog) = catalog();
        let told = Mutex::new(Vec::new());
        block_on(sync_songs_watched(
            &catalog,
            &Fake::default(),
            Mode::Always,
            |progress| told.lock().unwrap().push(progress),
        ))
        .unwrap();
        assert_eq!(told.into_inner().unwrap()[0].total, Some(10));
    }

    #[test]
    fn nothing_is_told_of_a_sync_with_nothing_to_fetch() {
        let (_dir, catalog) = catalog();
        let source = Fake::default();
        block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        assert!(watched(&catalog, &source, Mode::IfStale).is_empty());
    }

    #[test]
    fn artists_up_to_date_are_not_asked_for() {
        let (_dir, catalog) = catalog();
        let source = Fake::default();
        block_on(sync_artists(&catalog, &source, Mode::Always)).unwrap();
        let outcome = block_on(sync_artists(&catalog, &source, Mode::IfStale)).unwrap();
        assert_eq!(outcome, Outcome::UpToDate);
    }

    #[test]
    fn a_complete_sync_removes_what_the_server_no_longer_has() {
        let (_dir, catalog) = catalog();
        let mut source = Fake {
            albums: albums(3),
            ..Fake::default()
        };
        block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        source.albums.remove(1);
        block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        assert_eq!(ids(&catalog.albums().unwrap()), ["a0", "a2"]);
    }

    #[test]
    fn a_failed_sync_keeps_what_it_had() {
        let (_dir, catalog) = catalog();
        let mut source = Fake {
            albums: albums(700),
            ..Fake::default()
        };
        block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        source.albums.truncate(600);
        source.fail_at = Some(500);
        assert!(block_on(sync_albums(&catalog, &source, Mode::Always)).is_err());
        assert_eq!(catalog.album_count().unwrap(), 700);

        source.fail_at = None;
        let outcome = block_on(sync_albums(&catalog, &source, Mode::IfStale)).unwrap();
        assert_eq!(outcome, Outcome::Synced { count: 600 });
    }

    #[test]
    fn a_sync_during_a_scan_removes_nothing_and_stays_stale() {
        let (_dir, catalog) = catalog();
        let mut source = Fake {
            albums: albums(3),
            modified: Some(1000),
            ..Fake::default()
        };
        block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        source.albums.remove(0);
        source.scanning = true;
        let outcome = block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        assert_eq!(outcome, Outcome::Unsettled { count: 2 });
        assert_eq!(catalog.album_count().unwrap(), 3);

        // Nothing reports a change, yet the copy is known to be incomplete
        source.scanning = false;
        let outcome = block_on(sync_albums(&catalog, &source, Mode::IfStale)).unwrap();
        assert_eq!(outcome, Outcome::Synced { count: 2 });
        assert_eq!(catalog.album_count().unwrap(), 2);
    }

    #[test]
    fn a_scan_that_ran_during_the_sync_unsettles_it() {
        let (_dir, catalog) = catalog();
        let source = Fake {
            albums: albums(3),
            rescans: true,
            ..Fake::default()
        };
        let outcome = block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        assert_eq!(outcome, Outcome::Unsettled { count: 3 });
    }

    #[test]
    fn an_item_sent_twice_unsettles_the_sync() {
        let (_dir, catalog) = catalog();
        let mut source = Fake {
            albums: albums(600),
            ..Fake::default()
        };
        block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        // The second page starts with the first page's last album, as when
        // an album added before it shifts the pages.
        source.albums[500] = source.albums[499].clone();
        let outcome = block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        assert_eq!(outcome, Outcome::Unsettled { count: 599 });
        assert_eq!(catalog.album_count().unwrap(), 600);
    }

    #[test]
    fn if_stale_syncs_again_only_when_the_server_reports_a_change() {
        let (_dir, catalog) = catalog();
        let mut source = Fake {
            albums: albums(2),
            modified: Some(1000),
            ..Fake::default()
        };
        let first = block_on(sync_albums(&catalog, &source, Mode::IfStale)).unwrap();
        assert_eq!(first, Outcome::Synced { count: 2 });
        let again = block_on(sync_albums(&catalog, &source, Mode::IfStale)).unwrap();
        assert_eq!(again, Outcome::UpToDate);
        source.modified = Some(2000);
        let changed = block_on(sync_albums(&catalog, &source, Mode::IfStale)).unwrap();
        assert_eq!(changed, Outcome::Synced { count: 2 });
    }

    #[test]
    fn an_old_sync_is_stale_even_without_a_reported_change() {
        let (_dir, catalog) = catalog();
        let source = Fake {
            albums: albums(1),
            ..Fake::default()
        };
        block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        let later = SystemTime::now() + STALE_AFTER;
        assert!(
            catalog
                .is_stale(Kind::Albums, None, later, STALE_AFTER)
                .unwrap()
        );
    }

    #[test]
    fn songs_are_listed_under_their_albums_in_disc_and_track_order() {
        let (_dir, catalog) = catalog();
        let mut source = Fake {
            albums: vec![album("a1", "One"), album("a2", "Two")],
            songs: vec![
                song("s3", "a1", 2, 1),
                song("s1", "a1", 1, 1),
                song("s2", "a1", 1, 2),
                song("s4", "a2", 1, 1),
            ],
            ..Fake::default()
        };
        let id = AlbumId::new("a1");
        block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        assert!(catalog.album(&id).unwrap().is_none());

        let outcome = block_on(sync_songs(&catalog, &source, Mode::Always)).unwrap();
        assert_eq!(outcome, Outcome::Synced { count: 4 });
        let cached = catalog.album(&id).unwrap().unwrap();
        assert_eq!(song_ids(&cached), ["s1", "s2", "s3"]);

        source.songs.remove(1);
        block_on(sync_songs(&catalog, &source, Mode::Always)).unwrap();
        let cached = catalog.album(&id).unwrap().unwrap();
        assert_eq!(song_ids(&cached), ["s2", "s3"]);
        assert_eq!(catalog.song_count().unwrap(), 3);
    }

    #[test]
    fn an_album_fetched_on_its_own_replaces_its_songs() {
        let (_dir, catalog) = catalog();
        let source = Fake {
            albums: vec![album("a1", "One")],
            songs: vec![song("s1", "a1", 1, 1), song("s2", "a1", 1, 2)],
            ..Fake::default()
        };
        block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        block_on(sync_songs(&catalog, &source, Mode::Always)).unwrap();

        let fetched = subsonic::AlbumWithSongs::new(
            album("a1", "One (Remaster)"),
            vec![song("s9", "a1", 1, 1)],
        );
        catalog.put_album(&fetched).unwrap();
        let cached = catalog.album(&AlbumId::new("a1")).unwrap().unwrap();
        assert_eq!(cached.album().name(), "One (Remaster)");
        assert_eq!(song_ids(&cached), ["s9"]);

        // An album the catalog does not list waits for the album sync
        let unknown = subsonic::AlbumWithSongs::new(album("a7", "Seven"), Vec::new());
        catalog.put_album(&unknown).unwrap();
        assert!(catalog.album(&AlbumId::new("a7")).unwrap().is_none());
    }

    #[test]
    fn an_album_fetched_during_a_song_sync_survives_it() {
        let (_dir, catalog) = catalog();
        let source = Fake {
            albums: vec![album("a1", "One")],
            ..Fake::default()
        };
        block_on(sync_albums(&catalog, &source, Mode::Always)).unwrap();
        let generation = catalog.begin(Kind::Songs).unwrap();
        let fetched =
            subsonic::AlbumWithSongs::new(album("a1", "One"), vec![song("s1", "a1", 1, 1)]);
        catalog.put_album(&fetched).unwrap();
        catalog
            .finish(Kind::Songs, generation, None, SystemTime::now())
            .unwrap();
        assert_eq!(catalog.song_count().unwrap(), 1);
    }

    #[test]
    fn a_held_lock_makes_a_sync_busy() {
        let (_dir, catalog) = catalog();
        let _lock = catalog.try_lock_sync().unwrap().unwrap();
        let outcome = block_on(sync_albums(&catalog, &Fake::default(), Mode::Always)).unwrap();
        assert_eq!(outcome, Outcome::Busy);
    }

    #[test]
    fn a_scan_or_a_changed_count_unsettles_a_sync() {
        let idle = |count, last: &str| ScanStatus::new(false, Some(count), Some(last.to_owned()));
        let scanning = ScanStatus::new(true, Some(5), None);
        assert!(settled(Some(&idle(5, "a")), Some(&idle(5, "a"))));
        assert!(settled(None, None));
        assert!(settled(Some(&idle(5, "a")), None));
        assert!(!settled(Some(&idle(5, "a")), Some(&idle(6, "a"))));
        assert!(!settled(Some(&idle(5, "a")), Some(&idle(5, "b"))));
        assert!(!settled(Some(&scanning), Some(&idle(5, "a"))));
        assert!(!settled(Some(&scanning), None));
    }
}
