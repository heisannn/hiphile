use gpui_kit::base::slider::{Slider, SliderIndicator, SliderThumb, SliderTrack};
use gpui_kit::base::spring;
use gpui_kit::component::{ActiveTheme as _, h_flex, slider::SliderState};
use gpui_kit::{
    App, Entity, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, relative, rems,
};

/// Half the thumb's width, in rems. The track reaches this far past each end of
/// the line, so that a thumb at either end stays over the slider and keeps it
/// hovered, and the slider's margin takes it back so the line fills its lane.
const THUMB_OVERHANG: f32 = 0.375;
/// How far above the line the thumb starts, in rems, to centre it on the line.
const THUMB_RISE: f32 = 0.25;
/// Opacity of the part of the line past the thumb.
const LINE_OPACITY: f32 = 0.2;

/// Where the pointer is, kept across frames per slider.
#[derive(Default)]
struct Interaction {
    hovered: bool,
    pressed: bool,
}

impl Interaction {
    /// Dragging can take the pointer off the slider, so a press keeps the thumb
    /// up until it ends.
    fn shows_thumb(&self) -> bool {
        self.hovered || self.pressed
    }

    /// Records a press or its end, for mouse down and mouse up listeners alike.
    fn press_listener<E>(
        interaction: &Entity<Self>,
        pressed: bool,
    ) -> impl Fn(&E, &mut Window, &mut App) + use<E> {
        let interaction = interaction.clone();
        move |_, _, cx| {
            interaction.update(cx, |interaction, cx| {
                if interaction.pressed != pressed {
                    interaction.pressed = pressed;
                    cx.notify();
                }
            });
        }
    }
}

/// A thin slider that shows its thumb only while the pointer is on it or
/// dragging it. It fills its lane in a row. The owner keeps its [`SliderState`].
#[derive(IntoElement)]
pub(crate) struct HoverSlider {
    state: Entity<SliderState>,
    label: SharedString,
    disabled: bool,
}

impl HoverSlider {
    pub(crate) fn new(state: &Entity<SliderState>, label: impl Into<SharedString>) -> Self {
        Self {
            state: state.clone(),
            label: label.into(),
            disabled: false,
        }
    }

    /// Takes no value and shows no thumb.
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for HoverSlider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.state.entity_id();
        let interaction = window.use_keyed_state(("hover-slider-interaction", id), cx, |_, _| {
            Interaction::default()
        });
        let shown = !self.disabled && interaction.read(cx).shows_thumb();
        let opacity: f32 = spring(
            (("hover-slider", id), "thumb"),
            if shown { 1. } else { 0. },
            cx.theme().motion_tokens().spring_control,
            window,
            cx,
        );

        let state = self.state.read(cx);
        let (min, max) = (state.min_value(), state.max_value());
        let fraction = (state.value().end() - min) / (max - min);
        let color = cx.theme().slider_bar;
        let radius = cx.theme().radius_full();

        h_flex()
            .id(("hover-slider", id))
            .flex_1()
            .min_w_0()
            .mx(rems(-THUMB_OVERHANG))
            .on_hover({
                let interaction = interaction.clone();
                move |hovered, _, cx| {
                    let hovered = *hovered;
                    interaction.update(cx, |interaction, cx| {
                        if interaction.hovered != hovered {
                            interaction.hovered = hovered;
                            cx.notify();
                        }
                    });
                }
            })
            // The thumb stops propagation on mouse down to own the drag, so the
            // press is read in the capture phase.
            .capture_any_mouse_down(Interaction::press_listener(&interaction, true))
            .capture_any_mouse_up(Interaction::press_listener(&interaction, false))
            .on_mouse_up_out(
                MouseButton::Left,
                Interaction::press_listener(&interaction, false),
            )
            .child(
                Slider::new(&self.state)
                    .disabled(self.disabled)
                    .accessibility_label(self.label)
                    .w_full()
                    .child(
                        SliderTrack::new(&self.state)
                            .disabled(self.disabled)
                            .flex()
                            .items_center()
                            .w_full()
                            .h_4()
                            .px(rems(THUMB_OVERHANG))
                            .child(
                                SliderIndicator::new(&self.state)
                                    .relative()
                                    .w_full()
                                    .h_1()
                                    .rounded(radius)
                                    .bg(color.opacity(LINE_OPACITY))
                                    .child(
                                        div()
                                            .absolute()
                                            .left_0()
                                            .h_full()
                                            .w(relative(fraction))
                                            .rounded(radius)
                                            .bg(color),
                                    )
                                    .when(shown || (!self.disabled && opacity > 0.), |this| {
                                        this.child(
                                            SliderThumb::new(&self.state)
                                                .disabled(self.disabled)
                                                .debug_selector(|| "hover-slider-thumb".to_owned())
                                                .absolute()
                                                .top(rems(-THUMB_RISE))
                                                .left(relative(fraction))
                                                .ml(rems(-THUMB_OVERHANG))
                                                .size_3()
                                                .rounded(radius)
                                                .bg(color)
                                                .opacity(opacity),
                                        )
                                    }),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests;
