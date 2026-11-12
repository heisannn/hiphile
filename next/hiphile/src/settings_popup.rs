use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::icon_button;
use anti_webui_md3e::icons;
use anti_webui_md3e::motion::fast_effects;
use anti_webui_md3e::typography::TextStyle;
use next_ui::type_scale::styled_text;

/// The settings, over the window and as large as it allows. Escape and the scrim close it.
pub(crate) fn settings_popup(open: Signal<bool>) -> Show {
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
                let scrim = div().absolute().inset(0).dyn_style(move |s| {
                    let scrim = color_scheme().scrim;
                    s.bg(scrim.with_alpha(0.32 * shown.get().clamp(0.0, 1.0)))
                });
                let close = icon_button(icons::CLOSE)
                    .access(|a| a.label("Close"))
                    .on_click(move |_| open.set(false));
                let surface = div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .h_full()
                    .max_w(1040)
                    .max_h(760)
                    .rounded(28.0_f32)
                    .overflow_hidden()
                    .access(|a| a.label("Settings"))
                    .dyn_style(move |s| {
                        let c = color_scheme();
                        let v = shown.get().clamp(0.0, 1.0);
                        s.bg(c.surface_container_high)
                            .text_color(c.on_surface)
                            .opacity(v)
                            .scale(0.96 + 0.04 * v)
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_none()
                            .items_center()
                            .h(64)
                            .pl(24.0_f32)
                            .pr(8.0_f32)
                            .child(
                                styled_text(TextStyle::TITLE_LARGE, "Settings")
                                    .flex_1()
                                    .access(|a| a.role(Role::Heading).level(1)),
                            )
                            .child(close),
                    );
                div().size_full().child(modal.scrim(scrim)).child(
                    div()
                        .absolute()
                        .inset(0)
                        .flex()
                        .items_center()
                        .justify_center()
                        .p(48.0_f32)
                        .child(modal.surface(surface)),
                )
            }
        },
    )
}
