//! Resolve borrows the Viewer's existing renderer and frame tiers. No renderer,
//! frame cache or project snapshot is owned by this transport.
use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::Duration,
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        System::Memory::{
            CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_WRITE,
            MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
        },
    },
};

// One transient delivery surface per connection, sized to the current preview.
// The next request acknowledges that the client has finished reading it.
struct Transfer {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    name: String,
    size: usize,
}
impl Transfer {
    fn new(size: usize) -> std::io::Result<Self> {
        let name = format!(
            "Local\\LumitPreview-{}",
            lumit_peer::Token::generate()
                .map_err(std::io::Error::other)?
                .as_name()
        );
        let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: a private, pagefile-backed section of exactly `size` bytes;
        // the handle and mapped view stay owned until Drop.
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                size as u32,
                PCWSTR(wide.as_ptr()),
            )
        }
        .map_err(std::io::Error::other)?;
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_WRITE, 0, 0, size) };
        if view.Value.is_null() {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self {
            handle,
            view,
            name,
            size,
        })
    }
}
impl Drop for Transfer {
    fn drop(&mut self) {
        // SAFETY: both resources belong exclusively to this connection.
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}

pub(crate) enum Preview {
    Memory(u32, u32, Vec<u8>),
    Shared(u32, u32, u64),
}

pub(crate) struct Request {
    pub composition: String,
    pub seconds: f64,
    pub gpu: bool,
    pub end: Option<f64>,
    pub reply: mpsc::SyncSender<Result<Preview, String>>,
}

pub(super) fn frame(
    state: &mut WorkerState,
    name: &str,
    seconds: f64,
    gpu: bool,
    end: Option<f64>,
) -> Result<Preview, String> {
    let document = measure_document(&state.project).ok_or("The Lumit project closed.")?;
    let matches: Vec<_> = document
        .items
        .iter()
        .filter_map(|item| match item {
            lumit_core::model::ProjectItem::Composition(c)
                if c.id.to_string() == name || c.name == name =>
            {
                Some(c)
            }
            _ => None,
        })
        .collect();
    let [comp] = matches.as_slice() else {
        return Err("Choose one uniquely named Lumit composition.".into());
    };
    if let Some(end) = end {
        if !end.is_finite() || end <= 0.0 || end > 1e9 {
            return Err("Invalid Resolve clip end.".into());
        }
        let duration =
            lumit_core::time::Rational::new((end * 705_600_000.0).round() as i64, 705_600_000)
                .map_err(|e| e.to_string())?;
        if comp.duration.0 != duration {
            CompositionReference {
                project: state.project.id,
                id: comp.id,
            }
            .commit(lumit_core::ops::Op::SetCompSettings {
                comp: comp.id,
                name: comp.name.clone(),
                width: comp.width,
                height: comp.height,
                frame_rate: comp.frame_rate,
                duration: lumit_core::time::Duration(duration),
                background: comp.background,
            })
            .map_err(|e| e.to_string())?;
            // Re-read the edited document once, then use the ordinary viewer path.
            return frame(state, name, seconds, gpu, None);
        }
    }
    if !seconds.is_finite() {
        return Err("Invalid frame time.".into());
    }
    if seconds < 0.0 || seconds >= comp.duration.0.to_f64() {
        return Ok(Preview::Memory(1, 1, vec![0; 4]));
    }
    // Hold the frame containing this moment when Resolve and Lumit use
    // different rates. The tiny tolerance covers the host's floating time.
    let frame = (seconds * comp.frame_rate.fps() + 1e-7).floor() as u64;
    state.last_request = std::time::Instant::now();
    let id = comp.id;
    let scale = state.resolve_scales.get(&id).copied().unwrap_or(1.0);
    let view = state.serve_active();
    look_at(state, view);
    let document = viewed_through(state.prefix, state.camera_view(), document);
    state.last_shown = Some((
        view,
        CompositionReference {
            project: state.project.id,
            id,
        },
        frame,
        scale,
    ));
    state.fill_exhausted = false;
    let bgra = zero_copy_wants_bgra();
    let prepared = prepare_frame(
        state,
        &document,
        id,
        frame,
        still_quality(scale),
        bgra,
        true,
    )?;
    let (width, height) = prepared.size();
    if u64::from(width) * u64::from(height) * 4 > 256 << 20 {
        return Err("Preview exceeds the transfer limit.".into());
    }
    #[cfg(feature = "shared-texture")]
    if gpu {
        // A delivery surface in the existing viewer pool, independent of the
        // visible view. No second renderer or frame cache. The client finishes
        // its GPU read before requesting another frame.
        let shared = state.renderer.present_prepared(&prepared, u32::MAX)?;
        return Ok(Preview::Shared(width, height, shared.handle));
    }
    if gpu {
        return Err("This Lumit build does not support GPU frame delivery.".into());
    }
    let (_, _, mut pixels) = state
        .renderer
        .read_back_prepared(&prepared)
        .ok_or("Could not read the Lumit preview.")?;
    if bgra {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    Ok(Preview::Memory(width, height, pixels))
}

fn text(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut size = [0; 4];
    stream.read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    if size > 32768 {
        return Err(std::io::Error::other("Text exceeds the protocol limit."));
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(std::io::Error::other)
}

fn serve(mut stream: TcpStream, project: ProjectReference, token: String) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    if text(&mut stream)? != token {
        return Err(std::io::Error::other("Invalid link token."));
    }
    let mut word = [0; 4];
    stream.read_exact(&mut word)?;
    if u32::from_le_bytes(word) != 3 {
        return Err(std::io::Error::other("Update Lumit and the OFX together."));
    }
    let mut transfer: Option<Transfer> = None;
    loop {
        // Probe before consuming a packet, so an idle timeout cannot discard
        // half a length prefix and desynchronise the next frame request.
        let mut probe = [0];
        match stream.peek(&mut probe) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) && project.state().is_ok() =>
            {
                continue
            }
            Err(e) => return Err(e),
        }
        stream.read_exact(&mut word)?;
        let command = u32::from_le_bytes(word);
        if command == 1 {
            let document = measure_document(&project)
                .ok_or_else(|| std::io::Error::other("Project closed."))?;
            let mut payload = Vec::new();
            let mut count = 0;
            for item in &document.items {
                if let lumit_core::model::ProjectItem::Composition(comp) = item {
                    payload.extend_from_slice(comp.id.to_string().as_bytes());
                    payload.extend_from_slice(&(comp.name.len() as u32).to_le_bytes());
                    payload.extend_from_slice(comp.name.as_bytes());
                    count += 1;
                    if payload.len() > 1 << 20 {
                        return Err(std::io::Error::other("Composition list is too large."));
                    }
                }
            }
            for value in [
                0x33524d4c,
                2,
                count,
                0,
                payload.len() as u32,
                payload.len() as u32,
            ] {
                stream.write_all(&value.to_le_bytes())?;
            }
            stream.write_all(&payload)?;
            continue;
        }
        if !matches!(command, 0 | 2 | 4 | 5) {
            return Err(std::io::Error::other("Unknown preview request."));
        }
        let composition = text(&mut stream)?;
        let mut time = [0; 8];
        stream.read_exact(&mut time)?;
        let end = if command == 4 || command == 5 {
            let mut bytes = [0; 8];
            stream.read_exact(&mut bytes)?;
            Some(f64::from_le_bytes(bytes))
        } else {
            None
        };
        let (reply, receiver) = mpsc::sync_channel(1);
        {
            let owner = project.state().map_err(std::io::Error::other)?;
            let owner = owner
                .read()
                .map_err(|_| std::io::Error::other("Project lock failed."))?;
            let sender = owner
                .sender
                .as_ref()
                .ok_or_else(|| std::io::Error::other("Preview worker stopped."))?;
            sender
                .send(WorkerRequest::ResolveFrame(Request {
                    composition,
                    seconds: f64::from_le_bytes(time),
                    gpu: command == 2 || command == 5,
                    end,
                    reply,
                }))
                .map_err(|_| std::io::Error::other("Preview worker stopped."))?;
        }
        let result = receiver
            .recv_timeout(Duration::from_secs(10))
            .map_err(std::io::Error::other)?;
        let (status, width, height, bytes) = match result {
            Ok(Preview::Memory(w, h, bytes)) => (0u32, w, h, bytes),
            Ok(Preview::Shared(w, h, handle)) => (3u32, w, h, handle.to_le_bytes().to_vec()),
            Err(error) => (1, 0, 0, error.into_bytes()),
        };
        let payload = if status == 0 {
            if transfer.as_ref().is_none_or(|t| t.size != bytes.len()) {
                transfer = Some(Transfer::new(bytes.len())?);
            }
            let surface = transfer
                .as_ref()
                .ok_or_else(|| std::io::Error::other("No delivery surface."))?;
            // SAFETY: this section has exactly the validated frame size. The
            // client asks again only after copying the previous frame out.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    surface.view.Value.cast::<u8>(),
                    bytes.len(),
                );
            }
            surface.name.as_bytes()
        } else {
            &bytes
        };
        for value in [
            0x33524d4c,
            status,
            width,
            height,
            if status == 3 {
                width * height * 4
            } else {
                bytes.len() as u32
            },
            payload.len() as u32,
        ] {
            stream.write_all(&value.to_le_bytes())?;
        }
        stream.write_all(payload)?;
    }
}

