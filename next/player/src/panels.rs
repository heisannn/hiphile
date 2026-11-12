use anti_webui::prelude::*;
use next_ui::row_list::Selection;
use playback::EntryId;

/// The queue's width when it first opens, and the bounds of what it can be made.
pub const QUEUE_WIDTH: f32 = 320.0;
pub const QUEUE_MIN_WIDTH: f32 = 256.0;
pub const QUEUE_MAX_WIDTH: f32 = 448.0;

/// Which of the panels beside the pane are open, which the bar switches, and what the queue keeps
/// while it is closed.
#[non_exhaustive]
#[derive(Clone, Copy)]
pub struct Panels {
    /// Whether the queue is asked for. It shows only where it fits.
    pub queue: Signal<bool>,
    /// Whether the lyrics are asked for. They show over all of the window.
    pub lyrics: Signal<bool>,
    /// Whether any of the lyrics shows, coming or going, which then has the bar's cover.
    pub(crate) lyrics_shown: Signal<bool>,
    /// Whether the lyrics have come all the way and hide all that is under them.
    pub(crate) lyrics_cover: Signal<bool>,
    /// The bar's cover, which the lyrics' cover leaves from and comes back to.
    pub(crate) bar_cover: NodeRef,
    /// The most the queue can be given beside the screens.
    queue_room: Signal<f32>,
    /// What the user made it, which less room makes it narrower than.
    queue_width: Signal<f32>,
    pub(crate) queue_selection: Selection<EntryId>,
}

impl Panels {
    pub fn new() -> Self {
        Self {
            queue: signal(false),
            lyrics: signal(false),
            lyrics_shown: signal(false),
            lyrics_cover: signal(false),
            bar_cover: node_ref(),
            queue_room: signal(f32::INFINITY),
            queue_width: signal(QUEUE_WIDTH),
            queue_selection: Selection::new(),
        }
    }

    /// Whether the lyrics hide all that is under them, which then takes no keys. Tracks it.
    pub fn is_covered(&self) -> bool {
        self.lyrics_cover.get()
    }

    /// Tells the queue the most it can be given beside the screens, which the owner of the layout
    /// finds as it lays them out. With less than its least, the queue does not show, and cannot be
    /// opened, until there is room again; it stays asked for meanwhile.
    pub fn set_queue_room(&self, most: f32) {
        self.queue_room.set_if_changed(most);
    }

    /// Tracks it.
    pub fn queue_fits(&self) -> bool {
        self.queue_room.get() >= QUEUE_MIN_WIDTH
    }

    /// Whether the queue is asked for and fits. Tracks it.
    pub fn is_queue_shown(&self) -> bool {
        self.queue.get() && self.queue_fits()
    }

    /// Whether the queue shows and the lyrics are not over it. Tracks it.
    pub fn is_queue_in_view(&self) -> bool {
        self.is_queue_shown() && !self.lyrics.get()
    }

    /// Opens the queue or closes it, where it fits. Under the lyrics, it closes them and shows
    /// the queue.
    pub fn toggle_queue(&self) {
        if !untrack(|| self.queue_fits()) {
            return;
        }
        if self.lyrics.get_untracked() {
            self.lyrics.set(false);
            self.queue.set(true);
        } else {
            self.queue.update(|open| *open = !*open);
        }
    }

    /// How wide the queue shows. Tracks it.
    pub(crate) fn queue_width(&self) -> f32 {
        queue_width_in(self.queue_width.get(), self.queue_most())
    }

    /// The most the queue can be made now. Tracks it.
    pub(crate) fn queue_most(&self) -> f32 {
        self.queue_room
            .get()
            .clamp(QUEUE_MIN_WIDTH, QUEUE_MAX_WIDTH)
    }

    pub(crate) fn set_queue_width(&self, width: f32) {
        self.queue_width.set_if_changed(width);
    }
}

impl Default for Panels {
    fn default() -> Self {
        Self::new()
    }
}

/// How wide the queue shows when it is asked to be `wanted` wide and can be `most`: narrower when
/// there is less room, down to its least.
pub(crate) fn queue_width_in(wanted: f32, most: f32) -> f32 {
    wanted.min(most).max(QUEUE_MIN_WIDTH)
}

#[cfg(test)]
mod tests;
