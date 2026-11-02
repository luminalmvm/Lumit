//! Opening the graphics device, and in the browser build, showing a picture.

use crate::GpuError;

/// What every device is opened with: the card's own format features and
/// float filtering where it has them, and its widest texture.
pub(crate) fn descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        required_features: adapter.features()
            & (wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                | wgpu::Features::FLOAT32_FILTERABLE),
        required_limits: crate::widest_texture(adapter),
        ..Default::default()
    }
}

/// The browser's card, once [`prepare_web`] has it.
#[cfg(target_arch = "wasm32")]
struct WebDevice {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

#[cfg(target_arch = "wasm32")]
static WEB_DEVICE: std::sync::OnceLock<WebDevice> = std::sync::OnceLock::new();

/// Ask the browser for its graphics card. WebGPU answers with promises, and
/// the page's one thread can't wait for those in place, so this runs once,
/// awaited, before the first context is made. Every context then shares the
/// one device, as a browser tab would want.
#[cfg(target_arch = "wasm32")]
pub async fn prepare_web() -> Result<(), GpuError> {
    if WEB_DEVICE.get().is_some() {
        return Ok(());
    }
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::BROWSER_WEBGPU,
        ..Default::default()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        })
        .await
        .ok_or(GpuError::NoAdapter)?;
    let (device, queue) = adapter
        .request_device(&descriptor(&adapter), None)
        .await
        .map_err(|e| GpuError::Device(e.to_string()))?;
    let _ = WEB_DEVICE.set(WebDevice {
        instance,
        adapter,
        device,
        queue,
    });
    Ok(())
}

/// The adapter, device and queue [`prepare_web`] opened.
#[cfg(target_arch = "wasm32")]
pub(crate) fn web_device() -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), GpuError> {
    let web = WEB_DEVICE.get().ok_or(GpuError::NoAdapter)?;
    Ok((web.adapter.clone(), web.device.clone(), web.queue.clone()))
}

/// One Viewer view's canvas, sized to the last picture shown on it.
#[cfg(target_arch = "wasm32")]
struct Canvas {
    canvas: web_sys::OffscreenCanvas,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static CANVASES: std::cell::RefCell<Vec<(u32, Canvas)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static SHOWN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Put `picture` on view `view`'s canvas and leave it for the page as an
/// `ImageBitmap` at `globalThis.lumitViewer[view]`, where the front end takes
/// it. The picture stays on the card the whole way. Answers a number that
/// grows with each picture shown.
#[cfg(target_arch = "wasm32")]
pub fn show_on_canvas(
    ctx: &crate::GpuContext,
    picture: &wgpu::Texture,
    view: u32,
) -> Result<u64, String> {
    let web = WEB_DEVICE.get().ok_or("no graphics card")?;
    let (width, height) = (picture.width(), picture.height());
    let format = picture.format().remove_srgb_suffix();
    CANVASES.with(|canvases| {
        let mut canvases = canvases.borrow_mut();
        let at = match canvases.iter().position(|(v, _)| *v == view) {
            Some(at) => at,
            None => {
                let canvas = web_sys::OffscreenCanvas::new(width, height)
                    .map_err(|_| "no canvas".to_string())?;
                let surface = web
                    .instance
                    .create_surface(wgpu::SurfaceTarget::OffscreenCanvas(canvas.clone()))
                    .map_err(|e| e.to_string())?;
                let config = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format,
                    width,
                    height,
                    present_mode: wgpu::PresentMode::Fifo,
                    desired_maximum_frame_latency: 1,
                    alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
                    view_formats: Vec::new(),
                };
                surface.configure(&ctx.device, &config);
                canvases.push((view, Canvas { canvas, surface, config }));
                canvases.len() - 1
            }
        };
        let shown = &mut canvases[at].1;
        if (shown.config.width, shown.config.height, shown.config.format)
            != (width, height, format)
        {
            shown.canvas.set_width(width);
            shown.canvas.set_height(height);
            shown.config.width = width;
            shown.config.height = height;
            shown.config.format = format;
            shown.surface.configure(&ctx.device, &shown.config);
        }
        let target = shown.surface.get_current_texture().map_err(|e| e.to_string())?;
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("canvas") });
        encoder.copy_texture_to_texture(
            picture.as_image_copy(),
            target.texture.as_image_copy(),
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        ctx.queue.submit([encoder.finish()]);
        target.present();
        let bitmap = shown
            .canvas
            .transfer_to_image_bitmap()
            .map_err(|_| "no picture from the canvas".to_string())?;
        leave_for_the_page(view, bitmap.into());
        Ok(SHOWN.with(|n| {
            n.set(n.get() + 1);
            n.get()
        }))
    })
}

/// `globalThis.lumitViewer[view] = bitmap`, closing a picture the page never
/// took.
#[cfg(target_arch = "wasm32")]
fn leave_for_the_page(view: u32, bitmap: wasm_bindgen::JsValue) {
    use wasm_bindgen::JsCast;
    let global = js_sys::global();
    let key = wasm_bindgen::JsValue::from_str("lumitViewer");
    let shelf = match js_sys::Reflect::get(&global, &key) {
        Ok(shelf) if shelf.is_object() => shelf,
        _ => {
            let shelf: wasm_bindgen::JsValue = js_sys::Object::new().into();
            let _ = js_sys::Reflect::set(&global, &key, &shelf);
            shelf
        }
    };
    let slot = wasm_bindgen::JsValue::from(view);
    if let Ok(old) = js_sys::Reflect::get(&shelf, &slot) {
        if let Some(old) = old.dyn_ref::<web_sys::ImageBitmap>() {
            old.close();
        }
    }
    let _ = js_sys::Reflect::set(&shelf, &slot, &bitmap);
}
