//! Test doubles for the three injection seams §15.2 requires. Compiled only under the
//! `testkit` feature, so none of it can reach a shipped binary.

mod clock;
mod git;
mod mount;
mod scan;

pub use clock::FakeClock;
pub use git::{FakeGitBackend, GitReply, RecordedGitCall, RecordingGitBackend};
pub use mount::FakeMountResolver;
pub use scan::{MemScanStore, ScanEventFake, ScanLauncherFake};
