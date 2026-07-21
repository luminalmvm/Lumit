//! Lumit — entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use lumit_ui::Shell;

const STORAGE_KEY: &str = "lumit.shell";

struct LumitApp {
    shell: Shell,
}

impl LumitApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let restored = cc
            .storage
            .and_then(|s| eframe::get_value::<Shell>(s, STORAGE_KEY));
        // Real GPU information for the boot log (K-008).
        let boot_notes = match cc.wgpu_render_state.as_ref() {
            Some(rs) => {
                let info = rs.adapter.get_info();
                vec![format!("GPU: {} via {:?}", info.name, info.backend)]
            }
            None => vec!["GPU: unavailable — software rendering".to_owned()],
        };
        Self {
            shell: Shell::new(
                &cc.egui_ctx,
                restored,
                boot_notes,
                #[cfg(feature = "media")]
                cc.wgpu_render_state.clone(),
            ),
        }
    }
}

impl eframe::App for LumitApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shell.ui(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, &self.shell);
    }
}

/// The GPU backend Lumit drives, per K-011: DX12 on Windows, Metal on macOS
/// (Vulkan is reserved for the future CUDA-interop path — docs/02-DECISIONS.md).
///
/// Pinning a single backend matters on Windows: eframe's default enumerates DX12,
/// Vulkan and GL together, and on a hybrid-GPU machine wgpu can settle on a device
/// that is lost on the first `Surface::present` — the window opens, then closes
/// after about a second. Choosing one backend makes adapter selection deterministic.
fn default_backends() -> eframe::wgpu::Backends {
    use eframe::wgpu::Backends;
    if cfg!(target_os = "windows") {
        Backends::DX12
    } else if cfg!(target_os = "macos") {
        Backends::METAL
    } else {
        Backends::PRIMARY
    }
}

/// The window Lumit opens with.
///
/// On Windows and macOS boot begins as the splash card itself (K-008): a small
/// frameless window, centred, which grows into the application when the boot
/// log completes.
///
/// Wayland will not have that. A client there cannot resize itself — size is
/// the compositor's to decide — and toggling resizability after the window
/// exists is unreliable, so the runtime "now become 1440×900 and resizable"
/// commands were simply ignored and the editor was stuck in a 460×300 frame
/// nothing could stretch (reported from the Flatpak). So on Linux the window
/// opens decorated, resizable and at working size from the start, and the
/// splash draws its card centred inside it — still the small centred splash
/// K-008 asks for, just not its own window.
fn splash_viewport() -> egui::ViewportBuilder {
    let base = egui::ViewportBuilder::default()
        .with_title("Lumit")
        // Must match the Flatpak's desktop file, or a Wayland compositor
        // cannot pair the window with its .desktop entry — no icon, and the
        // wrong name in the dock.
        .with_app_id(if cfg!(target_os = "linux") {
            "io.github.luminalmvm.Lumit"
        } else {
            "lumit"
        });
    if cfg!(target_os = "linux") {
        base.with_inner_size([1440.0, 900.0])
            .with_min_inner_size([720.0, 480.0])
    } else {
        base.with_inner_size([460.0, 300.0])
            .with_min_inner_size([460.0, 300.0])
            .with_decorations(false)
            .with_resizable(false)
    }
}

fn main() -> eframe::Result<()> {
    let mut options = eframe::NativeOptions {
        centered: true,
        persist_window: false,
        viewport: splash_viewport(),
        ..Default::default()
    };

    // K-011 / docs/impl/gpu-foundation.md: one high-performance adapter on the
    // platform's native backend. WGPU_BACKEND / WGPU_POWER_PREF still override,
    // for debugging and the future Vulkan path.
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends =
            eframe::wgpu::Backends::from_env().unwrap_or_else(default_backends);
        setup.power_preference = eframe::wgpu::PowerPreference::from_env()
            .unwrap_or(eframe::wgpu::PowerPreference::HighPerformance);
    }

    eframe::run_native(
        "lumit",
        options,
        Box::new(|cc| Ok(Box::new(LumitApp::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::{default_backends, splash_viewport};
    use eframe::wgpu::Backends;

    /// Regression (reported from the Flatpak): the editor opened in a 460×300
    /// frame that could not be stretched. The window was created as a
    /// non-resizable splash and only became resizable through runtime viewport
    /// commands — which Wayland ignores, a client there not being allowed to
    /// resize itself. Linux must therefore open ready to work, and carry the
    /// app id its desktop file uses so the compositor can pair the two.
    #[test]
    fn linux_opens_resizable_at_working_size_with_the_flatpak_app_id() {
        let v = splash_viewport();
        if cfg!(target_os = "linux") {
            assert_ne!(v.resizable, Some(false), "Wayland cannot unset this later");
            assert_ne!(v.decorations, Some(false));
            assert_eq!(v.inner_size, Some(egui::vec2(1440.0, 900.0)));
            assert_eq!(v.app_id.as_deref(), Some("io.github.luminalmvm.Lumit"));
        } else {
            // Elsewhere the window *is* the splash card and grows into the app.
            assert_eq!(v.resizable, Some(false));
            assert_eq!(v.decorations, Some(false));
            assert_eq!(v.inner_size, Some(egui::vec2(460.0, 300.0)));
        }
    }

    // Guards K-011: on Windows the launch backend is DX12 alone. Enumerating
    // Vulkan/GL alongside it caused intermittent device-loss on the first frame,
    // so a regression back to PRIMARY must fail here.
    #[test]
    fn default_backend_matches_k011_for_this_platform() {
        let backends = default_backends();
        if cfg!(target_os = "windows") {
            assert!(backends.contains(Backends::DX12));
            assert!(!backends.contains(Backends::VULKAN));
            assert!(!backends.contains(Backends::GL));
        } else if cfg!(target_os = "macos") {
            assert!(backends.contains(Backends::METAL));
        }
    }
}
