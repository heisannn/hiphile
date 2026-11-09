use gpui_kit::{
    AppContext as _, Entity, Modifiers, MouseButton, Pixels, Point, TestAppContext,
    VisualTestContext, point,
};
use serde_json::json;
use ui::list_navigation::{bind_control_keys, bind_list_navigation};

use super::*;

fn song(id: &str) -> Song {
    serde_json::from_value(json!({ "id": id, "title": id })).unwrap()
}

/// The queue of a player that plays nothing, so that nothing here reaches an
/// output device or the server, holding a song per id with the one at
/// `current_ix` current.
fn queue<'a>(
    ids: &[&str],
    current_ix: usize,
    cx: &'a mut TestAppContext,
) -> (Entity<QueueView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_control_keys(cx);
        bind_list_navigation(QUEUE_CONTEXT, cx);
        actions::bind_toggle_favorite(QUEUE_CONTEXT, cx);
    });
    let songs: Vec<Song> = ids.iter().copied().map(song).collect();
    // The list takes its height from the window's root, as in the app
    let (handle, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
                let core = crate::testing::idle_core(cx);
                core.fill_queue(&songs, current_ix);
                cx.new(|cx| QueueView::new(core, window, cx))
            })
        })
        .unwrap();
    let cx = VisualTestContext::from_window(handle, cx).into_mut();
    cx.run_until_parked();
    (view, cx)
}

fn song_ids(view: &Entity<QueueView>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        view.read(cx)
            .queue()
            .entries()
            .iter()
            .map(|entry| entry.item().id().as_str().to_owned())
            .collect()
    })
}

fn selected_ix(view: &Entity<QueueView>, cx: &mut VisualTestContext) -> Option<usize> {
    cx.update(|_, cx| {
        let this = view.read(cx);
        this.queue().position(this.selected?)
    })
}

fn focus(view: &Entity<QueueView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let focus = view.read(cx).focus.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();
}

fn row(view: &Entity<QueueView>, ix: usize, cx: &mut VisualTestContext) -> Point<Pixels> {
    cx.update(|_, cx| {
        let this = view.read(cx);
        let len = this.queue().len();
        let state = this.scroll.0.borrow();
        let list = state.base_handle.bounds();
        let height = state.last_item_size.unwrap().contents.height / len as f32;
        point(
            list.origin.x + list.size.width / 2.,
            list.origin.y + height * (ix as f32 + 0.5),
        )
    })
}

fn drag(from: usize, to: usize, view: &Entity<QueueView>, cx: &mut VisualTestContext) {
    let (from, to) = (row(view, from, cx), row(view, to, cx));
    let none = Modifiers::default();
    cx.simulate_mouse_move(from, None, none);
    cx.simulate_mouse_down(from, MouseButton::Left, none);
    cx.simulate_mouse_move(to, MouseButton::Left, none);
    cx.simulate_mouse_move(to, MouseButton::Left, none);
    cx.simulate_mouse_up(to, MouseButton::Left, none);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_favorite_command_follows_the_entry_selected(cx: &mut TestAppContext) {
    let (view, cx) = queue(&["a", "b"], 0, cx);
    focus(&view, cx);
    cx.update(|_, cx| view.update(cx, |view, cx| view.select(1, cx)));
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        actions::set_known_favorites(Some(Default::default()), cx);
        let asked = asked.clone();
        cx.on_action(move |action: &actions::ToggleFavoriteOf, _| {
            asked.borrow_mut().push(action.favorite().target());
        });
    });

    let key = if cfg!(target_os = "macos") {
        "cmd-d"
    } else {
        "ctrl-d"
    };
    cx.simulate_keystrokes(key);
    cx.run_until_parked();

    assert_eq!(
        *asked.borrow(),
        [subsonic::StarTarget::Song(subsonic::SongId::new("b"))]
    );
}

#[test]
fn a_rows_label_says_how_far_along_it_is() {
    let title = SharedString::from("One");
    let label = |state, is_played| entry_label(&title, state, is_played);
    assert_eq!(label(Some(PlayState::Playing), false), "One, playing");
    assert_eq!(label(Some(PlayState::Paused), false), "One, paused");
    assert_eq!(label(None, true), "One, played");
    assert_eq!(label(None, false), "One");
}

#[gpui_kit::test]
fn an_entry_dragged_down_lands_after_the_row_dropped_on(cx: &mut TestAppContext) {
    let (view, cx) = queue(&["a", "b", "c"], 0, cx);

    drag(0, 2, &view, cx);

    assert_eq!(song_ids(&view, cx), ["b", "c", "a"]);
    assert_eq!(selected_ix(&view, cx), Some(2));
}

