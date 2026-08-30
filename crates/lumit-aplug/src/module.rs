//! Opening a `.clap` file, and asking its factory what is inside.
//!
//! # In plain terms
//!
//! A CLAP plugin ships as an ordinary shared library — a DLL on Windows —
//! whose only agreed-on export is a single structure called `clap_entry`. That
//! structure holds three function pointers: start up, shut down, and hand over
//! a *factory*. The factory is a short list: how many plugins are in this file,
//! what is each one called, and make me one of them.
//!
//! This module is that much and no more. It opens the file, checks that what it
//! found calls itself a version of CLAP we can speak, starts it, takes the
//! factory, and reads the list. Nothing here creates a plugin or plays a
//! sample; [`crate::instance`] does that.
//!
//! **The order matters and the spec is strict about it.** `init` runs before
//! anything else and may run only once per module; `deinit` runs after every
//! plugin from the module has been destroyed. Getting that wrong is how a host
//! unloads a library while somebody's audio thread is still inside it, so the
//! module owns the library and drops it last.

use std::ffi::{c_char, CStr, CString};
use std::path::{Path, PathBuf};

use clap_sys::entry::clap_plugin_entry;
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::version::clap_version_is_compatible;
use thiserror::Error;

/// Why a `.clap` file could not be opened, or held nothing usable.
#[derive(Debug, Error)]
pub enum ModuleError {
    /// The operating system would not load the library at all.
    #[error("the module did not load ({0})")]
    NotLoaded(String),
    /// The library loaded but exports no `clap_entry`, so it is not a CLAP
    /// plugin however it is named.
    #[error("the module exports no clap_entry")]
    NoEntry,
    /// The entry declares a CLAP version this host cannot speak.
    #[error("the module declares CLAP {major}.{minor}.{revision}, which this host cannot speak")]
    Incompatible {
        /// The major version it declared.
        major: u32,
        /// The minor version it declared.
        minor: u32,
        /// The revision it declared.
        revision: u32,
    },
    /// `clap_entry.init` answered false. The plugin has refused to start, and
    /// per the spec nothing else in the module may be called.
    #[error("the module refused to initialise")]
    InitRefused,
    /// The entry has no plugin factory — a preset-provider-only module, or a
    /// broken one.
    #[error("the module offers no plugin factory")]
    NoFactory,
    /// The path is not valid for the platform's loader.
    #[error("the module path cannot be passed to the loader")]
    BadPath,
}

/// What one plugin in a module calls itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleEntry {
    /// The plugin's own stable identifier — the key everything else uses.
    pub id: String,
    /// The name a person sees.
    pub name: String,
    /// Who wrote it.
    pub vendor: String,
    /// Its own version string, verbatim. CLAP does not say what shape it takes.
    pub version: String,
    /// The feature words it declares (`audio-effect`, `instrument`, `stereo`,
    /// …), which is how a plugin says what family it belongs to.
    pub features: Vec<String>,
}

/// One loaded `.clap` file.
///
/// **Not `Sync`.** CLAP splits its functions into main-thread and audio-thread
/// halves, and everything this module calls is the main-thread half. One
/// instance is processed single-threaded (docs/impl/audio-plugins.md §5);
/// parallelism is across layers, not inside one plugin.
pub struct Module {
    /// Kept so the library outlives every pointer taken out of it. Dropped
    /// last, after `deinit`.
    library: libloading::Library,
    entry: *const clap_plugin_entry,
    factory: *const clap_plugin_factory,
    path: PathBuf,
    entries: Vec<ModuleEntry>,
}

// SAFETY: the two raw pointers are into the loaded library's own read-only
// data, and the library is process-wide and outlives the `Module` that holds
// it — so moving a module to the thread that will drive it (the chain worker
// AP3 spawns) reaches the same bytes. Shared access reads the entry list and
// the pointers and nothing else; the one shared method that calls into the
// plugin, [`Module::create`], is `unsafe` and its contract carries CLAP's
// main-thread rule.
unsafe impl Send for Module {}
// SAFETY: as above.
unsafe impl Sync for Module {}

impl Module {
    /// Open a `.clap` file, start it, and read its plugin list.
    ///
    /// # Errors
    ///
    /// Every way a third party's file can disappoint us: it will not load, it
    /// is not a CLAP module, it speaks a CLAP we do not, it refuses to start,
    /// or it offers no plugins. All of them are report lines, never dialogues
    /// (docs/12 §2.6).
    pub fn open(path: &Path) -> Result<Self, ModuleError> {
        // SAFETY: loading a library runs its initialisers, which is inherently
        // third-party code. There is no safe spelling of this; the isolation
        // that makes it survivable is the broker process (AP2), not a Rust
        // keyword.
        let library = unsafe { libloading::Library::new(path) }
            .map_err(|error| ModuleError::NotLoaded(error.to_string()))?;

        let entry: *const clap_plugin_entry = {
            // SAFETY: `clap_entry` is a data symbol; `Symbol<*const T>` reads
            // the symbol's address as that pointer, which is what a data
            // symbol is.
            let symbol = unsafe { library.get::<*const clap_plugin_entry>(b"clap_entry\0") }
                .map_err(|_| ModuleError::NoEntry)?;
            *symbol
        };
        if entry.is_null() {
            return Err(ModuleError::NoEntry);
        }
        // SAFETY: a non-null data symbol from a loaded library points at that
        // library's own static, which lives as long as the library.
        let table = unsafe { &*entry };
        if !clap_version_is_compatible(table.clap_version) {
            return Err(ModuleError::Incompatible {
                major: table.clap_version.major,
                minor: table.clap_version.minor,
                revision: table.clap_version.revision,
            });
        }

        let text = path.to_str().ok_or(ModuleError::BadPath)?;
        let c_path = CString::new(text).map_err(|_| ModuleError::BadPath)?;
        let init = table.init.ok_or(ModuleError::NoEntry)?;
        // SAFETY: the entry's own function, called once, before anything else,
        // exactly as the spec requires.
        if !unsafe { init(c_path.as_ptr()) } {
            return Err(ModuleError::InitRefused);
        }

        let get_factory = table.get_factory.ok_or(ModuleError::NoFactory)?;
        // SAFETY: the entry's own function, called after a successful `init`.
        let factory =
            unsafe { get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }.cast::<clap_plugin_factory>();
        if factory.is_null() {
            // The module started, so it must be stopped again before the
            // library goes: a `deinit` skipped here is a leak in somebody
            // else's code we would never see.
            if let Some(deinit) = table.deinit {
                // SAFETY: the entry's own function, paired with the `init`
                // that succeeded above.
                unsafe { deinit() };
            }
            return Err(ModuleError::NoFactory);
        }

        let mut module = Self {
            library,
            entry,
            factory,
            path: path.to_path_buf(),
            entries: Vec::new(),
        };
        module.entries = module.read_entries();
        Ok(module)
    }

