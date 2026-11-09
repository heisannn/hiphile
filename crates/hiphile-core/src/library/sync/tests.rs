use gpui_kit::TestAppContext;
use serde_json::json;

use super::*;
use crate::test_support::session_saving_into;
use crate::testing::{NOWHERE, server_answering, wait_for};

/// A library on GPUI's executor that saves into `catalog`, whose server is at `url`. Its first
/// sync starts at once.
fn library_sync(
    catalog: Option<catalog::Catalog>,
    url: &str,
    cx: &mut TestAppContext,
) -> LibrarySync {
    // The requests wake the test from the network's own thread
    cx.executor().allow_parking();
    let executor = cx.update(|cx| gpui_host::executor(cx));
    LibrarySync::new(executor, session_saving_into(catalog, url))
}

/// How many of each notice came since the last look: the albums and songs synced, and the syncs
/// that failed.
fn count(notices: &mut broadcast::Receiver<LibraryNotice>, counts: &mut [usize; 3]) {
    while let Ok(notice) = notices.try_recv() {
        let ix = match notice {
            LibraryNotice::AlbumsSynced => 0,
            LibraryNotice::SongsSynced => 1,
            LibraryNotice::SyncFailed(_) => 2,
            LibraryNotice::ArtistsSynced
            | LibraryNotice::SyncFinished
            | LibraryNotice::StageSynced(..) => continue,
        };
        counts[ix] += 1;
    }
}

#[gpui_kit::test]
fn a_sync_saves_the_artists_and_says_so(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let url = server_answering(json!({
        "albumList2": { "album": [] },
        "artists": { "index": [
            { "name": "M", "artist": [{ "id": "m", "name": "Miles Davis" }] },
        ]},
        "searchResult3": { "song": [] },
        "scanStatus": { "scanning": false },
    }));
    let sync = library_sync(Some(catalog.clone()), &url, cx);
    let mut notices = sync.notices();
    let mut told = false;

    wait_for(cx, || {
        told |= std::iter::from_fn(|| notices.try_recv().ok())
            .any(|notice| notice == LibraryNotice::ArtistsSynced);
        told
    });

    let saved = catalog.artists().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name(), "Miles Davis");
}

#[gpui_kit::test]
fn artists_that_fail_to_sync_do_not_stop_the_songs(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let url = server_answering(json!({
        "albumList2": { "album": [] },
        "artists": "not a list of artists",
        "searchResult3": { "song": [{ "id": "s", "title": "So What" }] },
        "scanStatus": { "scanning": false },
    }));
    let sync = library_sync(Some(catalog.clone()), &url, cx);
    let mut notices = sync.notices();
    let mut counts = [0; 3];
    sync.refresh();

    wait_for(cx, || {
        count(&mut notices, &mut counts);
        counts[1] > 0 && counts[2] > 0
    });

    assert_eq!(catalog.songs().unwrap().len(), 1);
}

#[gpui_kit::test]
fn a_sync_without_a_saved_copy_says_the_songs_may_have_changed(cx: &mut TestAppContext) {
    let url = server_answering(json!({ "albumList2": { "album": [] } }));
    let sync = library_sync(None, &url, cx);
    let mut notices = sync.notices();
    let mut counts = [0; 3];

    wait_for(cx, || {
        count(&mut notices, &mut counts);
        counts[0] > 0
    });

    assert_eq!(counts[1], counts[0]);
}

#[gpui_kit::test]
fn only_a_sync_the_user_asked_for_says_when_it_fails(cx: &mut TestAppContext) {
    let sync = library_sync(None, NOWHERE, cx);
    let mut notices = sync.notices();
    let mut counts = [0; 3];

    wait_for(cx, || {
        count(&mut notices, &mut counts);
        !sync.status().is_running()
    });
    assert_eq!(counts[2], 0);

    sync.refresh();
    wait_for(cx, || {
        count(&mut notices, &mut counts);
        counts[2] > 0
    });
    assert_eq!(counts[2], 1);
}

#[gpui_kit::test]
fn albums_that_never_showed_fail_as_albums(cx: &mut TestAppContext) {
    let sync = library_sync(None, NOWHERE, cx);
    assert!(sync.status().is_running());
    assert_eq!(*sync.status().albums(), Albums::Loading);

    wait_for(cx, || !sync.status().is_running());

    let status = sync.status();
    assert!(
        matches!(status.albums(), Albums::Failed(error) if error.starts_with("Couldn't reach"))
    );
    assert_eq!(status.failure(), None);
}

#[gpui_kit::test]
fn albums_on_show_stay_when_a_refresh_fails(cx: &mut TestAppContext) {
    let sync = library_sync(None, NOWHERE, cx);
    wait_for(cx, || !sync.status().is_running());
    let albums: Arc<[Album]> =
        vec![serde_json::from_value(json!({ "id": "a", "name": "A" })).unwrap()].into();
    sync.0
        .state
        .update(|state| state.status.albums = Albums::Loaded(albums.clone()));

    sync.refresh();
    assert_eq!(*sync.status().albums(), Albums::Loaded(albums.clone()));
    wait_for(cx, || !sync.status().is_running());

    let status = sync.status();
    assert_eq!(*status.albums(), Albums::Loaded(albums));
    assert!(
        status
            .failure()
            .is_some_and(|error| error.starts_with("Couldn't reach"))
    );
}

#[gpui_kit::test]
fn a_refreshing_sync_says_how_each_stage_ended(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let url = server_answering(json!({
        "albumList2": { "album": [] },
        "artists": { "index": [] },
        "searchResult3": { "song": [] },
        "scanStatus": { "scanning": false },
    }));
    cx.executor().allow_parking();
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let sync = LibrarySync::refreshing(executor, session_saving_into(Some(catalog), &url));
    let mut notices = sync.notices();
    let mut stages = Vec::new();

    wait_for(cx, || {
        for notice in std::iter::from_fn(|| notices.try_recv().ok()) {
            if let LibraryNotice::StageSynced(stage, outcome) = notice {
                stages.push((stage, outcome));
            }
        }
        stages.len() == 3
    });

    let synced = Outcome::Synced { count: 0 };
    assert_eq!(
        stages,
        [
            (SyncStage::Albums, synced),
            (SyncStage::Artists, synced),
            (SyncStage::Songs, synced),
        ]
    );
    assert_eq!(*sync.status().albums(), Albums::Loaded(Arc::from([])));
}

#[gpui_kit::test]
fn a_refresh_asked_for_during_a_sync_runs_once_it_ends(cx: &mut TestAppContext) {
    let sync = library_sync(None, NOWHERE, cx);
    let mut notices = sync.notices();
    let mut counts = [0; 3];

    sync.refresh();
    let mut finished = 0;
    wait_for(cx, || {
        for notice in std::iter::from_fn(|| notices.try_recv().ok()) {
            match notice {
                LibraryNotice::SyncFinished => finished += 1,
                LibraryNotice::SyncFailed(_) => counts[2] += 1,
                _ => {}
            }
        }
        finished > 0
    });

    assert!(!sync.status().is_running());
    assert_eq!((finished, counts[2]), (1, 1));
}
