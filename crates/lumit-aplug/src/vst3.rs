//! VST3: the second standard, onto the road the first one built.
//!
//! # In plain terms
//!
//! CLAP and VST3 are two ways of saying the same five things — what am I, what
//! knobs have I got, here is a block of sound, here is what I remember, how far
//! behind do I run. [`crate::module`] and [`crate::instance`] say them in CLAP.
//! This file says them in VST3, and hands back **exactly the same values**:
//! a [`PluginDescriptor`] and, through [`crate::abi::AnyInstance`], a plugin
//! that plays 512 frames at a time. Nothing past describe knows which standard
//! it is talking to — the effect declaration, the broker, the ring, the
//! watchdog, the switched-off list and the mix seam are all AP1–AP3's, unchanged
//! (docs/impl/audio-plugins.md §5).
//!
//! Three things about VST3 are genuinely different from CLAP, and all three are
//! handled here so that nothing above has to know:
//!
//! 1. **A plugin is two objects, not one.** The `IComponent`/`IAudioProcessor`
//!    half makes the sound; the `IEditController` half owns the parameters. They
//!    may be separate classes the host has to find and wire together, and a
//!    state load has to reach **both** of them or the knobs and the sound
//!    disagree after a reload (§9's first trap).
//! 2. **Values travel normalised.** A VST3 automation queue carries nought to
//!    one; Lumit's properties carry the plain number a person reads. The
//!    controller does the conversion, at the boundary, every time — never cached
//!    across a state load, because a plugin re-scales its ranges out of a blob.
//! 3. **There is no "flush".** CLAP can hand a plugin a parameter value outside
//!    a block; VST3's processor learns values only from the queue that rides
//!    with a block. So the values a project holds are carried here and laid into
//!    **every** block as its baseline, with that block's own automation over the
//!    top. That is what makes "properties win over stale state" true for VST3 as
//!    well, and it costs one queue point per parameter per block.
//!
//! # Where the code comes from
//!
//! The declarations are the `vst3` crate (MIT/Apache-2.0) — flat `#[repr(C)]`
//! vtables of exactly the shape Steinberg's own plain-C projection declares, and
//! the same interface ids. **No SDK source is vendored and none is needed to
//! build.** Lumit hosts under the VST3 SDK's GPLv3 branch, which is the whole
//! reason VST3 is hostable here at all (docs/impl/audio-plugins.md §1).
//!
//! # Thread role and contract
//!
//! As [`crate::instance`]: plugin-facing, full of raw pointers, no panic may
//! cross back into C, and no lock is held across a call into a plugin. The
//! objects this host implements for the plugin to call — the stream, the
//! parameter queues, the component handler, the host context — take no lock and
//! answer from a `Cell` or a `RefCell` that is never borrowed twice.

// The generated enum constants are `c_int` on Windows and `c_uint` everywhere
// else, while the struct fields and arguments they go into are always `int32`.
// So every `as i32` below is redundant on this platform and required on the
// next, and clippy can only ever see one of the two.
#![allow(clippy::unnecessary_cast)]

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use vst3::Steinberg::Vst::{
    BusDirections_, BusInfo, BusTypes_, IAudioProcessor, IAudioProcessorTrait, IComponent,
    IComponentHandler, IComponentHandlerTrait, IComponentTrait, IEditController,
    IEditControllerTrait, IHostApplication, IHostApplicationTrait, IParamValueQueue,
    IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, MediaTypes_, ParamID,
    ParamValue, ParameterInfo, ParameterInfo_::ParameterFlags_, ProcessData, ProcessModes_,
    ProcessSetup, SpeakerArr, String128, SymbolicSampleSizes_, TChar,
};
use vst3::Steinberg::{
    kInvalidArgument, kNotImplemented, kResultOk, kResultTrue, tresult, FUnknown, IBStream,
    IBStreamTrait, IPluginBaseTrait, IPluginFactory, IPluginFactory2, IPluginFactory2Trait,
    IPluginFactoryTrait, PClassInfo, PClassInfo2, TUID,
};
use vst3::{uid, Class, ComPtr, ComWrapper};

use crate::describe::{ParamDescription, PortInfo, Ports};
use crate::instance::HostError;
use crate::module::{ModuleEntry, ModuleError};
use crate::process::{Block, ParamEvent, BLOCK_FRAMES, CHANNELS, SAMPLE_RATE};

/// The class category a plugin that makes sound declares. VST3 spells the
/// category as prose in a fixed-width field; a module also holds controller
/// classes and other furniture, and only this one is an effect.
const AUDIO_MODULE_CLASS: &str = "Audio Module Class";

/// The bundle extension.
pub const BUNDLE_EXTENSION: &str = "vst3";

// ------------------------------------------------------------- the bundle --

/// The binary inside a `.vst3` bundle, or the bundle itself where it is a plain
/// library.
///
/// A VST3 bundle is a folder with the plugin's real library at
/// `Contents/<architecture>/<name>.vst3`, and the architecture folder is named
/// by the platform rather than by anything readable off the file. The legacy
/// shape — a plain DLL called `Something.vst3` — is still installed by plenty of
/// vendors and is accepted as it is.
#[must_use]
pub fn payload(bundle: &Path) -> Option<PathBuf> {
    if bundle.is_file() {
        return Some(bundle.to_path_buf());
    }
    let contents = bundle.join("Contents");
    for folder in architecture_folders() {
        let dir = contents.join(folder);
        if let Some(found) = first_file(&dir) {
            return Some(found);
        }
    }
    // ponytail: an unknown architecture folder falls through to the first file
    // under Contents, sorted. A bundle that ships two architectures we cannot
    // name would be a coin toss, which is why the named folders are tried first.
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&contents)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    folders.iter().find_map(|dir| first_file(dir))
}

/// The architecture folder names this platform's plugins are installed under,
/// most likely first.
fn architecture_folders() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &["x86_64-win", "arm64-win", "x86-win"]
    }
    #[cfg(target_os = "macos")]
    {
        &["MacOS"]
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        &["x86_64-linux", "aarch64-linux"]
    }
}

/// The first file in a directory, by name. `None` for a directory that is not
/// there, which is the ordinary answer for an architecture this bundle does not
/// ship.
fn first_file(dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    files.into_iter().next()
}

// ------------------------------------------------------------- the module --

