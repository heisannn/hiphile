use catalog::{SONG_CAPACITY, SongStore};
use gpui_kit::{AppContext as _, Context, SharedString, Task};

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

/// Counting and removing touch every file, so they happen off the main thread.
pub(crate) struct SongCacheUsage {
    /// `None` where the system has no cache directory.
    songs: Option<SongStore>,
    /// `None` until first counted.
    kept: Option<u64>,
    counting: bool,
    _count: Task<()>,
    clearing: bool,
    _clear: Task<()>,
    error: Option<SharedString>,
}

impl SongCacheUsage {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            // Counting and removing keep to no capacity
            songs: SongStore::user(SONG_CAPACITY).ok(),
            kept: None,
            counting: false,
            _count: Task::ready(()),
            clearing: false,
            _clear: Task::ready(()),
            error: None,
        };
        this.count(cx);
        this
    }

    pub(crate) fn kept(&self) -> Option<u64> {
        self.kept
    }

    pub(crate) fn is_clearing(&self) -> bool {
        self.clearing
    }

    pub(crate) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub(crate) fn count(&mut self, cx: &mut Context<Self>) {
        let Some(songs) = self.songs.clone() else {
            return;
        };
        if self.counting || self.clearing {
            return;
        }
        self.counting = true;
        self._count = cx.spawn(async move |this, cx| {
            let kept = cx.background_spawn(async move { songs.kept_bytes() }).await;
            this.update(cx, |this, cx| {
                this.counting = false;
                if this.kept != Some(kept) {
                    this.kept = Some(kept);
                    cx.notify();
                }
            })
            .ok();
        });
    }

    /// Removes every song kept for every server. A song being played goes on
    /// from its open file.
    pub(crate) fn clear(&mut self, cx: &mut Context<Self>) {
        let Some(songs) = self.songs.clone() else {
            return;
        };
        if self.clearing {
            return;
        }
        // A count in progress would report files that are going
        self.counting = false;
        self._count = Task::ready(());
        self.clearing = true;
        self.error = None;
        cx.notify();
        self._clear = cx.spawn(async move |this, cx| {
            let cleared = cx.background_spawn(async move { songs.clear() }).await;
            this.update(cx, |this, cx| {
                this.clearing = false;
                this.error = cleared
                    .err()
                    .map(|error| format!("Couldn't remove every song: {error}.").into());
                this.count(cx);
                cx.notify();
            })
            .ok();
        });
    }
}

pub(crate) fn format_size(bytes: u64) -> String {
    let mib = (bytes + MIB / 2) / MIB;
    if mib < GIB / MIB {
        return format!("{mib} MiB");
    }
    let tenths = (bytes as f64 / GIB as f64 * 10.0).round() / 10.0;
    format!("{tenths} GiB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_mib_then_gib() {
        assert_eq!(format_size(0), "0 MiB");
        assert_eq!(format_size(512 * MIB), "512 MiB");
        assert_eq!(format_size(GIB - 1), "1 GiB");
        assert_eq!(format_size(GIB), "1 GiB");
        assert_eq!(format_size(GIB + GIB / 2), "1.5 GiB");
        assert_eq!(format_size(8 * GIB), "8 GiB");
    }
}
