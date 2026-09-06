//! After Effects' Curves blob, decoded
//! ([docs/11-AE-IMPORT.md](../../../../docs/11-AE-IMPORT.md) §5 — the honesty
//! note about the unreadable blob, answered for the direct route).
//!
//! # In plain terms
//!
//! After Effects stores the whole of a Curves effect — all five channels, all
//! their control points — as one lump of bytes it calls *arbitrary data*. Its
//! own scripting refuses to hand that lump over, which is why a Curves used to
//! import as an empty placeholder. The direct `.aep` route reads the
//! lump out of the file itself, and this module says what is inside it.
//!
//! The lump is 1,644 bytes and has three parts:
//!
//! 1. **Four bytes of header** — a version number, which is 1.
//! 2. **Five tables of 256 bytes**, one a channel. This is After Effects' own
//!    answer: table 40 of the red channel is what an input of 40 comes out as.
//!    Nothing downstream uses these numbers, but they are the reason this
//!    decoder can be *checked* rather than believed — see below.
//! 3. **Five records of 72 bytes**, one a channel and in the same order:
//!    sixteen `(x, y)` pairs of big-endian 16-bit numbers, then how many of
//!    those sixteen are real (2 to 16), then which one the panel had selected
//!    (−1 for none). The pairs past the count are zero and mean nothing.
//!
//! The five channels are Master, Red, Green, Blue and Alpha — After Effects'
//! own five, in the order its own Channel menu lists them, which is also the
//! order Lumit's Curves declares (docs/08 §3.30).
//!
//! # Why this is a decode and not a guess
//!
//! The layout above was read off the project the importer is measured against,
//! and then **checked against After Effects' own arithmetic**: for every one of
//! the 95 channels in that project's nineteen Curves instances, every control
//! point `(x, y)` sits exactly on the lookup table After Effects baked from it
//! — `table[x] == y`, to the byte, with no exceptions. A misread offset, a
//! wrong endianness or a shifted record would break that agreement immediately,
//! because the two halves of the blob are written by different code inside
//! After Effects and can only agree if both have been read correctly.
//!
//! So the check is not a test that ran once: [`decode`] performs it on **every
//! blob it is given**, and a blob that fails it is refused. A future After
//! Effects that changes the layout therefore gets the placeholder it used to
//! get, with the report row saying the property could not be read — never a
//! curve that is silently the wrong shape.
//!
//! (The same project also proved *how* After Effects draws the line between the
//! points — a natural cubic spline, reproducing all 95 tables byte for byte —
//! but that is the effect's business rather than the importer's, and Lumit's
//! Curves draws a clamped one. The difference is a report row, not an
//! arithmetic here.)

/// How long the blob is. Every part below is at a fixed offset inside it, so a
/// blob of any other length is not this format and is refused outright.
const BLOB_LEN: usize = 1644;

/// The only header version this understands.
const VERSION: u16 = 1;

/// How many channels: Master, Red, Green, Blue, Alpha.
pub(crate) const CHANNELS: usize = 5;

/// Entries in one channel's baked lookup table — one an 8-bit input value.
const TABLE: usize = 256;

/// Where the five lookup tables start.
const TABLES_AT: usize = 4;

/// Where the five point records start: past the header and the five tables.
const POINTS_AT: usize = TABLES_AT + CHANNELS * TABLE;

/// The most control points a record has room for. The same sixteen Lumit's own
/// curve carries (`lumit_core::fx::CURVE_MAX_POINTS`), which is why the
/// point list needs no thinning on the way across.
const MAX_POINTS: usize = 16;

/// One channel's record: sixteen pairs, the count, the selected index.
const RECORD: usize = MAX_POINTS * 4 + 4 + 4;

/// The largest value a control point coordinate takes — the blob is 8-bit
/// display values widened to 16 bits, not 16-bit values.
const MAX_COORD: u16 = 255;

