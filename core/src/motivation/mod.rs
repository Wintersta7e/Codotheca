//! §43: the motivation layer's presence — the one read the status band draws, the line that
//! waits for a click, and quiet mode, the closed list that hides the game and keeps every fact.
//!
//! `keys` owns the `app_meta` keys §43 adds, one reader and one writer each. `state` builds the
//! band's one payload; `line` holds its queue and `reason` what a rise is owed to.

pub mod keys;
pub mod line;
pub mod reason;
pub mod state;
