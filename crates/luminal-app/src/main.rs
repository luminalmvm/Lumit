//! Luminal — entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use luminal_ui::Shell;

const STORAGE_KEY: &str = "luminal.shell";

struct LuminalApp {
    shell: Shell,
}

impl LuminalApp {
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

impl eframe::App for LuminalApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shell.ui(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, &self.shell);
    }
}

/// The GPU backend Luminal drives, per K-011: DX12 on Windows, Metal on macOS
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

fn main() -> eframe::Result<()> {
    // Boot begins as the splash card (K-008): small, frameless, centred; the
    // same window expands into the application when the boot log completes.
    let mut options = eframe::NativeOptions {
        centered: true,
        persist_window: false,
        viewport: egui::ViewportBuilder::default()
            .with_title("Luminal")
            .with_inner_size([460.0, 300.0])
            .with_min_inner_size([460.0, 300.0])
            .with_decorations(false)
            .with_resizable(false)
            .with_app_id("luminal"),
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
        "luminal",
        options,
        Box::new(|cc| Ok(Box::new(LuminalApp::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::default_backends;
    use eframe::wgpu::Backends;

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
