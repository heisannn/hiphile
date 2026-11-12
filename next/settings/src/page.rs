use anti_webui_md3e::icons::{self, Symbol};

/// One page of the settings, in the order of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    Audio,
    Dsp,
    Queue,
    Lyrics,
    Storage,
    Discord,
    Appearance,
}

impl Page {
    pub(crate) const ALL: [Self; 7] = [
        Self::Audio,
        Self::Dsp,
        Self::Queue,
        Self::Lyrics,
        Self::Storage,
        Self::Discord,
        Self::Appearance,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Dsp => "DSP",
            Self::Queue => "Queue",
            Self::Lyrics => "Lyrics",
            Self::Storage => "Storage",
            Self::Discord => "Discord",
            Self::Appearance => "Appearance",
        }
    }

    pub(crate) fn icon(self) -> &'static Symbol {
        match self {
            Self::Audio => icons::GRAPHIC_EQ,
            Self::Dsp => icons::TUNE,
            Self::Queue => icons::QUEUE_MUSIC,
            Self::Lyrics => icons::LYRICS,
            Self::Storage => icons::HARD_DRIVE,
            Self::Discord => icons::PUBLIC,
            Self::Appearance => icons::PALETTE,
        }
    }
}
