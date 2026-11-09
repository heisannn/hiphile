//! What only a desktop has: the Discord status and the system's media controls. They follow the
//! player as any host would, showing its snapshot and sending it commands; a mobile app does the
//! same with the platform's own.

mod discord;
mod media;

pub use discord::{DiscordNotice, DiscordStatus};
pub use media::{MediaNotice, SystemMediaControls};

use std::time::Duration;

use media_controls::{MediaCommand, MediaControls};
use presence::Presence;

use crate::Workspace;
use crate::follow;
use crate::settings::DiscordSnapshot;

/// How often what the followers show is brought up to date with where the song is.
const REFRESH: Duration = Duration::from_millis(250);
/// Pixel size of the cover they show, the same as the lyrics view's.
const COVER_PIXELS: u32 = 512;

impl Workspace {
    /// Shows what the player plays as the user's Discord status through `presence`, as the
    /// settings have it, and tells them why a cover could not go on it.
    pub fn show_on_discord(&self, presence: Presence) -> DiscordStatus {
        let status = DiscordStatus::new(
            self.executor().clone(),
            self.player().clone(),
            self.session().clone(),
            self.covers().cloned(),
            presence,
        );
        let settings = self.settings().discord().clone();
        let changes = settings.subscribe();
        let (mut key, mut is_presence_on) = {
            let discord = changes.borrow();
            (
                discord.imgbb_key().map(str::to_owned),
                discord.is_presence_on(),
            )
        };
        status.set_cover_key(key.as_deref());
        status.set_on(is_presence_on);
        let settings_changes = follow::observe(self.executor(), changes, {
            let status = status.clone();
            move |discord: DiscordSnapshot| {
                if discord.imgbb_key() != key.as_deref() {
                    key = discord.imgbb_key().map(str::to_owned);
                    status.set_cover_key(key.as_deref());
                }
                if discord.is_presence_on() != is_presence_on {
                    is_presence_on = discord.is_presence_on();
                    status.set_on(is_presence_on);
                }
            }
        });
        let notices =
            follow::subscribe(
                self.executor(),
                status.notices(),
                move |notice| match notice {
                    DiscordNotice::CoverSought { error } => {
                        settings.set_upload_error(error.as_deref())
                    }
                },
            );
        self.keep([settings_changes, notices]);
        status
    }

    /// Shows what the player plays in the system's media controls, as
    /// [`MediaControls::connect`] gives them for the app's window, and passes their commands on.
    pub fn connect_media_controls(
        &self,
        controls: (MediaControls, async_channel::Receiver<MediaCommand>),
    ) -> SystemMediaControls {
        SystemMediaControls::new(
            self.executor().clone(),
            self.player().clone(),
            self.session().clone(),
            self.covers().cloned(),
            controls,
        )
    }
}
