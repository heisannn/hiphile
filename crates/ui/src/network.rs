use std::sync::Arc;

use gpui_kit::App;

use crate::http::NetHttpClient;

/// Installs `net` as GPUI's HTTP client, so `img(url)` cover art shares the API's transport and
/// is kept in `covers`, when there is somewhere to keep it. Call once at startup, after
/// `gpui_kit::init`.
pub fn set_http_client(net: net::Client, covers: Option<catalog::CoverStore>, cx: &mut App) {
    cx.set_http_client(Arc::new(NetHttpClient::new(net, covers)));
}
