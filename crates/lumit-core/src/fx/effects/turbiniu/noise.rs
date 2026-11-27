pub(super) struct Perlin([u8; 256]);

impl Perlin {
    pub(super) fn new(seed: u32) -> Self {
        let mut state = [0_u32; 624];
        state[0] = seed;
        for i in 1..624 {
            let prev = state[i - 1];
            state[i] = (prev ^ (prev >> 30))
                .wrapping_mul(1_812_433_253)
                .wrapping_add(i as u32);
        }
        for i in 0..624 {
            let pair = (state[i] & 0x8000_0000) | (state[(i + 1) % 624] & 0x7fff_ffff);
            state[i] =
                state[(i + 397) % 624] ^ (pair >> 1) ^ ((pair & 1).wrapping_neg() & 0x9908_b0df);
        }
        let mut p = std::array::from_fn(|i| i as u8);
        let mut words = state.into_iter();
        for i in 1..256 {
            let span = i as u32 + 1;
            let limit = (1_u64 << 32) / u64::from(span) * u64::from(span);
            let j = loop {
                let mut word = words.next().unwrap_or(0);
                word ^= word >> 11;
                word ^= (word << 7) & 0x9d2c_5680;
                word ^= (word << 15) & 0xefc6_0000;
                word ^= word >> 18;
                if u64::from(word) < limit {
                    break word % span;
                }
            };
            p.swap(i, j as usize);
        }
        Self(p)
    }

    pub(super) fn sample(&self, x: f32, y: f32, depth: f32, cycle: f32) -> f32 {
        if cycle <= 0.0 {
            return self.field(x, y, depth);
        }
        let phase = (depth / cycle).rem_euclid(1.0);
        let first = self.field(x, y, phase * cycle);
        let last = self.field(x, y, (phase - 1.0) * cycle);
        let spread = ((1.0 - phase).powi(2) + phase.powi(2)).sqrt();
        super::geometry::mix(first, last, phase) / spread
    }

    fn field(&self, x: f32, y: f32, z: f32) -> f32 {
        use super::geometry::mix;
        let [ix, iy, iz] = [x, y, z].map(|n| (n.floor() as i32) as usize & 255);
        let [x, y, z] = [x, y, z].map(|n| n - n.floor());
        let p = |i: usize| usize::from(self.0[i & 255]);
        let corner = |a, b, c| {
            let h = p(p(p(ix + a) + iy + b) + iz + c) & 15;
            let x = x - a as f32;
            let y = y - b as f32;
            let z = z - c as f32;
            let u = if h < 8 { x } else { y };
            let v = match h {
                0..=3 => y,
                12 | 14 => x,
                _ => z,
            };
            (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
        };
        let fade = |t: f32| t * t * t * (t * (6.0 * t - 15.0) + 10.0);
        let slab = |z| {
            mix(
                mix(corner(0, 0, z), corner(1, 0, z), fade(x)),
                mix(corner(0, 1, z), corner(1, 1, z), fade(x)),
                fade(y),
            )
        };
        mix(slab(0), slab(1), fade(z)) * 0.5
    }
}

pub(super) fn random(seed: usize, line: usize, salt: usize) -> f32 {
    let mut x = (seed as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (line as u64)
            .wrapping_add(1)
            .wrapping_mul(0xbf58_476d_1ce4_e5b9)
        ^ (salt as u64)
            .wrapping_add(1)
            .wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (x >> 40) as f32 / (1_u64 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::{random, Perlin};

    #[test]
    fn every_seed_varies_freely_and_strokes_do_not_echo() {
        for seed in [0, 1, 4999, 5000, 2499, 32768] {
            let mut values: Vec<u32> = (0..200)
                .map(|line| (random(seed, line, 4) * 1e6) as u32)
                .collect();
            values.sort_unstable();
            values.dedup();
            assert!(values.len() > 190, "seed {seed}: {} values", values.len());
        }
        let echoes = (0..200)
            .filter(|&line| random(7, line + 3, 1) == random(7, line, 4))
            .count();
        assert_eq!(echoes, 0);
    }

    #[test]
    fn a_looping_pattern_keeps_its_contrast() {
        let field = Perlin::new(3);
        let spread = |depth: f32| {
            let samples: Vec<f32> = (0..400)
                .map(|i| field.sample(i as f32 * 0.37, i as f32 * 0.23, depth, 2.0))
                .collect();
            let mean = samples.iter().sum::<f32>() / samples.len() as f32;
            (samples.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / samples.len() as f32).sqrt()
        };
        let (start, half) = (spread(0.0), spread(1.0));
        assert!((half / start - 1.0).abs() < 0.2, "{start} vs {half}");
    }
}
