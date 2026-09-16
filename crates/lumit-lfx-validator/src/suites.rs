//! The ten suites, each driving the bundle through a broker of its own
//! (docs/impl/lfx.md §9).
//!
//! # In plain terms
//!
//! Each of these asks one question of a plugin and answers it in a sentence.
//! Nothing here opens a module: every frame goes out over the pipe and comes
//! back through the ring, because docs/12:354-356 forbids an in-process path in
//! version 1 and because the one tool whose job is to find a crash must survive
//! the crash it finds.
//!
//! **One broker per plugin**, not one per bundle. The host spawns one broker
//! for a bundle and that is right for an editor; here it would mean a fuzz pass
//! that struck a plugin out three times took the other eleven down with it, and
//! a table with eleven rows saying "the bundle was put away" is a table that
//! says nothing about eleven plugins. The listing and the describe are read
//! once, from the first broker, because they are the bundle's own answers.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use half::f16;
use lumit_budget::Ledger;
use lumit_core::fx::Unit;
use lumit_lfx::describe::{Declaration, Declared};
use lumit_lfx::ipc::proto::{
    DeclaredTraits, DescribedPlugin, InstanceId, ParamValue, PixelDepth, RectI,
};
use lumit_lfx::ipc::ring::RingPlan;
use lumit_lfx::{
    subject, Broker, BrokerConfig, BrokerError, Ceiling, LfxRejection, Picture, ProcessJob,
    Rendered, Word,
};
use lumit_lfx_abi::{LFX_ABI_VERSION, LFX_ROI_PADDED};

use crate::baseline::{digest, Digests};
use crate::finding::Finding;
use crate::fuzz::{edges_of, Edge};
use crate::{Options, Outcome};

/// The lifecycle, in the order the header's entry table admits it.
///
/// Written out here rather than read from the header, which declares the entry
/// points and not an order over them: what a host may do when is prose in §2.1
/// and §2.6, and this is that prose pinned as a list a test can walk. The two
/// the host adds either side of the plugin's own are in it, because the ordering
/// the broker is for - the listing read with the module shut - is the first
/// thing a validator should prove is still true.
pub const ACTIONS: [&str; 6] = [
    "manifest",
    "describe",
    "create",
    "set-values",
    "process",
    "destroy",
];

/// The frame every suite works at.
///
/// Big enough that the ROI suite has somewhere to put a pixel outside a
/// declared eight pixels of padding and still have a region left to compare,
/// small enough that a twelve-effect bundle is a pass measured in seconds.
pub const FRAME: (u32, u32) = (48, 36);

/// How far past the declared reach the ROI suite puts its bright pixel.
///
/// Past the **reach**, which is the declared padding and not the region: a
/// plugin that says it reads eight pixels beyond the region it was asked for
/// may read them, so a pixel one past the region is one an honest plugin is
/// entitled to see (§4.6, and `lfx.h`'s `roi_padding_px`).
const OUTSIDE_BY: u32 = 1;

/// What two depths of the same frame may differ by before they are two
/// pictures: fp16 carries eleven bits of significand, so a relative eighth of a
/// per cent is the shape of the error and a flat absolute figure would pass a
/// dark frame and fail a bright one.
const DEPTH_TOLERANCE: f32 = 1.0 / 512.0;

/// How many value sets the threading suite drives, and how many times each.
const STRESS_VALUES: usize = 4;
/// How many times each value set is rendered in the stress pass.
const STRESS_REPEATS: usize = 3;
/// How many threads dispatch them.
const STRESS_THREADS: usize = 4;

/// How many declarations one fuzz pass walks. A panel of forty controls is a
/// panel whose first few are where the interesting numbers are, and a pass that
/// walked all of them would be a pass nobody runs.
const FUZZ_CONTROLS: usize = 4;

/// One plugin, its broker, and the frame everything is driven at.
pub(crate) struct Driver {
    /// The second process this plugin lives in.
    broker: Broker,
    /// What it said it is.
    plugin: DescribedPlugin,
    /// One value per declaration that carries one, at the plugin's own
    /// declared defaults.
    values: Vec<ParamValue>,
}

impl Driver {
    /// Start a broker over `bundle` and find `plugin` in what it describes.
    ///
    /// # Errors
    ///
    /// [`BrokerError`], which is every road out of a spawn or a describe.
    pub(crate) fn open(options: &Options, plugin: &str) -> Result<Self, BrokerError> {
        let mut broker = spawn(options)?;
        let described = broker.describe()?;
        let found = described
            .iter()
            .find(|candidate| candidate.identity.id == plugin)
            .cloned()
            .ok_or_else(|| BrokerError::NoSuchPlugin {
                id: plugin.to_owned(),
            })?;
        let values = values_at_defaults(&found);
        Ok(Self {
            broker,
            plugin: found,
            values,
        })
    }

    /// A live instance at the declared defaults.
    fn instance(&mut self) -> Result<InstanceId, BrokerError> {
        self.broker
            .create_instance(&self.plugin.identity.id, self.values.clone())
    }

    /// One frame, at the whole frame's region.
    fn render(
        &mut self,
        instance: InstanceId,
        input: &Picture,
        roi: RectI,
        neighbours: &[(f64, Picture)],
    ) -> Result<Rendered, BrokerError> {
        let job = ProcessJob {
            time: 0.0,
            bounds: RectI::of(FRAME.0, FRAME.1),
            roi,
            input,
            neighbours,
        };
        self.broker.process(instance, &job)
    }
}

/// A broker over the bundle the options name, with the options' own deadlines.
fn spawn(options: &Options) -> Result<Broker, BrokerError> {
    let mut config = BrokerConfig::new(
        options.bundle.clone(),
        options.payload.clone(),
        RingPlan::frame(FRAME.0, FRAME.1, PixelDepth::F32),
    );
    config.quirks.process_timeout = options.process_timeout;
    config.quirks.control_timeout = options.control_timeout;
    config.exe.clone_from(&options.broker_exe);
    Broker::spawn(config, &Ledger::new())
}

/// The listing, the describe and everything the bundle itself had to say.
///
/// One broker, dropped as soon as it has answered: a scan wants descriptors and
/// each plugin that is going to be driven gets a broker of its own below.
///
/// # Errors
///
/// [`BrokerError`].
pub(crate) fn survey(options: &Options) -> Result<Survey, BrokerError> {
    let mut broker = spawn(options)?;
    let listed = broker.manifest()?.len();
    let described = broker.describe()?.to_vec();
    Ok(Survey {
        listed,
        described,
        refused: broker.refused().to_vec(),
        report: broker.report().to_vec(),
    })
}

/// What the bundle answered before anything was driven.
pub(crate) struct Survey {
    /// How many plugins the listing claimed, read with the module shut.
    pub(crate) listed: usize,
    /// What the code answered.
    pub(crate) described: Vec<DescribedPlugin>,
    /// Which plugins never reached the catalogue, and why.
    pub(crate) refused: Vec<(String, LfxRejection)>,
    /// The lines that belong to the bundle rather than to any plugin in it.
    pub(crate) report: Vec<LfxRejection>,
}

// ------------------------------------------------------------ the fixtures --

/// One value per declaration that carries one, at the plugin's own declared
/// default.
///
/// Read off the describe rather than written out anywhere, so a bundle that
/// grows a control is a bundle whose new control is driven.
#[must_use]
pub fn values_at_defaults(plugin: &DescribedPlugin) -> Vec<ParamValue> {
    plugin.params.iter().filter_map(default_of).collect()
}

