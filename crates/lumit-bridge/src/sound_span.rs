//! The sound the clean-up effects read: a window of what arrives at an
//! effect around the playhead, a span of it for Noise reduction's Learn, the
//! quietest stretch of that span, and the span run through the effects ahead
//! of the one learning.
//!
//! Runs on whichever thread asked; nothing here keeps state.

use std::sync::Arc;

use lumit_core::model::{Document, EffectInstance};

/// A mono window of the sound arriving at an effect on `layer`, `half`
/// seconds either side of comp time `t`, and its rate: the layer's own file
/// for a footage layer, the row's own mix before its effects otherwise.
pub(crate) fn sound_window(
    doc: &Arc<Document>,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t: f64,
    lt: f64,
    half: f64,
) -> (Vec<f32>, u32) {
    use lumit_core::fx::registry::AudioTap;
    let tap = lumit_render::audio_tap::DocumentAudio::new(doc, comp, t);
    let mut window = Vec::new();
    let rate = match &layer.kind {
        lumit_core::model::LayerKind::Footage { .. } => {
            tap.samples(layer.id, lt - half, lt + half, &mut window)
        }
        _ => tap.strip(Some(layer.id), None, half, &mut window),
    };
    match rate {
        Some(rate) if rate > 0.0 => (window, rate.round() as u32),
        _ => (Vec::new(), 0),
    }
}

/// `window` as `columns` pairs of its lowest and highest sample.
pub(crate) fn columns(window: &[f32], columns: usize) -> Vec<f64> {
    let per = window.len().div_ceil(columns.max(1)).max(1);
    window
        .chunks(per)
        .flat_map(|chunk| {
            let low = chunk.iter().copied().fold(0.0f32, f32::min);
            let high = chunk.iter().copied().fold(0.0f32, f32::max);
            [f64::from(low), f64::from(high)]
        })
        .collect()
}

/// The layer's sound from comp time `a` to `b`, mono, and its rate: the
/// layer's own file for a footage layer, the row's own mix before its effects
/// otherwise.
pub(crate) fn read_span(
    doc: &Arc<Document>,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    a: f64,
    b: f64,
) -> (Vec<f32>, u32) {
    let half = (b - a) / 2.0;
    let t = a + half;
    sound_window(doc, comp, layer, t, t - layer.start_offset.0.to_f64(), half)
}

/// The quietest `seconds` of `mono`, by the energy of 20 ms frames: the
/// stretch whose louder frames are quietest, so a word passing through it
/// counts against it.
pub(crate) fn quietest(mono: &[f32], rate: u32, seconds: f64) -> Option<Vec<f32>> {
    let hop = (f64::from(rate) * 0.02).round().max(1.0) as usize;
    let energy: Vec<f64> = mono
        .chunks(hop)
        .map(|c| c.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>() / c.len() as f64)
        .collect();
    let span = ((seconds * f64::from(rate)) as usize / hop).clamp(1, energy.len().max(1));
    if energy.len() <= span {
        return (!mono.is_empty()).then(|| mono.to_vec());
    }
    let mut best: Option<(f64, usize)> = None;
    for start in 0..=energy.len() - span {
        let mut frames: Vec<f64> = energy[start..start + span].to_vec();
        frames.sort_by(f64::total_cmp);
        // The ninetieth percentile: a stretch is as quiet as its louder part.
        let loud = frames[(frames.len() * 9 / 10).min(frames.len() - 1)];
        if best.is_none_or(|(b, _)| loud < b) {
            best = Some((loud, start));
        }
    }
    let (level, start) = best?;
    if level <= 0.0 {
        return None;
    }
    let from = start * hop;
    Some(mono[from..(from + span * hop).min(mono.len())].to_vec())
}

/// `stereo` run through the effects ahead of `fx` on its rack, so the profile
/// is learned from what the effect itself hears. Unchanged when nothing on
/// the rack comes before it.
pub(crate) fn ahead_of(
    doc: &Arc<Document>,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    fx: &EffectInstance,
    stereo: &[f32],
    at: f64,
    rate: u32,
) -> Vec<f32> {
    let rack = if layer.effects.iter().any(|e| e.id == fx.id) {
        layer.effects.clone()
    } else {
        match &layer.kind {
            lumit_core::model::LayerKind::Sequence { clips } => clips
                .iter()
                .find(|c| c.effects.iter().any(|e| e.id == fx.id))
                .map(|c| c.effects.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    };
    let ahead: Vec<EffectInstance> = rack
        .into_iter()
        .take_while(|e| e.id != fx.id)
        .filter(|e| e.enabled)
        .collect();
    if ahead.is_empty() {
        return stereo.to_vec();
    }
    let chain = lumit_render::export::AudioChain {
        doc: Arc::clone(doc),
        comp: comp.id,
        layer: layer.id,
        effects: ahead,
        graph: layer.graph.clone(),
        offset_s: layer.start_offset.0.to_f64(),
        base_s: 0.0,
    };
    let start = (at * f64::from(rate)).round() as i64;
    match lumit_render::export::chain_bake(&chain, stereo, start, rate, true) {
        Some((wet, latency)) => {
            let skip = (latency as usize * 2).min(wet.len());
            let mut out = wet[skip..].to_vec();
            out.truncate(stereo.len());
            out
        }
        None => stereo.to_vec(),
    }
}
