//! Canonical remote keys (§1.1).

/// A canonical remote (§1.1): `<host>/<owner>/<name>`, lowercased, `.git` stripped, ssh and
/// https normalised to one form, port and trailing slash removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteKey {
    /// The stored value of `project.remote_key`.
    pub key: String,
    pub host: String,
    /// The first path segment. Two projects on one lineage whose owners differ are a fork
    /// (§1.1), so this is the field that decides it.
    pub owner: String,
    /// The last path segment.
    pub name: String,
}

/// `None` means the URL carries no remote identity — a local path, a `file://` URL, or a URL
/// with fewer than two path segments. It never means "no remote": a project with no remote at
/// all stores NULL, and NULL is what §1.1's weak row keys on.
#[must_use]
pub fn canonical_remote_key(url: &str) -> Option<RemoteKey> {
    let trimmed = url.trim();
    let (authority, path) = split_authority_and_path(trimmed)?;
    let host = host_of(authority)?;

    let mut segments: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if segments.len() < 2 {
        return None;
    }
    if let Some(last) = segments.last_mut() {
        if let Some(stripped) = last.strip_suffix(".git") {
            *last = stripped.to_owned();
        }
    }
    // `.git` alone strips to nothing, and an empty segment would make two different remotes
    // produce the same key.
    if segments.iter().any(String::is_empty) {
        return None;
    }

    let owner = segments.first()?.clone();
    let name = segments.last()?.clone();
    let key = format!("{host}/{}", segments.join("/"));
    Some(RemoteKey {
        key,
        host,
        owner,
        name,
    })
}

fn split_authority_and_path(url: &str) -> Option<(&str, &str)> {
    if let Some((scheme, rest)) = url.split_once("://") {
        if scheme.eq_ignore_ascii_case("file") {
            return None;
        }
        return rest.split_once('/');
    }
    // scp-like `user@host:owner/repo.git`: a colon that comes before any slash.
    let colon = url.find(':')?;
    if url.find('/').is_some_and(|slash| slash < colon) {
        return None;
    }
    let (authority, rest) = url.split_at(colon);
    let path = rest.strip_prefix(':')?;
    // A single character before the colon is a Windows drive letter, not a host.
    let hostish = authority.rsplit('@').next().unwrap_or(authority);
    if hostish.chars().count() <= 1 {
        return None;
    }
    Some((authority, path))
}

fn host_of(authority: &str) -> Option<String> {
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = match host.split_once(':') {
        // A numeric suffix is a port and is removed.
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
        Some(_) => return None,
        None => host,
    };
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// The argv that lists a repository's remote URLs. `--null` is used because a URL may contain
/// anything a newline can, and a config key with no matches exits **1 with empty output** —
/// which is not an error and must not be reported as one.
#[must_use]
pub fn remote_urls_argv() -> Vec<&'static str> {
    vec!["config", "--null", "--get-regexp", r"^remote\..*\.url$"]
}

/// Parses `git config --null --get-regexp` output: `key\nvalue\0` repeated.
#[must_use]
pub fn parse_remote_urls(stdout: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(stdout);
    text.split('\0')
        .filter(|rec| !rec.is_empty())
        .filter_map(|rec| {
            let (key, value) = rec.split_once('\n')?;
            let name = key.strip_prefix("remote.")?.strip_suffix(".url")?;
            if name.is_empty() {
                return None;
            }
            Some((name.to_owned(), value.to_owned()))
        })
        .collect()
}

/// §1.1 lists "`origin`+`upstream` forks with no rule for which remote is canonical" among the
/// combinations the old model mishandled, and supplies no rule. This is it: `origin`, then
/// `upstream`, then the lexicographically first remote whose URL canonicalises. Deterministic
/// and independent of the order git happened to print them in.
#[must_use]
pub fn pick_canonical_remote(remotes: &[(String, String)]) -> Option<RemoteKey> {
    let mut named: Vec<(&str, RemoteKey)> = remotes
        .iter()
        .filter_map(|(n, u)| canonical_remote_key(u).map(|k| (n.as_str(), k)))
        .collect();
    named.sort_by(|a, b| a.0.cmp(b.0));

    for preferred in ["origin", "upstream"] {
        if let Some((_, k)) = named.iter().find(|(n, _)| *n == preferred) {
            return Some(k.clone());
        }
    }
    named.into_iter().next().map(|(_, k)| k)
}