/// The value one declaration starts at, or `None` for a button, which carries
/// none and takes no element of the array.
fn default_of(declaration: &Declaration) -> Option<ParamValue> {
    #[allow(clippy::cast_possible_truncation)]
    match &declaration.kind {
        Declared::Float { default, .. } => Some(ParamValue::Float(*default)),
        Declared::Slider { default, .. } => Some(ParamValue::Slider(*default)),
        Declared::Int { default, .. } => Some(ParamValue::Int(*default)),
        Declared::Angle { default, .. } => Some(ParamValue::Angle(*default)),
        Declared::Bool { default } => Some(ParamValue::Bool(*default)),
        Declared::Choice { default, .. } => Some(ParamValue::Choice(*default)),
        Declared::Colour { default, .. } => Some(ParamValue::Colour([
            default[0] as f32,
            default[1] as f32,
            default[2] as f32,
            default[3] as f32,
        ])),
        Declared::Seed => Some(ParamValue::Seed(7)),
        Declared::Point2 { default, .. } => {
            Some(ParamValue::Point2([default.0 as f32, default.1 as f32]))
        }
        Declared::Point3 { default, .. } => Some(ParamValue::Point3([
            default.0 as f32,
            default.1 as f32,
            default.2 as f32,
        ])),
        Declared::Curve { default } => Some(ParamValue::Curve(default.clone())),
        Declared::File { .. } => Some(ParamValue::File(None)),
        Declared::Action => None,
    }
}

/// A picture with something in every channel and nothing flat about it, so a
/// plugin that copies the wrong row or reads the wrong channel shows.
#[must_use]
pub fn a_gradient() -> Vec<f32> {
    let (width, height) = (FRAME.0 as usize, FRAME.1 as usize);
    let mut out = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            #[allow(clippy::cast_precision_loss)]
            let (u, v) = (x as f32 / width as f32, y as f32 / height as f32);
            out.extend_from_slice(&[u, v, (u + v) * 0.5, 1.0]);
        }
    }
    out
}

/// The same picture at fp16, converted here because this is the caller's side
/// of the boundary the host never converts across.
fn as_halves(samples: &[f32]) -> Picture {
    Picture::F16(samples.iter().copied().map(f16::from_f32).collect())
}

/// The first sample at which two pictures differ, by bits, or `None`.
///
/// **By bits and not by value**, for the reason the in-process host's identity case gives: a
/// picture that went through the depth boundary and came back would compare
/// equal by value while being a different picture.
fn differs_at(left: &Picture, right: &Picture) -> Option<usize> {
    match (left, right) {
        (Picture::F32(a), Picture::F32(b)) => {
            if a.len() != b.len() {
                return Some(a.len().min(b.len()));
            }
            a.iter()
                .zip(b.iter())
                .position(|(x, y)| x.to_bits() != y.to_bits())
        }
        (Picture::F16(a), Picture::F16(b)) => {
            if a.len() != b.len() {
                return Some(a.len().min(b.len()));
            }
            a.iter()
                .zip(b.iter())
                .position(|(x, y)| x.to_bits() != y.to_bits())
        }
        _ => Some(0),
    }
}

/// Every sample of a picture as an `f32`, which is what a region comparison and
/// a finiteness check both want.
fn samples(picture: &Picture) -> Vec<f32> {
    match picture {
        Picture::F32(whole) => whole.clone(),
        Picture::F16(halves) => halves.iter().map(|half| half.to_f32()).collect(),
    }
}

/// A picture's own bytes, for the baseline digest.
fn bytes(picture: &Picture) -> Vec<u8> {
    match picture {
        Picture::F32(whole) => whole.iter().flat_map(|v| v.to_le_bytes()).collect(),
        Picture::F16(halves) => halves.iter().flat_map(|v| v.to_le_bytes()).collect(),
    }
}

/// The worst place two pictures of the same frame disagree past `tolerance`,
/// or `None` where they agree everywhere.
///
/// The tolerance is **relative to the sample**, because that is the shape of an
/// fp16 rounding error: a flat absolute figure passes a dark frame whatever it
/// does and fails a bright one that is fine. The `1.0 +` keeps a sample of
/// nought from having a tolerance of nought, which no two depths of one
/// computation can meet.
fn disagreement(left: &[f32], right: &[f32], tolerance: f32) -> Option<(f32, usize)> {
    let mut worst: Option<(f32, usize)> = None;
    for (at, (a, b)) in left.iter().zip(right.iter()).enumerate() {
        let allowed = tolerance * (1.0 + a.abs());
        let apart = (a - b).abs();
        // A NaN either side is a disagreement rather than a comparison that
        // quietly answers `false`: `apart > allowed` is false for a NaN, and a
        // plugin whose fp16 path produces one would otherwise pass this suite.
        let apart_or_worse = if apart.is_nan() { f32::INFINITY } else { apart };
        if apart_or_worse > allowed && worst.is_none_or(|(so_far, _)| apart_or_worse > so_far) {
            worst = Some((apart_or_worse, at));
        }
    }
    worst
}

/// The first sample inside `region` at which two pictures differ, by bits.
///
/// This is the ROI suite's whole claim, kept as a function of its own so that
/// what happens when a plugin *does* reach outside its declared padding can be
/// asserted without a fixture that misbehaves on purpose.
fn moved_inside(region: &[usize], calm: &[f32], lit: &[f32]) -> Option<usize> {
    region
        .iter()
        .copied()
        .find(|at| bits_at(calm, *at) != bits_at(lit, *at))
}

/// One sample's bits, or `None` where the picture is shorter than that.
///
/// By bits, for [`differs_at`]'s reason, and through the index rather than an
/// iterator because the region walks are the only places two pictures are
/// compared at somebody else's choice of samples.
fn bits_at(samples: &[f32], at: usize) -> Option<u32> {
    samples.get(at).copied().map(f32::to_bits)
}

/// The sample indices inside a region, in reading order.
fn indices_in(region: RectI) -> Vec<usize> {
    let mut out = Vec::new();
    for y in region.y0.max(0)..region.y1.min(FRAME.1 as i32) {
        for x in region.x0.max(0)..region.x1.min(FRAME.0 as i32) {
            let pixel = (y as usize) * (FRAME.0 as usize) + (x as usize);
            out.extend((pixel * 4)..(pixel * 4 + 4));
        }
    }
    out
}

/// An order that is nobody's dispatch order: the last frame first, then the
/// first, then inwards. Deterministic, because a stress pass that cannot be
/// re-run is a stress pass nobody can debug.
fn scrambled(count: usize) -> Vec<usize> {
    let mut out = Vec::with_capacity(count);
    let (mut low, mut high) = (0_usize, count);
    while low < high {
        high -= 1;
        out.push(high);
        if low < high {
            out.push(low);
            low += 1;
        }
    }
    out
}

// -------------------------------------------------------------- the suites --

