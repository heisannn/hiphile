use std::sync::Arc;

use executor::{Executor, Task};
use futures::executor::block_on;

/// The image at `url`, from `covers` while they keep it and else from the server, which then
/// keeps it, or nothing when neither has it. Runs as blocking work, as the covers are on the disk.
pub(crate) fn cover_image(
    executor: &Executor,
    net: net::Client,
    covers: Option<catalog::CoverStore>,
    url: Arc<str>,
) -> Task<Option<Arc<[u8]>>> {
    executor.spawn_blocking(move || {
        block_on(session::cover_image(&net, covers.as_ref(), &url)).map(Arc::from)
    })
}
