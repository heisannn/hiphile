use anti_webui::core::reactive::create_child_owner;

use super::*;

#[test]
fn what_is_added_covers_while_it_is_open_and_its_owner_lives() {
    let ((), root) = create_root(|| {
        let overlays = Overlays::new();
        let dialog = signal(false);
        let ((), screen) = create_child_owner(|| overlays.add(move || dialog.get()));
        let any = memo(move || overlays.any());

        assert!(!any.get());
        dialog.set(true);
        assert!(any.get());
        screen.dispose();
        assert!(!any.get());
    });
    root.dispose();
}
