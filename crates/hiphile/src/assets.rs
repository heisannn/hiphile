use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

// Icons used by the features beyond GPUI Kit's default set. An icon missing
// here renders as nothing, so add every new `gpui_kit::assets::IconName` the
// features start using.
gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        ArrowDown,
        ArrowUp,
        AudioLines,
        Disc3,
        ListMusic,
        MicVocal,
        Repeat,
        Repeat1,
        Shuffle,
        SkipBack,
        SkipForward,
        User,
        Volume1,
        Volume2,
        VolumeX
    ]
);

/// GPUI Kit's default assets plus the extra icons.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match ExtraIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
