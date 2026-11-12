use std::cell::Cell;

use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    IconButtonStyle, TextFieldStyle, icon_toggle_button, outlined_text_field,
};
use anti_webui_md3e::icons;
use hiphile_core::settings::{DiscordSettings, DiscordSnapshot};

use crate::controls::{never, switch_of, when};
use crate::group::{Group, Row, page};

const KEY_WIDTH: i32 = 480;

pub(crate) fn discord_page(discord: &DiscordSettings) -> Div {
    let shown = next_host::follow(discord.subscribe());
    let presence_off = when(move || !shown.with(DiscordSnapshot::is_presence_on));
    let status = {
        let discord = discord.clone();
        Row::new("Show current song as status", move || {
            switch_of(
                "Show current song as status",
                &discord,
                shown,
                DiscordSnapshot::is_presence_on,
                DiscordSettings::set_presence_on,
                never(),
            )
        })
    };
    let key = {
        let discord = discord.clone();
        Row::unnamed(move || imgbb_key_field(&discord, presence_off.clone()))
    };
    page(vec![
        Group::new()
            .rows([status, key])
            .errors(move || shown.with(|shown| shown.errors().map(str::to_owned).collect()))
            .view()
            .into_view(),
    ])
}

/// Only this field changes the key, so what it shows never has to follow the settings, and they
/// cannot replace what is being typed. Its look is chosen when it is made, so showing what is
/// typed makes it anew.
fn imgbb_key_field(discord: &DiscordSettings, disabled: crate::group::When) -> Div {
    let key = signal(
        discord
            .snapshot()
            .imgbb_key()
            .unwrap_or_default()
            .to_owned(),
    );
    let first = Cell::new(true);
    let typed = discord.clone();
    effect(move || {
        let text = key.get();
        if !first.replace(false) {
            typed.set_typed(&text);
        }
    });
    let visible = signal(false);
    // The field's own disabled look leaves it to Tab and typing
    let off = disabled.clone();
    div()
        .w_full()
        .max_w(KEY_WIDTH)
        .disabled(move || off())
        .child(dyn_view(move || {
            let shown = visible.get();
            let disabled = disabled.clone();
            untrack(move || {
                let toggle_off = disabled.clone();
                let mut style = TextFieldStyle::new()
                .label("ImgBB API key")
                .supporting(|| {
                    "Shows covers on your Discord status by uploading them to ImgBB, where anyone \
                     with the link can see them for 180 days. Get a key at api.imgbb.com."
                        .to_owned()
                })
                .disabled_when(move || disabled())
                .trailing(
                    icon_toggle_button(
                        icons::VISIBILITY,
                        visible,
                        IconButtonStyle::standard().disabled_when(move || toggle_off()),
                    )
                    .access(|a| a.label("Show ImgBB API key")),
                );
                if !shown {
                    style = style.password();
                }
                outlined_text_field(key, style).w_full()
            })
        }))
}

#[cfg(test)]
mod tests;
