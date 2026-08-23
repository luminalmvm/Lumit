// What the page generator must never do to a page somebody wrote by hand.
//
// In plain terms: the manual's effect pages are half machine-written (the
// parameter table, the picture) and half hand-written (everything that explains
// anything). The generator rewrites only what lies between its markers. This
// test builds a tiny fake catalogue and a page with prose above, below and
// between the blocks, runs the generator over it, and checks the prose came back
// byte for byte.

import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

import { generate } from "./gen-effect-pages.mjs";

const REFERENCE = {
  categories: [{ slug: "stylise", label: "Stylise" }],
  effects: [
    {
      slug: "fake-flare",
      label: "Fake flare",
      category: "Stylise",
      category_slug: "stylise",
      groups: [],
      params: [
        {
          id: "intensity",
          label: "Intensity",
          kind: "float",
          unit: "raw",
          default: 1,
          slider_min: 0,
          slider_max: 4,
        },
        ...[1, 2, 3, 4].map((i) => ({
          id: `el${i}`,
          label: `Element ${i}`,
          kind: "choice",
          unit: "raw",
          default: 0,
          options: ["As the lens file", "Uncoated"],
        })),
        { id: "matte", label: "Matte", kind: "layer", unit: "raw" },
        { id: "invert", label: "Invert", kind: "bool", unit: "raw", default: false },
      ],
    },
    {
      slug: "no-picture",
      label: "No picture",
      category: "Stylise",
      category_slug: "stylise",
      groups: [],
      params: [{ id: "matte", label: "Matte", kind: "layer", unit: "raw" }],
    },
  ],
};

/** The hand-written lines that must survive every run, exactly as they are. */
const PROSE = [
  "Fake flare is a hand-written sentence nobody may touch.",
  "## What it does",
  "A paragraph the owner wrote, with its own wording.",
  "## What each control does",
  "- **Intensity** is explained here, at length, by a person.",
];

/** A page carrying old generated blocks, a reworded marker, and that prose. */
const PAGE = `---
title: "Fake flare"
description: "A flare that is not real."
---

${PROSE[0]}

{/* GENERATED:example - don't edit this, it is the owner's own wording of the marker. */}

<figure class="example">an old figure</figure>

{/* END GENERATED */}

${PROSE[1]}

${PROSE[2]}

## Parameters

{/* GENERATED:parameters - rewritten by \`npm run docs:effects\`. Edit the prose, not this block. */}

| Parameter | Control | Range | Default | Unit |
| --- | --- | --- | --- | --- |
| **Something stale** | Slider | 0 to 1 | 0 | - |

{/* END GENERATED */}

${PROSE[3]}

${PROSE[4]}
`;

function fixture() {
  const dir = mkdtempSync(join(tmpdir(), "lumit-effect-pages-"));
  const reference = join(dir, "fx-reference.json");
  writeFileSync(reference, JSON.stringify(REFERENCE));

  const shots = join(dir, "shots");
  mkdirSync(join(shots, "stylise"), { recursive: true });
  writeFileSync(join(shots, "plate.webp"), "not really a picture");
  writeFileSync(join(shots, "stylise", "fake-flare.webp"), "not really a picture");

  const out = join(dir, "effects");
  mkdirSync(join(out, "stylise"), { recursive: true });
  writeFileSync(join(out, "stylise", "fake-flare.mdx"), PAGE);

  const run = () => generate({ reference, out, shots, components: "../../../../components" });
  const page = () => readFileSync(join(out, "stylise", "fake-flare.mdx"), "utf8");
  return { dir, out, run, page };
}

test("the hand-written prose and the owner's marker survive", () => {
  const f = fixture();
  try {
    f.run();
    const page = f.page();
    for (const line of PROSE) assert.ok(page.includes(line), `lost: ${line}`);
    assert.ok(
      page.includes(
        "{/* GENERATED:example - don't edit this, it is the owner's own wording of the marker. */}",
      ),
      "the reworded opening marker was overwritten",
    );
    assert.ok(!page.includes("an old figure"), "the old figure was left behind");
    assert.ok(!page.includes("Something stale"), "the old table was left behind");
  } finally {
    rmSync(f.dir, { recursive: true, force: true });
  }
});

test("the table matches the catalogue, with the repeated rows collapsed", () => {
  const f = fixture();
  try {
    f.run();
    const rows = f
      .page()
      .split("\n")
      .filter((l) => l.startsWith("| **"));
    assert.deepEqual(
      rows.map((l) => l.split(" | ")[0]),
      ["| **Intensity**", "| **Element 1**", "| **Element ...**", "| **Element 4**", "| **Matte**", "| **Invert**"],
    );
    assert.ok(f.page().includes("| **Intensity** | Slider | 0 to 4; any value by typing | 1 | - |"));
  } finally {
    rmSync(f.dir, { recursive: true, force: true });
  }
});

test("the matte sentence ends in one full stop", () => {
  const f = fixture();
  try {
    f.run();
    assert.ok(!f.page().includes(". ."), "the default matte sentence doubled its full stop");
    assert.ok(
      f.page().includes("- **Matte** is an input to an effect, scaling the strength. **Invert**"),
    );
  } finally {
    rmSync(f.dir, { recursive: true, force: true });
  }
});

test("the Compare import lands once, however often the generator runs", () => {
  const f = fixture();
  try {
    f.run();
    f.run();
    const line = 'import Compare from "../../../../components/Compare.astro";';
    assert.equal(f.page().split(line).length - 1, 1);
    assert.ok(f.page().includes('<Compare label="Fake flare" category="stylise" slug="fake-flare" />'));
  } finally {
    rmSync(f.dir, { recursive: true, force: true });
  }
});

test("an effect with no picture gets no figure and no import", () => {
  const f = fixture();
  try {
    f.run();
    const page = readFileSync(join(f.out, "stylise", "no-picture.mdx"), "utf8");
    assert.ok(!page.includes("import Compare"), "an imported component nothing uses");
    assert.ok(!page.includes("<Compare"), "a figure for a picture that is not there");
  } finally {
    rmSync(f.dir, { recursive: true, force: true });
  }
});
