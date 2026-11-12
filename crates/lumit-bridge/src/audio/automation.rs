//! Faders held from the mixer, and the automation they write.
//!
//! # In plain terms
//!
//! A fader, a pan or a send level dragged on the mixer is heard at once: the
//! value goes straight into the playing plan's [`LiveBank`], ahead of
//! whatever the document says. With the transport stopped, or the strip in
//! Read, letting go writes the value the way the mixer always has. While the
//! comp plays in Touch, Latch, Write or Trim, every move is noted against the
//! heard clock, and when the pass ends (pause, stop, a seek, a loop going
//! round, another comp loading) the notes become keys ([`super::record`]).
//!
//! Everything here is bookkeeping under the audio state's lock: a few atomic
//! stores on the bank and a point pushed on a list. Keys are written on a
//! thread of their own, since that takes the project's lock.

use super::record::{write_pass, Track};
use super::{heard, kick_prepare, lock, AudioState, Loaded};
use crate::api::mixer::BridgeMixTarget;
use lumit_audio::live::LiveBank;
use lumit_audio::mix::{db_to_gain, pan_gains};
use lumit_core::mixer::AutomationMode;
use lumit_core::model::{Composition, Document, Layer};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// How long a fader let go in Touch takes to glide back to its keys, in
/// milliseconds: Settings, Audio, Touch return.
static TOUCH_RETURN_MS: AtomicU32 = AtomicU32::new(250);

/// How many edits the live controls have written. A held fader is let go
/// once a mix asked for at or after its edit lands.
static EDITS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn set_touch_return_ms(ms: u32) {
    TOUCH_RETURN_MS.store(ms, Ordering::Relaxed);
}

/// The edit count now, for a prepare to carry.
pub(super) fn edits() -> u64 {
    EDITS.load(Ordering::Acquire)
}

/// The held controls and the pass being written.
#[derive(Default)]
pub(crate) struct Live {
    holds: Vec<Hold>,
    tracks: Vec<Track>,
    /// The comp they are all on.
    comp: Option<Uuid>,
    /// Where the pass began, in comp seconds, and when: a clock read before
    /// the engine has taken the seek that began it is not this pass's.
    since: Option<(f64, web_time::Instant)>,
    /// How many passes have ended, so each lands only its own holds.
    passes: u64,
    /// Every point the pass being written has drawn, in the order written,
    /// so its line can be fetched a few points at a time ([`pass_since`]):
    /// the control, whether it trims, the time and the value. A point moved
    /// on by a hold is drawn again.
    log: Vec<(BridgeMixTarget, bool, f64, f64)>,
    /// Which pass the log is of. Moves on whenever the log starts again.
    log_id: u32,
}

/// How far past the time the pass has run a point may lie and still be
/// taken for this pass's, in seconds.
const CLOCK_SLACK_S: f64 = 0.5;

impl Live {
    /// The log starts again for a pass of its own.
    fn new_log(&mut self) {
        self.log.clear();
        self.log_id = self.log_id.wrapping_add(1);
    }

    /// `t` as a time of this pass: held at its start where it reads earlier,
    /// None where it reads later than the pass can have run, which is the
    /// clock from before the seek.
    fn of_pass(&self, t: f64) -> Option<f64> {
        match self.since {
            Some((start, at)) => {
                (t <= start + at.elapsed().as_secs_f64() + CLOCK_SLACK_S).then_some(t.max(start))
            }
            None => Some(t),
        }
    }
}

/// One control held in the bank.
struct Hold {
    target: BridgeMixTarget,
    /// The value heard: dB for a Volume, a send or the master, -100 to 100
    /// for a Pan. For a trim, the change from `base`.
    value: f64,
    trim: bool,
    /// What the control read when it was taken.
    base: f64,
    /// The pointer is on it.
    touching: bool,
    wait: Wait,
}

