//! The numeric conversions the rasterizer needs and `std` has no `From` or `TryFrom` for.
//!
//! Integer scene coordinates become `f32` raster geometry, `f64` colour maths becomes `f32`
//! geometry and 8-bit channels. Each function holds the single `as` for its pair of types, so
//! every call site reads as a named conversion with the semantics `as` gives it.

/// A scene coordinate or percentage as raster geometry. Exact below 2^24 in magnitude, which the
/// `600×900` space is far inside; beyond that it rounds to the nearest `f32`.
// Integer to float has no conversion function; this is the one place the art module spells it.
#[allow(clippy::as_conversions, clippy::cast_precision_loss)]
pub(super) const fn i32_to_f32(value: i32) -> f32 {
    value as f32
}

/// A render target's pixel size as raster geometry. Exact below 2^24, as `i32_to_f32` is.
// Integer to float has no conversion function; this is the one place the art module spells it.
#[allow(clippy::as_conversions, clippy::cast_precision_loss)]
pub(super) const fn u32_to_f32(value: u32) -> f32 {
    value as f32
}

/// `f64` trigonometry narrowed to the rasterizer's `f32`, rounding to the nearest `f32`.
// Narrowing between float widths has no conversion function; this is its one spelling here.
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
pub(super) const fn f64_to_f32(value: f64) -> f32 {
    value as f32
}

/// An 8-bit channel from a value the caller has already rounded and clamped to `0..=255`.
/// Anything outside saturates and `NaN` becomes `0`, which is what `as` does.
// Float to integer has no conversion function; the clamp at every call site is the range check.
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub(super) const fn f32_to_u8(value: f32) -> u8 {
    value as u8
}

/// The `f64` form of `f32_to_u8`, for the colour pipeline's channels.
// Float to integer has no conversion function; the clamp at every call site is the range check.
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub(super) const fn f64_to_u8(value: f64) -> u8 {
    value as u8
}

/// A hue the caller has already rounded, as the integer §7.3 serialises. Saturates at the `i64`
/// bounds and maps `NaN` to `0`, which is what `as` does.
// Float to integer has no conversion function; this is the one place the art module spells it.
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
pub(super) const fn f64_to_i64(value: f64) -> i64 {
    value as i64
}

/// A scaled test sample coordinate, truncated toward zero and saturating, which is what `as` does.
// Float to integer has no conversion function; this is the one place the art tests spell it.
#[cfg(test)]
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub(super) const fn f64_to_u32(value: f64) -> u32 {
    value as u32
}
