// Footage for the browser build. Each file is read here with the browser's
// own decoders, through MediaBunny (mediabunny/, MPL 2.0, version 1.65.0,
// source at github.com/Vanilagy/mediabunny), and its bytes stay here. The
// engine is handed the file's numbers and sound when it is opened and each
// frame as it is decoded (crates/lumit-bridge/src/web_media.rs), and asks for
// frames through `want`. An export's file is written here too, with the
// browser's own encoders (`exportOpen` and the rest, at the end).

import {
  ALL_FORMATS,
  AudioBufferSink,
  AudioSample,
  AudioSampleSource,
  BlobSource,
  BufferTarget,
  EncodedPacketSink,
  Input,
  Mp4OutputFormat,
  Output,
  QUALITY_HIGH,
  StreamTarget,
  VideoSample,
  VideoSampleSink,
  VideoSampleSource,
  WebMOutputFormat,
  canEncodeAudio,
  canEncodeVideo,
} from './mediabunny/mediabunny.min.mjs';

// Rates a camera records at, which a rate read off the frames is snapped to.
const RATES = [
  [24000, 1001], [24, 1], [25, 1], [30000, 1001], [30, 1], [48, 1], [50, 1],
  [60000, 1001], [60, 1], [100, 1], [120000, 1001], [120, 1], [240, 1],
];

// A frame wanted further on than this from the last one decoded starts the
// decoder again from the keyframe before it, rather than decoding up to it.
const REACH_SECONDS = 2;

// The most frames waiting to be decoded. Past it the oldest are dropped.
const MOST_WAITING = 64;

const files = new Map();
let waiting = [];
let decoding = false;

// The rate over the whole run of frames, so a clock in milliseconds still
// tells 30 from 30000/1001.
function rateOf(stamps, ticks) {
  const span = stamps[stamps.length - 1] - stamps[0];
  if (!(span > 0)) return [0, 1];
  const fps = ((stamps.length - 1) * ticks) / span;
  const off = ([n, d]) => Math.abs(fps * d / n - 1);
  const near = RATES.reduce((best, rate) => (off(rate) < off(best) ? rate : best));
  return off(near) < 0.0005 ? near : [Math.round(fps * 1000), 1000];
}

// The file's first and last 64 KiB, which the engine fingerprints it by.
async function ends(file) {
  const chunk = 65536;
  const head = new Uint8Array(await file.slice(0, chunk).arrayBuffer());
  if (file.size <= chunk * 2) return head;
  const tail = new Uint8Array(await file.slice(file.size - chunk).arrayBuffer());
  const both = new Uint8Array(head.length + tail.length);
  both.set(head);
  both.set(tail, head.length);
  return both;
}

// The whole sound as interleaved stereo at its own rate.
// ponytail: all of it in memory, about 23 MB a minute. Decode stretches as
// they are played if long files run the page out of memory.
async function stereo(track) {
  const parts = [];
  let frames = 0;
  for await (const { buffer } of new AudioBufferSink(track).buffers()) {
    parts.push(buffer);
    frames += buffer.length;
  }
  const out = new Float32Array(frames * 2);
  let at = 0;
  for (const buffer of parts) {
    const left = buffer.getChannelData(0);
    const right = buffer.getChannelData(Math.min(1, buffer.numberOfChannels - 1));
    for (let i = 0; i < buffer.length; i++) {
      out[at++] = left[i];
      out[at++] = right[i];
    }
  }
  return out;
}

