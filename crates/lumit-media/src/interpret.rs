//! Footage interpretation: how a file's frames are read before the engine
//! sees them (docs/01-GLOSSARY.md: Interpretation).
//!
//! In plain terms: the file says one thing and the user knows better. Its
//! alpha was saved against a colour, it was shot at 60 fps to be played at 24,
//! it is interlaced, its pixels aren't square, or it should play three times
//! over. None of that changes the file. It changes which of the file's frames
//! is "frame 12", how many there are, how fast they run and what the pixels of
//! one mean.
//!
//! Everything here is arithmetic on frame numbers and on decoded pixels, and
//! opens no file. The probe, the frame index and the decoder apply it, so
//! whatever asks them about an interpreted [`crate::sequence::MediaSource`] is answered
//! in interpreted frames. Runs on decode threads.

use crate::{decode::DecodedFrame, decode::PixelFormat};

/// What a file's alpha channel means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Alpha {
    /// The colour channels are whole and the alpha cuts them out. What every
    /// file is read as until someone says otherwise.
    #[default]
    Straight,
    /// There is no transparency, whatever the channel holds.
    Ignore,
    /// The colour was saved already mixed into this matte colour, sRGB bytes.
    /// Reading it as straight leaves a fringe of that colour round every edge.
    Premultiplied([u8; 3]),
}

/// Whether a frame is two fields woven together, and which one came first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Fields {
    #[default]
    Off,
    /// The even rows, counting the top row as nought, are the earlier field.
    Upper,
    /// The odd rows are the earlier field.
    Lower,
}

/// A film cadence folded into interlaced video, to be taken back out.
///
/// The number is where the file's first frame sits in the cadence, 0 to 4.
/// Counting whole (W) and split (S) frames from there, 3:2 reads WWSSW, WSSWW,
/// SSWWW, SWWWS, WWWSS and the advanced kind reads WWSWW, WSWWW, SWWWW, WWWWS,
/// WWWSW.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Pulldown {
    #[default]
    Off,
    /// 3:2 pulldown: four film frames spread over five video frames, two of
    /// them split between neighbours.
    ThreeTwo(u8),
    /// 24Pa, the advanced cadence: one split frame in five, which is dropped.
    Advanced(u8),
}

/// How one file is read. The default reads it exactly as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Interpretation {
    pub alpha: Alpha,
    /// Swap what the alpha calls opaque and clear.
    pub invert_alpha: bool,
    /// Play the file's frames at this rate instead of its own. The same
    /// frames, so a clip conformed to half its rate lasts twice as long.
    pub conform_fps: Option<(u32, u32)>,
    pub fields: Fields,
    /// Fill a field's missing rows along the edges in the picture rather than
    /// straight down. Slower, and kinder to diagonals.
    pub preserve_edges: bool,
    /// Only read when [`Self::fields`] is on: the cadence is made of fields.
    pub pulldown: Pulldown,
    /// How much wider than tall one pixel is, as a fraction. `None` is square.
    pub pixel_aspect: Option<(u32, u32)>,
    /// How many times the file plays through. Nought and one both mean once.
    pub loops: u32,
    /// Whether the file's numbers are linear light. `None` is as the sample
    /// width says: float is, eight-bit carries the sRGB curve. `Some(true)`
    /// takes eight-bit numbers as linear too, and `Some(false)` says a float
    /// file carries the curve.
    pub linear: Option<bool>,
}

/// Which of the file's own frames make one interpreted frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// One frame as it is.
    Whole(usize),
    /// One field of a frame, its other rows filled in.
    Field { frame: usize, second: bool },
    /// The earlier field of one frame with the later field of another: a film
    /// frame that pulldown split across two.
    Weave { first: usize, second: usize },
}

/// For each film frame of a cadence, the video frame its earlier field is in
/// and the one its later field is in, counted from the cadence's start.
type Cadence = [(usize, usize); 4];

const THREE_TWO: Cadence = [(0, 0), (1, 1), (3, 2), (4, 4)];
const ADVANCED: Cadence = [(0, 0), (1, 1), (3, 3), (4, 4)];

