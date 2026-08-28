use std::path::PathBuf;

use super::{observe, row, Ctx};
use crate::corpus::fixtures as ids;
use crate::corpus::{
    write_file, CorpusError, CorpusFixture, FixtureExpect, BASE_UNIX, VOLUME_A, VOLUME_B,
};

fn finish(
    ctx: &Ctx<'_>,
    name: &str,
    volume: &str,
    path: PathBuf,
    mut expect: FixtureExpect,
) -> Result<Vec<CorpusFixture>, CorpusError> {
    let (head, roots) = observe(ctx, &path)?;
    expect.head_oid = head;
    expect.root_oids = roots;
    Ok(vec![row(name, volume, path, expect)])
}

pub(crate) fn build(ctx: &Ctx<'_>, name: &str) -> Result<Vec<CorpusFixture>, CorpusError> {
    match name {
        ids::FORK => fork(ctx),
        ids::COPY_ONE => copy(ctx, ids::COPY_ONE, VOLUME_A, ctx.vol_a),
        ids::COPY_TWO => copy(ctx, ids::COPY_TWO, VOLUME_B, ctx.vol_b),
        ids::AMBIGUOUS_LINEAGE => ambiguous(ctx),
        ids::REPO_INSIDE_REPO_OUTER => outer(ctx),
        ids::REPO_INSIDE_REPO_INNER => inner(ctx),
        other => Err(CorpusError::UnknownFixture(other.to_owned())),
    }
}

/// Same root commit as the upstream, one commit of its own, a different remote. Two projects
/// on one lineage — never one project.
fn fork(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::FORK);
    let source = ctx.vol_a.join(ids::UPSTREAM);
    ctx.git.run(
        ctx.vol_a,
        BASE_UNIX,
        &[
            "clone",
            "--no-hardlinks",
            &source.display().to_string(),
            &path.display().to_string(),
        ],
    )?;
    let url = "https://example.invalid/fork.git";
    ctx.git
        .run(&path, BASE_UNIX, &["remote", "set-url", "origin", url])?;
    write_file(&path.join("fork.txt"), b"fork\n")?;
    ctx.git.run(&path, BASE_UNIX + 300, &["add", "fork.txt"])?;
    ctx.git
        .run(&path, BASE_UNIX + 300, &["commit", "-m", "fork commit"])?;
    finish(
        ctx,
        ids::FORK,
        VOLUME_A,
        path,
        FixtureExpect {
            origin_url: Some(url.to_owned()),
            notes: Some("one commit ahead of origin/main, no FETCH_HEAD".to_owned()),
            ..FixtureExpect::default()
        },
    )
}

/// One clone, materialised twice on two different volumes, both pointing at one remote.
fn copy(
    ctx: &Ctx<'_>,
    name: &str,
    volume: &str,
    volume_path: &std::path::Path,
) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = volume_path.join(name);
    let source = ctx.vol_a.join(ids::UPSTREAM);
    ctx.git.run(
        volume_path,
        BASE_UNIX,
        &[
            "clone",
            "--no-hardlinks",
            &source.display().to_string(),
            &path.display().to_string(),
        ],
    )?;
    let url = "https://example.invalid/upstream.git";
    ctx.git
        .run(&path, BASE_UNIX, &["remote", "set-url", "origin", url])?;
    finish(
        ctx,
        name,
        volume,
        path,
        FixtureExpect {
            origin_url: Some(url.to_owned()),
            ..FixtureExpect::default()
        },
    )
}

/// Two root commits, no remote: two candidate lineages and nothing to break the tie.
fn ambiguous(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::AMBIGUOUS_LINEAGE);
    let first = ctx.vol_a.join(ids::UPSTREAM);
    let second = ctx.vol_a.join(ids::OTHER_UPSTREAM);
    ctx.git.run(
        ctx.vol_a,
        BASE_UNIX,
        &[
            "clone",
            "--no-hardlinks",
            &first.display().to_string(),
            &path.display().to_string(),
        ],
    )?;
    ctx.git
        .run(&path, BASE_UNIX, &["remote", "remove", "origin"])?;
    ctx.git.run(
        &path,
        BASE_UNIX,
        &["fetch", &second.display().to_string(), "main:other"],
    )?;
    ctx.git.run(
        &path,
        BASE_UNIX + 400,
        &[
            "merge",
            "--allow-unrelated-histories",
            "--no-edit",
            "-m",
            "join",
            "other",
        ],
    )?;
    finish(
        ctx,
        ids::AMBIGUOUS_LINEAGE,
        VOLUME_A,
        path,
        FixtureExpect {
            notes: Some("two candidate lineages, no remote".to_owned()),
            ..FixtureExpect::default()
        },
    )
}

fn outer(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::REPO_INSIDE_REPO_OUTER);
    ctx.git.run(
        ctx.vol_a,
        BASE_UNIX,
        &["init", &ctx.git.template_arg(), &path.display().to_string()],
    )?;
    write_file(&path.join("README.md"), b"Fixture repository.\n")?;
    ctx.git.run(&path, BASE_UNIX, &["add", "README.md"])?;
    ctx.git.run(&path, BASE_UNIX, &["commit", "-m", "first"])?;
    finish(
        ctx,
        ids::REPO_INSIDE_REPO_OUTER,
        VOLUME_A,
        path,
        FixtureExpect::default(),
    )
}

/// A plain `git init` inside another repository's worktree. No `.gitmodules`, no gitlink,
/// nothing tracking it — the outer repository simply sees an untracked directory.
fn inner(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let outer_path = ctx.vol_a.join(ids::REPO_INSIDE_REPO_OUTER);
    let path = outer_path.join("inner-plain");
    ctx.git.run(
        &outer_path,
        BASE_UNIX + 60,
        &["init", &ctx.git.template_arg(), &path.display().to_string()],
    )?;
    write_file(&path.join("inner.txt"), b"inner\n")?;
    ctx.git.run(&path, BASE_UNIX + 60, &["add", "inner.txt"])?;
    ctx.git
        .run(&path, BASE_UNIX + 60, &["commit", "-m", "inner first"])?;
    finish(
        ctx,
        ids::REPO_INSIDE_REPO_INNER,
        VOLUME_A,
        path,
        FixtureExpect {
            parent_fixture: Some(ids::REPO_INSIDE_REPO_OUTER.to_owned()),
            ..FixtureExpect::default()
        },
    )
}
