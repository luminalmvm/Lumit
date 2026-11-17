//! The jobs signature: what decides whether an edit changed the sound.

use lumit_render::mix::jobs::AudioJob;

/// A change-detection fingerprint of a comp's mixed audio — the bridge twin of
/// the egui side's `audio_jobs_signature`: the ordered contributing sources
/// with their placements and Volumes, plus the mix length. Any edit that
/// changes what the comp sounds like changes this; an unchanged mix is a
/// no-op. Session-only (a `DefaultHasher` is fine here — never persisted).
pub(crate) fn jobs_signature(jobs: &[AudioJob], duration_s: f64, master_db: f64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    jobs.len().hash(&mut h);
    duration_s.to_bits().hash(&mut h);
    master_db.to_bits().hash(&mut h);
    for j in jobs {
        // Which strip it sums into: the same sound moved to another row is
        // another desk.
        j.layer.hash(&mut h);
        j.sound.hash(&mut h);
        j.in_s.to_bits().hash(&mut h);
        j.out_s.to_bits().hash(&mut h);
        j.offset_s.to_bits().hash(&mut h);
        hash_animation(&mut h, &j.volume.animation);
        hash_animation(&mut h, &j.pan.animation);
        j.carriers.len().hash(&mut h);
        for c in &j.carriers {
            c.offset_s.to_bits().hash(&mut h);
            hash_animation(&mut h, &c.volume.animation);
            hash_animation(&mut h, &c.pan.animation);
            // The fade of a clip a nested comp plays through, as a job's own
            // is folded below.
            format!("{:?}", c.fade).hash(&mut h);
            // A Precomp layer's rack is a stage on the sum arriving through
            // it, so editing that rack changes the mix without touching a
            // keyframe anywhere below.
            c.chain.is_some().hash(&mut h);
            if let Some(chain) = &c.chain {
                hash_chain(&mut h, chain);
            }
            // A Precomp's retime changes what its whole comp sounds like.
            if let Some(bus) = &c.retime {
                format!("{:?}", bus.retime.map).hash(&mut h);
                bus.zero_s.to_bits().hash(&mut h);
                format!("{:?}", bus.span).hash(&mut h);
                bus.retime.pitch_follows_speed.hash(&mut h);
            }
        }
        // A *Duck under* wire changes what the layer sounds like without
        // touching a keyframe, so the chain — wires and its drivers' values —
        // folds in. Debug text rather than a field-by-field walk: session-only,
        // like the rest of this hash, and a graph is a handful of nodes.
        if let Some(d) = &j.driven {
            format!("{:?}", d.graph).hash(&mut h);
        }
        // A clip's fade: its two lengths and its two shapes. A shape edit
        // moves no clip, so without this a fade the user has just drawn would
        // meet the no-op gate and never be heard. Debug text, like the graph
        // above.
        format!("{:?}", j.fade).hash(&mut h);
        // A retime changes what plays without moving the job.
        if let Some(r) = &j.retime {
            format!("{:?}", r.map).hash(&mut h);
            r.origin_s.to_bits().hash(&mut h);
            format!("{:?}", r.held).hash(&mut h);
            r.pitch_follows_speed.hash(&mut h);
        }
        // The clip's own rack and then the layer's. Whether each is there at
        // all is hashed beside its contents, because an fx switch turned off
        // is a chain that is simply gone.
        for chain in [&j.clip_chain, &j.chain] {
            chain.is_some().hash(&mut h);
            if let Some(c) = chain {
                hash_chain(&mut h, c);
            }
        }
    }
    h.finish()
}

/// What the desk reads that the jobs do not carry: each top-level layer's
/// routing, sends, solo, solo safe, lower behind, Gain, Volume and Pan, a
/// Bus layer's rack, and the comp's master keys and loudness target. Debug
/// text where a field is a structure, as the chains are folded.
pub(crate) fn desk_signature(comp: &lumit_core::model::Composition) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for l in &comp.layers {
        l.id.hash(&mut h);
        l.is_bus().hash(&mut h);
        (l.switches.audible, l.switches.solo, l.switches.solo_safe).hash(&mut h);
        format!("{:?}{:?}", l.audio_routes, l.duck).hash(&mut h);
        l.audio_gain_db.to_bits().hash(&mut h);
        hash_animation(&mut h, &l.volume_db.animation);
        hash_animation(&mut h, &l.pan.animation);
        if l.is_bus() && !l.effects.is_empty() {
            format!("{:?}", l.effects).hash(&mut h);
            super::rack_stamp().hash(&mut h);
        }
    }
    comp.master_volume_db.to_bits().hash(&mut h);
    if let Some(keys) = &comp.master_keys {
        hash_animation(&mut h, keys);
    }
    format!("{:?}", comp.loudness_target).hash(&mut h);
    h.finish()
}

/// Fold one insert chain into the signature: dropping a plugin on the row,
/// dragging one of its knobs or bypassing it all change what the comp sounds
/// like without touching a Volume keyframe, so the whole stack and its wiring
/// go in. Debug text rather than a field-by-field walk, and a stack is a
/// handful of entries.
fn hash_chain(
    h: &mut std::collections::hash_map::DefaultHasher,
    chain: &lumit_render::mix::jobs::AudioChain,
) {
    use std::hash::Hash;
    format!("{:?}", chain.effects).hash(h);
    format!("{:?}", chain.graph).hash(h);
    // Whether each of THIS chain's plugins is switched off folds in too (AP5):
    // flicking the switch changes what the chain opens without touching the
    // document, and the rebake this provokes is what actually silences the
    // plugin rather than only badging it. This chain's plugins, not the whole
    // list, so switching off a plugin no comp uses re-plans nothing.
    if let Ok(disabled) = lumit_aplug::ipc::broker::session_disabled().lock() {
        for effect in &chain.effects {
            let name = effect.effect.match_name.as_str();
            if let Some(id) = lumit_core::fx::builtins::audio_plugin_id(name) {
                disabled.contains(id).hash(h);
            }
        }
    }
}

/// Fold one Volume animation into the signature: static hashes as one f64,
/// keyframed hashes every key.
fn hash_animation(
    h: &mut std::collections::hash_map::DefaultHasher,
    a: &lumit_core::anim::Animation,
) {
    use std::hash::Hash;
    match a {
        lumit_core::anim::Animation::Static(v) => v.to_bits().hash(h),
        lumit_core::anim::Animation::Keyframed(keys) => {
            keys.len().hash(h);
            for k in keys {
                k.time.to_f64().to_bits().hash(h);
                k.value.to_bits().hash(h);
            }
        }
        lumit_core::anim::Animation::Expression(expr) => expr.hash(h),
    }
}
