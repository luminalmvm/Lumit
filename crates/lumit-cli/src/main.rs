//! `lumit-cli`, Lumit on the command line.
//!
//! The command line lives in the bridge library the app already ships, so
//! this program only finds that library beside it, runs its
//! `lumit_cli_main` and exits with the status it answers. The library reads
//! the arguments itself.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The bridge library's file name on this system.
fn library_name() -> String {
    use std::env::consts::{DLL_PREFIX, DLL_SUFFIX};
    format!("{DLL_PREFIX}lumit_bridge{DLL_SUFFIX}")
}

/// Where the library may be, from the folder this program is in: beside it
/// (Windows' lib\, and a build's target folder), in lib/ below it (Linux),
/// and in the app's Frameworks (macOS).
fn places(dir: &Path) -> [PathBuf; 3] {
    let name = library_name();
    [
        dir.join(&name),
        dir.join("lib").join(&name),
        dir.join("..")
            .join("Frameworks")
            .join("lumit_bridge.framework")
            .join("lumit_bridge"),
    ]
}

fn main() {
    let code = run().unwrap_or_else(|why| {
        let _ = writeln!(std::io::stderr(), "lumit-cli: {why}");
        1
    });
    std::process::exit(code);
}

#[allow(unsafe_code)]
fn run() -> Result<i32, String> {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .ok_or("Could not tell where lumit-cli is.")?;
    let path = places(&dir)
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| format!("{} was not found beside lumit-cli.", library_name()))?;
    // SAFETY: it is Lumit's own library, shipped with this program, and
    // loading it runs nothing but its own start-up.
    let library = unsafe { libloading::Library::new(&path) }
        .map_err(|e| format!("{} could not be loaded: {e}", path.display()))?;
    // SAFETY: the bridge declares `lumit_cli_main` with exactly this type.
    let code = unsafe { library.get::<extern "C" fn() -> i32>(b"lumit_cli_main") }
        .map(|main| main())
        .map_err(|e| format!("{} has no command line: {e}", path.display()))?;
    // Kept loaded until the process ends, as threads it started may still
    // be winding down.
    std::mem::forget(library);
    Ok(code)
}