#[derive(Clone, Copy, PartialEq)]
enum Wait {
    /// Held as it is.
    No,
    /// Its value is being written by a release.
    Edit,
    /// Its value is being written by pass number n.
    Pass(u64),
    /// Written as edit number n: let go once a mix of n or later lands.
    Landing(u64),
}

/// `target`'s value in `doc` at comp second `t`, or None when its layer, or
/// the send, is not there.
pub(crate) fn value_at(doc: &Document, comp: Uuid, target: BridgeMixTarget, t: f64) -> Option<f64> {
    let c = doc.comp(comp)?;
    if let BridgeMixTarget::Master = target {
        return Some(master_at(c, t));
    }
    let l = layer_of(c, layer_in(target)?)?;
    let lt = t - l.start_offset.0.to_f64();
    match target {
        BridgeMixTarget::Volume { .. } => Some(l.volume_db.value_at(lt)),
        BridgeMixTarget::Pan { .. } => Some(l.pan.value_at(lt)),
        BridgeMixTarget::Send { bus, .. } => l
            .audio_routes
            .sends
            .iter()
            .find(|s| s.bus == bus)
            .map(|s| s.level_db.value_at(lt)),
        BridgeMixTarget::Master => None,
    }
}

/// The master fader at comp second `t`.
pub(crate) fn master_at(c: &Composition, t: f64) -> f64 {
    match &c.master_keys {
        Some(lumit_core::anim::Animation::Keyframed(keys)) => {
            lumit_core::anim::evaluate(keys, t).unwrap_or(c.master_volume_db)
        }
        Some(lumit_core::anim::Animation::Static(db)) => *db,
        _ => c.master_volume_db,
    }
}

pub(crate) fn layer_of(c: &Composition, id: Uuid) -> Option<&Layer> {
    c.layers.iter().find(|l| l.id == id)
}

/// The layer a target belongs to; None for the master.
pub(crate) fn layer_in(target: BridgeMixTarget) -> Option<Uuid> {
    match target {
        BridgeMixTarget::Volume { layer }
        | BridgeMixTarget::Pan { layer }
        | BridgeMixTarget::Send { layer, .. } => Some(layer),
        BridgeMixTarget::Master => None,
    }
}

/// How `target` writes its keys while the comp plays.
fn mode_of(doc: &Document, comp: Uuid, target: BridgeMixTarget) -> AutomationMode {
    let Some(c) = doc.comp(comp) else {
        return AutomationMode::Read;
    };
    match layer_in(target) {
        Some(layer) => layer_of(c, layer).map_or(AutomationMode::Read, |l| l.automation),
        None => c.master_automation,
    }
}

/// The plan held for `comp`, if it is the one loaded.
fn loaded_for(st: &AudioState, comp: Uuid) -> Option<&Loaded> {
    st.loaded_plan
        .as_ref()
        .filter(|_| st.loaded_comp == Some(comp))
}

/// A Volume and a Pan as a node's two gains.
fn stereo(db: f64, pan: f64) -> [f32; 2] {
    let g = db_to_gain(db);
    let p = pan_gains(pan);
    [p[0] * g, p[1] * g]
}

