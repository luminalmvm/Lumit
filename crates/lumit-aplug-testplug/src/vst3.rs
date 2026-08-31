//! The same eight fixtures, wearing VST3's face (K-707).
//!
//! # In plain terms
//!
//! One library, two standards. The file this crate builds exports `clap_entry`
//! *and* `GetPluginFactory`, so the very same eight personalities — a gain, a
//! reporter, a plugin that dies, a plugin that hangs, a state echo, a parameter
//! echo, a latency claimer and an instrument — can be laid out as a `.clap` file
//! or as a `.vst3` bundle and the host tested against both. The declarations
//! ([`Kind::params`]) are shared, so the two faces cannot drift in what they
//! claim to be.
//!
//! VST3 wants **two objects** where CLAP wants one: a component that makes the
//! sound and a controller that owns the knobs. Both are here, one Rust type
//! each, with a class id per personality — which is exactly the split shape a
//! host gets wrong, and therefore the shape worth testing against.
//!
//! Values in a VST3 queue are **normalised** nought to one; the declarations are
//! in plain units. Both halves convert with the same two functions, so a value
//! that goes out plain and comes back plain has been through the real
//! conversion, both ways, rather than past a test's own arithmetic.

// The generated enum constants are `c_int` on Windows and `c_uint` everywhere
// else, while the struct fields and arguments they go into are always `int32`.
// So every `as i32` below is redundant on this platform and required on the
// next, and clippy can only ever see one of the two.
#![allow(clippy::unnecessary_cast)]

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void};

use vst3::Steinberg::Vst::{
    AudioBusBuffers, BusDirections_, BusInfo, BusInfo_::BusFlags_, BusTypes_, IAudioProcessor,
    IAudioProcessorTrait, IComponent, IComponentHandler, IComponentTrait, IEditController,
    IEditControllerTrait, IParamValueQueueTrait, IParameterChangesTrait, IoMode, MediaTypes_,
    ParamID, ParamValue, ParameterInfo, ParameterInfo_::ParameterFlags_, ProcessData, ProcessSetup,
    RoutingInfo, SpeakerArr, SpeakerArrangement, String128, SymbolicSampleSizes_, TChar,
};
use vst3::Steinberg::{
    int32, kInvalidArgument, kNotImplemented, kResultFalse, kResultOk, kResultTrue, tresult,
    FIDString, FUnknown, IBStream, IBStreamTrait, IPlugView, IPluginBaseTrait, IPluginFactory,
    IPluginFactory2Trait, IPluginFactoryTrait, PClassInfo, PClassInfo2, PFactoryInfo,
    PFactoryInfo_::FactoryFlags_, TBool, TUID,
};
use vst3::{uid, Class, ComRef, ComWrapper};

use crate::{
    note, param_log, Kind, ParamDecl, CRASH_ON_BLOCK_ENV, HANG_ENV, KINDS, LATENCY_DEFAULT,
    LATENCY_ENV, STATE_ECHO_DEFAULT,
};

/// What a class that makes sound calls itself. The host keeps these and skips
/// everything else in the file.
const AUDIO_MODULE_CLASS: &[u8] = b"Audio Module Class";

/// What the other half calls itself.
const CONTROLLER_CLASS: &[u8] = b"Component Controller Class";

/// The class id of one personality's processor. `LUMI` and `TEST` in the first
/// two words, so a stray id in a log is recognisably ours.
fn processor_cid(kind: Kind) -> TUID {
    uid(0x4C554D49, 0x54455354, 0x50524F43, index_of(kind))
}

/// The class id of its controller.
fn controller_cid(kind: Kind) -> TUID {
    uid(0x4C554D49, 0x54455354, 0x4354524C, index_of(kind))
}

/// Where a personality sits in the factory's order.
fn index_of(kind: Kind) -> u32 {
    KINDS
        .iter()
        .position(|other| *other == kind)
        .map_or(0, |at| at as u32)
}

/// The class at one factory index: the eight processors, then the eight
/// controllers.
fn class_at(index: i32) -> Option<(Kind, bool)> {
    let at = usize::try_from(index).ok()?;
    match KINDS.get(at) {
        Some(kind) => Some((*kind, false)),
        None => KINDS
            .get(at.checked_sub(KINDS.len())?)
            .map(|kind| (*kind, true)),
    }
}

