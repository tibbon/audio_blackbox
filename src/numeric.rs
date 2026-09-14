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
