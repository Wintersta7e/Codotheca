//! §32.6's five parsers behind one dispatch, and the rule that makes a bounded reader safe.
//!
//! **Total, or `NotRead`.** A parser that meets a construct it does not understand **inside the
//! region it reads** returns [`LockfileRead::NotRead`] for the whole file. It never returns the
//! triples it managed to collect. A truncated or partial parse produces a silently short triple
//! set, and a short set produces a **false clean** — the one rendering this section exists to
//! prevent. There is deliberately no third variant carrying partial results.
//!
//! **No new dependency, and that is a decision with a reason.** `serde_yaml` is unmaintained, and
//! each of the four non-JSON formats needs two fields and a source marker from a
//! machine-generated file — too little to justify a YAML or TOML parser in the dependency tree.
//! Each parser below is a **bounded reader over the one region that carries names and versions**
//! — not a document parser — which also means no user's repository can hand this process an
//! arbitrary 16 MB document to evaluate. Re-decide at the MSRV follow-up.
//!
//! **Published names only.** Every pair a parser yields is sent to the advisory database. A
//! workspace member, a local directory, a link or a path crate is the user's own code: its name is
//! private and no advisory can name it, so each parser drops it rather than send it. A git, url or
//! tarball dependency is dropped for the same reason: it is not registry-published, its name may
//! be a private repository's, and it can collide with an unrelated registry package.

use crate::provider::PackageVersion;

/// The range or version prefixes that point at the user's own tree rather than a registry.
const LOCAL_PROTOCOLS: [&str; 4] = ["file:", "link:", "workspace:", "portal:"];

/// Whether a range or version names a local package rather than a published one.
fn is_local_spec(spec: &str) -> bool {
    LOCAL_PROTOCOLS.iter().any(|p| spec.starts_with(p))
}

/// Whether an npm-ecosystem `resolved` URL is a registry tarball. Every npm-compatible registry,
/// a mirror included, serves `<registry>/<name>/-/<file>.tgz`. A git checkout, a `github:`
/// shorthand, a `file:` tarball, a forge's archive and any other http tarball are not published.
fn is_registry_tarball(url: &str) -> bool {
    let path = url.split(['#', '?']).next().unwrap_or(url);
    (path.starts_with("https://") || path.starts_with("http://"))
        && path.contains("/-/")
        && std::path::Path::new(path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("tgz"))
}

/// Whether a yarn resolution points at the registry: Berry's `npm:` protocol, or a classic
/// `resolved` registry tarball. Every other protocol (`git`, `github:`, `patch:`, `exec:`, a
/// local one) and every other URL is not a published package.
fn is_registry_resolution(spec: &str) -> bool {
    spec.starts_with("npm:") || is_registry_tarball(spec)
}

/// Whether a version is `MAJOR.MINOR.PATCH`, with an optional pre-release or build suffix — what
/// a registry publishes. npm v1 records a git or tarball dependency's URL as its version.
fn is_semver_shaped(version: &str) -> bool {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

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
        "Cargo.lock" => parse_toml_packages(text, TomlDialect::Cargo),
        "poetry.lock" => parse_toml_packages(text, TomlDialect::Poetry),
        "uv.lock" => parse_toml_packages(text, TomlDialect::Uv),
        _ => LockfileRead::NotRead,
    }
}

fn pair(name: &str, version: &str) -> PackageVersion {
    PackageVersion {
        name: name.to_owned(),
        version: version.to_owned(),
    }
}

/// Split a `name@range` descriptor at **the first `@` after the first character**. A scoped npm
/// name begins with one (`@scope/pkg@1.0.0` is `@scope/pkg` at `1.0.0`), no name holds another,
/// and a range can (`git-pkg@git+ssh://git@host/…`), so the last `@` would split inside the URL.
fn split_name_version(descriptor: &str) -> Option<(&str, &str)> {
    let (at, _) = descriptor.char_indices().skip(1).find(|&(_, c)| c == '@')?;
    let (name, rest) = descriptor.split_at(at);
    Some((name, rest.get(1..)?))
}

/// Whether an npm entry is registry-published: a semver version, and a `resolved` (when the entry
/// records one) that is a registry tarball rather than a git, `file:` or other url source.
fn is_npm_published(entry: &serde_json::Value, version: &str) -> bool {
    is_semver_shaped(version)
        && entry
            .get("resolved")
            .and_then(serde_json::Value::as_str)
            .map_or(true, is_registry_tarball)
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
            // A key outside `node_modules/` is an install-free path: a workspace member or a
            // `file:` directory, the project's own code. The root entry's key is the empty string.
            if !(path.starts_with("node_modules/") || path.contains("/node_modules/")) {
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
            if is_npm_published(entry, version) {
                out.push(pair(name, version));
            }
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
        // v1 records a local directory, a link, a git checkout or a tarball url as its version;
        // what it depends on is still read.
        if is_npm_published(entry, version) {
            out.push(pair(name, version));
        }
        if let Some(nested) = entry
            .get("dependencies")
            .and_then(serde_json::Value::as_object)
        {
            collect_v1(nested, out)?;
        }
    }
    Some(())
}

