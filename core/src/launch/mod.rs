//! Launch targets: discovery, ranking, resolution, verification and the spawn.

pub mod catalogue;
pub mod probe;
#[cfg(not(windows))]
pub mod probe_linux;
// Declared unconditionally on purpose: the Shell Link and shell-verb parsers are platform-free,
// so they compile and are tested everywhere while only the registry walk is `#[cfg(windows)]`.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod probe_windows;
pub mod recents;
pub mod wslpath;

pub use catalogue::{CwdMode, TargetKind};

// R3: no `now_secs` helper. `Clock::now_unix()` is already epoch seconds, which is what every
// `session`, `session_segment` and `launch_target` column stores. Call it directly.
