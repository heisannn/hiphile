use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    Styled as _, Task, Window, div, prelude::FluentBuilder as _,
};
use library::SyncStage;
use session::{ReadingsState, ReadingsStatus, Session};

#[cfg(test)]
mod tests;

/// How long working out how the library reads may take before it is worth
/// mentioning: a few new texts take no time at all.
const SLOW_READING: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Status {
    label: &'static str,
    done: usize,
    /// How many in all, when that can be told. A guess, which `done` may pass.
    total: Option<usize>,
    is_percentage: bool,
}

impl Status {
    fn of_sync(stage: SyncStage, saved: usize, total: Option<usize>) -> Self {
        let label = match stage {
            SyncStage::Albums => "Syncing albums",
            SyncStage::Artists => "Syncing artists",
            SyncStage::Songs => "Syncing songs",
        };
        Self {
            label,
            done: saved,
            total,
            is_percentage: false,
        }
    }

    fn of_reading(done: usize, total: usize) -> Self {
        Self {
            label: "Preparing romaji search",
            done,
            total: Some(total),
            is_percentage: false,
        }
    }

    fn of_download(state: &ReadingsState) -> Option<Self> {
        let ReadingsState::Downloading(fraction) = state else {
            return None;
        };
        Some(Self {
            label: "Downloading dictionary",
            done: (fraction * 100.).round() as usize,
            total: Some(100),
            is_percentage: true,
        })
    }

    /// `total` while it still holds.
    fn known_total(&self) -> Option<usize> {
        self.total.filter(|total| *total > 0 && self.done <= *total)
    }

    fn percent(&self) -> Option<f32> {
        self.known_total()
            .map(|total| self.done as f32 / total as f32 * 100.)
    }

