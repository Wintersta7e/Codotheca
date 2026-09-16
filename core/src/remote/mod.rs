//! §25 — the remote half of a project, over facts somebody else fetched.
//!
//! **The core owns every decision; the renderer owns every string.** Four files and nothing else:
//! [`weburl`] holds the host allowlist and the one place a URL is constructed, [`facts`] assembles
//! `RemoteFacts` out of the three `remote_*` tables plus the account state, [`store`] writes those
//! tables under the observation-clock rules, and [`backup`] is §25.3's backup-state producer —
//! one declaration with two consumers, one of them in another process.
//!
//! Nothing in here renders. `RemoteFacts.state` is a wire enum precisely so the four states are a
//! decision the core made rather than a rule the renderer re-derives; staleness is the one thing
//! computed at render time, because it is a function of *now*.

pub mod facts;
pub mod store;
pub mod weburl;

use crate::index::Index;
use crate::proto::dispatch::CommandFailure;

/// Everything §25's commands need. `now` is unix **seconds**, supplied by the caller so no
/// handler reads the clock itself.
#[derive(Debug)]
pub struct RemoteCtx<'a> {
    pub index: &'a Index,
    pub now: i64,
}

/// The commands this module owns, as data, so the seam and the table cannot drift apart — the
/// shape of the defect R37 found between the schema and the handlers.
pub const REMOTE_COMMANDS: [&str; 1] = ["remote.webUrl"];

/// `None` means "this module does not own that command", which is what the router chains on.
#[must_use]
pub fn dispatch_remote_command(
    ctx: &RemoteCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Option<Result<serde_json::Value, CommandFailure>> {
    match command {
        "remote.webUrl" => Some(weburl::handle_web_url(ctx, args)),
        _ => None,
    }
}