pub(super) fn start(project: ProjectReference) {
    // Only this user's loopback endpoint is published. Connections never keep
    // the project's worker sender alive while idle, so closing a project frees
    // its renderer normally.
    std::thread::spawn(move || {
        let run = || -> std::io::Result<()> {
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
            listener.set_nonblocking(true)?;
            let folder = std::path::PathBuf::from(
                std::env::var_os("LOCALAPPDATA")
                    .ok_or_else(|| std::io::Error::other("No local app data folder."))?,
            )
            .join("Lumit/Resolve Preview");
            std::fs::create_dir_all(&folder)?;
            let path = folder.join(format!("{}-{}.txt", std::process::id(), project.id));
            let token = lumit_peer::Token::generate()
                .map_err(std::io::Error::other)?
                .as_name();
            let mut published = String::new();
            let connections = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            while let Ok(owner) = project.state() {
                let saved = owner
                    .read()
                    .ok()
                    .and_then(|s| s.path.clone().map(|path| (path, s.store.snapshot().id)));
                drop(owner);
                if let Some((saved, identity)) = saved {
                    let contents = format!(
                        "{}\n{}\n{}\n{}\n{}\n",
                        listener.local_addr()?.port(),
                        token,
                        std::process::id(),
                        saved.display(),
                        identity
                    );
                    if contents != published {
                        std::fs::write(&path, &contents)?;
                        // One last-known location per document, retained when the editor
                        // closes. Live connections still use the process-scoped record.
                        let location = folder.join(format!("{identity}.location"));
                        let previous = std::fs::read_to_string(&location).unwrap_or_default();
                        let mut paths = vec![saved.to_string_lossy().into_owned()];
                        for path in previous.lines() {
                            if !path.is_empty() && !paths.iter().any(|p| p == path) {
                                paths.push(path.to_owned());
                            }
                        }
                        std::fs::write(location, paths.join("\n"))?;
                        published = contents;
                    }
                }
                match listener.accept() {
                    Ok((stream, _))
                        if connections.load(std::sync::atomic::Ordering::Relaxed) < 8 =>
                    {
                        let (project, token, count) =
                            (project.clone(), token.clone(), connections.clone());
                        count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        std::thread::spawn(move || {
                            let _ = serve(stream, project, token);
                            count.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        });
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50))
                    }
                    Err(e) => return Err(e),
                }
            }
            let _ = std::fs::remove_file(path);
            Ok(())
        };
        if let Err(error) = run() {
            note!("Resolve preview endpoint: {error}");
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    fn frame(
        state: &mut WorkerState,
        name: &str,
        seconds: f64,
    ) -> Result<(u32, u32, Vec<u8>), String> {
        match super::frame(state, name, seconds, false, None)? {
            Preview::Memory(w, h, bytes) => Ok((w, h, bytes)),
            Preview::Shared(..) => Err("Expected CPU pixels".into()),
        }
    }
    #[test]
    fn resolve_reads_the_viewers_frame_and_half_resolution_cache() {
        let _cache = crate::framecache::test_guard();
        let (project, comp) = super::super::tests::project_with_solid_of(30);
        let mut state = super::super::tests::worker_state(project.clone()).expect("GPU renderer");
        let document = measure_document(&project).expect("document");
        let prepared = prepare_frame(
            &mut state,
            &document,
            comp,
            0,
            still_quality(1.0),
            true,
            true,
        )
        .expect("viewer frame");
        let (_, _, mut expected) = state
            .renderer
            .read_back_prepared(&prepared)
            .expect("viewer pixels");
        for pixel in expected.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let hits = state.renderer.frame_texture_hits();
        let (width, height, pixels) =
            frame(&mut state, &comp.to_string(), 0.0).expect("Resolve frame");
        assert_eq!((width, height), (64, 32));
        assert_eq!(pixels, expected);
        assert!(
            state.renderer.frame_texture_hits() > hits,
            "must reuse the Viewer's texture"
        );
        state.resolve_scales.insert(comp, 0.5);
        let (width, height, _) = frame(&mut state, "Scene", 0.0).expect("half preview");
        assert_eq!((width, height), (32, 16));
        frame(&mut state, "Scene", 0.5 / 30.0).expect("between frame boundaries");
        assert_eq!(state.last_shown.as_ref().expect("anchor").2, 0);
        assert_eq!(
            frame(&mut state, "Scene", -1.0).expect("outside comp").2,
            vec![0; 4]
        );
        assert!(frame(&mut state, "missing", 0.0).is_err());
        assert!(frame(&mut state, "Scene", f64::NAN).is_err());
        #[cfg(feature = "shared-texture")]
        {
            let hits = state.renderer.frame_texture_hits();
            match super::frame(&mut state, "Scene", 0.0, true, None).expect("GPU delivery") {
                Preview::Shared(w, h, handle) => {
                    assert_eq!((w, h), (32, 16));
                    assert_ne!(handle, 0);
                }
                Preview::Memory(..) => panic!("GPU delivery read back to CPU"),
            }
            assert!(state.renderer.frame_texture_hits() > hits);
        }
        let reference = CompositionReference {
            project: project.id,
            id: comp,
        };
        let before = reference.composition().expect("composition");
        super::frame(&mut state, "Scene", -1.0, false, Some(2.0)).expect("match end");
        let after = reference.composition().expect("matched composition");
        assert_eq!(after.duration.0.to_f64(), 2.0);
        assert_eq!(
            serde_json::to_value(&before.layers).unwrap(),
            serde_json::to_value(&after.layers).unwrap()
        );
        let revision = project.state().unwrap().read().unwrap().store.revision();
        super::frame(&mut state, "Scene", -1.0, false, Some(2.0)).expect("unchanged end");
        assert_eq!(
            project.state().unwrap().read().unwrap().store.revision(),
            revision
        );
        assert!(super::frame(&mut state, "Scene", -1.0, false, Some(f64::NAN)).is_err());
        project.undo().expect("undo duration");
        assert_eq!(reference.composition().unwrap().duration, before.duration);
        project.close().expect("close");
    }
}