#[gpui_kit::test]
fn an_entry_dragged_up_lands_before_the_row_dropped_on(cx: &mut TestAppContext) {
    let (view, cx) = queue(&["a", "b", "c"], 0, cx);

    drag(2, 0, &view, cx);

    assert_eq!(song_ids(&view, cx), ["c", "a", "b"]);
    assert_eq!(selected_ix(&view, cx), Some(0));
}

#[gpui_kit::test]
fn the_keys_move_the_selection_starting_from_the_current_entry(cx: &mut TestAppContext) {
    let (view, cx) = queue(&["a", "b", "c", "d"], 1, cx);
    focus(&view, cx);

    for (key, ix) in [("down", 1), ("down", 2), ("up", 1), ("end", 3), ("home", 0)] {
        cx.simulate_keystrokes(key);
        assert_eq!(selected_ix(&view, cx), Some(ix), "after {key}");
    }
}

#[gpui_kit::test]
fn removing_an_entry_selects_the_one_in_its_place(cx: &mut TestAppContext) {
    let (view, cx) = queue(&["a", "b", "c"], 0, cx);
    focus(&view, cx);
    cx.simulate_keystrokes("end up");

    cx.update(|_, cx| view.update(cx, |this, cx| this.remove_selected(cx)));
    assert_eq!(song_ids(&view, cx), ["a", "c"]);
    assert_eq!(selected_ix(&view, cx), Some(1));

    cx.update(|_, cx| view.update(cx, |this, cx| this.remove_selected(cx)));
    assert_eq!(song_ids(&view, cx), ["a"]);
    assert_eq!(selected_ix(&view, cx), Some(0));
}

#[gpui_kit::test]
fn the_selection_stays_on_its_entry_through_a_shuffle(cx: &mut TestAppContext) {
    let (view, cx) = queue(&["a", "b", "c", "d", "e", "f", "g", "h"], 0, cx);
    focus(&view, cx);
    cx.simulate_keystrokes("end");
    let selected = cx.update(|_, cx| view.read(cx).selected);

    let toggle = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            view.read(cx).core.toggle_shuffle();
        });
        cx.run_until_parked();
    };
    toggle(cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).selected), selected);
    assert_eq!(song_ids(&view, cx)[0], "a");

    toggle(cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).selected), selected);
    assert_eq!(selected_ix(&view, cx), Some(7));
    assert_eq!(
        song_ids(&view, cx),
        ["a", "b", "c", "d", "e", "f", "g", "h"]
    );
}

#[gpui_kit::test]
fn going_back_in_order_brings_the_current_entry_into_view(cx: &mut TestAppContext) {
    let ids: Vec<String> = (0..100).map(|ix| ix.to_string()).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (view, cx) = queue(&ids, 0, cx);
    let songs: Vec<Song> = ids.iter().copied().map(song).collect();
    cx.update(|_, cx| {
        let core = &view.read(cx).core;
        core.toggle_shuffle();
        // Shuffled, the song chosen comes first, wherever it was
        core.fill_queue(&songs, 80);
    });
    cx.run_until_parked();
    let offset = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| view.read(cx).scroll.0.borrow().base_handle.offset().y)
    };
    assert_eq!(offset(cx), gpui_kit::px(0.));

    cx.update(|_, cx| {
        view.read(cx).core.toggle_shuffle();
    });
    cx.run_until_parked();
    assert!(offset(cx) < gpui_kit::px(0.));
}

#[gpui_kit::test]
fn repeat_goes_on_from_off_to_all_to_one_and_back(cx: &mut TestAppContext) {
    let (view, cx) = queue(&["a"], 0, cx);
    let core = cx.update(|_, cx| view.read(cx).core.clone());
    for expected in [
        playback::RepeatMode::All,
        playback::RepeatMode::One,
        playback::RepeatMode::Off,
    ] {
        core.cycle_repeat();
        assert_eq!(core.snapshot().repeat(), expected);
    }
}

#[gpui_kit::test]
fn the_page_keys_move_by_the_rows_in_view(cx: &mut TestAppContext) {
    let ids: Vec<String> = (0..100).map(|ix| ix.to_string()).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (view, cx) = queue(&ids, 0, cx);
    focus(&view, cx);
    let page = cx.update(|_, cx| {
        let this = view.read(cx);
        let state = this.scroll.0.borrow();
        let row = state.last_item_size.unwrap().contents.height / ids.len() as f32;
        (state.base_handle.bounds().size.height / row).floor() as usize
    });
    assert!(page > 1, "the window shows {page} rows");

    cx.simulate_keystrokes("home pagedown");
    assert_eq!(selected_ix(&view, cx), Some(page));
    cx.simulate_keystrokes("pagedown");
    assert_eq!(selected_ix(&view, cx), Some(page * 2));
    cx.simulate_keystrokes("pageup");
    assert_eq!(selected_ix(&view, cx), Some(page));
}
