use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    list::ListItem,
    menu::ContextMenuExt as _,
    scroll::Scrollbar,
    v_flex,
};
use std::cmp::Ordering;

use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Pixels, Point, Render,
    ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    UniformListScrollHandle, Window, actions, div, prelude::FluentBuilder as _, uniform_list,
};
use playback::EntryId;
use subsonic::Song;
use ui::format_duration;
use ui::list_navigation::{
    Motion, SelectFirst, SelectLast, SelectNext, SelectPageDown, SelectPageUp, SelectPrevious,
    page_rows,
};

use crate::Player;

/// Key context of the focused queue list.
pub const QUEUE_CONTEXT: &str = "Queue";

actions!(
    queue,
    [
        /// Plays the selected entry.
        Play,
        /// Moves the selected entry to right after the current one.
        PlayNext,
        /// Removes the selected entry, unless it is the current one.
        Remove,
        MoveUp,
        MoveDown
    ]
);

/// The play queue as a list beside the library: what has played, what is
/// playing, and what plays next. The [`Player`] owns the queue; this view
/// owns the selection, focus, and scrolling.
pub struct QueueView {
    player: Entity<Player>,
    /// Kept by entry, so it follows the entry as entries move.
    selected: Option<EntryId>,
    /// The current entry the list last scrolled to.
    followed: Option<EntryId>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl QueueView {
    pub fn new(player: Entity<Player>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            cx.subscribe(&player, |this, _, _, cx| this.on_queue_changed(cx)),
            // Focus changes the selection treatment of the rows.
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
        ];
        let mut this = Self {
            player,
            selected: None,
            followed: None,
            focus,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.on_queue_changed(cx);
        this
    }

    /// Drops a selection whose entry is gone, and brings a new current
    /// entry into view.
    fn on_queue_changed(&mut self, cx: &mut Context<Self>) {
        let queue = self.player.read(cx).queue();
        if self
            .selected
            .is_some_and(|selected| queue.position(selected).is_none())
        {
            self.selected = None;
        }
        let current = queue.current_id();
        if current != self.followed {
            self.followed = current;
            if let Some(ix) = queue.current_ix() {
                self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            }
        }
        cx.notify();
    }

    fn selected_ix(&self, cx: &Context<Self>) -> Option<usize> {
        self.player.read(cx).queue().position(self.selected?)
    }

    /// Selects the entry wherever it is now, as a click on its row does:
    /// the row's position may be from before the queue last changed.
    fn select_entry(&mut self, id: EntryId, cx: &mut Context<Self>) {
        if let Some(ix) = self.player.read(cx).queue().position(id) {
            self.select(ix, cx);
        }
    }

    /// Which commands apply to the entry, as the queue is now.
    fn entry_commands(&self, id: EntryId, cx: &App) -> EntryCommands {
        let queue = self.player.read(cx).queue();
        let (Some(ix), current_ix) = (queue.position(id), queue.current_ix()) else {
            return EntryCommands::default();
        };
        let is_current = current_ix == Some(ix);
        EntryCommands {
            play_next: current_ix.is_some() && !is_current,
            move_up: ix > 0,
            move_down: ix + 1 < queue.len(),
            remove: !is_current,
        }
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(id) = self
            .player
            .read(cx)
            .queue()
            .entries()
            .get(ix)
            .map(|entry| entry.id())
        else {
            return;
        };
        self.selected = Some(id);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn move_selection(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let queue = self.player.read(cx).queue();
        let (len, current_ix) = (queue.len(), queue.current_ix());
        let selected_ix = self.selected_ix(cx);
        // A step from no selection lands on the current entry, where a
        // look at the queue usually starts.
        let target = match (selected_ix, current_ix, motion) {
            (None, Some(current), Motion::By(_) | Motion::Pages(_)) => Some(current),
            _ => motion.apply(selected_ix, len, page_rows(&self.scroll)),
        };
        if let Some(ix) = target {
            self.select(ix, cx);
        }
    }

    fn play_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected {
            self.player
                .update(cx, |player, cx| player.play_entry(id, cx));
        }
    }

    fn play_selected_next(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected {
            self.player
                .update(cx, |player, cx| player.move_entry_to_next(id, cx));
            self.reveal_selected(cx);
        }
    }

    /// Removes the selected entry and selects the one that took its place,
    /// so that repeated removals work down the list.
    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(ix)) = (self.selected, self.selected_ix(cx)) else {
            return;
        };
        self.player
            .update(cx, |player, cx| player.remove_entry(id, cx));
        let queue = self.player.read(cx).queue();
        if queue.position(id).is_some() {
            return;
        }
        match queue.len() {
            0 => self.selected = None,
            len => self.select(ix.min(len - 1), cx),
        }
    }

    fn move_selected(&mut self, delta: isize, cx: &mut Context<Self>) {
        let (Some(id), Some(ix)) = (self.selected, self.selected_ix(cx)) else {
            return;
        };
        let Some(to_ix) = ix.checked_add_signed(delta) else {
            return;
        };
        self.player
            .update(cx, |player, cx| player.move_entry(id, to_ix, cx));
        self.reveal_selected(cx);
    }

    /// Moves the dragged entry to where `target` is: after it when dragged
    /// down, before it when dragged up, as the row's drop line shows.
    fn drop_entry(
        &mut self,
        dragged: EntryId,
        target: EntryId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(to_ix) = self.player.read(cx).queue().position(target) else {
            return;
        };
        self.focus.focus(window, cx);
        self.selected = Some(dragged);
        self.player
            .update(cx, |player, cx| player.move_entry(dragged, to_ix, cx));
        self.reveal_selected(cx);
        cx.notify();
    }

    fn reveal_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected_ix(cx) {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let queue = self.player.read(cx).queue();
        let len = queue.len();
        // Clear keeps the current entry, so only more than that clears.
        let clearable = len > usize::from(queue.current_ix().is_some());
        h_flex()
            .h_10()
            .flex_none()
            .px_4()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().font_medium().child("Queue"))
            .when(len > 0, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(len.to_string()),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("clear-queue")
                    .ghost()
                    .xsmall()
                    .label("Clear")
                    .disabled(!clearable)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.player.update(cx, |player, cx| player.clear_queue(cx));
                    })),
            )
            .into_any_element()
    }

    /// The list's focus target, rendered whether or not there are entries
    /// so focus and the commands bound to it never land on an element that
    /// is gone.
    fn render_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("queue")
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .key_context(QUEUE_CONTEXT)
            .on_action(
                cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(Motion::By(1), cx)),
            )
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| {
                this.move_selection(Motion::By(-1), cx)
            }))
            .on_action(
                cx.listener(|this, _: &SelectFirst, _, cx| this.move_selection(Motion::First, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectLast, _, cx| this.move_selection(Motion::Last, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectPageDown, _, cx| {
                this.move_selection(Motion::Pages(1), cx)
            }))
            .on_action(cx.listener(|this, _: &SelectPageUp, _, cx| {
                this.move_selection(Motion::Pages(-1), cx)
            }))
            .on_action(cx.listener(|this, _: &Play, _, cx| this.play_selected(cx)))
            .on_action(cx.listener(|this, _: &PlayNext, _, cx| this.play_selected_next(cx)))
            .on_action(cx.listener(|this, _: &Remove, _, cx| this.remove_selected(cx)))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_selected(-1, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_selected(1, cx)))
            .child(self.render_rows(window, cx))
            .into_any_element()
    }

    fn render_rows(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let player = self.player.read(cx);
        let queue = player.queue();
        if queue.is_empty() {
            return div()
                .p_4()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Add songs with Play Next or Add to Queue.")
                .into_any_element();
        }
        let entries = queue.snapshot();
        let current_ix = queue.current_ix();
        let playing = player.is_playing();
        let selected = self.selected;
        let focused = self.focus.is_focused(window);
        let muted_foreground = cx.theme().muted_foreground;
        let primary = cx.theme().primary;
        let focus = self.focus.clone();
        let owner = cx.entity().downgrade();
        div()
            .id("queue-list")
            .relative()
            .size_full()
            .role(gpui_kit::Role::List)
            .aria_label("Queue")
            .child(
                uniform_list("queue-rows", entries.len(), move |range, _, _| {
                    range
                        .filter_map(|ix| {
                            let entry = entries.get(ix)?;
                            let id = entry.id();
                            let song = entry.item();
                            let is_selected = selected == Some(id);
                            let is_current = current_ix == Some(ix);
                            // With nothing current, playback ran off the end.
                            let played = current_ix.is_none_or(|current| ix < current);
                            let title = SharedString::from(song.title().to_owned());
                            let label = match (is_current, playing, played) {
                                (true, true, _) => format!("{title}, playing").into(),
                                (true, false, _) => format!("{title}, paused").into(),
                                (false, _, true) => format!("{title}, played").into(),
                                (false, _, false) => title.clone(),
                            };
                            let owner = owner.clone();
                            let focus = focus.clone();
                            let title_for_drag = title.clone();
                            Some(
                                ListItem::new(ElementId::NamedInteger(
                                    "queue-entry".into(),
                                    id.get(),
                                ))
                                .selected(is_selected)
                                .secondary_selected(focused && is_selected)
                                .h_12()
                                .px_4()
                                .w_full()
                                .accessibility_label(label)
                                .child(render_entry(
                                    song,
                                    title,
                                    is_current.then_some(playing),
                                    played,
                                    muted_foreground,
                                    primary,
                                ))
                                .on_click({
                                    let owner = owner.clone();
                                    move |event, window, cx| {
                                        owner
                                            .update(cx, |this, cx| {
                                                this.focus.focus(window, cx);
                                                this.select_entry(id, cx);
                                                if event.click_count() >= 2 {
                                                    this.play_selected(cx);
                                                }
                                            })
                                            .ok();
                                    }
                                })
                                // The menu acts on the selection, so the row
                                // under the pointer becomes it first.
                                .on_mouse_down(MouseButton::Right, {
                                    let owner = owner.clone();
                                    move |_, window, cx| {
                                        owner
                                            .update(cx, |this, cx| {
                                                this.focus.focus(window, cx);
                                                this.select_entry(id, cx);
                                            })
                                            .ok();
                                    }
                                })
                                .on_drag(
                                    DraggedEntry {
                                        id,
                                        from_ix: ix,
                                        title: title_for_drag,
                                    },
                                    |dragged, offset, _, cx| {
                                        cx.new(|_| DragPreview {
                                            title: dragged.title.clone(),
                                            offset,
                                        })
                                    },
                                )
                                // A line on the edge the entry would land beside.
                                .drag_over::<DraggedEntry>(move |style, dragged, _, cx| {
                                    let line = cx.theme().drag_border;
                                    match dragged.from_ix.cmp(&ix) {
                                        Ordering::Less => style.border_b_2().border_color(line),
                                        Ordering::Greater => style.border_t_2().border_color(line),
                                        Ordering::Equal => style,
                                    }
                                })
                                .on_drop({
                                    let owner = owner.clone();
                                    move |dragged: &DraggedEntry, window, cx| {
                                        owner
                                            .update(cx, |this, cx| {
                                                this.drop_entry(dragged.id, id, window, cx)
                                            })
                                            .ok();
                                    }
                                })
                                // Built when it opens, so it reads the queue
                                // as it is then.
                                .context_menu(
                                    move |menu, _, cx| {
                                        let commands = owner
                                            .upgrade()
                                            .map(|this| this.read(cx).entry_commands(id, cx))
                                            .unwrap_or_default();
                                        menu.action_context(focus.clone())
                                            .menu("Play", Box::new(Play))
                                            .menu_with_disabled(
                                                "Play Next",
                                                Box::new(PlayNext),
                                                !commands.play_next,
                                            )
                                            .separator()
                                            .menu_with_disabled(
                                                "Move Up",
                                                Box::new(MoveUp),
                                                !commands.move_up,
                                            )
                                            .menu_with_disabled(
                                                "Move Down",
                                                Box::new(MoveDown),
                                                !commands.move_down,
                                            )
                                            .separator()
                                            .menu_with_disabled(
                                                "Remove",
                                                Box::new(Remove),
                                                !commands.remove,
                                            )
                                    },
                                ),
                            )
                        })
                        .collect()
                })
                .size_full()
                .track_scroll(&self.scroll),
            )
            .child(Scrollbar::vertical(&self.scroll))
            .into_any_element()
    }
}

