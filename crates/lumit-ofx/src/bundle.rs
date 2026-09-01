//! Bundles: finding the plugin on disk, opening it, and the first two words
//! anyone says.
//!
//! # In plain terms
//!
//! An OFX plugin is shipped as a folder called something `.ofx.bundle`, with
//! a shared library buried in it at a fixed path. Opening it means loading
//! that library and asking it two questions: how many plugins are in here, and
//! give me the description of plugin number *n*.
//!
//! Then the order matters, and getting it wrong crashes plugins that are
//! otherwise blameless: **`setHost` first**, before any action at all, because
//! a plugin's load handler will immediately fetch suites from the host it was
//! given, and a plugin given nothing will read a null pointer. Only after that
//! comes `kOfxActionLoad`, and at the end `kOfxActionUnload`, once.

use std::ffi::{c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};

use libloading::{Library, Symbol};
use thiserror::Error;

use crate::ffi::{
    actions, OfxGetNumberOfPluginsFn, OfxGetPluginFn, OfxPlugin, K_OFX_GET_NUMBER_OF_PLUGINS,
    K_OFX_GET_PLUGIN, K_OFX_IMAGE_EFFECT_PLUGIN_API,
};
use crate::handles::Handle;
use crate::host::host;
use crate::status::Status;

/// The directory inside a bundle that holds the binary for this platform.
#[cfg(target_os = "windows")]
pub const BUNDLE_ARCH_DIR: &str = "Win64";
/// See above.
#[cfg(target_os = "macos")]
pub const BUNDLE_ARCH_DIR: &str = "MacOS";
/// See above.
#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
pub const BUNDLE_ARCH_DIR: &str = "Linux-x86-64";

/// What can go wrong before a plugin has said a word.
#[derive(Debug, Error)]
pub enum BundleError {
    /// The library would not load: missing, wrong architecture, or missing a
    /// dependency of its own.
    #[error("the plugin binary at {path} could not be opened: {source}")]
    Open {
        /// The binary we tried.
        path: PathBuf,
        /// What the loader said.
        source: libloading::Error,
    },
    /// A `.ofx` binary without both exports is not an OFX plugin.
    #[error("{path} does not export {export}, so it is not an OFX plugin")]
    MissingExport {
        /// The binary we tried.
        path: PathBuf,
        /// Which export was missing.
        export: &'static str,
    },
    /// `OfxGetNumberOfPlugins` answered something impossible.
    #[error("{path} says it holds {count} plugins")]
    ImplausiblePluginCount {
        /// The binary we tried.
        path: PathBuf,
        /// What it said.
        count: c_int,
    },
}

/// One plugin inside a bundle.
pub struct PluginRef {
    /// The plugin's own identifier, e.g. `net.sf.openfx.invertPlugin`.
    pub identifier: String,
    /// Major and minor version, as the plugin declares them.
    pub version: (u32, u32),
    /// Which API it claims, e.g. `OfxImageEffectPluginAPI`.
    pub api: String,
    /// The version of that API it was written against.
    pub api_version: c_int,
    /// What `kOfxActionLoad` answered, once [`Bundle::load`] has run.
    pub load_status: Option<Status>,
    /// The plugin's own struct, inside the loaded library. Valid only while
    /// the owning [`Bundle`] holds the library open.
    raw: *const OfxPlugin,
}

// SAFETY: `raw` points into the loaded library's own read-only image and is
// never written through; the [`Bundle`] that made it keeps that library open
// for as long as the reference exists, and unloading takes the bundle by value.
// Dispatching an action through it from two threads at once is exactly what the
// OFX thread-safety declaration governs, and `crate::instance::render_lock` is
// where that governing happens — a plugin that said it may not be entered twice
// is not entered twice (K-066). Without these the definition a plugin becomes
// (K-593) could not be `Sync`, and every effect in the catalogue is.
unsafe impl Send for PluginRef {}
// SAFETY: see above.
unsafe impl Sync for PluginRef {}

impl PluginRef {
    /// Whether this is an image effect of the API version this host speaks.
    #[must_use]
    pub fn is_supported_image_effect(&self) -> bool {
        self.api == K_OFX_IMAGE_EFFECT_PLUGIN_API
    }

