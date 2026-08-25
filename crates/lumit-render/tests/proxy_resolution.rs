//! The proxy resolution point, end to end through the two things that must
//! never disagree about it (K-501): the **decode plan**, which says which file
//! to open, and the **frame key**, which names the finished picture.
//!
//! # In plain terms
//!
//! A proxy is a small stand-in file the Viewer decodes instead of a big
//! original. The danger is not that the wrong file gets opened — that is
//! visible. It is that a frame decoded from the small file and a frame decoded
//! from the big one end up with the *same name* in the frame cache, so one is
//! handed back for the other and the Viewer shows a soft picture (or, worse, a
//! full-resolution export shows a soft one) with nothing on screen to say why.
//!
//! So every test here checks the pair together: which path the plan asked for,
//! and whether the name changed with it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{
    Composition, Document, FootageItem, Layer, LayerKind, LinearColour, MediaRef, ProjectItem,
    ProxyRef, Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::plan::{plan_comp_frame, Quality};
use lumit_render::source::{SourceProbe, SourceProbes};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

const ORIGINAL: &str = "/media/shot.mp4";
const PROXY: &str = "/media/shot_proxy.mov";

fn media(rel: &str, abs: &str) -> MediaRef {
    MediaRef {
        relative_path: rel.into(),
        absolute_path: abs.into(),
        fingerprint: None,
        extra: serde_json::Map::new(),
    }
}

fn video(fps: f64, width: u32, height: u32, frames: usize) -> SourceProbe {
    SourceProbe::Video {
        fps,
        width,
        height,
        frames,
        audio: false,
    }
}

/// One footage layer in one 64×64 comp, with the item's original probed as a
/// 1920×1080, 30 fps, 120-frame clip. The proxy — when the caller attaches one
/// — is half that size.
fn scene() -> (Arc<Document>, Uuid, Uuid) {
    let mut doc = Document::new();
    let item = Uuid::now_v7();
    doc.items.push(ProjectItem::Footage(FootageItem {
        id: item,
        name: "shot".into(),
        media: media("shot.mp4", ORIGINAL),
        extra: serde_json::Map::new(),
    }));
    let layer = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "shot".into(),
        kind: LayerKind::Footage { item },
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(4, 1).unwrap()),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup::default(),
        matte: None,
        parent: None,
        label: 0,
        volume_db: lumit_core::anim::Property::zero(),
        audio_only: false,
        retime: None,
        interpolation: lumit_core::retime::Interpolation::default(),
        parked_flow: None,
        blend: lumit_core::model::BlendMode::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        effects: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let comp = Composition {
        id: Uuid::now_v7(),
        name: "comp".into(),
        width: 64,
        height: 64,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(4, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer],
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), comp_id, item)
}

/// Attach a proxy to `item` and switch it on.
fn attach(doc: &Arc<Document>, item: Uuid) -> Arc<Document> {
    let mut d = Document::clone(doc);
    d.proxies.insert(
        item,
        ProxyRef {
            media: media("shot_proxy.mov", PROXY),
            enabled: true,
            extra: serde_json::Map::new(),
        },
    );
    Arc::new(d)
}

/// The probe pair: the original always a healthy clip, the proxy whatever the
/// case under test wants it to be.
fn probes(
    item: Uuid,
    proxy: Option<SourceProbe>,
) -> (HashMap<Uuid, SourceProbe>, HashMap<Uuid, SourceProbe>) {
    let mut originals = HashMap::new();
    originals.insert(item, video(30.0, 1920, 1080, 120));
    let mut proxies = HashMap::new();
    if let Some(p) = proxy {
        proxies.insert(item, p);
    }
    (originals, proxies)
}

/// The one decode job the scene's single footage layer plans, and the name its
/// frame is banked under: `(path, natural size, decode width, key)`.
fn plan_and_key(
    doc: &Arc<Document>,
    comp: Uuid,
    probes: &dyn SourceProbes,
    quality: Quality,
) -> (String, (u32, u32), Option<u32>, Option<u128>) {
    let composition = doc.comp(comp).unwrap();
    let jobs = plan_comp_frame(doc, composition, 1.0, quality, probes);
    assert_eq!(jobs.len(), 1, "one footage layer plans one decode");
    let job = &jobs[0];
    let key = lumit_render::cache::frame_key(doc, composition, 30, quality, probes);
    (
        job.path.to_string_lossy().into_owned(),
        (job.natural_w, job.natural_h),
        job.target_width,
        key,
    )
}

