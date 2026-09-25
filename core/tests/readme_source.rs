#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §25.5 — `projects.readme` answers the document, and says which of the four things it found.
//!
//! The panel this feeds renders markup, so the difference between *the document ended* and *we
//! stopped reading* is visible to a user: an unmarked cut reads as a README that simply stops.
//! Every assertion here is against a real temporary tree and the real migrated index — a mock
//! would agree with whatever the code happened to do about a directory it cannot list.

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::jobs::j6_content::J6_BYTE_CAP;
use codotheca_core::protocol::{ErrorCode, ReadmeSource, ReadmeStateKind};
use codotheca_core::readme::source::read_readme_source;
use codotheca_core::readme::{dispatch_readme_command, ReadmeCtx, ReadmeError, README_COMMANDS};
use codotheca_core::testing::TempIndex;

const NOW: i64 = 1_781_179_200;

/// A project with one present location at `dir`.
fn seeded(
    dir: &std::path::Path,
) -> (
    TempIndex,
    codotheca_core::protocol::ProjectId,
    codotheca_core::protocol::LocationId,
) {
    let index = TempIndex::new();
    let project = index.insert_project();
    let location = index.insert_location(project, &dir.to_string_lossy());
    (index, project, location)
}

fn read(
    index: &TempIndex,
    project: codotheca_core::protocol::ProjectId,
    location: codotheca_core::protocol::LocationId,
) -> Result<ReadmeSource, ReadmeError> {
    let sink = CollectingSink::default();
    let ctx = ReadmeCtx {
        index: index.index(),
        events: &sink,
        now: NOW,
    };
    read_readme_source(&ctx, project, location)
}

#[test]
fn ac_p2_25_19_the_whole_document_is_returned_under_the_existing_cap() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Two paragraphs. `ReadmeState.text` carries only the first — `j6_content::persist` stores
    // `first_paragraph(excerpt)` — and this command exists because the panel needs both.
    std::fs::write(
        tmp.path().join("README.md"),
        "# widget\n\nThe second paragraph, which the stored excerpt drops.\n",
    )
    .expect("write");
    let (index, project, location) = seeded(tmp.path());

    let source = read(&index, project, location).expect("present");
    assert_eq!(source.state, ReadmeStateKind::Present);
    assert_eq!(source.path.as_deref(), Some("README.md"));
    let text = source.text.expect("text");
    assert!(text.contains("# widget"), "the first paragraph is present");
    assert!(
        text.contains("The second paragraph, which the stored excerpt drops."),
        "the whole document, not first_paragraph's output: {text:?}"
    );
    assert!(!source.truncated, "a short document was not cut");
    assert_eq!(source.read_at, Some(NOW));
}

#[test]
fn a_document_one_byte_past_the_cap_is_truncated_and_says_so() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // The boundary is read from the constant, not written as a literal: change `J6_BYTE_CAP` and
    // this test's fixture moves with it, which is what one owner means.
    let body = "a".repeat(J6_BYTE_CAP + 1);
    std::fs::write(tmp.path().join("README.md"), &body).expect("write");
    let (index, project, location) = seeded(tmp.path());

    let source = read(&index, project, location).expect("present");
    assert!(source.truncated, "a document past the cap is cut");
    assert_eq!(
        source.text.as_deref().map(str::len),
        Some(J6_BYTE_CAP),
        "exactly the cap, never more"
    );

    // And one byte under it is not truncated, so `truncated` is not simply always true.
    let under = "a".repeat(J6_BYTE_CAP - 1);
    std::fs::write(tmp.path().join("README.md"), &under).expect("write");
    let under_cap = read(&index, project, location).expect("present");
    assert!(!under_cap.truncated);
    assert_eq!(
        under_cap.text.as_deref().map(str::len),
        Some(J6_BYTE_CAP - 1)
    );
}

#[test]
fn a_cut_lands_on_a_character_boundary_and_invents_no_replacement_character() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Fill to one byte short of the cap, then a two-byte character straddling it.
    let mut body = "a".repeat(J6_BYTE_CAP - 1).into_bytes();
    body.extend_from_slice("é".as_bytes());
    assert_eq!(body.len(), J6_BYTE_CAP + 1);
    std::fs::write(tmp.path().join("README.md"), &body).expect("write");
    let (index, project, location) = seeded(tmp.path());

    let source = read(&index, project, location).expect("present");
    let text = source.text.expect("text");
    assert!(source.truncated);
    assert!(
        !text.contains('\u{fffd}'),
        "a replacement character the file does not contain was invented"
    );
    assert_eq!(text.len(), J6_BYTE_CAP - 1, "cut back to the boundary");
}