// Open `file` (a File) and hand it to the engine, as `path` when it is one
// opened before. Its path, or null when the browser can decode none of it.
async function open(file, path) {
  const input = new Input({ source: new BlobSource(file), formats: ALL_FORMATS });
  if (!(await input.canRead())) return null;
  let video = await input.getPrimaryVideoTrack();
  let audio = await input.getPrimaryAudioTrack();
  if (video && !(await video.canDecode())) video = null;
  if (audio && !(await audio.canDecode())) audio = null;
  if (!video && !audio) return null;

  const fresh = path === undefined;
  path ??= `/web/${crypto.randomUUID().slice(0, 8)}/${file.name}`;
  const probe = {
    duration_seconds: await input.computeDuration(),
    container: (await input.getFormat()).name.toLowerCase(),
    video: null,
    audio: null,
  };
  let ticks = 1;
  const stamps = [];
  const keys = [];
  if (video) {
    ticks = Math.round(await video.getTimeResolution());
    const packets = new EncodedPacketSink(video).packets(undefined, undefined, { metadataOnly: true });
    const frames = [];
    for await (const packet of packets) {
      if (packet.timestamp + packet.duration <= 0) continue;
      frames.push([Math.round(packet.timestamp * ticks), packet.type === 'key' ? 1 : 0]);
    }
    frames.sort((a, b) => a[0] - b[0]);
    for (const [stamp, key] of frames) {
      stamps.push(stamp);
      keys.push(key);
    }
    const [fpsNum, fpsDen] = rateOf(stamps, ticks);
    probe.video = {
      width: await video.getDisplayWidth(),
      height: await video.getDisplayHeight(),
      fps_num: fpsNum,
      fps_den: fpsDen,
      codec: (await video.getCodec()) ?? 'unknown',
    };
  }
  if (audio) {
    probe.audio = {
      sample_rate: await audio.getSampleRate(),
      channels: await audio.getNumberOfChannels(),
      codec: (await audio.getCodec()) ?? 'unknown',
    };
  }
  const registered = wasm_bindgen.lumit_web_register(
    path, JSON.stringify(probe), file.size, Math.floor(file.lastModified / 1000),
    await ends(file), ticks, Float64Array.from(stamps), Uint8Array.from(keys));
  if (!registered) return null;
  if (audio) {
    wasm_bindgen.lumit_web_sound(path, probe.audio.sample_rate, await stereo(audio));
  }
  // Where each keyframe starts, in seconds, so a run starts exactly on one.
  const keyTimes = stamps.filter((_, i) => keys[i]).map((stamp) => stamp / ticks);
  // Decoded on the processor where it can be: each frame is read back as
  // bytes, which costs the page a third of what a frame on the graphics card
  // does.
  const soft = video && (await VideoDecoder.isConfigSupported({
    ...(await video.getDecoderConfig()), hardwareAcceleration: 'prefer-software',
  })).supported;
  const accel = soft ? 'prefer-software' : 'no-preference';
  files.set(path, { video, keyTimes, accel, runs: new Map() });
  if (fresh) await lumitFiles.hold(path, file).catch(() => {});
  return path;
}

// The engine wants frame `frame` of `path`, the one showing `at` seconds in,
// at `width` by `height`. One a render is waiting on goes first, and one
// playback will want soon (`ahead`) goes last.
function want(path, frame, at, width, height, ahead) {
  waiting = waiting.filter((w) => !(w.path === path && w.frame === frame && w.width === width));
  const wanted = { path, frame, at, width, height };
  if (ahead) waiting.push(wanted);
  else waiting.unshift(wanted);
  if (waiting.length > MOST_WAITING) waiting.length = MOST_WAITING;
  // Never inside the engine's own call: the frames land after it returns.
  if (!decoding) {
    decoding = true;
    queueMicrotask(decodeWaiting);
  }
}

async function decodeWaiting() {
  try {
    while (waiting.length) {
      const wanted = waiting.shift();
      const file = files.get(wanted.path);
      if (!file?.video) continue;
      try {
        await decode(file, wanted);
      } catch (e) {
        console.warn(`lumit: no frame ${wanted.frame} of ${wanted.path}`, e);
      }
    }
  } finally {
    decoding = false;
  }
}

// The frame as RGBA at the size wanted. The browser converts it on the way
// out when it is already that size, and a canvas scales it otherwise.
async function rgbaOf(sample, run, width, height) {
  if (sample.displayWidth === width && sample.displayHeight === height && !sample.rotation) {
    const rgba = new Uint8Array(width * height * 4);
    try {
      await sample.copyTo(rgba, { format: 'RGBA' });
      return rgba;
    } catch {
      // This browser can't convert it: draw it instead.
    }
  }
  run.canvas ??= new OffscreenCanvas(width, height);
  const context = run.canvas.getContext('2d');
  sample.drawWithFit(context, { fit: 'fill' });
  const { data } = context.getImageData(0, 0, width, height);
  return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
}

async function land(wanted, sample, run) {
  const rgba = await rgbaOf(sample, run, wanted.width, wanted.height);
  wasm_bindgen.lumit_web_frame(wanted.path, wanted.frame, wanted.width, wanted.height, rgba);
}

