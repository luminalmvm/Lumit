//! Tests of the layer span edits.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod unretime_tests {
    use super::super::*;
    use crate::time::{Rational, SourceTime};

    fn ct(secs: i64) -> CompTime {
        CompTime(Rational::new(secs, 1).unwrap())
    }
    fn st(secs: i64) -> SourceTime {
        SourceTime(Rational::new(secs, 1).unwrap())
    }
    fn dur(secs: i64) -> Duration {
        Duration(Rational::new(secs, 1).unwrap())
    }

    /// Anchored part-way in, only what is left of the source is available.
    #[test]
    fn anchored_mid_source_runs_out_sooner() {
        let (i, o, off) = unretimed_span(ct(10), ct(100), st(2), Some(dur(5))).unwrap();
        assert_eq!(i, ct(10));
        assert_eq!(o, ct(13), "three seconds of source were left");
        assert_eq!(off, ct(8), "source zero sits two seconds before the in");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod span_edit_tests {
    use super::super::*;
    use crate::time::Rational;

    fn ct(secs: i64) -> CompTime {
        CompTime(Rational::new(secs, 1).unwrap())
    }

    #[test]
    fn move_in_shifts_the_whole_layer_to_the_playhead() {
        // Layer visible [2,5), source-0 at comp 1; move its in point to 10.
        let (i, o, off) = edit_layer_span(ct(2), ct(5), ct(1), ct(10), SpanEdit::MoveIn).unwrap();
        // In lands on the playhead; duration (3s) and the source-at-in are kept.
        assert_eq!((i, o, off), (ct(10), ct(13), ct(9)));
    }

    #[test]
    fn trim_moves_one_edge_and_keeps_the_offset() {
        let (i, o, off) = edit_layer_span(ct(2), ct(5), ct(1), ct(3), SpanEdit::TrimIn).unwrap();
        assert_eq!((i, o, off), (ct(3), ct(5), ct(1)));
        let (i, o, off) = edit_layer_span(ct(2), ct(5), ct(1), ct(4), SpanEdit::TrimOut).unwrap();
        assert_eq!((i, o, off), (ct(2), ct(4), ct(1)));
    }

    #[test]
    fn a_degenerate_trim_is_rejected() {
        // Trimming the in point to or past the out point would invert the span.
        assert!(edit_layer_span(ct(2), ct(5), ct(1), ct(5), SpanEdit::TrimIn).is_none());
        assert!(edit_layer_span(ct(2), ct(5), ct(1), ct(6), SpanEdit::TrimIn).is_none());
        assert!(edit_layer_span(ct(2), ct(5), ct(1), ct(2), SpanEdit::TrimOut).is_none());
        // A move never inverts (duration is preserved), even past comp 0.
        let (i, o, _) = edit_layer_span(ct(2), ct(5), ct(1), ct(0), SpanEdit::MoveOut).unwrap();
        assert!(i < o);
    }
}
