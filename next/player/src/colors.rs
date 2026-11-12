use std::cell::RefCell;
use std::rc::Rc;

use anti_webui::core::images::decode_bitmap;
use anti_webui::prelude::*;
use anti_webui_md3e::dynamic::seed_colors;
use next_host::{Pending, Scope};

use crate::cover::{COVER_PIXELS, Cover};

/// The seed of the scheme that `cover` gives, worked out off the UI thread. `None` while there is
/// no cover, or none that can be read.
pub fn cover_seed(cover: Signal<Option<Cover>>) -> Signal<Option<Color>> {
    let seed = signal(None::<Color>);
    let scope = Scope::here();
    let working: Rc<RefCell<Option<Pending>>> = Rc::default();
    effect(move || {
        let cover = cover.get();
        untrack(|| {
            if let Some(working) = working.borrow_mut().take() {
                working.cancel();
            }
            let Some(cover) = cover else {
                seed.set(None);
                return;
            };
            let image = cover.image();
            let found = next_host::executor().spawn_blocking(move || seed_of(image));
            *working.borrow_mut() = Some(scope.on_ui(found, move |found| seed.set(found)));
        });
    });
    seed
}

/// The color of `image` that makes the best seed. Takes milliseconds: call it off the UI thread.
pub fn seed_of(image: impl Into<ImageSource>) -> Option<Color> {
    let bitmap = decode_bitmap(image, (COVER_PIXELS, COVER_PIXELS)).ok()?;
    let mut pixels = Vec::new();
    // Premultiplied, which leaves the opaque pixels that the seed is taken from as they are
    let mut take = |bytes: &[u8]| pixels.extend(bytes.as_chunks::<4>().0.iter().copied());
    bitmap.with_levels(|level, _, bytes| {
        if level == 0 {
            take(bytes);
        }
    });
    // An animation has frames rather than levels
    if let Some(first) = bitmap.frame(0) {
        take(first);
    }
    if pixels.is_empty() {
        return None;
    }
    seed_colors(pixels, 1).first().copied()
}

#[cfg(test)]
mod tests;
