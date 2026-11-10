use super::feature::Builder;
use super::*;
use std::cell::Cell;

const SR: f64 = RATE as f64;

/// A fixed sequence of roughly normal numbers.
struct Noise(u64);

impl Noise {
    /// SplitMix64, so nearby seeds give unrelated sound.
    fn uniform(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }

    fn normal(&mut self) -> f32 {
        (0..4).map(|_| self.uniform()).sum::<f32>() - 2.0
    }
}

/// Speech-like sound: noise under a level that jumps every 50 to 250 ms.
fn speech(seconds: f64, seed: u64) -> Vec<f32> {
    let mut noise = Noise(seed);
    let (mut level, mut left) = (0.0, 0);
    (0..(seconds * SR) as usize)
        .map(|_| {
            if left == 0 {
                level = noise.uniform().powi(3);
                left = 400 + (noise.uniform() * 1600.0) as usize;
            }
            left -= 1;
            level * noise.normal()
        })
        .collect()
}

/// `seconds` of `reference` from `from`, as a far camera mic hears it:
/// clock `ppm` fast against the reference, quieter, duller and noisier.
fn camera(reference: &[f32], from: f64, seconds: f64, ppm: f64, seed: u64) -> Vec<f32> {
    let mut noise = Noise(seed);
    let mut dull = 0.0;
    (0..(seconds * SR) as usize)
        .map(|n| {
            let at = from * SR + n as f64 * (1.0 + ppm * 1e-6);
            let i = at.floor() as usize;
            let f = (at - at.floor()) as f32;
            let x = reference.get(i).copied().unwrap_or(0.0) * (1.0 - f)
                + reference.get(i + 1).copied().unwrap_or(0.0) * f;
            dull += 0.3 * (x - dull);
            0.2 * dull + 0.01 * noise.normal()
        })
        .collect()
}

/// Onsets built from blocks of an odd size, as a decode would hand them.
fn onsets_of(sound: &[f32]) -> Onsets {
    let mut builder = Builder::new();
    for block in sound.chunks(10_007) {
        builder.push(block);
    }
    builder.finish()
}

struct Memory(Vec<Vec<f32>>);

impl Windows for Memory {
    fn window(&self, input: usize, start_seconds: f64, frames: usize) -> Option<Vec<f32>> {
        let sound = self.0.get(input)?;
        let start = (start_seconds * SR).round() as i64;
        Some(
            (0..frames as i64)
                .map(|i| {
                    usize::try_from(start + i)
                        .ok()
                        .and_then(|j| sound.get(j))
                        .copied()
                        .unwrap_or(0.0)
                })
                .collect(),
        )
    }
}

fn run(sounds: Vec<Vec<f32>>, drift: bool) -> Vec<Placed> {
    let curves: Vec<Onsets> = sounds.iter().map(|s| onsets_of(s)).collect();
    let inputs: Vec<Input> = curves
        .iter()
        .enumerate()
        .map(|(i, onsets)| Input {
            onsets,
            group: i as u32,
            order: 0,
            steady: false,
        })
        .collect();
    let windows = Memory(sounds);
    solve(&inputs, &windows, Options { drift }, &mut |_| {}, &|| false).unwrap()
}

fn assert_at(placed: &Placed, start: f64, speed: f64) {
    assert_eq!(placed.state, State::Synced, "{placed:?}");
    let at = placed.start.unwrap();
    assert!((at - start).abs() < 0.001, "start {at}, wanted {start}");
    let ppm = (placed.speed - speed) / speed * 1e6;
    assert!(ppm.abs() < 5.0, "speed {} off by {ppm} ppm", placed.speed);
}

#[test]
fn finds_offsets_through_gain_noise_filtering_and_drift() {
    let reference = speech(360.0, 1);
    let fast = camera(&reference, 60.0, 200.0, 50.0, 2);
    let steady = camera(&reference, 150.0, 150.0, 0.0, 3);
    let placed = run(vec![reference.clone(), fast.clone(), steady], true);
    assert_at(&placed[0], 0.0, 1.0);
    assert_at(&placed[1], 60.0, 1.0 / (1.0 + 50e-6));
    assert_at(&placed[2], 150.0, 1.0);

    // As a recorder that runs longest, against a camera that keeps its own
    // speed: the camera sets the clock and the recorder takes the drift.
    let curves = [onsets_of(&reference), onsets_of(&fast)];
    let inputs = [
        Input {
            onsets: &curves[0],
            group: 0,
            order: 0,
            steady: false,
        },
        Input {
            onsets: &curves[1],
            group: 1,
            order: 0,
            steady: true,
        },
    ];
    let windows = Memory(vec![reference, fast]);
    let placed = solve(
        &inputs,
        &windows,
        Options { drift: true },
        &mut |_| {},
        &|| false,
    )
    .unwrap();
    assert!((placed[1].speed - 1.0).abs() < 1e-12, "{placed:?}");
    let ppm = (placed[0].speed - (1.0 + 50e-6)) * 1e6;
    assert!(ppm.abs() < 5.0, "recorder speed off by {ppm} ppm");
}

