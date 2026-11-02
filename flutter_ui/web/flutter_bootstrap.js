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
const loading = document.getElementById('lumit-loading');
// The loading card goes once Lumit has drawn (lib/state/page_web.dart) and the
// engine has built its renderer (crates/lumit-bridge/src/turns.rs): on a first
// visit that build holds the page for a few seconds while the browser compiles
// every shader, and the card's bar keeps moving through it. A start that never
// gets a renderer still shows what it drew, after a while.
let drawn = false;
let built = false;
const fade = () => {
  if (!drawn || !built || !loading.isConnected) return;
  loading.classList.add('gone');
  setTimeout(() => loading.remove(), 250);
};
globalThis.lumitReady = () => {
  drawn = true;
  fade();
  setTimeout(() => { built = true; fade(); }, 20000);
};
globalThis.lumitEngineReady = () => {
  built = true;
  fade();
};

if (!navigator.gpu) {
  loading.remove();
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
