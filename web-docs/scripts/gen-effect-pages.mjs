// Builds the manual's Effects section from the engine's own catalogue.
//
// In plain terms: every effect's parameters - their names, ranges, defaults and
// units - are written down once, in Rust, on the effect's declaration. The
// engine exports that as crates/lumit-core/fx-reference.json (regenerate it with
// `cargo test -p lumit-core regenerate_fx_reference -- --ignored`). This script
// turns the JSON into one page per effect, plus one table per category on the
// section index. Categories have no page of their own: their headings and the
// sentences under them live on src/content/docs/effects/index.mdx, hand-written,
// and only the table under each heading is generated.
//
// It only ever rewrites the block between the GENERATED markers, and not even
// all of that: the opening marker line itself is kept exactly as the page has
// it. Everything else on a page - the front matter, the prose about what the
// effect is for, the notes on what each control does - is hand-written and is
// never touched. A page that does not exist yet is scaffolded whole, markers
// included.
//
// The work is `generate()`, which scripts/gen-effect-pages.test.mjs calls
// against a temporary directory; running this file as a script is the same work
// against the repository.
//
// Run:  npm run docs:effects        (from web-docs/)
//       npm run docs:effects -- --check   (fails if anything is out of date)

import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const REFERENCE = join(here, "../../crates/lumit-core/fx-reference.json");
const OUT = join(here, "../src/content/docs/effects");
// Where gen-effect-shots.mjs puts the example pictures.
const SHOTS = join(here, "../src/assets/effects");
// How an effect page, four folders deep in src/content/docs, reaches
// src/components. The figure is a component now, so the page needs the import.
const COMPONENTS = "../../../../components";

// MDX has no HTML comments - `{/* ... */}` is the expression-comment that
// compiles to nothing. The opening marker is found by its `GENERATED:<tag>`
// prefix alone, so the sentence after it can be reworded without stranding
// every page that already carries the old wording.
const begin = (tag) =>
  `{/* GENERATED:${tag} - rewritten by \`npm run docs:effects\`. Edit the prose, not this block. */}`;
const END = "{/* END GENERATED */}";

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

/**
 * The id Starlight's Markdown gives a heading, so a link can point at one. This
 * is github-slugger's rule for the labels a category can have: lower case, drop
 * anything that is not a word character, a hyphen or a space, then spaces to
 * hyphens - which is why "Blur & sharpen" becomes "blur--sharpen", two hyphens,
 * one for each space around the ampersand that went away.
 */
const anchor = (label) =>
  label.toLowerCase().replace(/[^\w\- ]/g, "").replaceAll(" ", "-");

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

/** The prefix of a label that ends in a whole number, or null. */
const numbered = (name) => /^(.*?)(\d+)$/.exec(name)?.[1] ?? null;

/**
 * Lens flare has one coating dropdown per glass element, twenty of them, and
 * printing all twenty says the same sentence twenty times. A run of three or
 * more consecutive rows whose labels differ only by a trailing number and whose
 * other four cells are identical becomes the first row, an "Element ..." row,
 * and the last.
 */
function collapse(rows) {
  const out = [];
  for (let i = 0; i < rows.length; ) {
    const prefix = numbered(rows[i].name);
    let j = i;
    if (prefix !== null) {
      while (
        j + 1 < rows.length &&
        rows[j + 1].cells === rows[i].cells &&
        numbered(rows[j + 1].name) === prefix
      ) {
        j += 1;
      }
    }
    if (j - i >= 2) out.push(rows[i], { name: `${prefix}...`, cells: rows[i].cells }, rows[j]);
    else for (let k = i; k <= j; k += 1) out.push(rows[k]);
    i = j + 1;
  }
  return out;
}