// -------------------------------------------------------------- the factory --

/// The one object a `.vst3` bundle hands out.
struct Factory;

impl Class for Factory {
    type Interfaces = (IPluginFactory, vst3::Steinberg::IPluginFactory2);
}

impl IPluginFactoryTrait for Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host gave a writable `PFactoryInfo`.
        let info = unsafe { &mut *info };
        fill(&mut info.vendor, b"Lumit");
        fill(&mut info.url, b"https://lumitlab.com");
        fill(&mut info.email, b"");
        info.flags = FactoryFlags_::kUnicode as int32;
        kResultOk
    }

    unsafe fn countClasses(&self) -> int32 {
        i32::try_from(KINDS.len().saturating_mul(2)).unwrap_or(0)
    }

    unsafe fn getClassInfo(&self, index: int32, info: *mut PClassInfo) -> tresult {
        let Some((kind, is_controller)) = class_at(index) else {
            return kInvalidArgument;
        };
        if info.is_null() {
            return kInvalidArgument;
        }
        // The one place the reporter's factory call is noted: `getClassInfo2`
        // is asked for every class straight after this one, and noting both
        // would say the host walked the list twice.
        if !is_controller {
            note(kind, "factory");
        }
        // SAFETY: the host gave a writable `PClassInfo`.
        let info = unsafe { &mut *info };
        info.cid = if is_controller {
            controller_cid(kind)
        } else {
            processor_cid(kind)
        };
        info.cardinality = i32::MAX;
        fill(
            &mut info.category,
            if is_controller {
                CONTROLLER_CLASS
            } else {
                AUDIO_MODULE_CLASS
            },
        );
        fill(&mut info.name, trim(kind.name()));
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        cid: FIDString,
        iid: FIDString,
        obj: *mut *mut c_void,
    ) -> tresult {
        if cid.is_null() || iid.is_null() || obj.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host passes a class id and an interface id, each sixteen
        // bytes, as the SDK declares.
        let asked = unsafe { *cid.cast::<TUID>() };
        let made = KINDS.iter().find_map(|kind| {
            if asked == processor_cid(*kind) {
                note(*kind, "create");
                return ComWrapper::new(Plug::new(*kind)).to_com_ptr::<FUnknown>();
            }
            if asked == controller_cid(*kind) {
                return ComWrapper::new(Ctrl::new(*kind)).to_com_ptr::<FUnknown>();
            }
            None
        });
        let Some(made) = made else {
            return kInvalidArgument;
        };
        let pointer = made.as_ptr();
        // SAFETY: the object was just built, and `queryInterface` is its own.
        unsafe { ((*(*pointer).vtbl).queryInterface)(pointer, iid.cast::<TUID>(), obj) }
    }
}

impl IPluginFactory2Trait for Factory {
    unsafe fn getClassInfo2(&self, index: int32, info: *mut PClassInfo2) -> tresult {
        let Some((kind, is_controller)) = class_at(index) else {
            return kInvalidArgument;
        };
        if info.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host gave a writable `PClassInfo2`.
        let info = unsafe { &mut *info };
        info.cid = if is_controller {
            controller_cid(kind)
        } else {
            processor_cid(kind)
        };
        info.cardinality = i32::MAX;
        fill(
            &mut info.category,
            if is_controller {
                CONTROLLER_CLASS
            } else {
                AUDIO_MODULE_CLASS
            },
        );
        fill(&mut info.name, trim(kind.name()));
        info.classFlags = 0;
        fill(&mut info.subCategories, b"Fx|Stereo");
        fill(&mut info.vendor, b"Lumit");
        fill(&mut info.version, b"2.0.0");
        fill(&mut info.sdkVersion, b"VST 3.7.0");
        kResultOk
    }
}

// ------------------------------------------------------------ the processor --

