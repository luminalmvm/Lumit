// Builds the manual's Effects section from the engine's own catalogue.
//
// In plain terms: every effect's parameters - their names, ranges, defaults and
// units - are written down once, in Rust, on the effect's declaration. The
// engine exports that as crates/lumit-core/fx-reference.json (regenerate it with
// `cargo test -p lumit-core regenerate_fx_reference -- --ignored`). This script
// turns the JSON into one page per effect and one page per category.
//
// It only ever rewrites the block between the GENERATED markers. Everything
// else on a page - the front matter, the prose about what the effect is for,
// the notes on what each control does - is hand-written and is never touched.
// A page that does not exist yet is scaffolded whole, markers included.
//
// Run:  npm run docs:effects        (from web-docs/)
//       npm run docs:effects -- --check   (fails if anything is out of date)

import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const REFERENCE = join(here, "../../crates/lumit-core/fx-reference.json");
const OUT = join(here, "../src/content/docs/effects");
// Where gen-effect-shots.mjs puts the example pictures.
const SHOTS = join(here, "../public/effects");

// MDX has no HTML comments - `{/* ... */}` is the expression-comment that
// compiles to nothing. The opening marker is found by its `GENERATED:<tag>`
// prefix alone, so the sentence after it can be reworded without stranding
// every page that already carries the old wording.
const begin = (tag) =>
  `{/* GENERATED:${tag} - rewritten by \`npm run docs:effects\`. Edit the prose, not this block. */}`;
const END = "{/* END GENERATED */}";

const check = process.argv.includes("--check");

/** A number as the manual writes it. */
const n = (v) => String(v);

const CONTROL = {
  float: "Slider",
  int: "Slider, whole numbers",
  angle: "Dial",
  choice: "Dropdown",
  bool: "Checkbox",
  colour: "Colour",
  seed: "Seed",
  file: "File",
  layer: "Layer",
  mask_path: "Masks",
  curve: "Curve",
};

const UNIT = {
  raw: "-",
  pct_diag: "Per cent of the composition diagonal",
  px: "Pixels at composition size",
  degrees: "Degrees",
  seconds: "Seconds",
};

/** Escape the characters that would break out of a Markdown table cell. */
const cell = (s) => String(s).replaceAll("|", "\\|").replaceAll("\n", " ");

function rangeCell(p) {
  switch (p.kind) {
    case "float":
    case "int": {
      const s = `${n(p.slider_min)} to ${n(p.slider_max)}`;
      if (p.hard_min === undefined && p.hard_max === undefined) return `${s}; any value by typing`;
      const past = [];
      if (p.hard_min === undefined) past.push("lower");
      else if (p.hard_min < p.slider_min) past.push(`down to ${n(p.hard_min)}`);
      if (p.hard_max === undefined) past.push("higher");
      else if (p.hard_max > p.slider_max) past.push(`up to ${n(p.hard_max)}`);
      return past.length ? `${s}; ${past.join(" and ")}` : s;
    }
    case "angle":
      return `Any angle; snaps every ${n(p.dial_step)}\u00b0 while a modifier is held`;
    case "choice":
      return p.options.join(" \u00b7 ");
    case "bool":
      return "On or off";
    case "colour":
      return `Each channel ${n(p.slider_min)} to ${n(p.slider_max)}, scene-linear`;
    case "seed":
      return "Any whole number";
    case "file":
      return p.file_filter.map((e) => `\`.${e}\``).join(", ");
    case "layer":
      return "Any layer in the composition";
    case "mask_path":
      return "This layer's masks";
    case "curve":
      return "2 to 16 points in the unit square";
    default:
      return "-";
  }
}

function defaultCell(p) {
  switch (p.kind) {
    case "float":
    case "int":
      return n(p.default);
    case "angle":
      return `${n(p.default)}\u00b0`;
    case "choice":
      return p.options[p.default] ?? "-";
    case "bool":
      return p.default ? "On" : "Off";
    case "colour":
      return p.default.map(n).join(", ");
    case "seed":
      return "A fresh random seed per instance";
    case "file":
      return "None";
    case "layer":
      return p.self_default ? "This layer" : "None";
    case "mask_path":
      return p.self_default ? "First mask" : "None";
    case "curve":
      return "The identity diagonal";
    default:
      return "-";
  }
}