/// Every struct's size prefix was one this header could read, and the ABI
/// version is this header's.
pub(crate) fn layout(plugin: &DescribedPlugin) -> Vec<Outcome> {
    let mut out = Vec::new();
    if plugin.identity.abi_version != LFX_ABI_VERSION {
        out.push(Outcome::Refused(Finding::AbiVersionNotThisHeader {
            declared: plugin.identity.abi_version,
            header: LFX_ABI_VERSION,
        }));
    }
    for line in &plugin.report {
        if matches!(line, LfxRejection::UnreadableDeclaration { .. }) {
            out.push(Outcome::Refused(Finding::SizePrefixUnreadable(
                line.clone(),
            )));
        }
    }
    if out.is_empty() {
        out.push(Outcome::Passed(format!(
            "every size prefix read, against LFX_ABI_VERSION {LFX_ABI_VERSION}"
        )));
    }
    out
}

/// What the describe said, and what it could not say.
///
/// The structural refusals are the host's own and arrive as themselves. Three
/// checks are the validator's alone, because the host is right to be laxer than
/// a vendor's own CI: a plugin with no picture family is filed under Utility
/// and loads, a version of 0.0.0 is a legal `u32` triple, and a control with no
/// unit never reaches a `Declaration` at all - the sink refuses it - so the
/// sweep over units here is the belt beside that brace.
pub(crate) fn describe(plugin: &DescribedPlugin) -> Vec<Outcome> {
    let mut out = Vec::new();
    let identity = &plugin.identity;
    if identity.major == 0 && identity.minor == 0 && identity.patch == 0 {
        out.push(Outcome::Refused(Finding::VersionIsNought));
    }
    let no_family = identity.categories.is_empty();
    if no_family {
        out.push(Outcome::Refused(Finding::NoCategoryDeclared));
    }
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for declaration in &plugin.params {
        if declaration.unit == Unit::Unset {
            out.push(Outcome::Refused(Finding::UnitUnset {
                id: declaration.id.clone(),
            }));
        }
        *seen.entry(declaration.id.as_str()).or_default() += 1;
    }
    for (id, count) in seen {
        if count > 1 {
            out.push(Outcome::Refused(Finding::DuplicateParamId {
                id: id.to_owned(),
            }));
        }
    }
    for over in past_the_ceilings(plugin) {
        out.push(Outcome::Refused(Finding::Declined(over)));
    }
    for line in &plugin.report {
        if matches!(line, LfxRejection::NoCategoryDeclared) {
            // The belt and the brace found the same missing family. One fault
            // is one row, so the host's line is only filed where the local
            // check did not already say it.
            if !no_family {
                out.push(Outcome::Refused(Finding::NoCategoryDeclared));
            }
        } else if !matches!(line, LfxRejection::UnreadableDeclaration { .. }) {
            out.push(Outcome::Noted(Finding::Declined(line.clone())));
        }
    }
    if out.is_empty() {
        out.push(Outcome::Passed(format!(
            "{} control(s), {} heading(s), nothing declined, \
             every count and string inside the header's ceilings",
            plugin.params.len(),
            plugin.groups.len()
        )));
    }
    out
}

/// Everything about one plugin that is past one of the header's ceilings, in
/// the header's own words.
///
/// **A case for each of the nine**, which is what §12's ABI header paragraph promised
/// the validator would carry. The wire already refuses these coming off the
/// pipe - `BrokerMessage::checked` reads the untrusted direction against
/// `LfxRejection::PastCeiling` before anything is kept - so nothing here fires
/// for a bundle that reached a catalogue. That is the point: this is the same
/// question asked on the side that **receives**, where there is a vendor to
/// tell, rather than on the side that refuses, where there is only a row that
/// did not appear. A limit each reader invents locally cannot be raised once a
/// vendor has shipped inside it; these read the header.
fn past_the_ceilings(plugin: &DescribedPlugin) -> Vec<LfxRejection> {
    let mut out = Vec::new();
    let mut count = |ceiling: Ceiling, subject: Word, given: usize| {
        if given as u64 > ceiling.limit() {
            out.push(LfxRejection::PastCeiling {
                ceiling,
                subject,
                given: given as u64,
            });
        }
    };
    let identity = &plugin.identity;
    count(Ceiling::StringBytes, subject::PLUGIN_ID, identity.id.len());
    count(
        Ceiling::StringBytes,
        subject::PLUGIN_NAME,
        identity.name.len(),
    );
    count(
        Ceiling::StringBytes,
        subject::PLUGIN_VENDOR,
        identity.vendor.len(),
    );
    count(
        Ceiling::Categories,
        subject::PLUGIN,
        identity.categories.len(),
    );
    count(
        Ceiling::RequiredExtensions,
        subject::PLUGIN,
        identity.required_extensions.len(),
    );
    for extension in &identity.required_extensions {
        count(
            Ceiling::StringBytes,
            subject::REQUIRED_EXTENSION,
            extension.len(),
        );
    }
    count(Ceiling::Params, subject::PANEL, plugin.params.len());
    for declaration in &plugin.params {
        count(
            Ceiling::StringBytes,
            subject::CONTROL_ID,
            declaration.id.len(),
        );
        count(
            Ceiling::StringBytes,
            subject::CONTROL_LABEL,
            declaration.label.len(),
        );
        match &declaration.kind {
            Declared::Choice {
                options,
                dividers_after,
                ..
            } => {
                count(Ceiling::Options, subject::DROPDOWN, options.len());
                count(Ceiling::Dividers, subject::DROPDOWN, dividers_after.len());
                for option in options {
                    count(Ceiling::StringBytes, subject::DROPDOWN_OPTION, option.len());
                }
            }
            Declared::File {
                filter,
                filter_name,
            } => {
                count(Ceiling::Filters, subject::FILE_CONTROL, filter.len());
                count(
                    Ceiling::StringBytes,
                    subject::FILE_CONTROL,
                    filter_name.len(),
                );
                for extension in filter {
                    count(Ceiling::StringBytes, subject::FILE_FILTER, extension.len());
                }
            }
            Declared::Curve { default } => {
                count(Ceiling::CurvePoints, subject::TONE_CURVE, default.len());
            }
            _ => {}
        }
    }
    for group in &plugin.groups {
        count(Ceiling::StringBytes, subject::HEADING_ID, group.id.len());
        count(
            Ceiling::StringBytes,
            subject::HEADING_LABEL,
            group.label.len(),
        );
    }
    out
}

/// Whether the bundle declares more effects than the header admits - the one
/// ceiling that is the bundle's rather than a plugin's, and the one the host
/// refuses outright rather than truncating.
pub(crate) fn bundle_past_its_ceiling(effects: usize) -> Option<LfxRejection> {
    let given = effects as u64;
    (given > Ceiling::EffectsPerBundle.limit()).then_some(LfxRejection::PastCeiling {
        ceiling: Ceiling::EffectsPerBundle,
        subject: subject::BUNDLE,
        given,
    })
}