/// One yarn entry as it is read, closed by the next entry header or the end of file.
struct YarnEntry {
    name: String,
    /// The header's range points at the user's own tree.
    local: bool,
    version: Option<String>,
    /// A classic `resolved` or Berry `resolution` that is not the registry.
    off_registry: bool,
}

impl YarnEntry {
    /// The entry's pair if it is published, nothing if it is not, or `Err` for an entry with no
    /// version line — a construct this reader does not understand.
    fn close(self) -> Result<Option<PackageVersion>, ()> {
        let version = self.version.ok_or(())?;
        Ok((!self.local && !self.off_registry).then(|| pair(&self.name, &version)))
    }
}

/// npm, bespoke and line-oriented, **in both dialects**: the classic v1 format and Berry's
/// YAML-shaped one. The region read is the entry blocks; an entry with no version line is a
/// construct this reader does not understand.
fn parse_yarn_lock(text: &str) -> LockfileRead {
    let mut out = Vec::new();
    let mut open: Option<YarnEntry> = None;

    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            // A new entry header closes the previous one, which must have produced a version.
            if let Some(done) = open.take() {
                match done.close() {
                    Ok(published) => out.extend(published),
                    Err(()) => return LockfileRead::NotRead,
                }
            }
            let Some(header) = line.strip_suffix(':') else {
                return LockfileRead::NotRead;
            };
            // Several descriptors may share one entry; they name one package, so the first is
            // enough and the rest are ranges of it.
            let first = header.split(", ").next().unwrap_or(header).trim();
            let descriptor = first.trim_matches('"');
            if descriptor == "__metadata" {
                continue;
            }
            let Some((name, range)) = split_name_version(descriptor) else {
                return LockfileRead::NotRead;
            };
            // The entry is still read to its version line, so a malformed local entry is as
            // unreadable as any other; only its pair is withheld.
            open = Some(YarnEntry {
                name: name.to_owned(),
                local: is_local_spec(range),
                version: None,
                off_registry: false,
            });
            continue;
        }
        let Some(entry) = open.as_mut() else {
            continue;
        };
        let trimmed = line.trim();
        // v1 writes `version "1.2.3"`, Berry writes `version: 1.2.3`.
        let version = trimmed
            .strip_prefix("version:")
            .or_else(|| trimmed.strip_prefix("version "));
        if let Some(value) = version {
            if entry.version.is_none() {
                entry.version = Some(value.trim().trim_matches('"').to_owned());
            }
            continue;
        }
        // v1 writes `resolved "<url>"`; Berry writes `resolution: "<name>@<protocol>…"`.
        if let Some(value) = trimmed.strip_prefix("resolved ") {
            entry.off_registry = !is_registry_resolution(value.trim().trim_matches('"'));
        } else if let Some(value) = trimmed.strip_prefix("resolution:") {
            let value = value.trim().trim_matches('"');
            entry.off_registry =
                split_name_version(value).map_or(true, |(_, spec)| !is_registry_resolution(spec));
        }
    }
    if let Some(done) = open {
        match done.close() {
            Ok(published) => out.extend(published),
            Err(()) => return LockfileRead::NotRead,
        }
    }
    LockfileRead::Parsed(out)
}

/// Whether a pnpm `resolution: {…}` names a source other than the registry: a git `repo`, a local
/// `directory`, or a `tarball` outside the registry's layout. A registry package records only its
/// `integrity`, or a registry tarball beside it.
fn is_pnpm_off_registry(resolution: &str) -> bool {
    let tarball = resolution
        .split_once("tarball:")
        .map(|(_, rest)| rest.split([',', '}']).next().unwrap_or(rest).trim());
    resolution.contains("repo:")
        || resolution.contains("type: git")
        || resolution.contains("directory:")
        || tarball.is_some_and(|url| !is_registry_tarball(url))
}