/**
 * The name a row goes under. Normally the parameter's own label - but an
 * effect may deliberately repeat a label across its groups, because the group
 * header is what says which channel or colour range the row belongs to
 * (Curves' four "Midtones", Hue and saturation's seven "Hue"). A table has no
 * headers, so a repeated label is prefixed with its group's; a label that
 * appears once is left exactly as it was, which is why no existing page moves.
 */
function rowName(effect, p) {
  const twice = effect.params.filter((q) => q.label === p.label).length > 1;
  if (!twice) return p.label;
  const group = effect.groups.find((g) => g.label && g.params.includes(p.id));
  return group ? `${group.label} › ${p.label}` : p.label;
}

function paramTable(effect) {
  const rows = effect.params.map(
    (p) =>
      `| **${cell(rowName(effect, p))}** | ${cell(CONTROL[p.kind] ?? p.kind)} | ${cell(rangeCell(p))} | ` +
      `${cell(defaultCell(p))} | ${cell(UNIT[p.unit] ?? p.unit)} |`,
  );
  return [
    "| Parameter | Control | Range | Default | Unit |",
    "| --- | --- | --- | --- | --- |",
    ...rows,
  ].join("\n");
}

/** Sentences about the twirls and the greyed-out rows, when there are any. */
function panelNotes(effect) {
  const label = (id) => effect.params.find((p) => p.id === id)?.label ?? id;
  const list = (ids) => ids.map((i) => `**${label(i)}**`).join(", ");
  const out = [];

  for (const g of effect.groups) {
    if (!g.label) continue;
    let s = `**${g.label}** groups ${list(g.params)}`;
    s += g.collapsed ? ", and starts closed." : ".";
    if (g.visible_when) {
      const on = effect.params.find((p) => p.id === g.visible_when.param);
      const opts = g.visible_when.values.map((v) => on?.options?.[v] ?? v).join(" or ");
      s = s.slice(0, -1) + `, and appears while **${on?.label ?? g.visible_when.param}** is ${opts}.`;
    }
    out.push(`- ${s}`);
  }

  for (const e of effect.enabled_when ?? []) {
    const on = effect.params.find((p) => p.id === e.on);
    const onLabel = `**${on?.label ?? e.on}**`;
    let when;
    if (e.cond === "bool_is") when = `${onLabel} is ${e.value ? "on" : "off"}`;
    else if (e.cond === "choice_is") when = `${onLabel} is set to ${on?.options?.[e.value] ?? e.value}`;
    else if (e.cond === "choice_is_not") when = `${onLabel} is anything but ${on?.options?.[e.value] ?? e.value}`;
    else when = `${onLabel} names a layer`;
    out.push(`- **${label(e.param)}** is editable while ${when}.`);
  }

  // What this effect's Matte row means (K-395). Every effect has the row, so
  // every page says what it does: the standard strength sentence unless the
  // effect claims the matte inside its own maths, in which case the sentence
  // comes from the effect's own declaration and a page cannot describe a matte
  // the engine stopped honouring.
  const matteRow = effect.params.find((p) => p.id === (effect.matte?.param ?? "matte"));
  if (matteRow) {
    const meaning =
      effect.matte?.meaning ??
      "is an input to an effect, scaling the strength. ";
    const invert = effect.params.find((p) => p.kind === "bool" && p.label === "Invert");
    const swap = invert ? ` **${invert.label}** inverts the mattes for calculating strength.` : "";
    out.push(`- **${matteRow.label}** ${meaning}.${swap}`);
  }

  if (!out.length) return "";
  return `\n\nIn the Effect Controls panel:\n\n${out.join("\n")}`;
}

/** The whole marked block: opening marker, content, closing marker. */
const marked = (tag, body) => `${begin(tag)}\n\n${body}\n\n${END}`;

