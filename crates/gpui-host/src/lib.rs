//! Depends on nothing of the app but `executor`, so that a core crate can test on GPUI's executor
//! without a cycle through `ui`, which depends on `hiphile-core`. A view follows the core with
//! these rather than `cx.observe` and `cx.subscribe`: they run on the next turn of the foreground
//! executor, so a test runs `run_until_parked` before it looks.

mod changes;
mod executor;
mod notices;

pub use changes::{notify_on_change, observe, observe_in};
pub use executor::executor;
pub use notices::{subscribe, subscribe_in};