    /// Dispatch one action at this plugin.
    ///
    /// The three handles are Lumit's own, never addresses (see
    /// [`crate::handles`]), and `None` is the null the no-argument actions
    /// take. Taking them as handles rather than pointers is what keeps this
    /// door safe to walk through: there is no raw pointer for a caller to get
    /// wrong.
    ///
    /// **No host lock may be held across this call** (docs/14 §7): the plugin
    /// re-enters the suites from inside the action, and a lock held here would
    /// deadlock on the first property it reads. The raw `OfxPlugin` stays
    /// private to this module so that every call into a plugin goes through
    /// this one door.
    #[must_use]
    pub fn action(
        &self,
        action: &str,
        handle: Option<Handle>,
        in_args: Option<Handle>,
        out_args: Option<Handle>,
    ) -> Status {
        let pointer =
            |handle: Option<Handle>| handle.map_or(std::ptr::null_mut::<c_void>(), Handle::as_ptr);
        let (handle, in_args, out_args) = (pointer(handle), pointer(in_args), pointer(out_args));
        // SAFETY: `raw` points into the library the owning `Bundle` holds open;
        // a `PluginRef` cannot outlive it, because `unload` clears the list
        // before dropping the library.
        let plugin = unsafe { &*self.raw };
        let Some(main_entry) = plugin.main_entry else {
            return Status::ErrFatal;
        };
        let Ok(action) = CString::new(action) else {
            return Status::ErrValue;
        };
        // SAFETY: the plugin's own entry point, given a valid action name and
        // the handles the caller minted.
        let code = unsafe { main_entry(action.as_ptr(), handle, in_args, out_args) };
        Status::from_code(code)
    }
}

/// An opened bundle binary and the plugins inside it.
///
/// The bundle owns the loaded library, and every [`PluginRef`] points into it,
/// so dropping the bundle unloads the library and invalidates them together.
/// A bundle is not `Send`: a plugin binary may have thread-affine state of its
/// own, and the out-of-process design gives each bundle one process anyway
/// (docs/12 §2.3).
pub struct Bundle {
    path: PathBuf,
    library: Option<Library>,
    plugins: Vec<PluginRef>,
    loaded: bool,
}

impl Bundle {
    /// Open a bundle binary and read the plugins it declares. Nothing is
    /// called on the plugins yet.
    ///
    /// # Errors
    ///
    /// [`BundleError`] if the library will not load or is not an OFX plugin.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BundleError> {
        let path = path.as_ref().to_path_buf();
        // SAFETY: loading a library runs its initialisers, which is arbitrary
        // code — unavoidable, and the reason docs/12 §2.3 puts plugins in
        // their own process. Nothing here can make that safer; what it can do
        // is keep the library alive for exactly as long as the pointers into
        // it are used, which is what `Bundle` owning it achieves.
        let library = unsafe { Library::new(&path) }.map_err(|source| BundleError::Open {
            path: path.clone(),
            source,
        })?;

        // The two symbols borrow the library, so they are read, used, and
        // dropped inside this block; the library itself is moved into the
        // bundle afterwards.
        let plugins = {
            // SAFETY: the two exports are read as the types OFX declares for
            // them; a binary that exports them at another type is not an OFX
            // plugin and there is no way to tell from here. The symbols are
            // used only while `library` is alive.
            let plugin_count: Symbol<OfxGetNumberOfPluginsFn> = unsafe {
                library.get(K_OFX_GET_NUMBER_OF_PLUGINS)
            }
            .map_err(|_| BundleError::MissingExport {
                path: path.clone(),
                export: "OfxGetNumberOfPlugins",
            })?;
            // SAFETY: as above.
            let get_plugin: Symbol<OfxGetPluginFn> = unsafe { library.get(K_OFX_GET_PLUGIN) }
                .map_err(|_| BundleError::MissingExport {
                    path: path.clone(),
                    export: "OfxGetPlugin",
                })?;

            // SAFETY: calling the plugin's own export, which takes no
            // arguments.
            let count = unsafe { plugin_count() };
            // A negative count is a broken plugin; a wildly large one is a
            // broken plugin that would otherwise have us call it a billion
            // times.
            if !(0..=4096).contains(&count) {
                return Err(BundleError::ImplausiblePluginCount { path, count });
            }

            let mut plugins = Vec::new();
            for index in 0..count {
                // SAFETY: index is within the count the plugin itself declared.
                let raw = unsafe { get_plugin(index) };
                if raw.is_null() {
                    continue;
                }
                // SAFETY: the plugin promises this points at a static
                // `OfxPlugin` that lives as long as the library; the fields
                // are read once and copied out, so nothing borrows it after.
                let plugin = unsafe { &*raw };
                // SAFETY: the plugin's own string constants.
                let Some(api) = (unsafe { read_c_string(plugin.plugin_api) }) else {
                    continue;
                };
                // SAFETY: as above.
                let Some(identifier) = (unsafe { read_c_string(plugin.plugin_identifier) }) else {
                    continue;
                };
                plugins.push(PluginRef {
                    identifier,
                    version: (plugin.plugin_version_major, plugin.plugin_version_minor),
                    api,
                    api_version: plugin.api_version,
                    load_status: None,
                    raw,
                });
            }
            plugins
        };