/// One loaded `.vst3` bundle.
///
/// The mirror of [`crate::module::Module`], and **not `Sync`** for the same
/// reason: one instance is driven by one thread, and parallelism is across
/// layers (docs/impl/audio-plugins.md §5).
pub struct Vst3Module {
    /// Released before the module is shut down and the library unloaded, which
    /// is why it is an `Option` rather than a plain pointer: `Drop` has to take
    /// it out in order.
    factory: Option<ComPtr<IPluginFactory>>,
    /// The module's own shutdown, if it exports one.
    exit: Option<unsafe extern "system" fn() -> bool>,
    /// Kept so the library outlives every pointer taken out of it.
    library: libloading::Library,
    /// The **bundle**, not the binary: it is the key the broker table is keyed
    /// by, and the thing a person would point at.
    path: PathBuf,
    entries: Vec<ModuleEntry>,
}

// SAFETY: as `crate::module::Module` — the module owns the library, and the
// factory pointer is into it. Moving a module to the thread that will drive it
// moves the whole plugin.
unsafe impl Send for Vst3Module {}
// SAFETY: as above; the one shared method that calls into the plugin,
// `create`, carries VST3's main-thread rule in its contract.
unsafe impl Sync for Vst3Module {}

impl Vst3Module {
    /// Open a bundle, start it, and read its class list.
    ///
    /// # Errors
    ///
    /// Every way a third party's bundle can disappoint us, each a report line
    /// rather than a dialogue (docs/12 §2.6).
    pub fn open(bundle: &Path) -> Result<Self, ModuleError> {
        let binary = payload(bundle).ok_or(ModuleError::BadPath)?;
        // SAFETY: loading a library runs its initialisers, which is inherently
        // third-party code. The isolation that makes it survivable is the
        // broker process, not a Rust keyword.
        let library = unsafe { libloading::Library::new(&binary) }
            .map_err(|error| ModuleError::NotLoaded(error.to_string()))?;

        // The module's own start-up, called once before anything else, exactly
        // as the SDK requires. A bundle that exports none is not broken — some
        // ship without one — so a missing symbol is not a refusal.
        for name in entry_symbols() {
            // SAFETY: the symbol, where present, is the module's own entry
            // point, whose signature the SDK fixes.
            let found =
                unsafe { library.get::<unsafe extern "system" fn(*mut c_void) -> bool>(name) };
            if let Ok(entry) = found {
                // SAFETY: the module's own function, called once, first.
                // ponytail: macOS's `bundleEntry` is handed null rather than
                // this bundle's `CFBundleRef`; nothing in the SDK's own hosting
                // module dereferences it, and building a CFBundle here would
                // pull Core Foundation into a Windows-first crate.
                if !unsafe { entry(std::ptr::null_mut()) } {
                    return Err(ModuleError::InitRefused);
                }
                break;
            }
        }

        // SAFETY: the one symbol every VST3 module must export, with the
        // signature the SDK fixes.
        let get_factory = unsafe {
            library.get::<unsafe extern "system" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0")
        }
        .map_err(|_| ModuleError::NoEntry)?;
        // SAFETY: the module's own function, called after its entry point.
        let raw = unsafe { get_factory() };
        // SAFETY: the factory comes back with one reference already taken,
        // which is what `from_raw` adopts.
        let factory = unsafe { ComPtr::from_raw(raw) }.ok_or(ModuleError::NoFactory)?;

        let exit = exit_symbols().iter().find_map(|name| {
            // SAFETY: the symbol, where present, is the module's own
            // shutdown, whose signature the SDK fixes.
            unsafe { library.get::<unsafe extern "system" fn() -> bool>(name) }
                .ok()
                .map(|symbol| *symbol)
        });

        let mut module = Self {
            factory: Some(factory),
            exit,
            library,
            path: bundle.to_path_buf(),
            entries: Vec::new(),
        };
        module.entries = module.read_entries();
        Ok(module)
    }

    /// The plugins this bundle declares, in the factory's own order.
    #[must_use]
    pub fn entries(&self) -> &[ModuleEntry] {
        &self.entries
    }

    /// The bundle this module came out of.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Walk the factory's class list once, keeping the classes that make sound.
    fn read_entries(&self) -> Vec<ModuleEntry> {
        let Some(factory) = self.factory.as_ref() else {
            return Vec::new();
        };
        let two = factory.cast::<IPluginFactory2>();
        // SAFETY: the factory's own function, on a live factory.
        let count = unsafe { factory.countClasses() };
        let mut entries = Vec::new();
        for index in 0..count {
            let mut info = blank_class_info();
            // SAFETY: `index` is below the count just reported, and `info` is a
            // writable `PClassInfo`.
            if unsafe { factory.getClassInfo(index, &mut info) } != kResultOk {
                continue;
            }
            if text_of(&info.category) != AUDIO_MODULE_CLASS {
                continue;
            }
            let (vendor, version, features) = match two.as_ref() {
                Some(two) => {
                    let mut wide = blank_class_info2();
                    // SAFETY: as `getClassInfo`, with a writable `PClassInfo2`.
                    if unsafe { two.getClassInfo2(index, &mut wide) } == kResultOk {
                        (
                            text_of(&wide.vendor),
                            text_of(&wide.version),
                            text_of(&wide.subCategories)
                                .split('|')
                                .filter(|word| !word.is_empty())
                                .map(str::to_owned)
                                .collect(),
                        )
                    } else {
                        (String::new(), String::new(), Vec::new())
                    }
                }
                None => (String::new(), String::new(), Vec::new()),
            };
            entries.push(ModuleEntry {
                id: hex_of(&info.cid),
                name: text_of(&info.name),
                vendor,
                version,
                features,
            });
        }
        entries
    }

    /// The class id one of this module's plugin identifiers names.
    fn cid_of(&self, plugin_id: &str) -> Option<TUID> {
        self.entries
            .iter()
            .find(|entry| entry.id == plugin_id)
            .and_then(|_| tuid_from_hex(plugin_id))
    }

    /// Make one object, by class id and interface.
    fn create<I: vst3::Interface>(&self, cid: &TUID) -> Option<ComPtr<I>> {
        let factory = self.factory.as_ref()?;
        let mut obj: *mut c_void = std::ptr::null_mut();
        let iid = I::IID;
        // SAFETY: the factory's own function, with a class id and an interface
        // id that both live for the call and a writable out pointer.
        let result = unsafe {
            factory.createInstance(
                cid.as_ptr().cast(),
                iid.as_ptr().cast(),
                std::ptr::addr_of_mut!(obj),
            )
        };
        if result != kResultOk || obj.is_null() {
            return None;
        }
        // SAFETY: `createInstance` answers with one reference already taken.
        unsafe { ComPtr::from_raw(obj.cast::<I>()) }
    }
}

