use std::path::{Path, PathBuf};

use super::{observe, row, Ctx};
use crate::corpus::fixtures as ids;
use crate::corpus::{
    file_url, write_file, CorpusError, CorpusFixture, FixtureExpect, BASE_UNIX, FUTURE_UNIX,
    VOLUME_A,
};

/// `git init` plus one commit. The content is neutral by design: fixtures are described by
/// shape, never by identity.
fn init_with_commit(ctx: &Ctx<'_>, path: &Path, at: i64) -> Result<(), CorpusError> {
    ctx.git.run(
        path.parent().unwrap_or(path),
        at,
        &["init", &ctx.git.template_arg(), &path.display().to_string()],
    )?;
    write_file(&path.join("README.md"), b"Fixture repository.\n")?;
    ctx.git.run(path, at, &["add", "README.md"])?;
    ctx.git.run(path, at, &["commit", "-m", "first"])?;
    Ok(())
}

fn commit_file(
    ctx: &Ctx<'_>,
    path: &Path,
    at: i64,
    name: &str,
    body: &[u8],
    message: &str,
) -> Result<(), CorpusError> {
    write_file(&path.join(name), body)?;
    ctx.git.run(path, at, &["add", name])?;
    ctx.git.run(path, at, &["commit", "-m", message])?;
    Ok(())
}

fn finish(
    ctx: &Ctx<'_>,
    name: &str,
    path: PathBuf,
    mut expect: FixtureExpect,
) -> Result<Vec<CorpusFixture>, CorpusError> {
    let (head, roots) = observe(ctx, &path)?;
    expect.head_oid = head;
    expect.root_oids = roots;
    Ok(vec![row(name, VOLUME_A, path, expect)])
}

pub(crate) fn build(ctx: &Ctx<'_>, name: &str) -> Result<Vec<CorpusFixture>, CorpusError> {
    match name {
        ids::UPSTREAM => upstream(ctx),
        ids::OTHER_UPSTREAM => other_upstream(ctx),
        ids::ZERO_COMMIT => zero_commit(ctx),
        ids::BARE => bare(ctx),
        ids::SHALLOW => shallow(ctx),
        ids::MULTI_ROOT => multi_root(ctx),
        ids::FUTURE_DATED => future_dated(ctx),
        ids::INDEX_LOCK_HELD => index_lock_held(ctx),
        ids::HUGE_UNTRACKED => huge_untracked(ctx),
        other => Err(CorpusError::UnknownFixture(other.to_owned())),
    }
}

/// Three commits and a remote. Everything that needs a source clones this.
fn upstream(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::UPSTREAM);
    init_with_commit(ctx, &path, BASE_UNIX)?;
    commit_file(ctx, &path, BASE_UNIX + 60, "one.txt", b"one\n", "second")?;
    commit_file(ctx, &path, BASE_UNIX + 120, "two.txt", b"two\n", "third")?;
    let url = "https://example.invalid/upstream.git";
    ctx.git
        .run(&path, BASE_UNIX, &["remote", "add", "origin", url])?;
    finish(
        ctx,
        ids::UPSTREAM,
        path,
        FixtureExpect {
            origin_url: Some(url.to_owned()),
            history_depth: Some(3),
            ..FixtureExpect::default()
        },
    )
}

/// An unrelated history, so the ambiguous-lineage fixture has a second candidate.
fn other_upstream(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::OTHER_UPSTREAM);
    // A different first commit date gives a different root OID, which is the whole point.
    init_with_commit(ctx, &path, BASE_UNIX + 5_000)?;
    let url = "https://example.invalid/other.git";
    ctx.git
        .run(&path, BASE_UNIX, &["remote", "add", "origin", url])?;
    finish(
        ctx,
        ids::OTHER_UPSTREAM,
        path,
        FixtureExpect {
            origin_url: Some(url.to_owned()),
            history_depth: Some(1),
            ..FixtureExpect::default()
        },
    )
}

/// `git init` and nothing else: an unborn HEAD. Every date-derived statistic must skip it
/// rather than record a zero.
fn zero_commit(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::ZERO_COMMIT);
    ctx.git.run(
        ctx.vol_a,
        BASE_UNIX,
        &["init", &ctx.git.template_arg(), &path.display().to_string()],
    )?;
    finish(ctx, ids::ZERO_COMMIT, path, FixtureExpect::default())
}

