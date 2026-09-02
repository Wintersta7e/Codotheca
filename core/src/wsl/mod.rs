//! Windows Subsystem for Linux integration.
//!
//! §13 — the WSL worker. The worker is a second binary: a Linux ELF that runs *inside* a distro
//! and speaks the same length-prefixed frames the core speaks to the shell. It exists because
//! §4.5 forbids walking the `\\wsl$` bridge — every `stat` across it is a round trip through a
//! virtual machine — and because spawning `wsl.exe` per repository, or per git command, costs
//! more than the work.

pub mod distros;
pub mod mounts;
pub mod path;
pub mod proto;