impl Drop for Vst3Module {
    fn drop(&mut self) {
        // The factory goes before the module is shut down, and the module before
        // the library is unloaded. Any other order unloads a library somebody is
        // still inside.
        drop(self.factory.take());
        if let Some(exit) = self.exit {
            // SAFETY: the module's own function, paired with the entry point
            // that succeeded in `open`, after every object from it is released
            // — each instance holds an `Arc` of this module.
            unsafe { exit() };
        }
        let _ = &self.library;
    }
}

/// The module start-up symbols, by platform.
fn entry_symbols() -> &'static [&'static [u8]] {
    #[cfg(target_os = "windows")]
    {
        &[b"InitDll\0"]
    }
    #[cfg(target_os = "macos")]
    {
        &[b"bundleEntry\0", b"BundleEntry\0"]
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        &[b"ModuleEntry\0"]
    }
}

/// The module shutdown symbols, by platform.
fn exit_symbols() -> &'static [&'static [u8]] {
    #[cfg(target_os = "windows")]
    {
        &[b"ExitDll\0"]
    }
    #[cfg(target_os = "macos")]
    {
        &[b"bundleExit\0", b"BundleExit\0"]
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        &[b"ModuleExit\0"]
    }
}

// ----------------------------------------------------------- the instance --

/// One live VST3 plugin: its two halves, and everything they were brought up
/// with.
pub struct Vst3Instance {
    /// Kept so the module outlives the plugin.
    module: Arc<Vst3Module>,
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    /// The parameter half. `None` for a plugin that offers no controller at
    /// all, which is a plugin with no rows — not a failure.
    controller: Option<ComPtr<IEditController>>,
    /// Whether that half is an **object of its own** rather than the component
    /// wearing a second face. It decides one thing, and it decides it twice: a
    /// single-object plugin is initialised once and terminated once.
    split: bool,
    /// The handler the controller was given, kept alive for as long as it holds
    /// the pointer.
    handler: ComWrapper<Handler>,
    /// The `IHostApplication` both halves were initialised with, likewise.
    context: ComWrapper<HostContext>,
    /// The queues one block's parameter values are laid into, allocated once.
    changes: ComWrapper<Changes>,
    /// The project's values, in **plain** units, by the plugin's own parameter
    /// id. Laid into every block as its baseline: VST3's processor has no door
    /// for a value outside a block, so the host carries them (see this module's
    /// note).
    initial: Vec<(u32, f64)>,
    offline: bool,
    activated: bool,
    processing: bool,
}

// SAFETY: an instance owns everything it points at — the module keeps the
// library loaded, the boxed host objects outlive the plugin — so moving it to
// the thread that will process it moves the whole plugin. Deliberately not
// `Sync`: one instance is processed single-threaded.
unsafe impl Send for Vst3Instance {}

impl Vst3Instance {
    /// Create one plugin from a module and initialise both its halves.
    ///
    /// # Errors
    ///
    /// [`HostError::NoSuchPlugin`] for a class this bundle does not declare,
    /// [`HostError::NotCreated`] when the factory refuses, and
    /// [`HostError::InitRefused`] when the component will not start.
    pub fn create(module: Arc<Vst3Module>, plugin_id: &str) -> Result<Self, HostError> {
        let cid = module
            .cid_of(plugin_id)
            .ok_or_else(|| HostError::NoSuchPlugin(plugin_id.to_owned()))?;
        let component: ComPtr<IComponent> = module.create(&cid).ok_or(HostError::NotCreated)?;

        let context = ComWrapper::new(HostContext);
        let as_unknown = context
            .to_com_ptr::<FUnknown>()
            .ok_or(HostError::NotCreated)?;
        // SAFETY: the plugin's own function, called once, before anything else,
        // with a host context that outlives the plugin.
        if unsafe { component.initialize(as_unknown.as_ptr()) } != kResultOk {
            return Err(HostError::InitRefused);
        }

        let processor = component
            .cast::<IAudioProcessor>()
            .ok_or(HostError::NoExtension("IAudioProcessor"))?;

        // The controller is either a class of its own — the split the SDK
        // encourages — or the component wearing a second face. Both are ordinary
        // and the host must handle both. Which one it turned out to be is
        // remembered, because a single-object plugin must be initialised and
        // terminated **once**, not twice: naming the class id is a plugin saying
        // where its controller *is*, not a promise that the factory will build
        // one.
        let mut controller_cid: TUID = [0; 16];
        // SAFETY: the plugin's own function, with a writable class id.
        let names_one =
            unsafe { component.getControllerClassId(std::ptr::addr_of_mut!(controller_cid)) }
                == kResultOk;
        let separate: Option<ComPtr<IEditController>> = if names_one {
            module.create(&controller_cid)
        } else {
            None
        };
        let split = separate.is_some();
        let controller = match separate {
            Some(controller) => {
                // SAFETY: the controller's own function, called once, first.
                unsafe { controller.initialize(as_unknown.as_ptr()) };
                Some(controller)
            }
            None => component.cast::<IEditController>(),
        };

        let handler = ComWrapper::new(Handler::default());
        if let Some(controller) = controller.as_ref() {
            if let Some(pointer) = handler.to_com_ptr::<IComponentHandler>() {
                // SAFETY: the controller's own function, with a handler that
                // outlives it.
                unsafe { controller.setComponentHandler(pointer.as_ptr()) };
            }
        }

        Ok(Self {
            module,
            component,
            processor,
            controller,
            split,
            handler,
            context,
            changes: ComWrapper::new(Changes::default()),
            initial: Vec::new(),
            offline: false,
            activated: false,
            processing: false,
        })
    }

    /// The module this plugin came out of.
    #[must_use]
    pub fn module(&self) -> &Arc<Vst3Module> {
        &self.module
    }

    /// Whether the plugin has asked to be brought up again — VST3's way of
    /// saying "my latency changed", the same signal CLAP sends with
    /// `request_restart` (docs/impl/audio-plugins.md §4).
    #[must_use]
    pub fn wants_restart(&self) -> bool {
        self.handler.restart.load(Ordering::Relaxed)
    }

    /// The plugin's audio buses, in Lumit's own words.
    #[must_use]
    pub fn ports(&self) -> Ports {
        Ports {
            inputs: self.buses(true),
            outputs: self.buses(false),
        }
    }

