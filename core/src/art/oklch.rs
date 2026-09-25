//! OKLCH → sRGB, gamut-mapped the way Chromium maps it (CSS Color 4 §13.2: chroma reduction
//! until ΔEOK ≤ 0.02). §7.3 pins eight jewel bins to their mapped values and prints the naively
//! clipped column beside them precisely so the difference is testable: **matching the clipped
//! column is a bug, not a rounding difference.**
//!
//! Both producers of the card — this rasterizer and the CSS plate — must land on the same
//! colour (§7.1a, "one design, two producers"). The browser maps; so does this.

/// CSS Color 4's just-noticeable difference in `OKLab`.
pub const JND: f64 = 0.02;
/// The bisection's stopping width, from the same algorithm.
pub const MAP_EPSILON: f64 = 0.0001;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Srgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

fn oklch_to_oklab(color: Oklch) -> (f64, f64, f64) {
    let rad = color.h.to_radians();
    (color.l, color.c * rad.cos(), color.c * rad.sin())
}

fn oklab_to_linear_srgb(lab: (f64, f64, f64)) -> (f64, f64, f64) {
    let (lightness, green_red, blue_yellow) = lab;
    let long = 0.215_803_757_3f64.mul_add(
        blue_yellow,
        0.396_337_777_4f64.mul_add(green_red, lightness),
    );
    let medium = 0.063_854_172_8f64.mul_add(
        -blue_yellow,
        0.105_561_345_8f64.mul_add(-green_red, lightness),
    );
    let short = 1.291_485_548_0f64.mul_add(
        -blue_yellow,
        0.089_484_177_5f64.mul_add(-green_red, lightness),
    );
    let (long, medium, short) = (
        long * long * long,
        medium * medium * medium,
        short * short * short,
    );
    (
        0.230_969_929_2f64.mul_add(
            short,
            3.307_711_591_3f64.mul_add(-medium, 4.076_741_662_1 * long),
        ),
        0.341_319_396_5f64.mul_add(
            -short,
            2.609_757_401_1f64.mul_add(medium, -1.268_438_004_6 * long),
        ),
        1.707_614_701_0f64.mul_add(
            short,
            0.703_418_614_7f64.mul_add(-medium, -0.004_196_086_3 * long),
        ),
    )
}

fn linear_srgb_to_oklab(rgb: (f64, f64, f64)) -> (f64, f64, f64) {
    let (red, green, blue) = rgb;
    let long = 0.051_445_992_9f64.mul_add(
        blue,
        0.536_332_536_3f64.mul_add(green, 0.412_221_470_8 * red),
    );
    let medium = 0.107_396_956_6f64.mul_add(
        blue,
        0.680_699_545_1f64.mul_add(green, 0.211_903_498_2 * red),
    );
    let short = 0.629_978_700_5f64.mul_add(
        blue,
        0.281_718_837_6f64.mul_add(green, 0.088_302_461_9 * red),
    );
    let (long, medium, short) = (long.cbrt(), medium.cbrt(), short.cbrt());
    (
        0.004_072_046_8f64.mul_add(
            -short,
            0.793_617_785_0f64.mul_add(medium, 0.210_454_255_3 * long),
        ),
        0.450_593_709_9f64.mul_add(
            short,
            2.428_592_205_0f64.mul_add(-medium, 1.977_998_495_1 * long),
        ),
        0.808_675_766_0f64.mul_add(
            -short,
            0.782_771_766_2f64.mul_add(medium, 0.025_904_037_1 * long),
        ),
    )
}

fn in_gamut(rgb: (f64, f64, f64)) -> bool {
    let (red, green, blue) = rgb;
    (0.0..=1.0).contains(&red) && (0.0..=1.0).contains(&green) && (0.0..=1.0).contains(&blue)
}

fn clip(rgb: (f64, f64, f64)) -> (f64, f64, f64) {
    let (red, green, blue) = rgb;
    (
        red.clamp(0.0, 1.0),
        green.clamp(0.0, 1.0),
        blue.clamp(0.0, 1.0),
    )
}

fn delta_e_ok(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let dl = a.0 - b.0;
    let da = a.1 - b.1;
    let db = a.2 - b.2;
    dl.mul_add(dl, da.mul_add(da, db * db)).sqrt()
}

