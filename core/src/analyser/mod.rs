//! §45's deletion analyser: the one function that decides whether a governed act may destroy a
//! byte, and every construction of an `UninstallBlocker`.
//!
//! This change lands its network seam first — [`remote`], the verifying read of one configured
//! remote — because `Intent::Fetch` retires in the same change and its one caller needs a
//! replacement. The analyser's steps follow in the changes that make each of them true.

pub mod remote;
