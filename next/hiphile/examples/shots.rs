//! Draws the window into PNG files without opening one, to see how it looks:
//! `cargo run -p next-hiphile --example shots -- <folder>`. It needs a GPU, not a display. Menus
//! open as windows of their own, which it does not draw.

use std::path::{Path, PathBuf};

use anti_webui::app::headless;
use anti_webui::core::Ui;
use anti_webui::prelude::{Role, signal};
use next_hiphile::{MenuBar, WINDOW, app};
use next_ui::testing::{NOWHERE, core, frames, press, signed_in_core};

const SCALE: f64 = 2.0;

fn main() {
    let folder = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "shots".to_owned()),
    );
    std::fs::create_dir_all(&folder).expect("a folder for the shots");
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        shot(
            &folder.join(format!("sign-in-{theme}.png")),
            dark,
            false,
            |_| {},
        );
        shot(
            &folder.join(format!("home-{theme}.png")),
            dark,
            true,
            |_| {},
        );
        shot(
            &folder.join(format!("albums-narrow-{theme}.png")),
            dark,
            true,
            |ui| {
                press(ui, Role::Button, "Close Navigation");
                press(ui, Role::Button, "Albums");
            },
        );
        shot(
            &folder.join(format!("add-server-{theme}.png")),
            dark,
            true,
            |ui| {
                press(ui, Role::Button, "Servers");
                press(ui, Role::MenuItem, "Add Server…");
            },
        );
        shot(
            &folder.join(format!("settings-{theme}.png")),
            dark,
            true,
            |ui| {
                press(ui, Role::Button, "Settings");
            },
        );
    }
}

fn shot(path: &Path, dark: bool, signed_in: bool, drive: impl FnOnce(&mut Ui)) {
    let (_core, hiphile) = if signed_in {
        signed_in_core(NOWHERE)
    } else {
        core("")
    };
    let (width, height) = (
        (WINDOW.0 as f64 * SCALE) as u32,
        (WINDOW.1 as f64 * SCALE) as u32,
    );
    let pixels = headless::render_driven_rgba(
        width,
        height,
        SCALE,
        move || app(hiphile, signal(dark), MenuBar::new()),
        |ui| {
            frames(ui, 100);
            drive(ui);
            frames(ui, 900);
        },
    )
    .expect("a GPU to draw with");
    headless::write_png(path, width, height, &pixels).expect("the shot written");
    println!("{}", path.display());
}
