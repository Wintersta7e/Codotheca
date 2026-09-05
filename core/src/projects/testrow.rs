//! A `ProjectRow` constructor for tests, and the reason it is not beside the type.
//!
//! `core/src/protocol.rs` is **generated** by `protocol/generate.mjs` and is gitignored, so
//! anything written into it is deleted by the next `npm run gen`. An inherent impl is legal
//! from anywhere in this crate, so the constructor lives here instead — gated on `testkit`,
//! like every other double, so it cannot reach a shipped binary.

use crate::protocol::{ArtState, LocationId, LocationRef, Presence, ProjectId, ProjectRow};

impl ProjectRow {
    /// The never-indexed repository **that has a working copy**: every `Option` `None`, every
    /// `bool` false, both clocks at zero. That is not a convenience — it is the shape §11.1
    /// describes and the one every "unknown is not zero" assertion is written against, so a test
    /// that needs an observed value sets exactly the field it is about.
    ///
    /// **`primary_location` and `presence` are one pair and are set together.** Before §23 this
    /// builder paired a null location with `Presence::Unscanned`, which is exactly the false
    /// claim §23.2 deleted from `rows.rs` — so the fixture would have been the last place in the
    /// tree still asserting it. Worse, *every* fixture in the tree was zero-location, so
    /// §23.4's classifier would have filed every one of them under `era:notcloned`. A default
    /// that silently satisfies the predicate under test is the same defect as a gate that scans
    /// zero files. The zero-location row is `for_test_not_cloned`, and a test has to ask for it.
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
            primary_location: Some(LocationRef {
                id: LocationId(id * 10),
                path_display: format!("/w/p{id}"),
            }),
            presence: Some(Presence::Present),
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

    /// §23.1's shape: a project Codotheca knows of and holds **no working copy of**. One
    /// `project` row, zero `location` rows — so `primary_location` is `None` and `presence` is
    /// `None` with it. The pair is the whole predicate; there is no second expression of it.
    ///
    /// Explicit, and named, because a test that asserts on a zero-location row has to say so.
    #[must_use]
    pub fn for_test_not_cloned(id: i64) -> Self {
        Self {
            primary_location: None,
            presence: None,
            ..Self::for_test(id)
        }
    }
}
