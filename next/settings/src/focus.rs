use anti_webui::core::tree::with_doc;
use anti_webui::prelude::*;

/// `view`, which scrolls what Tab and Shift-Tab move the focus to into view.
///
/// AntiWebUI scrolls to a text caret but not to the focus, so without this Tab leaves the focus
/// out of sight in a long page. Tab moves it after the handlers have run, so this waits a frame.
pub(crate) fn keep_focus_in_view(view: Div) -> Div {
    view.on_key_down(|e| {
        if matches!(e.key, Key::Named(NamedKey::Tab)) {
            request_frame(|_| {
                with_doc(|d| {
                    if let Some(focused) = d.state.focused {
                        d.pending_scroll.push(focused);
                        d.dirty.paint = true;
                    }
                });
            });
        }
    })
}
