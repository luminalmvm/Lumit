// The browser build's sound card. The engine mixes a little ahead on the
// page's turns (crates/lumit-audio/src/web.rs) and writes it into a ring here,
// and an audio worklet plays from the ring. The two share the ring and two
// counts: the frames written, and the frames played. This file is both halves:
// the page loads it, and the worklet loads it again.

// Frames the ring holds, a power of two. Far more than the engine keeps ahead.
const RING_FRAMES = 1 << 15;
const MASK = RING_FRAMES - 1;
const WRITTEN = 0;
const PLAYED = 1;

if (typeof registerProcessor === 'function') {
  registerProcessor('lumit-sound', class extends AudioWorkletProcessor {
    constructor(options) {
      super();
      ({ ring: this.ring, counts: this.counts } = options.processorOptions);
    }

    // Whatever is not in the ring yet is silence, and the count waits for it.
    process(_inputs, [[left, right]]) {
      const played = Atomics.load(this.counts, PLAYED);
      const ready = (Atomics.load(this.counts, WRITTEN) - played) | 0;
      const frames = Math.max(0, Math.min(left.length, ready));
      for (let i = 0; i < frames; i++) {
        const at = ((played + i) & MASK) * 2;
        left[i] = this.ring[at];
        right[i] = this.ring[at + 1];
      }
      Atomics.store(this.counts, PLAYED, played + frames);
      return true;
    }
  });
} else {
  let context = null;
  let running = false;
  let ring;
  let counts;
  let checked = 0;

  // The output, and its rate. Browsers keep a page's sound off until
  // somebody presses something, so every press until then turns it on.
  function open() {
    if (context) return context.sampleRate;
    context = new AudioContext({ latencyHint: 'interactive' });
    ring = new Float32Array(new SharedArrayBuffer(RING_FRAMES * 8));
    counts = new Int32Array(new SharedArrayBuffer(8));
    context.audioWorklet.addModule(import.meta.url).then(() => {
      const node = new AudioWorkletNode(context, 'lumit-sound', {
        numberOfInputs: 0,
        outputChannelCount: [2],
        processorOptions: { ring, counts },
      });
      node.connect(context.destination);
      running = context.state === 'running';
      context.onstatechange = () => { running = context.state === 'running'; };
    }).catch(() => {});
    const press = () => { if (context.state !== 'running') context.resume().catch(() => {}); };
    addEventListener('pointerdown', press, true);
    addEventListener('keydown', press, true);
    return context.sampleRate;
  }

  // Frames written and not played yet. While nothing plays them, they are
  // counted as played as time passes, so a picture that follows the sound
  // still moves.
  function queued() {
    const now = performance.now();
    if (!running) {
      const due = Math.round(((now - checked) * context.sampleRate) / 1000);
      const ready = Math.max(0, (Atomics.load(counts, WRITTEN) - Atomics.load(counts, PLAYED)) | 0);
      Atomics.add(counts, PLAYED, Math.min(due, ready));
    }
    checked = now;
    return Math.max(0, (Atomics.load(counts, WRITTEN) - Atomics.load(counts, PLAYED)) | 0);
  }

  function write(samples) {
    let at = Atomics.load(counts, WRITTEN);
    for (let i = 0; i + 1 < samples.length; i += 2, at++) {
      const to = (at & MASK) * 2;
      ring[to] = samples[i];
      ring[to + 1] = samples[i + 1];
    }
    Atomics.store(counts, WRITTEN, at);
  }

  function flush() {
    Atomics.store(counts, WRITTEN, Atomics.load(counts, PLAYED));
  }

  globalThis.lumitSound = { open, queued, write, flush };
}