/// One `ADBE CurvesCustom` blob's five channels, as point lists in the unit
/// square, in Lumit's own order: Master, Red, Green, Blue, Alpha.
///
/// `None` when the bytes are not a Curves blob this build understands — a
/// wrong length, an unknown version, a record that contradicts After Effects'
/// own lookup table. The caller's answer to `None` is the placeholder the
/// effect had before this module existed, which is why being strict here costs
/// nothing and being lenient would cost a wrong picture.
#[must_use]
pub(crate) fn decode(bytes: &[u8]) -> Option<[Vec<[f32; 2]>; CHANNELS]> {
    if bytes.len() != BLOB_LEN || u16_at(bytes, 0)? != VERSION {
        return None;
    }
    let mut out: [Vec<[f32; 2]>; CHANNELS] = Default::default();
    for (channel, points) in out.iter_mut().enumerate() {
        let table = bytes.get(TABLES_AT + channel * TABLE..)?.get(..TABLE)?;
        let record = POINTS_AT + channel * RECORD;
        let count = usize::try_from(u32_at(bytes, record + MAX_POINTS * 4)?).ok()?;
        if !(2..=MAX_POINTS).contains(&count) {
            return None;
        }
        let mut last: Option<u16> = None;
        for i in 0..count {
            let x = u16_at(bytes, record + i * 4)?;
            let y = u16_at(bytes, record + i * 4 + 2)?;
            // Ordered, inside the 8-bit square, and — the check that makes this
            // a decode — sitting on After Effects' own baked answer for that
            // input. A record read at the wrong offset fails this at once.
            if x > MAX_COORD || y > MAX_COORD || last.is_some_and(|l| x <= l) {
                return None;
            }
            if u16::from(*table.get(usize::from(x))?) != y {
                return None;
            }
            last = Some(x);
            points.push([
                f32::from(x) / f32::from(MAX_COORD),
                f32::from(y) / f32::from(MAX_COORD),
            ]);
        }
    }
    Some(out)
}

