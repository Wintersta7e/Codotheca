//! §32.6's five parsers behind one dispatch, and the rule that makes a bounded reader safe.
//!
//! **Total, or `NotRead`.** A parser that meets a construct it does not understand **inside the
//! region it reads** returns [`LockfileRead::NotRead`] for the whole file. It never returns the
//! triples it managed to collect. A truncated or partial parse produces a silently short triple
//! set, and a short set produces a **false clean** — the one rendering this section exists to
//! prevent. There is deliberately no third variant carrying partial results.
//!
//! **No new dependency, and that is a decision with a reason.** The workspace MSRV is 1.80,
//! `serde_yaml` is unmaintained, and each of the four non-JSON formats needs exactly two fields
//! from a machine-generated file. Each parser below is a **bounded reader over the one region that
//! carries names and versions** — not a document parser — which also means no user's repository
//! can hand this process an arbitrary 16 MB document to evaluate.

use crate::provider::PackageVersion;

/// What a lockfile read produced. **Two variants, and deliberately no third.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockfileRead {
    /// The whole region was understood; every `(name, version)` pair it carries.
    Parsed(Vec<PackageVersion>),
    /// Over the byte cap, unreadable, not UTF-8, or holding a construct the parser does not
    /// understand. Never a partial result.
    NotRead,
}

/// Dispatch on the filename. An unrecognised name reaches this only through a walk that matched
/// one of the six, so it is `NotRead` rather than a panic.
#[must_use]
pub fn parse_lockfile(name: &str, bytes: &[u8]) -> LockfileRead {
    let Ok(text) = std::str::from_utf8(bytes) else {
        // A lockfile is machine-generated ASCII-or-UTF-8. Bytes that are neither are a file this
        // reader does not understand, and understanding half of it is the failure mode.
        return LockfileRead::NotRead;
    };
    match name {
        "package-lock.json" => parse_package_lock(text),
        "yarn.lock" => parse_yarn_lock(text),
        "pnpm-lock.yaml" => parse_pnpm_lock(text),
        "Cargo.lock" | "poetry.lock" | "uv.lock" => parse_toml_packages(text),
        _ => LockfileRead::NotRead,
    }
}

fn pair(name: &str, version: &str) -> PackageVersion {
    PackageVersion {
        name: name.to_owned(),
        version: version.to_owned(),
    }
}

/// Split a `name@version` descriptor. **The `@` that matters is the last one**, because a scoped
/// npm name begins with one: `@scope/pkg@1.0.0` is `@scope/pkg` at `1.0.0`.
fn split_at_last_at(descriptor: &str) -> Option<(&str, &str)> {
    let at = descriptor.rfind('@')?;
    if at == 0 {
        return None;
    }
    let (name, rest) = descriptor.split_at(at);
    Some((name, rest.get(1..)?))
}

/// npm, JSON. **v2/v3's `packages` map and v1's nested `dependencies`**, which are two layouts of
/// one ecosystem rather than two ecosystems.
fn parse_package_lock(text: &str) -> LockfileRead {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(text) else {
        return LockfileRead::NotRead;
    };
    let mut out = Vec::new();

    // v2 and v3. The key is an install path; the package name is what follows the last
    // `node_modules/`. The root entry's key is the empty string and names the project itself.
    if let Some(packages) = doc.get("packages").and_then(serde_json::Value::as_object) {
        for (path, entry) in packages {
            if path.is_empty() {
                continue;
            }
            let Some(name) = path
                .rsplit("node_modules/")
                .next()
                .filter(|n| !n.is_empty())
            else {
                return LockfileRead::NotRead;
            };
            // A `link` entry points at a workspace member already listed under its own key, and a
            // workspace member has no published version to ask about.
            if entry.get("link").and_then(serde_json::Value::as_bool) == Some(true) {
                continue;
            }
            let Some(version) = entry.get("version").and_then(serde_json::Value::as_str) else {
                // An entry with no version is a construct this reader does not understand — not a
                // package to skip quietly, because skipping shortens the set.
                return LockfileRead::NotRead;
            };
            out.push(pair(name, version));
        }
        return LockfileRead::Parsed(out);
    }

    // v1. `dependencies` is a name-keyed tree, nested arbitrarily deep.
    if let Some(deps) = doc
        .get("dependencies")
        .and_then(serde_json::Value::as_object)
    {
        if collect_v1(deps, &mut out).is_none() {
            return LockfileRead::NotRead;
        }
        return LockfileRead::Parsed(out);
    }

    // A lockfile that declares neither map has no dependencies. That is an **answer**, and it is
    // what makes `no lockfile of any recognised ecosystem` and `a lockfile with nothing in it`
    // both honestly `clean`.
    LockfileRead::Parsed(out)
}