    /// The plugins this module declares, in the factory's own order.
    #[must_use]
    pub fn entries(&self) -> &[ModuleEntry] {
        &self.entries
    }

    /// The file this module came out of.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Make one plugin, by its own id.
    ///
    /// `host` must outlive the plugin: CLAP hands the pointer straight back on
    /// every callback. Answers `None` when the factory does not know the id or
    /// refuses to build it.
    ///
    /// # Safety
    ///
    /// `host` must point at a `clap_host` that stays put and stays alive until
    /// the returned plugin is destroyed.
    pub unsafe fn create(&self, id: &CStr, host: *const clap_host) -> Option<*const clap_plugin> {
        // SAFETY: the factory pointer came from a successful `get_factory` and
        // lives as long as the module.
        let factory = unsafe { &*self.factory };
        let create = factory.create_plugin?;
        // SAFETY: the factory's own function, with the factory it belongs to
        // and a host the caller guarantees.
        let plugin = unsafe { create(self.factory, host, id.as_ptr()) };
        (!plugin.is_null()).then_some(plugin)
    }

    /// Walk the factory's list once.
    fn read_entries(&self) -> Vec<ModuleEntry> {
        // SAFETY: as `create`.
        let factory = unsafe { &*self.factory };
        let (Some(count), Some(get)) = (factory.get_plugin_count, factory.get_plugin_descriptor)
        else {
            return Vec::new();
        };
        // SAFETY: the factory's own functions, with the factory they belong to.
        let total = unsafe { count(self.factory) };
        let mut entries = Vec::with_capacity(total as usize);
        for index in 0..total {
            // SAFETY: `index` is below the count the factory just reported.
            let descriptor = unsafe { get(self.factory, index) };
            if descriptor.is_null() {
                continue;
            }
            // SAFETY: a non-null descriptor is the module's own static, valid
            // while the module is loaded.
            let d = unsafe { &*descriptor };
            // SAFETY: every string on a descriptor is the module's own static
            // text, nul-terminated, and `features` a null-terminated array of
            // the same — which is what CLAP declares them to be.
            let entry = unsafe {
                let id = text(d.id);
                if id.is_empty() {
                    continue;
                }
                ModuleEntry {
                    id,
                    name: text(d.name),
                    vendor: text(d.vendor),
                    version: text(d.version),
                    features: texts(d.features),
                }
            };
            entries.push(entry);
        }
        entries
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        // SAFETY: the entry pointer is the library's own static and the
        // library is still loaded — it is dropped after this, being declared
        // first.
        let table = unsafe { &*self.entry };
        if let Some(deinit) = table.deinit {
            // SAFETY: the entry's own function, paired with the `init` that
            // succeeded in `open`, and every plugin from this module has been
            // destroyed because each one holds an `Arc` of this module.
            unsafe { deinit() };
        }
        let _ = &self.library;
    }
}

/// A C string the plugin owns, as one of ours. Empty for null and for anything
/// that is not UTF-8 — a report line, never a failure.
///
/// # Safety
///
/// `pointer` must be null or a nul-terminated string that lives for the call.
/// Everything reached here is a plugin's own static text, which CLAP says lives
/// as long as the module.
#[must_use]
pub unsafe fn text(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: the caller has a CLAP contract that this is nul-terminated and
    // lives as long as the module.
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .unwrap_or_default()
        .to_owned()
}

/// A null-terminated array of C strings, as ours.
///
/// # Safety
///
/// `list` must be null or a null-terminated array of nul-terminated strings.
#[must_use]
unsafe fn texts(list: *const *const c_char) -> Vec<String> {
    if list.is_null() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut index = 0usize;
    loop {
        // SAFETY: the caller guarantees the array runs to a null entry, and
        // `index` has not reached it yet.
        let item = unsafe { *list.add(index) };
        if item.is_null() {
            return out;
        }
        // SAFETY: a non-null entry in that array is a nul-terminated string.
        out.push(unsafe { text(item) });
        index = index.saturating_add(1);
        // A plugin with a missing terminator would walk us off the end. Stop
        // at a number no honest plugin reaches.
        // ponytail: a fixed cap rather than a length the API does not carry;
        // there is no better answer available at this boundary.
        if index > 64 {
            return out;
        }
    }
}
