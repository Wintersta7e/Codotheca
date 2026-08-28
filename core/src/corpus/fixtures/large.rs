use super::{observe, row, skipped, Ctx};
use crate::corpus::fixtures as ids;
use crate::corpus::{
    CorpusError, CorpusFixture, FixtureExpect, BASE_UNIX, CORPUS_AUTHOR, CORPUS_EMAIL, VOLUME_A,
};

/// Seconds between consecutive commits. Ten minutes keeps fifty thousand commits inside a
/// single year, so no fixture drifts into the future as a side effect of its own depth.
const STEP_SECONDS: i64 = 600;

pub(crate) fn build(ctx: &Ctx<'_>, name: &str) -> Result<Vec<CorpusFixture>, CorpusError> {
    match name {
        ids::DEEP_HISTORY => deep_history(ctx),
        other => Err(CorpusError::UnknownFixture(other.to_owned())),
    }
}

fn deep_history(ctx: &Ctx<'_>) -> Result<Vec<CorpusFixture>, CorpusError> {
    let path = ctx.vol_a.join(ids::DEEP_HISTORY);
    if !ctx.options.large {
        return Ok(vec![skipped(
            ids::DEEP_HISTORY,
            VOLUME_A,
            path,
            "not requested: pass --large",
        )]);
    }
    let count = ctx.options.deep_history_commits;
    ctx.git.run(
        ctx.vol_a,
        BASE_UNIX,
        &["init", &ctx.git.template_arg(), &path.display().to_string()],
    )?;
    let stream = fast_import_stream(count);
    ctx.git.run_bytes(
        &path,
        BASE_UNIX,
        &["fast-import", "--quiet", "--done"],
        &stream,
    )?;
    ctx.git
        .run(&path, BASE_UNIX, &["reset", "--hard", "main"])?;

    let (head, roots) = observe(ctx, &path)?;
    Ok(vec![row(
        ids::DEEP_HISTORY,
        VOLUME_A,
        path,
        FixtureExpect {
            head_oid: head,
            root_oids: roots,
            history_depth: Some(count),
            notes: Some("built with fast-import; the only fixture behind --large".to_owned()),
            ..FixtureExpect::default()
        },
    )])
}

/// One blob and one commit per step. Marks are unique and monotonic; dates are fixed, so the
/// resulting object ids are identical on every machine and every run.
fn fast_import_stream(count: u32) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for index in 0..count {
        let blob_mark = u64::from(index) * 2 + 1;
        let commit_mark = blob_mark + 1;
        let body = format!("{index}\n");
        let message = format!("commit {index}\n");
        let at = BASE_UNIX + i64::from(index) * STEP_SECONDS;

        out.extend_from_slice(b"blob\n");
        out.extend_from_slice(format!("mark :{blob_mark}\n").as_bytes());
        out.extend_from_slice(format!("data {}\n", body.len()).as_bytes());
        out.extend_from_slice(body.as_bytes());

        out.extend_from_slice(b"commit refs/heads/main\n");
        out.extend_from_slice(format!("mark :{commit_mark}\n").as_bytes());
        out.extend_from_slice(
            format!("author {CORPUS_AUTHOR} <{CORPUS_EMAIL}> {at} +0000\n").as_bytes(),
        );
        out.extend_from_slice(
            format!("committer {CORPUS_AUTHOR} <{CORPUS_EMAIL}> {at} +0000\n").as_bytes(),
        );
        out.extend_from_slice(format!("data {}\n", message.len()).as_bytes());
        out.extend_from_slice(message.as_bytes());
        out.extend_from_slice(format!("M 100644 :{blob_mark} counter.txt\n").as_bytes());
        out.extend_from_slice(b"\n");
    }
    out.extend_from_slice(b"done\n");
    out
}