/// One live copy of one personality's sounding half.
struct Plug {
    kind: Kind,
    /// [`Kind::Gain`]'s multiplier, in plain units.
    gain: Cell<f64>,
    /// What [`Kind::StateEcho`] was handed, byte for byte.
    state: RefCell<Vec<u8>>,
    loaded: Cell<bool>,
    block: Cell<u32>,
    activated: Cell<bool>,
}

impl Plug {
    fn new(kind: Kind) -> Self {
        let gain = kind
            .params()
            .iter()
            .find(|decl| decl.id == crate::PARAM_GAIN)
            .map_or(1.0, |decl| decl.default);
        Self {
            kind,
            gain: Cell::new(gain),
            state: RefCell::new(Vec::new()),
            loaded: Cell::new(false),
            block: Cell::new(0),
            activated: Cell::new(false),
        }
    }
}

impl Class for Plug {
    type Interfaces = (IComponent, IAudioProcessor);
}

impl IPluginBaseTrait for Plug {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        note(self.kind, "initialize");
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        note(self.kind, "terminate");
        kResultOk
    }
}

impl IComponentTrait for Plug {
    unsafe fn getControllerClassId(&self, class_id: *mut TUID) -> tresult {
        if class_id.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host gave a writable class id.
        unsafe { *class_id = controller_cid(self.kind) };
        kResultOk
    }

    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kNotImplemented
    }

    unsafe fn getBusCount(&self, media: i32, direction: i32) -> int32 {
        note(self.kind, "getBusCount");
        if media != MediaTypes_::kAudio as i32 {
            return 0;
        }
        if direction == BusDirections_::kInput as i32 {
            i32::from(self.kind.has_input())
        } else {
            1
        }
    }

    unsafe fn getBusInfo(
        &self,
        media: i32,
        direction: i32,
        index: int32,
        bus: *mut BusInfo,
    ) -> tresult {
        note(self.kind, "getBusInfo");
        let is_input = direction == BusDirections_::kInput as i32;
        if media != MediaTypes_::kAudio as i32
            || index != 0
            || bus.is_null()
            || (is_input && !self.kind.has_input())
        {
            return kInvalidArgument;
        }
        // SAFETY: the host gave a writable `BusInfo`.
        let bus = unsafe { &mut *bus };
        bus.mediaType = MediaTypes_::kAudio as i32;
        bus.direction = direction;
        bus.channelCount = 2;
        wide(&mut bus.name, if is_input { "In" } else { "Out" });
        bus.busType = BusTypes_::kMain as i32;
        bus.flags = BusFlags_::kDefaultActive as u32;
        kResultOk
    }

    unsafe fn getRoutingInfo(&self, _in: *mut RoutingInfo, _out: *mut RoutingInfo) -> tresult {
        kNotImplemented
    }

    unsafe fn activateBus(
        &self,
        _media: i32,
        _direction: i32,
        _index: int32,
        _state: TBool,
    ) -> tresult {
        note(self.kind, "activateBus");
        kResultOk
    }

    unsafe fn setActive(&self, state: TBool) -> tresult {
        note(self.kind, "setActive");
        self.activated.set(state != 0);
        if state != 0 {
            self.block.set(0);
        }
        kResultOk
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        note(self.kind, "setState");
        // SAFETY: the host gave a live stream, or null.
        let Some(stream) = (unsafe { ComRef::from_raw(state) }) else {
            return kInvalidArgument;
        };
        let bytes = read_all(&stream);
        if bytes.len() == 8 && self.kind != Kind::StateEcho {
            let mut eight = [0u8; 8];
            eight.copy_from_slice(&bytes);
            self.gain.set(f64::from_le_bytes(eight));
        }
        if let Ok(mut held) = self.state.try_borrow_mut() {
            *held = bytes;
        }
        self.loaded.set(true);
        kResultOk
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        note(self.kind, "getState");
        // SAFETY: the host gave a live stream, or null.
        let Some(stream) = (unsafe { ComRef::from_raw(state) }) else {
            return kInvalidArgument;
        };
        let bytes: Vec<u8> = match self.kind {
            Kind::StateEcho if self.loaded.get() => self
                .state
                .try_borrow()
                .map(|held| held.clone())
                .unwrap_or_default(),
            Kind::StateEcho => STATE_ECHO_DEFAULT.to_vec(),
            _ => self.gain.get().to_le_bytes().to_vec(),
        };
        write_all(&stream, &bytes)
    }
}

