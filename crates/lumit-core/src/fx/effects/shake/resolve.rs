//! Shake's resolve step: the noise, the waves, the Channels and the two
//! legacy folds, sampled at resolve time and pushed as `derived.*` values.

use super::Shake;
use crate::fx::{
    maths::shake_jump_noise, maths::shake_mb_offsets, maths::shake_noise, maths::shake_twitch_gate,
    maths::SHAKE_MB_DEFAULT_SAMPLES, maths::SHAKE_MB_SAMPLES, params::ParamId, params::Value,
    registry::ResolveCx, schema::EdgesMode,
};
use crate::model::EffectValue;

/// See [`ShakeDef::resolve_derived`](super::ShakeDef).
pub(super) fn derived(cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
    let (e, lt) = (cx.inst, cx.lt);
    let fl = |id: &str| e.float_at_with_context(id, lt, cx.context.clone());
    let freq = fl("frequency").unwrap_or(8.0).max(0.0);
    let rot_freq = fl("rot_freq").unwrap_or(1.0).max(0.0);
    let x_freq = fl("x_freq").unwrap_or(1.0).max(0.0);
    let y_freq = fl("y_freq").unwrap_or(1.0).max(0.0);
    let z_freq = fl("z_freq").unwrap_or(1.0).max(0.0);
    let seed = match e.param("seed") {
        Some(EffectValue::Seed(s)) => *s,
        _ => 0,
    };
    // Independent noise channels sampled at local time × frequency (per
    // axis, §3.4) — deterministic, hop-free, identical on every machine
    // (§2.4). One sampler drives the frame-time wobble and the motion-blur
    // sub-frames, so they agree bit-for-bit. Phase holds the whole shake
    // back, as S_Shake's does; at 0 the base is exactly what it always was.
    // A version 1 shake still on version 1 has its Phase driven by an
    // expression, which load could not turn round, so it reads the old way.
    let ahead = if e.effect.version < 2 { 1.0 } else { -1.0 };
    let phase = ahead * fl("phase").unwrap_or(0.0);
    let base = (lt + phase) * freq;
    // Style: 0 Normal, 1 Twitchy, 2 Jumpy. Normal takes the same path as
    // before, and its gate is exactly 1, so an old shake keeps its bits.
    let style = match e.param("style") {
        Some(EffectValue::Choice(c)) => *c,
        _ => 0,
    };
    let drift = fl("drift").unwrap_or(0.3).clamp(0.0, 1.0);
    let centre = fl("centre_bias").unwrap_or(0.0);
    let stillness = fl("stillness").unwrap_or(0.7);
    let twitch_freq = fl("twitch_freq").unwrap_or(2.0).max(0.0);
    let sample = |channel: u32, x: f64| match style {
        2 => shake_jump_noise(seed, channel, x, drift, centre),
        _ => shake_noise(seed, channel, x),
    };

    // z (depth/scale) amount: the new id, else the old `zoom_pump` (a
    // project saved before FX-11 keeps its pump), a scale-pump per cent
    // either way.
    let z_pct = fl("z_amp").or_else(|| fl("zoom_pump")).unwrap_or(0.0) as f32;
    let z_amp = (z_pct / 100.0).clamp(0.0, 1.0);
    push(Shake::DERIVED_Z_AMP, Value::Float(z_amp));

    // The waves, as (amount, × Frequency, phase in radians) per axis in
    // the noise's order. Z's amount is a per cent like Z amount's.
    let wave = |[amount, hz, ph]: [&str; 3], per: f64| {
        (
            fl(amount).unwrap_or(0.0).max(0.0) / per,
            fl(hz).unwrap_or(0.5).max(0.0),
            fl(ph).unwrap_or(0.0).to_radians(),
        )
    };
    let waves = [
        wave(["x_wave", "x_wave_freq", "x_wave_phase"], 1.0),
        wave(["y_wave", "y_wave_freq", "y_wave_phase"], 1.0),
        wave(["rot_wave", "rot_wave_freq", "rot_wave_phase"], 1.0),
        wave(["z_wave", "z_wave_freq", "z_wave_phase"], 100.0),
    ];
    let waving = waves.iter().any(|w| w.0 > 0.0);
    // With a wave on, the random part takes its per-axis amount here, so
    // the wave can be added on top in the same units.
    let amounts = [
        fl("x_amp").unwrap_or(1.0).max(0.0),
        fl("y_amp").unwrap_or(1.0).max(0.0),
        fl("rotation").unwrap_or(1.0).max(0.0),
        f64::from(z_amp),
    ];
    if waving {
        push(Shake::DERIVED_PRESCALED, Value::Bool(true));
    }
    // Amplitude scales the twist and the zoom as well, against its default
    // of 30, so a fresh shake looks as it always did and 0 stills it. Before
    // version 3 they ignored it, and a shake load could not carry over still
    // reads that way.
    let twist = match e.effect.version < 3 {
        true => 1.0,
        false => fl("amplitude").unwrap_or(30.0).max(0.0) / 30.0,
    };
    // Channels: each colour's amount and phase, and the wobble of its own
    // that Channel randomness adds. Untouched, the channels stay together.
    let colours = [
        ("red_amount", "red_phase"),
        ("green_amount", "green_phase"),
        ("blue_amount", "blue_phase"),
    ]
    .map(|(a, ph)| (fl(a).unwrap_or(1.0), fl(ph).unwrap_or(0.0)));
    let randomness = fl("rgb_randomness").unwrap_or(0.0).max(0.0);
    let rgb_freq = fl("rgb_freq").unwrap_or(1.0).max(0.0);
    let split = randomness > 0.0 || colours.iter().any(|&(a, ph)| a != 1.0 || ph != 0.0);

    // The twitch gate runs on time, not on the noise base, so a
    // motion-blur sub-frame gets its time back from its base.
    // A colour's own wobble reads noise channels 16 and up, clear of the
    // ones the axes and Jumpy use.
    let noise = |b: f64, colour: Option<u32>| {
        let gate = match style {
            1 => {
                let t = if freq > 0.0 { b / freq } else { lt + phase };
                shake_twitch_gate(seed, t * twitch_freq, stillness)
            }
            _ => 1.0,
        };
        let axis = |i: u32, f: f64| match colour {
            None => sample(i, b * f) * gate,
            Some(c) => {
                (sample(i, b * f) + randomness * sample(16 + 8 * c + i, b * f * rgb_freq)) * gate
            }
        };
        let mut n = [
            axis(0, x_freq),
            axis(1, y_freq),
            axis(2, rot_freq),
            axis(3, z_freq),
        ];
        if waving {
            for (v, (k, (a, f, p))) in n.iter_mut().zip(amounts.into_iter().zip(waves)) {
                *v = *v * k + a * (std::f64::consts::TAU * f * b + p).sin();
            }
        }
        n[2] *= twist;
        n[3] *= twist;
        n
    };
    // × 1 leaves a value as it was, so a shake without Channels keeps its bits.
    let value = |n: [f64; 4], k: f64| Value::Vec4(n.map(|v| (v * k) as f32));
    push(Shake::DERIVED_NOISE, value(noise(base, None), 1.0));

    // Edges (P3): the stored Choice, else the old Auto-scale bool
    // (on → Repeat hides the border as the cover once did; off →
    // Transparent), else Repeat.
    let edge = match e.param("edge") {
        Some(EffectValue::Choice(c)) => {
            EdgesMode::from_code((*c).min(2)).unwrap_or(EdgesMode::Repeat)
        }
        _ => match e.param("auto_scale") {
            Some(EffectValue::Bool(false)) => EdgesMode::Transparent,
            _ => EdgesMode::Repeat,
        },
    };
    // Y edges ride in the bits above, plus one, so Same leaves the code
    // as it always was (cpu::bilinear_edge reads them).
    let y_edge = match e.param("y_edge") {
        Some(EffectValue::Choice(c)) => (*c).min(3),
        _ => 0,
    };
    push(
        Shake::DERIVED_EDGE,
        Value::Choice(edge.code() | (y_edge << 2)),
    );

    // The shake's own motion blur (T18): when the toggle is on and
    // the amount is non-zero, sample the wobble across the shutter for the
    // dispatch to average; off is the plain single resample (the bit-exact
    // passthrough). The centre offset is 0, so the middle sample is the
    // frame-time wobble exactly.
    let motion_blur = e.bool_of("motion_blur").unwrap_or(false);
    let mb_amount = fl("mb_amount").unwrap_or(0.5);
    // A shake saved before the Samples row keeps the nine it was made with.
    let mb_samples =
        fl("mb_samples").map_or(SHAKE_MB_DEFAULT_SAMPLES, |s| s.round().max(0.0) as usize);
    let blur = motion_blur && mb_amount > 0.0;
    if split {
        // Each colour's placements in turn, a third of the set each.
        push(Shake::DERIVED_SPLIT, Value::Bool(true));
        let mut ids = Shake::DERIVED_MB_NOISE.iter();
        for (c, (amount, ph)) in (0u32..).zip(colours) {
            // A channel's phase holds it back, as S_Shake's does.
            let at = base - ph * freq;
            let placements = shake_mb_offsets(mb_amount, mb_samples.min(SHAKE_MB_SAMPLES / 3))
                .filter(|_| blur)
                .chain((!blur).then_some(0.0));
            for (db, id) in placements.zip(ids.by_ref()) {
                push(*id, value(noise(at + db, Some(c)), amount));
            }
        }
    } else if blur {
        for (id, db) in Shake::DERIVED_MB_NOISE
            .iter()
            .zip(shake_mb_offsets(mb_amount, mb_samples))
        {
            push(*id, value(noise(base + db, None), 1.0));
        }
    }
}