// Decode on from where this file was last decoded at this size when the
// frame is a little further on, and from the keyframe before it otherwise.
// Frames passed on the way that are also waiting land too.
async function decode(file, wanted) {
  const size = `${wanted.width}x${wanted.height}`;
  let run = file.runs.get(size);
  if (!run || wanted.at < run.at || wanted.at > run.at + REACH_SECONDS) {
    await run?.samples.return();
    run?.last?.close();
    const key = file.keyTimes.findLast((t) => t <= wanted.at) ?? 0;
    run = {
      sink: run?.sink ?? new VideoSampleSink(file.video, { hardwareAcceleration: file.accel }),
      canvas: run?.canvas,
      at: -Infinity,
      last: null,
    };
    run.samples = run.sink.samples(key);
    file.runs.set(size, run);
  }
  if (run.last && wanted.at < run.last.timestamp + run.last.duration) {
    await land(wanted, run.last, run);
    return;
  }
  for (;;) {
    const { value, done } = await run.samples.next();
    if (done) {
      file.runs.delete(size);
      if (run.last) await land(wanted, run.last, run);
      run.last?.close();
      return;
    }
    run.last?.close();
    run.at = value.timestamp;
    run.last = value;
    const end = value.timestamp + value.duration;
    if (wanted.at < end) {
      await land(wanted, value, run);
      return;
    }
    const passed = waiting.findIndex((w) => w.path === wanted.path && w.width === wanted.width
      && w.height === wanted.height && w.at >= value.timestamp && w.at < end);
    if (passed >= 0) await land(waiting.splice(passed, 1)[0], value, run);
  }
}

// What an export can be written as, by the export dialogue's keys: the
// container, the picture's codec, and the sound's, best first.
const EXPORTS = {
  h264: { ext: 'mp4', mime: 'video/mp4', video: 'avc', sound: ['aac', 'opus'] },
  hevc: { ext: 'mp4', mime: 'video/mp4', video: 'hevc', sound: ['aac', 'opus'] },
  vp9: { ext: 'webm', mime: 'video/webm', video: 'vp9', sound: ['opus', 'vorbis'] },
  av1: { ext: 'webm', mime: 'video/webm', video: 'av1', sound: ['opus', 'vorbis'] },
};

// The most frames handed to an encoder that it has not taken yet.
const MOST_QUEUED = 2;

// The keys this browser has an encoder for, once it has been asked.
let encodable = '';
(async () => {
  const keys = [];
  for (const [key, format] of Object.entries(EXPORTS)) {
    try {
      if (await canEncodeVideo(format.video, { width: 1920, height: 1080 })) keys.push(key);
    } catch {
      // No encoder for it.
    }
  }
  encodable = keys.join(',');
})();

const exports = new Map();
let nextExport = 1;

function exportCodecs() {
  return encodable;
}

// Where the file goes. Chromium asks, and the file is written there as it is
// made. Anywhere else, or when the browser won't ask (an export the queue
// starts later, with nobody's click behind it), it is made in memory and
// downloaded at the end.
async function target(job) {
  if (globalThis.showSaveFilePicker) {
    try {
      job.handle = await showSaveFilePicker({
        suggestedName: job.name,
        types: [{ accept: { [job.format.mime]: [`.${job.format.ext}`] } }],
      });
      const file = await job.handle.createWritable();
      // A cancelled export throws away what it wrote rather than keeping it.
      return new StreamTarget(new WritableStream({
        write: (chunk) => file.write(chunk),
        close: () => (job.cancelled ? file.abort() : file.close()),
      }), { chunked: true });
    } catch (e) {
      if (e.name === 'AbortError') throw new Error('cancelled');
    }
  }
  return new BufferTarget();
}