#[test]
fn places_a_and_c_through_b_alone() {
    let reference = speech(330.0, 4);
    let a = camera(&reference, 0.0, 150.0, 0.0, 5);
    let b = camera(&reference, 100.0, 150.0, 0.0, 6);
    let c = camera(&reference, 200.0, 130.0, 0.0, 7);
    let placed = run(vec![a, b, c], false);
    assert_at(&placed[0], 0.0, 1.0);
    assert_at(&placed[1], 100.0, 1.0);
    assert_at(&placed[2], 200.0, 1.0);
}

#[test]
fn a_file_with_no_shared_sound_is_unmatched() {
    let reference = speech(90.0, 8);
    let clip = camera(&reference, 20.0, 60.0, 0.0, 9);
    let stranger = speech(60.0, 10);
    let placed = run(vec![reference, clip, stranger], false);
    assert_at(&placed[1], 20.0, 1.0);
    assert_eq!(placed[2].state, State::Unmatched);
    assert_eq!(placed[2].start, None);
}

#[test]
fn cancelling_stops_the_solve() {
    let reference = speech(90.0, 11);
    let clip = camera(&reference, 20.0, 60.0, 0.0, 12);
    let curves = [onsets_of(&reference), onsets_of(&clip)];
    let inputs: Vec<Input> = curves
        .iter()
        .enumerate()
        .map(|(i, onsets)| Input {
            onsets,
            group: i as u32,
            order: 0,
            steady: false,
        })
        .collect();
    let windows = Memory(vec![reference, clip]);
    let asked = Cell::new(0);
    let cancelled = || {
        asked.set(asked.get() + 1);
        asked.get() > 2
    };
    let result = solve(
        &inputs,
        &windows,
        Options::default(),
        &mut |_| {},
        &cancelled,
    );
    assert!(matches!(result, Err(SyncError::Cancelled)));
}

/// Two one-hour inputs, from their features on. Run with
/// `cargo test -p lumit-audio --release sync -- --ignored`.
#[test]
#[ignore]
fn two_hours_solve_quickly() {
    use std::io::Write;
    let frames = 3600 * FPS;
    let shift = 1234 * FPS / 10;
    let mut noise = Noise(13);
    let base: Vec<f32> = (0..frames + shift).map(|_| noise.normal()).collect();
    let first = Onsets {
        feature: base[shift..].to_vec(),
        seconds: 3600.0,
    };
    let second = Onsets {
        feature: base[..frames].iter().map(|v| v + noise.normal()).collect(),
        seconds: 3600.0,
    };
    // Sound to match: the second starts 123.4 s before the first.
    struct Hashed;
    impl Windows for Hashed {
        fn window(&self, input: usize, start: f64, frames: usize) -> Option<Vec<f32>> {
            let from = (start * SR).round() as i64 + if input == 0 { 987_200 } else { 0 };
            Some(
                (0..frames as i64)
                    .map(|i| Noise((from + i) as u64).normal())
                    .collect(),
            )
        }
    }
    let inputs = [
        Input {
            onsets: &first,
            group: 0,
            order: 0,
            steady: false,
        },
        Input {
            onsets: &second,
            group: 1,
            order: 0,
            steady: false,
        },
    ];
    let clock = std::time::Instant::now();
    let placed = solve(
        &inputs,
        &Hashed,
        Options { drift: true },
        &mut |_| {},
        &|| false,
    )
    .unwrap();
    let took = clock.elapsed();
    let _ = writeln!(std::io::stderr(), "two one-hour inputs solved in {took:?}");
    assert!(
        (placed[0].start.unwrap() - 123.4).abs() < 0.001,
        "{placed:?}"
    );
    assert!(took.as_secs_f64() < 2.0);
}
