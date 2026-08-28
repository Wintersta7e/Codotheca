//! §7.3a's seed. The hash runs over the **UTF-16 code units** of `seed_basename` — the
//! directory basename recorded at first index (§7.4) — and never over `project.name`, which
//! absorbs the remote name and would re-roll a clone in a differently-named folder the moment
//! T1 learns its remote.

/// §7.4: the offset is a suffix on the **hashed string**, never a rewrite of the stored
/// basename.
#[must_use]
pub fn seed_string(seed_basename: &str, reroll_offset: u32) -> String {
    if reroll_offset == 0 {
        seed_basename.to_owned()
    } else {
        format!("{seed_basename}#{reroll_offset}")
    }
}

/// `h = 0; for (…) h = (h * 31 + s.charCodeAt(i)) >>> 0;`
///
/// `charCodeAt` yields a UTF-16 code unit, so an astral character contributes **two** draws.
/// `wrapping_*` is `>>> 0`: JavaScript wraps here and a panic on a long basename would take a
/// scan down.
#[must_use]
pub fn seed_hash(s: &str) -> u32 {
    let mut h: u32 = 0;
    for unit in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(u32::from(unit));
    }
    h
}

/// §7.3's `seed` field: the exact string hashed, plus its uint32.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Seed {
    pub s: String,
    pub h: u32,
}

#[must_use]
pub fn seed_of(seed_basename: &str, reroll_offset: u32) -> Seed {
    let s = seed_string(seed_basename, reroll_offset);
    let h = seed_hash(&s);
    Seed { s, h }
}

/// One `(h >>> shift) % modulus` draw. Shifts of 32 or more yield `0`, matching neither
/// JavaScript's `>>>` (which masks the shift to 5 bits) nor a Rust overflow — the derivation
/// never uses one, and this makes adding one a visible zero rather than a panic.
#[must_use]
pub fn draw(h: u32, shift: u32, modulus: u32) -> u32 {
    if modulus == 0 {
        return 0;
    }
    let shifted = if shift >= 32 { 0 } else { h >> shift };
    shifted % modulus
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

    #[test]
    fn the_offset_suffixes_the_hashed_string_and_never_rewrites_the_basename() {
        // §7.4: `s = reroll_offset === 0 ? seed_basename : seed_basename + "#" + reroll_offset`
        assert_eq!(seed_string("alpha-tool", 0), "alpha-tool");
        assert_eq!(seed_string("alpha-tool", 1), "alpha-tool#1");
        assert_eq!(seed_string("alpha-tool", 12), "alpha-tool#12");
    }

    #[test]
    fn the_hash_runs_over_utf16_code_units_not_bytes_and_not_scalar_values() {
        // §7.3a: `h = (h * 31 + s.charCodeAt(i)) >>> 0` over the UTF-16 code units.
        assert_eq!(seed_hash("a"), 97);
        // U+00E9 is one code unit: 233. A UTF-8 byte walk would give 0xC3*31 + 0xA9 = 6222.
        assert_eq!(seed_hash("\u{e9}"), 233);
        // U+1F600 is a surrogate PAIR: 0xD83D * 31 + 0xDE00 = 1_772_899.
        // A char walk would give the scalar value 128_512 instead.
        assert_eq!(seed_hash("\u{1f600}"), 1_772_899);
        assert_ne!(seed_hash("\u{1f600}"), 128_512);
    }

    #[test]
    fn the_hash_wraps_at_thirty_two_bits_rather_than_panicking() {
        // A long basename overflows u32 many times over. `>>> 0` is a wrap, not a trap, and a
        // debug-mode panic here would take the whole scan down on one oddly named directory.
        let long = "x".repeat(4096);
        let _ = seed_hash(&long);
        assert_eq!(seed_hash(""), 0);
    }

    #[test]
    fn the_recorded_pair_matches_the_string_it_claims_to_hash() {
        // §7.3: the document carries `s` and `h` together "so a mismatch is detectable rather
        // than silent".
        let seed = seed_of("alpha-tool", 0);
        assert_eq!(seed.s, "alpha-tool");
        assert_eq!(seed.h, 3_035_956_391);
        assert_eq!(seed_of("alpha-tool", 1).h, 1_271_298_901);
        assert_eq!(seed_of("beta-lib", 0).h, 1_852_290_344);
        assert_eq!(seed_of("widget", 0).h, 3_506_920_004);
        assert_eq!(seed_of("atlas", 0).h, 93_144_203);
    }

    #[test]
    fn a_draw_is_a_shift_then_a_modulus_and_never_shifts_off_the_end() {
        let h = seed_hash("alpha-tool");
        assert_eq!(draw(h, 0, 8), h % 8);
        assert_eq!(draw(h, 5, 7), (h >> 5) % 7);
        // `h >>> 31` in JavaScript is a real shift; `h >> 32` in Rust is UB-adjacent and
        // clippy-denied. The wrapper clamps so a future draw cannot introduce one.
        assert_eq!(draw(h, 32, 4), 0);
        assert_eq!(draw(h, 99, 4), 0);
    }
}
