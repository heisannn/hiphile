use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Task, Window, div,
    prelude::FluentBuilder as _,
};
use hiphile_core::library::LibraryActivity;

/// The notice that shows what the library is busy with.
pub(crate) struct ActivityNotice {
    activity: LibraryActivity,
    _changes: Task<()>,
}

impl ActivityNotice {
    pub(crate) fn new(activity: LibraryActivity, cx: &mut Context<Self>) -> Self {
        Self {
            _changes: gpui_host::notify_on_change(activity.subscribe(), cx),
            activity,
        }
    }

    /// The notification to put up, which stays until taken down or closed.
    pub(crate) fn notification(notice: &Entity<Self>) -> Notification {
        let closed = notice.downgrade();
        let content = notice.clone();
        Notification::new()
            .id::<Self>()
            .autohide(false)
            .on_close(move |_, cx| {
                closed
                    .update(cx, |this, _| this.activity.notice_closed())
                    .ok();
            })
            .content(move |_, _, _| content.clone().into_any_element())
    }
}

impl Render for ActivityNotice {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(status) = self.activity.status().shown().cloned() else {
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
                    .child(status.label())
                    .when_some(status.count(), |this, count| {
                        this.child(div().text_color(cx.theme().muted_foreground).child(count))
                    }),
            )
            .child(
                Progress::new("library-activity")
                    .w_full()
                    .accessibility_label(status.label())
                    .value(percent.unwrap_or_default())
                    .loading(percent.is_none()),
            )
            .into_any_element()
    }
}
