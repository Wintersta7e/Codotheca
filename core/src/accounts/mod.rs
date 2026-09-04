//! §20: accounts, credentials and the `accounts.*` command surface.
//!
//! A token reaches the database never. The database stores `token_ref`, the OS keychain stores
//! the secret, and the renderer holds neither.

pub mod keychain;