/// The pinned order, driven forward and then driven backwards.
///
/// Every step of [`ACTIONS`] is taken here in turn - the manifest and the
/// describe by the survey that found this plugin, then create, set-values,
/// process and destroy on an instance of its own - and the sentence the row
/// prints is built from the steps that actually answered rather than from the
/// list. A step that never ran cannot name itself.
///
/// *ponytail:* the second half, where a destroyed instance is told its values
/// and asked for a frame, is a **host-side** check rather than an answer from
/// the bundle: `Broker::set_values` and `Broker::process` refuse a handle the
/// host's own map no longer holds before anything crosses the pipe, so what it
/// proves is that §3.5's calm refusal is where it should be. The order the
/// plugin *observed* is not readable from another process for the same reason
/// the threading suite's rendezvous is not - the fixture's call-sequence
/// exports are exports of the plugin - so §9's "observed call order" is the
/// order this suite drove, which is the order the header's entry table admits.
pub(crate) fn lifecycle(driver: &mut Driver, listed: usize) -> Vec<Outcome> {
    // The first two were taken by the survey, which is how this plugin was
    // found at all: the listing with the module shut, then the describe.
    let mut driven: Vec<&'static str> = vec![ACTIONS[0], ACTIONS[1]];
    let instance = match driver.instance() {
        Ok(instance) => instance,
        Err(why) => {
            return vec![Outcome::Refused(Finding::InstanceWouldNotCreate {
                why: why.to_string(),
            })]
        }
    };
    driven.push(ACTIONS[2]);
    if let Err(why) = driver.broker.set_values(instance, driver.values.clone()) {
        driver.broker.destroy(instance);
        return vec![Outcome::Refused(Finding::ActionWouldNotAnswer {
            action: ACTIONS[3],
            why: why.to_string(),
        })];
    }
    driven.push(ACTIONS[3]);
    let input = Picture::F32(a_gradient());
    if let Err(why) = driver.render(instance, &input, RectI::of(FRAME.0, FRAME.1), &[]) {
        driver.broker.destroy(instance);
        return vec![Outcome::Refused(Finding::ActionWouldNotAnswer {
            action: ACTIONS[4],
            why: why.to_string(),
        })];
    }
    driven.push(ACTIONS[4]);
    driver.broker.destroy(instance);
    driven.push(ACTIONS[5]);

    let mut out = Vec::new();
    if driver
        .broker
        .set_values(instance, driver.values.clone())
        .is_ok()
    {
        out.push(Outcome::Refused(Finding::ActionAnsweredOutOfOrder {
            action: ACTIONS[3],
        }));
    }
    if driver
        .render(instance, &input, RectI::of(FRAME.0, FRAME.1), &[])
        .is_ok()
    {
        out.push(Outcome::Refused(Finding::ActionAnsweredOutOfOrder {
            action: ACTIONS[4],
        }));
    }
    if out.is_empty() {
        out.push(Outcome::Passed(format!(
            "{} in order over {listed} listed effect(s), and nothing after destroy",
            driven.join(", ")
        )));
    }
    out
}

/// The whole frame at both mandatory depths, compared.
pub(crate) fn depth(driver: &mut Driver) -> Vec<Outcome> {
    let source = a_gradient();
    let whole = RectI::of(FRAME.0, FRAME.1);
    let mut rendered = Vec::new();
    for (name, input) in [
        ("fp32", Picture::F32(source.clone())),
        ("fp16", as_halves(&source)),
    ] {
        let instance = match driver.instance() {
            Ok(instance) => instance,
            Err(why) => {
                return vec![Outcome::Refused(Finding::InstanceWouldNotCreate {
                    why: why.to_string(),
                })]
            }
        };
        let frame = driver.render(instance, &input, whole, &[]);
        driver.broker.destroy(instance);
        match frame {
            Ok(frame) => rendered.push((name, frame.pixels)),
            Err(why) => {
                return vec![Outcome::Refused(Finding::DepthRefused {
                    depth: name,
                    why: why.to_string(),
                })]
            }
        }
    }
    let Some(((_, whole_depth), (_, half_depth))) = rendered.first().zip(rendered.get(1)) else {
        return vec![Outcome::Skipped("neither depth rendered".to_owned())];
    };
    for (asked, frame) in [("fp32", whole_depth), ("fp16", half_depth)] {
        let answered = depth_name(frame.depth());
        if answered != asked {
            return vec![Outcome::Refused(Finding::DepthCrossedAsTheOther {
                asked,
                answered,
            })];
        }
    }
    let (left, right) = (samples(whole_depth), samples(half_depth));
    if let Some((worst, at)) = disagreement(&left, &right, DEPTH_TOLERANCE) {
        return vec![Outcome::Refused(Finding::DepthsDisagree {
            worst,
            at,
            tolerance: DEPTH_TOLERANCE,
        })];
    }
    vec![Outcome::Passed(format!(
        "fp16 and fp32 agree over {} samples within {DEPTH_TOLERANCE}",
        left.len()
    ))]
}

/// What the table calls one of the two mandatory depths.
const fn depth_name(depth: PixelDepth) -> &'static str {
    match depth {
        PixelDepth::F32 => "fp32",
        PixelDepth::F16 => "fp16",
    }
}

/// The same frame twice on one instance, and once more on a fresh one.
pub(crate) fn determinism(driver: &mut Driver) -> Vec<Outcome> {
    let input = Picture::F32(a_gradient());
    let whole = RectI::of(FRAME.0, FRAME.1);
    let Ok(instance) = driver.instance() else {
        return vec![Outcome::Skipped("no instance to render twice".to_owned())];
    };
    let first = driver.render(instance, &input, whole, &[]);
    let second = driver.render(instance, &input, whole, &[]);
    driver.broker.destroy(instance);
    let (Ok(first), Ok(second)) = (first, second) else {
        return vec![Outcome::Skipped(
            "the frame did not render twice".to_owned(),
        )];
    };
    if let Some(at) = differs_at(&first.pixels, &second.pixels) {
        return vec![Outcome::Refused(Finding::NotDeterministic { at })];
    }

    // The same picture again out of an instance that has never seen a frame,
    // which is what "pool size changes nothing" means from the outside: the
    // pool hands a frame whichever live instance is free, and a plugin that
    // carried state between frames would answer differently here.
    let Ok(fresh) = driver.instance() else {
        return vec![Outcome::Skipped("no second instance".to_owned())];
    };
    let third = driver.render(fresh, &input, whole, &[]);
    driver.broker.destroy(fresh);
    match third {
        Ok(third) => match differs_at(&first.pixels, &third.pixels) {
            Some(at) => vec![Outcome::Refused(Finding::NotDeterministic { at })],
            None => vec![Outcome::Passed(
                "two renders on one instance and one on a fresh one are the same bits".to_owned(),
            )],
        },
        Err(_) => vec![Outcome::Skipped(
            "the fresh instance did not render".to_owned(),
        )],
    }
}

