use std::path::{Path, PathBuf};

use super::{observe, row, Ctx};
use crate::corpus::fixtures as ids;
use crate::corpus::{write_file, CorpusError, CorpusFixture, FixtureExpect, BASE_UNIX, VOLUME_A};

fn seed(ctx: &Ctx<'_>, path: &Path, at: i64, body: &[u8]) -> Result<(), CorpusError> {
    ctx.git.run(
        path.parent().unwrap_or(path),
        at,
        &["init", &ctx.git.template_arg(), &path.display().to_string()],
    )?;
    write_file(&path.join("README.md"), body)?;
    ctx.git.run(path, at, &["add", "README.md"])?;
    ctx.git.run(path, at, &["commit", "-m", "first"])?;
    Ok(())
}

fn finish(
    ctx: &Ctx<'_>,
    name: &str,
    path: PathBuf,
    mut expect: FixtureExpect,
) -> Result<CorpusFixture, CorpusError> {
    let (head, roots) = observe(ctx, &path)?;
    expect.head_oid = head;
    expect.root_oids = roots;
    Ok(row(name, VOLUME_A, path, expect))
}

pub(super) fn build(ctx: &Ctx<'_>, name: &str) -> Result<Vec<CorpusFixture>, CorpusError> {
    match name {
        ids::WORKTREE_PARENT => worktree_parent(ctx).map(|f| vec![f]),
        ids::LINKED_WORKTREE => linked_worktree(ctx).map(|f| vec![f]),
        ids::SUBMODULE_PARENT => submodules(ctx),
        // Both rows are emitted by `submodules`; `dependencies` guarantees it has run.
        ids::SUBMODULE_CHILD | ids::SUBMODULE_NESTED => Ok(Vec::new()),
        other => Err(CorpusError::UnknownFixture(other.to_owned())),
    }
}

fn worktree_parent(ctx: &Ctx<'_>) -> Result<CorpusFixture, CorpusError> {
    let path = ctx.vol_a.join(ids::WORKTREE_PARENT);
    seed(ctx, &path, BASE_UNIX, b"Fixture repository.\n")?;
    finish(ctx, ids::WORKTREE_PARENT, path, FixtureExpect::default())
}

/// A second working tree of one repository. Same project, extra location.
fn linked_worktree(ctx: &Ctx<'_>) -> Result<CorpusFixture, CorpusError> {
    let parent = ctx.vol_a.join(ids::WORKTREE_PARENT);
    let path = ctx.vol_a.join(ids::LINKED_WORKTREE);
    ctx.git.run(
        &parent,
        BASE_UNIX + 60,
        &[
            "worktree",
            "add",
            "-b",
            "second",
            &path.display().to_string(),
        ],
    )?;
    finish(
        ctx,
        ids::LINKED_WORKTREE,
        path,
        FixtureExpect {
            parent_fixture: Some(ids::WORKTREE_PARENT.to_owned()),
            notes: Some("common dir belongs to the parent repository".to_owned()),
            ..FixtureExpect::default()
        },
    )
}

/// Three repositories: a parent, a submodule inside it, and a submodule inside that. The two
/// clone sources live outside both volumes so a scan never finds four projects.
fn submodules(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let inner_src = ctx.sources.join("sub-inner");
    seed(ctx, &inner_src, BASE_UNIX + 1_000, b"Nested fixture.\n")?;

    let child_src = ctx.sources.join("sub-child");
    seed(ctx, &child_src, BASE_UNIX + 2_000, b"Child fixture.\n")?;
    // The URLs below are relative, not absolute. `.gitmodules` is committed content, so an
    // absolute source path puts the corpus root inside the commit and two runs into two
    // directories produce different object ids — exactly what the determinism gate catches.
    // What they are relative to is fixed by `generate`: `<root>/sources` and `<root>/vol-a`.
    ctx.git.run(
        &child_src,
        BASE_UNIX + 2_100,
        &["submodule", "add", "../sub-inner", "inner"],
    )?;
    ctx.git.run(
        &child_src,
        BASE_UNIX + 2_100,
        &["commit", "-m", "add nested submodule"],
    )?;

    let parent = ctx.vol_a.join(ids::SUBMODULE_PARENT);
    seed(ctx, &parent, BASE_UNIX + 3_000, b"Parent fixture.\n")?;
    ctx.git.run(
        &parent,
        BASE_UNIX + 3_100,
        &["submodule", "add", "../../sources/sub-child", "sub"],
    )?;
    ctx.git.run(
        &parent,
        BASE_UNIX + 3_100,
        &["commit", "-m", "add submodule"],
    )?;
    ctx.git.run(
        &parent,
        BASE_UNIX + 3_100,
        &["submodule", "update", "--init", "--recursive"],
    )?;

    let child_path = parent.join("sub");
    let nested_path = child_path.join("inner");
    Ok(vec![
        finish(ctx, ids::SUBMODULE_PARENT, parent, FixtureExpect::default())?,
        finish(
            ctx,
            ids::SUBMODULE_CHILD,
            child_path,
            FixtureExpect {
                parent_fixture: Some(ids::SUBMODULE_PARENT.to_owned()),
                submodule_path: Some("sub".to_owned()),
                ..FixtureExpect::default()
            },
        )?,
        finish(
            ctx,
            ids::SUBMODULE_NESTED,
            nested_path,
            FixtureExpect {
                parent_fixture: Some(ids::SUBMODULE_CHILD.to_owned()),
                submodule_path: Some("inner".to_owned()),
                ..FixtureExpect::default()
            },
        )?,
    ])
}