/// The owner segment of a stored `project.remote_key`, so a fork test needs no second column.
#[must_use]
pub fn owner_of(key: &str) -> Option<&str> {
    let (_host, rest) = key.split_once('/')?;
    let owner = rest.split('/').next()?;
    if owner.is_empty() {
        None
    } else {
        Some(owner)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::canonical_remote_key as k;

    #[test]
    fn ssh_and_https_normalise_to_one_form() {
        let expected = "forge.example/acme/widget";
        for url in [
            "https://forge.example/acme/widget.git",
            "https://forge.example/Acme/Widget.git",
            "https://forge.example/acme/widget/",
            "http://forge.example:8080/acme/widget",
            "git@forge.example:acme/widget.git",
            "ssh://git@forge.example:22/acme/widget.git",
            "git://forge.example/acme/widget.git",
            "  https://forge.example/acme/widget.git  ",
        ] {
            assert_eq!(k(url).map(|r| r.key), Some(expected.to_owned()), "{url}");
        }
    }

    #[test]
    fn owner_and_name_are_the_first_and_last_path_segments() {
        let r = k("https://forge.example/group/sub/widget.git").unwrap();
        assert_eq!(r.key, "forge.example/group/sub/widget");
        assert_eq!(r.host, "forge.example");
        assert_eq!(r.owner, "group");
        assert_eq!(r.name, "widget");
    }

    #[test]
    fn a_url_with_no_host_or_no_owner_has_no_key() {
        // A key is <host>/<owner>/<name>. Anything that cannot supply all three is None, and
        // None means "no remote evidence" — never an empty string, which would compare equal
        // to another repository that also has none.
        for url in [
            "",
            "   ",
            "/srv/git/widget.git",
            "../sibling",
            "file:///srv/git/widget.git",
            "https://forge.example/widget.git",
            "C:\\repos\\widget",
            "git@forge.example:widget.git",
        ] {
            assert_eq!(k(url), None, "{url}");
        }
    }

    fn r(name: &str, url: &str) -> (String, String) {
        (name.to_owned(), url.to_owned())
    }

    #[test]
    fn origin_is_canonical_then_upstream_then_the_first_by_name() {
        // A fork's own URL is its `origin`. Taking `upstream` would give the fork the same
        // remote_key as its parent, which is exactly the collapse §1.1 forbids.
        let both = [
            r("upstream", "https://forge.example/acme/widget.git"),
            r("origin", "https://forge.example/mine/widget.git"),
        ];
        assert_eq!(
            super::pick_canonical_remote(&both).map(|k| k.key),
            Some("forge.example/mine/widget".to_owned())
        );

        let no_origin = [
            r("upstream", "https://forge.example/acme/widget.git"),
            r("zeta", "https://forge.example/z/widget.git"),
        ];
        assert_eq!(
            super::pick_canonical_remote(&no_origin).map(|k| k.key),
            Some("forge.example/acme/widget".to_owned())
        );

        let neither = [
            r("zeta", "https://forge.example/z/widget.git"),
            r("alpha", "https://forge.example/a/widget.git"),
        ];
        assert_eq!(
            super::pick_canonical_remote(&neither).map(|k| k.key),
            Some("forge.example/a/widget".to_owned())
        );
    }

    #[test]
    fn a_remote_that_does_not_canonicalise_is_skipped_not_stored() {
        let mixed = [
            r("origin", "/srv/git/widget.git"),
            r("mirror", "git@forge.example:a/w.git"),
        ];
        assert_eq!(
            super::pick_canonical_remote(&mixed).map(|k| k.key),
            Some("forge.example/a/w".to_owned())
        );
        assert_eq!(super::pick_canonical_remote(&[]), None);
    }

    #[test]
    fn owner_of_reads_the_segment_after_the_host() {
        assert_eq!(super::owner_of("forge.example/acme/widget"), Some("acme"));
        assert_eq!(
            super::owner_of("forge.example/group/sub/widget"),
            Some("group")
        );
        assert_eq!(super::owner_of("forge.example"), None);
    }

    #[test]
    fn remote_urls_are_read_nul_separated_from_git_config() {
        let out = b"remote.origin.url\nhttps://forge.example/a/w.git\0\
                    remote.upstream.url\nssh://git@forge.example/b/w.git\0";
        assert_eq!(
            super::parse_remote_urls(out),
            vec![
                (
                    "origin".to_owned(),
                    "https://forge.example/a/w.git".to_owned()
                ),
                (
                    "upstream".to_owned(),
                    "ssh://git@forge.example/b/w.git".to_owned()
                ),
            ]
        );
        assert_eq!(super::parse_remote_urls(b""), Vec::new());
    }
}