/// The declared reach, tested by moving a pixel outside it; and one call
/// against four.
///
/// The bright pixel goes [`OUTSIDE_BY`] px past the furthest sample the
/// declaration admits - the region's own edge less the declared padding -
/// rather than one pixel outside the region. The difference is the whole check:
/// a pixel one outside the region sits **inside** the padding of any plugin
/// that declared some, so putting it there refuses an honest padded plugin and
/// never reaches the dishonest one. [`outside_the_reach`] is where that
/// arithmetic lives, and it is pinned on its own below.
pub(crate) fn roi(driver: &mut Driver) -> Vec<Outcome> {
    let traits = driver.plugin.traits;
    let padding = declared_padding(traits);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let reach = padding.ceil().max(0.0) as u32;
    let margin = reach.saturating_add(OUTSIDE_BY);
    if margin.saturating_mul(2) >= FRAME.0.min(FRAME.1) {
        return vec![Outcome::Skipped(format!(
            "the declared {padding} px of padding covers the whole fixture frame"
        ))];
    }

    let whole = RectI::of(FRAME.0, FRAME.1);
    let region = RectI {
        x0: margin as i32,
        y0: margin as i32,
        x1: (FRAME.0 - margin) as i32,
        y1: (FRAME.1 - margin) as i32,
    };
    let inside = indices_in(region);

    let plain = a_gradient();
    // One pixel exactly `OUTSIDE_BY` beyond what the plugin said it reads,
    // turned all the way up. A plugin whose kernel reaches further than it
    // declared moves the region; one that told the truth does not, because the
    // pixel is past its own declared padding and not merely past the region.
    let mut bright = plain.clone();
    let corner = outside_the_reach(margin, reach);
    let distance = margin.saturating_sub(corner);
    let pixel = (corner as usize) * (FRAME.0 as usize) + (corner as usize);
    for channel in 0..4 {
        if let Some(sample) = bright.get_mut(pixel * 4 + channel) {
            *sample = 16.0;
        }
    }

    let Ok(instance) = driver.instance() else {
        return vec![Outcome::Skipped("no instance to test a region".to_owned())];
    };
    let calm = driver.render(instance, &Picture::F32(plain.clone()), region, &[]);
    let lit = driver.render(instance, &Picture::F32(bright), region, &[]);
    let one_call = driver.render(instance, &Picture::F32(plain.clone()), whole, &[]);
    let quarters: Vec<_> = tiles()
        .into_iter()
        .map(|tile| {
            (
                tile,
                driver.render(instance, &Picture::F32(plain.clone()), tile, &[]),
            )
        })
        .collect();
    driver.broker.destroy(instance);

    let mut out = Vec::new();
    match (calm, lit) {
        (Ok(calm), Ok(lit)) => {
            let (calm, lit) = (samples(&calm.pixels), samples(&lit.pixels));
            if let Some(at) = moved_inside(&inside, &calm, &lit) {
                out.push(Outcome::Refused(Finding::RoiNotHonoured {
                    padding,
                    distance,
                    at,
                }));
            }
        }
        _ => out.push(Outcome::Skipped(
            "the region did not render twice".to_owned(),
        )),
    }

    match one_call {
        Ok(one_call) => {
            let whole_frame = samples(&one_call.pixels);
            for (tile, rendered) in quarters {
                let Ok(rendered) = rendered else {
                    out.push(Outcome::Skipped("a tile did not render".to_owned()));
                    continue;
                };
                let tiled = samples(&rendered.pixels);
                if let Some(at) = indices_in(tile)
                    .into_iter()
                    .find(|at| bits_at(&whole_frame, *at) != bits_at(&tiled, *at))
                {
                    out.push(Outcome::Refused(Finding::TilesDisagree { at }));
                    break;
                }
            }
        }
        Err(_) => out.push(Outcome::Skipped(
            "the whole frame did not render".to_owned(),
        )),
    }

    if out.is_empty() {
        out.push(Outcome::Passed(format!(
            "a pixel {distance} px outside the region - {OUTSIDE_BY} px past the declared \
             {padding} px of padding - moved nothing, and four tiles make the frame \
             one call makes"
        )));
    }
    out
}

/// The four quadrants of the fixture frame.
fn tiles() -> Vec<RectI> {
    let (half_x, half_y) = ((FRAME.0 / 2) as i32, (FRAME.1 / 2) as i32);
    let (full_x, full_y) = (FRAME.0 as i32, FRAME.1 as i32);
    vec![
        RectI {
            x0: 0,
            y0: 0,
            x1: half_x,
            y1: half_y,
        },
        RectI {
            x0: half_x,
            y0: 0,
            x1: full_x,
            y1: half_y,
        },
        RectI {
            x0: 0,
            y0: half_y,
            x1: half_x,
            y1: full_y,
        },
        RectI {
            x0: half_x,
            y0: half_y,
            x1: full_x,
            y1: full_y,
        },
    ]
}

/// Where the bright pixel goes, given a region that starts at `margin` and a
/// plugin that declared a reach of `reach`.
///
/// [`OUTSIDE_BY`] px further out than the furthest input sample an honest
/// plugin may read while rendering the region's first pixel. A plugin that
/// reads exactly what it declared cannot see it; one that reads a single pixel
/// more can, which is the tile seam §4.6 calls a correctness bug.
const fn outside_the_reach(margin: u32, reach: u32) -> u32 {
    margin.saturating_sub(reach).saturating_sub(OUTSIDE_BY)
}

/// The padding a trait block declared, which is nought unless it said `PADDED`.
fn declared_padding(traits: DeclaredTraits) -> f32 {
    if traits.roi_kind == LFX_ROI_PADDED {
        traits.roi_padding_px.max(0.0)
    } else {
        0.0
    }
}

/// The declared window against what the instance actually asks for.
///
/// *ponytail:* version 1 offers no `lfx.temporal` table (§10), so what an
/// instance asks for is the declaration read back and this suite cannot yet
/// catch the plugin §4.6's trap is about - one that samples neighbours it never
/// declared. What it does catch today is the other direction, which is real: a
/// host or a broker that answered an offset outside the window would have the
/// frame key computed over a narrower window than the plugin reads. The check
/// becomes the one the note asks for on the day the extension has a table
/// behind it, with nothing here to change.
pub(crate) fn temporal(driver: &mut Driver) -> Vec<Outcome> {
    let (lo, hi) = driver.plugin.traits.temporal_window();
    let source = a_gradient();
    let input = Picture::F32(source.clone());
    let neighbours: Vec<(f64, Picture)> = (lo..=hi)
        .filter(|offset| *offset != 0)
        .map(|offset| (f64::from(offset), Picture::F32(source.clone())))
        .collect();

    let Ok(instance) = driver.instance() else {
        return vec![Outcome::Skipped("no instance to ask".to_owned())];
    };
    let frame = driver.render(instance, &input, RectI::of(FRAME.0, FRAME.1), &neighbours);
    driver.broker.destroy(instance);
    let Ok(frame) = frame else {
        return vec![Outcome::Skipped(
            "the frame with its neighbours did not render".to_owned(),
        )];
    };

    let mut out = Vec::new();
    for offset in &frame.frames_needed {
        if *offset < lo || *offset > hi {
            out.push(Outcome::Refused(Finding::AskedOutsideTheDeclaredWindow {
                offset: *offset,
                lo,
                hi,
            }));
        }
    }
    if out.is_empty() {
        out.push(Outcome::Passed(format!(
            "asked for {} frame(s), every one inside the declared window [{lo}, {hi}]",
            frame.frames_needed.len()
        )));
    }
    out
}

