import rss from "@astrojs/rss";
import type { APIContext } from "astro";
import { releasesNewestFirst } from "../../scripts/releases";

// The changelog as a feed, for anyone who would rather be told about a release
// than come and look. One item per release, newest first, carrying the notes
// themselves so a reader does not have to click through to read them.
export async function GET(context: APIContext) {
  // The order the changelog page lists them in, which tells two releases
  // on one day apart by their version.
  const releases = await releasesNewestFirst();
  return rss({
    title: "Lumit releases",
    description: "Lumit changelog",
    site: context.site!,
    items: releases.map((release) => ({
      title: release.data.title,
      description: release.data.description,
      pubDate: release.data.date,
      link: `/releases/${release.id}`,
      // The loader has already turned the Markdown into HTML.
      content: release.rendered?.html,
    })),
  });
}
