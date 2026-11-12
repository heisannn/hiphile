//! The settings, in a popup over the window. The settings themselves are the core's
//! (`hiphile_core::settings`), which saves each change and says when saving fails. The appearance
//! and the color reach the theme through the shell, which follows them.

mod audio_page;
mod balance_slider;
mod choice_field;
mod color_picker;
mod controls;
mod discord_page;
mod dsd_section;
mod dsp_page;
mod focus;
mod group;
mod number_field;
mod page;
mod popup;
mod r8brain_section;
mod small_pages;
mod soxr_section;
mod storage_page;
mod swatches;
#[cfg(test)]
mod testing;

pub use popup::settings_popup;