/// The stress scheduler: several value sets, dispatched out of order from
/// several threads, each frame held to the picture its own values make.
///
/// The reference pass comes first and is rendered serially, so what the stress
/// pass is compared against is a picture nothing else was happening during.
/// **Every frame in both passes is the same comp frame**, because the claim is
/// about values and not about time: a generator whose maths reads
/// `lfx_process.time` paints a different and entirely correct picture at each
/// frame, and a stress pass that dispatched frame 1 and compared it against
/// frame 0 would refuse it by name. What varies between dispatches is the value
/// set, which is the only thing a lease can hand to the wrong instance.
///
/// *ponytail:* §9 asks for a deliberate overlap barrier in the fixture, and
/// `lumit-lfx-testplug` has one - `LumitLfxProbeRendezvous`. It is an export of
/// the plugin, so arming it means being in the process that loaded the plugin,
/// and this tool deliberately never is. Through a broker there is nothing to
/// overlap either: one bundle has one broker and its lock is held across a
/// render (§4.4), so frames of one bundle are serialised whatever a host does
/// with its threads. So the suite **measures** the overlap rather than assuming
/// it, and files [`Finding::NothingOverlapped`] as a report line when there was
/// none - which is §11 item 12's rule kept rather than quietly broken: a stress
/// test that asserts an absence proves nothing unless it records the presence.
///
/// *ponytail:* what the overlap measures today is this suite's own `Mutex`
/// over the `&mut Broker` the render path wants, which holds from `set_values`
/// through `process` and cannot let a second frame in whatever the host does.
/// So [`Finding::NothingOverlapped`] is a line about the validator as much as
/// about the bundle, and it becomes a measurement of the *host* on the day
/// `Broker` offers a `&self` render path.
pub(crate) fn threading(driver: &mut Driver, seed: u64) -> Vec<Outcome> {
    let input = Picture::F32(a_gradient());
    let whole = RectI::of(FRAME.0, FRAME.1);
    let sets = value_sets(&driver.plugin, &driver.values, seed);

    // The reference: each value set rendered on its own, in order.
    let Ok(instance) = driver.instance() else {
        return vec![Outcome::Skipped("no instance to stress".to_owned())];
    };
    let mut reference = Vec::with_capacity(sets.len());
    for values in &sets {
        if driver.broker.set_values(instance, values.clone()).is_err() {
            driver.broker.destroy(instance);
            return vec![Outcome::Skipped("the values would not be set".to_owned())];
        }
        match driver.render(instance, &input, whole, &[]) {
            Ok(frame) => reference.push(digest(&bytes(&frame.pixels))),
            Err(_) => {
                driver.broker.destroy(instance);
                return vec![Outcome::Skipped("the reference did not render".to_owned())];
            }
        }
    }
    driver.broker.destroy(instance);

    // One live instance per thread, which is what the pool hands out.
    let mut instances = Vec::new();
    for _ in 0..STRESS_THREADS {
        match driver.instance() {
            Ok(instance) => instances.push(instance),
            Err(_) => break,
        }
    }
    if instances.is_empty() {
        return vec![Outcome::Skipped(
            "no live instance for the stress pass".to_owned(),
        )];
    }

    let order = scrambled(sets.len() * STRESS_REPEATS);
    let in_flight = Arc::new(AtomicUsize::new(0));
    let most = Arc::new(AtomicUsize::new(0));
    let wrong = Arc::new(Mutex::new(Vec::<usize>::new()));
    // *ponytail:* this lock is why nothing overlaps. `Broker::process` wants
    // `&mut self`, so the stress pass holds one broker between threads rather
    // than rendering through it from several at once.
    let shared = Mutex::new(&mut driver.broker);
    let sets = &sets;
    let reference = &reference;
    let input = &input;

    std::thread::scope(|scope| {
        for (thread, instance) in instances.iter().copied().enumerate() {
            let (in_flight, most, wrong) = (
                Arc::clone(&in_flight),
                Arc::clone(&most),
                Arc::clone(&wrong),
            );
            let (shared, order) = (&shared, &order);
            scope.spawn(move || {
                for step in order.iter().skip(thread).step_by(STRESS_THREADS.max(1)) {
                    let which = step % sets.len();
                    let Some(values) = sets.get(which) else {
                        continue;
                    };
                    let Ok(mut broker) = shared.lock() else {
                        return;
                    };
                    if broker.set_values(instance, values.clone()).is_err() {
                        continue;
                    }
                    let job = ProcessJob {
                        // The frame the reference was rendered at, so what the
                        // digests compare is the values and nothing else.
                        time: 0.0,
                        bounds: RectI::of(FRAME.0, FRAME.1),
                        roi: RectI::of(FRAME.0, FRAME.1),
                        input,
                        neighbours: &[],
                    };
                    let here = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    most.fetch_max(here, Ordering::SeqCst);
                    let frame = broker.process(instance, &job);
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    drop(broker);
                    if let Ok(frame) = frame {
                        if reference.get(which) != Some(&digest(&bytes(&frame.pixels))) {
                            if let Ok(mut wrong) = wrong.lock() {
                                wrong.push(*step);
                            }
                        }
                    }
                }
            });
        }
    });

    for instance in instances {
        driver.broker.destroy(instance);
    }

    let mut out = Vec::new();
    if let Ok(wrong) = wrong.lock() {
        for step in wrong.iter().copied() {
            out.push(Outcome::Refused(Finding::PaintedWithAnothersValues {
                step,
            }));
        }
    }
    let overlapped = most.load(Ordering::SeqCst);
    if overlapped < 2 {
        out.push(Outcome::Noted(Finding::NothingOverlapped {
            most: overlapped,
        }));
    }
    // The overlap line is a note rather than a failure, so a run with nothing
    // but that line still says what it proved: the frames came back as
    // themselves.
    if !out.iter().any(Outcome::is_refusal) {
        out.push(Outcome::Passed(format!(
            "{} frame(s) out of order over {} value set(s), each the frame its own values make",
            order.len(),
            sets.len()
        )));
    }
    out
}

/// Several distinct value sets, made by walking the first control that carries
/// one across its own declared range.
///
/// Seeded from the run's own seed, so the number the table prints reproduces
/// every pass in it rather than most of them.
fn value_sets(
    plugin: &DescribedPlugin,
    defaults: &[ParamValue],
    seed: u64,
) -> Vec<Vec<ParamValue>> {
    let mut sets = vec![defaults.to_vec()];
    let Some(declaration) = plugin.params.iter().find(|row| default_of(row).is_some()) else {
        return sets;
    };
    let edges = edges_of(declaration, seed);
    for edge in edges
        .into_iter()
        .filter(|edge| edge.declared)
        // One fewer, because the defaults are already the first set.
        .take(STRESS_VALUES.saturating_sub(1))
    {
        let mut values = defaults.to_vec();
        if let Some(slot) = values.first_mut() {
            *slot = edge.value;
        }
        sets.push(values);
    }
    sets
}

/// Every control driven to its own edges and one step past them.
pub(crate) fn fuzz(driver: &mut Driver, seed: u64) -> Vec<Outcome> {
    let input = Picture::F32(a_gradient());
    let whole = RectI::of(FRAME.0, FRAME.1);
    let Ok(instance) = driver.instance() else {
        return vec![Outcome::Skipped("no instance to fuzz".to_owned())];
    };

    let mut driven = 0_usize;
    let mut out = Vec::new();
    let plan: Vec<(usize, Declaration)> = driver
        .plugin
        .params
        .iter()
        .filter(|row| default_of(row).is_some())
        .cloned()
        .enumerate()
        .take(FUZZ_CONTROLS)
        .collect();

    for (element, declaration) in plan {
        for Edge {
            value,
            label,
            declared,
        } in edges_of(&declaration, seed)
        {
            let mut values = driver.values.clone();
            let Some(slot) = values.get_mut(element) else {
                continue;
            };
            *slot = value;
            if let Err(why) = driver.broker.set_values(instance, values) {
                out.push(Outcome::Refused(Finding::NoAnswer {
                    id: declaration.id.clone(),
                    value: label,
                    why: why.to_string(),
                }));
                continue;
            }
            driven += 1;
            match driver.render(instance, &input, whole, &[]) {
                Ok(frame) if declared => {
                    if let Some(at) = samples(&frame.pixels)
                        .iter()
                        .position(|sample| !sample.is_finite())
                    {
                        out.push(Outcome::Refused(Finding::NotFiniteInsideItsOwnBounds {
                            id: declaration.id.clone(),
                            value: label,
                            at,
                        }));
                    }
                }
                Ok(_) => {}
                Err(why) => out.push(Outcome::Refused(Finding::NoAnswer {
                    id: declaration.id.clone(),
                    value: label,
                    why: why.to_string(),
                })),
            }
        }
    }
    driver.broker.destroy(instance);

    if out.is_empty() {
        out.push(Outcome::Passed(format!(
            "{driven} edge value(s) answered, seed {seed}"
        )));
    }
    out
}

