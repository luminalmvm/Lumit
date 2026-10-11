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

> **Note.** Those three names are the whole release - `release.yml` builds one
> artefact per platform and no others, and every job gates the tag, so a release that
> publishes at all publishes all three. The Linux asset was a `.tar.gz` up to and
> including v0.1.0.

## Lumit Pro

Pro is bought inside Lumit, and paid for on `/pro/checkout`, where Paddle's script draws
its form over the page. Paddle's public client-side token is written into the page. To
test against the sandbox, build with `PUBLIC_PADDLE_TOKEN` set to the sandbox's token and
`PUBLIC_PADDLE_ENV=sandbox`. The two prices on `/pro` are written into the page, in
dollars.

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

One section is read by more than the site. A `## Before you update` section, for a
release that changes something a project may depend on, is copied by `release.yml` to
the top of the GitHub release, and Lumit shows it after the update is downloaded and
before it restarts. It has to be in the file when the tag is pushed, and it can be
edited on the GitHub release afterwards.

## Brand

`web/src/components/Wordmark.astro` builds the wordmark out of the app icon, and it is
the hero. On the home page it is `controlled`: `HeroTimeline.astro` under it is a small
working timeline whose keyframes say where the wordmark is between the icon and the
word, how bright the field's light is (`HeroField.astro`), and when the flare and the
RGB split fire. It loops, opening forwards and closing by the same move played
backwards, and a visitor can drag the keyframes and the playhead. Without script (or
under reduced motion) the markup is the finished lockup standing still.
`web/public/lumit-wordmark.svg` is the same lockup as a static file, and that is what
the header shows.

Its "umi" is outlined letterforms, not live text - they were traced from Schibsted
Grotesk, which the site no longer sets its copy in (Hanken Grotesk for text,
Paper Mono for numbers and container labels). The logotype is fixed artwork now and
does not follow the body face.

The regeneration script is not checked in; the component is the source of truth. To
change the geometry, edit the keyframes and the `viewBox` anchors directly.

The social card every page shares is `web/public/og.png`, rasterised from
`web/src/assets/og.svg` by `npm run og`. Its text is outlines too, so it comes out the
same on any machine; changing the words means outlining them again.

## Screenshots

`src/assets/shots/` holds the pictures the front page shows. Every one is a real
capture of the application - nothing is a mockup and nothing has a fake window
frame around it:

| File | Where it appears |
| --- | --- |
| `hero.png` | the wide picture under "Composite the way you know" |
| `lanes.png` | "Intuitively designed." |
| `effects.png` | "Dynamic effects." |
| `workspace.png` | Meet the interface, Timeline |
| `graph-workspace.png` | Meet the interface, Graph |
| `nodes-workspace.png` | Meet the interface, Nodes |
| `audio-workspace.png` | Meet the interface, Audio |
| `story-shared.png` | Inside Lumit, Shared projects |
| `story-setup.png` | Inside Lumit, Setup |
| `story-shortcuts.png` | Inside Lumit, Shortcuts |
| `story-text.png` | Inside Lumit, Text |

They sit in `src/assets/` rather than `public/` so Astro's `<Image>` resizes
each one, re-encodes it as WebP and writes the `srcset` the page serves. Source
format and size do not matter; a 1.5 MB PNG leaves the build as a 149 KB WebP.
To replace a picture, overwrite the file with a capture of the same shape - the
two card pictures are both 1910 by 666, the rest are whole windows or whole
panels shown at their own aspect.

The tab pictures are whole windows, one per workspace. The two card pictures
are cut out of a window with the Timeline given half its height: `lanes.png`
is the Timeline from its left edge, and `effects.png` is the row above it,
Effect controls beside the Viewer.

The pictures are of motion-graphics projects built in Lumit for the site, and
Meet the interface is the one section still on an edit of footage.

The four `story-` pictures are cut out of whole-window captures, so the interface
in them can be read. A wide card and a narrow one share a row, and the two
pictures in a row are cut to come out the same height: 1424 by 864 beside
1000 by 860, and 1636 by 700 beside 876 by 530.

The small drawings above the ten reasons are diagrams and not captures. They are
drawn in `src/components/home/Features.astro`.

## Clips

The wide picture, the two cards under it, the four workspaces and the four `story-`
cards each play a clip over the picture. The clips are in `src/assets/clips/`, one MP4
a picture under the picture's own name, with no sound, at 30 frames a second. A clip
is the same box as its picture and the picture is the clip's first
frame, so replacing one means replacing both. A clip is fetched when it first comes on
screen, plays while it is in view, and is left out for a visitor who has asked for
less motion.

They are recordings of the application, made a frame at a time: a script puts Lumit in
the state a frame wants, waits for it to draw, and saves the picture, and ffmpeg makes
the clip from the frames. The pointer in them is drawn by the script, since the capture
reads Lumit's own picture and the system's pointer is not in it. The script is not
checked in.

## Projects

`/projects` hands out the motion-graphics projects the front page's pictures are
of. Each one is three files under one name: the project in `public/projects/` as a
`.lum`, and in `src/assets/projects/` a PNG of one frame at 1920 by 1080 with an MP4
of the whole piece at half that size, 30 frames a second, and no sound. The card's
words, its layer count, and the fonts it names are in `src/pages/projects.astro`, and
the size beside the download is read off the file when the site is built.

To replace a project, save it over its `.lum`, export the piece again for the MP4, and
save one frame of it for the PNG. If its fonts or layer count changed, change them on
its card too. A project set in a font the visitor does not have opens in Inter, which
is why each card names its fonts.

The clips here and on the front page are played by `src/scripts/clips.ts`.

## Media

`/media` hands out the wordmark, the wordmark opening, and the mark, each on a
transparent, black, or white background. The files are in `public/media/`. The stills
are the lockup out of `Wordmark.astro` and the mark out of `assets/brand/`, and the
animated ones are `Wordmark.astro` posed a frame at a time and handed to ffmpeg, so they
are the same move the home page plays. The script that makes them is not checked in.
White letters do not show on white, so the white files have dark letters.

## Sections

Every section opens with `SectionHead.astro`: the ruled line with its name on it,
the heading, and a line under it. The home page below the hero is one component a
section, in `src/components/home/`, and each keeps its own words and styles.

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
