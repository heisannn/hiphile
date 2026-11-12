//! Signing in, and switching between the servers signed in with.

mod form;
mod sign_in;
mod sign_in_dialog;

pub use sign_in::sign_in;
pub use sign_in_dialog::{SignInRequest, sign_in_dialog};
