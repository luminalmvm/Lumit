// Lumit's files, kept in this browser's own storage on this computer so they
// outlast the page. Nothing is sent anywhere. The engine holds them in memory
// (crates/lumit-ingress/src/files.rs). Each second the ones that changed are
// copied in here, and the next page puts them all back before it reads any
// (installFiles in flutter_ui/lib/state/files_web.dart).

let base = null;

function asked(request) {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

// Copy what changed since the last time into storage.
function keep() {
  const changed = wasm_bindgen.lumit_web_changed();
  if (!base || !changed.length) return;
  const files = base.transaction('files', 'readwrite').objectStore('files');
  for (const path of changed) {
    const bytes = wasm_bindgen.lumit_web_file(path);
    if (bytes) files.put(bytes, path);
    else files.delete(path);
  }
}

// Put back what an earlier page kept, then keep from here on.
async function restore() {
  try {
    const opening = indexedDB.open('lumit', 1);
    opening.onupgradeneeded = () => opening.result.createObjectStore('files');
    base = await asked(opening);
    const files = base.transaction('files').objectStore('files');
    const [paths, contents] = await Promise.all([asked(files.getAllKeys()), asked(files.getAll())]);
    paths.forEach((path, i) => wasm_bindgen.lumit_web_restore(path, contents[i]));
  } catch (e) {
    // No storage here, as in some private windows. The files last as long as
    // the page.
    console.warn("Lumit can't keep files in this browser:", e);
    base = null;
  }
  wasm_bindgen.lumit_web_changed();
  setInterval(keep, 1000);
  addEventListener('pagehide', keep);
  // Asked once, and the browser answers without asking the person, or asks
  // them once.
  if (!localStorage.getItem('lumit-kept')) {
    localStorage.setItem('lumit-kept', '1');
    navigator.storage?.persist?.();
  }
}

// How much this browser keeps for Lumit, in bytes, or null when it won't say.
async function used() {
  return (await navigator.storage?.estimate?.())?.usage ?? null;
}

globalThis.lumitFiles = { restore, used };