/// Put `target`'s held value into the bank of the plan playing, with its
/// partner (a node's Volume or Pan) read from the document at `t` where it is
/// not held itself.
fn apply(st: &AudioState, comp: Uuid, target: BridgeMixTarget, doc: &Document, t: f64) {
    let Some(loaded) = loaded_for(st, comp) else {
        return;
    };
    let bank: &LiveBank = &loaded.plan.live;
    let held = |target: BridgeMixTarget| st.live.holds.iter().find(|h| h.target == target);
    // What the control is heard at, and what its keys say.
    let level = |target: BridgeMixTarget| {
        let base = value_at(doc, comp, target, t).unwrap_or(0.0);
        match held(target) {
            Some(h) if h.trim => (base + h.value, base),
            Some(h) => (h.value, base),
            None => (base, base),
        }
    };
    let trims = |target: BridgeMixTarget| held(target).is_some_and(|h| h.trim);
    match target {
        BridgeMixTarget::Volume { layer } | BridgeMixTarget::Pan { layer } => {
            let Some(node) = loaded.info.node_of(layer) else {
                return;
            };
            let (volume, pan) = (
                BridgeMixTarget::Volume { layer },
                BridgeMixTarget::Pan { layer },
            );
            let ((db, db_base), (p, p_base)) = (level(volume), level(pan));
            let gains = stereo(db, p);
            if trims(volume) || trims(pan) {
                // Heard as a factor on the keys, so a ride already written
                // keeps its shape under the change.
                let base = stereo(db_base, p_base);
                let ratio = |g: f32, b: f32| if b > 1e-6 { g / b } else { 1.0 };
                bank.trim(
                    usize::from(node),
                    [ratio(gains[0], base[0]), ratio(gains[1], base[1])],
                );
            } else {
                bank.hold(usize::from(node), gains);
            }
        }
        BridgeMixTarget::Send { layer, bus } => {
            let Some(fader) = send_slot(loaded, layer, bus).and_then(|s| bank.send(s)) else {
                return;
            };
            hold_fader(fader, level(target).0, held(target));
        }
        BridgeMixTarget::Master => {
            if let Some(fader) = bank.master() {
                hold_fader(fader, level(target).0, held(target));
            }
        }
    }
}

/// A mono fader held at `db`, or trimmed by the held change.
fn hold_fader(fader: &lumit_audio::live::LiveFader, db: f64, held: Option<&Hold>) {
    match held {
        Some(h) if h.trim => fader.trim([db_to_gain(h.value); 2]),
        _ => fader.hold([db_to_gain(db); 2]),
    }
}

/// The bank slot of `layer`'s send to `bus`.
fn send_slot(loaded: &Loaded, layer: Uuid, bus: Uuid) -> Option<usize> {
    let node = loaded.info.node_of(layer)?;
    let sends = &loaded.info.nodes.get(usize::from(node))?.sends;
    sends
        .iter()
        .find(|(to, _)| *to == bus)
        .and_then(|(_, slot)| slot.map(usize::from))
}

/// Let `target` go in the bank: over `glide` frames back to its keys, or at
/// once with no glide. A node's Volume and Pan are one fader, so while the
/// other of the two is still held it is held alone instead.
fn let_go(
    st: &AudioState,
    comp: Uuid,
    target: BridgeMixTarget,
    doc: &Document,
    t: f64,
    glide: u32,
) {
    let Some(loaded) = loaded_for(st, comp) else {
        return;
    };
    let bank = &loaded.plan.live;
    let partner = match target {
        BridgeMixTarget::Volume { layer } => Some(BridgeMixTarget::Pan { layer }),
        BridgeMixTarget::Pan { layer } => Some(BridgeMixTarget::Volume { layer }),
        _ => None,
    };
    if let Some(partner) = partner.filter(|p| st.live.holds.iter().any(|h| h.target == *p)) {
        apply(st, comp, partner, doc, t);
        return;
    }
    let fader = match target {
        BridgeMixTarget::Volume { layer } | BridgeMixTarget::Pan { layer } => loaded
            .info
            .node_of(layer)
            .and_then(|n| bank.fader(usize::from(n))),
        BridgeMixTarget::Send { layer, bus } => {
            send_slot(loaded, layer, bus).and_then(|s| bank.send(s))
        }
        BridgeMixTarget::Master => bank.master(),
    };
    if let Some(fader) = fader {
        if glide > 0 {
            fader.release(glide);
        } else {
            fader.drop_hold();
        }
    }
}

