use super::*;
use crate::testing::{find, frames, has_text, press, start};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Order {
    Title,
    Year,
}

struct Sorted {
    order: Signal<Order>,
    descending: Signal<bool>,
    covers: Covers,
}

impl Sorted {
    fn new() -> Self {
        Self {
            order: signal(Order::Title),
            descending: signal(false),
            covers: Covers::new(),
        }
    }

    fn view(&self) -> Div {
        let (order, descending) = (self.order, self.descending);
        let by = |label: &str, to: Order| {
            SortOption::new(label, move || order.get() == to, move || order.set(to))
        };
        let way = |label: &str, to: bool| {
            SortOption::new(
                label,
                move || descending.get() == to,
                move || descending.set(to),
            )
        };
        sort_menu(
            self.covers,
            move || descending.get(),
            vec![
                vec![by("Title", Order::Title), by("Year", Order::Year)],
                vec![way("Ascending", false), way("Descending", true)],
            ],
        )
    }
}

#[test]
fn the_button_names_the_order() {
    let sorted = Sorted::new();
    let view = sorted.view();
    let mut ui = start(move || view);

    assert!(has_text("Title"));
    assert!(find(Role::Button, "Sort by Title, Ascending").is_some());

    sorted.order.set(Order::Year);
    sorted.descending.set(true);
    ui.update();
    assert!(has_text("Year"));
    assert!(find(Role::Button, "Sort by Year, Descending").is_some());
}

#[test]
fn choosing_from_the_menu_orders_the_list_and_closes_the_menu() {
    let sorted = Sorted::new();
    let view = sorted.view();
    let mut ui = start(move || view);
    assert!(!sorted.covers.any());

    press(&mut ui, Role::Button, "Sort by Title, Ascending");
    frames(&mut ui, 300);
    assert_eq!(
        find(Role::Button, "Sort by Title, Ascending")
            .unwrap()
            .expanded,
        Some(true)
    );
    assert!(sorted.covers.any());
    press(&mut ui, Role::MenuItem, "Year");
    frames(&mut ui, 300);

    assert_eq!(sorted.order.get_untracked(), Order::Year);
    let button = find(Role::Button, "Sort by Year, Ascending").expect("the button");
    assert_eq!(button.expanded, Some(false));
    assert!(find(Role::MenuItem, "Year").is_none());
    assert!(!sorted.covers.any());
}

#[test]
fn the_way_is_chosen_from_the_same_menu() {
    let sorted = Sorted::new();
    let view = sorted.view();
    let mut ui = start(move || view);

    press(&mut ui, Role::Button, "Sort by Title, Ascending");
    frames(&mut ui, 300);
    press(&mut ui, Role::MenuItem, "Descending");

    assert!(sorted.descending.get_untracked());
}

#[test]
fn without_a_checked_order_the_button_says_sort() {
    let _ui = start(|| sort_menu(Covers::new(), || false, Vec::new()));

    assert!(find(Role::Button, "Sort").is_some());
}