/// CSS Color 4 §13.2, transcribed. Binary-search the chroma down until the clipped result is
/// within one JND of the reduced colour.
fn map_to_gamut(color: Oklch) -> (f64, f64, f64) {
    if color.l >= 1.0 {
        return (1.0, 1.0, 1.0);
    }
    if color.l <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let origin = oklch_to_oklab(color);
    let rgb = oklab_to_linear_srgb(origin);
    if in_gamut(rgb) {
        return clip(rgb);
    }
    let mut clipped = clip(rgb);
    if delta_e_ok(linear_srgb_to_oklab(clipped), origin) < JND {
        return clipped;
    }
    let mut low = 0.0_f64;
    let mut high = color.c;
    let mut low_in_gamut = true;
    while high - low > MAP_EPSILON {
        let chroma = (low + high) / 2.0;
        let current = oklch_to_oklab(Oklch { c: chroma, ..color });
        let current_rgb = oklab_to_linear_srgb(current);
        if low_in_gamut && in_gamut(current_rgb) {
            low = chroma;
            continue;
        }
        clipped = clip(current_rgb);
        let error = delta_e_ok(linear_srgb_to_oklab(clipped), current);
        if error < JND {
            if JND - error < MAP_EPSILON {
                return clipped;
            }
            low_in_gamut = false;
            low = chroma;
        } else {
            high = chroma;
        }
    }
    clipped
}

fn encode_gamma(channel: f64) -> f64 {
    if channel <= 0.003_130_8 {
        12.92 * channel
    } else {
        1.055f64.mul_add(channel.powf(1.0 / 2.4), -0.055)
    }
}

// The value is clamped to 0..=1 before the cast, so the truncation clippy warns about is the
// rounding we asked for.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_byte(channel: f64) -> u8 {
    (encode_gamma(channel) * 255.0).round().clamp(0.0, 255.0) as u8
}

fn pack(rgb: (f64, f64, f64)) -> Srgb8 {
    Srgb8 {
        r: to_byte(rgb.0),
        g: to_byte(rgb.1),
        b: to_byte(rgb.2),
    }
}

/// The value every surface must agree on.
#[must_use]
pub fn to_srgb8(color: Oklch) -> Srgb8 {
    pack(map_to_gamut(color))
}

/// Whether the colour is representable in sRGB at all, before any mapping or rounding.
///
/// §7.3 states five of the eight jewel bins are outside sRGB at chroma `0.175`, and that is a
/// claim about the *gamut*. It is not the same as the two hex columns disagreeing: bin 148 sits
/// outside by less than one 8-bit step, so it maps and clips to one value. Without this
/// predicate the spec's sentence can only be tested through that proxy, which answers 4.
#[must_use]
pub fn is_in_srgb_gamut(color: Oklch) -> bool {
    in_gamut(oklab_to_linear_srgb(oklch_to_oklab(color)))
}

/// The *wrong* answer, exported only so the test can prove this module does not produce it.
#[must_use]
pub fn clip_srgb8(color: Oklch) -> Srgb8 {
    pack(clip(oklab_to_linear_srgb(oklch_to_oklab(color))))
}