function paramTable(effect) {
  const rows = collapse(
    effect.params.map((p) => ({
      name: cell(rowName(effect, p)),
      cells:
        `${cell(CONTROL[p.kind] ?? p.kind)} | ${cell(rangeCell(p))} | ` +
        `${cell(defaultCell(p))} | ${cell(UNIT[p.unit] ?? p.unit)}`,
    })),
  );
  return [
    "| Parameter | Control | Range | Default | Unit |",
    "| --- | --- | --- | --- | --- |",
    ...rows.map((r) => `| **${r.name}** | ${r.cells} |`),
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
  // the engine stopped honouring. Either sentence supplies its own full stop
  // when it already ends in one, so the page never gets two.
  const matteRow = effect.params.find((p) => p.id === (effect.matte?.param ?? "matte"));
  if (matteRow) {
    const meaning = (
      effect.matte?.meaning ?? "is an input to an effect, scaling the strength."
    ).trim();
    const stop = meaning.endsWith(".") ? "" : ".";
    const invert = effect.params.find((p) => p.kind === "bool" && p.label === "Invert");
    const swap = invert ? ` **${invert.label}** inverts the mattes for calculating strength.` : "";
    out.push(`- **${matteRow.label}** ${meaning}${stop}${swap}`);
  }

  if (!out.length) return "";
  return `\n\nIn the Effect Controls panel:\n\n${out.join("\n")}`;
}

/** The whole marked block: opening marker, content, closing marker. */
const marked = (tag, body) => `${begin(tag)}\n\n${body}\n\n${END}`;

/**
 * Replace the marked block in an existing page, or return null if it has none.
 * The page's own opening marker line survives verbatim: an owner who reworded
 * that comment keeps their wording, and only what lies between the two markers
 * is rewritten.
 */
function splice(existing, tag, block) {
  const a = existing.indexOf(`{/* GENERATED:${tag}`);
  if (a === -1) return null;
  const b = existing.indexOf(END, a);
  if (b === -1) return null;
  const eol = existing.indexOf("\n", a);
  const marker = existing.slice(a, eol === -1 ? existing.length : eol);
  const body = block.slice(block.indexOf("\n"));
  return existing.slice(0, a) + marker + body + existing.slice(b + END.length);
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

/**
 * The figure is a component, and a component needs an import. The line goes
 * directly after the front matter, once: a page that already has it is left
 * alone, so running the generator twice does not stack two imports.
 */
function ensureImport(text, importLine) {
  if (/^import Compare from /m.test(text)) return text;
  const front = /^---\r?\n[\s\S]*?\r?\n---\r?\n/.exec(text);
  if (!front) return text;
  return `${text.slice(0, front[0].length)}\n${importLine}\n${text.slice(front[0].length)}`;
}

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
 * The example picture, rendered by `npm run docs:effect-shots` from
 * src/assets/effects/. It is a component, so that both halves of the wipe go
 * through Astro's image pipeline: a raw `<img>` out of `public/` is what the dev
 * toolbar's audit objects to, and the figure sits above the fold on every page.
 */
function exampleFigure(e, shots) {
  // A couple of effects have no picture and cannot have one: Posterize time is a
  // change to the clock, and Matte key wants a screen the example frame does not
  // contain. The harness says so and writes nothing, and an absent file means an
  // absent figure rather than a broken image.
  const file = join(shots, e.category_slug, `${e.slug}.webp`);
  if (!existsSync(file)) return "";
  const label = e.label.replaceAll('"', "&quot;");
  return `<Compare label="${label}" category="${e.category_slug}" slug="${e.slug}" />`;
}

// ---------------------------------------------------------------------------

/**
 * Write (or, with `check`, only count) every page the catalogue implies.
 * Returns what changed, what has no markers to splice into, and what nothing
 * claims any more; the caller does the talking.
 */
export function generate({
  reference = REFERENCE,
  out = OUT,
  shots = SHOTS,
  check = false,
  components = COMPONENTS,
} = {}) {
  const importLine = `import Compare from "${components}/Compare.astro";`;
  const changed = [];
  const stranded = [];
  const scaffolded = [];

  function put(path, tag, scaffold, block, insertAnchor, withImport = false) {
    const existing = existsSync(path) ? readFileSync(path, "utf8") : null;
    let next = scaffold;
    if (existing !== null) {
      next = splice(existing, tag, block);
      if (next === null && insertAnchor) next = insertAt(existing, insertAnchor, block);
      if (next === null) {
        stranded.push(path);
        return;
      }
    }
    if (withImport) next = ensureImport(next, importLine);
    if (existing === next) return;
    changed.push(path);
    if (!check) {
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, next);
    }
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
        const page = join(out, e.category_slug, `${e.slug}.mdx`);
        const link = `[${e.label}](/effects/${e.category_slug}/${e.slug}/)`;
        return `| ${cell(link)} | ${cell(description(page))} |`;
      }),
    ].join("\n");
  }

  const ref = JSON.parse(readFileSync(reference, "utf8"));
  const byCategory = new Map(ref.categories.map((c) => [c.slug, []]));
  for (const e of ref.effects) byCategory.get(e.category_slug)?.push(e);

  // One page per effect.
  for (const e of ref.effects) {
    const table = marked("parameters", `${paramTable(e)}${panelNotes(e)}`);
    const example = exampleFigure(e, shots);
    const figure = marked("example", example);
    const scaffold = `---
title: ${e.label}
description: What ${e.label} does, and what each of its controls means.
---
${example ? `\n${importLine}\n` : ""}
${e.label} is a **${e.category}** effect.

${figure}

## What it does

## Parameters

${table}

## What each control does

${e.params.map((p) => `- **${p.label}**`).join("\n")}

## Related

- [${e.category}](/effects/#${anchor(e.category)})
- [Effects](/use/effects/)
`;
    const path = join(out, e.category_slug, `${e.slug}.mdx`);
    put(path, "parameters", scaffold, table);
    put(path, "example", scaffold, figure, "## What it does", Boolean(example));
  }

  // The section index: one hand-written heading and sentence per category, each
  // with a generated table under it. The headings and the prose are the owner's
  // and are never rewritten; only the block tagged with the category's slug is.
  // A category the page does not mention yet - a new one in the engine - gets an
  // empty scaffold appended and a warning, because the description is prose and
  // prose is not this script's to write.
  {
    const path = join(out, "index.mdx");
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
uses, from the same untouched frame.

For applying effects, ordering a stack and using adjustment layers, see
[Effects](/use/effects/).

## The categories
`;

    const existing = existsSync(path) ? readFileSync(path, "utf8") : null;
    let next = existing ?? scaffold;
    for (const c of ref.categories) {
      const tag = `list:${c.slug}`;
      const block = marked(tag, effectTable(byCategory.get(c.slug) ?? []));
      const spliced = splice(next, tag, block);
      if (spliced !== null) {
        next = spliced;
        continue;
      }
      scaffolded.push(c.label);
      next = `${next.trimEnd()}\n\n### ${c.label}\n\n${block}\n`;
    }
    if (existing !== next) {
      changed.push(path);
      if (!check) {
        mkdirSync(dirname(path), { recursive: true });
        writeFileSync(path, next);
      }
    }
  }

  // Pages nobody claims any more - a renamed effect leaves its old page behind,
  // and a stale page is worse than a missing one.
  const wanted = new Set([
    "index.mdx",
    ...ref.effects.map((e) => `${e.category_slug}/${e.slug}.mdx`),
  ]);
  const orphans = [];
  if (existsSync(out)) {
    for (const entry of readdirSync(out, { withFileTypes: true, recursive: true })) {
      if (!entry.isFile()) continue;
      const rel = join(entry.parentPath ?? entry.path, entry.name)
        .slice(out.length + 1)
        .replaceAll("\\", "/");
      if (!wanted.has(rel)) orphans.push(rel);
    }
  }

  return { changed, stranded, orphans, scaffolded };
}

// Run as a script.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const check = process.argv.includes("--check");
  const { changed, stranded, orphans, scaffolded } = generate({ check });

  for (const o of orphans) console.warn(`orphan page (no effect claims it): ${o}`);
  for (const s of stranded) console.warn(`page has no GENERATED markers, left alone: ${s}`);
  for (const c of scaffolded) {
    console.warn(`category "${c}" was appended to the effects index - write its description`);
  }
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
}
