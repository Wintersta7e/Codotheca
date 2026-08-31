//! The command layer: one module per §2.4 command group. Each exposes a
//! `dispatch_*_command` that returns `None` for a command it does not own, so the assembling
//! `CommandHandler` can try them in turn.

pub mod launch;
pub mod targets;
