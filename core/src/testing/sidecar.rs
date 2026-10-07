//! Fixtures for the sidecar's registered sections: one per section, so every registered section
//! can be populated by a test.

use rusqlite::Transaction;

use crate::index::IndexError;
use crate::protocol::{LocationId, ProjectId};

/// The rows a fixture library already holds, for a section fixture to plant its own against.
#[derive(Debug, Clone, Default)]
pub struct FixtureIds {
    /// The library's projects, in creation order.
    pub projects: Vec<ProjectId>,
    /// The library's locations, in creation order.
    pub locations: Vec<LocationId>,
}

/// A fixture that plants rows for one registered section.
pub type SectionFixture = fn(&Transaction<'_>, &FixtureIds) -> Result<(), IndexError>;

/// One fixture per registered section, by section name. A section registers its fixture in the
/// change that registers the section; `sidecar_registry` holds the two lists equal.
pub const SECTION_FIXTURES: &[(&str, SectionFixture)] = &[];
