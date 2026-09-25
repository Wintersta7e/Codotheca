//! §4bis.1's discovery seam: what the host has installed, from every source, de-duplicated.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::launch::catalogue::{lookup, CatalogueEntry};

/// Where a probe found an application; [`ProbeSource::rank`] orders them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeSource {
    /// A Windows App Paths registry key.
    Registry,
    /// A Linux desktop entry's `Exec=` line.
    Desktop,
    /// The target of a Windows Start Menu shortcut.
    StartMenu,
    /// An IDE vendor's toolbox scripts directory.
    Toolbox,
    /// An installed flatpak, started through `flatpak run`.
    Flatpak,
    /// An installed snap, under `/snap/bin`.
    Snap,
    /// A Windows package manager's shim directory.
    Shim,
    /// The OS's registered handler for folders.
    OsHandler,
    /// An executable inside a WSL distro (§4bis.1).
    Distro,
    /// A directory on `PATH`.
    Path,
}

impl ProbeSource {
    /// Higher wins when two sources name one executable.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Registry | Self::Desktop => 5,
            Self::StartMenu | Self::Toolbox => 4,
            Self::Flatpak | Self::Snap => 3,
            Self::Shim | Self::OsHandler => 2,
            Self::Distro | Self::Path => 1,
        }
    }
}

/// One installed application a probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbedApp {
    /// The executable found for it.
    pub exec: PathBuf,
    /// The executable's file stem, which the catalogue is matched on.
    pub stem: String,
    /// Where it was found; decides which of two sightings of one executable survives.
    pub source: ProbeSource,
    /// `Some` only for a target that lives inside a WSL distro (§4bis.1).
    pub distro: Option<String>,
    /// Executable mtime, seconds. Step 5 of §4bis.2's ranking.
    pub installed_at: Option<i64>,
}

/// Everything one probe of the host found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbeFacts {
    /// The applications found. The system probes keep catalogued ones only, de-duplicated.
    pub apps: Vec<ProbedApp>,
    /// `$EDITOR` or `$VISUAL`, reduced to a stem. Step 3.
    pub editor_env: Option<String>,
    /// The OS default handler for a folder, reduced to a stem. Step 4.
    pub folder_handler_stem: Option<String>,
}

/// §4bis.1's discovery seam: what the host has installed.
pub trait TargetProbe: Send + Sync + std::fmt::Debug {
    /// Everything this probe finds, from all of its sources.
    fn probe(&self) -> ProbeFacts;
}

/// The probe for the platform this build runs on.
#[must_use]
pub fn system_probe() -> Box<dyn TargetProbe> {
    #[cfg(windows)]
    {
        Box::new(crate::launch::probe_windows::WindowsProbe::new())
    }
    #[cfg(not(windows))]
    {
        Box::new(crate::launch::probe_linux::LinuxProbe::new())
    }
}

/// One entry per executable per distro: the highest-ranked source's sighting, the first on a
/// tie. The result is in key order, not input order.
#[must_use]
pub fn dedupe(apps: Vec<ProbedApp>) -> Vec<ProbedApp> {
    let mut best: BTreeMap<(Option<String>, Vec<u8>), ProbedApp> = BTreeMap::new();
    for app in apps {
        // R2: the executable lives on the host being probed, so native comparison rules apply.
        let key = (
            app.distro.clone(),
            crate::paths::path_key(&app.exec, crate::index::path::native_platform()),
        );
        match best.get(&key) {
            Some(existing) if existing.source.rank() >= app.source.rank() => {}
            _ => {
                best.insert(key, app);
            }
        }
    }
    best.into_values().collect()
}

/// Drops anything the catalogue does not recognise: a target we cannot build an argv for is
/// not a target, and §4bis.5 forbids assembling one by guesswork.
#[must_use]
pub fn catalogued(apps: &[ProbedApp]) -> Vec<(&ProbedApp, &'static CatalogueEntry)> {
    apps.iter()
        .filter_map(|a| lookup(&a.stem).map(|e| (a, e)))
        .collect()
}

/// A [`TargetProbe`] that reports exactly what a test configured.
#[cfg(feature = "testkit")]
#[derive(Debug, Default)]
pub struct FakeProbe {
    facts: ProbeFacts,
}