/// The whole point, in one test: with the proxy on, the plan opens the proxy
/// and the frame gets a different name; with it off, both go back to exactly
/// what they were before proxies existed.
#[test]
fn the_plan_and_the_key_switch_together() {
    let (plain, comp, item) = scene();
    let with_proxy = attach(&plain, item);
    let probes = probes(item, Some(video(30.0, 960, 540, 120)));
    let q = Quality::default();

    let (base_path, base_size, base_w, base_key) = plan_and_key(&plain, comp, &probes, q);
    assert_eq!(base_path, ORIGINAL);

    let (path, size, width, key) = plan_and_key(&with_proxy, comp, &probes, q);
    assert_eq!(path, PROXY, "the proxy is what gets opened");
    assert_ne!(
        key, base_key,
        "a proxy frame and a full-resolution frame must never share a name"
    );
    assert!(key.is_some(), "a probed proxy is perfectly nameable");

    // The layer is laid out in the ORIGINAL's pixels whichever file is read:
    // px@comp (K-419) means the original's raster, so no transform, mask or
    // effect parameter changes meaning when a proxy is switched on.
    assert_eq!(size, base_size, "geometry stays the original's");
    assert_eq!((size.0, size.1), (1920, 1080));
    assert_eq!(width, base_w, "and so does the preview-resolution tier");

    // Switching the item's own tick off is the same picture as never having
    // attached one — the same file, and the same name, so every frame banked
    // before the proxy existed is still served.
    let mut off = Document::clone(&with_proxy);
    off.proxies.get_mut(&item).unwrap().enabled = false;
    let off = Arc::new(off);
    assert_eq!(
        plan_and_key(&off, comp, &probes, q),
        (base_path.clone(), base_size, base_w, base_key)
    );

    // And so is the project-wide master switch, which overrules every item.
    let mut master_off = Document::clone(&with_proxy);
    master_off.use_proxies = false;
    let master_off = Arc::new(master_off);
    assert_eq!(
        plan_and_key(&master_off, comp, &probes, q),
        (base_path, base_size, base_w, base_key)
    );
}

/// The same proxy under two preview-resolution tiers keeps the tiers apart, so
/// proxy-ness is an axis of the name beside quality rather than instead of it.
#[test]
fn proxy_frames_still_key_per_resolution_tier() {
    let (plain, comp, item) = scene();
    let with_proxy = attach(&plain, item);
    let probes = probes(item, Some(video(30.0, 960, 540, 120)));
    let full = Quality::default();
    let half = Quality {
        divisor: 2,
        ..Quality::default()
    };

    let (_, _, _, proxy_full) = plan_and_key(&with_proxy, comp, &probes, full);
    let (_, _, _, proxy_half) = plan_and_key(&with_proxy, comp, &probes, half);
    let (_, _, _, plain_half) = plan_and_key(&plain, comp, &probes, half);
    assert_ne!(proxy_full, proxy_half, "the two tiers are two names");
    assert_ne!(proxy_half, plain_half, "and the two files are two names");
}

/// A proxy that disagrees with the original about how long the footage is, or
/// how fast it runs, is a stand-in for something else: frame 30 of it is not
/// frame 30 of the original. It is refused, and everything falls back to the
/// original — path, geometry and name alike.
#[test]
fn a_proxy_that_disagrees_about_the_footage_is_not_used() {
    let (plain, comp, item) = scene();
    let with_proxy = attach(&plain, item);
    let q = Quality::default();
    let good = probes(item, Some(video(30.0, 960, 540, 120)));
    let baseline = plan_and_key(&plain, comp, &good, q);

    let bad = [
        // One frame short: every source frame after the first cut would land
        // somewhere else.
        ("a different frame count", video(30.0, 960, 540, 119)),
        // Same count, different rate — which is a different duration.
        ("a different frame rate", video(25.0, 960, 540, 120)),
        // Present but unreadable, and simply not on disk.
        ("an unreadable file", SourceProbe::Failed),
        ("a file that is not there", SourceProbe::Missing),
        // A sound file named as a proxy has no picture to stand in with.
        ("no video stream", SourceProbe::AudioOnly),
    ];
    for (why, proxy) in bad {
        let probes = probes(item, Some(proxy));
        assert_eq!(
            plan_and_key(&with_proxy, comp, &probes, q),
            baseline,
            "a proxy with {why} falls back to the original, name included"
        );
    }

    // And a proxy nobody has probed yet: not a failure, just not usable until
    // it has been, so the original is read in the meantime.
    let unprobed = probes(item, None);
    assert_eq!(
        plan_and_key(&with_proxy, comp, &unprobed, q),
        plan_and_key(&plain, comp, &unprobed, q),
        "an unprobed proxy reads the original"
    );

    // A rate stated a hair differently by two containers is the same rate: a
    // proxy is not refused over a thousandth of a frame per second.
    let rounded = probes(item, Some(video(30.000_1, 960, 540, 120)));
    let (path, _, _, _) = plan_and_key(&with_proxy, comp, &rounded, q);
    assert_eq!(path, PROXY, "a rounding difference is not a disagreement");
}

