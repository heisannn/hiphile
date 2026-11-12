use anti_webui::prelude::Role;
use next_ui::testing::{core, find, has_text, press, start, wait_until};

use super::*;
use crate::testing::value;

#[test]
fn each_cache_has_its_limit() {
    let (_core, hiphile) = core("song_cache_gib = 20\n");
    let settings = hiphile.settings().clone();
    let (usage, readings) = (hiphile.song_cache_usage(), hiphile.readings().clone());
    let _ui = start(move || {
        storage_page(
            settings.song_cache(),
            settings.cover_cache(),
            &usage,
            &readings,
        )
    });

    assert_eq!(value(Role::TextInput, "Song cache limit"), "20");
    assert!(find(Role::Button, "Reset Song cache limit").is_some());
    assert!(find(Role::TextInput, "Cover cache limit").is_some());
    assert!(find(Role::Button, "Reset Cover cache limit").is_none());
    assert!(has_text("0 means no limit."));
}

#[test]
fn nothing_kept_leaves_nothing_to_clear() {
    let (_core, hiphile) = core("");
    let settings = hiphile.settings().clone();
    let readings = hiphile.readings().clone();
    // The test's core keeps no songs, which would never be counted
    let songs = tempfile::tempdir().unwrap();
    let store = catalog::SongStore::at(songs.path(), 0);
    let usage = SongCacheUsage::new(next_host::executor(), Some(store));
    let mut ui = start(move || {
        storage_page(
            settings.song_cache(),
            settings.cover_cache(),
            &usage,
            &readings,
        )
    });

    wait_until(&mut ui, || has_text("0 MiB"));
    assert!(find(Role::Button, "Clear Song Cache").unwrap().disabled);
}

#[test]
fn a_dictionary_that_cannot_be_fetched_says_why_and_can_be_tried_again() {
    let (_core, hiphile) = core("");
    let settings = hiphile.settings().clone();
    let readings = hiphile.readings().clone();
    let usage = hiphile.song_cache_usage();
    let mut ui = start({
        let readings = readings.clone();
        move || {
            storage_page(
                settings.song_cache(),
                settings.cover_cache(),
                &usage,
                &readings,
            )
        }
    });
    assert!(has_text(&format!(
        "Downloads a dictionary of {}, which takes {} on this computer.",
        format_size(BUNDLE.download_bytes),
        format_size(BUNDLE.disk_bytes),
    )));
    assert!(find(Role::Button, "Retry Downloading the Dictionary").is_none());

    press(&mut ui, Role::Switch, "Find kanji by romaji");
    wait_until(&mut ui, || {
        find(Role::Button, "Retry Downloading the Dictionary").is_some()
    });
    let ReadingsState::Failed(error) = readings.status().state().clone() else {
        panic!("the dictionary was fetched");
    };
    assert!(has_text(&error), "{error}");
}
