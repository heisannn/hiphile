use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _,
    radio::{Radio, RadioGroup},
    setting::{SettingField, SettingGroup, SettingItem, SettingPage},
    v_flex,
};
use gpui_kit::{Entity, ParentElement as _, SharedString, Styled as _};
use hiphile_core::settings::{CROSSFEED_PRESETS, DspSettings, DspSnapshot};

use crate::audio_options::dropdown_of;
use crate::balance_slider::BalanceSlider;

pub(crate) fn dsp_page(dsp: &DspSettings, balance: &Entity<BalanceSlider>) -> SettingPage {
    let snapshot = dsp.snapshot();
    let errors: Vec<SharedString> = snapshot.errors().map(SharedString::new).collect();
    let mut crossfeed = SettingGroup::new().items(crossfeed_items(dsp, &snapshot));
    if !errors.is_empty() {
        crossfeed = crossfeed.footer(move |_, cx| {
            v_flex()
                .gap_1()
                .text_color(cx.theme().danger)
                .children(errors.iter().cloned())
        });
    }
    SettingPage::new("DSP")
        .icon(IconName::SlidersHorizontal)
        .group(crossfeed)
        .group(SettingGroup::new().item(balance_item(balance)))
}

fn crossfeed_items(dsp: &DspSettings, snapshot: &DspSnapshot) -> Vec<SettingItem> {
    let switch = SettingField::switch(
        {
            let dsp = dsp.clone();
            move |_| dsp.snapshot().is_crossfeed_on()
        },
        {
            let dsp = dsp.clone();
            move |is_on, _| dsp.set_crossfeed(is_on)
        },
    );
    let mut items = vec![
        SettingItem::new("Crossfeed", switch)
            .description("Makes headphones sound more like speakers. Lowers the level by 3 dB.")
            .keywords(["headphones", "bs2b"]),
    ];
    if !snapshot.is_crossfeed_on() {
        return items;
    }
    let preset = SettingField::render({
        let dsp = dsp.clone();
        move |_, _, _| {
            let selected = dsp.snapshot().crossfeed_preset_ix();
            let dsp = dsp.clone();
            RadioGroup::horizontal("crossfeed-preset")
                .selected_index(Some(selected))
                .children(
                    CROSSFEED_PRESETS
                        .iter()
                        .enumerate()
                        .map(|(ix, label)| Radio::new(("crossfeed-preset", ix)).label(*label)),
                )
                .on_click(move |ix, _, _| dsp.select_crossfeed_preset(*ix))
        }
    });
    let separation = dropdown_of(
        dsp,
        DspSettings::snapshot,
        snapshot.separation_options(),
        DspSnapshot::separation_value,
        DspSettings::set_crossfeed_separation,
    );
    let cutoff = dropdown_of(
        dsp,
        DspSettings::snapshot,
        snapshot.cutoff_options(),
        DspSnapshot::cutoff_value,
        DspSettings::set_crossfeed_cutoff,
    );
    items.extend(
        [
            SettingItem::new("Preset", preset),
            SettingItem::new("Separation", separation),
            SettingItem::new("Cutoff", cutoff),
        ]
        .map(|item| item.keywords(["crossfeed", "headphones"])),
    );
    items
}

fn balance_item(balance: &Entity<BalanceSlider>) -> SettingItem {
    let field = SettingField::render({
        let balance = balance.clone();
        move |_, _, _| balance.clone()
    })
    .on_reset(
        {
            let balance = balance.clone();
            move |cx| balance.read(cx).is_resettable()
        },
        {
            let balance = balance.clone();
            move |window, cx| balance.update(cx, |balance, cx| balance.reset(window, cx))
        },
    );
    SettingItem::new("Balance", field).keywords(["left", "right", "channel"])
}