/// The pointer is on `target` at `value`: heard at once, and written down if
/// the comp is playing in a mode that records.
pub(crate) fn touch(comp: Uuid, target: BridgeMixTarget, value: f64, doc: &Arc<Document>) {
    let mut st = lock();
    if st.live.comp != Some(comp) {
        end_pass(&mut st);
        clear(&mut st);
        st.live.comp = Some(comp);
    }
    let (t, playing) = heard(&st).unwrap_or((0.0, false));
    let playing = playing && st.loaded_comp == Some(comp);
    let mode = mode_of(doc, comp, target);
    let recording = playing && mode != AutomationMode::Read;
    let i = match st.live.holds.iter().position(|h| h.target == target) {
        Some(i) => i,
        None => {
            st.live.holds.push(Hold {
                target,
                value,
                trim: false,
                base: value,
                touching: false,
                wait: Wait::No,
            });
            st.live.holds.len() - 1
        }
    };
    let h = &mut st.live.holds[i];
    if !h.touching {
        // Taken afresh: a trim measures from where the control is now.
        h.trim = recording && mode == AutomationMode::Trim;
        h.base = value_at(doc, comp, target, t).unwrap_or(value);
    }
    h.touching = true;
    h.wait = Wait::No;
    h.value = if h.trim { value - h.base } else { value };
    let (written, trim) = (h.value, h.trim);
    apply(&st, comp, target, doc, t);
    let Some(t) = st.live.of_pass(t).filter(|_| recording) else {
        return;
    };
    let open = st
        .live
        .tracks
        .iter_mut()
        .rev()
        .find(|k| k.target == target && k.open);
    match open {
        Some(k) if k.trim == trim => {
            k.points.push((t, written));
            st.live.log.push((target, trim, t, written));
        }
        open => {
            // Changes and levels are not one line: a control taken afresh in
            // another way starts a track of its own.
            if let Some(k) = open {
                k.open = false;
            }
            st.live.tracks.push(Track {
                target,
                mode,
                trim,
                points: vec![(t, written)],
                open: true,
            });
            st.live.log.push((target, trim, t, written));
        }
    }
}

/// The pointer has left `target`. Answers where and what to write when the
/// value is to be written as the mixer writes it (stopped, or in Read): the
/// playhead in comp seconds and the value. The caller writes it and then
/// calls [`written`].
pub(crate) fn release(
    comp: Uuid,
    target: BridgeMixTarget,
    doc: &Arc<Document>,
) -> Option<(f64, f64)> {
    let mut st = lock();
    if st.live.comp != Some(comp) {
        return None;
    }
    let i = st.live.holds.iter().position(|h| h.target == target)?;
    let (t, _) = heard(&st).unwrap_or((0.0, false));
    st.live.holds[i].touching = false;
    let value = st.live.holds[i].value;
    let trim = st.live.holds[i].trim;
    let open = st
        .live
        .tracks
        .iter()
        .rposition(|k| k.target == target && k.open);
    if let Some(k) = open {
        if matches!(
            st.live.tracks[k].mode,
            AutomationMode::Latch | AutomationMode::Write
        ) {
            // Holds and goes on writing until the transport stops.
            return None;
        }
        // Touch and Trim glide back to the keys from the value held up to
        // now, and the glide's end is the last point written.
        let ms = TOUCH_RETURN_MS.load(Ordering::Relaxed);
        let rate = loaded_for(&st, comp).map_or(48_000, |l| l.rate);
        let glide = (u64::from(ms) * u64::from(rate) / 1000).min(u64::from(u32::MAX)) as u32;
        let now = st.live.of_pass(t).unwrap_or(t);
        if st.live.tracks[k].points.last().is_some_and(|p| now > p.0) {
            st.live.tracks[k].points.push((now, value));
            let line = st.live.tracks[k].trim;
            st.live.log.push((target, line, now, value));
        }
        let end = now + f64::from(ms) / 1000.0;
        let end_value = if st.live.tracks[k].trim {
            0.0
        } else {
            value_at(doc, comp, target, end).unwrap_or(value)
        };
        st.live.tracks[k].points.push((end, end_value));
        let line = st.live.tracks[k].trim;
        st.live.log.push((target, line, end, end_value));
        st.live.tracks[k].open = false;
        st.live.holds.remove(i);
        let_go(&st, comp, target, doc, t, glide);
        return None;
    }
    // Written as the mixer writes it. Held until the mix of the edit lands,
    // so there is no jump back in between. A trim is written as the level it
    // made: the keys there moved by the change.
    st.live.holds[i].wait = Wait::Edit;
    let level = if trim {
        value_at(doc, comp, target, t).unwrap_or(0.0) + value
    } else {
        value
    };
    Some((t, level))
}

