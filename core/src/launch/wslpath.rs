pub const DEFAULT_DRVFS_ROOT: &str = "/mnt";
const UNC_PREFIXES: [&str; 2] = [r"\\wsl.localhost\", r"\\wsl$\"];

/// Splits `\\wsl.localhost\<distro>\<rest>` into the distro and a Linux-shaped path.
#[must_use]
pub fn is_wsl_unc(win: &str) -> Option<(String, String)> {
    let rest = UNC_PREFIXES.iter().find_map(|p| win.strip_prefix(p))?;
    let (distro, tail) = match rest.split_once('\\') {
        Some((d, t)) => (d, t),
        None => (rest, ""),
    };
    if distro.is_empty() {
        return None;
    }
    let mut linux = String::from("/");
    linux.push_str(&tail.replace('\\', "/"));
    while linux.len() > 1 && linux.ends_with('/') {
        linux.pop();
    }
    Some((distro.to_owned(), linux))
}

#[must_use]
pub fn windows_to_wsl(win: &str, distro: &str, drvfs_root: &str) -> Option<String> {
    if let Some((found, linux)) = is_wsl_unc(win) {
        return if found.eq_ignore_ascii_case(distro) {
            Some(linux)
        } else {
            None
        };
    }
    if win.starts_with(r"\\") {
        return None; // an ordinary UNC share is not reachable from inside the distro
    }
    let mut chars = win.chars();
    let letter = chars.next()?.to_ascii_lowercase();
    if !letter.is_ascii_alphabetic() || chars.next()? != ':' {
        return None;
    }
    let tail = win.get(2..)?;
    if !tail.is_empty() && !tail.starts_with('\\') && !tail.starts_with('/') {
        return None; // "C:relative" is drive-relative, which has no absolute meaning here
    }
    let mut out = format!("{drvfs_root}/{letter}");
    let body = tail.trim_start_matches(['\\', '/']).replace('\\', "/");
    if !body.is_empty() {
        out.push('/');
        out.push_str(&body);
    }
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    Some(out)
}

#[must_use]
pub fn wsl_to_windows(linux: &str, distro: &str, drvfs_root: &str) -> Option<String> {
    if !linux.starts_with('/') {
        return None;
    }
    let root = drvfs_root.trim_end_matches('/');
    if let Some(after) = linux.strip_prefix(&format!("{root}/")) {
        let mut parts = after.splitn(2, '/');
        let letter = parts.next().unwrap_or_default();
        let mut it = letter.chars();
        if let (Some(c), None) = (it.next(), it.next()) {
            if c.is_ascii_alphabetic() {
                let drive = c.to_ascii_uppercase();
                let rest = parts.next().unwrap_or_default().replace('/', "\\");
                return Some(format!(r"{drive}:\{rest}"));
            }
        }
    }
    let body = linux.trim_start_matches('/').replace('/', "\\");
    Some(format!(r"\\wsl.localhost\{distro}\{body}"))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    #[test]
    fn drive_paths_map_onto_the_drvfs_root() {
        assert_eq!(
            windows_to_wsl(r"C:\code\thing", "distro-a", DEFAULT_DRVFS_ROOT).as_deref(),
            Some("/mnt/c/code/thing")
        );
        assert_eq!(
            windows_to_wsl(r"D:\", "distro-a", DEFAULT_DRVFS_ROOT).as_deref(),
            Some("/mnt/d")
        );
        assert_eq!(
            wsl_to_windows("/mnt/c/code/thing", "distro-a", DEFAULT_DRVFS_ROOT).as_deref(),
            Some(r"C:\code\thing")
        );
    }

    #[test]
    fn a_distro_internal_path_round_trips_through_the_unc_form() {
        let unc = wsl_to_windows("/home/u/w", "distro-a", DEFAULT_DRVFS_ROOT).unwrap();
        assert_eq!(unc, r"\\wsl.localhost\distro-a\home\u\w");
        assert_eq!(
            windows_to_wsl(&unc, "distro-a", DEFAULT_DRVFS_ROOT).as_deref(),
            Some("/home/u/w")
        );
    }

    #[test]
    fn the_legacy_unc_prefix_is_still_recognised() {
        assert_eq!(
            is_wsl_unc(r"\\wsl$\distro-b\srv\x"),
            Some(("distro-b".to_owned(), "/srv/x".to_owned()))
        );
    }

    #[test]
    fn a_unc_path_for_another_distro_does_not_translate() {
        assert_eq!(
            windows_to_wsl(r"\\wsl$\distro-b\srv\x", "distro-a", DEFAULT_DRVFS_ROOT),
            None
        );
    }

    #[test]
    fn a_relative_or_unrooted_path_is_refused_rather_than_guessed() {
        assert_eq!(
            windows_to_wsl(r"code\thing", "distro-a", DEFAULT_DRVFS_ROOT),
            None
        );
        assert_eq!(
            wsl_to_windows("home/u", "distro-a", DEFAULT_DRVFS_ROOT),
            None
        );
        assert_eq!(
            windows_to_wsl(r"\\server\share\x", "distro-a", DEFAULT_DRVFS_ROOT),
            None
        );
    }

    #[test]
    fn the_drvfs_root_is_a_parameter_not_a_constant() {
        assert_eq!(
            windows_to_wsl(r"C:\x", "distro-a", "/windows").as_deref(),
            Some("/windows/c/x")
        );
        assert_eq!(
            wsl_to_windows("/windows/c/x", "distro-a", "/windows").as_deref(),
            Some(r"C:\x")
        );
        // With a different root configured, /mnt is an ordinary distro path.
        assert_eq!(
            wsl_to_windows("/mnt/c/x", "distro-a", "/windows").as_deref(),
            Some(r"\\wsl.localhost\distro-a\mnt\c\x")
        );
    }
}
