//! §20.6's keychain seam, and the two rules that cannot be tested behaviourally.
//!
//! One of them — *no file fallback* — is a claim about code that does not exist, so it is
//! asserted by reading `core/src/accounts/keychain.rs` and printing the byte count read. A
//! source assertion whose passing run read zero bytes is a failing gate.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::accounts::keychain::{
    token_ref, KeychainError, KeyringTokenStore, SecretToken, TokenStore, KEYCHAIN_SERVICE,
};
use codotheca_core::testing::FakeTokenStore;

/// The module's own source. Read at run time rather than `include_str!`ed so the byte count the
/// test prints is the count it actually read off disk.
fn keychain_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/accounts/keychain.rs");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()));
    assert!(
        !text.is_empty(),
        "the source assertions below would pass vacuously against an empty string"
    );
    text
}

#[test]
fn the_entry_name_is_provider_host_login_and_the_service_is_codotheca() {
    assert_eq!(
        token_ref("github", "forge.example.invalid", "someone"),
        "github:forge.example.invalid:someone"
    );
    assert_eq!(KEYCHAIN_SERVICE, "codotheca");
}

#[test]
fn a_secret_token_does_not_print_itself() {
    let token = SecretToken::new("sentinel-not-a-real-token-0000".to_owned());
    let debug = format!("{token:?}");
    assert!(
        !debug.contains("sentinel"),
        "Debug leaked the token: {debug}"
    );
    assert_eq!(debug, "SecretToken(<redacted>)");
    // The one accessor still works, and its name is what a reviewer greps for.
    assert_eq!(token.expose(), "sentinel-not-a-real-token-0000");
}

/// A derive cannot be observed at run time, so this reads the source. `Serialize` on this type
/// would put a credential on the wire the moment any struct holding one was serialised.
#[test]
fn a_secret_token_carries_no_serde_and_no_display() {
    let text = keychain_source();
    eprintln!(
        "accounts_keychain: read {} bytes of core/src/accounts/keychain.rs",
        text.len()
    );

    // The attribute lines immediately above the declaration, and nothing else. A byte window
    // reaches into the doc comment above them, where the word `Serialize` appears in prose
    // *about this very rule* — the measured margin was 313 bytes, so roughly 113 bytes of
    // added prose would have turned this red for the wrong reason.
    let lines: Vec<&str> = text.lines().collect();
    let declaration = lines
        .iter()
        .position(|line| line.contains("pub struct SecretToken"))
        .expect("SecretToken is declared in this module");
    let attributes: Vec<&str> = lines[..declaration]
        .iter()
        .rev()
        .copied()
        .take_while(|line| line.trim_start().starts_with('#'))
        .collect();
    assert!(
        !attributes.is_empty(),
        "the declaration carries no attributes, so this scan read nothing"
    );
    for banned in ["Serialize", "Deserialize"] {
        assert!(
            !attributes.iter().any(|line| line.contains(banned)),
            "SecretToken derives {banned}, which puts a credential on the wire"
        );
    }
    assert!(
        !text.contains("impl std::fmt::Display for SecretToken"),
        "SecretToken has a Display impl, so it can be interpolated into any message"
    );
}

/// §20.6: no file fallback, no obfuscation, no plaintext. That is a claim about code that is
/// **absent**, and the only way to assert an absence is to read the module.
#[test]
fn the_keychain_module_touches_no_file() {
    let text = keychain_source();
    eprintln!(
        "accounts_keychain: scanned {} bytes for a filesystem fallback",
        text.len()
    );
    for banned in [
        "std::fs",
        "File::create",
        "write_all",
        "OpenOptions",
        "fs::write",
    ] {
        assert!(
            !text.contains(banned),
            "keychain.rs names {banned}: a file fallback is exactly what §20.6 forbids"
        );
    }
}

/// The headless machine. §20.6 makes this the expected shape of the failure, not a defect.
#[test]
fn an_unavailable_keychain_refuses_and_stores_nothing() {
    let store = FakeTokenStore::unavailable();
    assert!(matches!(store.probe(), Err(KeychainError::Unavailable)));

    let entry = token_ref("github", "forge.example.invalid", "someone");
    let stored = store.store(&entry, &SecretToken::new("sentinel-0000".to_owned()));
    assert!(
        matches!(stored, Err(KeychainError::Unavailable)),
        "a store against an unavailable keychain must refuse with that named reason"
    );
    assert!(
        store.entry_names().is_empty(),
        "nothing was written: {:?}",
        store.entry_names()
    );
    assert!(matches!(
        store.read(&entry),
        Err(KeychainError::Unavailable)
    ));
}

#[test]
fn an_available_keychain_round_trips_and_deletes() {
    let store = FakeTokenStore::available();
    let entry = token_ref("github", "forge.example.invalid", "someone");
    store
        .store(&entry, &SecretToken::new("sentinel-0000".to_owned()))
        .unwrap();

    let names = store.entry_names();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0], entry);
    assert!(store.holds(&entry, "sentinel-0000"));
    assert_eq!(store.read(&entry).unwrap().expose(), "sentinel-0000");

    store.delete(&entry).unwrap();
    assert!(store.entry_names().is_empty());
    assert!(matches!(store.read(&entry), Err(KeychainError::NotFound)));
    assert!(matches!(store.delete(&entry), Err(KeychainError::NotFound)));
}

/// R49: the trait and its production implementation land together. The seam is used as
/// `&dyn TokenStore`, so object safety is proved by construction rather than by a comment.
#[test]
fn the_production_store_exists_beside_the_trait_and_the_seam_is_object_safe() {
    let production: &dyn TokenStore = &KeyringTokenStore::new();
    let fake: &dyn TokenStore = &FakeTokenStore::available();
    // Both sides answer the same call. The production one is expected to report `Unavailable`
    // on a headless session, which is §20.6's own sentence and not a failure of this test.
    let _ = production.probe();
    fake.probe().unwrap();

    let text = keychain_source();
    eprintln!(
        "accounts_keychain: read {} bytes looking for the production impl",
        text.len()
    );
    assert!(
        text.contains("impl TokenStore for KeyringTokenStore"),
        "the trait has no production implementation in this module"
    );
}
