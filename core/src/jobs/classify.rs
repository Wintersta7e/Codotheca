//! Language and archetype, derived from the tracked path set (§4.1 J3, §1.2).
//!
//! Nothing here reads a file's contents. J3 already enumerates the tracked paths and their HEAD
//! blob sizes, and that is the whole input.

use std::collections::BTreeMap;

/// One recognised language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lang {
    /// The canonical name. §4bis.2a matches launch targets on this exact string.
    pub name: &'static str,
    /// Markup, styling and data are recorded in `language_bytes` and can never be primary.
    pub programming: bool,
}

const fn prog(name: &'static str) -> Lang {
    Lang {
        name,
        programming: true,
    }
}
const fn markup(name: &'static str) -> Lang {
    Lang {
        name,
        programming: false,
    }
}

/// `.h` is ambiguous between C and C++; the common convention wins and it counts as C.
const BY_EXT: &[(&str, Lang)] = &[
    ("rs", prog("Rust")),
    ("ts", prog("TypeScript")),
    ("tsx", prog("TypeScript")),
    ("js", prog("JavaScript")),
    ("jsx", prog("JavaScript")),
    ("mjs", prog("JavaScript")),
    ("cjs", prog("JavaScript")),
    ("py", prog("Python")),
    ("pyi", prog("Python")),
    ("cpp", prog("C++")),
    ("cc", prog("C++")),
    ("cxx", prog("C++")),
    ("hpp", prog("C++")),
    ("hh", prog("C++")),
    ("hxx", prog("C++")),
    ("c", prog("C")),
    ("h", prog("C")),
    ("cs", prog("C#")),
    ("java", prog("Java")),
    ("go", prog("Go")),
    ("sh", prog("Shell")),
    ("bash", prog("Shell")),
    ("zsh", prog("Shell")),
    ("lua", prog("Lua")),
    ("rb", prog("Ruby")),
    ("kt", prog("Kotlin")),
    ("kts", prog("Kotlin")),
    ("swift", prog("Swift")),
    ("php", prog("PHP")),
    ("md", markup("Markdown")),
    ("rst", markup("Markdown")),
    ("html", markup("HTML")),
    ("css", markup("CSS")),
    ("scss", markup("CSS")),
    ("json", markup("JSON")),
    ("yml", markup("YAML")),
    ("yaml", markup("YAML")),
    ("toml", markup("TOML")),
];

fn base_of(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, b)| b)
}

/// The lowercased extension of a path's **basename**, or `None` when it has none.
///
/// From the basename because a dot in a directory name is not an extension: reading
/// `a.b/Makefile` as extension `b/Makefile` files it under nothing while looking classified.
/// Lowercased because a filesystem that preserves case still means `README.MD` is a README.
fn ext_of(path: &str) -> Option<String> {
    base_of(path)
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
}

/// Whether a path carries this extension, case-folded.
fn ext_is(path: &str, want: &str) -> bool {
    ext_of(path).is_some_and(|e| e == want)
}

/// Which language a tracked path belongs to, or `None` when nothing is recognised.
#[must_use]
pub fn language_of_path(path: &str) -> Option<Lang> {
    let ext = ext_of(path)?;
    BY_EXT.iter().find(|(e, _)| *e == ext).map(|(_, l)| *l)
}

/// Every distinct **programming** language name `BY_EXT` declares, sorted.
///
/// §29.2's rule 3 gives `BY_EXT` a second reader, and it was written for the language byte
/// census: someone adding an extension there to make a language appear in the language bar would
/// silently widen what this product reads off the user's disk. The consent surface renders this
/// list, which makes that a visible change to a rendered policy rather than a table edit.
///
/// **No count is written anywhere** — every checker derives both sides.
#[must_use]
pub fn programming_languages() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = BY_EXT
        .iter()
        .filter(|(_, lang)| lang.programming)
        .map(|(_, lang)| lang.name)
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The largest **programming** language by HEAD blob bytes.
///
/// `None` is *not computed* / *nothing recognised* and must never resolve a language row or be
/// coalesced into "any" (§4bis.2a). Ties break on the name so two scans of the same tree agree.
#[must_use]
pub fn primary_language(bytes: &BTreeMap<String, u64>) -> Option<String> {
    bytes
        .iter()
        .filter(|(name, _)| {
            BY_EXT
                .iter()
                .any(|(_, l)| l.name == name.as_str() && l.programming)
        })
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(name, _)| name.clone())
}

fn has(paths: &[String], pred: impl Fn(&str) -> bool) -> bool {
    paths.iter().any(|p| pred(p.as_str()))
}

fn all(paths: &[String], pred: impl Fn(&str) -> bool) -> bool {
    !paths.is_empty() && paths.iter().all(|p| pred(p.as_str()))
}

const MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "go.mod",
    "CMakeLists.txt",
    "Makefile",
];

