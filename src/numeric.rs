//! Integer-to-float conversions that lose precision past a known bound.
//!
//! `clippy::cast_precision_loss` flags every such `as` cast. The few places
//! that need one call these helpers instead, so the bound and the reason it
//! is acceptable are written down once (DOLL-653).

/// A count (samples, frames, allocations) as `f64`.
///
/// Exact below 2^53. The counts averaged or reported here are sample and
/// allocation totals, which stay far below that.
#[expect(
    clippy::cast_precision_loss,
    reason = "exact below 2^53, far above any count this crate converts"
)]
#[must_use]
#[inline]
pub(crate) const fn count_to_f64(count: u64) -> f64 {
    count as f64
}

/// A `usize` length or count as `f64`; see [`count_to_f64`].
#[cfg(test)]
#[must_use]
#[inline]
pub(crate) const fn len_to_f64(len: usize) -> f64 {
    count_to_f64(len as u64)
}

/// A `usize` length, count or index as `f32`.
///
/// Exact below 2^24. Callers either stay below that (metric counts) or use
/// the value as the phase of a synthetic test waveform, where rounding
/// beyond it only nudges the generated samples.
#[cfg(any(test, feature = "benchmarking"))]
#[expect(
    clippy::cast_precision_loss,
    reason = "exact below 2^24; see the doc comment for why larger values are acceptable"
)]
#[must_use]
#[inline]
pub(crate) const fn len_to_f32(len: usize) -> f32 {
    len as f32
}

/// `value` as `i32`, truncating toward zero and saturating at the `i32`
/// bounds (NaN becomes 0).
///
/// That is Rust's float-to-int `as`, which PCM conversion relies on: callers
/// round and clamp first, and a 32-bit full-scale +1.0 (2^31 in `f32`) must
/// land on `i32::MAX` rather than wrap.
#[expect(
    clippy::cast_possible_truncation,
    reason = "float-to-int `as` saturates at the i32 bounds, which is the intended behavior"
)]
#[must_use]
#[inline]
pub(crate) const fn saturating_i32(value: f32) -> i32 {
    value as i32
}

/// `f64` counterpart of [`saturating_i32`], for tests that synthesize
/// 32-bit PCM in double precision.
#[cfg(test)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "float-to-int `as` saturates at the i32 bounds, which is the intended behavior"
)]
#[must_use]
pub(crate) const fn saturating_i32_from_f64(value: f64) -> i32 {
    value as i32
}
