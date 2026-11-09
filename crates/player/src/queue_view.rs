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
    AnyElement, App, Context, ElementId, FocusHandle, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Render, RenderOnce, ScrollStrategy, SharedString, Styled as _,
    Subscription, Task, UniformListScrollHandle, WeakEntity, Window, actions, div,
    prelude::FluentBuilder as _,
};
use hiphile_core::favorites::Favorite;
use playback::{EntryId, EntryState, Queue};
use subsonic::{Song, StarTarget};
use ui::list_navigation::{ListNavigationExt as _, Motion, page_rows};
use ui::{
    ArtistLine, PlayState, PlayStateIcon, ReorderExt as _, RowText, format_duration, row_list,
    song_artist_link,
};

use crate::SaveQueue;

pub const QUEUE_CONTEXT: &str = "Queue";

actions!(
    queue,
    [Play, PlayNext, Remove, MoveUp, MoveDown, AddToPlaylist]
);

/// The [`playback::Player`] owns the queue, and this view the selection, focus, and scrolling.
pub struct QueueView {
    core: playback::Player,
    /// Whether the queue was shuffled, as the last snapshot showed it.
    was_shuffled: bool,
    /// Kept by entry, so it follows the entry as entries move.
    selected: Option<EntryId>,
    /// The current entry the list last scrolled to.
    followed: Option<EntryId>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    _changes: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl QueueView {
    pub fn new(core: playback::Player, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let changes = gpui_host::observe(core.subscribe(), cx, |this, shown, cx| {
            this.on_queue_changed(cx);
            // Back in order, the current entry may be far from where it was
            if std::mem::replace(&mut this.was_shuffled, shown.is_shuffled()) != shown.is_shuffled()
                && let Some(ix) = shown.queue().current_ix()
            {
                this.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            }
        });
        let subscriptions = vec![
            // Focus changes the selection treatment of the rows
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
        ];
        let mut this = Self {
            was_shuffled: core.snapshot().is_shuffled(),
            core,
            selected: None,
            followed: None,
            focus,
            scroll: UniformListScrollHandle::new(),
            _changes: changes,
            _subscriptions: subscriptions,
        };
        this.on_queue_changed(cx);
        this
    }

    fn queue(&self) -> Queue<Song> {
        self.core.snapshot().queue().clone()
    }

    fn on_queue_changed(&mut self, cx: &mut Context<Self>) {
        let queue = self.queue();
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

    fn selected_ix(&self) -> Option<usize> {
        self.queue().position(self.selected?)
    }

    /// Selects the entry wherever it is now, as a click on its row does:
    /// the row's position may be from before the queue last changed.
    fn select_entry(&mut self, id: EntryId, cx: &mut Context<Self>) {
        if let Some(ix) = self.queue().position(id) {
            self.select(ix, cx);
        }
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(id) = self.queue().entries().get(ix).map(|entry| entry.id()) else {
            return;
        };
        self.selected = Some(id);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn move_selection(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let queue = self.queue();
        let (len, current_ix) = (queue.len(), queue.current_ix());
        let selected_ix = self.selected_ix();
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

    fn play_selected(&self) {
        if let Some(id) = self.selected {
            self.core.play_entry(id);
        }
    }

    fn play_selected_next(&mut self) {
        if let Some(id) = self.selected {
            self.core.move_entry_to_next(id);
            self.reveal_selected();
        }
    }

    /// Removes the selected entry and selects the one that took its place,
    /// so that repeated removals work down the list.
    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(ix)) = (self.selected, self.selected_ix()) else {
            return;
        };
        self.core.remove_entry(id);
        let queue = self.queue();
        if queue.position(id).is_some() {
            return;
        }
        match queue.len() {
            0 => self.selected = None,
            len => self.select(ix.min(len - 1), cx),
        }
    }

    fn go_to_selected_artist(&self, window: &mut Window, cx: &mut Context<Self>) {
        let artist = self
            .selected_ix()
            .and_then(|ix| self.queue().entries().get(ix)?.item().artist_id().cloned());
        if let Some(artist) = artist {
            window.dispatch_action(Box::new(actions::ShowArtist::new(artist)), cx);
        }
    }

    fn toggle_selected_favorite(&self, window: &mut Window, cx: &mut Context<Self>) {
        let song = self
            .selected_ix()
            .and_then(|ix| Some(self.queue().entries().get(ix)?.item().clone()));
        if let Some(song) = song {
            actions::toggle_favorite(Favorite::Song(song), window, cx);
        }
    }

    fn choose_playlist_for_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let song = self
            .selected_ix()
            .and_then(|ix| Some(self.queue().entries().get(ix)?.item().clone()));
        if let Some(song) = song {
            window.dispatch_action(
                Box::new(actions::ChoosePlaylist::new(Arc::from([song]))),
                cx,
            );
        }
    }

    fn move_selected(&mut self, delta: isize) {
        let (Some(id), Some(ix)) = (self.selected, self.selected_ix()) else {
            return;
        };
        let Some(to_ix) = ix.checked_add_signed(delta) else {
            return;
        };
        self.core.move_entry(id, to_ix);
        self.reveal_selected();
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
        let Some(to_ix) = self.queue().position(target) else {
            return;
        };
        self.focus.focus(window, cx);
        self.selected = Some(dragged);
        self.core.move_entry(dragged, to_ix);
        self.reveal_selected();
        cx.notify();
    }

    fn reveal_selected(&mut self) {
        if let Some(ix) = self.selected_ix() {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let queue = self.queue();
        let len = queue.len();
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
                    .disabled(!queue.is_clearable())
                    .on_click(cx.listener(|this, _, _, _| this.core.clear_queue())),
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
            .on_action(cx.listener(|this, _: &Play, _, _| this.play_selected()))
            .on_action(cx.listener(|this, _: &PlayNext, _, _| this.play_selected_next()))
            .on_action(cx.listener(|this, _: &Remove, _, cx| this.remove_selected(cx)))
            .on_action(cx.listener(|this, _: &MoveUp, _, _| this.move_selected(-1)))
            .on_action(cx.listener(|this, _: &MoveDown, _, _| this.move_selected(1)))
            .on_action(cx.listener(|this, _: &AddToPlaylist, window, cx| {
                this.choose_playlist_for_selected(window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::GoToArtist, window, cx| {
                this.go_to_selected_artist(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &actions::ToggleFavorite, window, cx| {
                    this.toggle_selected_favorite(window, cx)
                }),
            )
            .child(self.render_rows(window, cx))
            .into_any_element()
    }

    fn render_rows(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self.core.snapshot();
        let queue = snapshot.queue();
        if queue.is_empty() {
            return div()
                .p_4()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Add songs with Play Next or Add to Queue.")
                .into_any_element();
        }
        let entries = queue.snapshot();
        let queue = queue.clone();
        let playing = snapshot.is_playing();
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
                let entry_state = queue.entry_state(ix);
                Some(EntryRow {
                    id: entry.id(),
                    song: entry.item().clone(),
                    ix,
                    state: (entry_state == EntryState::Current).then(|| PlayState::new(playing)),
                    is_played: entry_state == EntryState::Played,
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

/// A screen reader's name for an entry's row, which says what the row shows.
fn entry_label(title: &SharedString, state: Option<PlayState>, is_played: bool) -> SharedString {
    if state.is_none() && is_played {
        format!("{title}, played").into()
    } else {
        PlayState::row_label(title, state)
    }
}

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
                    // The marker's lane stays when it is empty, so titles line up
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
                            this.play_selected();
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
            // Built when it opens, so it reads the queue as it is then
            .context_menu(move |menu, window, cx| {
                let commands = owner
                    .upgrade()
                    .and_then(|this| this.read(cx).queue().commands_for(id))
                    .unwrap_or_default();
                let menu = menu
                    .action_context(focus.clone())
                    .menu("Play", Box::new(Play))
                    .menu_with_disabled("Play Next", Box::new(PlayNext), !commands.play_next)
                    .separator()
                    .menu_with_disabled("Move Up", Box::new(MoveUp), !commands.move_up)
                    .menu_with_disabled("Move Down", Box::new(MoveDown), !commands.move_down)
                    .separator();
                let menu = actions::add_to_playlist_submenu(
                    menu,
                    actions::add_songs_command(Arc::from([song.clone()])),
                    window,
                    cx,
                );
                actions::favorite_menu_item(menu, &StarTarget::Song(song.id().clone()), cx)
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
