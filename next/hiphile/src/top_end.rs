use anti_webui::prelude::*;
use anti_webui_md3e::components::icon_button;
use anti_webui_md3e::icons;
use hiphile_core::{Hiphile, HiphileSnapshot};
use next_account::SignInRequest;
use next_ui::pane::GAP;
use next_ui::window::{kept_still, moves_window};
use next_ui::{APP_BAR_HEIGHT, Command, Commands};

use crate::server_menu::server_menu;

/// The settings and who is signed in, at the end of the window's top: over the end of the
/// screen's bar, which leaves `width` free for them, or of the queue's while it shows.
pub(crate) fn top_end(
    hiphile: Hiphile,
    shown: Signal<HiphileSnapshot>,
    commands: Commands,
    sign_in: Signal<Option<SignInRequest>>,
    width: Signal<f32>,
) -> Div {
    let settings = kept_still(
        icon_button(icons::SETTINGS)
            .access(|a| a.label("Settings"))
            .on_click(move |_| commands.send(Command::OpenSettings)),
    );
    moves_window(
        div()
            .absolute()
            .top(0)
            .right(0)
            .h(APP_BAR_HEIGHT)
            .flex()
            .flex_row()
            .items_center()
            .gap(8.0_f32)
            // The avatar's edge on the panes'
            .pr(GAP)
            .on_resized(move |w, _| width.set_if_changed(w))
            .child(settings)
            .child(server_menu(hiphile, shown, sign_in)),
    )
}
