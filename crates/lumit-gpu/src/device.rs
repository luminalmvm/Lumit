//! Opening the graphics device, and in the browser build, showing a picture.

#[cfg(target_arch = "wasm32")]
use crate::GpuError;

/// What every device is opened with: the card's own format features and
/// float filtering where it has them, and its widest texture.
pub(crate) fn descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        required_features: adapter.features()
            & (wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                | wgpu::Features::FLOAT32_FILTERABLE),
        required_limits: widest_texture(adapter),
        ..Default::default()
    }
}

/// wgpu's guaranteed limits, with the widest 2D texture the adapter takes, a
/// buffer big enough to read one back, and a binding as large as it allows for
/// the posing of large models.
pub(crate) fn widest_texture(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let card = adapter.limits();
    wgpu::Limits {
        max_texture_dimension_2d: card.max_texture_dimension_2d,
        max_buffer_size: card.max_buffer_size,
        max_storage_buffer_binding_size: card.max_storage_buffer_binding_size,
        ..Default::default()
    }
}

/// An error in the browser build, where nothing may wait for the card: a copy
/// lands only once the page's event loop runs, so waiting for one in place
/// never ends. Callers answer it as they answer any failed read-back. Nothing
/// on the desktop, which waits.
pub fn wait_here() -> Result<(), crate::GpuError> {
    if cfg!(target_arch = "wasm32") {
        return Err(crate::GpuError::Readback(
            "the browser build reads back between turns".into(),
        ));
    }
    Ok(())
}

/// The bytes of `buffer`, which the caller has copied into and submitted. The
/// desktop waits for them. The browser build answers with the last copy that
/// landed for `key` ([`read`]), and `None` before one has.
pub fn read_buffer<T: bytemuck::Pod>(
    ctx: &crate::GpuContext,
    key: impl std::hash::Hash,
    buffer: &wgpu::Buffer,
) -> Option<Vec<T>> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ctx;
        let bytes = read::buffer(key, buffer)?;
        let mut out = vec![T::zeroed(); bytes.len() / std::mem::size_of::<T>()];
        let whole = std::mem::size_of_val(out.as_slice());
        bytemuck::cast_slice_mut(&mut out).copy_from_slice(bytes.get(..whole)?);
        Some(out)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        ctx.device.poll(wgpu::Maintain::Wait);
        rx.recv().ok()?.ok()?;
        let out = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
        buffer.unmap();
        Some(out)
    }
}

/// [`crate::ColourEngine::readback8`], or in the browser build the last copy of
/// a texture this size that landed at `place` ([`read`]).
pub fn read_texture8(
    ctx: &crate::GpuContext,
    colour: &crate::ColourEngine,
    place: &str,
    tex: &wgpu::Texture,
) -> Result<Vec<u8>, crate::GpuError> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = colour;
        read::texture(ctx, (place, tex.width(), tex.height()), tex)
            .ok_or_else(|| crate::GpuError::Readback("nothing has landed yet".into()))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = place;
        colour.readback8(ctx, tex)
    }
}

/// How many read-backs a frame stood in for have since landed with a new
/// answer, in the browser build ([`read`]). A picture drawn before this moved
/// is worth drawing again. Nought on the desktop.
#[must_use]
pub fn reads_landed() -> u64 {
    #[cfg(target_arch = "wasm32")]
    return read::landed();
    #[cfg(not(target_arch = "wasm32"))]
    0
}

/// How many times a frame has drawn from an earlier read-back than its own,
/// in the browser build. A picture drawn while this moved is never kept.
/// Nought on the desktop.
#[must_use]
pub fn read_stand_ins() -> u64 {
    #[cfg(target_arch = "wasm32")]
    return read::stand_ins();
    #[cfg(not(target_arch = "wasm32"))]
    0
}

#[cfg(target_arch = "wasm32")]
pub mod read;

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
                canvases.push((
                    view,
                    Canvas {
                        canvas,
                        surface,
                        config,
                    },
                ));
                canvases.len() - 1
            }
        };
        let shown = &mut canvases[at].1;
        if (shown.config.width, shown.config.height, shown.config.format) != (width, height, format)
        {
            shown.canvas.set_width(width);
            shown.canvas.set_height(height);
            shown.config.width = width;
            shown.config.height = height;
            shown.config.format = format;
            shown.surface.configure(&ctx.device, &shown.config);
        }
        let target = shown
            .surface
            .get_current_texture()
            .map_err(|e| e.to_string())?;
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("canvas"),
            });
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