impl IAudioProcessorTrait for Plug {
    unsafe fn setBusArrangements(
        &self,
        inputs: *mut SpeakerArrangement,
        num_ins: int32,
        outputs: *mut SpeakerArrangement,
        num_outs: int32,
    ) -> tresult {
        note(self.kind, "setBusArrangements");
        if num_outs != 1 || outputs.is_null() {
            return kResultFalse;
        }
        // SAFETY: the host says there is one arrangement each way.
        let out = unsafe { *outputs };
        let ins_ok = if self.kind.has_input() {
            // SAFETY: as above.
            num_ins == 1 && !inputs.is_null() && unsafe { *inputs } == SpeakerArr::kStereo
        } else {
            num_ins == 0
        };
        if ins_ok && out == SpeakerArr::kStereo {
            kResultTrue
        } else {
            kResultFalse
        }
    }

    unsafe fn getBusArrangement(
        &self,
        _direction: i32,
        index: int32,
        arrangement: *mut SpeakerArrangement,
    ) -> tresult {
        if index != 0 || arrangement.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host gave a writable arrangement.
        unsafe { *arrangement = SpeakerArr::kStereo };
        kResultOk
    }

    unsafe fn canProcessSampleSize(&self, size: int32) -> tresult {
        if size == SymbolicSampleSizes_::kSample32 as i32 {
            kResultOk
        } else {
            kNotImplemented
        }
    }

    unsafe fn getLatencySamples(&self) -> u32 {
        note(self.kind, "getLatencySamples");
        if self.kind != Kind::Latency {
            return 0;
        }
        std::env::var(LATENCY_ENV)
            .ok()
            .and_then(|text| text.parse::<u32>().ok())
            .unwrap_or(LATENCY_DEFAULT)
    }

    unsafe fn setupProcessing(&self, _setup: *mut ProcessSetup) -> tresult {
        note(self.kind, "setupProcessing");
        kResultOk
    }

    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        note(self.kind, "setProcessing");
        kResultOk
    }

    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        note(self.kind, "process");
        if data.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host guarantees live process data for the call.
        let data = unsafe { &*data };
        let block = self.block.get();
        self.block.set(block.saturating_add(1));

        // SAFETY: the parameter changes, when present, are the host's and live
        // for the call.
        unsafe { self.read_changes(data, block) };

        if self.kind == Kind::Hang && std::env::var_os(HANG_ENV).is_some() {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        if self.kind == Kind::Crash {
            let at = std::env::var(CRASH_ON_BLOCK_ENV)
                .ok()
                .and_then(|text| text.parse::<u32>().ok());
            if at == Some(block) {
                std::process::abort();
            }
        }

        // SAFETY: the bus arrays and their channel pointers are the host's, live
        // for the call, and `numSamples` long.
        unsafe { self.copy_audio(data) };
        kResultOk
    }

    unsafe fn getTailSamples(&self) -> u32 {
        0
    }
}

impl Plug {
    /// Take every point out of the host's queues, newest last.
    ///
    /// # Safety
    ///
    /// `data.inputParameterChanges` must be null or a live `IParameterChanges`.
    unsafe fn read_changes(&self, data: &ProcessData, block: u32) {
        // SAFETY: the caller guarantees a live list or null.
        let Some(changes) = (unsafe { ComRef::from_raw(data.inputParameterChanges) }) else {
            return;
        };
        // SAFETY: the list's own function.
        let count = unsafe { changes.getParameterCount() };
        for index in 0..count {
            // SAFETY: `index` is below the count just reported.
            let queue = unsafe { changes.getParameterData(index) };
            // SAFETY: a non-null queue is live for the call.
            let Some(queue) = (unsafe { ComRef::from_raw(queue) }) else {
                continue;
            };
            // SAFETY: the queue's own functions.
            let (id, points) = unsafe { (queue.getParameterId(), queue.getPointCount()) };
            for point in 0..points {
                let mut offset = 0i32;
                let mut value = 0f64;
                // SAFETY: `point` is below the count just reported, and both
                // out-parameters are live locals.
                let got = unsafe { queue.getPoint(point, &mut offset, &mut value) };
                if got != kResultOk {
                    continue;
                }
                let plain = plain_of(self.kind, id, value);
                if id == crate::PARAM_GAIN {
                    self.gain.set(plain);
                }
                if self.kind == Kind::ParamEcho {
                    param_log(format!("{block}:{offset}:{id}:{plain:.6}"));
                }
            }
        }
    }

