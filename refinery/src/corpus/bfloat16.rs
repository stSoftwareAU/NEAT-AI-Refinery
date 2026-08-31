//! The bfloat16 value encoding: an IEEE-754 binary32 keeping only its top 16
//! bits.
//!
//! bfloat16 keeps the sign and all eight exponent bits of an `f32` and drops
//! the bottom sixteen mantissa bits, leaving seven stored mantissa bits — eight
//! bits of significand once the implicit leading one is counted.
//!
//! ```text
//! f32       s eeeeeeee mmmmmmm mmmmmmmmmmmmmmmm
//! bfloat16  s eeeeeeee mmmmmmm
//!                              └── discarded, with round-to-nearest-even
//! ```
//!
//! Keeping the whole exponent is what makes the scheme conservative: the
//! representable range is the `f32` range, so no finite value overflows to
//! infinity except within half an interval of the largest `f32`, and no value
//! underflows that would not already have underflowed as an `f32`.
//!
//! The error characteristics that follow from that layout are documented in
//! `docs/quantisation.md` and asserted in the tests below.

/// Encodes `value` as bfloat16, rounding to nearest with ties to even.
///
/// Truncation would bias every magnitude downwards, so the discarded bits are
/// rounded rather than dropped: the result is the nearest representable
/// bfloat16, and a tie lands on the neighbour with an even mantissa.
#[must_use]
pub fn from_f32(value: f32) -> u16 {
    let bits = value.to_bits();

    if value.is_nan() {
        // A NaN whose payload lives only in the discarded bits would truncate
        // to an infinity, turning "no value" into a very large one. Setting the
        // top mantissa bit keeps it a (quiet) NaN.
        return ((bits >> 16) as u16) | 0x0040;
    }

    // Round to nearest, ties to even: add half an interval, plus one more when
    // the surviving mantissa is already odd. Infinities and the largest finite
    // magnitudes stay saturated because the carry runs into the exponent, which
    // is exactly what rounding to the nearest bfloat16 asks for.
    let ties_to_even = (bits >> 16) & 1;
    let rounded = bits.wrapping_add(0x0000_7FFF + ties_to_even);
    (rounded >> 16) as u16
}