impl Interpretation {
    /// Whether this reads the file exactly as it is.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// Whether the rows of a frame are being pulled apart, which only the
    /// file's own full-size frames can stand. A scaled-down copy has already
    /// blended its fields together.
    #[must_use]
    pub fn separates_fields(&self) -> bool {
        self.fields != Fields::Off
    }

    fn cadence(&self) -> Option<(Cadence, usize)> {
        if self.fields == Fields::Off {
            return None;
        }
        match self.pulldown {
            Pulldown::Off => None,
            Pulldown::ThreeTwo(at) => Some((THREE_TWO, usize::from(at % 5))),
            Pulldown::Advanced(at) => Some((ADVANCED, usize::from(at % 5))),
        }
    }

    /// How many frames one play-through has, given the file's own count.
    #[must_use]
    pub fn frames_per_pass(&self, raw: usize) -> usize {
        // One picture is a still, which has no fields or cadence to read.
        if raw <= 1 {
            return raw;
        }
        match (self.fields, self.cadence()) {
            (Fields::Off, _) => raw,
            (_, None) => raw.saturating_mul(2),
            (_, Some((cadence, at))) => {
                // Film frames are counted from the cadence's own start, which
                // is `at` frames before the file's. The ones that begin before
                // the file does are lost, as are any the file ends part way
                // through.
                let lost = cadence.iter().filter(|(a, b)| *a.min(b) < at).count();
                let end = raw.saturating_add(at);
                let whole = (end / 5).saturating_mul(4);
                let part = cadence.iter().filter(|(a, b)| *a.max(b) < end % 5).count();
                (whole + part).saturating_sub(lost)
            }
        }
    }

    /// How many frames the source plays in all. A still is one frame however
    /// many times it is told to play.
    #[must_use]
    pub fn frame_count(&self, raw: usize) -> usize {
        if raw <= 1 {
            return raw;
        }
        self.frames_per_pass(raw)
            .saturating_mul(self.loops.max(1) as usize)
    }

    /// The rate the interpreted frames run at, given the file's own.
    ///
    /// Separated fields are two pictures a frame, so they run twice as fast,
    /// and removing pulldown leaves four frames of every five.
    #[must_use]
    pub fn fps(&self, native: (i32, i32)) -> (i32, i32) {
        let (num, den) = match self.conform_fps {
            Some((n, d)) if n > 0 && d > 0 => (i64::from(n), i64::from(d)),
            _ => (i64::from(native.0), i64::from(native.1)),
        };
        let (num, den) = match (self.fields, self.cadence()) {
            (Fields::Off, _) => (num, den),
            (_, None) => (num * 2, den),
            (_, Some(_)) => (num * 4, den * 5),
        };
        let by = gcd(num.unsigned_abs(), den.unsigned_abs()).max(1) as i64;
        match (i32::try_from(num / by), i32::try_from(den / by)) {
            (Ok(n), Ok(d)) => (n, d),
            _ => native,
        }
    }

    /// How long the picture lasts, given the file's own length and rate.
    #[must_use]
    pub fn duration(&self, raw_seconds: f64, native: (i32, i32)) -> f64 {
        let conformed = match self.conform_fps {
            Some((n, d)) if n > 0 && d > 0 && native.0 > 0 && native.1 > 0 => {
                raw_seconds * (f64::from(native.0) / f64::from(native.1))
                    / (f64::from(n) / f64::from(d))
            }
            _ => raw_seconds,
        };
        conformed * f64::from(self.loops.max(1))
    }

    /// How wide the picture is once its pixels are square.
    #[must_use]
    pub fn width(&self, raw: u32) -> u32 {
        match self.pixel_aspect {
            Some((n, d)) if n > 0 && d > 0 && n != d => {
                let wide = (u64::from(raw) * u64::from(n) + u64::from(d) / 2) / u64::from(d);
                u32::try_from(wide).unwrap_or(raw).max(1)
            }
            _ => raw,
        }
    }

