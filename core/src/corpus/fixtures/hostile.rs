use std::path::{Path, PathBuf};

use super::{observe, row, skipped, Ctx};
use crate::corpus::fixtures as ids;
use crate::corpus::{write_file, CorpusError, CorpusFixture, FixtureExpect, BASE_UNIX, VOLUME_A};

fn seed(ctx: &Ctx<'_>, path: &Path, at: i64) -> Result<(), CorpusError> {
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

pub(super) fn build(ctx: &Ctx<'_>, name: &str) -> Result<Vec<CorpusFixture>, CorpusError> {
    match name {
        ids::NON_UTF8_PATH => non_utf8(ctx),
        ids::LONG_PATH => long_path(ctx),
        ids::SYMLINK_CYCLE => symlink_cycle(ctx),
        ids::DUBIOUS_OWNERSHIP => dubious(ctx),
        other => Err(CorpusError::UnknownFixture(other.to_owned())),
    }
}

/// A tracked path whose bytes are not valid UTF-8. Built through the index rather than the
/// filesystem, because a Windows filename cannot hold the byte at all — and because argv
/// cannot carry it either, the path is fed to `update-index` on **stdin**.
fn non_utf8(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::NON_UTF8_PATH);
    seed(ctx, &path, BASE_UNIX)?;

    let blob = ctx
        .git
        .run(&path, BASE_UNIX, &["hash-object", "-w", "--stdin"])?;
    let mut entry: Vec<u8> = format!("100644 {blob}\t").into_bytes();
    entry.extend_from_slice(b"data-");
    entry.push(0xff); // never valid UTF-8, in any encoding git might guess at
    entry.extend_from_slice(b".bin");
    entry.push(0);
    ctx.git.run_bytes(
        &path,
        BASE_UNIX,
        &["update-index", "--add", "-z", "--index-info"],
        &entry,
    )?;

    let tree = ctx.git.run(&path, BASE_UNIX, &["write-tree"])?;
    let head = ctx.git.run(&path, BASE_UNIX, &["rev-parse", "HEAD"])?;
    let commit = ctx.git.run_bytes(
        &path,
        BASE_UNIX + 10,
        &[
            "commit-tree",
            &tree,
            "-p",
            &head,
            "-m",
            "add an unrepresentable path",
        ],
        &[],
    )?;
    let commit = String::from_utf8_lossy(&commit).trim().to_owned();
    ctx.git.run(
        &path,
        BASE_UNIX + 10,
        &["update-ref", "refs/heads/main", &commit],
    )?;

    finish(
        ctx,
        ids::NON_UTF8_PATH,
        path,
        FixtureExpect {
            notes: Some("the invalid byte lives in the tree and index, never on disk".to_owned()),
            ..FixtureExpect::default()
        },
    )
}

/// A tracked file whose absolute path is longer than 260 characters.
fn long_path(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::LONG_PATH);
    seed(ctx, &path, BASE_UNIX)?;

    let segment = "s".repeat(48);
    let mut relative = PathBuf::new();
    for _ in 0..6 {
        relative.push(&segment);
    }
    relative.push("deep.txt");
    // std::fs uses the Windows extended-length form for long absolute paths, so this works
    // where a raw Win32 call would fail at MAX_PATH.
    write_file(&path.join(&relative), b"deep\n")?;
    ctx.git.run(
        &path,
        BASE_UNIX + 10,
        &["add", "--", &relative.display().to_string()],
    )?;
    ctx.git.run(
        &path,
        BASE_UNIX + 10,
        &["commit", "-m", "add a very long path"],
    )?;

    finish(
        ctx,
        ids::LONG_PATH,
        path,
        FixtureExpect {
            requires_long_paths: true,
            ..FixtureExpect::default()
        },
    )
}

#[cfg(unix)]
fn make_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn make_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

/// A directory symlink pointing at its own ancestor. A walker without cycle detection never
/// terminates here.
fn symlink_cycle(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::SYMLINK_CYCLE);
    seed(ctx, &path, BASE_UNIX)?;
    let dir = path.join("a");
    std::fs::create_dir_all(&dir).map_err(|e| CorpusError::Io {
        path: dir.clone(),
        message: e.to_string(),
    })?;
    write_file(&dir.join("keep.txt"), b"keep\n")?;
    if make_dir_symlink(&dir, &dir.join("loop")).is_err() {
        return Ok(vec![skipped(
            ids::SYMLINK_CYCLE,
            VOLUME_A,
            path,
            "symlink creation refused by the platform",
        )]);
    }
    finish(
        ctx,
        ids::SYMLINK_CYCLE,
        path,
        FixtureExpect {
            notes: Some("a/loop points at a".to_owned()),
            ..FixtureExpect::default()
        },
    )
}

/// An ordinary repository standing in for one owned by another user.
fn dubious(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::DUBIOUS_OWNERSHIP);
    seed(ctx, &path, BASE_UNIX)?;
    finish(
        ctx,
        ids::DUBIOUS_OWNERSHIP,
        path,
        FixtureExpect {
            notes: Some(
                "ownership cannot be forged without a second account; git's refusal is injected \
                 with FakeGitBackend::dubious_ownership, and this path is the exact one \
                 -c safe.directory must carry"
                    .to_owned(),
            ),
            ..FixtureExpect::default()
        },
    )
}
