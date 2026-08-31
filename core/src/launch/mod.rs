//! Launch targets: discovery, ranking, resolution, verification and the spawn.

pub mod catalogue;
pub mod probe;
pub mod wslpath;

pub use catalogue::{CwdMode, TargetKind};

// R3: no `now_secs` helper. `Clock::now_unix()` is already epoch seconds, which is what every
// `session`, `session_segment` and `launch_target` column stores. Call it directly.
