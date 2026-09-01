//! §11.4: "basenames and volume shapes". Paths *are* the product, so a bundle
//! without them is useless; a bundle with them leaks the whole directory tree.
//! The shape keeps the basename, the depth and a per-volume pseudonym.

use std::collections::BTreeMap;

/// [R27] `location.volume_key` is nullable, and NULL means *no stable identifier exists* —
/// a bind mount, overlayfs, tmpfs. Giving those a number would claim they are one volume, so
/// they get this instead and consume no number.
pub const UNKNOWN_VOLUME: &str = "vol-?";

/// Stable per-run pseudonyms for `location.volume_key`, in first-seen order.
#[derive(Debug, Default)]
pub struct VolumeShapes {
    seen: BTreeMap<String, String>,
}

impl VolumeShapes {
    #[must_use]
    pub fn new() -> VolumeShapes {
        VolumeShapes::default()
    }

    /// The pseudonym for one volume key, stable for the life of this bundle.
    pub fn shape(&mut self, volume_key: &str) -> String {
        if volume_key.is_empty() {
            return UNKNOWN_VOLUME.to_owned();
        }
        let next = self.seen.len() + 1;
        self.seen
            .entry(volume_key.to_owned())
            .or_insert_with(|| format!("vol-{next}"))
            .clone()
    }
}

/// `/one/two/three/alpha` under `vol-1` becomes `vol-1/…3…/alpha`.
///
/// Both separators are folded, so a drive letter is a segment like any other and does not
/// survive as a name.
#[must_use]
pub fn anonymise_path(display: &str, volume: &str) -> String {
    let segments: Vec<&str> = display
        .split(['/', '\\'])
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    match segments.split_last() {
        None => volume.to_owned(),
        Some((basename, [])) => format!("{volume}/{basename}"),
        Some((basename, rest)) => format!("{volume}/…{}…/{basename}", rest.len()),
    }
}