/// npm, YAML, **read over the `packages:` block only**. Keys are `/name@version` in v6 and
/// `name@version` in v9, and a peer-dependency suffix travels in parentheses.
fn parse_pnpm_lock(text: &str) -> LockfileRead {
    let mut out = Vec::new();
    let mut in_packages = false;
    // The open package's pair, and whether its `resolution` names a source off the registry.
    let mut open: Option<(PackageVersion, bool)> = None;

    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let depth = line.len() - line.trim_start().len();
        // A top-level line or the next package key closes the open package.
        if depth <= 2 {
            if let Some((published, false)) = open.take() {
                out.push(published);
            }
        }
        if depth == 0 {
            in_packages = line.trim_end() == "packages:";
            continue;
        }
        if !in_packages {
            continue;
        }
        if depth == 4 {
            if let (Some((_, off)), Some(value)) =
                (open.as_mut(), line.trim().strip_prefix("resolution:"))
            {
                *off = is_pnpm_off_registry(value);
            }
            continue;
        }
        // Exactly one level inside the block is a package key; anything deeper is that package's
        // own metadata and is not read.
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
        // A local directory: v6 keys it `file:local-pkg`, v9 `local-pkg@file:local-pkg`.
        // Workspace members and `link:` targets appear only under `importers`, which is not read.
        if is_local_spec(key) {
            continue;
        }
        let Some((name, version)) = split_name_version(key) else {
            return LockfileRead::NotRead;
        };
        // A git or tarball dependency is keyed by its url (`git-pkg@git+ssh://…`,
        // `tarball-pkg@https://…`) where a registry package carries its version.
        if is_local_spec(version) || version.contains(':') {
            continue;
        }
        open = Some((pair(name, version), false));
    }
    if let Some((published, false)) = open {
        out.push(published);
    }
    LockfileRead::Parsed(out)
}

/// Which of the three `[[package]]` files is being read: they share a shape and differ only in
/// how a package says where it came from.
#[derive(Clone, Copy)]
enum TomlDialect {
    /// A registry crate's `source` starts `registry+` or `sparse+`; a git crate's `git+`, and a
    /// path crate and the root carry none.
    Cargo,
    /// A non-registry package carries a `[package.source]` sub-table with a `type`.
    Poetry,
    /// Every package carries an inline `source = { … }`; only `registry` is published.
    Uv,
}

/// One `[[package]]` block as it is read, closed by the next table header or the end of file.
#[derive(Default)]
struct TomlPackage {
    name: Option<String>,
    version: Option<String>,
    /// The block's own `source = …` value, unparsed.
    source: Option<String>,
    /// Poetry's `[package.source]` `type`.
    source_type: Option<String>,
}

impl TomlPackage {
    /// The block's pair if it is published, nothing if it is local, or `Err` for a block missing
    /// either field — a construct this reader does not understand.
    fn close(self, dialect: TomlDialect) -> Result<Option<PackageVersion>, ()> {
        let (Some(name), Some(version)) = (self.name, self.version) else {
            return Err(());
        };
        let published = match dialect {
            TomlDialect::Cargo => self.source.as_deref().is_some_and(|s| {
                let s = s.trim_start_matches('"');
                s.starts_with("registry+") || s.starts_with("sparse+")
            }),
            TomlDialect::Poetry => !matches!(
                self.source_type.as_deref(),
                Some("directory" | "file" | "git" | "url")
            ),
            TomlDialect::Uv => self.source.as_deref().is_some_and(|s| {
                s.trim_start_matches('{')
                    .trim_start()
                    .starts_with("registry ")
            }),
        };
        Ok(published.then(|| pair(&name, &version)))
    }
}

/// `Cargo.lock`, `poetry.lock` and `uv.lock` all write the **same `[[package]]` shape**, so one
/// reader serves three files across two ecosystems. The region read is the `[[package]]` blocks
/// and their `[package.…]` sub-tables, of which only poetry's `[package.source]` is looked into:
/// every other table is skipped without being understood, and a block missing either field is a
/// construct this reader does not understand.
fn parse_toml_packages(text: &str, dialect: TomlDialect) -> LockfileRead {
    let mut out = Vec::new();
    let mut block: Option<TomlPackage> = None;
    // Inside a `[package.…]` sub-table, whose keys are not the package's own.
    let mut in_sub_table = false;
    let mut in_source_table = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') {
            let sub_table = trimmed != "[[package]]"
                && (trimmed.starts_with("[package.") || trimmed.starts_with("[[package."));
            if block.is_some() && sub_table {
                in_sub_table = true;
                in_source_table = trimmed == "[package.source]";
                continue;
            }
            if let Some(done) = block.take() {
                match done.close(dialect) {
                    Ok(published) => out.extend(published),
                    Err(()) => return LockfileRead::NotRead,
                }
            }
            in_sub_table = false;
            in_source_table = false;
            if trimmed == "[[package]]" {
                block = Some(TomlPackage::default());
            }
            continue;
        }
        let Some(current) = block.as_mut() else {
            continue;
        };
        if in_source_table {
            if let Some(value) = trimmed.strip_prefix("type = ") {
                current.source_type = Some(value.trim().trim_matches('"').to_owned());
            }
            continue;
        }
        if in_sub_table {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("name = ") {
            current.name = Some(value.trim().trim_matches('"').to_owned());
        } else if let Some(value) = trimmed.strip_prefix("version = ") {
            current.version = Some(value.trim().trim_matches('"').to_owned());
        } else if let Some(value) = trimmed.strip_prefix("source = ") {
            current.source = Some(value.trim().to_owned());
        }
    }
    if let Some(done) = block {
        match done.close(dialect) {
            Ok(published) => out.extend(published),
            Err(()) => return LockfileRead::NotRead,
        }
    }
    LockfileRead::Parsed(out)
}
