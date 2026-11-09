use gpui_kit::{TestAppContext, VisualTestContext};

use super::*;

struct Fixture<'a> {
    limit: CacheLimit,
    field: Entity<CacheLimitField>,
    cx: &'a mut VisualTestContext,
}

fn fixture(cx: &mut TestAppContext) -> Fixture<'_> {
    cx.update(gpui_kit::init);
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let limit = CacheLimit::new(CacheKind::Covers, executor, Err(config::Error::NoHomeDir));
    let held = limit.clone();
    let (field, cx) = cx.add_window_view(|window, cx| CacheLimitField::new(held, window, cx));
    Fixture { limit, field, cx }
}

impl Fixture<'_> {
    fn shown(&mut self) -> String {
        self.field.read_with(self.cx, |field, cx| {
            field.input.read(cx).value().to_string()
        })
    }

    fn type_text(&mut self, text: &str) {
        let text = text.to_owned();
        self.field.update_in(self.cx, |field, window, cx| {
            field
                .input
                .update(cx, |input, cx| input.replace_all(text, window, cx));
        });
    }

    fn blur(&mut self) {
        self.field.update_in(self.cx, |field, _, cx| {
            field.input.update(cx, |_, cx| cx.emit(InputEvent::Blur));
        });
        self.cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn leaving_the_field_shows_the_limit_as_it_counts(cx: &mut TestAppContext) {
    let mut it = fixture(cx);
    assert_eq!(it.shown(), "8");

    it.type_text("007");
    assert_eq!(it.limit.snapshot().gib(), 7.0);
    assert_eq!(it.shown(), "007");
    it.blur();
    assert_eq!(it.shown(), "7");

    it.type_text("");
    it.blur();
    assert_eq!(it.shown(), "8");
}

#[gpui_kit::test]
fn reset_shows_the_default(cx: &mut TestAppContext) {
    let mut it = fixture(cx);
    it.type_text("3");
    it.field
        .read_with(it.cx, |field, _| assert!(field.is_resettable()));

    it.field
        .update_in(it.cx, |field, window, cx| field.reset(window, cx));

    assert_eq!(it.shown(), "8");
    it.field
        .read_with(it.cx, |field, _| assert!(!field.is_resettable()));
    it.cx.run_until_parked();
}
