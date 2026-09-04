//! A scripted `TokenStore`, for the flows that must be provable without a machine keychain.
//!
//! `KeyringTokenStore::probe` fails on a headless session, and §20.6 makes that the *expected*
//! shape of that failure rather than a defect — so every test of the connect path needs a store
//! whose availability it controls.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::accounts::keychain::{KeychainError, SecretToken, TokenStore};

/// An in-memory keychain with a scripted `probe`.
pub struct FakeTokenStore {
    available: bool,
    fail_store: bool,
    fail_delete: bool,
    entries: Mutex<BTreeMap<String, String>>,
}

impl Default for FakeTokenStore {
    fn default() -> Self {
        Self::available()
    }
}

impl FakeTokenStore {
    #[must_use]
    pub fn available() -> Self {
        Self {
            available: true,
            fail_store: false,
            fail_delete: false,
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// The headless machine: `probe` reports unavailable and nothing may be written.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            available: false,
            ..Self::available()
        }
    }

    /// A keychain that exists and refuses to write. §20's ordering rule turns on this case:
    /// a failed store must leave no `account` row behind.
    #[must_use]
    pub fn refusing_store() -> Self {
        Self {
            fail_store: true,
            ..Self::available()
        }
    }

    /// A keychain that refuses to delete. §20.9's disconnect must fail and keep the row.
    #[must_use]
    pub fn refusing_delete() -> Self {
        Self {
            fail_delete: true,
            ..Self::available()
        }
    }

    /// The entry **names** it holds, sorted. Never the secrets: a helper returning those would
    /// be the first place a token reached a test's failure output.
    #[must_use]
    pub fn entry_names(&self) -> Vec<String> {
        self.guard().keys().cloned().collect()
    }

    /// Whether `entry` holds exactly `expected`. A predicate rather than a getter, for the same
    /// reason `entry_names` returns no values.
    #[must_use]
    pub fn holds(&self, entry: &str, expected: &str) -> bool {
        self.guard().get(entry).is_some_and(|v| v == expected)
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, String>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Entry names only. A derived `Debug` would print the map's values, which are the secrets.
///
/// `missing_fields_in_debug` is allowed deliberately: omitting the map's *values* is the whole
/// point of writing this by hand, and `finish_non_exhaustive` would say the same thing while
/// still inviting someone to "complete" it later.
#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for FakeTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeTokenStore")
            .field("available", &self.available)
            .field("fail_store", &self.fail_store)
            .field("fail_delete", &self.fail_delete)
            .field("entries", &self.entry_names())
            .finish()
    }
}

impl TokenStore for FakeTokenStore {
    fn probe(&self) -> Result<(), KeychainError> {
        if self.available {
            Ok(())
        } else {
            Err(KeychainError::Unavailable)
        }
    }

    fn store(&self, entry: &str, token: &SecretToken) -> Result<(), KeychainError> {
        self.probe()?;
        if self.fail_store {
            return Err(KeychainError::Backend("scripted store failure".to_owned()));
        }
        self.guard()
            .insert(entry.to_owned(), token.expose().to_owned());
        Ok(())
    }

    fn read(&self, entry: &str) -> Result<SecretToken, KeychainError> {
        self.probe()?;
        self.guard()
            .get(entry)
            .map(|v| SecretToken::new(v.clone()))
            .ok_or(KeychainError::NotFound)
    }

    fn delete(&self, entry: &str) -> Result<(), KeychainError> {
        self.probe()?;
        if self.fail_delete {
            return Err(KeychainError::Backend("scripted delete failure".to_owned()));
        }
        if self.guard().remove(entry).is_none() {
            return Err(KeychainError::NotFound);
        }
        Ok(())
    }
}