async function begin(job, width, height, bitRate, soundRate, soundChannels, soundBitRate) {
  job.target = await target(job);
  if (job.cancelled) return;
  const output = new Output({
    format: job.format.ext === 'mp4' ? new Mp4OutputFormat() : new WebMOutputFormat(),
    target: job.target,
  });
  job.picture = new VideoSampleSource({
    codec: job.format.video,
    bitrate: bitRate > 0 ? bitRate : QUALITY_HIGH,
  });
  output.addVideoTrack(job.picture, { frameRate: job.fpsNum / job.fpsDen });
  // The first sound codec that takes the rate asked for, or failing that its
  // own idea of a good one: Chromium's AAC refuses 320 kb/s.
  const rates = soundBitRate > 0 ? [soundBitRate, QUALITY_HIGH] : [QUALITY_HIGH];
  sound: for (const codec of soundRate > 0 ? job.format.sound : []) {
    for (const bitrate of rates) {
      const options = { numberOfChannels: soundChannels, sampleRate: soundRate, bitrate };
      if (await canEncodeAudio(codec, options)) {
        job.sound = new AudioSampleSource({ codec, bitrate });
        output.addAudioTrack(job.sound);
        break sound;
      }
    }
  }
  await output.start();
  Object.assign(job, { output, width, height, soundRate, soundChannels });
}

function fail(job, e) {
  job.error ??= e?.message || String(e);
}

// Start writing a file called `name` as `codec`, one of exportCodecs(). An
// id for the calls below.
function exportOpen(name, codec, width, height, fpsNum, fpsDen, bitRate,
  soundRate, soundChannels, soundBitRate) {
  const id = nextExport++;
  const job = {
    name: name.split(/[\\/]/).pop(),
    format: EXPORTS[codec],
    fpsNum,
    fpsDen,
    queued: 0,
    pictures: Promise.resolve(),
    sounds: Promise.resolve(),
  };
  exports.set(id, job);
  if (!job.format) {
    fail(job, `no encoder for ${codec}`);
  } else {
    begin(job, width, height, bitRate, soundRate, soundChannels, soundBitRate)
      .catch((e) => fail(job, e));
  }
  return id;
}

// 'room' for another frame, 'busy', 'done', or what went wrong.
function exportState(id) {
  const job = exports.get(id);
  if (!job) return 'the export has gone';
  if (job.error) return job.error;
  if (job.done) {
    exports.delete(id);
    return 'done';
  }
  return job.output && !job.finishing && job.queued < MOST_QUEUED ? 'room' : 'busy';
}

// Frame `n`, as RGBA. The bytes are the engine's and are copied at once.
function exportFrame(id, rgba, n) {
  const job = exports.get(id);
  if (!job?.output) return;
  const sample = new VideoSample(new Uint8Array(rgba), {
    format: 'RGBX',
    codedWidth: job.width,
    codedHeight: job.height,
    timestamp: (n * job.fpsDen) / job.fpsNum,
    duration: job.fpsDen / job.fpsNum,
  });
  job.queued++;
  job.pictures = job.pictures
    .then(() => job.picture.add(sample))
    .catch((e) => fail(job, e))
    .finally(() => {
      sample.close();
      job.queued--;
    });
}

// Interleaved sound, `first` samples a channel into the file.
function exportSound(id, samples, first) {
  const job = exports.get(id);
  if (!job?.sound) return;
  const sample = new AudioSample({
    data: new Float32Array(samples),
    format: 'f32',
    numberOfChannels: job.soundChannels,
    sampleRate: job.soundRate,
    timestamp: first / job.soundRate,
  });
  job.sounds = job.sounds
    .then(() => job.sound.add(sample))
    .catch((e) => fail(job, e))
    .finally(() => sample.close());
}

function exportFinish(id) {
  const job = exports.get(id);
  if (!job?.output) return;
  job.finishing = true;
  (async () => {
    await Promise.all([job.pictures, job.sounds]);
    if (job.error) return;
    await job.output.finalize();
    if (job.target instanceof BufferTarget) {
      const url = URL.createObjectURL(new Blob([job.target.buffer], { type: job.format.mime }));
      const link = document.createElement('a');
      link.href = url;
      link.download = job.name;
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 60000);
    }
    job.done = true;
  })().catch((e) => fail(job, e));
}

// Stop, and leave no file behind.
function exportCancel(id) {
  const job = exports.get(id);
  if (!job) return;
  exports.delete(id);
  job.cancelled = true;
  (async () => {
    await job.output?.cancel();
    await job.handle?.remove?.();
  })().catch(() => {});
}

globalThis.lumitMedia = {
  open, holds: (path) => files.has(path), want, exportCodecs, exportOpen, exportState,
  exportFrame, exportSound, exportFinish, exportCancel,
};