fn collect_v1(
    deps: &serde_json::Map<String, serde_json::Value>,
    out: &mut Vec<PackageVersion>,
) -> Option<()> {
    for (name, entry) in deps {
        let version = entry.get("version").and_then(serde_json::Value::as_str)?;
        out.push(pair(name, version));
        if let Some(nested) = entry
            .get("dependencies")
            .and_then(serde_json::Value::as_object)
        {
            collect_v1(nested, out)?;
        }
    }
    Some(())
}

/// npm, bespoke and line-oriented, **in both dialects**: the classic v1 format and Berry's
/// YAML-shaped one. The region read is the entry blocks; an entry with no version line is a
/// construct this reader does not understand.
fn parse_yarn_lock(text: &str) -> LockfileRead {
    let mut out = Vec::new();
    let mut pending: Option<String> = None;
    let mut saw_version = true;

    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            // A new entry header closes the previous one, which must have produced a version.
            if !saw_version {
                return LockfileRead::NotRead;
            }
            let Some(header) = line.strip_suffix(':') else {
                return LockfileRead::NotRead;
            };
            // Several descriptors may share one entry; they name one package, so the first is
            // enough and the rest are ranges of it.
            let first = header.split(", ").next().unwrap_or(header).trim();
            let descriptor = first.trim_matches('"');
            if descriptor == "__metadata" {
                pending = None;
                saw_version = true;
                continue;
            }
            let Some((name, _range)) = split_at_last_at(descriptor) else {
                return LockfileRead::NotRead;
            };
            pending = Some(name.to_owned());
            saw_version = false;
            continue;
        }
        let trimmed = line.trim();
        // v1 writes `version "1.2.3"`, Berry writes `version: 1.2.3`.
        let value = trimmed
            .strip_prefix("version:")
            .or_else(|| trimmed.strip_prefix("version "));
        if let (Some(value), Some(name)) = (value, pending.as_ref()) {
            if saw_version {
                continue;
            }
            out.push(pair(name, value.trim().trim_matches('"')));
            saw_version = true;
        }
    }
    if !saw_version {
        return LockfileRead::NotRead;
    }
    LockfileRead::Parsed(out)
}

/// npm, YAML, **read over the `packages:` block only**. Keys are `/name@version` in v6 and
/// `name@version` in v9, and a peer-dependency suffix travels in parentheses.
fn parse_pnpm_lock(text: &str) -> LockfileRead {
    let mut out = Vec::new();
    let mut in_packages = false;

    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            in_packages = line.trim_end() == "packages:";
            continue;
        }
        if !in_packages {
            continue;
        }
        // Exactly one level inside the block is a package key; anything deeper is that package's
        // own metadata and is not read.
        let depth = line.len() - line.trim_start().len();
        if depth != 2 {
            continue;
        }
        let Some(key) = line.trim().strip_suffix(':') else {
            return LockfileRead::NotRead;
        };
        let key = key.trim_matches('\'').trim_matches('"');
        let key = key.strip_prefix('/').unwrap_or(key);
        // `foo@1.2.3(react@18.0.0)` — the peer set is not a version.
        let key = key.split('(').next().unwrap_or(key);
        let Some((name, version)) = split_at_last_at(key) else {
            return LockfileRead::NotRead;
        };
        out.push(pair(name, version));
    }
    LockfileRead::Parsed(out)
}

/// `Cargo.lock`, `poetry.lock` and `uv.lock` all write the **same `[[package]]` shape**, so one
/// reader serves three files across two ecosystems. The region read is the `[[package]]` blocks:
/// every other table is skipped without being understood, and a block missing either field is a
/// construct this reader does not understand.
fn parse_toml_packages(text: &str) -> LockfileRead {
    let mut out = Vec::new();
    let mut in_package = false;
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') {
            if in_package {
                match (name.take(), version.take()) {
                    (Some(n), Some(v)) => out.push(pair(&n, &v)),
                    _ => return LockfileRead::NotRead,
                }
            }
            in_package = trimmed == "[[package]]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("name = ") {
            name = Some(value.trim().trim_matches('"').to_owned());
        } else if let Some(value) = trimmed.strip_prefix("version = ") {
            version = Some(value.trim().trim_matches('"').to_owned());
        }
    }
    if in_package {
        match (name, version) {
            (Some(n), Some(v)) => out.push(pair(&n, &v)),
            _ => return LockfileRead::NotRead,
        }
    }
    LockfileRead::Parsed(out)
}