    /// How many buses one direction has. Asked on its own where the count is
    /// all that is wanted — activating them — so that switching a bus off does
    /// not also re-read every bus's name.
    fn bus_count(&self, is_input: bool) -> i32 {
        let direction = if is_input {
            BusDirections_::kInput
        } else {
            BusDirections_::kOutput
        };
        // SAFETY: the plugin's own function, on a live component.
        unsafe {
            self.component
                .getBusCount(MediaTypes_::kAudio as i32, direction as i32)
        }
    }

    fn buses(&self, is_input: bool) -> Vec<PortInfo> {
        let direction = if is_input {
            BusDirections_::kInput
        } else {
            BusDirections_::kOutput
        };
        let media = MediaTypes_::kAudio as i32;
        // SAFETY: the plugin's own function, on a live component.
        let count = unsafe { self.component.getBusCount(media, direction as i32) };
        let mut ports = Vec::with_capacity(count.max(0) as usize);
        for index in 0..count {
            let mut info = blank_bus_info();
            // SAFETY: `index` is below the count just reported, and `info` is a
            // writable `BusInfo`.
            let ok = unsafe {
                self.component
                    .getBusInfo(media, direction as i32, index, &mut info)
            };
            if ok != kResultOk {
                continue;
            }
            ports.push(PortInfo {
                id: index.max(0) as u32,
                name: wide_text(&info.name),
                main: info.busType == BusTypes_::kMain as i32,
                channels: info.channelCount.max(0) as u32,
            });
        }
        ports
    }

    /// Every parameter the controller declares, in its own order, with **plain**
    /// ranges and CLAP's flag word.
    ///
    /// The flags are translated rather than carried, so that
    /// [`ParamDescription::row_worthy`] asks one question of both standards and
    /// the schema is minted by one piece of code.
    #[must_use]
    pub fn params(&self) -> Vec<ParamDescription> {
        let Some(controller) = self.controller.as_ref() else {
            return Vec::new();
        };
        // SAFETY: the controller's own function, on a live controller.
        let count = unsafe { controller.getParameterCount() };
        let mut params = Vec::with_capacity(count.max(0) as usize);
        for index in 0..count {
            let mut info = blank_parameter_info();
            // SAFETY: `index` is below the count just reported, and `info` is a
            // writable `ParameterInfo`.
            if unsafe { controller.getParameterInfo(index, &mut info) } != kResultOk {
                continue;
            }
            // SAFETY: the controller's own conversion, on a live controller.
            let (min, max, default) = unsafe {
                (
                    controller.normalizedParamToPlain(info.id, 0.0),
                    controller.normalizedParamToPlain(info.id, 1.0),
                    controller.normalizedParamToPlain(info.id, info.defaultNormalizedValue),
                )
            };
            params.push(ParamDescription {
                id: info.id,
                name: wide_text(&info.title),
                // VST3 groups parameters by unit rather than by a path string,
                // and a unit needs `IUnitInfo` — which is a panel question
                // (AP5), not a describe one. Top-level until then.
                module: String::new(),
                min,
                max,
                default,
                flags: clap_flags_of(&info),
            });
        }
        params
    }

    /// Whether the plugin can report latency. Every VST3 processor can, so this
    /// is always true — the number is still read off the live instance, because
    /// latency changes with the parameters (§4).
    #[must_use]
    pub const fn reports_latency(&self) -> bool {
        true
    }

    /// The latency the plugin reports, in samples.
    #[must_use]
    pub fn latency(&self) -> u32 {
        // SAFETY: the plugin's own function, on a live processor.
        unsafe { self.processor.getLatencySamples() }
    }

    /// Tell the plugin whether this is an export or a preview. VST3 carries the
    /// answer in `setupProcessing`, so it is remembered here and applied at
    /// [`Vst3Instance::activate`].
    pub fn set_offline(&mut self, offline: bool) -> bool {
        self.offline = offline;
        true
    }