/// The project's shape, from its tracked paths.
///
/// First match wins. Reads no file contents — only the tracked path set, which is exactly what
/// J3 already enumerates. `"unclassified"` means J3 ran and nothing matched; a NULL column means
/// J3 has not run. Two different states, and a surface may not render one as the other.
#[must_use]
pub fn archetype_of(paths: &[String]) -> &'static str {
    if has(paths, |p| ext_is(p, "ipynb")) {
        return "notebook";
    }
    if has(paths, |p| {
        matches!(base_of(p), "Dockerfile" | "Containerfile")
    }) {
        return "service";
    }
    if has(paths, |p| base_of(p) == "index.html") {
        return "site";
    }
    let entry = |p: &str| {
        matches!(
            p,
            "src/main.rs" | "main.go" | "main.py" | "__main__.py" | "Program.cs" | "src/main.c"
        ) || p.ends_with("/main.go")
            || p.ends_with("/Program.cs")
    };
    let manifest = |p: &str| MANIFESTS.contains(&base_of(p)) || ext_is(p, "csproj");
    if has(paths, entry) {
        return "cli";
    }
    if has(paths, manifest) {
        return "library";
    }
    let docish = |p: &str| {
        ext_is(p, "md")
            || ext_is(p, "rst")
            || ext_is(p, "txt")
            || base_of(p) == "LICENSE"
            || p.starts_with("docs/")
    };
    if all(paths, docish) {
        return "docs";
    }
    let configish = |p: &str| {
        base_of(p).starts_with('.')
            || ext_is(p, "yml")
            || ext_is(p, "yaml")
            || ext_is(p, "toml")
            || ext_is(p, "ini")
            || ext_is(p, "conf")
            || ext_is(p, "nix")
    };
    if all(paths, configish) {
        return "config";
    }
    "unclassified"
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn canonical_language_names_are_the_ones_launch_targets_match_on() {
        // §4bis.2a: the stored value is project.primary_language, never the drawer's tag.
        assert_eq!(language_of_path("src/lib.rs").map(|l| l.name), Some("Rust"));
        assert_eq!(
            language_of_path("a/b.tsx").map(|l| l.name),
            Some("TypeScript")
        );
        assert_eq!(language_of_path("a/b.py").map(|l| l.name), Some("Python"));
        assert_eq!(language_of_path("a/b.cpp").map(|l| l.name), Some("C++"));
        assert_eq!(language_of_path("a/b.cs").map(|l| l.name), Some("C#"));
        assert_eq!(language_of_path("a/b.unknownext"), None);
    }

    /// A dot in a directory name is not an extension. Taking it as one silently classifies
    /// nothing while looking like it classified something.
    #[test]
    fn the_extension_comes_from_the_basename_not_the_whole_path() {
        assert_eq!(
            language_of_path("v1.2/main.rs").map(|l| l.name),
            Some("Rust")
        );
        assert_eq!(language_of_path("a.b/Makefile"), None);
        assert_eq!(language_of_path(".gitignore"), None);
    }

    #[test]
    fn markup_is_recorded_but_can_never_be_the_primary_language() {
        let mut bytes = BTreeMap::new();
        bytes.insert("Markdown".to_owned(), 900_000_u64);
        bytes.insert("Rust".to_owned(), 12_u64);
        assert_eq!(primary_language(&bytes).as_deref(), Some("Rust"));
    }

    #[test]
    fn a_repository_with_no_programming_language_has_none_not_a_default() {
        // §4bis.2a: NULL is *not computed* / *nothing recognised*, and must never resolve a
        // language row. It is never coalesced into "any".
        let mut bytes = BTreeMap::new();
        bytes.insert("Markdown".to_owned(), 900_000_u64);
        assert_eq!(primary_language(&bytes), None);
        assert_eq!(primary_language(&BTreeMap::new()), None);
    }

    #[test]
    fn ties_break_on_the_language_name_so_two_scans_agree() {
        let mut bytes = BTreeMap::new();
        bytes.insert("Rust".to_owned(), 100_u64);
        bytes.insert("Go".to_owned(), 100_u64);
        assert_eq!(primary_language(&bytes).as_deref(), Some("Go"));
    }

    #[test]
    fn archetypes_are_decided_from_the_tracked_path_set_alone() {
        assert_eq!(
            archetype_of(&paths(&["analysis.ipynb", "README.md"])),
            "notebook"
        );
        assert_eq!(archetype_of(&paths(&["Dockerfile", "main.go"])), "service");
        assert_eq!(archetype_of(&paths(&["index.html", "style.css"])), "site");
        assert_eq!(archetype_of(&paths(&["Cargo.toml", "src/main.rs"])), "cli");
        assert_eq!(
            archetype_of(&paths(&["Cargo.toml", "src/lib.rs"])),
            "library"
        );
        assert_eq!(archetype_of(&paths(&["README.md", "docs/a.md"])), "docs");
        assert_eq!(
            archetype_of(&paths(&[".gitignore", "config.toml"])),
            "config"
        );
    }

    #[test]
    fn nothing_recognised_is_unclassified_not_null() {
        // NULL means J3 has not run. `unclassified` means it ran and nothing matched — two
        // different states, and a surface may not render one as the other.
        assert_eq!(archetype_of(&paths(&["a.bin", "b.bin"])), "unclassified");
    }

    /// A case-preserving filesystem still means `README.MD` is a README.
    #[test]
    fn extensions_are_matched_case_folded() {
        assert_eq!(language_of_path("A/B.RS").map(|l| l.name), Some("Rust"));
        assert_eq!(archetype_of(&paths(&["README.MD", "docs/A.RST"])), "docs");
        assert_eq!(archetype_of(&paths(&["Analysis.IPYNB"])), "notebook");
    }

    /// `all` over an empty slice is vacuously true, which would file an empty repository under
    /// `docs`. An empty tracked set recognised nothing.
    #[test]
    fn an_empty_path_set_is_unclassified_not_vacuously_docs() {
        assert_eq!(archetype_of(&[]), "unclassified");
    }
}
