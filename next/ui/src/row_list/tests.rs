use std::collections::BTreeSet;

use anti_webui::core::Ui;
use anti_webui::core::event::{Key, Modifiers, NamedKey};

use super::*;
use crate::row_text::row_text;
use crate::testing::{
    click, click_with, double_click, drag, has_text, key, key_with, middle_of, nodes, right_click,
    scroll, start,
};

#[derive(Clone, Debug, PartialEq)]
struct Song {
    id: u32,
    title: String,
}

fn songs(ids: impl IntoIterator<Item = u32>) -> Rc<[Song]> {
    ids.into_iter()
        .map(|id| Song {
            id,
            title: format!("Song {id}"),
        })
        .collect()
}

struct List {
    rows: Signal<Rc<[Song]>>,
    selection: Selection<u32>,
    node: NodeRef,
    opened: Signal<Vec<u32>>,
    menus: Signal<Vec<(u32, PointF)>>,
    reorders: Signal<Vec<(Vec<u32>, usize)>>,
    /// How many rows were made.
    made: Rc<Cell<usize>>,
}

impl List {
    fn of(count: u32) -> Self {
        Self {
            rows: signal(songs(0..count)),
            selection: Selection::new(),
            node: node_ref(),
            opened: signal(Vec::new()),
            menus: signal(Vec::new()),
            reorders: signal(Vec::new()),
            made: Rc::default(),
        }
    }

    fn view(&self) -> Div {
        let (rows, opened, menus, reorders) = (self.rows, self.opened, self.menus, self.reorders);
        let made = self.made.clone();
        let list = RowList::new(
            "Songs",
            move || rows.get(),
            |song: &Song| song.id,
            move |song, _| {
                made.set(made.get() + 1);
                row_text(song.title.clone(), None).px(16.0_f32)
            },
        )
        .selection(self.selection)
        .node_ref(self.node)
        .on_open(move |id| opened.update(|opened| opened.push(id)))
        .on_context_menu(move |id, at| menus.update(|menus| menus.push((id, at))))
        .on_reorder(move |ids, to| reorders.update(|reorders| reorders.push((ids, to))));
        div().size_full().flex().flex_col().child(row_list(list))
    }

    fn start(&self) -> Ui {
        let view = self.view();
        start(move || view)
    }

    fn selected(&self) -> BTreeSet<u32> {
        self.selection
            .0
            .with_untracked(|selected| selected.ids().copied().collect())
    }

    fn scrolled(&self) -> f32 {
        self.node.scroll_offset().y
    }
}

fn ids(ids: impl IntoIterator<Item = u32>) -> BTreeSet<u32> {
    ids.into_iter().collect()
}

fn click_song(ui: &mut Ui, id: u32) {
    let (x, y) = middle_of(&format!("Song {id}"));
    click(ui, x, y);
}

fn click_song_with(ui: &mut Ui, id: u32, modifiers: Modifiers) {
    let (x, y) = middle_of(&format!("Song {id}"));
    click_with(ui, x, y, modifiers);
}

fn shift() -> Modifiers {
    Modifiers::default().with_shift()
}

fn key_shifted(ui: &mut Ui, named: NamedKey) {
    key_with(ui, Key::Named(named), shift());
}

#[test]
fn clicking_a_row_selects_it_alone() {
    let list = List::of(10);
    let mut ui = list.start();

    click_song(&mut ui, 2);
    click_song(&mut ui, 4);

    assert_eq!(list.selected(), ids([4]));
}

#[test]
fn shift_clicking_selects_the_rows_from_the_anchor() {
    let list = List::of(10);
    let mut ui = list.start();

    click_song(&mut ui, 2);
    click_song_with(&mut ui, 5, shift());
    assert_eq!(list.selected(), ids(2..=5));

    click_song_with(&mut ui, 1, shift());
    assert_eq!(list.selected(), ids(1..=2));
}

#[test]
fn primary_clicking_adds_a_row_or_takes_it_away() {
    let list = List::of(10);
    let mut ui = list.start();

    click_song(&mut ui, 2);
    click_song_with(&mut ui, 5, Modifiers::PRIMARY);
    click_song_with(&mut ui, 7, Modifiers::PRIMARY);
    click_song_with(&mut ui, 2, Modifiers::PRIMARY);

    assert_eq!(list.selected(), ids([5, 7]));
}

