#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §48.3 row 11 (R161): the default diagnostics bundle carries no project name, no note text and
//! no full path; it keeps basenames and volume shapes. The one-click reveal puts all three back.
//! Driven through the production dispatcher, and read from the file the command wrote.

use codotheca_core::index::Index;
use codotheca_core::surfaces::{diag, dispatch_surface_command, SurfaceCtx};

const NOW: i64 = 900;
const NAME: &str = "zqnamesentinel";
const NOTE: &str = "zqnotesentinel";
const REMOTE: &str = "zqremotesentinel";
/// Interior segments of the location's and the root's paths. Their basenames are ordinary.
const SEGMENTS: [&str; 4] = ["zqlocsegone", "zqlocsegtwo", "zqrootsegone", "zqrootsegtwo"];
const LOCATION: &str = "/zqlocsegone/zqlocsegtwo/ordinary";
const BASENAME: &str = "ordinary";

fn seeded(dir: &std::path::Path) -> Index {
    let index = Index::open(dir).expect("open");
    index
        .conn()
        .execute_batch(&format!(
            "INSERT INTO project (id, name, seed_basename, created_at, updated_at,
                                  last_touched_at, notes, remote_key)
             VALUES (1, '{NAME}', '{BASENAME}', 1, 1, 1, '{NOTE}', '{REMOTE}');
             INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key,
                                   path_display, volume_key, store_key, presence, repo_kind)
             VALUES (1, 1, 'linux', '', X'2f61', X'2f61', '{LOCATION}', 'vol-abcdef',
                     'store-1', 'present', 'worktree');
             INSERT INTO scan_root (id, kind, distro, path_bytes, path_key, path_display,
                                    enabled, added_by, descend_into_repos, added_at)
             VALUES (1, 'linux', '', X'2f62', X'2f62', '/zqrootsegone/zqrootsegtwo/code',
                     1, 'user', 0, 1);"
        ))
        .expect("seed");
    index
}

/// The bundle the production command wrote, as text.
fn bundle(index: &Index, dir: &std::path::Path, reveal: bool) -> String {
    let ctx = SurfaceCtx { index, now: NOW };
    let answer = dispatch_surface_command(
        &ctx,
        "diag.bundle",
        serde_json::json!({ "includeRealPaths": reveal }),
    )
    .expect("diag.bundle is a surface command")
    .expect("the bundle is written");
    let file = dir.join(format!("{}{NOW}.json", diag::BUNDLE_FILE_PREFIX));
    assert!(
        answer["pathDisplay"]
            .as_str()
            .is_some_and(|p| p.ends_with(&format!("{}{NOW}.json", diag::BUNDLE_FILE_PREFIX))),
        "the answer names the file it wrote: {answer}"
    );
    assert_eq!(answer["anonymised"], serde_json::json!(!reveal));
    std::fs::read_to_string(file).expect("the bundle file")
}

#[test]
fn ac_p4_48_8_the_default_bundle_carries_no_name_no_note_and_no_full_path_and_the_reveal_restores_them(
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    let sentinels: Vec<&str> = [NAME, NOTE, REMOTE].into_iter().chain(SEGMENTS).collect();

    let default = bundle(&index, dir.path(), false);
    let in_default: Vec<&&str> = sentinels.iter().filter(|s| default.contains(**s)).collect();
    let revealed = bundle(&index, dir.path(), true);
    let in_revealed = sentinels.iter().filter(|s| revealed.contains(**s)).count();
    eprintln!(
        "sentinels scanned: {}; in default: {}; in revealed: {in_revealed}",
        sentinels.len(),
        in_default.len()
    );
    assert!(
        !sentinels.is_empty(),
        "a scan of no sentinel proves nothing"
    );

    assert!(
        in_default.is_empty(),
        "the default bundle carries {in_default:?}"
    );
    assert!(
        default.contains(BASENAME),
        "basenames stay: a bundle without paths is useless"
    );
    assert!(default.contains("vol-1"), "the volume's shape stays");

    for kept in [NAME, NOTE, LOCATION] {
        assert!(revealed.contains(kept), "the reveal puts {kept} back");
    }
}