/// The edit [`release`] asked for has been made (or refused): the held
/// controls waiting on it go once its mix lands. `doc` is the document after
/// it.
pub(crate) fn written(comp: Uuid, doc: Arc<Document>) {
    landing(comp, doc, Wait::Edit);
}

/// The holds waiting as `waiting` on `comp` go once the mix of `doc` lands.
fn landing(comp: Uuid, doc: Arc<Document>, waiting: Wait) {
    let seq = EDITS.fetch_add(1, Ordering::AcqRel) + 1;
    let mut st = lock();
    if st.live.comp != Some(comp) {
        // Another comp's take is live now; this one's holds went with its mix.
        return;
    }
    for h in &mut st.live.holds {
        if h.wait == waiting {
            h.wait = Wait::Landing(seq);
        }
    }
    if st.loaded_comp == Some(comp) {
        kick_prepare(&mut st, comp, doc);
    } else {
        clear(&mut st);
    }
}

/// Playback has started at comp second `at`: every control of each strip in
/// Write, and the master in Write, holds its value from here and writes it.
/// `newer` is the document playback was asked for with, where it is newer
/// than the one the mix was built from.
pub(super) fn start_pass(st: &mut AudioState, at: f64, newer: Option<&Arc<Document>>) {
    st.live.since = Some((at, web_time::Instant::now()));
    let Some(comp) = st.loaded_comp else {
        return;
    };
    let Some(loaded) = st.loaded_plan.clone() else {
        return;
    };
    let Some(doc) = newer.or(loaded.doc.as_ref()).map(|d| d.as_ref()) else {
        return;
    };
    let Some(c) = doc.comp(comp) else {
        return;
    };
    let writes =
        |l: &&Layer| l.automation == AutomationMode::Write && loaded.info.node_of(l.id).is_some();
    let mut targets: Vec<BridgeMixTarget> = Vec::new();
    for l in c.layers.iter().filter(writes) {
        targets.push(BridgeMixTarget::Volume { layer: l.id });
        targets.push(BridgeMixTarget::Pan { layer: l.id });
        targets.extend(l.audio_routes.sends.iter().map(|s| BridgeMixTarget::Send {
            layer: l.id,
            bus: s.bus,
        }));
    }
    if c.master_automation == AutomationMode::Write {
        targets.push(BridgeMixTarget::Master);
    }
    if targets.is_empty() {
        return;
    }
    if st.live.comp != Some(comp) {
        clear(st);
        st.live.comp = Some(comp);
        st.live.since = Some((at, web_time::Instant::now()));
    }
    for target in targets {
        // One already held, or still waiting for its last pass to land,
        // is left as it is.
        if st.live.holds.iter().any(|h| h.target == target) {
            continue;
        }
        let Some(value) = value_at(doc, comp, target, at) else {
            continue;
        };
        st.live.holds.push(Hold {
            target,
            value,
            trim: false,
            base: value,
            touching: false,
            wait: Wait::No,
        });
        apply(st, comp, target, doc, at);
        st.live.tracks.push(Track {
            target,
            mode: AutomationMode::Write,
            trim: false,
            points: vec![(at, value)],
            open: true,
        });
        st.live.log.push((target, false, at, value));
    }
}