/// No `.git` entry at all — the shape the original detector could not see.
fn bare(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::BARE);
    let source = ctx.vol_a.join(ids::UPSTREAM);
    ctx.git.run(
        ctx.vol_a,
        BASE_UNIX,
        &[
            "clone",
            "--bare",
            "--no-hardlinks",
            &source.display().to_string(),
            &path.display().to_string(),
        ],
    )?;
    finish(
        ctx,
        ids::BARE,
        path,
        FixtureExpect {
            bare: true,
            ..FixtureExpect::default()
        },
    )
}

/// One commit, 14,000-file-clone shape in miniature: cheap history, and excluded from every
/// history statistic.
fn shallow(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::SHALLOW);
    let source = file_url(&ctx.vol_a.join(ids::UPSTREAM));
    // `--depth` needs a real transport, so the source is a file:// URL rather than a path.
    ctx.git.run(
        ctx.vol_a,
        BASE_UNIX,
        &[
            "clone",
            "--depth",
            "1",
            &source,
            &path.display().to_string(),
        ],
    )?;
    finish(
        ctx,
        ids::SHALLOW,
        path,
        FixtureExpect {
            shallow: true,
            history_depth: Some(1),
            ..FixtureExpect::default()
        },
    )
}

/// Two unrelated histories merged, so `rev-list --max-parents=0 HEAD` returns two.
fn multi_root(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::MULTI_ROOT);
    init_with_commit(ctx, &path, BASE_UNIX)?;
    ctx.git
        .run(&path, BASE_UNIX + 10, &["checkout", "--orphan", "second"])?;
    // Not `--cached`: that empties the index and leaves the file in the working tree, so the
    // checkout back to `main` refuses with "untracked working tree files would be overwritten".
    ctx.git.run(&path, BASE_UNIX + 10, &["rm", "-rf", "."])?;
    write_file(&path.join("other.txt"), b"other\n")?;
    ctx.git.run(&path, BASE_UNIX + 10, &["add", "other.txt"])?;
    ctx.git
        .run(&path, BASE_UNIX + 10, &["commit", "-m", "second root"])?;
    ctx.git.run(&path, BASE_UNIX + 20, &["checkout", "main"])?;
    ctx.git.run(
        &path,
        BASE_UNIX + 20,
        &[
            "merge",
            "--allow-unrelated-histories",
            "--no-edit",
            "-m",
            "join",
            "second",
        ],
    )?;
    finish(ctx, ids::MULTI_ROOT, path, FixtureExpect::default())
}

/// A commit dated 2100-01-01. Nothing may let it become "most recently touched".
fn future_dated(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::FUTURE_DATED);
    init_with_commit(ctx, &path, BASE_UNIX)?;
    commit_file(
        ctx,
        &path,
        FUTURE_UNIX,
        "later.txt",
        b"later\n",
        "from the future",
    )?;
    finish(
        ctx,
        ids::FUTURE_DATED,
        path,
        FixtureExpect {
            notes: Some("HEAD is dated 2100-01-01".to_owned()),
            ..FixtureExpect::default()
        },
    )
}

/// Another process holds the index. Every git write refuses; §3.5 defers with backoff.
fn index_lock_held(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::INDEX_LOCK_HELD);
    init_with_commit(ctx, &path, BASE_UNIX)?;
    let built = finish(
        ctx,
        ids::INDEX_LOCK_HELD,
        path.clone(),
        FixtureExpect {
            index_lock_held: true,
            ..FixtureExpect::default()
        },
    )?;
    // Written last, because it blocks the very commands the observation above needs.
    write_file(&path.join(".git").join("index.lock"), b"")?;
    Ok(built)
}

/// A large untracked area — the shape that makes `git status` expensive without any history
/// to speak of.
fn huge_untracked(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::HUGE_UNTRACKED);
    init_with_commit(ctx, &path, BASE_UNIX)?;
    let count = ctx.options.untracked_files;
    for i in 0..count {
        write_file(&path.join("scratch").join(format!("f{i:06}.bin")), b"")?;
    }
    finish(
        ctx,
        ids::HUGE_UNTRACKED,
        path,
        FixtureExpect {
            untracked_files: count,
            ..FixtureExpect::default()
        },
    )
}
