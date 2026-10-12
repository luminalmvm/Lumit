//! A circular delay line with a fractional read.
//!
//! # In plain terms
//!
//! The echo, the reverb's combs, the chorus and the limiter's lookahead are
//! all this: sound written into a ring and read back from somewhere behind
//! the write head. The read takes a delay in samples and it may have a
//! fraction, because a chorus moves its delay smoothly and a delay rounded to
//! whole samples steps audibly as it moves.
//!
//! Two reads are offered. Linear is right for a delay that sits still and
//! costs almost nothing. Cubic is right for one that moves, where linear
//! interpolation acts as a low pass that opens and closes with the
//! modulation and dulls the top end.
//!
//! The buffer is taken once, at construction, from a maximum the effect knows
//! from its longest row. Nothing here allocates per sample.

/// Room past the maximum delay for the cubic read's outer taps.
const HEADROOM: usize = 4;

/// A ring of samples and the head that writes into it.
#[derive(Clone, Debug)]
pub struct DelayLine {
    buf: Vec<f32>,
    write: usize,
    max: usize,
}

impl DelayLine {
    /// A line that can be read up to `max_frames` behind. The effect works
    /// this out from its longest row before its first block.
    #[must_use]
    pub fn new(max_frames: usize) -> Self {
        let max = max_frames.max(1);
        Self {
            buf: vec![0.0; max + HEADROOM],
            write: 0,
            max,
        }
    }

    /// The longest delay this line answers. A longer read is held here.
    #[must_use]
    pub fn max_delay(&self) -> usize {
        self.max
    }

    /// Empty the line and put the write head back at the start.
    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
    }

    /// One sample in. `read_linear(0.0)` then answers it.
    pub fn push(&mut self, sample: f32) {
        if let Some(slot) = self.buf.get_mut(self.write) {
            *slot = sample;
        }
        self.write += 1;
        if self.write == self.buf.len() {
            self.write = 0;
        }
    }

    /// The sample `back` whole samples behind the write head.
    fn tap(&self, back: usize) -> f32 {
        let len = self.buf.len();
        let index = (self.write + len - 1 - back % len) % len;
        self.buf.get(index).copied().unwrap_or(0.0)
    }

    /// A delay of `delay` samples, straight between the two neighbours. The
    /// delay is held to the line's length, so a modulation that overshoots
    /// simply stops moving rather than reading somebody else's sound.
    #[must_use]
    pub fn read_linear(&self, delay: f64) -> f32 {
        let delay = delay.clamp(0.0, self.max as f64);
        let whole = delay.floor();
        let frac = (delay - whole) as f32;
        let index = whole as usize;
        let near = self.tap(index);
        let far = self.tap(index + 1);
        near + (far - near) * frac
    }

    /// The same delay through a four-point cubic Hermite, which is what a
    /// moving read wants. The delay is held to at least one sample, because
    /// the curve needs a neighbour on the near side and there is no sound
    /// ahead of the write head.
    #[must_use]
    pub fn read_cubic(&self, delay: f64) -> f32 {
        let delay = delay.clamp(1.0, self.max as f64);
        let whole = delay.floor();
        let frac = (delay - whole) as f32;
        let index = whole as usize;
        let y0 = self.tap(index - 1);
        let y1 = self.tap(index);
        let y2 = self.tap(index + 1);
        let y3 = self.tap(index + 2);
        // De Soras' four-point third-order Hermite, in order of increasing
        // delay: y1 and y2 are the neighbours, y0 and y3 give the slope.
        let c0 = y1;
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * frac + c2) * frac + c1) * frac + c0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_past_the_end_is_held_rather_than_wrapped() {
        let mut line = DelayLine::new(8);
        assert_eq!(line.max_delay(), 8);
        for n in 0..64 {
            line.push(n as f32);
        }
        assert_eq!(line.read_linear(1_000.0), line.read_linear(8.0));
        assert_eq!(line.read_cubic(0.0), line.read_cubic(1.0));
    }
}
