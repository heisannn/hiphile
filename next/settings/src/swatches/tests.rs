use anti_webui::core::event::NamedKey;
use next_ui::testing::{find, focus, key, nodes, press, shows_keys, start};

use super::*;

fn chosen_swatch() -> String {
    let checked: Vec<_> = nodes(Role::Radio)
        .into_iter()
        .filter(|radio| radio.checked == Some(anti_webui::core::access::Checked::True))
        .filter_map(|radio| radio.label)
        .collect();
    assert_eq!(checked.len(), 1, "{checked:?}");
    checked[0].clone()
}

fn start_with(color: ThemeColor) -> (anti_webui::core::Ui, Signal<ThemeColor>) {
    let chosen = std::rc::Rc::new(std::cell::Cell::new(None));
    let ui = start({
        let chosen = chosen.clone();
        move || {
            let color = signal(color);
            chosen.set(Some(color));
            swatches(
                move || color.get(),
                move |picked| color.set(picked),
                Schemes::new(|| false, signal(None)),
            )
        }
    });
    (ui, chosen.get().expect("the view made"))
}

#[test]
fn there_is_a_swatch_for_each_color_and_the_chosen_one_is_checked() {
    let (_ui, _) = start_with(ThemeColor::Cover);

    assert!(find(Role::RadioGroup, "Color").is_some());
    let names: Vec<_> = nodes(Role::Radio)
        .into_iter()
        .filter_map(|radio| radio.label)
        .collect();
    let expected: Vec<_> = THEME_COLORS.iter().map(|(_, name)| *name).collect();
    assert_eq!(names, expected);
    assert_eq!(chosen_swatch(), "From the cover");
}

#[test]
fn pressing_a_swatch_chooses_its_color() {
    let (mut ui, color) = start_with(ThemeColor::Cover);

    press(&mut ui, Role::Radio, "Teal");
    assert_eq!(color.get_untracked(), ThemeColor::Teal);
    assert_eq!(chosen_swatch(), "Teal");
}

#[test]
fn the_arrow_keys_choose_the_next_color_and_go_around() {
    let (mut ui, color) = start_with(ThemeColor::Cover);
    focus(&mut ui, Role::RadioGroup, "Color");

    key(&mut ui, NamedKey::ArrowLeft);
    assert_eq!(color.get_untracked(), ThemeColor::Pink);
    key(&mut ui, NamedKey::ArrowRight);
    key(&mut ui, NamedKey::ArrowRight);
    assert_eq!(color.get_untracked(), ThemeColor::Red);
    assert_eq!(chosen_swatch(), "Red");
}

#[test]
fn the_keys_show_on_the_chosen_swatch() {
    let (mut ui, _) = start_with(ThemeColor::Blue);
    focus(&mut ui, Role::RadioGroup, "Color");
    key(&mut ui, NamedKey::ArrowRight);

    assert!(shows_keys(Role::Radio, "Purple"));
    assert!(!shows_keys(Role::Radio, "Blue"));
}
