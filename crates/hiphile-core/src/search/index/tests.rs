use executor::testing::TestExecutor;
use serde_json::{Value, json};
use subsonic::{Album, ScanStatus, Song};

use super::*;
use crate::search::Hit;
use crate::test_support::session_saving_into;
use crate::testing::NOWHERE;

struct Saved {
    albums: Value,
    songs: Value,
}

impl catalog::Source for Saved {
    async fn albums_page(&self, _: u32, _: u32) -> Result<Vec<Album>, client::Error> {
        Ok(serde_json::from_value(self.albums.clone()).unwrap())
    }

    async fn artists(&self) -> Result<Vec<subsonic::Artist>, client::Error> {
        Ok(Vec::new())
    }

    async fn songs_page(&self, _: u32, _: u32) -> Result<Vec<Song>, client::Error> {
        Ok(serde_json::from_value(self.songs.clone()).unwrap())
    }

    async fn scan_status(&self) -> Result<ScanStatus, client::Error> {
        Ok(serde_json::from_value(json!({ "scanning": false })).unwrap())
    }

    async fn library_modified(&self) -> Result<Option<u64>, client::Error> {
        Ok(None)
    }
}

fn save(catalog: &catalog::Catalog, source: &Saved) {
    let mode = catalog::Mode::Always;
    futures::executor::block_on(catalog::sync_albums(catalog, source, mode)).unwrap();
    futures::executor::block_on(catalog::sync_songs(catalog, source, mode)).unwrap();
}

fn saved_library(dir: &tempfile::TempDir) -> catalog::Catalog {
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    save(
        &catalog,
        &Saved {
            albums: json!([
                { "id": "blue", "name": "Kind of Blue", "artist": "Miles Davis",
                  "artistId": "miles" },
            ]),
            songs: json!([
                { "id": "so-what", "title": "So What", "artist": "Miles Davis",
                  "artistId": "miles", "album": "Kind of Blue", "albumId": "blue" },
                { "id": "naima", "title": "Naima", "artist": "John Coltrane",
                  "artistId": "john" },
            ]),
        },
    );
    catalog
}

fn resync(catalog: &catalog::Catalog, title: &str) {
    save(
        catalog,
        &Saved {
            albums: json!([]),
            songs: json!([{ "id": title, "title": title, "artist": "Someone", "artistId": "s" }]),
        },
    );
}

fn index(catalog: Option<catalog::Catalog>, cx: &TestExecutor) -> SearchIndex {
    let executor = cx.executor();
    SearchIndex::new(executor, session_saving_into(catalog, NOWHERE))
}

fn found_songs(index: &SearchIndex) -> Vec<String> {
    index
        .status()
        .results()
        .sections
        .iter()
        .flat_map(|(_, hits)| hits)
        .filter_map(|hit| match hit {
            Hit::Song(song) => Some(song.title().to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_library_that_changed_shows_in_the_search_without_asking_again() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let catalog = saved_library(&dir);
    let index = index(Some(catalog.clone()), cx);
    index.open();
    cx.run_until_parked();
    index.search("a");
    cx.run_until_parked();
    assert!(found_songs(&index).contains(&"Naima".to_owned()));

    resync(&catalog, "A Love Supreme");
    index.library_changed();
    cx.run_until_parked();

    assert_eq!(found_songs(&index), ["A Love Supreme"]);
}

#[test]
fn a_search_before_the_first_reading_is_done_finds_once_it_is() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let index = index(Some(saved_library(&dir)), cx);

    index.open();
    index.search("naima");
    assert!(index.status().is_waiting());
    cx.run_until_parked();

    assert!(!index.status().is_waiting());
    assert_eq!(found_songs(&index), ["Naima"]);
}

#[test]
fn the_last_search_is_the_one_that_shows() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let index = index(Some(saved_library(&dir)), cx);
    index.open();
    cx.run_until_parked();

    index.search("so what");
    index.search("naima");
    cx.run_until_parked();

    assert_eq!(found_songs(&index), ["Naima"]);
}

#[test]
fn an_empty_search_finds_nothing_at_once() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let index = index(Some(saved_library(&dir)), cx);
    index.open();
    index.search("naima");
    cx.run_until_parked();

    index.search(" ");

    assert!(index.status().results().sections.is_empty());
}

#[test]
fn a_library_that_changed_while_never_opened_waits_to_be() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let index = index(Some(saved_library(&dir)), cx);

    index.library_changed();

    let status = index.status();
    assert!(!status.is_reading());
    assert_eq!(*status.library(), IndexState::Unread);
}

#[test]
fn without_a_saved_library_it_says_so() {
    let cx = &TestExecutor::new();
    let index = index(None, cx);

    index.open();

    let status = index.status();
    assert!(matches!(status.library(), IndexState::Failed(_)));
    assert_eq!(
        status.failure(),
        Some(
            "There's no library saved on this computer to search. Check that the disk has free space, then sign out and sign in again."
        )
    );
}
