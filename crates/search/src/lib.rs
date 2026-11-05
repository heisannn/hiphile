mod results;
mod search_view;

use gpui_kit::{App, KeyBinding, actions};

pub use search_view::{Search, SearchEvent};

actions!(search, [OpenSearch]);

/// Call before building the menu bar, which shows the first of them.
pub fn init(context: &'static str, cx: &mut App) {
    let keys = if cx.platform_info().is_mac() {
        ["cmd-k", "cmd-shift-p"]
    } else {
        ["ctrl-k", "ctrl-shift-p"]
    };
    cx.bind_keys(keys.map(|key| KeyBinding::new(key, OpenSearch, Some(context))));
}
