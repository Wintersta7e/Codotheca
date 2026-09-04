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
    #[must_use]
    pub fn new(value: String) -> Self {
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
    #[error("no system keychain is available")]
    Unavailable,
    #[error("no such keychain entry")]
    NotFound,
    #[error("the keychain refused the operation: {0}")]
    Backend(String),
}

/// The keychain seam. One production implementation, and one fake under `testkit`.
pub trait TokenStore: Send + Sync + std::fmt::Debug {
    /// Whether a keychain exists at all. Reads; never writes, so a probe leaves no entry behind.
    fn probe(&self) -> Result<(), KeychainError>;
    fn store(&self, entry: &str, token: &SecretToken) -> Result<(), KeychainError>;
    fn read(&self, entry: &str) -> Result<SecretToken, KeychainError>;
    fn delete(&self, entry: &str) -> Result<(), KeychainError>;
}

/// The keychain entry name for an account. `<provider>:<host>:<login>`, verbatim.
#[must_use]
pub fn token_ref(provider: &str, host: &str, login: &str) -> String {
    format!("{provider}:{host}:{login}")
}

/// The one production implementation, over the `keyring` crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringTokenStore;

impl KeyringTokenStore {
    #[must_use]
    pub fn new() -> Self {
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
fn classify(error: keyring::Error) -> KeychainError {
    match error {
        keyring::Error::NoEntry => KeychainError::NotFound,
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
            KeychainError::Unavailable
        }
        other => KeychainError::Backend(other.to_string()),
    }
}

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
