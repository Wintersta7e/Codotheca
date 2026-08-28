use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::mount::{MountError, MountFacts, MountResolver};

/// A mount table the test writes.
///
/// Prefixes are matched longest-first, so a volume nested inside another volume's directory
/// resolves to the inner one — which is exactly the corpus's two-volume layout.
#[derive(Debug, Default)]
pub struct FakeMountResolver {
    entries: Mutex<Vec<(PathBuf, MountFacts)>>,
    unmounted: Mutex<HashSet<String>>,
}

impl FakeMountResolver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Map every path under `prefix` to `facts`. Returns `&Self` so calls chain.
    pub fn map(&self, prefix: impl Into<PathBuf>, facts: MountFacts) -> &Self {
        if let Ok(mut entries) = self.entries.lock() {
            entries.push((prefix.into(), facts));
            entries.sort_by_key(|(p, _)| std::cmp::Reverse(p.components().count()));
        }
        self
    }

    /// Pull the drive out. Paths on it resolve to `NotMounted`; nothing on disk is touched.
    pub fn unmount(&self, volume_key: &str) {
        if let Ok(mut set) = self.unmounted.lock() {
            set.insert(volume_key.to_owned());
        }
    }

    /// Plug it back in.
    pub fn mount(&self, volume_key: &str) {
        if let Ok(mut set) = self.unmounted.lock() {
            set.remove(volume_key);
        }
    }
}

impl MountResolver for FakeMountResolver {
    fn resolve(&self, path: &Path) -> Result<MountFacts, MountError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| MountError::Io("poisoned".to_owned()))?;
        let found = entries
            .iter()
            .find(|(prefix, _)| path.starts_with(prefix))
            .map(|(_, facts)| facts.clone());
        drop(entries);
        match found {
            None => Err(MountError::Unsupported(path.display().to_string())),
            Some(facts) => match facts.volume_key.as_deref() {
                Some(key) if !self.is_volume_mounted(key) => Err(MountError::NotMounted),
                _ => Ok(facts),
            },
        }
    }

    fn is_volume_mounted(&self, volume_key: &str) -> bool {
        self.unmounted
            .lock()
            .map_or(true, |set| !set.contains(volume_key))
    }
}

use crate::corpus::CorpusManifest;

impl FakeMountResolver {
    /// A mount table matching the corpus's simulated volumes. Every fixture path resolves to
    /// the volume it was written on, and `unmount` then makes exactly those paths disappear.
    #[must_use]
    pub fn from_manifest(manifest: &CorpusManifest) -> Self {
        let resolver = Self::new();
        for volume in &manifest.volumes {
            resolver.map(
                volume.path.clone(),
                MountFacts {
                    store_key: volume.store_key.clone(),
                    volume_key: Some(volume.volume_key.clone()),
                    class: volume.class,
                },
            );
        }
        resolver
    }
}
