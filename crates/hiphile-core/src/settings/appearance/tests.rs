use executor::testing::TestExecutor;

use super::*;

fn store_with(contents: &str) -> (tempfile::TempDir, config::Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    std::fs::create_dir_all(store.dir()).unwrap();
    std::fs::write(store.config_path(), contents).unwrap();
    (dir, store)
}

#[test]
fn a_new_color_is_told_and_saved_and_the_same_one_is_neither() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with("");
    let settings = AppearanceSettings::new(cx.executor(), Ok(store.clone()));
    let told = settings.subscribe();
    assert_eq!(settings.snapshot().theme_color(), ThemeColor::Cover);

    settings.set_theme_color(ThemeColor::Cover);
    cx.run_until_parked();
    assert!(!told.has_changed().unwrap());

    settings.set_theme_color(ThemeColor::Teal);
    cx.run_until_parked();
    assert!(told.has_changed().unwrap());
    assert_eq!(store.theme_color().unwrap(), ThemeColor::Teal);
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.theme_color(), ThemeColor::Teal);
    assert_eq!(snapshot.errors().count(), 0);
}

#[test]
fn a_color_that_cannot_be_read_says_so_and_leaves_the_appearance() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with("appearance = \"dark\"\ntheme_color = \"mauve\"\n");
    let snapshot = AppearanceSettings::new(cx.executor(), Ok(store)).snapshot();

    assert_eq!(snapshot.appearance(), Some(Appearance::Dark));
    assert_eq!(snapshot.theme_color(), ThemeColor::Cover);
    let errors: Vec<_> = snapshot.errors().collect();
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].starts_with("Couldn't read the color"),
        "{}",
        errors[0]
    );
}

#[test]
fn the_custom_color_stays_when_another_is_chosen() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with("theme_color = \"#4f5bd5\"\n");
    let settings = AppearanceSettings::new(cx.executor(), Ok(store.clone()));
    assert_eq!(settings.snapshot().custom_color(), Some(0x4f5bd5));

    settings.set_theme_color(ThemeColor::Custom(0x123456));
    settings.set_theme_color(ThemeColor::Teal);
    cx.run_until_parked();
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.theme_color(), ThemeColor::Teal);
    assert_eq!(snapshot.custom_color(), Some(0x123456));
    assert_eq!(store.theme_color().unwrap(), ThemeColor::Teal);
}