    fn count(&self) -> Option<String> {
        if self.is_percentage {
            return Some(format!("{}%", self.done));
        }
        match self.known_total() {
            Some(total) => Some(format!("{} of {total}", self.done)),
            None => (self.done > 0).then(|| self.done.to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Notice {
    is_shown: bool,
    /// The user closed it, which holds until the library next goes quiet.
    is_dismissed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoticeChange {
    Show,
    Hide,
}

impl Notice {
    fn follow(&mut self, is_busy: bool) -> Option<NoticeChange> {
        if !is_busy {
            self.is_dismissed = false;
        }
        match (is_busy, self.is_shown) {
            (true, false) if !self.is_dismissed => {
                self.is_shown = true;
                Some(NoticeChange::Show)
            }
            (false, true) => {
                self.is_shown = false;
                Some(NoticeChange::Hide)
            }
            _ => None,
        }
    }

    /// The notice went away. If it was up by then, the user closed it: taking it
    /// down is marked first, in [`Self::follow`].
    fn closed(&mut self) {
        if self.is_shown {
            self.is_shown = false;
            self.is_dismissed = true;
        }
    }
}

pub(crate) struct ActivityChanged;

/// Works out how the library reads after each sync, so nothing has to wait for it later.
pub(crate) struct LibraryActivity {
    session: Session,
    sync: Option<Status>,
    download: Option<Status>,
    reading: Option<Status>,
    is_reading: bool,
    /// Something changed during the reading, which is then read again.
    read_again: bool,
    is_reading_slow: bool,
    notice: Notice,
    /// What the notice last showed, to keep showing as it leaves.
    shown: Option<Status>,
    _read_task: Task<()>,
    _slow_reading_task: Task<()>,
    _readings_changes: Vec<Task<()>>,
}

impl EventEmitter<ActivityChanged> for LibraryActivity {}

impl LibraryActivity {
    pub(crate) fn new(session: Session, cx: &mut Context<Self>) -> Self {
        let readings_changes = session.readings().map_or_else(Vec::new, |readings| {
            vec![
                ui::on_readings_changed(readings, cx, |this: &mut Self, cx| this.read_ahead(cx)),
                gpui_host::observe(
                    readings.subscribe(),
                    cx,
                    |this: &mut Self, status: ReadingsStatus, cx| {
                        this.download_progress(Status::of_download(status.state()), cx);
                    },
                ),
            ]
        });
        Self {
            session,
            sync: None,
            download: None,
            reading: None,
            is_reading: false,
            read_again: false,
            is_reading_slow: false,
            notice: Notice::default(),
            shown: None,
            _read_task: Task::ready(()),
            _slow_reading_task: Task::ready(()),
            _readings_changes: readings_changes,
        }
    }

    pub(crate) fn sync_progress(
        &mut self,
        stage: SyncStage,
        saved: usize,
        total: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.sync = Some(Status::of_sync(stage, saved, total));
        self.changed(cx);
    }

    pub(crate) fn sync_finished(&mut self, cx: &mut Context<Self>) {
        self.sync = None;
        self.changed(cx);
        self.read_ahead(cx);
    }

    fn download_progress(&mut self, download: Option<Status>, cx: &mut Context<Self>) {
        if self.download != download {
            self.download = download;
            self.changed(cx);
        }
    }

    fn read_ahead(&mut self, cx: &mut Context<Self>) {
        if self.is_reading {
            self.read_again = true;
            return;
        }
        let Some(book) = self.session.reading_book() else {
            return;
        };
        self.is_reading = true;
        let (sender, mut progress) = mpsc::unbounded();
        let reading = cx.background_spawn(async move {
            book.read_saved_library(|done, total| {
                sender.unbounded_send((done, total)).ok();
            });
        });
        self._read_task = cx.spawn(async move |this, cx| {
            // Ends when the reading does, which drops the sender
            while let Some((done, total)) = progress.next().await {
                this.update(cx, |this, cx| this.reading_progress(done, total, cx))
                    .ok();
            }
            reading.await;
            this.update(cx, |this, cx| this.finish_reading(cx)).ok();
        });
    }

    fn reading_progress(&mut self, done: usize, total: usize, cx: &mut Context<Self>) {
        if self.reading.is_none() {
            let timer = cx.background_executor().timer(SLOW_READING);
            self._slow_reading_task = cx.spawn(async move |this, cx| {
                timer.await;
                this.update(cx, |this, cx| {
                    this.is_reading_slow = true;
                    this.changed(cx);
                })
                .ok();
            });
        }
        self.reading = Some(Status::of_reading(done, total));
        self.changed(cx);
    }

    fn finish_reading(&mut self, cx: &mut Context<Self>) {
        self.is_reading = false;
        self.reading = None;
        self.is_reading_slow = false;
        self._slow_reading_task = Task::ready(());
        self.changed(cx);
        if std::mem::take(&mut self.read_again) {
            self.read_ahead(cx);
        }
    }

    /// What to show, the sync first. A reading is left out until it proves slow.
    fn status(&self) -> Option<&Status> {
        self.sync
            .as_ref()
            .or(self.download.as_ref())
            .or(self.reading.as_ref().filter(|_| self.is_reading_slow))
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(status) = self.status() {
            self.shown = Some(status.clone());
        }
        cx.emit(ActivityChanged);
        cx.notify();
    }

    pub(crate) fn notice_change(&mut self) -> Option<NoticeChange> {
        let is_busy = self.status().is_some();
        self.notice.follow(is_busy)
    }

    /// The notice to put up, which stays until taken down or closed.
    pub(crate) fn notice(activity: &Entity<Self>) -> Notification {
        let closed = activity.downgrade();
        let content = activity.clone();
        Notification::new()
            .id::<Self>()
            .autohide(false)
            .on_close(move |_, cx| {
                closed.update(cx, |this, _| this.notice.closed()).ok();
            })
            .content(move |_, _, _| content.clone().into_any_element())
    }
}

impl Render for LibraryActivity {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(status) = self.shown.clone() else {
            return div().into_any_element();
        };
        let percent = status.percent();
        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .text_sm()
                    .child(status.label)
                    .when_some(status.count(), |this, count| {
                        this.child(div().text_color(cx.theme().muted_foreground).child(count))
                    }),
            )
            .child(
                Progress::new("library-activity")
                    .w_full()
                    .accessibility_label(status.label)
                    .value(percent.unwrap_or_default())
                    .loading(percent.is_none()),
            )
            .into_any_element()
    }
}