    /// Hand both halves the blob the project saved.
    ///
    /// The blob is the two VST3 streams, length-prefixed
    /// ([`split_state`]): the processor's, then the controller's. **Both** are
    /// delivered — the processor gets its own, and the controller gets the
    /// processor's through `setComponentState` as well as its own through
    /// `setState`. Skipping the middle one is §9's first trap: the knobs and the
    /// sound then disagree after a reload.
    ///
    /// # Errors
    ///
    /// [`HostError::WhileProcessing`] mid-stream, and
    /// [`HostError::StateRefused`] when the component will not take the blob.
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), HostError> {
        if self.processing {
            return Err(HostError::WhileProcessing);
        }
        let (processor_state, controller_state) = split_state(bytes);
        let stream = ComWrapper::new(Stream::over(processor_state.to_vec()));
        let pointer = stream
            .to_com_ptr::<IBStream>()
            .ok_or(HostError::StateRefused)?;
        // SAFETY: the plugin's own function, with a stream that outlives the
        // call, while the plugin is deactivated.
        if unsafe { self.component.setState(pointer.as_ptr()) } != kResultOk {
            return Err(HostError::StateRefused);
        }
        if let Some(controller) = self.controller.as_ref() {
            stream.rewind();
            // SAFETY: as above — the controller reads the *processor's* blob so
            // that the two halves agree.
            unsafe { controller.setComponentState(pointer.as_ptr()) };
            let own = ComWrapper::new(Stream::over(controller_state.to_vec()));
            if let Some(pointer) = own.to_com_ptr::<IBStream>() {
                // SAFETY: as above, with the controller's own blob.
                unsafe { controller.setState(pointer.as_ptr()) };
            }
        }
        Ok(())
    }

    /// The blob to write into the `.lum`: both halves, length-prefixed. Never
    /// parsed, always round-tripped (§4).
    ///
    /// # Errors
    ///
    /// [`HostError::WhileProcessing`] mid-stream and
    /// [`HostError::StateUnsaved`] when the component will not save.
    pub fn save_state(&self) -> Result<Vec<u8>, HostError> {
        if self.processing {
            return Err(HostError::WhileProcessing);
        }
        let processor_state = ComWrapper::new(Stream::default());
        let pointer = processor_state
            .to_com_ptr::<IBStream>()
            .ok_or(HostError::StateUnsaved)?;
        // SAFETY: the plugin's own function, with a stream that outlives the
        // call, while the plugin is not processing.
        if unsafe { self.component.getState(pointer.as_ptr()) } != kResultOk {
            return Err(HostError::StateUnsaved);
        }
        let mut controller_bytes = Vec::new();
        if let Some(controller) = self.controller.as_ref() {
            let own = ComWrapper::new(Stream::default());
            if let Some(pointer) = own.to_com_ptr::<IBStream>() {
                // SAFETY: as above.
                if unsafe { controller.getState(pointer.as_ptr()) } == kResultOk {
                    controller_bytes = own.taken();
                }
            }
        }
        Ok(join_state(&processor_state.taken(), &controller_bytes))
    }

    /// Remember the project's values, and put them on the controller so that its
    /// own saved state agrees with them.
    ///
    /// This is VST3's stand-in for CLAP's `params.flush`, and it runs at the
    /// same moment: after the state, because a saved blob is last year's answer
    /// and the project's keyframes are this year's.
    ///
    /// # Errors
    ///
    /// [`HostError::WhileProcessing`] mid-stream.
    pub fn flush_params(&mut self, events: &[ParamEvent]) -> Result<(), HostError> {
        if self.processing {
            return Err(HostError::WhileProcessing);
        }
        for event in events {
            if let Some(slot) = self.initial.iter_mut().find(|(id, _)| *id == event.id) {
                slot.1 = event.value;
            } else {
                self.initial.push((event.id, event.value));
            }
            if let Some(controller) = self.controller.as_ref() {
                // SAFETY: the controller's own functions, on a live controller,
                // outside a block.
                unsafe {
                    let normalised = controller.plainParamToNormalized(event.id, event.value);
                    controller.setParamNormalized(event.id, normalised);
                }
            }
        }
        Ok(())
    }

    /// Prepare the plugin for 512-frame blocks at 48 kHz, on a stereo pair.
    ///
    /// The buses are negotiated here rather than at describe because
    /// `setBusArrangements` is an inactive-state call that changes what the
    /// plugin *is*: v1 hosts stereo effect plugins, main in and main out, and
    /// every other bus is left inactive (§4).
    ///
    /// # Errors
    ///
    /// [`HostError::BusRefused`] when the plugin will not take a stereo pair,
    /// and [`HostError::ActivateRefused`].
    pub fn activate(&mut self) -> Result<(), HostError> {
        if self.activated {
            return Ok(());
        }
        let mut stereo = SpeakerArr::kStereo;
        // SAFETY: the plugin's own function, with one arrangement each way,
        // while the plugin is inactive.
        let arranged = unsafe {
            self.processor.setBusArrangements(
                std::ptr::addr_of_mut!(stereo),
                1,
                std::ptr::addr_of_mut!(stereo),
                1,
            )
        };
        if arranged != kResultOk && arranged != kResultTrue {
            return Err(HostError::BusRefused);
        }

        // The mains carry the layer's sound; anything else — an aux, a
        // sidechain — is switched off rather than fed silence.
        for (is_input, direction) in [
            (true, BusDirections_::kInput),
            (false, BusDirections_::kOutput),
        ] {
            let count = self.bus_count(is_input);
            for index in 0..count {
                let on = u8::from(index == 0);
                // SAFETY: `index` is below the count the plugin reported.
                unsafe {
                    self.component.activateBus(
                        MediaTypes_::kAudio as i32,
                        direction as i32,
                        index,
                        on,
                    );
                }
            }
        }

        let mut setup = ProcessSetup {
            processMode: if self.offline {
                ProcessModes_::kOffline as i32
            } else {
                ProcessModes_::kRealtime as i32
            },
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            maxSamplesPerBlock: i32::try_from(BLOCK_FRAMES).unwrap_or(i32::MAX),
            sampleRate: SAMPLE_RATE,
        };
        // SAFETY: the plugin's own function, with a live setup, while inactive.
        if unsafe { self.processor.setupProcessing(&mut setup) } != kResultOk {
            return Err(HostError::ActivateRefused);
        }
        // SAFETY: the plugin's own function, after a successful setup.
        if unsafe { self.component.setActive(1) } != kResultOk {
            return Err(HostError::ActivateRefused);
        }
        self.activated = true;
        Ok(())
    }

    /// Enter the processing state.
    ///
    /// # Errors
    ///
    /// [`HostError::StartRefused`], including when the plugin was never
    /// activated.
    pub fn start_processing(&mut self) -> Result<(), HostError> {
        if self.processing {
            return Ok(());
        }
        if !self.activated {
            return Err(HostError::StartRefused);
        }
        // SAFETY: the plugin's own function, after a successful `setActive`.
        if unsafe { self.processor.setProcessing(1) } != kResultOk {
            return Err(HostError::StartRefused);
        }
        self.processing = true;
        Ok(())
    }

    /// One block: the input planes in, the output planes out.
    ///
    /// # Errors
    ///
    /// [`HostError::NotProcessing`] before `start_processing`, and
    /// [`HostError::ProcessFailed`] when the plugin answers with an error.
    pub fn process(&mut self, block: &mut Block, events: &[ParamEvent]) -> Result<(), HostError> {
        if !self.processing {
            return Err(HostError::NotProcessing);
        }
        self.load_changes(events);

        let (input, output) = block.planes();
        // Separate in and out, always: in-place is where plugin bugs live (§9).
        let (in_left, in_right) = input.split_at_mut(BLOCK_FRAMES);
        let (out_left, out_right) = output.split_at_mut(BLOCK_FRAMES);
        let mut in_planes: [*mut f32; CHANNELS] = [in_left.as_mut_ptr(), in_right.as_mut_ptr()];
        let mut out_planes: [*mut f32; CHANNELS] = [out_left.as_mut_ptr(), out_right.as_mut_ptr()];

        let mut incoming = bus_buffers(&mut in_planes);
        let mut outgoing = bus_buffers(&mut out_planes);
        let changes = self
            .changes
            .to_com_ptr::<IParameterChanges>()
            .ok_or(HostError::ProcessFailed)?;

        let mut data = ProcessData {
            processMode: if self.offline {
                ProcessModes_::kOffline as i32
            } else {
                ProcessModes_::kRealtime as i32
            },
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            numSamples: i32::try_from(BLOCK_FRAMES).unwrap_or(i32::MAX),
            numInputs: 1,
            numOutputs: 1,
            inputs: std::ptr::addr_of_mut!(incoming),
            outputs: std::ptr::addr_of_mut!(outgoing),
            inputParameterChanges: changes.as_ptr(),
            outputParameterChanges: std::ptr::null_mut(),
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            // No transport in v1, as CLAP: tempo is supplied only where the comp
            // has a confirmed BPM grid (§3).
            processContext: std::ptr::null_mut(),
        };

        // Denormals off for the call and restored after it, as both standards
        // assume (§3).
        let _denormals = crate::process::Denormals::on();
        // SAFETY: every pointer in `data` is to a live local that outlives this
        // call, the planes are `numSamples` long, and the plugin is in the
        // processing state VST3 requires.
        if unsafe { self.processor.process(std::ptr::addr_of_mut!(data)) } != kResultOk {
            return Err(HostError::ProcessFailed);
        }
        Ok(())
    }

    /// Lay this block's values into the queues: the project's baseline first,
    /// then whatever automation this block carries over the top.
    fn load_changes(&self, events: &[ParamEvent]) {
        self.changes.reset();
        for (id, plain) in &self.initial {
            if events.iter().any(|event| event.id == *id) {
                continue;
            }
            self.changes.point(*id, 0, self.normalise(*id, *plain));
        }
        let mut sorted: Vec<&ParamEvent> = events.iter().collect();
        sorted.sort_by_key(|event| event.time);
        for event in sorted {
            let offset = i32::try_from(event.time).unwrap_or(0);
            self.changes
                .point(event.id, offset, self.normalise(event.id, event.value));
        }
    }

    /// One plain value as the queue carries it. **Asked of the controller every
    /// time** — never cached, because a plugin re-scales its ranges out of a
    /// state blob (§9's second trap).
    fn normalise(&self, id: u32, plain: f64) -> f64 {
        match self.controller.as_ref() {
            // SAFETY: the controller's own function, on a live controller, from
            // the one thread that drives this instance.
            Some(controller) => unsafe { controller.plainParamToNormalized(id, plain) },
            None => plain.clamp(0.0, 1.0),
        }
    }

    /// Leave the processing state.
    pub fn stop_processing(&mut self) {
        if !self.processing {
            return;
        }
        self.processing = false;
        // SAFETY: the plugin's own function, paired with the `setProcessing`
        // that succeeded.
        unsafe { self.processor.setProcessing(0) };
    }

    /// Undo [`Vst3Instance::activate`]. Stops processing first: a plugin may not
    /// be deactivated while it is processing.
    pub fn deactivate(&mut self) {
        self.stop_processing();
        if !self.activated {
            return;
        }
        self.activated = false;
        // SAFETY: the plugin's own function, paired with the `setActive` that
        // succeeded.
        unsafe { self.component.setActive(0) };
    }
}