#[test]
fn keys_move_the_selection_and_bring_it_into_view() {
    let list = List::of(1000);
    let mut ui = list.start();
    click_song(&mut ui, 0);

    key(&mut ui, NamedKey::End);
    assert_eq!(list.selected(), ids([999]));
    assert!(has_text("Song 999"));
    assert!(list.scrolled() > 40_000.0);

    key(&mut ui, NamedKey::Home);
    assert_eq!(list.selected(), ids([0]));
    assert_eq!(list.scrolled(), 0.0);

    key(&mut ui, NamedKey::ArrowDown);
    key(&mut ui, NamedKey::ArrowDown);
    key(&mut ui, NamedKey::ArrowUp);
    assert_eq!(list.selected(), ids([1]));

    let page = (crate::testing::WINDOW.1 as f32 / ROW_HEIGHT).floor() as u32;
    key(&mut ui, NamedKey::PageDown);
    assert_eq!(list.selected(), ids([1 + page]));
    key(&mut ui, NamedKey::PageUp);
    assert_eq!(list.selected(), ids([1]));
}

#[test]
fn shift_with_the_keys_selects_from_the_anchor() {
    let list = List::of(10);
    let mut ui = list.start();
    click_song(&mut ui, 3);

    key_shifted(&mut ui, NamedKey::ArrowDown);
    key_shifted(&mut ui, NamedKey::ArrowDown);
    assert_eq!(list.selected(), ids(3..=5));

    for _ in 0..3 {
        key_shifted(&mut ui, NamedKey::ArrowUp);
    }
    assert_eq!(list.selected(), ids(2..=3));

    key_shifted(&mut ui, NamedKey::End);
    assert_eq!(list.selected(), ids(3..=9));
}

#[test]
fn primary_a_selects_every_row() {
    let list = List::of(30);
    let mut ui = list.start();
    click_song(&mut ui, 3);

    key_with(&mut ui, Key::Character("a".into()), Modifiers::PRIMARY);

    assert_eq!(list.selected(), ids(0..30));
}

#[test]
fn the_selection_stays_with_its_rows_as_they_are_sorted_and_filtered() {
    let list = List::of(10);
    let mut ui = list.start();
    click_song(&mut ui, 5);
    click_song_with(&mut ui, 6, shift());

    list.rows.set(songs((0..10).rev()));
    ui.update();
    assert_eq!(list.selected(), ids([5, 6]));
    let selected: Vec<String> = nodes(Role::ListOption)
        .into_iter()
        .filter(|row| row.selected == Some(true))
        .filter_map(|row| row.label)
        .collect();
    assert_eq!(selected, ["Song 6", "Song 5"]);

    // The cursor is on 6, which now comes before 5
    key(&mut ui, NamedKey::ArrowDown);
    assert_eq!(list.selected(), ids([5]));

    list.rows.set(songs([1, 3, 5, 7]));
    ui.update();
    key(&mut ui, NamedKey::ArrowDown);
    assert_eq!(list.selected(), ids([7]));
}

#[test]
fn the_cursor_stays_in_view_as_rows_come_before_it() {
    let list = List::of(20);
    let mut ui = list.start();
    click_song(&mut ui, 3);

    list.rows.set(
        songs(1000..1100)
            .iter()
            .cloned()
            .chain(songs(0..20).iter().cloned())
            .collect(),
    );
    ui.update();
    // The scroll is laid out on the first, and the rows it shows are made on the next
    ui.update();

    assert!(has_text("Song 3"));
    assert!(list.scrolled() > 0.0);
}

#[test]
fn scrolling_shows_the_rows_it_comes_to_and_keeps_the_selection() {
    let list = List::of(1000);
    let mut ui = list.start();
    click_song(&mut ui, 1);

    scroll(&mut ui, 200.0, 200.0, 500.0 * ROW_HEIGHT);
    ui.update();

    assert!(!has_text("Song 1"));
    assert!(has_text("Song 505"));
    assert_eq!(list.selected(), ids([1]));
}

#[test]
fn rows_that_come_before_one_out_of_view_leave_the_scroll() {
    let list = List::of(200);
    let mut ui = list.start();
    click_song(&mut ui, 3);
    list.node.scroll_to(0.0, 100.0 * ROW_HEIGHT);
    ui.update();

    list.rows.set(
        songs(1000..1010)
            .iter()
            .cloned()
            .chain(songs(0..200).iter().cloned())
            .collect(),
    );
    ui.update();

    assert_eq!(list.scrolled(), 100.0 * ROW_HEIGHT);
}

#[test]
fn enter_and_a_double_click_open_a_row() {
    let list = List::of(10);
    let mut ui = list.start();

    click_song(&mut ui, 2);
    key(&mut ui, NamedKey::Enter);
    let (x, y) = middle_of("Song 6");
    double_click(&mut ui, x, y);

    assert_eq!(list.opened.get_untracked(), [2, 6]);
    assert_eq!(list.selected(), ids([6]));
}