    /// Which of the file's `raw` frames interpreted frame `n` is made of. A
    /// frame past the end is the last one, as it is for a plain file.
    #[must_use]
    pub fn pick(&self, n: usize, raw: usize) -> Pick {
        let pass = self.frames_per_pass(raw);
        if pass == 0 || raw <= 1 {
            return Pick::Whole(0);
        }
        let n = n.min(self.frame_count(raw).saturating_sub(1)) % pass;
        match (self.fields, self.cadence()) {
            (Fields::Off, _) => Pick::Whole(n),
            (_, None) => Pick::Field {
                frame: n / 2,
                second: n % 2 == 1,
            },
            (_, Some((cadence, at))) => {
                let lost = cadence.iter().filter(|(a, b)| *a.min(b) < at).count();
                let film = n + lost;
                let (first, second) = cadence[film % 4];
                let frame = |i: usize| ((film / 4) * 5 + i).saturating_sub(at);
                if first == second {
                    Pick::Whole(frame(first))
                } else {
                    Pick::Weave {
                        first: frame(first),
                        second: frame(second),
                    }
                }
            }
        }
    }

    /// The file's own frames that interpreted frame `n` is made of: one, or
    /// two for a film frame woven back together.
    pub fn raw_frames(&self, n: usize, raw: usize) -> impl Iterator<Item = usize> {
        let (first, second) = match self.pick(n, raw) {
            Pick::Whole(frame) | Pick::Field { frame, .. } => (frame, None),
            Pick::Weave { first, second } => (first, Some(second)),
        };
        std::iter::once(first).chain(second)
    }

