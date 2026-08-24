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

`public/shots/` holds the pictures the front page shows. Every one is a real
capture of the application, cropped and re-encoded as WebP - nothing is a
mockup and nothing has a fake window frame around it:

| File | Size | Where it appears |
| --- | --- | --- |
| `workspace.webp` | 1704x961, 60 KB | the wide picture under "Everything you know" |
| `timeline.webp` | 1160x520, 29 KB | "Familiar by design." |
| `welcome.webp` | 1160x520, 12 KB | "Yours to keep." |
| `poster-retime.webp` | 768x364, 8 KB | poster for the Retime slot |
| `poster-flare.webp` | 768x364, 3 KB | poster for the lens-flare slot |
| `poster-camera.webp` | 768x364, 12 KB | poster for the camera-solve slot |

About 124 KB in total, all of it lazily loaded except the wide one. To replace
a picture, capture the application, crop to the aspect in the table and save as
WebP at quality 80 - there is no build-time image pipeline here on purpose.

## Feature clips

The three slots under the screenshots play a short clip on hover. Each is a real
`<video>` pointing at `public/clips/` (`retime.webm`, `flare.webm`, `camera.webm`)
with a screenshot crop as its poster. **None of the three clips has been recorded
yet**, so what a visitor sees today is the poster: hovering removes the "plays on
hover" label and leaves the picture, which is a real screenshot and worth showing
on its own. Drop the files in and the slots start moving with no code change. The
three are listed as content debt in `docs/TODO.md`.
