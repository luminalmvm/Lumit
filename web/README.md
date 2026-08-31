# lumitlab.com

Two Astro sites, both static, both deployed from this repository to Cloudflare
Workers (static assets - the successor to Pages).

| Directory   | Domain                | What it is                          |
| ----------- | --------------------- | ----------------------------------- |
| `web/`      | `lumitlab.com`        | Marketing site and the download page |
| `web-docs/` | `docs.lumitlab.com`   | Starlight documentation             |

They are separate Pages projects because a real subdomain needs its own deployment
target. Each is small and builds in about a second.

```bash
cd web && npm install && npm run dev      # http://localhost:4321
```

## Deploying

The domain is already on Cloudflare, so Pages is the path of least resistance: it is
free, has no bandwidth cap, and serves from Cloudflare's CDN.

Each directory is its own Worker, with a `wrangler.jsonc` declaring `dist` as its
static asset directory. There is no server code - Cloudflare serves the built files
from the edge.

| Setting            | `lumitlab.com`      | `docs.lumitlab.com` |
| ------------------ | ------------------- | ------------------- |
| Worker name        | `lumit`             | `lumit-docs`        |
| Root directory     | `web`               | `web-docs`          |
| Build command      | `npm run build`     | `npm run build`     |
| Deploy command     | `npx wrangler deploy` | `npx wrangler deploy` |
| Build watch path   | `web/*`             | `web-docs/*`        |

The watch paths are **case-sensitive** - `Web/*` will silently never match and the
Worker will simply stop building on push. Node is pinned by `.node-version` (22) in
each directory, because the platform default is older than Astro 5 will build on.

The Worker name in `wrangler.jsonc` must match the Worker the dashboard created, or
`wrangler deploy` makes a second one alongside it.

Then add the custom domain to each under **Domains**. Because the DNS is already in
the same Cloudflare account, the records are created for you.

Pushing to the production branch deploys; other branches get preview URLs.

## Where the downloads come from

Nothing is hosted here. `web/src/pages/download.astro` asks the GitHub releases API
for the newest release and points the three buttons at its assets:

- `lumit-<version>-windows-x64-setup.exe`
- `lumit-<version>-linux-x64.flatpak`
- `lumit-<version>-macos-arm64.dmg`

So **tagging a release updates the site with no deploy** - `.github/workflows/release.yml`
builds and publishes on any `v*` tag, and the download page picks it up on next load.
GitHub serves release assets from its own CDN with no bandwidth limit, which is what
every comparable project does; there is nothing to scale here.

If the API call fails or is rate-limited (60 requests/hour per IP, unauthenticated),
every button falls back to the releases page, which is a hard-coded `href` in the
markup. The page is still fully usable with JavaScript disabled.

> **Note.** Those three names are the whole release (K-304) - `release.yml` builds one
> artefact per platform and no others, and every job gates the tag, so a release that
> publishes at all publishes all three. The Linux asset was a `.tar.gz` up to and
> including v0.1.0.

## Release notes

`/releases` is the changelog, in the shape of Astro's Starlog example: a sticky
version pill on the left, that release's notes beside it, newest first. One
Markdown file per release under `web/src/content/releases`, named for its version -
`0.1.0.md` is served at `/releases/0.1.0`, and each release also gets that page of
its own. The frontmatter is `title`, `description`, `versionNumber` and `date`;
`_template.md` in that directory is the file to copy, and the leading underscore
keeps it out of the collection.

The notes are written by hand and there are none yet, so the page shows a short
line pointing at GitHub releases instead. That empty state is a supported build,
not a broken one - the only sign of it is Astro warning during `npm run build` that the
glob matched nothing and that the collection is empty.

This is separate from the download page, which reads the GitHub releases API: the
API gives the assets, these files give the prose. Nothing here needs a tag to exist,
so notes can be written before or after the release goes out.

## Brand

`web/src/components/Wordmark.astro` builds the wordmark out of the app icon on load,
and it is the hero: the animation runs once, and without script (or under reduced
motion) the markup is the finished lockup standing still. `web/public/lumit-wordmark.svg`
is the same lockup as a static file, and that is what the header shows.

Its "umi" is outlined letterforms, not live text - they were traced from Schibsted
Grotesk, which the site no longer sets its copy in (K-438: Hanken Grotesk for text,
Geist Mono for numbers and container labels). The logotype is fixed artwork now and
does not follow the body face.

The regeneration script is not checked in; the component is the source of truth. To
change the geometry, edit the keyframes and the `viewBox` anchors directly.

## Screenshots

`src/assets/shots/` holds the pictures the front page shows. Every one is a real
capture of the application - nothing is a mockup and nothing has a fake window
frame around it:

| File | Where it appears |
| --- | --- |
| `hero.png` | the wide picture under "Composite the way you know" |
| `timeline.png` | "Intuitively designed." |
| `node-graph.png` | "Dynamic effects." |
| `workspace.png` | Meet the interface, Timeline |
| `graph-workspace.png` | Meet the interface, Graph |
| `nodes-workspace.png` | Meet the interface, Nodes |
| `audio-workspace.png` | Meet the interface, Audio |

They sit in `src/assets/` rather than `public/` so Astro's `<Image>` resizes
each one, re-encodes it as WebP and writes the `srcset` the page serves. Source
format and size do not matter; a 1.5 MB PNG leaves the build as a 149 KB WebP.
To replace a picture, overwrite the file with a capture of the same shape - the
two card pictures are cut to 580:260, the rest are whole windows or whole
panels shown at their own aspect.

The tab pictures are whole windows, one per workspace. `timeline.png` is two
captures cut together on the diagonal, and `node-graph.png` is cut out of
`nodes-workspace.png` - so replacing that one means recutting this one.

## The front page arrives

Nothing below the wordmark is drawn at load. The wordmark plays its own
animation and, on the frame the lockup lands, dispatches `wordmark:home` and
plays `public/audio/click.mp3` at a quarter volume - a browser that refuses to
play audio nobody asked for simply does not, and nothing else changes.

That cue releases the page. The hero goes in order: the download button, the
line above it a word at a time, then the two lines under it. Everything further
down arrives as it scrolls into view. Each piece takes 500ms, blurring and
lifting into place; `--i` on an element pushes it 70ms later than its
neighbours, and `--base` on the hero line delays its first word.

Under `prefers-reduced-motion` and without script the whole page is simply
there. The source of the sound is `assets/audio/click.mp3` at the repo root;
`public/audio/` holds the site's copy, as `public/` does for the brand marks.