/// The pass ends: every control still writing writes its held value up to
/// now, and the keys are written on a thread of their own. What was held
/// for the pass stays held until the mix with its keys lands.
pub(super) fn end_pass(st: &mut AudioState) {
    if st.live.tracks.is_empty() {
        return;
    }
    let mut tracks = std::mem::take(&mut st.live.tracks);
    let Some(comp) = st.live.comp else {
        return;
    };
    let t = heard(st).map_or(0.0, |(t, _)| t);
    let t = st.live.of_pass(t).unwrap_or(t);
    let rate = loaded_for(st, comp).map_or(48_000, |l| l.rate);
    for k in tracks.iter_mut().filter(|k| k.open) {
        let held = st.live.holds.iter().find(|h| h.target == k.target);
        if let Some(h) = held {
            if k.points.last().is_some_and(|p| t > p.0) {
                k.points.push((t, h.value));
            }
        }
        k.open = false;
    }
    st.live.passes += 1;
    let pass = Wait::Pass(st.live.passes);
    st.live.since = None;
    st.live.new_log();
    for h in &mut st.live.holds {
        if !h.touching && h.wait == Wait::No {
            h.wait = pass;
        }
    }
    super::engine::spawn(move || {
        if let Some(project) = project_of(comp) {
            write_pass(&project, comp, &tracks, rate);
            let doc = project.read().ok().map(|p| p.store.snapshot());
            if let Some(doc) = doc {
                landing(comp, doc, pass);
                return;
            }
        }
        let mut st = lock();
        if st.live.comp == Some(comp) {
            clear(&mut st);
        }
    });
}

/// The project holding `comp`.
fn project_of(comp: Uuid) -> Option<Arc<std::sync::RwLock<crate::api::state::LumitBridgeState>>> {
    let projects = crate::api::state::PROJECTS.read().ok()?;
    projects
        .values()
        .find(|p| {
            p.read()
                .is_ok_and(|p| p.store.snapshot().comp(comp).is_some())
        })
        .cloned()
}

/// A mix asked for at edit `seq` has landed: the controls waiting on an edit
/// up to it are let go, so they read their new keys.
pub(super) fn landed(st: &mut AudioState, seq: u64) {
    let Some(comp) = st.live.comp.filter(|c| st.loaded_comp == Some(*c)) else {
        return;
    };
    let Some(doc) = st.loaded_plan.as_ref().and_then(|l| l.doc.clone()) else {
        return;
    };
    let t = heard(st).map_or(0.0, |(t, _)| t);
    while let Some(i) = st
        .live
        .holds
        .iter()
        .position(|h| matches!(h.wait, Wait::Landing(n) if n <= seq))
    {
        let gone = st.live.holds.remove(i);
        let_go(st, comp, gone.target, &doc, t, 0);
    }
}

/// Let go of everything held and forget the pass.
pub(super) fn clear(st: &mut AudioState) {
    let holds = std::mem::take(&mut st.live.holds);
    st.live.tracks.clear();
    st.live.new_log();
    if let Some(comp) = st.live.comp {
        let doc = st.loaded_plan.as_ref().and_then(|l| l.doc.clone());
        if let Some(doc) = doc {
            for h in holds {
                let_go(st, comp, h.target, &doc, 0.0, 0);
            }
        }
    }
}

/// The clock reads comp second `t` while playing: every control writing
/// and held notes its value there, so a control held still writes a flat
/// line rather than a ramp toward its next move. A run of one value grows
/// by moving its last point on, so a long hold adds nothing to the pass.
pub(super) fn sample(st: &mut AudioState, t: f64) {
    if st.live.tracks.is_empty() || st.live.comp != st.loaded_comp {
        return;
    }
    let Some(t) = st.live.of_pass(t) else {
        return;
    };
    let Live {
        holds, tracks, log, ..
    } = &mut st.live;
    for k in tracks.iter_mut().filter(|k| k.open) {
        let held = holds
            .iter()
            .find(|h| h.target == k.target && h.wait == Wait::No && h.trim == k.trim);
        let Some(value) = held.map(|h| h.value) else {
            continue;
        };
        let before = k.points.len().checked_sub(2).and_then(|i| k.points.get(i));
        let flat = before.is_some_and(|p| p.1 == value);
        match k.points.last_mut() {
            Some(last) if t > last.0 && flat && last.1 == value => last.0 = t,
            Some(last) if t > last.0 => k.points.push((t, value)),
            _ => continue,
        }
        log.push((k.target, k.trim, t, value));
    }
}

