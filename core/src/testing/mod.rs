//! Test doubles for the three injection seams §15.2 requires. Compiled only under the
//! `testkit` feature, so none of it can reach a shipped binary.

mod bins;
mod clock;
mod git;
mod gitw;
mod http;
mod index;
mod mount;
mod remote;
mod scan;
mod tokens;
pub mod wsl;

pub use bins::CountingTrash;
pub use clock::FakeClock;
pub use git::{FakeGitBackend, GitReply, RecordedGitCall, RecordingGitBackend};
pub use gitw::{CloneBehaviour, FakeMutatingGit};
pub use http::FakeTransport;
pub use index::TempIndex;
pub use mount::FakeMountResolver;
pub use remote::FixtureRemoteVerifier;
pub use scan::{MemScanStore, ScanEventFake, ScanLauncherFake};
pub use tokens::FakeTokenStore;