        Ok(Self {
            path,
            library: Some(library),
            plugins,
            loaded: false,
        })
    }

    /// Where the binary came from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The plugins this bundle declares.
    #[must_use]
    pub fn plugins(&self) -> &[PluginRef] {
        &self.plugins
    }

    /// Hand every plugin the host and then load it.
    ///
    /// `setHost` runs before any action, once per plugin, and calling this
    /// twice does nothing the second time. A plugin that refuses to load
    /// records its status in [`PluginRef::load_status`] and is skipped from
    /// then on; the rest of the bundle carries on.
    pub fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let host = host();
        for plugin in &mut self.plugins {
            if !plugin.is_supported_image_effect() {
                continue;
            }
            // SAFETY: `raw` came from this bundle's still-open library, and
            // the host outlives every plugin by construction (it is leaked).
            let raw = unsafe { &*plugin.raw };
            if let Some(set_host) = raw.set_host {
                // SAFETY: the plugin's own function, given the one argument it
                // takes. No host lock is held across this call: the plugin
                // will re-enter the suites from inside it.
                unsafe { set_host(host) };
            }
            plugin.load_status = Some(plugin.action(actions::LOAD, None, None, None));
        }
    }

    /// Unload every plugin and close the library. Safe to call twice: the
    /// second call has nothing left to do, which is the difference between a
    /// tidy shutdown and a double `kOfxActionUnload` — a call plugins are not
    /// required to survive.
    pub fn unload(&mut self) {
        if self.loaded {
            for plugin in &mut self.plugins {
                if plugin.load_status != Some(Status::Ok) {
                    continue;
                }
                let _ = plugin.action(actions::UNLOAD, None, None, None);
                plugin.load_status = None;
            }
            self.loaded = false;
        }
        self.plugins.clear();
        // Dropping the library is what unloads it; taking it means a second
        // call finds nothing.
        drop(self.library.take());
    }
}

impl Drop for Bundle {
    fn drop(&mut self) {
        self.unload();
    }
}

/// Copy a C string out of a plugin, or `None` if it is null or not text.
///
/// # Safety
///
/// `ptr` must be null or a NUL-terminated string owned by the plugin.
unsafe fn read_c_string(ptr: *const std::ffi::c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the caller's contract, plus the null check above.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

/// The directories OFX plugins live in, per docs/12 §2.6: the standard
/// location for the platform, plus anything in `OFX_PLUGIN_PATH`.
#[must_use]
pub fn search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    #[cfg(target_os = "windows")]
    paths.push(PathBuf::from(r"C:\Program Files\Common Files\OFX\Plugins"));
    #[cfg(target_os = "macos")]
    paths.push(PathBuf::from("/Library/OFX/Plugins"));
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    paths.push(PathBuf::from("/usr/OFX/Plugins"));

    if let Some(extra) = std::env::var_os("OFX_PLUGIN_PATH") {
        paths.extend(std::env::split_paths(&extra));
    }
    paths
}

/// How far below a search path a bundle is looked for.
///
/// **Not nought, which is what this used to be.** The OFX plugin path is
/// searched *recursively*, and vendors rely on it: Red Giant Universe installs
/// into `OFX/Plugins/Red Giant Universe/`, Magic Bullet into
/// `OFX/Plugins/Magic Bullet Suite/`, and Boris and Sapphire do the same. A
/// scan that read only the top of the folder found none of them — a machine
/// with a hundred plugins on it offered nothing at all.
///
/// Four levels is more than any installer uses and is also the floor under a
/// folder that contains itself: this walk follows directories, and a symbolic
/// link back up one would otherwise never end.
const MAX_BUNDLE_DEPTH: usize = 4;

/// The bundle binaries at or below one directory:
/// `**/*.ofx.bundle/Contents/<arch>/*.ofx`.
///
/// Sorted, so two runs discover plugins in the same order — which is what
/// makes an effect list stable between sessions.
#[must_use]
pub fn scan_dir(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect_bundles(dir, 0, &mut found);
    found.sort();
    found
}

/// One directory's worth of the walk: the bundles in it, then the folders
/// under it, until [`MAX_BUNDLE_DEPTH`].
fn collect_bundles(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let bundle = entry.path();
        if bundle
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".ofx.bundle"))
        {
            let binaries = bundle.join("Contents").join(BUNDLE_ARCH_DIR);
            let Ok(binaries) = std::fs::read_dir(binaries) else {
                continue;
            };
            for binary in binaries.flatten() {
                let path = binary.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("ofx") {
                    found.push(path);
                }
            }
            // A bundle is not looked inside for another bundle: what is in
            // there is one plugin's own business.
            continue;
        }
        if depth < MAX_BUNDLE_DEPTH && bundle.is_dir() {
            collect_bundles(&bundle, depth + 1, found);
        }
    }
}

/// Every bundle binary in every search path.
#[must_use]
pub fn discover() -> Vec<PathBuf> {
    search_paths()
        .iter()
        .flat_map(|dir| scan_dir(dir))
        .collect()
}
