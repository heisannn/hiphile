use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    list::ListItem,
    menu::ContextMenuExt as _,
    v_flex,
};
use std::sync::Arc;

use gpui_kit::{
    AnyElement, App, Context, ElementId, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Render, RenderOnce, ScrollStrategy, SharedString, Styled as _,
    Subscription, UniformListScrollHandle, WeakEntity, Window, actions, div,
    prelude::FluentBuilder as _,
};
use playback::EntryId;
use subsonic::Song;
use ui::list_navigation::{ListNavigationExt as _, Motion, page_rows};
use ui::{
    ArtistLine, PlayState, PlayStateIcon, ReorderExt as _, RowText, format_duration, row_list,
    song_artist_link,
};

use crate::{Player, SaveQueue};

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
        MoveDown,
        /// Asks which playlist to add the selected entry to.
        AddToPlaylist
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
            _ => motion.apply(selected_ix, len, page_rows(&self.scroll, len)),
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

    fn go_to_selected_artist(&self, window: &mut Window, cx: &mut Context<Self>) {
        let artist = self.selected_ix(cx).and_then(|ix| {
            let entries = self.player.read(cx).queue().entries();
            entries.get(ix)?.item().artist_id().cloned()
        });
        if let Some(artist) = artist {
            window.dispatch_action(Box::new(actions::ShowArtist::new(artist)), cx);
        }
    }

    fn choose_playlist_for_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let song = self.selected_ix(cx).and_then(|ix| {
            let entries = self.player.read(cx).queue().entries();
            Some(entries.get(ix)?.item().clone())
        });
        if let Some(song) = song {
            window.dispatch_action(
                Box::new(actions::ChoosePlaylist::new(Arc::from([song]))),
                cx,
            );
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
                Button::new("save-queue")
                    .ghost()
                    .xsmall()
                    .label("Save as Playlist…")
                    .disabled(len == 0)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(SaveQueue), cx)),
            )
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
            .on_list_navigation(|this, motion, _, cx| this.move_selection(motion, cx), cx)
            .on_action(cx.listener(|this, _: &Play, _, cx| this.play_selected(cx)))
            .on_action(cx.listener(|this, _: &PlayNext, _, cx| this.play_selected_next(cx)))
            .on_action(cx.listener(|this, _: &Remove, _, cx| this.remove_selected(cx)))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_selected(-1, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_selected(1, cx)))
            .on_action(cx.listener(|this, _: &AddToPlaylist, window, cx| {
                this.choose_playlist_for_selected(window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::GoToArtist, window, cx| {
                this.go_to_selected_artist(window, cx)
            }))
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
        let is_focused = self.focus.is_focused(window);
        let focus = self.focus.clone();
        let owner = cx.entity().downgrade();
        row_list(
            "queue-list",
            "Queue",
            entries.len(),
            &self.scroll,
            move |ix| {
                let entry = entries.get(ix)?;
                let is_current = current_ix == Some(ix);
                Some(EntryRow {
                    id: entry.id(),
                    song: entry.item().clone(),
                    ix,
                    state: is_current.then(|| PlayState::new(playing)),
                    // With nothing current, playback ran off the end.
                    is_played: current_ix.is_none_or(|current| ix < current),
                    is_selected: selected == Some(entry.id()),
                    is_focused,
                    owner: owner.clone(),
                    focus: focus.clone(),
                })
            },
        )
        .size_full()
        .into_any_element()
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

/// A screen reader's name for an entry's row, which says what the row shows.
fn entry_label(title: &SharedString, state: Option<PlayState>, is_played: bool) -> SharedString {
    if state.is_none() && is_played {
        format!("{title}, played").into()
    } else {
        PlayState::row_label(title, state)
    }
}

/// An entry of the queue, whose commands act on the selection.
#[derive(IntoElement)]
struct EntryRow {
    id: EntryId,
    song: Song,
    ix: usize,
    /// There on the current entry.
    state: Option<PlayState>,
    is_played: bool,
    is_selected: bool,
    is_focused: bool,
    owner: WeakEntity<QueueView>,
    focus: FocusHandle,
}

impl RenderOnce for EntryRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            song,
            ix,
            state,
            is_played,
            is_selected,
            is_focused,
            owner,
            focus,
        } = self;
        let muted_foreground = cx.theme().muted_foreground;
        let title = SharedString::from(song.title().to_owned());
        let artist = ArtistLine::new(song_artist_link(&song), None);
        let has_artist = song.artist_id().is_some();
        let duration = song.duration().map(format_duration).unwrap_or_default();
        let on_click = owner.clone();
        let on_right_click = owner.clone();
        let on_drop = owner.clone();
        ListItem::new(ElementId::NamedInteger("queue-entry".into(), id.get()))
            .selected(is_selected)
            .secondary_selected(is_focused && is_selected)
            .h_12()
            .px_4()
            .w_full()
            .accessibility_label(entry_label(&title, state, is_played))
            .child(
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
                            .when_some(state, |this, state| this.child(PlayStateIcon::new(state))),
                    )
                    .child(
                        RowText::new(title.clone())
                            .detail_line(artist)
                            .dimmed(is_played),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(muted_foreground)
                            .child(duration),
                    ),
            )
            .on_click(move |event, window, cx| {
                on_click
                    .update(cx, |this, cx| {
                        this.focus.focus(window, cx);
                        this.select_entry(id, cx);
                        if event.click_count() >= 2 {
                            this.play_selected(cx);
                        }
                    })
                    .ok();
            })
            // The menu acts on the selection, so the row under the pointer
            // becomes it first.
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                on_right_click
                    .update(cx, |this, cx| {
                        this.focus.focus(window, cx);
                        this.select_entry(id, cx);
                    })
                    .ok();
            })
            .reorderable(id, ix, title, move |dragged, window, cx| {
                on_drop
                    .update(cx, |this, cx| this.drop_entry(dragged, id, window, cx))
                    .ok();
            })
            // Built when it opens, so it reads the queue as it is then.
            .context_menu(move |menu, window, cx| {
                let commands = owner
                    .upgrade()
                    .map(|this| this.read(cx).entry_commands(id, cx))
                    .unwrap_or_default();
                let menu = menu
                    .action_context(focus.clone())
                    .menu("Play", Box::new(Play))
                    .menu_with_disabled("Play Next", Box::new(PlayNext), !commands.play_next)
                    .separator()
                    .menu_with_disabled("Move Up", Box::new(MoveUp), !commands.move_up)
                    .menu_with_disabled("Move Down", Box::new(MoveDown), !commands.move_down)
                    .separator();
                actions::add_to_playlist_submenu(
                    menu,
                    actions::add_songs_command(Arc::from([song.clone()])),
                    window,
                    cx,
                )
                .when(has_artist, |menu| {
                    menu.menu("Go to Artist", Box::new(actions::GoToArtist))
                })
                .separator()
                .menu_with_disabled("Remove", Box::new(Remove), !commands.remove)
            })
    }
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

#[cfg(test)]
mod tests;