/// A missing original slates whatever proxy is attached (docs/07 §3.3): the
/// layer *is* the original, so a lost clip must show the colour bars that lead
/// to the relink, not quietly go on playing the stand-in as though nothing had
/// happened.
#[test]
fn a_missing_original_slates_even_with_a_good_proxy() {
    let (plain, comp, item) = scene();
    let with_proxy = attach(&plain, item);
    let mut originals = HashMap::new();
    originals.insert(item, SourceProbe::Missing);
    let mut proxies = HashMap::new();
    proxies.insert(item, video(30.0, 960, 540, 120));
    let probes = (originals, proxies);

    let composition = with_proxy.comp(comp).unwrap();
    let jobs = plan_comp_frame(
        &with_proxy,
        composition,
        1.0,
        Quality::default(),
        &probes as &dyn SourceProbes,
    );
    assert_eq!(jobs.len(), 1);
    assert!(jobs[0].slate, "the slate is what a missing original draws");
    assert_eq!(jobs[0].path.to_string_lossy(), ORIGINAL);
}

/// K-031 with proxies on: an export delivers what the export was asked for, not
/// what the Viewer happens to be set to. The project is working through
/// proxies; the export's own `use_proxies` is off by default, and the snapshot
/// it renders from reads the originals at every depth.
#[test]
fn an_export_delivers_full_resolution_however_the_viewer_is_working() {
    let (plain, comp, item) = scene();
    let with_proxy = attach(&plain, item);
    assert!(with_proxy.use_proxies, "the project is working on proxies");
    let probes = probes(item, Some(video(30.0, 960, 540, 120)));
    let q = Quality::default();

    let delivery = lumit_render::export::RenderOptions::default();
    assert!(
        !delivery.use_proxies,
        "an export takes full resolution unless asked otherwise"
    );
    let snapshot = lumit_render::export::apply_render_overrides(&with_proxy, &delivery)
        .expect("a project on proxies is changed by a delivery that is not");
    assert!(!snapshot.use_proxies);

    // The delivered frame is byte-for-byte the frame a project with no proxy at
    // all delivers — same file, same name.
    assert_eq!(
        plan_and_key(&snapshot, comp, &probes, q),
        plan_and_key(&plain, comp, &probes, q),
        "export reads the originals"
    );

    // The one export that wants a proxy — a draft for review — asks for it,
    // and then gets exactly the picture the Viewer is showing.
    let draft = lumit_render::export::RenderOptions {
        use_proxies: true,
        ..lumit_render::export::RenderOptions::default()
    };
    let snapshot = lumit_render::export::apply_render_overrides(&with_proxy, &draft);
    let snapshot = snapshot.unwrap_or(Arc::clone(&with_proxy));
    assert_eq!(
        plan_and_key(&snapshot, comp, &probes, q),
        plan_and_key(&with_proxy, comp, &probes, q),
        "a draft export reads the proxies"
    );

    // The "return None when nothing would change" rule the other render
    // overrides keep. Nearly every project has the master switch on and no
    // proxies at all, and cloning a whole document per export to turn a switch
    // that governs nothing would be a cost paid on every delivery.
    assert!(
        lumit_render::export::apply_render_overrides(&plain, &delivery).is_none(),
        "a project with no proxies is not cloned for a delivery that turns them off"
    );
    let mut attached_but_off = Document::clone(&with_proxy);
    attached_but_off.proxies.get_mut(&item).unwrap().enabled = false;
    assert!(
        lumit_render::export::apply_render_overrides(&Arc::new(attached_but_off), &delivery)
            .is_none(),
        "nor one whose only proxy is switched off — it is already reading originals"
    );
    let mut already_off = Document::clone(&with_proxy);
    already_off.use_proxies = false;
    assert!(
        lumit_render::export::apply_render_overrides(&Arc::new(already_off), &delivery).is_none(),
        "nor one whose master switch is already off"
    );
}

/// `effective_media` answers about the item it was asked about, and calmly
/// answers nothing for an id that is not footage — the planner's `continue`
/// rather than a panic on a solid or a comp.
#[test]
fn only_footage_has_an_effective_source() {
    let (doc, comp, item) = scene();
    let probes = probes(item, None);
    assert!(lumit_render::source::effective_media(&doc, &probes, comp).is_none());
    assert!(lumit_render::source::effective_media(&doc, &probes, Uuid::now_v7()).is_none());
    let (media, _) = lumit_render::source::effective_media(&doc, &probes, item).unwrap();
    assert_eq!(media.absolute_path, ORIGINAL);
}
