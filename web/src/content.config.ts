import { defineCollection } from "astro:content";
import { glob } from "astro/loaders";
import { z } from "astro/zod";

// Release notes, in the shape Astro's Starlog example uses: one Markdown file
// per release under src/content/releases, named for its version (0.1.0.md), with
// the version and date in frontmatter and the notes themselves as the body.
//
// The collection is deliberately empty for now - the notes are written by hand,
// and both the index and the per-release page build fine with nothing in it.
const releases = defineCollection({
  // The [!_] prefix keeps _template.md out of the collection: it is the example
  // to copy, not a release.
  loader: glob({
    base: "./src/content/releases",
    pattern: "**/[!_]*.md",
    // Without this the loader slugifies the file name and 0.1.0 becomes 010.
    // The version is the URL, dots and all: /releases/0.1.0.
    generateId: ({ entry }) => entry.replace(/\.md$/, ""),
  }),
  schema: z.object({
    // Shown as the heading of the single-release page and in its <title>.
    title: z.string(),
    // Used for the page description and the social card text.
    description: z.string(),
    // The pill on the left rail, e.g. "0.1.0". No leading "v" - the page adds it.
    versionNumber: z.string(),
    // Sorts the index, newest first. Any form Date understands: 2026-08-06.
    date: z.coerce.date(),
  }),
});

export const collections = { releases };
