//! A view follows the core with these rather than `cx.observe` and `cx.subscribe`: they run on the
//! next turn of the foreground executor, so a test runs `run_until_parked` before it looks.

mod changes;
mod executor;
mod notices;
#[cfg(test)]
mod testing;
mod window;

pub use changes::{notify_on_change, observe, observe_in};
pub use executor::executor;
pub use notices::{subscribe, subscribe_in};
pub use window::update_in_window;
