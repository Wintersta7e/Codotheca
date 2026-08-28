//! The corpus inventory. Tests read this instead of hardcoding paths, so a fixture can be
//! renamed or moved in one place.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::corpus::{CorpusError, CorpusGit, CORPUS_VERSION};
use crate::mount::StoreClass;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusManifest {
    pub corpus_version: u32,
    pub git_version: String,
    pub root: PathBuf,
    pub volumes: Vec<CorpusVolume>,
    pub fixtures: Vec<CorpusFixture>,
}

/// A simulated storage device. The corpus lays fixtures out across two so the removable-drive
/// criterion has something to unplug.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusVolume {
    pub id: String,
    pub path: PathBuf,
    pub store_key: String,
    pub volume_key: String,
    pub class: StoreClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusFixture {
    pub name: String,
    pub volume: String,
    /// Absolute path to the repository root, worktree root, or bare directory.
    pub path: PathBuf,
    /// False where the platform refused — a symlink without the privilege, a fixture behind
    /// `--large`. A test that needs it must report the reason, never quietly pass.
    pub materialised: bool,
    pub skip_reason: Option<String>,
    pub expect: FixtureExpect,
}

/// What the generator promises about a fixture. Every field defaults, so an older manifest
/// deserialises and a newer field is simply absent rather than fatal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
// Four booleans, one per independent property of a repository; collapsing them into an enum
// would claim they are mutually exclusive, and a bare repository can also be shallow.
#[allow(clippy::struct_excessive_bools)]
pub struct FixtureExpect {
    pub bare: bool,
    pub shallow: bool,
    pub head_oid: Option<String>,
    /// Every root commit reachable from HEAD, in `rev-list --max-parents=0` order.
    pub root_oids: Vec<String>,
    pub history_depth: Option<u32>,
    pub origin_url: Option<String>,
    pub parent_fixture: Option<String>,
    pub submodule_path: Option<String>,
    pub index_lock_held: bool,
    pub requires_long_paths: bool,
    pub untracked_files: u32,
    pub notes: Option<String>,
}

impl CorpusManifest {
    /// Read `<dir>/manifest.json`.
    pub fn load(dir: &Path) -> Result<Self, CorpusError> {
        let path = dir.join("manifest.json");
        let text = std::fs::read_to_string(&path).map_err(|e| CorpusError::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
        let manifest: Self =
            serde_json::from_str(&text).map_err(|e| CorpusError::Manifest(e.to_string()))?;
        if manifest.corpus_version != CORPUS_VERSION {
            return Err(CorpusError::Manifest(format!(
                "corpus version {} on disk, {CORPUS_VERSION} expected",
                manifest.corpus_version
            )));
        }
        Ok(manifest)
    }

    pub fn save(&self, dir: &Path) -> Result<(), CorpusError> {
        let path = dir.join("manifest.json");
        let text =
            serde_json::to_string_pretty(self).map_err(|e| CorpusError::Manifest(e.to_string()))?;
        std::fs::write(&path, text).map_err(|e| CorpusError::Io {
            path,
            message: e.to_string(),
        })
    }

    #[must_use]
    pub fn fixture(&self, name: &str) -> Option<&CorpusFixture> {
        self.fixtures.iter().find(|f| f.name == name)
    }

    /// The accessor a test should use: an absent or unmaterialised fixture is an error with a
    /// reason attached, never a skip nobody notices.
    pub fn require(&self, name: &str) -> Result<&CorpusFixture, CorpusError> {
        let fixture = self
            .fixture(name)
            .ok_or_else(|| CorpusError::MissingFixture(name.to_owned()))?;
        if !fixture.materialised {
            let reason = fixture
                .skip_reason
                .clone()
                .unwrap_or_else(|| "unknown".to_owned());
            return Err(CorpusError::MissingFixture(format!("{name}: {reason}")));
        }
        Ok(fixture)
    }

    #[must_use]
    pub fn volume(&self, id: &str) -> Option<&CorpusVolume> {
        self.volumes.iter().find(|v| v.id == id)
    }

    /// A hermetic git bound to this corpus's private config home.
    pub fn git(&self) -> Result<CorpusGit, CorpusError> {
        CorpusGit::new(self.root.join("githome"))
    }
}
