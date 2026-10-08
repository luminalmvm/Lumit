import rss from "@astrojs/rss";
import type { APIContext } from "astro";
import { getCollection } from "astro:content";

// The changelog as a feed, for anyone who would rather be told about a release
// than come and look. One item per release, newest first, carrying the notes
// themselves so a reader does not have to click through to read them.
export async function GET(context: APIContext) {
  const releases = (await getCollection("releases")).sort(
    (a, b) => +b.data.date - +a.data.date,
  );
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
