//! The window has no title bar: what is at its top moves it.

use anti_webui::prelude::*;

/// Dragging `area` moves the window, and double-clicking it zooms the window, as a title bar does.
/// What is pressable in it is [`kept_still`].
pub fn moves_window(area: Div) -> Div {
    area.on_pointer_down(|event| match event.button {
        Some(PointerButton::Primary) if event.click_count == 2 => toggle_window_maximized(),
        Some(PointerButton::Primary) => start_window_move(),
        _ => {}
    })
}

/// Pressing `control` in a [`moves_window`] area does not move the window.
pub fn kept_still(control: Div) -> Div {
    control.on_pointer_down(|event| event.stop_propagation())
}