    /// Input to output, times the gain. Silence where there is no input.
    ///
    /// # Safety
    ///
    /// The bus arrays and their channel pointers must be the host's, live for
    /// the call, and `numSamples` long.
    unsafe fn copy_audio(&self, data: &ProcessData) {
        let frames = data.numSamples.max(0) as usize;
        let gain = if self.kind == Kind::Gain {
            self.gain.get() as f32
        } else {
            1.0
        };
        if data.outputs.is_null() || data.numOutputs < 1 {
            return;
        }
        // SAFETY: the host declared at least one output bus.
        let out: &AudioBusBuffers = unsafe { &*data.outputs };
        let input: Option<&AudioBusBuffers> = if data.inputs.is_null() || data.numInputs < 1 {
            None
        } else {
            // SAFETY: the host declared at least one input bus.
            Some(unsafe { &*data.inputs })
        };
        for channel in 0..out.numChannels.max(0) as usize {
            // SAFETY: the union's 32-bit arm is the one the host asked for in
            // `symbolicSampleSize`, and `channel` is below the declared count.
            let dst = unsafe { *out.__field0.channelBuffers32.add(channel) };
            if dst.is_null() {
                continue;
            }
            let src = input.and_then(|bus| {
                if channel >= bus.numChannels.max(0) as usize {
                    return None;
                }
                // SAFETY: as above.
                let plane = unsafe { *bus.__field0.channelBuffers32.add(channel) };
                (!plane.is_null()).then_some(plane)
            });
            for frame in 0..frames {
                // SAFETY: both planes are `numSamples` long by contract.
                unsafe {
                    let value = src.map_or(0.0, |plane| *plane.add(frame));
                    *dst.add(frame) = value * gain;
                }
            }
        }
    }
}

// ----------------------------------------------------------- the controller --

/// One live copy of one personality's parameter half.
struct Ctrl {
    kind: Kind,
    /// What the host last set, normalised, by parameter id.
    values: RefCell<Vec<(u32, f64)>>,
    /// What its own `setState` was handed.
    state: RefCell<Vec<u8>>,
}

impl Ctrl {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            values: RefCell::new(Vec::new()),
            state: RefCell::new(Vec::new()),
        }
    }

    fn decl(&self, id: u32) -> Option<&'static ParamDecl> {
        self.kind.params().iter().find(|decl| decl.id == id)
    }
}

impl Class for Ctrl {
    type Interfaces = (IEditController,);
}

impl IPluginBaseTrait for Ctrl {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        note(self.kind, "controller.initialize");
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        note(self.kind, "controller.terminate");
        kResultOk
    }
}

impl IEditControllerTrait for Ctrl {
    unsafe fn setComponentState(&self, _state: *mut IBStream) -> tresult {
        note(self.kind, "setComponentState");
        kResultOk
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        note(self.kind, "controller.setState");
        // SAFETY: the host gave a live stream, or null.
        let Some(stream) = (unsafe { ComRef::from_raw(state) }) else {
            return kInvalidArgument;
        };
        if let Ok(mut held) = self.state.try_borrow_mut() {
            *held = read_all(&stream);
        }
        kResultOk
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        note(self.kind, "controller.getState");
        // SAFETY: the host gave a live stream, or null.
        let Some(stream) = (unsafe { ComRef::from_raw(state) }) else {
            return kInvalidArgument;
        };
        let bytes = self
            .state
            .try_borrow()
            .map(|held| held.clone())
            .unwrap_or_default();
        write_all(&stream, &bytes)
    }

    unsafe fn getParameterCount(&self) -> int32 {
        note(self.kind, "getParameterCount");
        i32::try_from(self.kind.params().len()).unwrap_or(0)
    }

