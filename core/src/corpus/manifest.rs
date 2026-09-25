//! The corpus inventory. Tests read this instead of hardcoding paths, so a fixture can be
//! renamed or moved in one place.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::corpus::{CorpusError, CorpusGit, CORPUS_VERSION};
use crate::mount::StoreClass;

/// The inventory a generated corpus writes to `<root>/manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusManifest {
    /// The `CORPUS_VERSION` that built the corpus; [`CorpusManifest::load`] rejects any other.
    pub corpus_version: u32,
    /// The version of the git that built it.
    pub git_version: String,
    /// The absolute corpus root.
    pub root: PathBuf,
    /// The two simulated storage devices.
    pub volumes: Vec<CorpusVolume>,
    /// One row per fixture, built or skipped, in build order.
    pub fixtures: Vec<CorpusFixture>,
}

/// A simulated storage device. The corpus lays fixtures out across two so the removable-drive
/// criterion has something to unplug.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusVolume {
    /// `vol-a` or `vol-b` — what a fixture's `volume` names.
    pub id: String,
    /// The volume's directory under the corpus root.
    pub path: PathBuf,
    /// The runtime device identity a fake mount resolver reports for paths on this volume.
    pub store_key: String,
    /// The persistent volume identity; unmounting it in a fake resolver is how a test unplugs
    /// the device.
    pub volume_key: String,
    /// Local for the fixed volume, removable for the one a test unplugs.
    pub class: StoreClass,
}

/// One fixture's manifest row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusFixture {
    /// The fixture's id — one of the `corpus::fixtures` constants.
    pub name: String,
    /// The id of the volume it lives on.
    pub volume: String,
    /// Absolute path to the repository root, worktree root, or bare directory.
    pub path: PathBuf,
    /// False where the platform refused — a symlink without the privilege, a fixture behind
    /// `--large`. A test that needs it must report the reason, never quietly pass.
    pub materialised: bool,
    /// Why the fixture was not built; `None` when it was.
    pub skip_reason: Option<String>,
    /// What the generator promises about it.
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
    /// A bare repository, with no working tree.
    pub bare: bool,
    /// A shallow clone.
    pub shallow: bool,
    /// `HEAD`'s commit id; `None` on an unborn `HEAD`.
    pub head_oid: Option<String>,
    /// Every root commit reachable from HEAD, in `rev-list --max-parents=0` order.
    pub root_oids: Vec<String>,
    /// How many commits `HEAD` reaches, where the fixture pins it.
    pub history_depth: Option<u32>,
    /// The `origin` remote's URL, where the fixture sets one.
    pub origin_url: Option<String>,
    /// For a submodule, the fixture it is a submodule of.
    pub parent_fixture: Option<String>,
    /// For a submodule, its path inside that parent.
    pub submodule_path: Option<String>,
    /// Whether `.git/index.lock` is held, so every git write refuses.
    pub index_lock_held: bool,
    /// Whether the fixture needs long-path support to exist on disk at all.
    pub requires_long_paths: bool,
    /// How many untracked files the worktree holds.
    pub untracked_files: u32,
    /// What the fixture stands for, or what it cannot prove, in words.
    pub notes: Option<String>,
}

impl CorpusManifest {
    /// Read `<dir>/manifest.json`.
    ///
    /// # Errors
    /// `CorpusError::Io` when the file cannot be read, and `CorpusError::Manifest` when it does
    /// not parse or a different `CORPUS_VERSION` wrote it.
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

    /// Write the manifest, pretty-printed, to `<dir>/manifest.json`.
    ///
    /// # Errors
    /// `CorpusError::Manifest` when it does not serialise, and `CorpusError::Io` when the file
    /// cannot be written.
    pub fn save(&self, dir: &Path) -> Result<(), CorpusError> {
        let path = dir.join("manifest.json");
        let text =
            serde_json::to_string_pretty(self).map_err(|e| CorpusError::Manifest(e.to_string()))?;
        std::fs::write(&path, text).map_err(|e| CorpusError::Io {
            path,
            message: e.to_string(),
        })
    }

    /// The row for `name`, materialised or not; a test wants [`CorpusManifest::require`].
    #[must_use]
    pub fn fixture(&self, name: &str) -> Option<&CorpusFixture> {
        self.fixtures.iter().find(|f| f.name == name)
    }

    /// The accessor a test should use: an absent or unmaterialised fixture is an error with a
    /// reason attached, never a skip nobody notices.
    ///
    /// # Errors
    /// `CorpusError::MissingFixture` when no row has that name, or when the row was not built —
    /// then carrying its skip reason.
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

    /// The volume whose id is `id`.
    #[must_use]
    pub fn volume(&self, id: &str) -> Option<&CorpusVolume> {
        self.volumes.iter().find(|v| v.id == id)
    }

    /// A hermetic git bound to this corpus's private config home.
    ///
    /// # Errors
    /// `CorpusError::Io` when the config home's directories cannot be created.
    pub fn git(&self) -> Result<CorpusGit, CorpusError> {
        CorpusGit::new(self.root.join("githome"))
    }
}
