//! Lossless WebP. `image-webp` is pure Rust, so there is no C dependency to build for two
//! targets and no `unsafe` to justify against the crate's own `forbid`.
//!
//! Lossless matters beyond file size: §7.4's walk back promises offset *n*−1 re-derives
//! **byte-identically**, and that is only true end to end if the same pixels always encode to
//! the same bytes.

use image_webp::{ColorType, WebPEncoder};
use tiny_skia::Pixmap;

use crate::art::ArtError;

/// tiny-skia stores **premultiplied** RGBA; the encoder is handed straight RGBA. The two are the
/// same bytes only while every pixel is opaque, which the card always is — so rather than
/// demultiplying, the precondition is checked and a violation is an error, not a silent shift.
pub fn encode_webp(pixmap: &Pixmap) -> Result<Vec<u8>, ArtError> {
    if let Some(pixel) = pixmap.pixels().iter().find(|p| p.alpha() != 255) {
        return Err(ArtError::Encode(format!(
            "card art must be opaque; found alpha {}",
            pixel.alpha()
        )));
    }
    let mut out = Vec::new();
    WebPEncoder::new(&mut out)
        .encode(
            pixmap.data(),
            pixmap.width(),
            pixmap.height(),
            ColorType::Rgba8,
        )
        .map_err(|e| ArtError::Encode(e.to_string()))?;
    Ok(out)
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
    use crate::art::generate::{generate, SceneInputs};
    use crate::art::raster::{render, CARD_TARGET};

    fn card() -> Pixmap {
        let scene = generate(&SceneInputs {
            seed_basename: "alpha-tool".to_owned(),
            size_tracked_bytes: Some(2_000_000),
            ..SceneInputs::default()
        });
        render(&scene, CARD_TARGET).expect("render")
    }

    #[test]
    fn the_encoder_produces_a_riff_webp_container() {
        let bytes = encode_webp(&card()).expect("encode");
        assert_eq!(bytes.get(0..4), Some(b"RIFF".as_slice()));
        assert_eq!(bytes.get(8..12), Some(b"WEBP".as_slice()));
        // VP8L is the lossless chunk. A lossy encoder would write VP8 (with a trailing space).
        assert_eq!(bytes.get(12..16), Some(b"VP8L".as_slice()));
    }

    #[test]
    fn the_same_pixmap_encodes_to_the_same_bytes() {
        let a = encode_webp(&card()).expect("a");
        let b = encode_webp(&card()).expect("b");
        assert_eq!(a, b);
    }

    #[test]
    fn a_transparent_pixel_is_refused_rather_than_silently_premultiplied() {
        // tiny_skia keeps premultiplied bytes; the encoder is handed straight ones. The two
        // agree only while the card is opaque, so a non-opaque pixmap must not be encoded.
        let mut pm = Pixmap::new(4, 4).expect("pixmap");
        pm.fill(tiny_skia::Color::from_rgba8(200, 100, 50, 128));
        assert!(encode_webp(&pm).is_err());
    }
}