/** Replace the marked block in an existing page, or return null if it has none. */
function splice(existing, tag, block) {
  const a = existing.indexOf(`{/* GENERATED:${tag}`);
  if (a === -1) return null;
  const b = existing.indexOf(END, a);
  if (b === -1) return null;
  return existing.slice(0, a) + block + existing.slice(b + END.length);
}

/**
 * Where a block that a page does not carry yet should be inserted. A page
 * written before the block existed has no marker to splice into, and leaving it
 * behind for ever is how half a manual ends up with a figure and half without.
 * The anchor is a line the scaffold guarantees; the block lands above it.
 */
function insertAt(existing, anchor, block) {
  const at = existing.indexOf(anchor);
  if (at === -1) return null;
  return `${existing.slice(0, at)}${block}

${existing.slice(at)}`;
}

const changed = [];
const stranded = [];
function put(path, tag, scaffold, block, anchor) {
  const existing = existsSync(path) ? readFileSync(path, "utf8") : null;
  let next = scaffold;
  if (existing !== null) {
    next = splice(existing, tag, block);
    if (next === null && anchor) next = insertAt(existing, anchor, block);
    if (next === null) {
      stranded.push(path);
      return;
    }
  }
  if (existing === next) return;
  changed.push(path);
  if (!check) {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, next);
  }
}

// ---------------------------------------------------------------------------

/**
 * A page's own `description:`, which is the one-line summary its author already
 * wrote. The category tables reuse it rather than asking for a second summary
 * that would drift away from the first.
 */