#[cfg(feature = "testkit")]
impl FakeProbe {
    /// A probe that finds nothing until configured.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an application at `exec`, its stem taken from the path.
    pub fn app(&mut self, exec: &str, source: ProbeSource) -> &mut Self {
        let path = PathBuf::from(exec);
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.facts.apps.push(ProbedApp {
            exec: path,
            stem,
            source,
            distro: None,
            installed_at: None,
        });
        self
    }

    /// Add an application at `exec` inside WSL distro `distro`.
    pub fn in_distro(&mut self, exec: &str, distro: &str) -> &mut Self {
        self.app(exec, ProbeSource::Distro);
        if let Some(last) = self.facts.apps.last_mut() {
            last.distro = Some(distro.to_owned());
        }
        self
    }

    /// Set the install time, unix seconds, on every application already added at `exec`.
    pub fn installed_at(&mut self, exec: &str, at: i64) -> &mut Self {
        for a in &mut self.facts.apps {
            if a.exec == *exec {
                a.installed_at = Some(at);
            }
        }
        self
    }

    /// Report `value` as the `$VISUAL`/`$EDITOR` stem.
    pub fn editor_env(&mut self, value: &str) -> &mut Self {
        self.facts.editor_env = Some(value.to_owned());
        self
    }

    /// Report `stem` as the OS default handler for a folder.
    pub fn folder_handler(&mut self, stem: &str) -> &mut Self {
        self.facts.folder_handler_stem = Some(stem.to_owned());
        self
    }
}

#[cfg(feature = "testkit")]
impl TargetProbe for FakeProbe {
    fn probe(&self) -> ProbeFacts {
        self.facts.clone()
    }
}

#[cfg(all(test, feature = "testkit"))]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::launch::catalogue::TargetKind;
    use std::path::PathBuf;

    fn app(exec: &str, source: ProbeSource) -> ProbedApp {
        ProbedApp {
            exec: PathBuf::from(exec),
            stem: PathBuf::from(exec)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            source,
            distro: None,
            installed_at: None,
        }
    }

    #[test]
    fn one_application_reached_by_two_sources_collapses_to_one_candidate() {
        let out = dedupe(vec![
            app("/opt/app/bin/code", ProbeSource::Path),
            app("/opt/app/bin/code", ProbeSource::Desktop),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].source,
            ProbeSource::Desktop,
            "the more direct source wins"
        );
    }

    #[test]
    fn two_installs_of_one_binary_stay_two_candidates() {
        let out = dedupe(vec![
            app("/opt/a/code", ProbeSource::Path),
            app("/opt/b/code", ProbeSource::Path),
        ]);
        assert_eq!(out.len(), 2, "different paths are different applications");
    }

    #[test]
    fn the_same_binary_in_two_distros_stays_two_candidates() {
        let mut a = app("/usr/bin/nvim", ProbeSource::Distro);
        a.distro = Some("distro-a".to_owned());
        let mut b = app("/usr/bin/nvim", ProbeSource::Distro);
        b.distro = Some("distro-b".to_owned());
        assert_eq!(dedupe(vec![a, b]).len(), 2);
    }

    #[test]
    fn an_application_the_catalogue_does_not_know_is_dropped_from_detection() {
        let apps = vec![
            app("/opt/x/somethingelse", ProbeSource::Path),
            app("/usr/bin/kitty", ProbeSource::Path),
        ];
        let kept = catalogued(&apps);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].1.kind, TargetKind::Terminal);
    }

    #[test]
    fn the_fake_probe_reports_exactly_what_was_configured() {
        let mut p = FakeProbe::new();
        p.app("/usr/bin/code", ProbeSource::Desktop)
            .installed_at("/usr/bin/code", 1_700_000_000)
            .editor_env("nvim")
            .folder_handler("nautilus");
        let facts = p.probe();
        assert_eq!(facts.apps.len(), 1);
        assert_eq!(facts.apps[0].stem, "code");
        assert_eq!(facts.apps[0].installed_at, Some(1_700_000_000));
        assert_eq!(facts.editor_env.as_deref(), Some("nvim"));
        assert_eq!(facts.folder_handler_stem.as_deref(), Some("nautilus"));
    }
}