#[must_use]
pub fn hex(c: Srgb8) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// §7.3's serialisation: three decimals for lightness and chroma, an integer hue.
#[allow(clippy::cast_possible_truncation)]
#[must_use]
pub fn css(color: Oklch) -> String {
    let hue = color.h.round();
    format!("oklch({:.3} {:.3} {})", color.l, color.c, hue as i64)
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

    const BINS: [u32; 8] = [26, 58, 96, 148, 188, 232, 284, 328];

    /// §7.3's table: bin hue, the mapped colour, the naively clipped one, and `jewelInk`.
    const TABLE: [(u32, &str, &str, &str); 8] = [
        (26, "#d54a45", "#d54a45", "#ffc3bc"),
        (58, "#c66000", "#c95d00", "#f8caa7"),
        (96, "#9a7e00", "#a07c00", "#e2d5a0"),
        (148, "#009b3c", "#009b3c", "#b6e2bb"),
        (188, "#00978e", "#009f94", "#9ee4dd"),
        (232, "#008dc7", "#008ed5", "#a6ddfb"),
        (284, "#776ce3", "#776ce3", "#cecfff"),
        (328, "#b553b4", "#b553b4", "#eec5ec"),
    ];

    #[test]
    fn every_jewel_bin_reproduces_the_mapped_column() {
        for (hue, mapped, _, _) in TABLE {
            let got = hex(to_srgb8(Oklch {
                l: 0.600,
                c: 0.175,
                h: f64::from(hue),
            }));
            assert_eq!(got, mapped, "bin {hue} must map, not clip");
        }
    }

    #[test]
    fn five_of_the_eight_bins_are_outside_srgb_and_clipping_is_visibly_wrong() {
        let mut outside = 0;
        let mut differ = 0;
        for (hue, mapped, clipped, _) in TABLE {
            let color = Oklch {
                l: 0.600,
                c: 0.175,
                h: f64::from(hue),
            };
            assert_eq!(hex(clip_srgb8(color)), clipped, "bin {hue} naive clip");
            if !is_in_srgb_gamut(color) {
                outside += 1;
            }
            if mapped != clipped {
                differ += 1;
            }
        }
        // §7.3: "Five of the eight jewel bins are outside sRGB at C 0.175". This is the claim
        // the spec makes, and it is about the gamut, not about the rendered byte triple.
        assert_eq!(outside, 5, "bins outside sRGB at C 0.175");
        // **Four**, not five, and the gap is the point. Bin 148 is outside the gamut by less
        // than one 8-bit step (linear red -0.0016), so mapping and clipping round to the same
        // `#009b3c` and the error is invisible. Counting differing hex columns is a *proxy* for
        // the gamut question and answers it wrongly; asserting 5 here would be a bar written
        // past the defect, passing only if the mapper were broken into disagreeing at 148.
        assert_eq!(differ, 4, "bins where the visible byte triple differs");
    }

    #[test]
    fn jewel_ink_is_in_gamut_at_every_bin() {
        for (hue, _, _, ink) in TABLE {
            let got = hex(to_srgb8(Oklch {
                l: 0.87,
                c: 0.07,
                h: f64::from(hue),
            }));
            assert_eq!(got, ink, "jewelInk at bin {hue}");
            // In gamut means mapping and clipping agree.
            assert_eq!(
                got,
                hex(clip_srgb8(Oklch {
                    l: 0.87,
                    c: 0.07,
                    h: f64::from(hue)
                }))
            );
        }
    }

    #[test]
    fn the_three_plate_stops_are_in_gamut() {
        // §7.3: plate at step 0, hue 76.
        assert_eq!(
            hex(to_srgb8(Oklch {
                l: 0.185,
                c: 0.005,
                h: 76.0
            })),
            "#141210"
        );
        assert_eq!(
            hex(to_srgb8(Oklch {
                l: 0.145,
                c: 0.005,
                h: 76.0
            })),
            "#0b0a08"
        );
        assert_eq!(
            hex(to_srgb8(Oklch {
                l: 0.085,
                c: 0.004,
                h: 76.0
            })),
            "#020201"
        );
    }

    #[test]
    fn the_lightness_extremes_short_circuit_rather_than_bisecting_forever() {
        assert_eq!(
            to_srgb8(Oklch {
                l: 1.0,
                c: 0.4,
                h: 0.0
            }),
            Srgb8 {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(
            to_srgb8(Oklch {
                l: 0.0,
                c: 0.4,
                h: 0.0
            }),
            Srgb8 { r: 0, g: 0, b: 0 }
        );
        assert_eq!(
            to_srgb8(Oklch {
                l: -0.5,
                c: 0.0,
                h: 0.0
            }),
            Srgb8 { r: 0, g: 0, b: 0 }
        );
    }

    #[test]
    fn the_hue_wraps_so_a_jittered_bin_never_leaves_the_wheel() {
        // §7.3a jitters a bin by up to ±3, and bin 26 - 3 = 23 while 328 + 3 = 331; but a
        // future table could sit on 0 or 360, and cos/sin must not care.
        assert_eq!(
            to_srgb8(Oklch {
                l: 0.6,
                c: 0.1,
                h: 0.0
            }),
            to_srgb8(Oklch {
                l: 0.6,
                c: 0.1,
                h: 360.0
            })
        );
        assert_eq!(
            to_srgb8(Oklch {
                l: 0.6,
                c: 0.1,
                h: -10.0
            }),
            to_srgb8(Oklch {
                l: 0.6,
                c: 0.1,
                h: 350.0
            })
        );
    }

    #[test]
    fn the_css_form_is_three_decimals_and_an_integer_hue() {
        // §7.3: "Lightness and chroma serialise to three decimals; hue … to integers."
        assert_eq!(
            css(Oklch {
                l: 0.6,
                c: 0.155,
                h: 330.0
            }),
            "oklch(0.600 0.155 330)"
        );
        assert_eq!(
            css(Oklch {
                l: 0.87,
                c: 0.07,
                h: 26.0
            }),
            "oklch(0.870 0.070 26)"
        );
    }

    #[test]
    fn every_bin_stays_in_gamut_across_the_whole_jitter_and_lightness_grid() {
        // The derivation can produce L in {0.6, 0.565, 0.53} and C in {0.175, 0.155, 0.135}
        // at any bin ±3. None of those may panic and all must round-trip to a byte triple.
        for bin in BINS {
            for jitter in -3_i64..=3 {
                for l in [0.6_f64, 0.565, 0.53, 0.48, 0.4] {
                    for c in [0.175_f64, 0.155, 0.135, 0.0675] {
                        let hue = f64::from(i32::try_from(i64::from(bin) + jitter).unwrap_or(0));
                        let out = to_srgb8(Oklch { l, c, h: hue });
                        assert_eq!(hex(out).len(), 7);
                    }
                }
            }
        }
    }
}
