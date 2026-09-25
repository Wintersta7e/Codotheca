//! §20.6: the OS keychain, and the type that makes redaction structural.
//!
//! **The database stores `token_ref` and never a token.** The service is `codotheca` and the
//! entry name is `<provider>:<host>:<login>`, stored verbatim in `account.token_ref`.
//!
//! **No keychain, no connection.** If the keychain is unavailable, connect fails with that named
//! reason and stores nothing: no file fallback, no obfuscation, no plaintext, no "weaker but
//! working" path. A headless session commonly has no Secret Service running, and that is the
//! expected shape of this failure rather than a defect — it reads
//! `NO SYSTEM KEYCHAIN · NOTHING WAS STORED`, which is a statement about the machine and not
//! about the user's token.
//!
//! **This module performs no filesystem operation of any kind**, and `core/tests/accounts_keychain.rs`
//! asserts that by reading this source. That is what makes "no file fallback" a gate rather than
//! an intention.

/// §20.6's service name. One value, stated here.
pub const KEYCHAIN_SERVICE: &str = "codotheca";

/// The entry `probe` reads. It is never written, so a probe neither creates nor deletes.
const PROBE_ENTRY: &str = "codotheca-keychain-probe";

/// A token, and the only way to get at one is a method whose name is greppable.
///
/// Manual `Debug`, no `Display`, and deliberately **no** `Serialize` or `Deserialize`: a derive
/// here would put a credential on the wire the moment any struct holding one was serialised.
///
/// **Zeroing on drop is not attempted.** It needs `unsafe` or a further dependency and this
/// crate forbids `unsafe`. Recorded rather than silently omitted.
#[derive(Clone)]
pub struct SecretToken(String);

impl SecretToken {
    /// Wrap a token so that nothing but [`Self::expose`] can read it back.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// The one accessor. Named so a reviewer can grep every place a secret is read.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretToken(<redacted>)")
    }
}

/// Three outcomes, and only one carries text.
///
/// `Unavailable` and `NotFound` are unit variants on purpose: neither has anything to say that
/// is not already in the variant name, and every string that crosses this boundary is a string
/// §20.10's redaction audit has to clear.
#[derive(Debug, thiserror::Error)]
pub enum KeychainError {
    /// The machine has no reachable keychain, e.g. a headless session with no Secret Service.
    #[error("no system keychain is available")]
    Unavailable,
    /// The keychain works and holds nothing under that entry name.
    #[error("no such keychain entry")]
    NotFound,
    /// The keychain refused for another reason; the text is the backend's, never a secret.
    #[error("the keychain refused the operation: {0}")]
    Backend(String),
}

/// The keychain seam. One production implementation, and one fake under `testkit`.
pub trait TokenStore: Send + Sync + std::fmt::Debug {
    /// Whether a keychain exists at all. Reads; never writes, so a probe leaves no entry behind.
    ///
    /// # Errors
    ///
    /// `KeychainError::Unavailable` when there is no keychain, `KeychainError::Backend` when it
    /// refuses the read. An absent probe entry is success.
    fn probe(&self) -> Result<(), KeychainError>;
    /// Put `token` under `entry`, replacing whatever was there.
    ///
    /// # Errors
    ///
    /// `KeychainError::Unavailable` when there is no keychain, `KeychainError::Backend` when it
    /// refuses the write.
    fn store(&self, entry: &str, token: &SecretToken) -> Result<(), KeychainError>;
    /// The token stored under `entry`.
    ///
    /// # Errors
    ///
    /// `KeychainError::NotFound` when nothing is stored there, `KeychainError::Unavailable` when
    /// there is no keychain, `KeychainError::Backend` when it refuses the read.
    fn read(&self, entry: &str) -> Result<SecretToken, KeychainError>;
    /// Remove the token stored under `entry`.
    ///
    /// # Errors
    ///
    /// `KeychainError::NotFound` when nothing is stored there, `KeychainError::Unavailable` when
    /// there is no keychain, `KeychainError::Backend` when it refuses the delete.
    fn delete(&self, entry: &str) -> Result<(), KeychainError>;
}

/// The keychain entry name for an account. `<provider>:<host>:<login>`, verbatim.
#[must_use]
pub fn token_ref(provider: &str, host: &str, login: &str) -> String {
    format!("{provider}:{host}:{login}")
}

/// The one production implementation, over the `keyring` crate.
///
/// Behind the default-on `keychain` feature, and everything above it — the trait, the token
/// type, the error vocabulary, `token_ref` — is not. §13's WSL worker is built from this same
/// crate and holds no credentials of its own, so it is built **without** the feature: `keyring`'s
/// Linux backend is `libdbus-sys`, which needs system headers to compile and an arm64 sysroot to
/// cross-compile, and a 67-line helper copied into a distro has no business carrying either.
#[cfg(feature = "keychain")]
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringTokenStore;

#[cfg(feature = "keychain")]
impl KeyringTokenStore {
    /// The store over the OS keychain's `codotheca` service; it holds no state of its own.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    fn entry(name: &str) -> Result<keyring::Entry, KeychainError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, name).map_err(classify)
    }
}

/// `keyring`'s error vocabulary into this module's three.
///
/// `NoEntry` is *not* a broken keychain: it is a working one with nothing under that name, which
/// is why `probe` treats it as success.
///
/// **`other.to_string()` interpolates the upstream crate's own text, so what that text can
/// contain is a checked fact and not an assumption.** Read off `keyring` 3.6.3's `Display`
/// (`error.rs:61-86`): the two variants carrying a boxed platform error are classified above and
/// never reach here; of the four that do, `BadEncoding` prints *"Data is not UTF-8 encoded"* and
/// not its bytes, `TooLong` and `Invalid` print an attribute **name**, and `Ambiguous` prints the
/// matching credentials' attributes — which are the `token_ref` the database already holds, never
/// a password. **Re-check this on any `keyring` upgrade**: it is the crate's rendering, not ours.
#[cfg(feature = "keychain")]
fn classify(error: keyring::Error) -> KeychainError {
    match error {
        keyring::Error::NoEntry => KeychainError::NotFound,
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
            KeychainError::Unavailable
        }
        other => KeychainError::Backend(other.to_string()),
    }
}

#[cfg(feature = "keychain")]
impl TokenStore for KeyringTokenStore {
    fn probe(&self) -> Result<(), KeychainError> {
        let entry = Self::entry(PROBE_ENTRY)?;
        match entry.get_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(classify(error)),
        }
    }

    fn store(&self, entry: &str, token: &SecretToken) -> Result<(), KeychainError> {
        Self::entry(entry)?
            .set_password(token.expose())
            .map_err(classify)
    }

    fn read(&self, entry: &str) -> Result<SecretToken, KeychainError> {
        Self::entry(entry)?
            .get_password()
            .map(SecretToken::new)
            .map_err(classify)
    }

    fn delete(&self, entry: &str) -> Result<(), KeychainError> {
        Self::entry(entry)?.delete_credential().map_err(classify)
    }
}
