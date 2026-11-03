//! Feature: the home screen.
//!
//! The way to something to listen to from the moment the app opens.

mod home_view;

pub use home_view::Home;

/// Key context of the focused home screen.
pub const HOME_CONTEXT: &str = "Home";