    unsafe fn getParameterInfo(&self, index: int32, info: *mut ParameterInfo) -> tresult {
        note(self.kind, "getParameterInfo");
        let Some(decl) = usize::try_from(index)
            .ok()
            .and_then(|at| self.kind.params().get(at))
        else {
            return kInvalidArgument;
        };
        if info.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host gave a writable `ParameterInfo`.
        let info = unsafe { &mut *info };
        info.id = decl.id;
        wide(&mut info.title, &String::from_utf8_lossy(trim(decl.name)));
        wide(
            &mut info.shortTitle,
            &String::from_utf8_lossy(trim(decl.name)),
        );
        wide(&mut info.units, "");
        info.stepCount = 0;
        info.defaultNormalizedValue = normalised_of(decl, decl.default);
        info.unitId = 0;
        info.flags = vst3_flags_of(decl);
        kResultOk
    }

    unsafe fn getParamStringByValue(
        &self,
        _id: ParamID,
        _value: ParamValue,
        _string: *mut String128,
    ) -> tresult {
        kNotImplemented
    }

    unsafe fn getParamValueByString(
        &self,
        _id: ParamID,
        _string: *mut TChar,
        _value: *mut ParamValue,
    ) -> tresult {
        kNotImplemented
    }

    unsafe fn normalizedParamToPlain(&self, id: ParamID, value: ParamValue) -> ParamValue {
        self.decl(id)
            .map_or(value, |decl| plain_of_decl(decl, value))
    }

    unsafe fn plainParamToNormalized(&self, id: ParamID, value: ParamValue) -> ParamValue {
        self.decl(id)
            .map_or(value, |decl| normalised_of(decl, value))
    }

    unsafe fn getParamNormalized(&self, id: ParamID) -> ParamValue {
        self.values
            .try_borrow()
            .ok()
            .and_then(|values| {
                values
                    .iter()
                    .find(|(known, _)| *known == id)
                    .map(|(_, value)| *value)
            })
            .or_else(|| self.decl(id).map(|decl| normalised_of(decl, decl.default)))
            .unwrap_or(0.0)
    }

    unsafe fn setParamNormalized(&self, id: ParamID, value: ParamValue) -> tresult {
        note(self.kind, "setParamNormalized");
        let Ok(mut values) = self.values.try_borrow_mut() else {
            return kInvalidArgument;
        };
        match values.iter_mut().find(|(known, _)| *known == id) {
            Some(slot) => slot.1 = value,
            None => values.push((id, value)),
        }
        kResultOk
    }

    unsafe fn setComponentHandler(&self, _handler: *mut IComponentHandler) -> tresult {
        kResultOk
    }

    unsafe fn createView(&self, _name: FIDString) -> *mut IPlugView {
        std::ptr::null_mut()
    }
}

// ------------------------------------------------------------- the exports --

/// The one symbol a `.vst3` bundle must export.
///
/// # Safety
///
/// Called by the host's loader; takes and returns what the SDK declares.
#[no_mangle]
pub extern "system" fn GetPluginFactory() -> *mut IPluginFactory {
    ComWrapper::new(Factory)
        .to_com_ptr::<IPluginFactory>()
        .map_or(std::ptr::null_mut(), vst3::ComPtr::into_raw)
}

/// The module's start-up on Windows.
#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "system" fn InitDll() -> bool {
    true
}

/// And its shutdown.
#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "system" fn ExitDll() -> bool {
    true
}

/// The macOS pair. Both spellings, because hosts differ on which they look for.
#[cfg(target_os = "macos")]
#[no_mangle]
pub extern "system" fn bundleEntry(_bundle: *mut c_void) -> bool {
    true
}

/// The macOS shutdown.
#[cfg(target_os = "macos")]
#[no_mangle]
pub extern "system" fn bundleExit() -> bool {
    true
}

/// The Linux start-up.
#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
#[no_mangle]
pub extern "system" fn ModuleEntry(_handle: *mut c_void) -> bool {
    true
}

/// The Linux shutdown.
#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
#[no_mangle]
pub extern "system" fn ModuleExit() -> bool {
    true
}