    /// The row parity (0 for even rows) a field keeps: the earlier field's
    /// rows, or the later one's when `second`.
    #[must_use]
    pub fn parity(&self, second: bool) -> usize {
        usize::from((self.fields == Fields::Lower) != second)
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// One channel of a pixel as a number, whichever width the frame's samples are.
/// Eight-bit reads as 0 to 1, so the same sums serve both.
trait Sample {
    const BYTES: usize;
    fn read(px: &[u8], channel: usize) -> f32;
    fn write(px: &mut [u8], channel: usize, value: f32);
}

struct Byte;
struct Float;

impl Sample for Byte {
    const BYTES: usize = 1;
    fn read(px: &[u8], channel: usize) -> f32 {
        px.get(channel).map_or(0.0, |v| f32::from(*v) / 255.0)
    }
    fn write(px: &mut [u8], channel: usize, value: f32) {
        if let Some(slot) = px.get_mut(channel) {
            *slot = (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
}

impl Sample for Float {
    const BYTES: usize = 4;
    fn read(px: &[u8], channel: usize) -> f32 {
        px.get(channel * 4..channel * 4 + 4)
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
            .map_or(0.0, f32::from_le_bytes)
    }
    fn write(px: &mut [u8], channel: usize, value: f32) {
        if let Some(slot) = px.get_mut(channel * 4..channel * 4 + 4) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
    }
}

/// Call a function generic over [`Sample`] at the frame's own sample width.
macro_rules! by_format {
    ($format:expr, $call:ident($($arg:expr),*)) => {
        match $format {
            PixelFormat::Srgb8 => $call::<Byte>($($arg),*),
            PixelFormat::LinearF32 => $call::<Float>($($arg),*),
        }
    };
}

/// Read a frame's alpha the way the interpretation says. The frame stays
/// straight, which is what the engine takes. `linear` is whether the frame's
/// numbers are linear light, since the matte colour has to be in whatever
/// they are.
pub fn interpret_alpha(frame: &mut DecodedFrame, alpha: Alpha, invert: bool, linear: bool) {
    if alpha == Alpha::Straight && !invert {
        return;
    }
    let matte = match alpha {
        Alpha::Premultiplied(c) if linear => Some(c.map(|v| srgb_to_linear(f32::from(v) / 255.0))),
        Alpha::Premultiplied(c) => Some(c.map(|v| f32::from(v) / 255.0)),
        _ => None,
    };
    by_format!(
        frame.format,
        alpha_pixels(&mut frame.rgba, alpha, matte, invert)
    );
}

fn alpha_pixels<S: Sample>(rgba: &mut [u8], alpha: Alpha, matte: Option<[f32; 3]>, invert: bool) {
    for px in rgba.chunks_exact_mut(S::BYTES * 4) {
        let a = S::read(px, 3);
        if let Some(matte) = matte {
            // Saved colour is the true colour times alpha plus the matte
            // showing through the rest, so take the matte's share back out.
            if a > 0.0 && a < 1.0 {
                for (c, m) in matte.iter().enumerate() {
                    S::write(px, c, (S::read(px, c) - m * (1.0 - a)) / a);
                }
            }
        }
        let a = if invert { 1.0 - a } else { a };
        S::write(px, 3, if alpha == Alpha::Ignore { 1.0 } else { a });
    }
}

/// Everything an interpretation does to one frame's pixels once its rows
/// and its size are settled: the alpha, then what the numbers mean.
#[must_use]
pub fn finish(mut frame: DecodedFrame, it: &Interpretation) -> DecodedFrame {
    // What the numbers are said to be, which the sample width only suggests.
    let linear = it.linear.unwrap_or(frame.format == PixelFormat::LinearF32);
    interpret_alpha(&mut frame, it.alpha, it.invert_alpha, linear);
    match (it.linear, frame.format) {
        // Eight-bit numbers that are already linear go on as floats, which is
        // the form the engine takes linear numbers in and decodes nothing of.
        (Some(true), PixelFormat::Srgb8) => DecodedFrame {
            rgba: frame
                .rgba
                .iter()
                .flat_map(|v| (f32::from(*v) / 255.0).to_le_bytes())
                .collect(),
            format: PixelFormat::LinearF32,
            ..frame
        },
        (Some(false), PixelFormat::LinearF32) => {
            for px in frame.rgba.chunks_exact_mut(16) {
                for c in 0..3 {
                    Float::write(px, c, srgb_to_linear(Float::read(px, c)));
                }
            }
            frame
        }
        _ => frame,
    }
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Keep the rows of one field and fill the others in from them, in place.
///
/// `parity` is the kept rows' (0 for even). Plainly, a missing row is the
/// average of the rows above and below it. With `preserve_edges` each pixel is
/// averaged along whichever of three directions its neighbours agree on most,
/// so a diagonal edge stays a line instead of a staircase.
pub fn fill_field(frame: &mut DecodedFrame, parity: usize, preserve_edges: bool) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    by_format!(
        frame.format,
        fill_rows(&mut frame.rgba, w, h, parity & 1, preserve_edges)
    );
}

fn fill_rows<S: Sample>(rgba: &mut [u8], w: usize, h: usize, parity: usize, edges: bool) {
    let px = S::BYTES * 4;
    let row = w * px;
    if row == 0 || rgba.len() < row * h {
        return;
    }
    for y in (0..h).filter(|y| y % 2 != parity) {
        let (before, rest) = rgba.split_at_mut(y * row);
        let (this, after) = rest.split_at_mut(row);
        let up = (y > 0).then(|| &before[(y - 1) * row..]);
        let down = (y + 1 < h).then(|| &after[..row]);
        match (up, down) {
            (Some(up), Some(down)) => {
                for x in 0..w {
                    let (a, b) = if edges {
                        edge_pair::<S>(up, down, x, w)
                    } else {
                        (x, x)
                    };
                    for c in 0..4 {
                        let v = (S::read(&up[a * px..], c) + S::read(&down[b * px..], c)) / 2.0;
                        S::write(&mut this[x * px..], c, v);
                    }
                }
            }
            // The top or bottom row has one neighbour, so it is that row.
            (Some(only), None) | (None, Some(only)) => this.copy_from_slice(&only[..row]),
            (None, None) => {}
        }
    }
}

/// The pixel above and the pixel below that agree best across `x`: straight
/// down, or along either diagonal.
fn edge_pair<S: Sample>(up: &[u8], down: &[u8], x: usize, w: usize) -> (usize, usize) {
    let px = S::BYTES * 4;
    let apart = |a: usize, b: usize| {
        (0..3)
            .map(|c| (S::read(&up[a * px..], c) - S::read(&down[b * px..], c)).abs())
            .sum::<f32>()
    };
    let mut best = (x, x);
    let mut least = apart(x, x);
    if x > 0 && x + 1 < w {
        for pair in [(x - 1, x + 1), (x + 1, x - 1)] {
            let d = apart(pair.0, pair.1);
            // Straight down wins a tie, so flat areas are left alone.
            if d + 1e-4 < least {
                least = d;
                best = pair;
            }
        }
    }
    best
}

/// One field's rows, converted as a picture of their own, set back on the
/// rows of `parity` in a frame `height` tall. The rows between are left for
/// [`fill_field`].
#[must_use]
pub fn spread_field(field: DecodedFrame, height: u32, parity: usize) -> DecodedFrame {
    let row = field.width as usize * field.format.bytes_per_px();
    let mut rgba = vec![0u8; row * height as usize];
    if row > 0 {
        let rows = rgba.chunks_exact_mut(row).skip(parity & 1).step_by(2);
        for (to, from) in rows.zip(field.rgba.chunks_exact(row)) {
            to.copy_from_slice(from);
        }
    }
    DecodedFrame {
        height,
        rgba,
        ..field
    }
}

/// Put the rows of `parity` from `first` into `second`, which keeps its other
/// rows: the two halves of a film frame that pulldown split across two frames.
#[must_use]
pub fn weave(first: &DecodedFrame, mut second: DecodedFrame, parity: usize) -> DecodedFrame {
    let row = second.width as usize * second.format.bytes_per_px();
    if row == 0 || first.rgba.len() != second.rgba.len() {
        return second;
    }
    let rows = second
        .rgba
        .chunks_exact_mut(row)
        .zip(first.rgba.chunks_exact(row));
    for (to, from) in rows.skip(parity & 1).step_by(2) {
        to.copy_from_slice(from);
    }
    second
}

/// Stretch or shrink a frame to `width` by `height`.
// ponytail: bilinear, so a field-separated clip previewed far below full size
// aliases a little. Averaging the pixels each one covers is the upgrade.
#[must_use]
pub fn resize(frame: DecodedFrame, width: u32, height: u32) -> DecodedFrame {
    if (width, height) == (frame.width, frame.height) || width == 0 || height == 0 {
        return frame;
    }
    let rgba = by_format!(
        frame.format,
        resize_pixels(
            &frame.rgba,
            frame.width as usize,
            frame.height as usize,
            width as usize,
            height as usize
        )
    );
    DecodedFrame {
        width,
        height,
        rgba,
        format: frame.format,
    }
}

fn resize_pixels<S: Sample>(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let px = S::BYTES * 4;
    let mut out = vec![0u8; dw * dh * px];
    if sw == 0 || sh == 0 || src.len() < sw * sh * px {
        return out;
    }
    // Where an output pixel's centre lands in the source, and the two source
    // pixels it sits between.
    let place = |i: usize, from: usize, to: usize| {
        let at = ((i as f32 + 0.5) * from as f32 / to as f32 - 0.5).clamp(0.0, (from - 1) as f32);
        let lo = at.floor() as usize;
        (lo, (lo + 1).min(from - 1), at - lo as f32)
    };
    for y in 0..dh {
        let (y0, y1, fy) = place(y, sh, dh);
        for x in 0..dw {
            let (x0, x1, fx) = place(x, sw, dw);
            let at = |xx: usize, yy: usize| &src[(yy * sw + xx) * px..];
            let to = &mut out[(y * dw + x) * px..];
            for c in 0..4 {
                let top = S::read(at(x0, y0), c) * (1.0 - fx) + S::read(at(x1, y0), c) * fx;
                let bottom = S::read(at(x0, y1), c) * (1.0 - fx) + S::read(at(x1, y1), c) * fx;
                S::write(to, c, top * (1.0 - fy) + bottom * fy);
            }
        }
    }
    out
}

/// What a frame's alpha looks like it means.
///
/// A premultiplied picture can't hold a colour brighter than its alpha lets
/// through, so one pixel that does settles it as straight. `None` for a
/// picture with no transparency in it, which has nothing to tell.
#[must_use]
pub fn guess_alpha(frame: &DecodedFrame) -> Option<Alpha> {
    let (black, white, seen) = by_format!(frame.format, matte_fits(&frame.rgba));
    match (seen, black, white) {
        (false, _, _) => None,
        (_, true, _) => Some(Alpha::Premultiplied([0; 3])),
        (_, _, true) => Some(Alpha::Premultiplied([255; 3])),
        _ => Some(Alpha::Straight),
    }
}

/// Whether every see-through pixel could have been mixed into black, into
/// white, and whether there were any at all.
fn matte_fits<S: Sample>(rgba: &[u8]) -> (bool, bool, bool) {
    // Two eight-bit steps of slack, for the rounding a codec leaves behind.
    const SLACK: f32 = 2.0 / 255.0;
    let (mut black, mut white, mut seen) = (true, true, false);
    for px in rgba.chunks_exact(S::BYTES * 4) {
        let a = S::read(px, 3);
        if a >= 1.0 {
            continue;
        }
        seen = true;
        for c in 0..3 {
            let v = S::read(px, c);
            black &= v <= a + SLACK;
            white &= v >= 1.0 - a - SLACK;
        }
        if !black && !white {
            break;
        }
    }
    (black, white, seen)
}

/// How much a frame looks like two different moments woven together: how far
/// each row sits from the average of its neighbours, against how far it sits
/// from the row two away. Near 1 for a clean frame, well above for a split one.
#[must_use]
pub fn comb_score(frame: &DecodedFrame) -> f64 {
    let (w, h) = (frame.width as usize, frame.height as usize);
    by_format!(frame.format, comb(&frame.rgba, w, h))
}

fn comb<S: Sample>(rgba: &[u8], w: usize, h: usize) -> f64 {
    let px = S::BYTES * 4;
    let row = w * px;
    if h < 4 || row == 0 || rgba.len() < row * h {
        return 1.0;
    }
    let luma = |y: usize, x: usize| {
        let at = &rgba[y * row + x * px..];
        f64::from(S::read(at, 0) + S::read(at, 1) + S::read(at, 2))
    };
    let (mut woven, mut clean) = (0.0, 0.0);
    // Every fourth pixel is plenty to tell, and keeps a guess quick.
    for y in (1..h - 2).step_by(2) {
        for x in (0..w).step_by(4) {
            woven += (luma(y, x) - (luma(y - 1, x) + luma(y + 1, x)) / 2.0).abs();
            clean += (luma(y, x) - luma(y + 2, x)).abs();
        }
    }
    (woven + 1e-6) / (clean + 1e-6)
}

/// The cadence a run of comb scores shows, where `scores[i]` is the file's
/// frame `start + i`. `Off` when no cadence stands clear of the rest.
#[must_use]
pub fn guess_pulldown(scores: &[f64], start: usize) -> Pulldown {
    if scores.len() < 10 {
        return Pulldown::Off;
    }
    // For a cadence starting `at` frames in: how far the least combed of the
    // frames it calls split stands above the most combed of those it calls
    // whole. Real footage is mostly still, so a split frame scores only a
    // little higher, but in a true cadence every one of them does.
    let stands_out = |split: &[usize], at: usize| {
        let (mut least_split, mut most_whole) = (f64::INFINITY, 0.0_f64);
        for (i, score) in scores.iter().enumerate() {
            if split.contains(&((start + i + at) % 5)) {
                least_split = least_split.min(*score);
            } else {
                most_whole = most_whole.max(*score);
            }
        }
        least_split / most_whole.max(1e-9)
    };
    let mut best = (Pulldown::Off, 1.15);
    for at in 0..5u8 {
        for (split, kind) in [
            (&[2usize, 3][..], Pulldown::ThreeTwo(at)),
            (&[2usize][..], Pulldown::Advanced(at)),
        ] {
            let ratio = stands_out(split, usize::from(at));
            if ratio > best.1 {
                best = (kind, ratio);
            }
        }
    }
    best.0
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32, px: impl Fn(u32, u32) -> [u8; 4]) -> DecodedFrame {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&px(x, y));
            }
        }
        DecodedFrame {
            width,
            height,
            rgba,
            format: PixelFormat::Srgb8,
        }
    }

    /// The frame arithmetic is what every layer's timing hangs off, so a
    /// conformed, field-separated or looped clip must count its frames and
    /// name the right ones.
    #[test]
    fn fields_loops_and_conform_count_and_pick_the_right_frames() {
        let fields = Interpretation {
            fields: Fields::Lower,
            ..Default::default()
        };
        assert_eq!(fields.frame_count(10), 20);
        assert_eq!(fields.fps((30000, 1001)), (60000, 1001));
        assert_eq!(
            fields.pick(7, 10),
            Pick::Field {
                frame: 3,
                second: true
            }
        );
        // Lower first: the earlier field is the odd rows.
        assert_eq!((fields.parity(false), fields.parity(true)), (1, 0));

        let looped = Interpretation {
            loops: 3,
            conform_fps: Some((12, 1)),
            ..Default::default()
        };
        assert_eq!(looped.frame_count(10), 30);
        assert_eq!(looped.pick(13, 10), Pick::Whole(3));
        assert_eq!(looped.pick(99, 10), Pick::Whole(9));
        // A still is one picture whatever it is told: no fields, no loop.
        let all = Interpretation {
            fields: Fields::Upper,
            pulldown: Pulldown::ThreeTwo(2),
            loops: 4,
            ..Default::default()
        };
        assert_eq!((all.frame_count(1), all.pick(3, 1)), (1, Pick::Whole(0)));
        assert_eq!(all.frame_count(0), 0);
        assert_eq!(looped.fps((24, 1)), (12, 1));
        // The same frames at half the rate last twice as long, three times over.
        assert!((looped.duration(2.0, (24, 1)) - 12.0).abs() < 1e-9);
    }

    /// Removing 3:2 pulldown must hand back the four film frames in order,
    /// whichever frame of the cadence the file starts on.
    #[test]
    fn pulldown_removal_finds_each_film_frame_at_every_phase() {
        // Video frame `v` of an unbroken WWSSW cadence: which film frame its
        // earlier and later fields show.
        let fields_of = |v: usize| {
            let (cycle, i) = (v / 5, v % 5);
            let (a, b) = [(0, 0), (1, 1), (1, 2), (2, 3), (3, 3)][i];
            (cycle * 4 + a, cycle * 4 + b)
        };
        for at in 0..5u8 {
            let it = Interpretation {
                fields: Fields::Upper,
                pulldown: Pulldown::ThreeTwo(at),
                ..Default::default()
            };
            // The file's frame r is cadence frame r + at.
            let film_of = |pick: Pick| match pick {
                Pick::Whole(r) => {
                    let (a, b) = fields_of(r + usize::from(at));
                    assert_eq!(a, b, "a whole pick must be a whole frame");
                    a
                }
                Pick::Weave { first, second } => {
                    let a = fields_of(first + usize::from(at)).0;
                    let b = fields_of(second + usize::from(at)).1;
                    assert_eq!(a, b, "both halves must be the same film frame");
                    a
                }
                Pick::Field { .. } => panic!("pulldown never hands back a lone field"),
            };
            let count = it.frame_count(25);
            let films: Vec<usize> = (0..count).map(|n| film_of(it.pick(n, 25))).collect();
            assert!(count >= 18, "phase {at} lost too many frames: {count}");
            assert!(
                films.windows(2).all(|w| w[1] == w[0] + 1),
                "phase {at} film frames out of order: {films:?}"
            );
            assert_eq!(it.fps((30000, 1001)), (24000, 1001));
        }
    }

    /// A premultiplied picture read as straight keeps a fringe of its matte
    /// colour. Unmatting must give the true colour back, and ignore and invert
    /// must do what they say.
    #[test]
    fn alpha_is_unmatted_ignored_and_inverted() {
        // Pure red at half alpha, saved over white: 255 red, and green and
        // blue showing half the white through.
        let saved = || frame(1, 1, |_, _| [255, 128, 128, 128]);
        let mut f = saved();
        interpret_alpha(&mut f, Alpha::Premultiplied([255; 3]), false, false);
        assert_eq!(f.rgba[3], 128);
        assert!(
            f.rgba[0] >= 254 && f.rgba[1] <= 2 && f.rgba[2] <= 2,
            "{:?}",
            f.rgba
        );

        let mut f = saved();
        interpret_alpha(&mut f, Alpha::Ignore, true, false);
        assert_eq!(f.rgba, [255, 128, 128, 255]);

        let mut f = saved();
        interpret_alpha(&mut f, Alpha::Straight, true, false);
        assert_eq!(f.rgba, [255, 128, 128, 127]);
    }

    /// A field keeps its own rows untouched and fills the rest from them, and
    /// a woven frame takes each half from the frame it was asked to.
    #[test]
    fn a_field_fills_its_missing_rows_and_a_weave_takes_both_halves() {
        // Even rows 100, odd rows 200: two different moments woven together.
        let woven = || frame(4, 4, |_, y| [if y % 2 == 0 { 100 } else { 200 }; 4]);
        let mut upper = woven();
        fill_field(&mut upper, 0, false);
        assert!(upper.rgba.iter().all(|v| *v == 100), "{:?}", upper.rgba);
        let mut lower = woven();
        fill_field(&mut lower, 1, true);
        assert!(lower.rgba.iter().all(|v| *v == 200), "{:?}", lower.rgba);

        let a = frame(2, 4, |_, _| [10; 4]);
        let b = frame(2, 4, |_, _| [20; 4]);
        let out = weave(&a, b, 1);
        let rows: Vec<u8> = out.rgba.chunks(8).map(|r| r[0]).collect();
        assert_eq!(rows, [20, 10, 20, 10]);
        assert!(comb_score(&woven()) > 3.0);
        assert!(comb_score(&frame(4, 8, |_, y| [(y * 20) as u8; 4])) < 1.5);
    }

    /// The guesses are only worth having if they name the right answer.
    #[test]
    fn the_guesses_name_the_matte_and_the_cadence() {
        let over_black = frame(2, 1, |x, _| {
            if x == 0 {
                [60, 0, 0, 64]
            } else {
                [9, 9, 9, 255]
            }
        });
        assert_eq!(guess_alpha(&over_black), Some(Alpha::Premultiplied([0; 3])));
        let over_white = frame(1, 1, |_, _| [255, 200, 200, 64]);
        assert_eq!(
            guess_alpha(&over_white),
            Some(Alpha::Premultiplied([255; 3]))
        );
        let straight = frame(2, 1, |x, _| {
            if x == 0 {
                [255, 0, 0, 64]
            } else {
                [0, 0, 0, 0]
            }
        });
        assert_eq!(guess_alpha(&straight), Some(Alpha::Straight));
        assert_eq!(guess_alpha(&frame(1, 1, |_, _| [9, 9, 9, 255])), None);

        // WSSWW from the file's first frame is the cadence one frame in. The
        // scores start at the file's frame 3.
        let scores: Vec<f64> = (3..18)
            .map(|r| {
                if [2, 3].contains(&((r + 1) % 5)) {
                    6.0
                } else {
                    1.0
                }
            })
            .collect();
        assert_eq!(guess_pulldown(&scores, 3), Pulldown::ThreeTwo(1));
        assert_eq!(guess_pulldown(&[1.0; 15], 0), Pulldown::Off);
    }

    /// Linear light is a statement about what the numbers mean, so the
    /// numbers must come through unchanged in the form that says so.
    #[test]
    fn linear_numbers_go_on_as_they_are() {
        let it = Interpretation {
            linear: Some(true),
            ..Default::default()
        };
        let out = finish(frame(1, 1, |_, _| [51, 102, 255, 255]), &it);
        assert_eq!(out.format, PixelFormat::LinearF32);
        let px: Vec<f32> = (0..4).map(|c| Float::read(&out.rgba, c)).collect();
        assert!(
            (px[0] - 0.2).abs() < 1e-6 && (px[1] - 0.4).abs() < 1e-6,
            "{px:?}"
        );
        assert_eq!((px[2], px[3]), (1.0, 1.0));

        let curved = Interpretation {
            linear: Some(false),
            ..Default::default()
        };
        let back = finish(out, &curved);
        assert!((Float::read(&back.rgba, 0) - srgb_to_linear(0.2)).abs() < 1e-6);
        assert_eq!(Float::read(&back.rgba, 3), 1.0, "alpha has no curve");
    }

    #[test]
    fn square_pixels_widen_the_picture() {
        let dv = Interpretation {
            pixel_aspect: Some((10, 11)),
            ..Default::default()
        };
        assert_eq!(dv.width(720), 655);
        let stretched = resize(frame(2, 2, |x, _| [if x == 0 { 0 } else { 200 }; 4]), 4, 2);
        assert_eq!((stretched.width, stretched.height), (4, 2));
        assert!(stretched.rgba[0] < stretched.rgba[4] && stretched.rgba[8] < stretched.rgba[12]);
    }
}
