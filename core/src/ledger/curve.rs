//! §38.4: the curve — what each level costs, the XP at which it starts, and the level an XP
//! total reaches. There is no maximum level.
//!
//! The renderer never computes a level, a threshold or a price; it receives a reading built here.

/// `T(1..=200)`: the XP at which each level starts, generated from `cost` and checked in so that
/// no platform's floating point decides it. `ledger_xp.rs` regenerates it from `cost` and
/// compares every element.
pub const LEVEL_THRESHOLDS: [i64; 200] = [
    0, 120, 360, 720, 1_200, 1_800, 2_520, 3_360, 4_320, 5_400, 6_600, 8_630, 10_910, 13_440,
    16_220, 19_260, 22_570, 26_150, 30_010, 34_150, 38_570, 43_280, 48_280, 53_580, 59_180, 65_090,
    71_310, 77_840, 84_690, 91_860, 99_350, 107_170, 115_320, 123_800, 132_610, 141_760, 151_250,
    161_090, 171_280, 181_810, 192_700, 203_940, 215_540, 227_500, 239_820, 252_510, 265_570,
    279_000, 292_800, 306_970, 321_520, 336_450, 351_760, 367_460, 383_540, 400_010, 416_870,
    434_120, 451_770, 469_810, 488_250, 507_090, 526_340, 545_990, 566_050, 586_520, 607_400,
    628_690, 650_390, 672_510, 695_050, 718_010, 741_390, 765_190, 789_410, 814_060, 839_140,
    864_650, 890_590, 916_960, 943_770, 971_010, 998_690, 1_026_810, 1_055_370, 1_084_380,
    1_113_830, 1_143_730, 1_174_070, 1_204_860, 1_236_100, 1_267_800, 1_299_950, 1_332_550,
    1_365_610, 1_399_130, 1_433_110, 1_467_550, 1_502_450, 1_537_810, 1_573_640, 1_609_940,
    1_646_700, 1_683_930, 1_721_630, 1_759_810, 1_798_460, 1_837_580, 1_877_180, 1_917_260,
    1_957_820, 1_998_860, 2_040_380, 2_082_380, 2_124_860, 2_167_830, 2_211_280, 2_255_220,
    2_299_650, 2_344_570, 2_389_980, 2_435_890, 2_482_290, 2_529_180, 2_576_570, 2_624_460,
    2_672_850, 2_721_740, 2_771_130, 2_821_020, 2_871_410, 2_922_310, 2_973_710, 3_025_620,
    3_078_040, 3_130_970, 3_184_410, 3_238_360, 3_292_820, 3_347_790, 3_403_280, 3_459_290,
    3_515_810, 3_572_850, 3_630_410, 3_688_490, 3_747_090, 3_806_210, 3_865_860, 3_926_030,
    3_986_730, 4_047_950, 4_109_700, 4_171_980, 4_234_790, 4_298_130, 4_362_000, 4_426_400,
    4_491_340, 4_556_810, 4_622_820, 4_689_370, 4_756_450, 4_824_070, 4_892_230, 4_960_930,
    5_030_170, 5_099_960, 5_170_290, 5_241_170, 5_312_590, 5_384_560, 5_457_080, 5_530_140,
    5_603_750, 5_677_910, 5_752_630, 5_827_900, 5_903_720, 5_980_100, 6_057_030, 6_134_520,
    6_212_560, 6_291_160, 6_370_320, 6_450_040, 6_530_320, 6_611_160, 6_692_560, 6_774_530,
    6_857_060, 6_940_160, 7_023_820, 7_108_050, 7_192_850, 7_278_220, 7_364_160, 7_450_670,
    7_537_750, 7_625_400,
];

/// What leaving level `n` costs: `120·n` up to level 10, then `10·round(9·n^1.3)`, rounded half
/// away from zero.
#[must_use]
pub fn cost(n: u32) -> i64 {
    if n <= 10 {
        return 120 * i64::from(n);
    }
    // §38.4 is defined over the reals and no integer form rounds `9·n^1.3` the same way. The
    // nearest it comes to a `.5` edge below level 200 is 3.6e-4, so the table above is identical
    // on every platform.
    #[allow(clippy::float_arithmetic)]
    let steps = 9.0 * f64::from(n).powf(1.3);
    10 * rounded(steps)
}

/// `value` rounded half away from zero. Saturates at the `i64` bounds, which `cost` cannot
/// reach for any `u32` level.
// Float to integer has no conversion function; this is the one place the curve spells it.
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
fn rounded(value: f64) -> i64 {
    value.round() as i64
}

/// The thresholds every reader goes through: `LEVEL_THRESHOLDS`, unless a test overrode them.
fn thresholds() -> [i64; 200] {
    #[cfg(feature = "testkit")]
    if let Some(curve) = super::price::testkit::curve() {
        return curve;
    }
    LEVEL_THRESHOLDS
}

/// `T(level)`, the XP at which `level` starts: the table to 200, the formula above it. Level 0
/// does not exist and reads as level 1.
#[must_use]
pub fn threshold(level: u32) -> i64 {
    let table = thresholds();
    let index = usize::try_from(level.max(1) - 1).unwrap_or(usize::MAX);
    if let Some(&at) = table.get(index) {
        return at;
    }
    let [.., top] = table;
    (200..level).fold(top, |at, n| at.saturating_add(cost(n)))
}

/// The greatest level whose threshold `xp` has reached. Anything below `T(2)` is level 1.
#[must_use]
pub fn level_of(xp: i64) -> u32 {
    let table = thresholds();
    let reached = table.partition_point(|&at| at <= xp);
    if reached < table.len() {
        return u32::try_from(reached).unwrap_or(u32::MAX).max(1);
    }
    let [.., top] = table;
    let (mut level, mut at) = (200_u32, top);
    loop {
        let next = at.saturating_add(cost(level));
        if next > xp || next == at || level == u32::MAX {
            return level;
        }
        at = next;
        level += 1;
    }
}