/// The frame at the declared defaults, hashed, at **both** mandatory depths.
///
/// Both, because both are the plugin's own code (§2.5) and a record of the fp32
/// path alone would pass a vendor who rewrote their fp16 maths and left the
/// version standing - which is the one release `--baseline` exists to catch.
pub(crate) fn baseline_of(driver: &mut Driver) -> Option<Digests> {
    let source = a_gradient();
    Some(Digests {
        fp32: one_digest(driver, &Picture::F32(source.clone()))?,
        fp16: one_digest(driver, &as_halves(&source))?,
    })
}

/// One frame at the declared defaults, on an instance of its own, hashed.
fn one_digest(driver: &mut Driver, input: &Picture) -> Option<String> {
    let instance = driver.instance().ok()?;
    let frame = driver.render(instance, input, RectI::of(FRAME.0, FRAME.1), &[]);
    driver.broker.destroy(instance);
    Some(digest(&bytes(&frame.ok()?.pixels)))
}

/// The plugin this driver is holding.
impl Driver {
    /// What the bundle said this plugin is.
    pub(crate) const fn plugin(&self) -> &DescribedPlugin {
        &self.plugin
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{a_gradient, differs_at, indices_in, scrambled, ACTIONS, FRAME};
    use lumit_lfx::ipc::proto::RectI;
    use lumit_lfx::Picture;

    /// The dispatch order is nobody's arrival order and covers every frame
    /// exactly once - which is what "out of order" has to mean if the stress
    /// pass is to mean anything.
    #[test]
    fn the_stress_order_is_a_permutation_and_is_not_in_order() {
        for count in [1_usize, 2, 5, 12] {
            let order = scrambled(count);
            assert_eq!(order.len(), count, "every frame is dispatched once");
            let mut sorted = order.clone();
            sorted.sort_unstable();
            assert_eq!(
                sorted,
                (0..count).collect::<Vec<_>>(),
                "and no frame twice: {order:?}"
            );
        }
        assert_ne!(
            scrambled(6),
            (0..6).collect::<Vec<_>>(),
            "the order must not be the order"
        );
        assert_eq!(scrambled(6), scrambled(6), "and it must be repeatable");
    }

    /// The fixture frame is a picture rather than a flat field, and the region
    /// walk lands inside it.
    #[test]
    fn the_fixture_frame_is_a_picture_and_a_region_is_inside_it() {
        let frame = a_gradient();
        assert_eq!(frame.len(), (FRAME.0 * FRAME.1 * 4) as usize);
        assert!(
            frame.windows(8).any(|run| run[0] != run[4]),
            "a flat frame would hide a plugin that copies the wrong column"
        );
        let region = indices_in(RectI {
            x0: 2,
            y0: 2,
            x1: 6,
            y1: 4,
        });
        assert_eq!(region.len(), 4 * 2 * 4, "four by two pixels, four channels");
        assert!(
            region.iter().all(|at| *at < frame.len()),
            "every index is inside the frame"
        );
    }

    /// Two pictures are compared by bits, so a value that survives a depth
    /// round trip is still a difference - and two depths are never the same
    /// picture.
    #[test]
    fn pictures_are_compared_by_bits() {
        let left = Picture::F32(vec![1.0, 2.0, 3.0]);
        assert_eq!(differs_at(&left, &left.clone()), None);
        assert_eq!(
            differs_at(&left, &Picture::F32(vec![1.0, 2.5, 3.0])),
            Some(1)
        );
        assert_eq!(differs_at(&left, &Picture::F32(vec![1.0, 2.0])), Some(2));
        assert_eq!(
            differs_at(&left, &Picture::F16(vec![half::f16::ONE; 3])),
            Some(0),
            "two depths are two pictures"
        );
        assert_eq!(
            differs_at(&Picture::F32(vec![0.0]), &Picture::F32(vec![-0.0])),
            Some(0),
            "nought and minus nought are the same value and different bits"
        );
    }

    /// The ROI claim bites: a sample inside the region that moved is found, and
    /// one outside it is not.
    ///
    /// The fixture has no personality that reaches past its own declared
    /// padding - none of the twelve was written to misbehave that way - so the
    /// half of the ROI suite that *finds* a dishonest reach is asserted here,
    /// against two pictures, rather than left to a bundle that cannot produce
    /// it.
    #[test]
    fn a_sample_that_moved_inside_the_region_is_found_and_one_outside_it_is_not() {
        let region = super::indices_in(RectI {
            x0: 1,
            y0: 0,
            x1: 3,
            y1: 1,
        });
        let calm = vec![0.0_f32; 16];
        let mut moved = calm.clone();
        // Sample 9 is the second channel of pixel 2, which is inside.
        if let Some(sample) = moved.get_mut(9) {
            *sample = 1.0;
        }
        assert_eq!(super::moved_inside(&region, &calm, &moved), Some(9));

        let mut outside = calm.clone();
        // Sample 0 is pixel 0, which the region starts after.
        if let Some(sample) = outside.get_mut(0) {
            *sample = 1.0;
        }
        assert_eq!(super::moved_inside(&region, &calm, &outside), None);
    }

    /// Two depths agree within a relative tolerance, and a NaN is a
    /// disagreement rather than a comparison that quietly answers false.
    #[test]
    fn two_depths_disagree_relatively_and_a_nan_always_disagrees() {
        let left = [1.0_f32, 100.0, 0.0];
        let close = [1.001_f32, 100.1, 0.0];
        assert_eq!(super::disagreement(&left, &close, 1.0 / 512.0), None);

        let apart = [1.0_f32, 105.0, 0.0];
        assert_eq!(
            super::disagreement(&left, &apart, 1.0 / 512.0).map(|(_, at)| at),
            Some(1)
        );

        let not_a_number = [1.0_f32, f32::NAN, 0.0];
        assert_eq!(
            super::disagreement(&left, &not_a_number, 1.0 / 512.0).map(|(_, at)| at),
            Some(1),
            "a plugin whose fp16 path answers NaN must not pass the depth suite"
        );
    }

    /// Every one of the header's nine ceilings has a case, and each is asked in
    /// the header's own number rather than one written out here.
    ///
    /// Built by pushing one plugin past one ceiling at a time, so a ceiling
    /// nobody checks fails this rather than passing quietly - which is the
    /// promise §12's ABI header paragraph made on the validator's behalf.
    #[test]
    fn every_ceiling_the_header_declares_has_a_case() {
        use lumit_lfx::describe::{Declaration, Declared, GroupRun};
        use lumit_lfx::ipc::proto::{DeclaredTraits, DescribedPlugin, PluginIdentity};
        use lumit_lfx::{Ceiling, LfxRejection};

        let identity = PluginIdentity {
            id: "org.example.one".to_owned(),
            name: "One".to_owned(),
            vendor: "Somebody".to_owned(),
            major: 1,
            minor: 0,
            patch: 0,
            categories: vec![1],
            abi_version: lumit_lfx_abi::LFX_ABI_VERSION,
            required_extensions: Vec::new(),
        };
        let plain = |kind: Declared| Declaration {
            id: "a".to_owned(),
            label: "A".to_owned(),
            unit: lumit_core::fx::Unit::Raw,
            flags: 0,
            kind,
        };
        let long = |ceiling: Ceiling| "x".repeat(ceiling.limit() as usize + 1);

        // One plugin per ceiling, each past exactly that one.
        let mut over_string = DescribedPlugin {
            identity: identity.clone(),
            traits: DeclaredTraits::default(),
            params: Vec::new(),
            groups: Vec::new(),
            report: Vec::new(),
        };
        over_string.identity.name = long(Ceiling::StringBytes);

        let mut over_categories = over_string.clone();
        over_categories.identity.name = "One".to_owned();
        over_categories.identity.categories = vec![1; Ceiling::Categories.limit() as usize + 1];

        let mut over_extensions = over_categories.clone();
        over_extensions.identity.categories = vec![1];
        over_extensions.identity.required_extensions =
            vec!["lfx.x".to_owned(); Ceiling::RequiredExtensions.limit() as usize + 1];

        let mut over_params = over_extensions.clone();
        over_params.identity.required_extensions = Vec::new();
        over_params.params =
            vec![plain(Declared::Bool { default: false }); Ceiling::Params.limit() as usize + 1];

        let one = |kind: Declared| {
            let mut plugin = over_params.clone();
            plugin.params = vec![plain(kind)];
            plugin
        };
        let over_options = one(Declared::Choice {
            options: vec!["o".to_owned(); Ceiling::Options.limit() as usize + 1],
            default: 0,
            dividers_after: Vec::new(),
        });
        let over_dividers = one(Declared::Choice {
            options: vec!["o".to_owned()],
            default: 0,
            dividers_after: vec![0; Ceiling::Dividers.limit() as usize + 1],
        });
        let over_filters = one(Declared::File {
            filter: vec!["png".to_owned(); Ceiling::Filters.limit() as usize + 1],
            filter_name: "Pictures".to_owned(),
        });
        let over_points = one(Declared::Curve {
            default: vec![[0.0, 0.0]; Ceiling::CurvePoints.limit() as usize + 1],
        });

        let mut over_heading = over_params.clone();
        over_heading.params = vec![plain(Declared::Bool { default: false })];
        over_heading.groups = vec![GroupRun {
            id: long(Ceiling::StringBytes),
            label: "A".to_owned(),
            hidden: false,
            first: 0,
            len: 1,
        }];

        for (ceiling, plugin) in [
            (Ceiling::StringBytes, &over_string),
            (Ceiling::Categories, &over_categories),
            (Ceiling::RequiredExtensions, &over_extensions),
            (Ceiling::Params, &over_params),
            (Ceiling::Options, &over_options),
            (Ceiling::Dividers, &over_dividers),
            (Ceiling::Filters, &over_filters),
            (Ceiling::CurvePoints, &over_points),
        ] {
            let found = super::past_the_ceilings(plugin);
            assert!(
                found.iter().any(|line| matches!(
                    line,
                    LfxRejection::PastCeiling { ceiling: named, .. } if *named == ceiling
                )),
                "{ceiling:?} has no case: {found:?}"
            );
        }
        assert!(
            !super::past_the_ceilings(&over_heading).is_empty(),
            "a heading past the string ceiling has a case too"
        );

        // The ninth is the bundle's rather than a plugin's.
        assert!(super::bundle_past_its_ceiling(1).is_none());
        assert!(matches!(
            super::bundle_past_its_ceiling(Ceiling::EffectsPerBundle.limit() as usize + 1),
            Some(LfxRejection::PastCeiling {
                ceiling: Ceiling::EffectsPerBundle,
                ..
            })
        ));

        // And an honest plugin is past none of them.
        let honest = DescribedPlugin {
            identity,
            traits: DeclaredTraits::default(),
            params: vec![plain(Declared::Bool { default: false })],
            groups: Vec::new(),
            report: Vec::new(),
        };
        assert!(super::past_the_ceilings(&honest).is_empty());
    }

    /// The pinned list is the header's entry table in the order §2.1 and §2.6
    /// admit it, each step named once.
    ///
    /// This asserts the **list**, which is all a test with no broker in it can
    /// assert: that the list is the order the lifecycle suite actually drives
    /// is read off the row that suite prints, in `lumit-lfx-broker`'s
    /// `every_suite_passes_over_the_fixtures_honest_personalities`, where the
    /// sentence is built from the steps that answered.
    #[test]
    fn the_pinned_lifecycle_is_the_headers_entry_table() {
        assert_eq!(
            ACTIONS,
            [
                "manifest",
                "describe",
                "create",
                "set-values",
                "process",
                "destroy"
            ]
        );
        let mut once = ACTIONS.to_vec();
        once.sort_unstable();
        once.dedup();
        assert_eq!(once.len(), ACTIONS.len(), "no step is named twice");
    }

    /// The frame a plugin reading `reach` px around each output pixel would
    /// paint, given one bright input pixel at (`corner`, `corner`).
    fn smeared_from(corner: u32, reach: u32) -> Vec<f32> {
        let mut out = vec![0.0_f32; (FRAME.0 * FRAME.1 * 4) as usize];
        for y in 0..FRAME.1 {
            for x in 0..FRAME.0 {
                if x.abs_diff(corner) > reach || y.abs_diff(corner) > reach {
                    continue;
                }
                let pixel = (y as usize) * (FRAME.0 as usize) + (x as usize);
                for channel in 0..4 {
                    if let Some(sample) = out.get_mut(pixel * 4 + channel) {
                        *sample = 1.0;
                    }
                }
            }
        }
        out
    }

    /// The bright pixel sits past the plugin's declared **reach**, not merely
    /// past the region: a plugin that reads all of the padding it declared must
    /// not be able to see it, and one that reads a single pixel further must.
    ///
    /// Fixture-free, because none of the twelve personalities reaches past what
    /// it declared - so a geometry that puts the pixel inside the declared
    /// padding passes every one of them while refusing the first honest padded
    /// plugin a vendor ships (§4.6).
    #[test]
    fn the_bright_pixel_is_past_the_declared_reach_rather_than_past_the_region() {
        let reach = 8_u32;
        let margin = reach + super::OUTSIDE_BY;
        let corner = super::outside_the_reach(margin, reach);
        assert!(
            margin.saturating_sub(corner) > reach,
            "the lit pixel must be further from the region than the declared reach"
        );

        let region = RectI {
            x0: margin as i32,
            y0: margin as i32,
            x1: (FRAME.0 - margin) as i32,
            y1: (FRAME.1 - margin) as i32,
        };
        let inside = indices_in(region);
        let calm = vec![0.0_f32; (FRAME.0 * FRAME.1 * 4) as usize];

        assert_eq!(
            super::moved_inside(&inside, &calm, &smeared_from(corner, reach)),
            None,
            "a plugin reading exactly the {reach} px it declared must not move the region"
        );
        assert!(
            super::moved_inside(&inside, &calm, &smeared_from(corner, reach + 1)).is_some(),
            "a plugin reading one pixel further than it declared must be caught"
        );
    }
}
