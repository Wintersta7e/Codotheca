//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature
//! gates. Without the gate a bare `cargo test` fails to compile rather than skipping, and the
//! real gate — `npm test`, and CI — passes `--features testkit`. `app/test/toolchain.test.ts`
//! asserts that flag is still there, so it cannot be dropped silently.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use codotheca_core::mount::{MountError, MountFacts, MountResolver, StoreClass};
use codotheca_core::testing::FakeMountResolver;

fn facts(store: &str, volume: &str, class: StoreClass) -> MountFacts {
    MountFacts {
        store_key: store.to_owned(),
        volume_key: Some(volume.to_owned()),
        class,
    }
}

fn resolver() -> FakeMountResolver {
    let r = FakeMountResolver::new();
    r.map(
        PathBuf::from("/corpus"),
        facts("store-a", "volume-a", StoreClass::Local),
    );
    r.map(
        PathBuf::from("/corpus/vol-b"),
        facts("store-b", "volume-b", StoreClass::Removable),
    );
    r
}

#[test]
fn slow_stores_get_one_slot_and_everything_else_gets_four() {
    assert_eq!(StoreClass::Network.per_store_cap(), 1);
    assert_eq!(StoreClass::Hdd.per_store_cap(), 1);
    assert_eq!(StoreClass::Fuse.per_store_cap(), 1);
    assert_eq!(StoreClass::Removable.per_store_cap(), 1);
    assert_eq!(StoreClass::Local.per_store_cap(), 4);
    assert_eq!(StoreClass::Unknown.per_store_cap(), 4);
}

#[test]
fn the_longest_matching_prefix_wins() {
    let r = resolver();
    let outer = r.resolve(Path::new("/corpus/vol-a/repo")).unwrap();
    assert_eq!(outer.store_key, "store-a");
    let inner = r.resolve(Path::new("/corpus/vol-b/repo")).unwrap();
    assert_eq!(inner.store_key, "store-b");
    assert_eq!(inner.class, StoreClass::Removable);
}

#[test]
fn an_unmapped_path_is_unsupported_not_silently_local() {
    let r = resolver();
    let err = r.resolve(Path::new("/elsewhere/repo")).unwrap_err();
    assert!(matches!(err, MountError::Unsupported(_)), "got {err:?}");
}

#[test]
fn unmounting_a_volume_makes_its_paths_unresolvable_and_replugging_restores_them() {
    let r = resolver();
    assert!(r.is_volume_mounted("volume-b"));
    r.unmount("volume-b");
    assert!(!r.is_volume_mounted("volume-b"));
    assert_eq!(
        r.resolve(Path::new("/corpus/vol-b/repo")),
        Err(MountError::NotMounted)
    );
    // The sibling volume is untouched: unplugging one drive is not a global outage.
    assert!(r.resolve(Path::new("/corpus/vol-a/repo")).is_ok());
    r.mount("volume-b");
    assert!(r.resolve(Path::new("/corpus/vol-b/repo")).is_ok());
}

#[test]
fn the_fake_is_usable_through_the_trait_object() {
    let r: Box<dyn MountResolver> = Box::new(resolver());
    assert!(r.resolve(Path::new("/corpus/vol-a")).is_ok());
}

#[test]
fn mount_facts_round_trip_under_serde_with_the_column_names_intact() {
    // R7: these three fields are `location` columns and they cross to the WSL worker.
    let before = facts("store-a", "volume-a", StoreClass::Removable);
    let text = serde_json::to_string(&before).unwrap();
    assert!(
        text.contains("\"store_key\""),
        "field names are column names: {text}"
    );
    assert!(
        text.contains("\"volume_key\""),
        "field names are column names: {text}"
    );
    let after: MountFacts = serde_json::from_str(&text).unwrap();
    assert_eq!(after, before);

    // A store with no stable identifier must survive the crossing as absent, not as "".
    let anonymous = MountFacts {
        store_key: "store-x".to_owned(),
        volume_key: None,
        class: StoreClass::Fuse,
    };
    let back: MountFacts =
        serde_json::from_str(&serde_json::to_string(&anonymous).unwrap()).unwrap();
    assert_eq!(back.volume_key, None);
}

#[test]
fn the_system_resolver_answers_for_the_current_directory() {
    use codotheca_core::mount::SystemMountResolver;
    let r = SystemMountResolver::new();
    let cwd = std::env::current_dir().unwrap();
    let facts = r.resolve(&cwd).unwrap();
    assert!(
        !facts.store_key.is_empty(),
        "a resolved store must always carry a key"
    );
}

#[test]
fn unplugging_a_corpus_volume_hides_its_repositories_and_deletes_nothing() {
    use codotheca_core::corpus::fixtures as ids;
    use codotheca_core::corpus::{generate, CorpusOptions};

    let dir = std::env::temp_dir().join("codotheca-corpus-test-unplug");
    let mut options = CorpusOptions::new(dir);
    options.only = Some(vec![ids::COPY_ONE.to_owned(), ids::COPY_TWO.to_owned()]);
    let manifest = generate(&options).unwrap();

    let resolver = FakeMountResolver::from_manifest(&manifest);
    let one = manifest.require(ids::COPY_ONE).unwrap();
    let two = manifest.require(ids::COPY_TWO).unwrap();
    let removable = manifest.volume("vol-b").unwrap().volume_key.clone();

    assert!(resolver.resolve(&one.path).is_ok());
    assert!(resolver.resolve(&two.path).is_ok());

    resolver.unmount(&removable);
    assert_eq!(resolver.resolve(&two.path), Err(MountError::NotMounted));
    assert!(
        resolver.resolve(&one.path).is_ok(),
        "the other volume is unaffected"
    );
    assert!(
        two.path.join(".git").is_dir(),
        "an offline location is frozen, never deleted"
    );

    resolver.mount(&removable);
    assert!(resolver.resolve(&two.path).is_ok());
}
