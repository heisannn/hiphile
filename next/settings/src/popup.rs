use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{drawer_item, icon_button};
use anti_webui_md3e::icons;
use anti_webui_md3e::motion::fast_effects;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::Hiphile;
use next_ui::type_scale::styled_text;

use crate::audio_page::audio_page;
use crate::discord_page::discord_page;
use crate::dsp_page::dsp_page;
use crate::focus::keep_focus_in_view;
use crate::page::Page;
use crate::small_pages::{appearance_page, lyrics_page, queue_page};
use crate::storage_page::storage_page;

const LIST_WIDTH: i32 = 240;

/// The settings over the window and as large as it allows, with the list of pages beside the page
/// chosen. Escape and the scrim close it, and the page chosen stays chosen for the next time.
pub fn settings_popup(hiphile: Hiphile, open: Signal<bool>) -> Show {
    let chosen = signal(Page::Audio);
    modal_with(
        move || open.get(),
        move || open.set(false),
        move |modal| {
            let shown = follow(
                move || if modal.open() { 1.0f32 } else { 0.0 },
                fast_effects(),
            );
            modal.hold_while(move || shown.get() > 0.0);
            move || {
                let settings = hiphile.settings().clone();
                let usage = hiphile.song_cache_usage();
                let fresh = [settings.audio().keep_fresh(), usage.keep_fresh()];
                on_cleanup(move || drop(fresh));
                let readings = hiphile.readings().clone();

                let scrim = div().absolute().inset(0).dyn_style(move |s| {
                    let scrim = color_scheme().scrim;
                    s.bg(scrim.with_alpha(0.32 * shown.get().clamp(0.0, 1.0)))
                });
                let close = icon_button(icons::CLOSE)
                    .access(|a| a.label("Close"))
                    .on_click(move |_| open.set(false));
                let header = div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .h(64)
                    .pl(24.0_f32)
                    .pr(8.0_f32)
                    .child(
                        styled_text(TextStyle::HEADLINE_SMALL, "Settings")
                            .flex_1()
                            .access(|a| a.role(Role::Heading).level(1)),
                    )
                    .child(close);
                // Closing by a key or from assistive technology leaves the focus in a field, which
                // counts what was typed into it only as it loses the focus
                let surface_node = node_ref();
                effect(move || {
                    if !modal.open() {
                        untrack(|| surface_node.blur());
                    }
                });
                let page = dyn_view(move || {
                    let page = chosen.get();
                    let (settings, usage, readings) =
                        (settings.clone(), usage.clone(), readings.clone());
                    untrack(move || {
                        let content = match page {
                            Page::Audio => audio_page(settings.audio()),
                            Page::Dsp => dsp_page(settings.dsp()),
                            Page::Queue => queue_page(settings.queue()),
                            Page::Lyrics => lyrics_page(settings.lyrics()),
                            Page::Storage => storage_page(
                                settings.song_cache(),
                                settings.cover_cache(),
                                &usage,
                                &readings,
                            ),
                            Page::Discord => discord_page(settings.discord()),
                            Page::Appearance => appearance_page(settings.appearance()),
                        };
                        div()
                            .flex()
                            .flex_col()
                            .gap(16.0_f32)
                            .h_full()
                            .overflow_y_scroll()
                            .px(24.0_f32)
                            .pb(24.0_f32)
                            .access(move |a| a.role(Role::TabPanel).label(page.label()))
                            .child(
                                styled_text(TextStyle::TITLE_LARGE, page.label())
                                    .flex_none()
                                    .access(|a| a.role(Role::Heading).level(2)),
                            )
                            .child(content.flex_none())
                    })
                });
                let body = div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(0)
                    .child(page_list(chosen))
                    .child(div().flex_1().min_w(0).h_full().child(page));
                let surface = div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .h_full()
                    .max_w(1040)
                    .max_h(760)
                    .rounded(28.0_f32)
                    .overflow_hidden()
                    .node_ref(surface_node)
                    .access(|a| a.label("Settings"))
                    .dyn_style(move |s| {
                        let c = color_scheme();
                        let v = shown.get().clamp(0.0, 1.0);
                        s.bg(c.surface_container_high)
                            .text_color(c.on_surface)
                            .opacity(v)
                            .scale(0.96 + 0.04 * v)
                    })
                    .child(header)
                    .child(body);
                div().size_full().child(modal.scrim(scrim)).child(
                    div()
                        .absolute()
                        .inset(0)
                        .flex()
                        .items_center()
                        .justify_center()
                        .p(48.0_f32)
                        .child(modal.surface(keep_focus_in_view(surface))),
                )
            }
        },
    )
}

/// The pages as tabs, which Up and Down also go between.
fn page_list(chosen: Signal<Page>) -> Div {
    let items = Page::ALL.map(|page| (page, node_ref()));
    let tabs = items
        .iter()
        .enumerate()
        .map(|(i, &(page, item))| {
            drawer_item(page.icon(), page.label(), move || chosen.get() == page)
                .node_ref(item)
                .access(move |a| {
                    a.role(Role::Tab)
                        .label(page.label())
                        .selected(chosen.get() == page)
                })
                .on_click(move |_| chosen.set(page))
                .on_key_down(move |e| {
                    let to = match e.key {
                        Key::Named(NamedKey::ArrowDown) => (i + 1) % items.len(),
                        Key::Named(NamedKey::ArrowUp) => (i + items.len() - 1) % items.len(),
                        _ => return,
                    };
                    e.prevent_default();
                    let (page, item) = items[to];
                    chosen.set(page);
                    item.focus();
                })
        })
        .collect::<Vec<_>>();
    div()
        .flex()
        .flex_col()
        .flex_none()
        .w(LIST_WIDTH)
        .h_full()
        .overflow_y_scroll()
        .px(12.0_f32)
        .pb(12.0_f32)
        .access(|a| a.role(Role::TabList).label("Settings"))
        .children(tabs)
}

#[cfg(test)]
mod tests;
