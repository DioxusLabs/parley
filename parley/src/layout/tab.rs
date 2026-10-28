// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tab stops, following [CSS `tab-size`][tab-size]. See [`TabAdvance`] for how a tab's advance
//! flows through layout.
//!
//! [tab-size]: https://www.w3.org/TR/css-text-4/#tab-size-property
//! [`TabAdvance`]: crate::layout::data::TabAdvance

/// The advance of a tab that starts `x` past the line's start edge, with tab stops every
/// `tab_size`.
///
/// `x` includes the line's indent, as tab stops are measured from the line's start edge. The tab
/// advances to the next stop, or to the one after it if the next is closer than `skip_threshold`
/// (see [`TabSize::skip_threshold`](crate::TabSize::skip_threshold)). Only that one stop is
/// skipped, so with stops closer together than the threshold the advance can still be less than
/// it. A tab size that isn't positive gives the tab no advance.
pub(crate) fn tab_advance(x: f32, tab_size: f32, skip_threshold: f32) -> f32 {
    if tab_size.is_nan() || tab_size <= 0. {
        return 0.;
    }

    // The distance past the previous tab stop, in `0.0..tab_size`. The remainder takes the sign of
    // `x`, which can be negative under a negative indent.
    let abs_x = f64::from(x.abs());
    let size = f64::from(tab_size);
    let quotient = abs_x / size;
    // Below 2^24, the quotient times the 24-bit f32 significand fits exactly in f64.
    // The f64 quotient also cannot round across an integer in this range.
    let mut past_stop = if tab_size.is_finite() && quotient < 16_777_216. {
        let whole = quotient as u32;
        let remainder = (abs_x - f64::from(whole) * size) as f32;
        if x.is_sign_negative() {
            -remainder
        } else {
            remainder
        }
    } else {
        x % tab_size
    };
    if past_stop < 0. {
        past_stop += tab_size;
    }

    let mut distance = tab_size - past_stop;
    if distance < skip_threshold {
        distance += tab_size;
    }
    distance
}

#[cfg(test)]
mod tests {
    use super::tab_advance;

    #[test]
    fn tab_advances_to_the_next_stop() {
        assert_eq!(tab_advance(0., 80., 5.), 80.);
        assert_eq!(tab_advance(80., 80., 5.), 80.);
        assert_eq!(tab_advance(30., 80., 5.), 50.);
        assert_eq!(tab_advance(170., 80., 5.), 70.);
    }

    #[test]
    fn tab_skips_a_stop_closer_than_the_skip_threshold() {
        assert_eq!(tab_advance(76., 80., 5.), 84.);
        // Exactly the threshold away is not "closer than", so the next stop is kept.
        assert_eq!(tab_advance(75., 80., 5.), 5.);
        assert_eq!(tab_advance(74., 80., 5.), 6.);
        // With a zero threshold, no stop is skipped.
        assert_eq!(tab_advance(79., 80., 0.), 1.);
        // Only one stop is skipped: with stops closer together than the threshold, the advance
        // ends up below the threshold.
        assert_eq!(tab_advance(3., 4., 8.), 5.);
    }

    #[test]
    fn tab_under_a_negative_indent_measures_from_the_previous_stop() {
        assert_eq!(tab_advance(-30., 80., 5.), 30.);
        assert_eq!(tab_advance(-80., 80., 5.), 80.);
    }

    #[test]
    fn tab_size_that_is_not_positive_gives_no_advance() {
        assert_eq!(tab_advance(10., 0., 5.), 0.);
        assert_eq!(tab_advance(10., -1., 5.), 0.);
        assert_eq!(tab_advance(10., f32::NAN, 5.), 0.);
    }

    fn reference(x: f32, size: f32, threshold: f32) -> f32 {
        if size.is_nan() || size <= 0. {
            return 0.;
        }
        let mut past = x % size;
        if past < 0. {
            past += size;
        }
        let mut distance = size - past;
        if distance < threshold {
            distance += size;
        }
        distance
    }

    #[test]
    fn tab_advance_matches_remainder_arithmetic() {
        let check = |x: f32, size: f32, threshold: f32| {
            let expected = reference(x, size, threshold);
            let actual = tab_advance(x, size, threshold);
            assert!(
                actual.to_bits() == expected.to_bits() || (actual.is_nan() && expected.is_nan()),
                "x={x:?} size={size:?} threshold={threshold:?}: {actual:?} != {expected:?}"
            );
        };
        for size in [
            0.,
            -0.,
            -1.,
            0.1,
            4.,
            31.6875,
            f32::MIN_POSITIVE,
            f32::MAX,
            f32::INFINITY,
            f32::NAN,
        ] {
            for x in [
                0.,
                -0.,
                1.,
                -1.,
                size,
                -size,
                3. * size,
                -3. * size,
                f32::MAX,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NAN,
            ] {
                for threshold in [0., 0.5, size / 2., f32::INFINITY, f32::NAN] {
                    check(x, size, threshold);
                }
            }
        }
        let mut seed = 1u32;
        for _ in 0..100_000 {
            let mut next = || {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                f32::from_bits(seed)
            };
            check(next(), next(), next());
        }
        for size in [0.1f32, 4., 31.6875, f32::MIN_POSITIVE] {
            for n in [1., 2., 17., 1000., 8_388_607., 16_777_216.] {
                let stop = size * n;
                for bits in [
                    stop.to_bits().saturating_sub(1),
                    stop.to_bits(),
                    stop.to_bits().saturating_add(1),
                ] {
                    let x = f32::from_bits(bits);
                    check(x, size, 0.5);
                    check(-x, size, 0.5);
                }
            }
        }
        for _ in 0..100_000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let size = f32::from_bits((seed & 0x007f_ffff) | 0x3f00_0000);
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let stop = size * (seed & 0x00ff_ffff) as f32;
            let x = f32::from_bits(stop.to_bits().saturating_sub(seed % 3));
            check(x, size, 0.5);
            check(-x, size, 0.5);
        }
    }
}
