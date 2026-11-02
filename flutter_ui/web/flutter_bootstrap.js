{{flutter_js}}
{{flutter_build_config}}

// The engine runs on this page's one thread. The bridge hands each call to
// what it takes for a worker, and this one runs the call straight away
// (crates/lumit-bridge/src/api/handler.rs).
globalThis.lumitOnThePage = {
  postMessage(data) {
    const [payload, ...transfer] = data;
    wasm_bindgen.receive_transfer_closure(payload, transfer);
  },
};

// Lumit draws with WebGPU. Without it there is no Viewer, so say so before
// anything is downloaded.
if (!navigator.gpu) {
  document.getElementById('lumit-no-webgpu').style.display = 'flex';
} else {
  _flutter.loader.load({
    config: {
      // Every browser that can run WebAssembly with garbage collection gets
      // the WebAssembly build, not only Chromium.
      wasmAllowList: { blink: true, gecko: true, webkit: true },
      // Fonts for scripts Lumit doesn't carry come from this site, never from
      // a font service.
      fontFallbackBaseUrl: 'fonts/fallback/',
    },
  });
}