/// The value each held control is heard at now, for [`super::positions`]:
/// `(target, value, trim)`, a trim's value being its change.
pub(super) fn held_now(st: &AudioState) -> impl Iterator<Item = (BridgeMixTarget, f64, bool)> + '_ {
    st.live.holds.iter().map(|h| (h.target, h.value, h.trim))
}

/// One recording control and the points it has drawn, in comp seconds, as
/// heard (a trim's changes laid on the keys they move).
pub(crate) type PassLine = (BridgeMixTarget, Vec<(f64, f64)>);

/// The point `(t, value)` of `target` as drawn: a trim's change laid on the
/// keys of `doc` at `t`.
fn drawn(
    doc: Option<&Document>,
    comp: Uuid,
    target: BridgeMixTarget,
    trim: bool,
    t: f64,
    value: f64,
) -> f64 {
    match (trim, doc) {
        (true, Some(doc)) => value_at(doc, comp, target, t).unwrap_or(0.0) + value,
        _ => value,
    }
}

/// The points written so far this pass on `comp`: each recording control
/// and the values it was heard at. Read with the document the playing mix
/// was built from, so it never waits on an edit.
pub(crate) fn pass(comp: Uuid) -> Vec<PassLine> {
    let st = lock();
    if st.live.comp != Some(comp) {
        return Vec::new();
    }
    let doc = st.loaded_plan.as_ref().and_then(|l| l.doc.clone());
    st.live
        .tracks
        .iter()
        .map(|k| {
            let points = k.points.iter();
            let points =
                points.map(|&(t, v)| (t, drawn(doc.as_deref(), comp, k.target, k.trim, t, v)));
            (k.target, points.collect())
        })
        .collect()
}

/// What the pass on `comp` has drawn since point `from` of pass `pass`:
/// which pass is being written, how many points it has drawn, and the new
/// ones by control in the order drawn. Asked about another pass than the
/// one being written, it answers from that pass's start. Copies only what
/// is new, so a line can be fetched every frame.
pub(crate) fn pass_since(comp: Uuid, pass: u32, from: usize) -> (u32, usize, Vec<PassLine>) {
    let st = lock();
    let id = st.live.log_id;
    if st.live.comp != Some(comp) {
        return (id, 0, Vec::new());
    }
    let log = &st.live.log;
    let from = if pass == id { from.min(log.len()) } else { 0 };
    let doc = st.loaded_plan.as_ref().and_then(|l| l.doc.as_deref());
    let mut lines: Vec<PassLine> = Vec::new();
    for &(target, trim, t, v) in &log[from..] {
        let point = (t, drawn(doc, comp, target, trim, t, v));
        match lines.iter_mut().find(|(at, _)| *at == target) {
            Some((_, points)) => points.push(point),
            None => lines.push((target, vec![point])),
        }
    }
    (id, log.len(), lines)
}

/// Do `work` on a thread of its own, away from the caller.
pub(crate) fn later(work: impl FnOnce() + Send + 'static) {
    super::engine::spawn(work);
}

/// The document the mix playing `comp` was built from, if it is loaded.
pub(crate) fn loaded_doc(comp: Uuid) -> Option<Arc<Document>> {
    let st = lock();
    loaded_for(&st, comp).and_then(|l| l.doc.clone())
}