/// An entry being dragged to a new place in the queue.
struct DraggedEntry {
    id: EntryId,
    /// Where it was when the drag started, which tells the rows under the
    /// pointer which side it would land on.
    from_ix: usize,
    title: SharedString,
}

/// What follows the pointer while an entry is dragged.
struct DragPreview {
    title: SharedString,
    /// The pointer's offset in the row the drag started on.
    offset: Point<Pixels>,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The drag draws this from the row's corner, where it was relative to
        // the pointer; the measured offset brings the chip to the pointer.
        div().pl(self.offset.x).pt(self.offset.y).child(
            div()
                .max_w_64()
                .px_3()
                .py_1()
                .truncate()
                .text_sm()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .text_color(cx.theme().popover_foreground)
                .shadow_md()
                .child(self.title.clone()),
        )
    }
}

/// The commands an entry's context menu enables.
#[derive(Clone, Copy, Default)]
struct EntryCommands {
    play_next: bool,
    move_up: bool,
    move_down: bool,
    remove: bool,
}

/// A row's content. `marker` is `Some(playing)` on the current entry.
fn render_entry(
    song: &Song,
    title: SharedString,
    marker: Option<bool>,
    played: bool,
    muted_foreground: gpui_kit::Hsla,
    primary: gpui_kit::Hsla,
) -> impl IntoElement {
    let artist = song
        .artist()
        .map(|artist| SharedString::from(artist.to_owned()));
    let duration = song.duration().map(format_duration).unwrap_or_default();
    h_flex()
        .w_full()
        .min_w_0()
        .gap_3()
        // The marker's lane stays when it is empty, so titles line up.
        .child(
            h_flex()
                .w_4()
                .flex_none()
                .justify_center()
                .when_some(marker, |this, playing| {
                    this.child(
                        Icon::new(IconName::AudioLines)
                            .small()
                            .text_color(if playing { primary } else { muted_foreground }),
                    )
                }),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .truncate()
                        .when(played, |this| this.text_color(muted_foreground))
                        .child(title),
                )
                .when_some(artist, |this, artist| {
                    this.child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(muted_foreground)
                            .child(artist),
                    )
                }),
        )
        .child(
            div()
                .flex_none()
                .text_xs()
                .text_color(muted_foreground)
                .child(duration),
        )
}

impl Render for QueueView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().sidebar)
            .child(self.render_header(cx))
            .child(self.render_list(window, cx))
    }
}
