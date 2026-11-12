use anti_webui::prelude::*;
use hiphile_core::settings::{CROSSFEED_PRESETS, DspSettings, DspSnapshot};

use crate::balance_slider::balance_slider;
use crate::choice_field::choice_of;
use crate::controls::{never, radios_of, switch_of};
use crate::group::{Group, Row, page};

pub(crate) fn dsp_page(dsp: &DspSettings) -> Div {
    let shown = next_host::follow(dsp.subscribe());
    let is_on = move || shown.with(DspSnapshot::is_crossfeed_on);
    let crossfeed = {
        let dsp = dsp.clone();
        Row::new("Crossfeed", move || {
            switch_of(
                "Crossfeed",
                &dsp,
                shown,
                DspSnapshot::is_crossfeed_on,
                DspSettings::set_crossfeed,
                never(),
            )
        })
        .supporting_text("Makes headphones sound more like speakers. Lowers the level by 3 dB.")
    };
    let preset = {
        let dsp = dsp.clone();
        Row::new("Preset", move || {
            radios_of(
                "Preset",
                &CROSSFEED_PRESETS,
                &dsp,
                shown,
                DspSnapshot::crossfeed_preset_ix,
                DspSettings::select_crossfeed_preset,
                never(),
            )
        })
        .visible_when(is_on)
    };
    let separation = {
        let dsp = dsp.clone();
        Row::new("Separation", move || {
            choice_of(
                "Separation",
                &dsp,
                shown,
                DspSnapshot::separation_options,
                DspSnapshot::separation_value,
                DspSettings::set_crossfeed_separation,
                never(),
            )
        })
        .visible_when(is_on)
    };
    let cutoff = {
        let dsp = dsp.clone();
        Row::new("Cutoff", move || {
            choice_of(
                "Cutoff",
                &dsp,
                shown,
                DspSnapshot::cutoff_options,
                DspSnapshot::cutoff_value,
                DspSettings::set_crossfeed_cutoff,
                never(),
            )
        })
        .visible_when(is_on)
    };
    let balance = {
        let dsp = dsp.clone();
        Row::new("Balance", move || balance_slider(&dsp))
    };
    page(vec![
        Group::new()
            .rows([crossfeed, preset, separation, cutoff])
            .errors(move || shown.with(|shown| shown.errors().map(str::to_owned).collect()))
            .view()
            .into_view(),
        Group::new().row(balance).view().into_view(),
    ])
}

#[cfg(test)]
mod tests;