/// Decodes a bfloat16 back into the `f32` it stands for.
///
/// Reconstruction is exact and lossless: the sixteen discarded bits are zero,
/// so every bfloat16 names one `f32` and decoding never introduces error of its
/// own.
#[must_use]
pub const fn to_f32(bits: u16) -> f32 {
    f32::from_bits((bits as u32) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The relative error bound the scheme claims: half an interval of an
    /// eight-bit significand, `2^-8`.
    const MAX_RELATIVE_ERROR: f32 = 1.0 / 256.0;

    /// The gap between neighbouring bfloat16 values at `1.0` — `2^-7`, seven
    /// stored mantissa bits.
    const INTERVAL_AT_ONE: f32 = 1.0 / 128.0;

    #[test]
    fn spans_one_representable_interval() {
        // Seven stored mantissa bits: the neighbours of 1.0 are 2^-7 away, and
        // half of that is the error bound the scheme claims.
        assert_eq!(
            to_f32(from_f32(1.0 + INTERVAL_AT_ONE)),
            1.0 + INTERVAL_AT_ONE
        );
        assert_eq!(MAX_RELATIVE_ERROR, INTERVAL_AT_ONE / 2.0);
    }

    #[test]
    fn keeps_values_that_are_already_representable() {
        for value in [0.0_f32, -0.0, 1.0, -1.0, 0.5, 2.0, -256.0, 1.5, -3.5] {
            let round_tripped = to_f32(from_f32(value));

            assert_eq!(
                round_tripped.to_bits(),
                value.to_bits(),
                "{value} must survive unchanged"
            );
        }
    }

    #[test]
    fn rounds_to_the_nearest_representable_value() {
        // A quarter of an interval above 1.0 rounds down; three quarters up
        // rounds to the next representable value.
        let interval = INTERVAL_AT_ONE;

        assert_eq!(to_f32(from_f32(1.0 + interval / 4.0)), 1.0);
        assert_eq!(to_f32(from_f32(1.0 + 3.0 * interval / 4.0)), 1.0 + interval);
    }

    #[test]
    fn breaks_ties_towards_an_even_mantissa() {
        let interval = INTERVAL_AT_ONE;

        // Exactly half way between 1.0 (even mantissa) and 1.0 + interval (odd):
        // the even neighbour wins.
        assert_eq!(to_f32(from_f32(1.0 + interval / 2.0)), 1.0);
        // Half way between the odd 1.0 + interval and the even 1.0 + 2 * interval.
        assert_eq!(
            to_f32(from_f32(1.0 + 3.0 * interval / 2.0)),
            1.0 + 2.0 * interval
        );
    }

    #[test]
    fn holds_the_relative_error_bound_across_the_exponent_range() {
        // One representative mantissa pattern swept across every finite
        // exponent, positive and negative.
        for exponent in -126_i32..=126 {
            for mantissa in [0.0_f32, 0.3, 0.517, 0.999] {
                for sign in [1.0_f32, -1.0] {
                    let value = sign * (1.0 + mantissa) * 2.0_f32.powi(exponent);
                    let error = (to_f32(from_f32(value)) - value).abs() / value.abs();

                    assert!(
                        error <= MAX_RELATIVE_ERROR,
                        "{value:e} lost {error:e}, above the {MAX_RELATIVE_ERROR:e} bound"
                    );
                }
            }
        }
    }

    #[test]
    fn preserves_zero_and_its_sign() {
        assert_eq!(from_f32(0.0), 0x0000);
        assert_eq!(from_f32(-0.0), 0x8000);
        assert!(to_f32(from_f32(-0.0)).is_sign_negative());
    }

    #[test]
    fn preserves_infinities() {
        assert!(to_f32(from_f32(f32::INFINITY)).is_infinite());
        assert!(to_f32(from_f32(f32::INFINITY)).is_sign_positive());
        assert!(to_f32(from_f32(f32::NEG_INFINITY)).is_infinite());
        assert!(to_f32(from_f32(f32::NEG_INFINITY)).is_sign_negative());
    }

    #[test]
    fn keeps_a_nan_a_nan_however_its_payload_is_placed() {
        // A NaN whose payload lives entirely in the sixteen discarded bits is
        // the case truncation would corrupt into an infinity.
        let low_payload = f32::from_bits(0x7F80_0001);
        assert!(low_payload.is_nan());

        assert!(to_f32(from_f32(low_payload)).is_nan());
        assert!(to_f32(from_f32(f32::NAN)).is_nan());
    }

    #[test]
    fn saturates_rather_than_wrapping_at_the_top_of_the_range() {
        // Above the largest bfloat16 there is nowhere to round but infinity —
        // the value must never wrap around to a small or negative one.
        let rounded = to_f32(from_f32(f32::MAX));

        assert!(rounded.is_infinite() || rounded > 0.0, "{rounded}");
        assert!(rounded.is_sign_positive());
    }

    #[test]
    fn rounds_a_subnormal_f32_towards_zero_without_changing_its_sign() {
        // f32 subnormals are far below the smallest normal bfloat16; they round
        // to a signed zero rather than to something arbitrary.
        let subnormal = f32::from_bits(0x0000_0001);

        let rounded = to_f32(from_f32(subnormal));

        assert_eq!(rounded, 0.0);
        assert!(rounded.is_sign_positive());
        assert!(to_f32(from_f32(-subnormal)).is_sign_negative());
    }

    #[test]
    fn is_deterministic_for_the_same_input() {
        for step in 0..1_000_u32 {
            let value = f32::from_bits(0x3F80_0000 + step * 7919);

            assert_eq!(from_f32(value), from_f32(value));
        }
    }
}
