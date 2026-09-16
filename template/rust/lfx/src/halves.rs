//! The two half-float conversions, written out because Rust has no half type
//! and a plugin should not need a dependency for one.
//!
//! # In plain terms
//!
//! Lumit's working texture is fp16 and the project decides which depth a frame
//! crosses at; the host never converts to accommodate a plugin. So every LFX
//! plugin needs these two functions, and here they are: bit arithmetic, no
//! table, no crate.
//!
//! The rounding is to nearest and ties to even, which is what the host does on
//! its own side of the boundary - so a picture that went out at fp16 and came
//! back is the picture the same maths at fp32 would have made, to within the
//! eleven bits fp16 has.
//!
//! # Thread role and contract
//!
//! None: pure arithmetic, called from wherever the frame is.

/// One half-float as an `f32`.
#[must_use]
pub const fn from_half(bits: u16) -> f32 {
    let sign = (bits as u32 & 0x8000) << 16;
    let exponent = (bits as u32 >> 10) & 0x1f;
    let mut mantissa = bits as u32 & 0x3ff;
    let whole = if exponent == 0 {
        if mantissa == 0 {
            sign
        } else {
            // A subnormal half is a normal float: shift the leading one up
            // into place and pay for it in the exponent.
            let mut shifted: i32 = 1;
            while mantissa & 0x400 == 0 {
                mantissa <<= 1;
                shifted -= 1;
            }
            mantissa &= 0x3ff;
            sign | (((shifted + 112) as u32) << 23) | (mantissa << 13)
        }
    } else if exponent == 31 {
        sign | 0x7f80_0000 | (mantissa << 13)
    } else {
        sign | ((exponent + 112) << 23) | (mantissa << 13)
    };
    f32::from_bits(whole)
}

/// One `f32` as a half, rounding to nearest and ties to even.
#[must_use]
pub const fn to_half(value: f32) -> u16 {
    let whole = value.to_bits();
    let sign = (whole >> 16) & 0x8000;
    let biased = (whole >> 23) & 0xff;
    let mut mantissa = whole & 0x7f_ffff;
    let exponent = biased as i32 - 127 + 15;

    if biased == 0xff {
        // Infinity keeps its sign; a NaN stays a NaN rather than becoming one.
        let payload = if mantissa != 0 {
            0x200 | (mantissa >> 13)
        } else {
            0
        };
        return (sign | 0x7c00 | payload) as u16;
    }
    if exponent >= 31 {
        return (sign | 0x7c00) as u16;
    }
    if exponent <= 0 {
        if exponent < -10 {
            return sign as u16;
        }
        mantissa |= 0x80_0000;
        let shift = (14 - exponent) as u32;
        let mut narrowed = mantissa >> shift;
        let remainder = mantissa & ((1 << shift) - 1);
        let halfway = 1 << (shift - 1);
        if remainder > halfway || (remainder == halfway && narrowed & 1 != 0) {
            narrowed += 1;
        }
        return (sign | narrowed) as u16;
    }
    let mut narrowed = ((exponent as u32) << 10) | (mantissa >> 13);
    let remainder = mantissa & 0x1fff;
    if remainder > 0x1000 || (remainder == 0x1000 && narrowed & 1 != 0) {
        narrowed += 1;
    }
    (sign | narrowed) as u16
}

#[cfg(test)]
mod tests {
    use super::{from_half, to_half};

    /// Every half there is, out and back: the conversion is exact in that
    /// direction, which is what lets a plugin do its maths in `f32` and hand
    /// the picture back unchanged where its own numbers did not move.
    #[test]
    fn every_half_round_trips_through_a_float() {
        for bits in 0..=u16::MAX {
            let whole = from_half(bits);
            if whole.is_nan() {
                assert_eq!(bits >> 10 & 0x1f, 0x1f, "only the NaN pattern is NaN");
                continue;
            }
            assert_eq!(to_half(whole), bits, "half {bits:#06x} did not come back");
        }
    }

    /// The numbers a picture is actually made of, and the three that are not
    /// numbers at all.
    #[test]
    fn the_edges_convert_as_the_host_converts_them() {
        assert_eq!(to_half(0.0), 0x0000);
        assert_eq!(to_half(-0.0), 0x8000);
        assert_eq!(to_half(1.0), 0x3c00);
        assert_eq!(to_half(-2.0), 0xc000);
        // 65504 is the largest half; anything above it is an infinity.
        assert_eq!(to_half(65504.0), 0x7bff);
        assert_eq!(to_half(65536.0), 0x7c00);
        assert_eq!(to_half(f32::INFINITY), 0x7c00);
        assert_eq!(to_half(f32::NEG_INFINITY), 0xfc00);
        assert!(from_half(to_half(f32::NAN)).is_nan());
        // Ties to even, as the host rounds: halfway between two halves goes to
        // the one whose last bit is nought.
        let between = (from_half(0x3c00) + from_half(0x3c01)) * 0.5;
        assert_eq!(to_half(between), 0x3c00);
        // The smallest subnormal, and half of it, which rounds back to nought.
        assert_eq!(to_half(from_half(0x0001)), 0x0001);
        assert_eq!(to_half(from_half(0x0001) * 0.25), 0x0000);
    }
}
