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

// Video and sound stay in the person's own files, so where the browser can,
// the page keeps a handle to each one it opened, by the path the engine knows
// it by, and the next page asks for it again rather than for a relink.
// Chromium has these, from its own file picker and from a drop.

// Handles to files just picked or dropped, by name and size, until media.js
// opens them.
const offered = new Map();
const nameOf = (file) => `${file.name}|${file.size}`;

addEventListener('drop', (event) => {
  for (const item of event.dataTransfer?.items ?? []) {
    const file = item.kind === 'file' && item.getAsFileSystemHandle && item.getAsFile();
    if (file) offered.set(nameOf(file), item.getAsFileSystemHandle());
  }
}, true);

// Files picked through the browser's own picker, which hands back handles,
// or null where there is none. Empty when it was cancelled. The type is
// Lumit's own, so the picker offers just these extensions.
async function pick(description, extensions) {
  if (!globalThis.showOpenFilePicker) return null;
  try {
    const accept = { 'application/x-lumit': extensions.map((e) => `.${e}`) };
    const handles = await showOpenFilePicker({ multiple: true, types: [{ description, accept }] });
    return await Promise.all(handles.map(async (handle) => {
      const file = await handle.getFile();
      offered.set(nameOf(file), handle);
      return file;
    }));
  } catch (e) {
    if (e.name === 'AbortError') return [];
    throw e;
  }
}

// Keep the handle `file` came with, if it came with one, as `path`.
async function hold(path, file) {
  const handle = await offered.get(nameOf(file));
  offered.delete(nameOf(file));
  if (base && handle?.kind === 'file') {
    base.transaction('handles', 'readwrite').objectStore('handles').put(handle, path);
  }
}

// Handles the browser will only give back once the person says so.
const asking = new Map();

// Open again the kept files the project at `project` uses that aren't open,
// where the browser still allows it. How many are left for `allow` to ask
// about.
async function reopen(project) {
  if (!base) return 0;
  const folders = wasm_bindgen.lumit_web_used([project]);
  const handles = base.transaction('handles').objectStore('handles');
  const [paths, kept] = await Promise.all([asked(handles.getAllKeys()), asked(handles.getAll())]);
  await Promise.all(paths.map(async (path, i) => {
    if (!folders.some((folder) => path.startsWith(folder)) || lumitMedia.holds(path)) return;
    try {
      if (await kept[i].queryPermission({ mode: 'read' }) === 'granted') {
        await lumitMedia.open(await kept[i].getFile(), path);
      } else {
        asking.set(path, kept[i]);
      }
    } catch {
      // Moved or deleted since. It is missing, and relinked as ever.
    }
  }));
  return asking.size;
}

// Ask for every file `reopen` couldn't open, all at once, on a click. The
// paths that came back.
async function allow() {
  const waiting = [...asking];
  asking.clear();
  const answers = await Promise.all(waiting.map(([, handle]) =>
    handle.requestPermission({ mode: 'read' }).catch(() => 'denied')));
  const back = [];
  for (const [i, [path, handle]] of waiting.entries()) {
    try {
      if (answers[i] === 'granted' && await lumitMedia.open(await handle.getFile(), path)) {
        back.push(path);
      }
    } catch {
      // Gone since. It stays missing.
    }
  }
  return back;
}

// Forget the files opened into folders `projects` don't use, and the caches.
// Settings, autosaves and journals stay.
async function forget(projects) {
  if (!base) return;
  const used = new Set(wasm_bindgen.lumit_web_used(projects));
  const unused = (path) => path.startsWith('/web/')
    && !used.has(path.slice(0, path.indexOf('/', 5) + 1));
  const stores = base.transaction(['files', 'handles'], 'readwrite');
  const done = new Promise((resolve) => { stores.oncomplete = stores.onabort = resolve; });
  for (const name of ['files', 'handles']) {
    const store = stores.objectStore(name);
    for (const path of await asked(store.getAllKeys())) {
      if (unused(path)) store.delete(path);
    }
  }
  // Gone before anyone asks how much is kept.
  await done;
  for (const name of await (globalThis.caches?.keys() ?? [])) await caches.delete(name);
}

// Put back what an earlier page kept, then keep from here on.
async function restore() {
  try {
    const opening = indexedDB.open('lumit', 2);
    opening.onupgradeneeded = () => {
      for (const name of ['files', 'handles']) {
        if (!opening.result.objectStoreNames.contains(name)) opening.result.createObjectStore(name);
      }
    };
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

globalThis.lumitFiles = { restore, used, pick, hold, reopen, allow, forget };