impl Drop for Vst3Instance {
    fn drop(&mut self) {
        self.deactivate();
        if let Some(controller) = self.controller.as_ref() {
            // SAFETY: the controller's own functions — the handler is dropped
            // after this, so it is still alive while the controller lets go of
            // it. `terminate` only where the controller is an object of its own:
            // the component's own is called below, and calling it twice on one
            // object is how a single-object plugin is torn down twice.
            unsafe {
                controller.setComponentHandler(std::ptr::null_mut());
                if self.split {
                    controller.terminate();
                }
            }
        }
        // SAFETY: the plugin's own function, paired with the `initialize` that
        // succeeded. The module `Arc` is dropped after this, so the library is
        // still loaded while the plugin tears itself down.
        unsafe { self.component.terminate() };
        let _ = (&self.context, &self.handler, &self.changes);
    }
}

/// One block's audio, as a bus.
fn bus_buffers(planes: &mut [*mut f32; CHANNELS]) -> vst3::Steinberg::Vst::AudioBusBuffers {
    vst3::Steinberg::Vst::AudioBusBuffers {
        numChannels: i32::try_from(CHANNELS).unwrap_or(2),
        silenceFlags: 0,
        __field0: vst3::Steinberg::Vst::AudioBusBuffers__type0 {
            channelBuffers32: planes.as_mut_ptr(),
        },
    }
}

// -------------------------------------------------- what the plugin calls --

/// The host, as the plugin sees it.
///
/// It answers its name and refuses to build anything, which is a complete and
/// honest answer: the objects a plugin asks a host to create — messages,
/// attribute lists — belong to the connection between the two halves, and v1
/// does not wire one (see [`Vst3Instance::create`]).
struct HostContext;

impl Class for HostContext {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostContext {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the caller guarantees a writable `String128`.
        unsafe { write_wide(&mut *name, "Lumit") };
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        _cid: *mut TUID,
        _iid: *mut TUID,
        _obj: *mut *mut c_void,
    ) -> tresult {
        kNotImplemented
    }
}

/// What a plugin can ask of the host from inside a callback.
///
/// One flag, atomic, for the same reason CLAP's three are: a plugin may set it
/// from the processing thread, and a host that took a lock there would deadlock
/// against its own loop (docs/14 §7). The gestures — begin, perform, end — are
/// accepted and dropped: v1 has nowhere to put a value the plugin changed,
/// because the plugin's own window is the follow-on package (§6).
#[derive(Default)]
struct Handler {
    restart: AtomicBool,
}

impl Class for Handler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for Handler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn performEdit(&self, _id: ParamID, _value: ParamValue) -> tresult {
        kResultOk
    }

    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn restartComponent(&self, _flags: i32) -> tresult {
        self.restart.store(true, Ordering::Relaxed);
        kResultOk
    }
}

/// A state blob, as VST3 wants to read and write one.
#[derive(Default)]
struct Stream {
    bytes: RefCell<Vec<u8>>,
    at: Cell<i64>,
}

impl Stream {
    /// A stream the plugin will read from.
    fn over(bytes: Vec<u8>) -> Self {
        Self {
            bytes: RefCell::new(bytes),
            at: Cell::new(0),
        }
    }
}

/// The two things a caller does with a stream after the plugin has finished
/// with it.
trait Rewind {
    /// Put the cursor back at the start, so a second reader sees the whole blob.
    fn rewind(&self);
    /// The bytes, taken.
    fn taken(&self) -> Vec<u8>;
}

impl Rewind for ComWrapper<Stream> {
    fn rewind(&self) {
        self.at.set(0);
    }

    fn taken(&self) -> Vec<u8> {
        self.bytes
            .try_borrow()
            .map(|bytes| bytes.clone())
            .unwrap_or_default()
    }
}