function description(path) {
  if (!existsSync(path)) return "";
  const m = /^description:\s*(.*)$/m.exec(readFileSync(path, "utf8"));
  if (!m) return "";
  return m[1].trim().replace(/^["'](.*)["']$/s, "$1").trim();
}

/**
 * One category's effects as a table. The second column is each page's own
 * description, so the row says what the effect is for and not merely that it
 * exists.
 */
function effectTable(effects) {
  return [
    "| Effect | What it does |",
    "| --- | --- |",
    ...effects.map((e) => {
      const page = join(OUT, e.category_slug, `${e.slug}.mdx`);
      const link = `[${e.label}](/effects/${e.category_slug}/${e.slug}/)`;
      return `| ${cell(link)} | ${cell(description(page))} |`;
    }),
  ].join("\n");
}

/**
 * The example picture, rendered by `npm run docs:effect-shots` and served from
 * `public/`. Plain HTML rather than a component: a figure that needs an import
 * line at the top of every page is a figure that goes missing from half of
 * them, and a `public/` path costs nothing when the file is not there yet.
 */
function exampleFigure(e) {
  // A couple of effects have no picture and cannot have one: Posterize time is a
  // change to the clock, and Matte key wants a screen the example frame does not
  // contain. The harness says so and writes nothing, and an absent file means an
  // absent figure rather than a broken image.
  const file = join(SHOTS, e.category_slug, `${e.slug}.webp`);
  if (!existsSync(file)) return "";
  const size = `width="1280" height="544" loading="lazy" decoding="async"`;
  return [
    `<figure class="example">`,
    `  <div class="compare" style="--split:50%">`,
    `    <img src="/effects/plate.webp" alt="The frame before ${e.label} is applied" ${size} />`,
    `    <img class="compare__after" src="/effects/${e.category_slug}/${e.slug}.webp"`,
    `      alt="The same frame with ${e.label} applied" ${size} />`,
    `    <span class="compare__handle" aria-hidden="true"></span>`,
    `    <span class="compare__label compare__label--a" aria-hidden="true">Original</span>`,
    `    <span class="compare__label compare__label--b" aria-hidden="true">${e.label}</span>`,
    `    <input class="compare__range" type="range" min="0" max="100" step="0.1" value="50"`,
    `      aria-label="Reveal ${e.label}. Left of the handle is the original frame, the right has the ${e.label} applied." />`,
    `  </div>`,
    `  <figcaption>Drag the handle. ${e.label} applied on the right, the original frame on the left. </figcaption>`,
    `</figure>`,
  ].join("\n");
}

const ref = JSON.parse(readFileSync(REFERENCE, "utf8"));
const byCategory = new Map(ref.categories.map((c) => [c.slug, []]));
for (const e of ref.effects) byCategory.get(e.category_slug)?.push(e);

// One page per effect.
for (const e of ref.effects) {
  const table = marked("parameters", `${paramTable(e)}${panelNotes(e)}`);
  const figure = marked("example", exampleFigure(e));
  const scaffold = `---
title: ${e.label}
description: What ${e.label} does, and what each of its controls means.
---

${e.label} is a **${e.category}** effect.

${figure}

## What it does

## Parameters

${table}

## What each control does

${e.params.map((p) => `- **${p.label}**`).join("\n")}

## Related

- [${e.category}](/effects/${e.category_slug}/)
- [Effects](/use/effects/)
`;
  const path = join(OUT, e.category_slug, `${e.slug}.mdx`);
  put(path, "parameters", scaffold, table);
  put(path, "example", scaffold, figure, "## What it does");
}

// One page per category.
for (const c of ref.categories) {
  const effects = byCategory.get(c.slug) ?? [];
  const block = marked("list", effectTable(effects));
  const scaffold = `---
title: ${c.label}
description: The ${c.label} effects, and what the family is for.
sidebar:
  order: 0
---

## What this family does

## The effects

${block}

## Related

- [All effects](/effects/)
- [Effects](/use/effects/)
`;
  put(join(OUT, c.slug, "index.mdx"), "list", scaffold, block);
}

// The section index: every category, what it is for, and its own table.
{
  const body = ref.categories
    .map((c) => {
      const effects = byCategory.get(c.slug) ?? [];
      const summary = description(join(OUT, c.slug, "index.mdx"));
      const count = `${effects.length} effect${effects.length === 1 ? "" : "s"}.`;
      return [
        `### [${c.label}](/effects/${c.slug}/)`,
        summary ? `${summary} ${count}` : count,
        effectTable(effects),
      ].join("\n\n");
    })
    .join("\n\n");
  const block = marked("list", body);
  const scaffold = `---
title: Effects
description: Every built-in effect, what it is for, and what each parameter does.
sidebar:
  order: 0
---

Every effect Lumit ships with has a page here: what it is for, and a table of
every parameter with its range, its default and its unit. The tables come
straight from the engine's own catalogue, so they cannot drift from the
application.

Each page also carries a picture of the effect on one frame of footage. Every
one of those pictures is rendered by the engine, through the walk the Viewer
uses, from the frame below.

<figure class="example">
  <img src="/effects/plate.webp" alt="The untouched frame every example picture starts from"
    width="1280" height="544" loading="lazy" decoding="async" />
  <figcaption>The untouched frame. Every example on these pages is this frame with one
  effect on it.</figcaption>
</figure>

For applying effects, ordering a stack and using adjustment layers, see
[Effects](/use/effects/).

## The categories

${block}
`;
  put(join(OUT, "index.mdx"), "list", scaffold, block);
}

// Pages nobody claims any more - a renamed effect leaves its old page behind,
// and a stale page is worse than a missing one.
const wanted = new Set([
  "index.mdx",
  ...ref.categories.map((c) => `${c.slug}/index.mdx`),
  ...ref.effects.map((e) => `${e.category_slug}/${e.slug}.mdx`),
]);
const orphans = [];
if (existsSync(OUT)) {
  for (const entry of readdirSync(OUT, { withFileTypes: true, recursive: true })) {
    if (!entry.isFile()) continue;
    const rel = join(entry.parentPath ?? entry.path, entry.name)
      .slice(OUT.length + 1)
      .replaceAll("\\", "/");
    if (!wanted.has(rel)) orphans.push(rel);
  }
}

for (const o of orphans) console.warn(`orphan page (no effect claims it): ${o}`);
for (const s of stranded) console.warn(`page has no GENERATED markers, left alone: ${s}`);
if (check) {
  if (changed.length) {
    console.error(`out of date:\n${changed.map((c) => `  ${c}`).join("\n")}`);
    process.exit(1);
  }
  console.log("effect pages are up to date");
} else {
  console.log(
    changed.length ? `wrote ${changed.length} page(s)` : "no changes - pages already match the catalogue",
  );
}