// ------------------------------------------------------------ the plumbing --

/// One declaration's flags, in VST3's word.
fn vst3_flags_of(decl: &ParamDecl) -> int32 {
    use clap_sys::ext::params::{
        CLAP_PARAM_IS_AUTOMATABLE, CLAP_PARAM_IS_HIDDEN, CLAP_PARAM_IS_READONLY,
    };
    let mut flags = 0;
    if decl.flags & CLAP_PARAM_IS_AUTOMATABLE != 0 {
        flags |= ParameterFlags_::kCanAutomate;
    }
    if decl.flags & CLAP_PARAM_IS_HIDDEN != 0 {
        flags |= ParameterFlags_::kIsHidden;
    }
    if decl.flags & CLAP_PARAM_IS_READONLY != 0 {
        flags |= ParameterFlags_::kIsReadOnly;
    }
    flags
}

/// A plain value as VST3 carries it.
fn normalised_of(decl: &ParamDecl, plain: f64) -> f64 {
    let span = decl.max - decl.min;
    if span <= 0.0 {
        return 0.0;
    }
    ((plain - decl.min) / span).clamp(0.0, 1.0)
}

/// And back.
fn plain_of_decl(decl: &ParamDecl, normalised: f64) -> f64 {
    decl.min + normalised.clamp(0.0, 1.0) * (decl.max - decl.min)
}

/// One personality's own conversion, for the processing half, which has the same
/// declarations and does not hold the controller.
fn plain_of(kind: Kind, id: u32, normalised: f64) -> f64 {
    kind.params()
        .iter()
        .find(|decl| decl.id == id)
        .map_or(normalised, |decl| plain_of_decl(decl, normalised))
}

/// Everything in a stream the host handed over.
fn read_all(stream: &ComRef<'_, IBStream>) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        let mut got = 0i32;
        // SAFETY: the stream's own function, with a buffer writable for its own
        // length and a live count.
        let ok = unsafe {
            stream.read(
                chunk.as_mut_ptr().cast::<c_void>(),
                i32::try_from(chunk.len()).unwrap_or(0),
                &mut got,
            )
        };
        if ok != kResultOk || got <= 0 {
            return bytes;
        }
        let Some(read) = chunk.get(..got as usize) else {
            return bytes;
        };
        bytes.extend_from_slice(read);
    }
}

/// Everything into a stream the host handed over.
fn write_all(stream: &ComRef<'_, IBStream>, bytes: &[u8]) -> tresult {
    let mut sent = 0usize;
    while sent < bytes.len() {
        let Some(rest) = bytes.get(sent..) else {
            return kInvalidArgument;
        };
        let mut wrote = 0i32;
        // SAFETY: the stream's own function, with a live slice and a live count.
        let ok = unsafe {
            stream.write(
                rest.as_ptr().cast::<c_void>().cast_mut(),
                i32::try_from(rest.len()).unwrap_or(i32::MAX),
                &mut wrote,
            )
        };
        if ok != kResultOk || wrote <= 0 {
            return kInvalidArgument;
        }
        sent = sent.saturating_add(wrote as usize);
    }
    kResultOk
}

/// A nul-terminated literal without its nul.
fn trim(text: &'static [u8]) -> &'static [u8] {
    text.strip_suffix(b"\0").unwrap_or(text)
}

/// Copy bytes into a fixed C char array, nul-terminated and never overrun.
fn fill(dst: &mut [c_char], text: &[u8]) {
    for (slot, byte) in dst.iter_mut().zip(text.iter()) {
        *slot = *byte as c_char;
    }
    let cut = text.len().min(dst.len().saturating_sub(1));
    if let Some(slot) = dst.get_mut(cut) {
        *slot = 0;
    }
}

/// The same into a VST3 `String128`.
fn wide(dst: &mut [TChar], text: &str) {
    let mut written = 0;
    for (slot, unit) in dst.iter_mut().zip(text.encode_utf16()) {
        *slot = unit;
        written += 1;
    }
    if let Some(slot) = dst.get_mut(written.min(dst.len().saturating_sub(1))) {
        *slot = 0;
    }
}