impl Class for Stream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for Stream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: i32, read: *mut i32) -> tresult {
        if buffer.is_null() || num_bytes < 0 {
            return kInvalidArgument;
        }
        let Ok(bytes) = self.bytes.try_borrow() else {
            return kInvalidArgument;
        };
        let at = usize::try_from(self.at.get()).unwrap_or(usize::MAX);
        let left = bytes.len().saturating_sub(at);
        let wanted = usize::try_from(num_bytes).unwrap_or(0).min(left);
        if wanted > 0 {
            // SAFETY: `wanted` bytes remain in the blob and the caller
            // guarantees the buffer holds them.
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr().add(at), buffer.cast::<u8>(), wanted);
            }
        }
        self.at.set(
            self.at
                .get()
                .saturating_add(i64::try_from(wanted).unwrap_or(0)),
        );
        if !read.is_null() {
            // SAFETY: the caller guarantees a writable count.
            unsafe { *read = i32::try_from(wanted).unwrap_or(0) };
        }
        kResultOk
    }

    unsafe fn write(&self, buffer: *mut c_void, num_bytes: i32, written: *mut i32) -> tresult {
        if buffer.is_null() || num_bytes < 0 {
            return kInvalidArgument;
        }
        let Ok(mut bytes) = self.bytes.try_borrow_mut() else {
            return kInvalidArgument;
        };
        let count = usize::try_from(num_bytes).unwrap_or(0);
        // A plugin in a loop must not fill our memory (docs/12 §2.3), the same
        // ceiling the CLAP stream carries.
        const CAP: usize = 64 * 1024 * 1024;
        if bytes.len().saturating_add(count) > CAP {
            return kInvalidArgument;
        }
        // SAFETY: the caller guarantees `count` readable bytes.
        let incoming = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), count) };
        let at = usize::try_from(self.at.get()).unwrap_or(usize::MAX);
        if at > bytes.len() {
            bytes.resize(at, 0);
        }
        let end = at.saturating_add(count);
        if end > bytes.len() {
            bytes.resize(end, 0);
        }
        if let Some(slot) = bytes.get_mut(at..end) {
            slot.copy_from_slice(incoming);
        }
        self.at.set(i64::try_from(end).unwrap_or(i64::MAX));
        if !written.is_null() {
            // SAFETY: the caller guarantees a writable count.
            unsafe { *written = i32::try_from(count).unwrap_or(0) };
        }
        kResultOk
    }

    unsafe fn seek(&self, pos: i64, mode: i32, result: *mut i64) -> tresult {
        let Ok(bytes) = self.bytes.try_borrow() else {
            return kInvalidArgument;
        };
        /// From the start.
        const SET: i32 = 0;
        /// From where we are.
        const CUR: i32 = 1;
        /// From the end.
        const END: i32 = 2;
        let base = match mode {
            SET => 0,
            CUR => self.at.get(),
            END => i64::try_from(bytes.len()).unwrap_or(i64::MAX),
            _ => return kInvalidArgument,
        };
        let at = base.saturating_add(pos).max(0);
        self.at.set(at);
        if !result.is_null() {
            // SAFETY: the caller guarantees a writable position.
            unsafe { *result = at };
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut i64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the caller guarantees a writable position.
        unsafe { *pos = self.at.get() };
        kResultOk
    }
}

/// One parameter's points inside one block.
#[derive(Default)]
struct Queue {
    id: Cell<u32>,
    points: RefCell<Vec<(i32, f64)>>,
}

impl Class for Queue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for Queue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id.get()
    }

    unsafe fn getPointCount(&self) -> i32 {
        self.points
            .try_borrow()
            .map_or(0, |points| i32::try_from(points.len()).unwrap_or(0))
    }

    unsafe fn getPoint(&self, index: i32, offset: *mut i32, value: *mut ParamValue) -> tresult {
        let Ok(points) = self.points.try_borrow() else {
            return kInvalidArgument;
        };
        let Some(point) = usize::try_from(index).ok().and_then(|at| points.get(at)) else {
            return kInvalidArgument;
        };
        if offset.is_null() || value.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the caller guarantees both are writable.
        unsafe {
            *offset = point.0;
            *value = point.1;
        }
        kResultOk
    }

    unsafe fn addPoint(&self, offset: i32, value: ParamValue, index: *mut i32) -> tresult {
        let Ok(mut points) = self.points.try_borrow_mut() else {
            return kInvalidArgument;
        };
        points.push((offset, value));
        if !index.is_null() {
            // SAFETY: the caller guarantees a writable index.
            unsafe { *index = i32::try_from(points.len().saturating_sub(1)).unwrap_or(0) };
        }
        kResultOk
    }
}

/// The queues one block carries, allocated once and refilled.
///
/// A plugin reads them from inside `process` and never after it, so the same
/// objects serve every block — which is what keeps a block free of allocation
/// (docs/14's budgeted allocations).
#[derive(Default)]
struct Changes {
    queues: RefCell<Vec<ComWrapper<Queue>>>,
    live: Cell<usize>,
}

impl Class for Changes {
    type Interfaces = (IParameterChanges,);
}

impl Changes {
    /// Forget last block's points.
    fn reset(&self) {
        self.live.set(0);
        if let Ok(queues) = self.queues.try_borrow() {
            for queue in queues.iter() {
                if let Ok(mut points) = queue.points.try_borrow_mut() {
                    points.clear();
                }
            }
        }
    }

    /// One value for one parameter at one frame. Points for the same parameter
    /// share a queue, which is what VST3 expects and what lets a plugin read a
    /// ramp in order.
    fn point(&self, id: u32, offset: i32, normalised: f64) {
        let Ok(mut queues) = self.queues.try_borrow_mut() else {
            return;
        };
        let live = self.live.get();
        let existing = queues
            .iter()
            .take(live)
            .position(|queue| queue.id.get() == id);
        let at = match existing {
            Some(at) => at,
            None => {
                if queues.len() <= live {
                    queues.push(ComWrapper::new(Queue::default()));
                }
                if let Some(queue) = queues.get(live) {
                    queue.id.set(id);
                }
                self.live.set(live.saturating_add(1));
                live
            }
        };
        if let Some(queue) = queues.get(at) {
            if let Ok(mut points) = queue.points.try_borrow_mut() {
                points.push((offset, normalised));
            }
        }
    }
}

impl IParameterChangesTrait for Changes {
    unsafe fn getParameterCount(&self) -> i32 {
        i32::try_from(self.live.get()).unwrap_or(0)
    }

    unsafe fn getParameterData(&self, index: i32) -> *mut IParamValueQueue {
        let Ok(queues) = self.queues.try_borrow() else {
            return std::ptr::null_mut();
        };
        let Some(at) = usize::try_from(index)
            .ok()
            .filter(|at| *at < self.live.get())
        else {
            return std::ptr::null_mut();
        };
        queues.get(at).map_or(std::ptr::null_mut(), |queue| {
            queue
                .as_com_ref::<IParamValueQueue>()
                .map_or(std::ptr::null_mut(), |reference| reference.as_ptr())
        })
    }

