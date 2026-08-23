// Renders the manual's effect example pictures, using the engine itself.
//
// In plain terms: every effect page carries a picture of what the effect does.
// Nobody draws those. The engine renders them, through the same walk the Viewer
// uses, so a figure on the website is a frame the application would actually
// make. This script is the runner around that:
//
//   1. ffmpeg turns the committed source photo into a short clip with a slow
//      pan, so effects that read motion (Motion blur, Echo, Datamosh) have
//      something to read.
//   2. `cargo test -p lumit-render --test effect_examples` renders one frame per
//      effect and writes raw RGBA8, one file each.
//   3. sharp encodes those to WebP under src/assets/effects/, where the effect
//      pages' figures look for them. They live under src/assets/ rather than
//      public/ so that Astro's own image pipeline processes them.
//
// Raw RGBA is the handover format because nothing in the Rust workspace encodes
// an image, and a throwaway encoder written for documentation tooling is exactly
// the code that should not exist.
//
// The two committed inputs are one frame of a Counter-Strike 2 demo recorded and
// rendered by the project owner: src/assets/effect-plate.jpg is the picture, and
// src/assets/effect-depth.png is the depth pass the same render produced. The
// depth pass is what makes Depth of field's figure a real one, and it stands in
// for a second layer wherever an effect reads one.
//
// Run:  npm run docs:effect-shots
//       npm run docs:effect-shots -- --keep    (leave the working files behind)

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import sharp from "sharp";

const here = dirname(fileURLToPath(import.meta.url));
const REPO = join(here, "../..");
const PLATE = join(here, "../src/assets/effect-plate.jpg");
const DEPTH = join(here, "../src/assets/effect-depth.png");
// Gitignored, and preferred when present: see step 1 below for what they buy.
const PLATE_CLIP = join(here, "../src/assets/effect-plate.mp4");
const DEPTH_CLIP = join(here, "../src/assets/effect-depth.mp4");
// Also gitignored. A real grade for the LUT page, rather than the plain warm
// cube the harness writes for itself when this is absent.
const LUT = join(here, "../src/assets/effect-lut.cube");
const OUT = join(here, "../src/assets/effects");
const WORK = join(tmpdir(), "lumit-fx-examples");

const keep = process.argv.includes("--keep");

/**
 * The clip's size and length, matching the constants in the Rust harness. The
 * render is done at the plate's native size, so an effect whose point control
 * defaults to 960, 540 lands where its author meant it to.
 */
const W = 1920;
const H = 816;
const SECONDS = 2;
const FPS = 24;

/** What the pages actually display. The encode step scales down to this. */
const FIGURE_W = 1280;

/** WebP quality. 80 is indistinguishable from source at figure size. */
const QUALITY = 80;

function run(cmd, args, label) {
  const r = spawnSync(cmd, args, { stdio: "inherit", cwd: REPO, shell: false });
  if (r.error) die(`${label}: ${r.error.message}`);
  if (r.status !== 0) die(`${label} exited ${r.status}`);
}

function die(message) {
  console.error(message);
  process.exit(1);
}

// --- 1. the source clip ----------------------------------------------------
//
// There are two ways to feed the harness, and the difference matters to about
// four of the eighty-odd pictures.
//
// The good way is a real clip: src/assets/effect-plate.mp4 and its depth twin,
// forty-eight frames of the actual recording sampled to 24 fps, with frame 24
// being the frame the manual is built around. Real footage moves, and Fast
// motion blur, Echo and Datamosh all smear motion that is already in the
// footage. Those clips are gitignored, because a couple of megabytes of video
// per file is not what a repository is for.
//
// The fallback is the committed pair of stills, panned sideways so that
// something at least moves. Everything except the temporal handful looks
// identical either way, which is why the stills are what ships.

const clipsPresent = existsSync(PLATE_CLIP);
if (!clipsPresent && !existsSync(PLATE)) {
  die(
    `no source at ${PLATE_CLIP} and none at ${PLATE}\n` +
      `Every example picture is a render of one frame. Supply either, then run\n` +
      `this again.`,
  );
}

mkdirSync(WORK, { recursive: true });
const clip = clipsPresent ? PLATE_CLIP : join(WORK, "plate.mp4");
const depthClip = clipsPresent ? DEPTH_CLIP : join(WORK, "depth.mp4");

// A slow left-to-right pan across a slightly oversized crop. The depth pass gets
// the identical filter, so the two stay registered frame for frame and Depth of
// field blurs the scene it is actually looking at.
const pan = [
  `scale=${Math.round(W * 1.15)}:${Math.round(H * 1.15)}`,
  `crop=${W}:${H}:'(iw-ow)*t/${SECONDS}':'(ih-oh)/2'`,
].join(",");

function makeClip(still, out) {
  run(
    "ffmpeg",
    ["-y", "-loglevel", "error", "-loop", "1", "-i", still, "-vf", pan,
     "-t", String(SECONDS), "-r", String(FPS), "-c:v", "libx264", "-crf", "16",
     "-pix_fmt", "yuv420p", out],
    "ffmpeg",
  );
}

let hasDepth;
if (clipsPresent) {
  console.log("using the recorded clips");
  hasDepth = existsSync(DEPTH_CLIP);
} else {
  console.log("no recorded clip; panning the committed stills instead");
  makeClip(PLATE, clip);
  hasDepth = existsSync(DEPTH);
  if (hasDepth) makeClip(DEPTH, depthClip);
}
if (!hasDepth) console.log("no depth pass; the aux layer falls back to a gradient");

// --- 2. the renders --------------------------------------------------------

const raws = join(WORK, "raw");
rmSync(raws, { recursive: true, force: true });
mkdirSync(raws, { recursive: true });

console.log("rendering one frame per effect (this takes a few minutes)");
process.env.LUMIT_FX_EXAMPLES_CLIP = clip;
process.env.LUMIT_FX_EXAMPLES_OUT = raws;
if (hasDepth) process.env.LUMIT_FX_EXAMPLES_DEPTH = depthClip;
if (existsSync(LUT)) process.env.LUMIT_FX_EXAMPLES_LUT = LUT;
run(
  "cargo",
  ["test", "-p", "lumit-render", "--release", "--test", "effect_examples",
   "--", "--ignored", "--nocapture"],
  "cargo test",
);

// --- 3. the encode ---------------------------------------------------------

/** `<name>.<w>x<h>.raw` -> `{ name, width, height }`, or null. */
function parse(file) {
  const m = /^(.+)\.(\d+)x(\d+)\.raw$/.exec(file);
  return m ? { name: m[1], width: Number(m[2]), height: Number(m[3]) } : null;
}

let written = 0;
const jobs = [];
for (const entry of readdirSync(raws, { withFileTypes: true, recursive: true })) {
  if (!entry.isFile()) continue;
  const meta = parse(entry.name);
  if (!meta) continue;
  const rel = join(entry.parentPath ?? entry.path, entry.name).slice(raws.length + 1);
  const dir = join(OUT, dirname(rel));
  mkdirSync(dir, { recursive: true });
  const raw = readFileSync(join(raws, rel));
  jobs.push(
    sharp(raw, { raw: { width: meta.width, height: meta.height, channels: 4 } })
      .resize({ width: FIGURE_W, withoutEnlargement: true })
      .webp({ quality: QUALITY })
      .toFile(join(dir, `${meta.name}.webp`))
      .then(() => {
        written += 1;
      }),
  );
}

await Promise.all(jobs);
console.log(`encoded ${written} picture(s) into ${OUT}`);

if (!keep) rmSync(WORK, { recursive: true, force: true });