/// The hex string the `.aep` walker records a blob as, back into bytes.
///
/// `None` on anything that is not an even run of hex digits — the capture is a
/// file on disk that a hand can edit, so it is checked rather than trusted.
#[must_use]
pub(crate) fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let digit = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    };
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| Some(digit(pair[0])? << 4 | digit(pair[1])?))
        .collect()
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A blob built to the layout: `curves[c]` is one channel's control points
    /// as 8-bit `(x, y)` pairs, and the lookup tables are filled in from them
    /// the way After Effects fills them in — linearly here, which is enough,
    /// because [`decode`] only ever reads the table *at* a control point.
    fn blob(curves: &[&[(u16, u16)]; CHANNELS]) -> Vec<u8> {
        let mut out = vec![0u8; BLOB_LEN];
        out[0..2].copy_from_slice(&VERSION.to_be_bytes());
        out[2..4].copy_from_slice(&1u16.to_be_bytes());
        for (channel, points) in curves.iter().enumerate() {
            for input in 0..TABLE {
                let x = input as u16;
                let mut y = points[0].1;
                for pair in points.windows(2) {
                    let (x0, y0) = pair[0];
                    let (x1, y1) = pair[1];
                    if x >= x0 && x <= x1 {
                        let span = u32::from(x1 - x0).max(1);
                        let step = u32::from(x - x0) * u32::from(y1.abs_diff(y0)) / span;
                        y = if y1 >= y0 {
                            y0 + step as u16
                        } else {
                            y0 - step as u16
                        };
                    } else if x > x1 {
                        y = y1;
                    }
                }
                out[TABLES_AT + channel * TABLE + input] = y as u8;
            }
            // Pin the table at the control points exactly: the linear fill
            // above rounds, and the decoder demands the exact byte.
            for (x, y) in points.iter().copied() {
                out[TABLES_AT + channel * TABLE + usize::from(x)] = y as u8;
            }
            let record = POINTS_AT + channel * RECORD;
            for (i, (x, y)) in points.iter().copied().enumerate() {
                out[record + i * 4..record + i * 4 + 2].copy_from_slice(&x.to_be_bytes());
                out[record + i * 4 + 2..record + i * 4 + 4].copy_from_slice(&y.to_be_bytes());
            }
            let count = points.len() as u32;
            out[record + MAX_POINTS * 4..record + MAX_POINTS * 4 + 4]
                .copy_from_slice(&count.to_be_bytes());
            out[record + MAX_POINTS * 4 + 4..record + RECORD]
                .copy_from_slice(&(-1i32).to_be_bytes());
        }
        out
    }

    const IDENTITY: &[(u16, u16)] = &[(0, 0), (255, 255)];

    #[test]
    fn a_default_curves_reads_as_five_identity_diagonals() {
        let bytes = blob(&[IDENTITY; CHANNELS]);
        let got = decode(&bytes).expect("the default blob decodes");
        for channel in &got {
            assert_eq!(channel.as_slice(), &[[0.0, 0.0], [1.0, 1.0]]);
        }
    }

    /// The shape the fixture project's own grade has: a contrast S on Master
    /// and nothing on the rest. The point of the assertion is the *placement* —
    /// a record read one channel out would put the S on Red.
    #[test]
    fn a_shaped_channel_lands_on_its_own_channel() {
        let s: &[(u16, u16)] = &[(0, 0), (73, 42), (143, 197), (255, 255)];
        let bytes = blob(&[s, IDENTITY, IDENTITY, IDENTITY, IDENTITY]);
        let got = decode(&bytes).expect("the blob decodes");
        assert_eq!(got[0].len(), 4);
        assert!((got[0][1][0] - 73.0 / 255.0).abs() < 1e-6);
        assert!((got[0][1][1] - 42.0 / 255.0).abs() < 1e-6);
        for channel in &got[1..] {
            assert_eq!(channel.as_slice(), &[[0.0, 0.0], [1.0, 1.0]]);
        }

        let alpha = blob(&[IDENTITY, IDENTITY, IDENTITY, IDENTITY, s]);
        let got = decode(&alpha).expect("the blob decodes");
        assert_eq!(got[0].as_slice(), &[[0.0, 0.0], [1.0, 1.0]]);
        assert_eq!(got[4].len(), 4);
    }

    /// A descending curve — the fixture's blood-splatter matte inverts one —
    /// and one that stops before white, which After Effects allows and which
    /// its own table holds flat past the last point.
    #[test]
    fn a_descending_curve_that_stops_short_still_decodes() {
        let bytes = blob(&[
            IDENTITY,
            IDENTITY,
            IDENTITY,
            IDENTITY,
            &[(0, 255), (37, 103), (128, 0)],
        ]);
        let got = decode(&bytes).expect("the blob decodes");
        assert_eq!(got[4].len(), 3);
        assert!((got[4][0][1] - 1.0).abs() < 1e-6);
        assert!((got[4][2][0] - 128.0 / 255.0).abs() < 1e-6);
        assert!(got[4][2][1].abs() < 1e-6);
    }

    /// Every way the blob can fail to be the thing this module claims it is.
    /// Each of these is a placeholder rather than a wrong curve.
    #[test]
    fn a_blob_that_is_not_this_format_is_refused() {
        let good = blob(&[IDENTITY; CHANNELS]);
        assert!(decode(&good).is_some());

        assert!(decode(&[]).is_none(), "empty");
        assert!(decode(&good[..BLOB_LEN - 1]).is_none(), "short");
        let mut long = good.clone();
        long.push(0);
        assert!(decode(&long).is_none(), "long");

        let mut version = good.clone();
        version[1] = 2;
        assert!(decode(&version).is_none(), "an unknown version");

        // A count outside 2..=16 — one point has no curve, seventeen has no
        // room.
        for count in [0u32, 1, 17, u32::MAX] {
            let mut bad = good.clone();
            let at = POINTS_AT + MAX_POINTS * 4;
            bad[at..at + 4].copy_from_slice(&count.to_be_bytes());
            assert!(decode(&bad).is_none(), "count {count}");
        }

        // A point that does not sit on After Effects' own table: the check
        // that turns this from a guess into a decode.
        let mut moved = good.clone();
        moved[POINTS_AT + 3] = 9;
        assert!(decode(&moved).is_none(), "a point off the table");

        // The same bytes read little-endian would put the count in the wrong
        // half of its word, so a byte-swapped blob is refused too.
        let mut swapped = good.clone();
        let at = POINTS_AT + MAX_POINTS * 4;
        swapped[at..at + 4].copy_from_slice(&2u32.to_le_bytes());
        assert!(decode(&swapped).is_none(), "little-endian");

        // Points out of x order — a record read at a shifted offset looks like
        // this long before it looks like anything else.
        let mut shifted = good.clone();
        shifted.copy_within(POINTS_AT + 2..BLOB_LEN, POINTS_AT);
        assert!(decode(&shifted).is_none(), "a shifted record");
    }

    #[test]
    fn hex_reads_back_the_bytes_it_was_written_from() {
        assert_eq!(from_hex("00ff10AB"), Some(vec![0x00, 0xff, 0x10, 0xab]));
        assert_eq!(from_hex(""), Some(Vec::new()));
        assert_eq!(from_hex("abc"), None, "an odd run of digits");
        assert_eq!(from_hex("zz"), None, "not hex");
    }
}