    unsafe fn addParameterData(
        &self,
        _id: *const ParamID,
        _index: *mut i32,
    ) -> *mut IParamValueQueue {
        // The host writes this list; a plugin adding to it would be a plugin
        // automating the host, which is the editor window's business (§6).
        std::ptr::null_mut()
    }
}

// ------------------------------------------------------------ translation --

/// One VST3 parameter's flags, in CLAP's word.
///
/// The two standards ask the same four questions with different spellings, so
/// the answer is translated once, here, and
/// [`ParamDescription::row_worthy`](crate::describe::ParamDescription::row_worthy)
/// stays one rule for both.
fn clap_flags_of(info: &ParameterInfo) -> u32 {
    use clap_sys::ext::params::{
        CLAP_PARAM_IS_AUTOMATABLE, CLAP_PARAM_IS_BYPASS, CLAP_PARAM_IS_HIDDEN,
        CLAP_PARAM_IS_READONLY, CLAP_PARAM_IS_STEPPED,
    };
    let mut flags = 0;
    if info.flags & ParameterFlags_::kCanAutomate != 0 {
        flags |= CLAP_PARAM_IS_AUTOMATABLE;
    }
    if info.flags & ParameterFlags_::kIsHidden != 0 {
        flags |= CLAP_PARAM_IS_HIDDEN;
    }
    if info.flags & ParameterFlags_::kIsReadOnly != 0 {
        flags |= CLAP_PARAM_IS_READONLY;
    }
    if info.flags & ParameterFlags_::kIsBypass != 0 {
        flags |= CLAP_PARAM_IS_BYPASS;
    }
    if info.stepCount > 0 {
        flags |= CLAP_PARAM_IS_STEPPED;
    }
    flags
}

/// How many bytes of a saved blob belong to the processor.
const PROCESSOR_LENGTH: usize = 4;

/// The two halves of a VST3 state, as one blob.
///
/// Four bytes of length and then the two runs. It is not a format anybody parses
/// — the `.lum` round-trips it whole — but it has to be split back into two
/// streams, and a length is the only honest way to do that.
#[must_use]
pub fn join_state(processor: &[u8], controller: &[u8]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(PROCESSOR_LENGTH + processor.len() + controller.len());
    blob.extend_from_slice(&u32::try_from(processor.len()).unwrap_or(0).to_le_bytes());
    blob.extend_from_slice(processor);
    blob.extend_from_slice(controller);
    blob
}

/// The two halves back out. A blob too short to hold a length is all the
/// processor's, which is what a blob written by another program would be.
#[must_use]
pub fn split_state(blob: &[u8]) -> (&[u8], &[u8]) {
    let Some(head) = blob.get(..PROCESSOR_LENGTH) else {
        return (blob, &[]);
    };
    let mut four = [0u8; PROCESSOR_LENGTH];
    four.copy_from_slice(head);
    let length = u32::from_le_bytes(four) as usize;
    let body = blob.get(PROCESSOR_LENGTH..).unwrap_or(&[]);
    match body.split_at_checked(length) {
        Some(halves) => halves,
        None => (blob, &[]),
    }
}

/// A class id as the 32 hex digits that name it.
///
/// The plugin's identifier, everywhere: it is what the switched-off list holds,
/// what the effect's match name is spelled from, and what a saved project
/// stores. It is a class id rather than a name for the same reason a row is
/// `p<number>` rather than a label — the id is what the vendor promises not to
/// change (docs/impl/audio-plugins.md §4).
#[must_use]
pub fn hex_of(cid: &TUID) -> String {
    let mut text = String::with_capacity(32);
    for byte in cid {
        text.push_str(&format!("{:02x}", *byte as u8));
    }
    text
}

/// The class id 32 hex digits name, or `None` for anything else.
#[must_use]
pub fn tuid_from_hex(text: &str) -> Option<TUID> {
    if text.len() != 32 {
        return None;
    }
    let mut cid: TUID = [0; 16];
    for (index, slot) in cid.iter_mut().enumerate() {
        let pair = text.get(index * 2..index * 2 + 2)?;
        *slot = u8::from_str_radix(pair, 16).ok()? as i8;
    }
    Some(cid)
}

/// A fixed-width C string field, as one of ours. Empty for anything that is not
/// UTF-8 — a report line, never a failure.
fn text_of(field: &[i8]) -> String {
    let bytes: Vec<u8> = field
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| *byte as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A VST3 `String128` — UTF-16, nul-terminated — as one of ours.
fn wide_text(field: &[TChar]) -> String {
    let units: Vec<u16> = field
        .iter()
        .take_while(|unit| **unit != 0)
        .copied()
        .collect();
    String::from_utf16_lossy(&units)
}

/// One of ours into a `String128`, nul-terminated and never overrun.
fn write_wide(field: &mut [TChar], text: &str) {
    let mut written = 0;
    for (slot, unit) in field.iter_mut().zip(text.encode_utf16()) {
        *slot = unit;
        written += 1;
    }
    if let Some(slot) = field.get_mut(written.min(field.len().saturating_sub(1))) {
        *slot = 0;
    }
}

/// An empty class description for the factory to fill in.
fn blank_class_info() -> PClassInfo {
    PClassInfo {
        cid: [0; 16],
        cardinality: 0,
        category: [0; 32],
        name: [0; 64],
    }
}

/// The wider one.
fn blank_class_info2() -> PClassInfo2 {
    PClassInfo2 {
        cid: [0; 16],
        cardinality: 0,
        category: [0; 32],
        name: [0; 64],
        classFlags: 0,
        subCategories: [0; 128],
        vendor: [0; 64],
        version: [0; 64],
        sdkVersion: [0; 64],
    }
}

/// An empty bus description for the plugin to fill in.
fn blank_bus_info() -> BusInfo {
    BusInfo {
        mediaType: 0,
        direction: 0,
        channelCount: 0,
        name: [0; 128],
        busType: 0,
        flags: 0,
    }
}

/// An empty parameter description for the controller to fill in.
fn blank_parameter_info() -> ParameterInfo {
    ParameterInfo {
        id: 0,
        title: [0; 128],
        shortTitle: [0; 128],
        units: [0; 128],
        stepCount: 0,
        defaultNormalizedValue: 0.0,
        unitId: 0,
        flags: 0,
    }
}

/// A class id from four words, the way the SDK's own headers spell one — the
/// byte order differs between Windows and everywhere else, and this is the
/// single place that knows it.
#[must_use]
pub const fn class_id(a: u32, b: u32, c: u32, d: u32) -> TUID {
    uid(a, b, c, d)
}
