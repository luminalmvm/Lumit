//! The motion an effect is handed is measured on the picture, whatever width
//! its samples are.

use super::*;

/// A float plate with detail at a few sizes, its content moved `shift` pixels
/// to the right.
fn plate(w: u32, h: u32, shift: f32) -> lumit_media::decode::DecodedFrame {
    let mut rgba = Vec::with_capacity((w * h * 16) as usize);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 - shift, y as f32);
            let v = 0.4
                + 0.2 * (fx * 0.35 + fy * 0.1).sin() * (fy * 0.27).cos()
                + 0.15 * (fx * 0.07).sin() * (fy * 0.05).sin();
            for c in [v, v * 0.8, v * 0.6, 1.0] {
                rgba.extend_from_slice(&c.to_le_bytes());
            }
        }
    }
    lumit_media::decode::DecodedFrame {
        width: w,
        height: h,
        rgba,
        format: lumit_media::decode::PixelFormat::LinearF32,
    }
}

/// **A float plate's motion is its motion.** The field a Motion blur or a
/// Datamosh reads was measured with the eight-bit reader whatever the plate
/// was, so on an OpenEXR the bytes of each float were read as colours and the
/// motion came out as noise.
#[test]
fn a_float_plates_motion_is_measured_on_its_floats() {
    let (w, h, shift) = (128u32, 128u32, 6.0f32);
    let (layer, item) = (Uuid::now_v7(), Uuid::now_v7());
    let read = lumit_media::sequence::MediaSource::file("plate.exr");
    let mut pool = DecodePool::new();
    pool.preload(item, &read, 0, None, plate(w, h, 0.0));
    pool.preload(item, &read, 1, None, plate(w, h, shift));
    let job = CompJob {
        layer,
        at: 0.0,
        item,
        source: read.clone(),
        source_frame: 0,
        target_width: None,
        natural_w: w,
        natural_h: h,
        blend: None,
        flow: None,
        temporal: vec![(1, 1)],
        flow_neighbours: vec![(1, lumit_core::fx::temporal::FlowWant::Full)],
        slate: false,
        channels: None,
        shutter: Vec::new(),
        shutter_flow: None,
        cuts: Vec::new(),
    };
    let frame = pool
        .decode_comp(Uuid::now_v7(), 0, &[job], 0, &|_| {})
        .unwrap();
    let (_, field) = frame.layers[0]
        .flow_fields
        .iter()
        .find(|(o, _)| *o == 1)
        .expect("the field asked for");
    assert_eq!(
        (field.w as u32, field.h as u32),
        (w, h),
        "at the frame's own size"
    );
    // The middle of the frame, away from the edges the content slid in at.
    let mut us: Vec<f32> = Vec::new();
    let mut vs: Vec<f32> = Vec::new();
    for y in 32..96 {
        for x in 32..96 {
            let i = (y * w + x) as usize;
            us.push(field.texels[i * 4]);
            vs.push(field.texels[i * 4 + 1]);
        }
    }
    us.sort_by(f32::total_cmp);
    vs.sort_by(f32::total_cmp);
    let (mu, mv) = (us[us.len() / 2], vs[vs.len() / 2]);
    assert!(
        (mu - shift).abs() < 1.0 && mv.abs() < 1.0,
        "the plate moved {shift} px across and the field says ({mu}, {mv})"
    );
}
