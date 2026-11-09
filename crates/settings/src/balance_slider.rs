use gpui_kit::base::slider::{Slider, SliderIndicator, SliderThumb, SliderTrack};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme as _, ThemeStyled as _, h_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, Render, Styled as _, Subscription, Window, actions, div,
    prelude::FluentBuilder as _, relative, rems,
};
use hiphile_core::settings::DspSettings;

const CONTEXT: &str = "BalanceSlider";
/// Half the thumb's width, in rems
const THUMB_OVERHANG: f32 = 0.5;
/// How far above the line the thumb starts, in rems, to center it on the line
const THUMB_RISE: f32 = 0.3125;
const LINE_OPACITY: f32 = 0.2;
const THUMB_EDGE_OPACITY: f32 = 0.5;

actions!(balance_slider, [StepLeft, StepRight, ToLeftEnd, ToRightEnd]);

pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", StepLeft, Some(CONTEXT)),
        KeyBinding::new("right", StepRight, Some(CONTEXT)),
        KeyBinding::new("home", ToLeftEnd, Some(CONTEXT)),
        KeyBinding::new("end", ToRightEnd, Some(CONTEXT)),
    ]);
}

/// Only this slider changes the balance, so what it shows never has to follow the settings.
/// Built from the base slider because the component's line fills from the minimum, where a
/// balance's has to fill from the center towards the thumb.
pub(crate) struct BalanceSlider {
    dsp: DspSettings,
    slider: Entity<SliderState>,
    focus: FocusHandle,
    _slider_events: Subscription,
}

impl BalanceSlider {
    pub(crate) fn new(dsp: DspSettings, cx: &mut Context<Self>) -> Self {
        let range = DspSettings::balance_range();
        let db = dsp.snapshot().balance_db();
        let slider = cx.new(|_| {
            SliderState::new()
                .min(range.min() as f32)
                .max(range.max() as f32)
                .step(range.step() as f32)
                .default_value(db as f32)
        });
        Self {
            dsp,
            _slider_events: cx.subscribe(&slider, |this, _, event: &SliderEvent, cx| {
                let (SliderEvent::Change(value) | SliderEvent::Release(value)) = event;
                this.dsp.set_balance(f64::from(value.end()));
                cx.notify();
            }),
            slider,
            focus: cx.focus_handle().tab_stop(true),
        }
    }

    pub(crate) fn is_resettable(&self) -> bool {
        self.dsp.snapshot().balance_db() != 0.0
    }

    pub(crate) fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set(0.0, window, cx);
    }

    fn step(&mut self, steps: f64, window: &mut Window, cx: &mut Context<Self>) {
        let db = self.dsp.snapshot().balance_db() + steps * DspSettings::balance_range().step();
        self.set(db, window, cx);
    }

    fn set(&mut self, db: f64, window: &mut Window, cx: &mut Context<Self>) {
        self.dsp.set_balance(db);
        let db = self.dsp.snapshot().balance_db() as f32;
        self.slider
            .update(cx, |slider, cx| slider.set_value(db, window, cx));
        cx.notify();
    }
}

impl Render for BalanceSlider {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.slider.read(cx);
        let (min, max) = (state.min_value(), state.max_value());
        let fraction = (state.value().end() - min) / (max - min);
        let (from, to) = (fraction.min(0.5), fraction.max(0.5));
        let color = cx.theme().slider_bar;
        let radius = cx.theme().radius_full();
        let is_focused = self.focus.is_focused(window);
        let range = DspSettings::balance_range();

        h_flex()
            .id("balance")
            .key_context(CONTEXT)
            // The thumb stops a press from reaching the slider, which would otherwise take focus
            .capture_any_mouse_down(cx.listener(|this, _, window, cx| this.focus.focus(window, cx)))
            .on_action(cx.listener(|this, _: &StepLeft, window, cx| this.step(-1.0, window, cx)))
            .on_action(cx.listener(|this, _: &StepRight, window, cx| this.step(1.0, window, cx)))
            .on_action(cx.listener(move |this, _: &ToLeftEnd, window, cx| {
                this.set(range.min(), window, cx);
            }))
            .on_action(cx.listener(move |this, _: &ToRightEnd, window, cx| {
                this.set(range.max(), window, cx);
            }))
            .gap_3()
            .child(
                Slider::new(&self.slider)
                    .accessibility_label("Balance")
                    .track_focus(&self.focus)
                    .w_48()
                    .child(
                        SliderTrack::new(&self.slider)
                            .flex()
                            .items_center()
                            .w_full()
                            .h_6()
                            .child(
                                SliderIndicator::new(&self.slider)
                                    .debug_selector(|| "balance-line".to_owned())
                                    .relative()
                                    .w_full()
                                    .h_1p5()
                                    .rounded(radius)
                                    .bg(color.opacity(LINE_OPACITY))
                                    .child(
                                        div()
                                            .absolute()
                                            .h_full()
                                            .left(relative(from))
                                            .w(relative(to - from))
                                            .rounded(radius)
                                            .bg(color),
                                    )
                                    .child(
                                        SliderThumb::new(&self.slider)
                                            .debug_selector(|| "balance-thumb".to_owned())
                                            .absolute()
                                            .top(rems(-THUMB_RISE))
                                            .left(relative(fraction))
                                            .ml(rems(-THUMB_OVERHANG))
                                            .size_4()
                                            .p_px()
                                            .rounded(radius)
                                            .bg(color.opacity(THUMB_EDGE_OPACITY))
                                            .child(
                                                div()
                                                    .size_full()
                                                    .rounded(radius)
                                                    .bg(cx.theme().slider_thumb),
                                            )
                                            .when(is_focused, |this| {
                                                this.focus_ring_style(window, cx)
                                            }),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .w_32()
                    .text_right()
                    .text_sm()
                    .child(self.dsp.snapshot().balance_text()),
            )
    }
}

#[cfg(test)]
mod tests;
