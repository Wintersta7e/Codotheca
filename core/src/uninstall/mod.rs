//! §24.6–§24.8: Uninstall. **This module declares no removal primitive** — `core/src/removal/`
//! is p2-24's and this plan extends it with one warrant variant rather than a second module.

pub mod stash;
pub mod verdict;

pub use stash::{read_stash_truth, StashTruth};
pub use verdict::{fold_disposition, is_unknown_blocker, VerdictSeal, ALL_BLOCKERS};