#[test]
fn the_secondary_button_opens_a_menu_on_the_selection() {
    let list = List::of(10);
    let mut ui = list.start();
    click_song(&mut ui, 1);

    let (x, y) = middle_of("Song 4");
    right_click(&mut ui, x, y);
    assert_eq!(list.selected(), ids([4]));

    click_song_with(&mut ui, 6, shift());
    let (x6, y6) = middle_of("Song 5");
    right_click(&mut ui, x6, y6);
    assert_eq!(list.selected(), ids(4..=6));

    let menus = list.menus.get_untracked();
    assert_eq!(menus.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [4, 5]);
    assert_eq!(menus[0].1, PointF::new(x, y));
}

#[test]
fn rows_dragged_go_before_the_row_they_are_let_go_on() {
    let list = List::of(10);
    let mut ui = list.start();
    let half = ROW_HEIGHT / 4.0;

    let (_, top_of_3) = middle_of("Song 3");
    drag(&mut ui, middle_of("Song 0"), (200.0, top_of_3 - half));

    click_song(&mut ui, 1);
    click_song_with(&mut ui, 2, shift());
    let (_, bottom_of_5) = middle_of("Song 5");
    drag(&mut ui, middle_of("Song 2"), (200.0, bottom_of_5 + half));

    assert_eq!(
        list.reorders.get_untracked(),
        [(vec![0], 3), (vec![1, 2], 6)]
    );
}

#[test]
fn a_row_is_made_again_only_when_another_comes_to_its_place() {
    let list = List::of(5);
    let mut ui = list.start();
    let made = list.made.get();
    assert_eq!(made, 5);

    list.rows.set(songs([0, 1, 3, 4]));
    ui.update();

    assert!(!has_text("Song 2"));
    assert!(has_text("Song 4"));
    // 3 and 4 move up, and the last place has no row
    assert_eq!(list.made.get(), made + 2);
}

#[test]
fn the_list_says_it_takes_more_than_one_and_which_rows_are_selected() {
    let list = List::of(3);
    let mut ui = list.start();
    click_song(&mut ui, 1);

    let boxes = nodes(Role::ListBox);
    assert_eq!(boxes.len(), 1);
    assert!(boxes[0].multiselectable);
    assert_eq!(boxes[0].label.as_deref(), Some("Songs"));
    let rows: Vec<(Option<String>, Option<bool>)> = nodes(Role::ListOption)
        .into_iter()
        .map(|row| (row.label, row.selected))
        .collect();
    assert_eq!(
        rows,
        [
            (Some("Song 0".into()), Some(false)),
            (Some("Song 1".into()), Some(true)),
            (Some("Song 2".into()), Some(false)),
        ]
    );
}

#[test]
fn motions_stay_within_the_rows() {
    assert_eq!(Motion::By(1).apply(Some(9), 10, 4), Some(9));
    assert_eq!(Motion::By(-1).apply(Some(0), 10, 4), Some(0));
    assert_eq!(Motion::Pages(1).apply(Some(2), 10, 4), Some(6));
    assert_eq!(Motion::Pages(1).apply(Some(8), 10, 4), Some(9));
    assert_eq!(Motion::Pages(-1).apply(Some(2), 10, 4), Some(0));
    assert_eq!(Motion::First.apply(Some(5), 10, 4), Some(0));
    assert_eq!(Motion::Last.apply(Some(5), 10, 4), Some(9));
}

#[test]
fn a_motion_from_nowhere_starts_at_an_end() {
    assert_eq!(Motion::By(1).apply(None, 10, 4), Some(0));
    assert_eq!(Motion::Pages(-1).apply(None, 10, 4), Some(0));
    assert_eq!(Motion::Last.apply(None, 10, 4), Some(9));
    assert_eq!(Motion::By(1).apply(None, 0, 4), None);
}

#[test]
fn a_row_can_be_called_other_than_what_it_shows() {
    let rows = songs(0..2);
    let _ui = start(move || {
        let list = RowList::new(
            "Songs",
            move || rows.clone(),
            |song: &Song| song.id,
            |song, _| row_text(song.title.clone(), Some("Nobody".into())),
        )
        .row_label(|song| format!("{}, by Nobody", song.title));
        div().size_full().flex().flex_col().child(row_list(list))
    });

    let labels: Vec<String> = nodes(Role::ListOption)
        .into_iter()
        .filter_map(|row| row.label)
        .collect();
    assert_eq!(labels, ["Song 0, by Nobody", "Song 1, by Nobody"]);
}
