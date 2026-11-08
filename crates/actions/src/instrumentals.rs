//! The views' copy of whether instrumentals are skipped, which the app shell keeps in step with
//! the saved setting. What counts as one is `playback`'s.

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::{Action, App, Global};
use subsonic::Song;

#[derive(Default)]
struct SkipInstrumentals(bool);

impl Global for SkipInstrumentals {}

/// Every window draws again, as any of them may show the songs this greys out.
pub fn set_skip_instrumentals(is_on: bool, cx: &mut App) {
    cx.set_global(SkipInstrumentals(is_on));
    cx.refresh_windows();
}

fn is_skipping(cx: &App) -> bool {
    cx.try_global::<SkipInstrumentals>()
        .is_some_and(|skip| skip.0)
}

pub fn is_skipped(song: &Song, cx: &App) -> bool {
    playback::is_skipped(song, is_skipping(cx))
}

pub fn without_skipped(songs: &[Song], cx: &App) -> Vec<Song> {
    playback::without_skipped(songs, is_skipping(cx))
}

pub fn queue_menu_items(
    menu: PopupMenu,
    song: &Song,
    play: impl Action,
    play_next: impl Action,
    add_to_queue: impl Action,
    cx: &App,
) -> PopupMenu {
    let is_skipped = is_skipped(song, cx);
    menu.menu_with_disabled("Play", Box::new(play), is_skipped)
        .menu_with_disabled("Play Next", Box::new(play_next), is_skipped)
        .menu_with_disabled("Add to Queue", Box::new(add_to_queue), is_skipped)
}