#[test]
fn the_name_that_matched_is_the_name_reported() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Not `README.md`. The panel's header rendered the literal `README.md` whatever was read.
    std::fs::write(tmp.path().join("README.rst"), "widget\n======\n").expect("write");
    let (index, project, location) = seeded(tmp.path());

    let source = read(&index, project, location).expect("present");
    assert_eq!(source.path.as_deref(), Some("README.rst"));
}

#[test]
fn a_readable_directory_with_no_readme_is_absent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.rs"), "fn main() {}\n").expect("write");
    let (index, project, location) = seeded(tmp.path());

    let source = read(&index, project, location).expect("absent");
    assert_eq!(source.state, ReadmeStateKind::Absent);
    assert_eq!(source.path, None);
    assert_eq!(source.text, None);
    assert!(!source.truncated);
}

#[test]
fn a_directory_that_cannot_be_listed_is_repo_unreadable_and_never_absent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gone = tmp.path().join("no-such-directory");
    let (index, project, location) = seeded(&gone);

    let error = read(&index, project, location).expect_err("must refuse");
    assert_eq!(
        error.code(),
        ErrorCode::RepoUnreadable,
        "never claim an absence you have not established: {error}"
    );
}

#[test]
fn a_location_that_is_not_that_projects_is_a_protocol_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("README.md"), "x\n").expect("write");
    let (index, project, location) = seeded(tmp.path());
    let other = index.insert_project();

    let sink = CollectingSink::default();
    let ctx = ReadmeCtx {
        index: index.index(),
        events: &sink,
        now: NOW,
    };
    let error = read_readme_source(&ctx, other, location).expect_err("must refuse");
    assert_eq!(error.code(), ErrorCode::Protocol);

    // …and an id that names no project at all is the same answer.
    let unknown_error =
        read_readme_source(&ctx, codotheca_core::protocol::ProjectId(9_999), location)
            .expect_err("must refuse");
    assert_eq!(unknown_error.code(), ErrorCode::Protocol);
    let _ = project;
}

#[test]
fn the_command_is_dispatched_by_name_and_serialises_the_wire_shape() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("README.md"), "# widget\n").expect("write");
    let (index, project, location) = seeded(tmp.path());
    let sink = CollectingSink::default();
    let ctx = ReadmeCtx {
        index: index.index(),
        events: &sink,
        now: NOW,
    };

    let answered = dispatch_readme_command(
        &ctx,
        "projects.readme",
        serde_json::json!({ "projectId": project.0, "locationId": location.0 }),
    )
    .expect("this module owns projects.readme")
    .expect("it answers");
    assert_eq!(answered["state"], "present");
    assert_eq!(answered["path"], "README.md");
    assert_eq!(answered["truncated"], false);

    assert!(
        dispatch_readme_command(&ctx, "projects.get", serde_json::json!({})).is_none(),
        "a command this module does not own is None, which is what the router chains on"
    );
}

/// R37's defect, in this module's terms: the schema, the router and the module's own census must
/// name the same three commands. The Rust side is compared here because no JavaScript test can
/// read a Rust constant, and `protocol/test/surface.test.mjs` declares its own list of the same
/// names — two lists, so something has to compare them to the thing they describe.
#[test]
fn the_census_is_exactly_what_the_router_sends_to_this_module() {
    use codotheca_core::assembly::route::{command_name, route, Route};

    let schema = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../protocol/schema/protocol.json"),
    )
    .expect("protocol.json is readable");
    let doc: serde_json::Value = serde_json::from_str(&schema).expect("protocol.json parses");
    let mut routed: Vec<String> = doc["commands"]
        .as_array()
        .expect("commands is an array")
        .iter()
        .filter_map(|c| c["name"].as_str())
        .filter(|name| {
            matches!(
                route(command_name(name).expect("routable")),
                Route::Readme | Route::ReadmeNet
            )
        })
        .map(str::to_owned)
        .collect();
    routed.sort();

    let mut census: Vec<String> = README_COMMANDS.iter().map(|c| (*c).to_owned()).collect();
    census.sort();
    assert_eq!(
        census, routed,
        "the census and the router name different commands"
    );
}
