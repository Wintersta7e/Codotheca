//! A `ProjectRow` constructor for tests, and the reason it is not beside the type.
//!
//! `core/src/protocol.rs` is **generated** by `protocol/generate.mjs` and is gitignored, so
//! anything written into it is deleted by the next `npm run gen`. An inherent impl is legal
//! from anywhere in this crate, so the constructor lives here instead — gated on `testkit`,
//! like every other double, so it cannot reach a shipped binary.

use crate::protocol::{ArtState, Presence, ProjectId, ProjectRow};

impl ProjectRow {
    /// The never-indexed repository: every `Option` `None`, every `bool` false, both clocks at
    /// zero. That is not a convenience — it is the shape §11.1 describes and the one every
    /// "unknown is not zero" assertion is written against, so a test that needs an observed
    /// value sets exactly the field it is about.
    #[must_use]
    pub fn for_test(id: i64) -> Self {
        Self {
            id: ProjectId(id),
            name: format!("p{id}"),
            seed_basename: format!("p{id}"),
            reroll_offset: 0,
            owner: None,
            description: None,
            description_source: None,
            birth_year: None,
            primary_language: None,
            archetype: None,
            art_scene_hash: None,
            art_state: ArtState::Pending,
            condition_signal: None,
            completion_lit: None,
            completion_applicable: None,
            is_pinned: false,
            is_archived: false,
            is_hidden: false,
            is_reference: false,
            is_fork: false,
            is_bare: false,
            is_shallow: false,
            is_submodule: false,
            ambiguous_lineage: false,
            last_touched_at: 0,
            last_interaction_at: None,
            last_commit_at: None,
            last_commit_subject: None,
            first_commit_at: None,
            created_at: 0,
            acknowledged_at: None,
            size_tracked_bytes: None,
            tracked_files: None,
            collection_ids: Vec::new(),
            primary_location: None,
            presence: Some(Presence::Unscanned),
            branch: None,
            is_dirty: None,
            untracked_count: None,
            ahead: None,
            behind: None,
            stash_count: None,
            interrupted_op: None,
            fetch_head_at: None,
            refstate_observed_at: None,
            worktree_observed_at: None,
            error_kind: None,
            error_at: None,
            era_section_id: String::new(),
        }
    }
}
