use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use next_ui::testing::{signed_in_core, start, wait_until};
use serde_json::json;
use subsonic::Song;

use super::*;
use crate::now_playing_cover;
use crate::testing::{CoverServer, png};

fn image(pixel: impl Fn(u32, u32) -> [u8; 4]) -> ImageSource {
    ImageSource::from(Arc::<[u8]>::from(png(64, pixel)))
}

fn hue_of(color: Color) -> f32 {
    let [r, g, b] = [color.r, color.g, color.b];
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let delta = max - min;
    let hue = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    hue.rem_euclid(360.0)
}

#[test]
fn the_seed_of_a_cover_is_its_color() {
    let seed = seed_of(image(|_, _| [30, 90, 200, 255])).unwrap();

    let hue = hue_of(seed);
    assert!((200.0..240.0).contains(&hue), "{seed:?} is not blue");
}

#[test]
fn the_seed_goes_by_the_color_that_most_of_the_cover_has() {
    let seed = seed_of(image(|x, _| {
        if x < 52 {
            [210, 60, 40, 255]
        } else {
            [40, 160, 70, 255]
        }
    }))
    .unwrap();

    let hue = hue_of(seed);
    assert!(!(60.0..300.0).contains(&hue), "{seed:?} is not red");
}

#[test]
fn what_is_not_an_image_gives_no_seed() {
    let bytes: Arc<[u8]> = Arc::from(&b"<html>not found</html>"[..]);

    assert_eq!(seed_of(ImageSource::from(bytes)), None);
}

#[test]
fn the_cover_of_what_plays_gives_the_seed_and_none_without_one() {
    let server = CoverServer::new(png(16, |_, _| [30, 90, 200, 255]));
    let (_core, hiphile) = signed_in_core(server.url());
    let workspace = hiphile.workspace().unwrap();
    let player = workspace.player().clone();
    let seen = Rc::new(Cell::new(None));
    let kept = seen.clone();
    let mut ui = start(move || {
        kept.set(Some(cover_seed(now_playing_cover(&hiphile, &workspace))));
        div()
    });
    let seed = seen.get().unwrap();
    let song = |cover: Option<&str>| {
        let mut song = json!({ "id": "a", "title": "a" });
        if let Some(cover) = cover {
            song["coverArt"] = json!(cover);
        }
        serde_json::from_value::<Song>(song).unwrap()
    };

    player.fill_queue(&[song(Some("al-a"))], 0);
    wait_until(&mut ui, || seed.get().is_some());
    let hue = hue_of(seed.get().unwrap());
    assert!((200.0..240.0).contains(&hue), "{hue} is not blue");

    player.fill_queue(&[song(None)], 0);
    wait_until(&mut ui, || seed.get().is_none());
}
