# Lumit UI specification

**Status: canonical.** This document specifies the structure and behaviour of every panel in
Lumit's interface. Terminology follows [01-GLOSSARY.md](01-GLOSSARY.md) exactly; locked
decisions in [02-DECISIONS.md](02-DECISIONS.md) are assumed. All colour, type, spacing, and
iconography specifics are deferred to [15-DESIGN.md](15-DESIGN.md) — this document says *what
exists and how it behaves*, never what it looks like.

RFC-2119 keywords (MUST, SHOULD, MAY) are used with their standard meanings.

This is the **target** UI specification; the shipping Flutter frontend implements a subset of
it. Read the sections below as the design, not a claim of current state — the gaps are
tracked in [TODO.md](TODO.md), which is the one document that says what is built.

The base arrangement is deliberately After Effects-shaped, because the target audience arrives
from AE: Viewer in the centre, Project panel on the left, Effect Controls / Effects & Presets /
Scopes on the right, Timeline across the bottom. Everything beyond that shape is movable —
the interface is truly the user's.

---

## 1. Application shell and docking

> **Shell shape (K-074, K-086).** The shell is a tiling dock. Panels stacked together form a
> tab group with a title tab per panel, draggable to re-arrange the workspace; a panel that
> sits alone renders as a bare pane with no tab bar — the Viewer's look on every solo panel.
> A tabbed panel's pop-out button lifts it into its own OS window and dragging its tab moves
> it; a bare pane, having no tab bar to carry either, is popped out by right-clicking
> anywhere empty in it. It carries **no drag grip of its own** (K-478 — the corner square of
> dots is withdrawn): it stays a drop target for any panel dragged onto it, and rearranging
> from a bare pane is done through Window → Workspace. The Timeline's comp-tab-strip
> right-click pop-out is this same mechanism, not a special case. Closing a popped-out
> window docks the panel back.

### 1.1 Frames, groups, tabs

- The main window is divided into a tree of **frames**. Each frame holds one **panel group**;
  a group is a tabbed set of panels with exactly one visible at a time.
- Every panel MUST carry a tab with its name. Dragging a tab MUST begin a docking drag.
  Dragging the group's tab-bar background moves the whole group.
- Each panel MUST have a panel menu (top-right of the group) containing panel-specific
  options plus the common entries: Undock (float), Close, Maximise, and Help for this panel.
- Pressing `` ` `` (backtick) with the pointer over a panel MUST toggle that panel between
  its docked size and maximised to the full window (AE's tilde behaviour; backtick and tilde
  share a key on the layouts our audience uses).

### 1.2 Drop zones

While a panel drag is active, the target frame MUST display five drop zones:

| Zone | Result |
|---|---|
| Centre | Join the target group as a new tab |
| Top / bottom / left / right edge of a group | Split the frame; insert the panel adjacent; siblings resize proportionally |
| Window edges | Dock against the whole window (full-height/full-width split) |

Docking MUST never overlap panels inside a window; only floating windows overlap. Drop zones
MUST be rendered as explicit highlighted regions during the drag (no invisible targets), and
the pending layout SHOULD be previewed as an outline before release.

### 1.3 Floating windows

- Dropping a panel outside any drop zone, choosing Undock, or holding `Ctrl` during the drop
  MUST create a floating window. Floating windows are true OS windows
  and MUST be placeable on any monitor.
- A floating window hosts its own frame tree: users MAY dock several panels into one floating
  window and split it like the main window.
- Closing a floating window closes its panels (recoverable from the Window menu, which lists
  every panel type; opening an already-open panel focuses it instead).

### 1.4 Workspaces

A **workspace** is a named, saveable arrangement of panels (glossary §7).

- Lumit MUST ship four workspace presets: **Edit**, **Effects**, **Colour**, **Audio**
  (§1.6). Preset workspaces MUST be restorable to their factory layout at any time
  (*Reset workspace*), individually, without touching user workspaces.
- Layout changes MUST persist automatically to the active workspace across sessions.
  *Save as new workspace…* creates a user workspace; user workspaces MAY be renamed,
  reordered, deleted, exported, and imported.
- Workspaces are stored per user in the configuration directory as individual
  human-readable files, so they can be shared (the montage scene shares everything —
  K-065). They are never stored in the project.
- A workspace switcher MUST be visible in the main window chrome (a compact strip of
  workspace names) and in the Window menu; `Alt+Shift+1…9` switches by position.
- Switching workspaces MUST NOT close, reload, or re-evaluate anything — it only
  rearranges panels. Panels open in the old workspace but absent from the new one keep
  their state in memory for the session.

### 1.5 Workspace state versus project state

The AE lesson: a viewer locked to a specific comp is a reference to *project content*, so it
cannot live in a workspace (AE silently drops such locks). Lumit splits state explicitly:

| Lives in the workspace (per user) | Lives in the project (`project.json` session block) |
|---|---|
| Frame tree, panel groups, tab order | Which comps are open, and the active comp |
| Panel sizes, floating window geometry | Viewer-to-item locks (§2.6) |
| Which panel types are open | Per-comp Viewer state: preview resolution, magnification, channel view, transparency grid, guide/ruler visibility |
| Timeline column visibility and widths | Per-comp Timeline state: twirl-down expansion, work area, playhead position, zoom range |
| Toolbar layout | Guides themselves, markers, region of interest |

Opening a project MUST restore the project-side state above regardless of which workspace is
active. A project opened on another machine therefore looks like the same *edit* even though
the panel arrangement is the local user's own.

### 1.6 Shipped workspace presets

Structure only, with three exceptions: **Retiming**'s panel inventory differs because the
Easing panel is in no other arrangement (K-349) and **Nodes**' (K-445, K-471) because the
Graph and Node panels are in no other.

**No shipped arrangement carries the Hierarchy panel** (K-614) — not the default, and not
one preset. The parenting tree is a thing you go and look at, not a thing you work beside,
and a tab nobody opens is a tab in the way of the ones they do. It is one tick away in the
Window menu, exactly as Easing, Graph and Node are.

- **Edit** (default): Project panel left, fronted, with Effect Controls tabbed
  behind it; Viewer centre; **right column Effects & Presets, fronted**, with Scopes and the
  Debug view tabbed behind (K-322 — the panel used to be a fourth tab on the *left*, buried
  behind Project, with Debug fronting the right column instead); Timeline across the full
  bottom at roughly one-third window height. Shares: 0.68/0.32 vertically, 0.22/0.58/0.20
  across the upper band.
- **Effects**: Effect Controls promoted to its own left column beside the Project panel;
  Effects & Presets expanded on the right with Scopes and the Debug view tabbed behind;
  Timeline slightly shorter than Edit. Seeing the picture *at* one effect rather than at
  the end of the stack is not a panel of this preset or any other: it is the Viewer's own
  **"at effect" chip** (K-528, §2.2), which appears over the picture whenever an effect is
  selected — in the Effect controls stack or as a box on the graph — and shows the
  composition with that layer's effects stopping there.
- **Colour**: Scopes given a wide right-hand column showing waveform and vectorscope
  simultaneously (two panels stacked); Effect Controls left; Effects & Presets tabbed away;
  Viewer centre-dominant.
- **Audio**: Audio panel promoted to a tall right column; Timeline taller than Edit with
  audio waveforms expanded by default; Viewer reduced. This is the v1 audio surface — the
  future Composer workspace is specified in [09-AUDIO.md](09-AUDIO.md) and deliberately not
  here.
- **Retiming** (K-349): the arrangement for shaping how things move. The **Easing** panel
  takes the right-hand column outright — a bare pane, not tabbed, because the point of the
  panel over the popup is that it stays on screen while the selection changes underneath
  it; Project fronted left with Effect Controls and Effects & Presets tabbed
  behind; Viewer centre; Timeline as tall as Audio's, retiming being timeline work. Shares:
  0.55/0.45 vertically, 0.20/0.58/0.22 across the upper band.
- **Nodes** (K-445, K-471): the graph as the main surface. The Graph panel takes the
  centre and left outright, with the ordinary Timeline, short, beneath it inside the same
  column; the small **Viewer** — keeping its whole bar — sits upper right, and the **Node
  panel** (the selected node's parameter rows) lower right. The approved Nodes-workspace
  drawing governs the layout; the panel rules are 15-DESIGN §12A.7 and the model is
  [impl/node-graph.md](impl/node-graph.md). Picking a box and seeing the picture there is
  the small Viewer's own chip (K-528), so this preset needs no second viewport for it. It
  is also the one preset whose root splits
  **across** rather than down, because the Timeline runs under the graph column only:
  shares 0.76/0.24 across, the graph column 0.82 Graph to 0.18 Timeline, the right column
  0.80 Viewer to 0.20 Node. It carries no Project panel, which is the drawing's own
  inventory — wiring a layer is work you arrive at with the layer already chosen.

### 1.7 The toolbar (K-216)

A single **toolbar** spans the window immediately below the menu bar and above the dock. It
is chrome, not a panel: it cannot be closed, moved, tabbed or floated, and it is the same
strip whatever workspace is active.

**What it holds, left to right.**

| Group | Tools, in flyout order | Shortcut |
|---|---|---|
| Selection | Selection | `V` |
| Hand | Hand | `H` |
| Zoom | Zoom | `Z` |
| Rotation | Rotation | `W` |
| Anchor point | Anchor point | `Y` |
| Razor | Razor | `C` |
| Shape | Rectangle, Rounded rectangle, Ellipse, Polygon, Star | `Q` |
| | *(with a layer selected these draw a **mask** on it; §2.3.1)* | |
| Pen | Pen, Add vertex, Delete vertex, Convert vertex, Mask feather | `G` |
| Type | Horizontal type, Vertical type | `Ctrl+T` |
| Paint | Brush, Clone stamp, Eraser | `Ctrl+B` |
| Roto | Roto brush, Refine edge | `Alt+W` |
| Puppet | Puppet position pin, Puppet starch pin, Puppet overlap pin, Puppet bend pin | `Ctrl+P` |
| Camera | Orbit camera, Pan camera, Dolly camera | `Shift+C` |

The right-hand end carries the **tool options** area (below) and the **workspace strip** §1.4
requires in the window chrome.

**The strip is 30px tall and its buttons are 44px wide** (K-230). 15-DESIGN §7.2's hit extent
is kept *across* the row, which is what the strip is read and aimed by, and given up down the
page: the strip runs the full width of the window, so a 44px band of mostly empty chrome is
height taken from the panels underneath for nothing.

**There is no snapping switch** (K-230). One was here, and nothing in the application read it.
A toggle that governs nothing is worse than a missing one — it makes the reader doubt what
snapping *is* here rather than what it is set to — so it returns with the snapping it governs
(§4.5).

**Tool options** (K-225). The armed tool's own settings, where After Effects puts them, and
empty for the tools that draw nothing:

| Armed tool | Options |
|---|---|
| Type | **Fill** swatch, **size** in pixels |
| Brush, Clone stamp, Eraser | **Fill** swatch, brush **size**, **hardness**, **opacity** (K-227) |
| Shape, Pen | **Fill** swatch, **Stroke** swatch, **stroke width** in pixels — all live (K-237) |

Every option is session state, like the armed tool itself, and every one is live: fill and size
say what the next thing drawn is made with, and the stroke pair outlines a new shape layer's art
(K-237 — a width of zero draws no outline).

**Behaviour.**

- Exactly one tool is armed at a time, app-wide. The armed tool is session state: it is
  neither project state (arming one changes no document) nor workspace state (§1.5), and
  every session opens on Selection.
- A group of several tools MUST show one button carrying the member last used, marked with a
  corner triangle. Press-and-hold or right-click MUST open the group's flyout; clicking the
  button arms the member shown.
- A group's shortcut arms the member last used; pressing it again while that group is
  already armed MUST step to the next member and wrap — so `Q` walks the five shape tools
  without opening the flyout.
- Tool bindings live in the keymap's own `Tools` context (§15) and are remappable there like
  every other binding. That context is not a panel, so a chord resolves against the focused
  panel and the global table first and reaches the `Tools` table only if both decline —
  which is what lets `C` cut a clip in the Timeline and arm the razor everywhere else.
- Every tool button MUST carry a tooltip naming the tool and its current chord (§14). A tool
  whose behaviour is not built yet MUST say so in that tooltip rather than being hidden: the
  tool set above is the specification, and a strip missing half of it teaches the wrong shape
  of the application.
- **A tool whose behaviour is not built MUST NOT be armable** (K-228). Its button and its
  flyout row are drawn disabled, and the button, the row and the keyboard chord all decline
  together — the refusal belongs to the state that holds the armed tool, because there are
  three ways in and only one of them is a button. A group with nothing built in it takes no
  click at all and offers no flyout; a group's chord cycles only its built members, and a group
  whose first member is unbuilt opens on one that works. A tool you can pick that then does
  nothing reads as a broken application; shown, disabled and labelled is the honest version of
  showing it.

**Implementation status (2026-07-31).** The strip, the groups, the flyouts, the shortcuts,
the tool options area and the workspace strip are built. Built tools:
Selection (§2.3), Hand, Zoom (§2.2), Rotation, Anchor point, Razor (§4.4), the five shape
tools and the Pen (§2.3.1), Horizontal type (§2.3.2), the three painting tools (§2.3.4) and the
three camera tools (§2.3.5). **Disabled** (K-228 — shown, not armable): vertical type, the Pen's
four editing siblings, the Roto tools and the Puppet pins. Each tool's behaviour is tracked
separately in [TODO.md](TODO.md).

### 1.8 The menu bar (K-244)

The bar carries nine menus in this order: **File, Edit, Composition, Layer, Effect, Animation,
View, Window, Help**. The arrangement is deliberately After Effects', for the same reason the
panel layout is.

- Every command the finished application will carry MUST be listed, whether or not it is built.
  An unbuilt one MUST read "(Not implemented)" after its name and MUST be disabled.
- **A submenu MUST survive the diagonal to itself — the safe triangle (K-318).** A flyout
  opens beside the row that owns it, so the natural path to its first entry crosses the rows
  *below* that row. Switching on the row merely passed over takes the flyout away before it
  can be reached, and the user has to travel the corner instead. So while a flyout is open,
  a hover report from another row of the same surface is **held** while the pointer is inside
  the triangle from where it left the owning row to the flyout's near edge; the held switch
  lands when the pointer leaves that triangle, or after a short grace (300ms) if it simply
  stops there — resting on a row still means that row. Reaching the flyout voids anything
  pending. A move that is plainly *not* travel — straight down the menu, outside the triangle
  — switches at once, with no delay to feel. The rule holds for every popup that uses the
  shared menu surface, not the menu bar alone: the Add effect browser's category flyouts and
  every right-click menu get it from the same place.
- A command whose preconditions are absent (no project, no composition, no selected layer) MUST
  grey out rather than fail when pressed.
- A row MUST show the chord the keymap currently binds to its action, and MUST take it from the
  keymap rather than carrying a chord of its own (§15, K-199).
- On **macOS** the bar MUST be the system menu bar, not an in-window strip, with About and
  Settings in the application menu. On Windows and Linux Settings sits under Edit and About
  under Help. The item tree MUST be shared between the two renderings. With no in-window
  menu strip on macOS, the wordmark and the workspace tabs move into the toolbar row
  (K-448).
- **Window** MUST list every panel with a tick showing whether it is in the arrangement;
  toggling one adds or drops it, and the change persists because the arrangement does. The last
  remaining panel MUST NOT be hideable.
- **Effect** MUST offer one submenu per effect category, each item applying to *every* selected
  layer (K-217), and the whole menu MUST be disabled with nothing selected.
- **File ▸ Open recent** lists the ten most recent project paths, newest first.
- **Help ▸ Check for updates** MUST carry the whole update sequence in the one row (K-296):
  disabled and reading "Checking for updates…" while a check runs, then either
  "Click to update - v*X.Y.Z*" or back to "Check for updates" with *Lumit is up to date* in
  the status line. Pressing it MUST NOT close the menu, and the row MUST redraw in place as
  the state changes. Downloading MUST show progress in the same row, and a downloaded update
  MUST read "Restart to finish updating" until it is applied.
- **How an update is applied** follows where Lumit is installed (K-297), and the restart
  window MUST say which it is: swapped in place and restarted (a per-user installation, the
  normal case), handed to the installer (anywhere Lumit cannot write to its own files), or
  handed to Flatpak with the install command, in which case Lumit MUST NOT offer to restart
  because it is not replacing anything.

---

## 2. Viewer

The Viewer displays a comp, a footage item, or a single layer's source. One Viewer exists by
default; users MAY open additional Viewers (Window menu or comp context menu) and place them
anywhere, including other monitors.

### 2.1 Display modes

- **Comp mode** (default): the rendered composite at the playhead.
- **Footage mode**: a project footage item, pre-comp, with its interpretation applied — used
  for source review and setting source in/out before inserting into a Sequence layer.
- **Layer mode**: one layer's source before transform — the surface for slip edits, drawing
  masks on source, and (later) paint/tracking. Layer mode MUST show its own source-time strip
  for slipping a clip or layer under fixed in/out points.

### 2.2 Viewer bars

**The Viewer wears two strips** (K-466, superseding K-411 and settling K-448's split):

- **The header strip**, 22 tall, carrying the panel's kicker and — at its right-hand end,
  6 apart, each an 18-tall picker with a 10px label — the **magnification** (item 1), the
  **preview quality** (item 2, whose menu also carries the playback behaviour) and the
  **colour pipeline** (item 8, whose menu also carries the tone map, item 13).
- **The bottom bar**, 22 tall, padded 10 either end, glyphs at 14 and gaps of 8: the
  **transparency board** (item 4), the **view menu** (items 5–6, which also carries the
  layer-controls switch, the region of interest of item 7 and the composition background
  of item 10), the **channel** (item 3) and the **exposure** (item 12, with its reset mark
  to the left of the number while it is engaged); a hairline seam;
  the **snapshot pair** (item 14). Then, spaced 10, the five transport marks and the **clock**
  (item 11). At the right-hand end the **reading** — `comp · time · source → preview ·
  zoom` — which is where degradation is stated (item 9), and the preview progress bar,
  which is nothing at all until a frame is genuinely waited on.

**Settings → Appearance → Viewer → Viewer bars** chooses between the three arrangements
(K-448, K-467): the drawing's **split**, or **one bar at the top** or **at the bottom**,
which gathers the panel's kicker, the three pickers and everything the bottom bar carries
onto a single strip in that same order. No control is added or dropped by the choice.

Every control on either strip keeps the behaviour its item below defines. The items:

1. **Magnification** dropdown: Fit, Fit up to 100%, then 25 / 33.3 / 50 / 100 / 200 / 400 /
   800 %. Magnification is display scaling only; it MUST NOT change render resolution.
   `Ctrl+scroll` zooms about the pointer; `Shift+/` fits.
   **Every magnification change is anchored** (K-218): the comp point the gesture names —
   under the pointer for a wheel notch or a click, the middle of the box for a sweep — MUST
   still be under that point afterwards. A magnification MUST be clamped to a sane range
   rather than running to zero or infinity.
   **Zooming is animated** where the shell animates at all: the picture travels to the new
   magnification over the shell's motion duration (15-DESIGN §7) and cuts instantly under
   *No animation*. The interpolation MUST be geometric — magnification is a ratio, so a
   1× → 8× flight moves at a constant *rate*, not a constant number per frame. The wheel is
   the exception and MUST stay instant: it already arrives as a stream of small steps, and
   animating each one makes the picture lag the hand.
   **The Zoom tool** (§1.7): clicking zooms in about the point clicked, `Alt`-clicking zooms
   out about it, and dragging a box zooms so that box fits the panel and is centred;
   `Alt`+box is the exact inverse — the whole view shrinks into the box, still centred on it.
   The pointer MUST show which way the click will go before it is clicked, changing as `Alt`
   is pressed and released. A drag of only a few pixels MUST be treated as a click. Its
   pointer is **drawn** (K-230, §2.3.3): Windows ships no magnifier, and Flutter's name for
   one silently becomes the ordinary arrow there.
   **Magnification MUST NOT change the resolution rendered** (K-230). The scale the engine is
   asked for follows the *panel* — a Viewer docked small is cheap — and not the zoom inside
   it: zooming out used to lower it, which threw away every cached frame and made the picture
   coarser for a gesture that only meant "let me see more of it", and zooming in cannot raise
   it above composition resolution because there is nothing there to render.
   **The transparency board MUST cost the panel, not the picture** (K-230): bounded by the
   panel and clipped to the picture, never a surface the size of a magnified composition.
2. **Preview resolution** dropdown: Full / Half / Third / Quarter / Auto (glossary §5).
   True raster downsampling — Half renders a quarter of the pixels. **Auto** renders only
   the pixels the current magnification can display. The setting is **stored per comp** in
   the project. Preview resolution MUST never affect export.
3. **Channel view**: RGB / Red / Green / Blue / Alpha (alpha as greyscale matte). One
   **bare mark** on the bar, opening a menu that lists the names in full. The closed face
   is a **coloured circle for the view in force** (K-478, §5's one glyph with colour of its
   own): the tri-colour mark for RGB, a single circle in the channel's own colour for R, G
   and B, and the near-white a matte reads as for alpha. It is not boxed as a dropdown
   (K-466): a border round a colour is a box round a colour.
4. **Transparency grid** toggle (checkerboard behind transparent pixels instead of the comp
   background colour). An icon — the checkerboard itself — rather than the word (K-411).
5. **Wireframe/overlay menu**: layer wireframes, motion paths, mask paths, gizmo visibility,
   and a full wireframe display mode (outlines only, no raster) for heavy comps.
   **Built so far**: it is the same menu as item 6 — the bar has one overlay mark (K-466)
   — and it carries the layer-controls switch (K-217), which turns the wireframes, handles
   and hover highlight on and off as one. Separating them, and the full wireframe display
   mode, is still owed (docs/TODO.md).
6. **Guides menu**: rulers (`Ctrl+R`), guides (drag out of rulers; lock/clear), grid,
   title/action safe overlays, snapping-to-guides toggle.
   **Built so far (K-416, K-466)**: the menu itself, one mark beside the transparency
   board, with four checkable entries — Grid, Title/action safe, Layer controls and Region
   of interest — and a fifth row for the composition background, which opens the colour
   picker and carries a swatch of the colour it would write. The grid
   is the frame's own **eighths**, drawn as theme hairlines; the safe areas are the
   standard **90 % action / 80 % title** rectangles, square-cornered hairlines with no
   labels. Both are worked out from the picture's rectangle, so they zoom and pan with
   the shot rather than sitting still on the panel, and both are display-side: no engine
   copy, no cache entry, and nothing an export can see. State is **per comp, in the
   session** — keeping it with the project is owed, as are rulers, draggable guides and
   snapping, which land as further entries in this same menu rather than as new chrome.
7. **Region of interest** (K-362, landed): drag a rectangle; the engine renders only that
   region for preview. MUST be clearable in one click and MUST never affect export. Armed
   from the view menu of items 5–6 (K-466), swept on the picture, and outlined whenever it is in force. It is a
   window on the composite rather than a crop of a finished frame, so it saves the composite,
   the display encode and the publish — but not the effect stack, which runs per layer at the
   layer's own size. Frames rendered through a region take their own names, so scrubbing
   inside one still uses the cache. Per comp, in the session beside the preview resolution.
8. **Colour management indicator**: the current display transform (e.g. working space →
   display). Read-only badge; clicking opens colour settings. Always visible so "what am I
   looking at" is never ambiguous.
9. **Degradation indicator**: users MUST be able to tell a degraded frame from a final one
   at a glance (K-018). **Stated by the bar's reading rather than by a badge** (K-466): the
   reading always says the pixel count the engine actually made beside the composition's
   own — `1920×1080 → 960×540` — so the tier is legible at every moment without a box that
   appears and disappears mid-playback and drags the bar about as it goes. Naming the
   individual steps that were skipped ("glow skipped") is still owed (docs/TODO.md).
10. **Background colour**: per-comp background (project state), plus quick black / grey /
    custom. It is a **row in the view menu** (K-466) carrying a swatch of the colour it
    would write — the drawing gives it no seat of its own on the bar, and it is the one
    entry there that is a document edit rather than a way of looking, so it goes through
    an op and Ctrl+Z undoes it.
11. **Current time** readout in the comp's timecode; click to type a time. A time outside
    the composition lands on the nearest end rather than being refused (K-287).
12. **Exposure** (K-314): the number alone on the bar — 10px mono, no inset under it and
    no aperture beside it (K-466) — scrubbing on drag and taking a typed number, reading
    signed stops to one decimal: `+0.0`, `+1.4`, `-2.3`.
    The number means what the Exposure effect's does: the same `2^stops` gain in
    scene-linear (K-106), so the two agree. **Preview only; it MUST NOT change the
    export.**
    A **reset mark** stands to the left of the number and puts it back to `+0.0`. It is
    there only while the exposure is not zero (K-478): the drawing puts none beside a
    resting exposure, and a mark that is always there says a control is engaged when it
    is not.
13. **Tone mapping** toggle (K-314): an icon, no menu. A fixed highlight rolloff — the
    identity below the knee, so an ordinary composite is untouched, and a smooth shoulder
    above it that folds however-bright highlights back under 1 instead of letting them clip
    flat. It is the "what is actually up there" switch for a comp whose values run past 1,
    not a grade. **Preview only; it MUST NOT change the export**, and it MUST NOT adapt to
    the frame's content: a picture that re-exposes itself per frame breathes across every
    cut, and a revisited frame would stop being the frame it was.
    It is a **row in the colour-pipeline menu** (K-466), inside the display transform it is
    part of, and that row is **absent unless Settings → Interface asks for it**: most work
    never reads a picture this way. While it is hidden the tone map MUST also read as **off**
    whatever the comp stored — hiding the control must never strand an engaged look with
    nothing left to turn it off. The exposure of item 12 is never hidden.
14. **Snapshots** (K-416, K-532, superseding K-466's single mark): **a pair of marks**
    behind the bar's hairline seam — **Take** and **Show**. A click on Take stores what
    the Viewer is showing this instant; a **press and hold** on Show puts the stored
    picture back over the live one for as long as the button is down, which is the
    before/after read every grade leans on. They MUST be two marks and not one glyph
    carrying both gestures: a snapshot with nothing on screen to say it exists, and no
    control naming the comparison, is a snapshot nobody finds.
    One slot in v1 (After Effects' four-slot family can follow on the same mechanism).
    It is a *display* affordance and lives entirely in the display: the stage photographs
    its own picture through a `RepaintBoundary` around the picture alone — so the layer
    controls, the region outline and the guides of item 6 are **not** in the photograph —
    and nothing crosses to the engine, into a cache, or near an export. Show MUST be muted
    until a snapshot exists, and MUST say so when hovered. Releasing the button is the
    whole of a snapshot's lifecycle, so item 8's badge does not engage for it: a held
    comparison is not a lying picture, it is a second picture. A snapshot MUST NOT store
    more pixels than the panel can show: the boundary is the picture's rectangle, which
    at high magnification is the comp and not the panel (an HD comp at 400 % is 7680
    pixels across), so what is photographed is the **visible region** of that rectangle
    and not the whole of it (K-612) — never more pixels than the panel has, and every
    one of them at the resolution the live picture is drawn at, which is what a
    before/after at 400 % needs. The stored picture MUST go back over the part of the
    live picture it was taken from, so a zoom or pan since compares like with like; a
    snapshot taken while the whole picture was on screen therefore covers the whole of
    it, as before. A picture panned entirely off the panel has nothing to photograph and
    Take MUST stand down rather than store an empty picture.

Items 12 and 13 both persist **per comp** with the project, and while either is engaged
the Viewer MUST say the picture is not the export — item 8's badge is where that lives,
stated calmly rather than warned about (15-DESIGN).

The bar MUST remain one row; overflow collapses from the right into a chevron menu.

**Nothing on the bar may move as the picture changes (K-287).** Every part of it whose
text varies — the clock, the playback-mode button, the degradation badge, the preview
progress — sits in a slot sized for the longest thing it can ever say, and a part that
comes and goes keeps its slot while it is away. The bar is read while playback runs, and a
control that re-letters or resizes itself sixty times a second is movement in the corner
of the eye that means nothing. For the same reason the **playback-mode button says only
which mode is in force** ("Adaptive res" or "Every frame") and never the tier it has
settled on: which tier a frame was made at is item 9's badge, which appears only when
there is something to say.

#### 2.2.1 The "at effect" chip (K-528)

**Whenever exactly one effect is selected, the Viewer offers to stop the picture there.**
A small chip reading *at &lt;effect name&gt;* appears over the top-left of the picture,
and clicking it flips the Viewer between the finished composition and the composition
rendered with that layer's effect stack **truncated after the selected effect** — the
blur applied and nothing after it, without soloing the layer, bypassing anything, or
opening a second viewport. Clicking it again goes back.

- **Both selection surfaces offer it, because there is one selection.** An effect picked
  in the Effect controls stack and a box picked on the node graph are the same pick
  (K-300), so the chip is the same chip. Picking a *driver* offers nothing: a driver
  makes a number, not a picture.
- **It is the Viewer, not a preview of one.** The cut picture arrives down the ordinary
  frame transport at the Viewer's own quality and magnification, and every way of looking
  — the exposure, the channel, the transparency board, the scopes reading it — applies to
  it unchanged. It is a *way of looking*, and the chip is what says so: it MUST read as
  engaged while it is on, naming the effect the picture stops at, so the Viewer is never
  quietly showing an unfinished composition.
- **It clears itself.** Deselecting, selecting a second effect, selecting a different
  layer, or deleting the effect takes the chip away and returns the picture — a chip that
  outlived its selection would leave the Viewer quietly showing a truncated composition
  with nothing on screen saying why.
- **It is not on the bar.** The bar is a fixed row of ways of looking that are always
  available; this one exists only while something is selected, so it sits over the
  picture where the selection is, and takes no width from anything when it is away.

### 2.3 Layer controls: the wireframe and the transform gizmo (K-217)

- Selecting a visual layer shows a combined gizmo in comp space: move (body drag), scale
  (corner/edge handles, `Shift` for uniform), rotate (a bar standing off the top edge), and
  anchor point (distinct centre handle, `Y` tool to drag anchor without moving the layer).
- The **wireframe** is the box itself: the layer's own content rectangle put through its
  transform, so it turns and stretches with the layer rather than staying axis-aligned. A
  layer's rectangle is its content's — a clip's frame size, a solid's dimensions, a nested
  comp's size — and comp-sized for the kinds that have no content of their own (adjustment).
  **Text measures its own line** (K-230): the point size tall and the engine's own width
  estimate wide, with an empty line keeping one character's worth so a layer waiting to be
  typed into is still visible. It was comp-sized, which drew a box the size of the frame round
  twelve-pixel text. A Null draws its own 100×100 box, so a layer with no
  picture can still be selected and dragged.
- **A layer switched off is not on the picture** (K-231): it gets no wireframe, no hover
  highlight and no handles, and a click over it MUST fall through to whatever is under it. Its
  eye being off is how a layer is got out of the way.
- **Selecting on the picture** (Selection tool): clicking takes the topmost *visible* layer
  whose box contains the pointer; `Shift`-clicking adds to or removes from the selection; clicking
  empty space clears it. Hovering a layer that is not selected MUST draw its box faintly, so
  a click never selects something the user could not see coming.
- **The marquee**: dragging from a point inside no layer rubber-bands a rectangle and, on
  release, selects every layer **wholly** inside it. `Shift` adds the catch to the current
  selection instead of replacing it.
- **Dragging** a layer's body moves it; dragging one that is not selected selects it first,
  and dragging one that is already part of a selection moves the whole selection together.
  **A press inside something already selected takes that**, even where a higher layer overlaps
  the same spot (K-230); only a plain click still takes the topmost, which is how a layer
  underneath gets chosen with the mouse at all. Without the rule a layer chosen in the Timeline
  could not be dragged wherever anything covered it.
- **One gesture is one undo step** (K-230). A drag writes Position x and y, and a scale writes
  both axes, in a single batched op: an undo that put the layer back along one axis only reads
  as the undo being broken rather than as two honest edits.
- **Scale may be negative** (K-231). A handle dragged past the anchor turns the layer over,
  which is how a layer is mirrored; only a scale of exactly zero is barred, because the
  layer↔screen map inverts it. The box MUST follow a scale drag as it happens, the same rule a
  turn follows.
- The gizmo's **centre handle is the anchor point** (K-221), and dragging it pans behind —
  the pivot moves, the picture does not, and the **mark moves as it is dragged** rather than on
  release (K-235) — with the same `Shift` axis lock and `Ctrl`/`Cmd`
  key-point snapping the Anchor point tool has. Its grab radius MUST be much tighter than a
  scale handle's: it sits where a body drag naturally begins, and a generous one would turn
  every move into a pan-behind.
- The gizmo MUST operate in the layer's transformed space (including parents) and respect
  3D orientation when the layer is 3D.
- The **Hand tool** never edits on the picture: with it armed the wireframe is a read-out of
  what is selected — no handles, no hover highlight — and every drag pans the view.
- The **Rotation tool** (K-219) turns the **selection** — every selected layer, each about
  **its own anchor point** — from a drag anywhere over the picture; `Shift` locks the turn to
  45° steps, and clicking picks a layer as the Selection tool does. Each selected layer's
  anchor MUST be marked while the tool is armed: it is the pin the layer spins on, and a
  rotation about an unseen point is a rotation nobody can predict. A set turns as one gesture
  — the angle is swept about the first selected layer's anchor and applied to all of them —
  rather than each layer chasing its own angle from the same pointer.
- The **Anchor point tool** (After Effects' Pan Behind, K-220) drags a layer's anchor while
  **Position compensates**, so the pivot slides and the picture does not move at all. It acts
  on the layer under the pointer (selecting it, as the Selection tool does), or on the
  selection when the pointer is elsewhere. `Shift` locks the drag to one screen axis;
  `Ctrl` (`Cmd`) snaps the anchor to the layer's own key points — its four corners, four edge
  midpoints and centre — with the snap distance measured in **screen** pixels, so it is as
  precise as the magnification allows (§4.5's rule for every snap).
  **The pivot goes where the pointer is** (K-233): a click places it there and a drag keeps it
  under the pointer. It MUST NOT be a nudge from where the anchor already was — that lets a
  pivot be pushed towards a place but never put at one. `Shift`+click stays a selection gesture
  and moves nothing. The whole drag MUST be
  one undo step: half of it would move the picture, which is the one thing pan-behind
  promises not to do. Its pointer is a **reticle** — the anchor's own ring with gapped crosshair
  arms, centred on the point the pivot will land at (K-235). It MUST NOT carry an arrow or any
  other tip: the pivot lands in the middle of the ring, and a tip elsewhere claims a place the
  tool does not act at. The layer's live anchor is marked while the tool is armed.
- The Rotation tool's **pointer is a curved arrow**, drawn rather than a system cursor
  (no platform ships one). It MUST lean round the anchor — the curve faces the way the layer
  would turn from where the pointer is — and MUST be tighter towards a corner than along an
  edge, measured in the layer's own space so it follows the layer's rotation. The system
  pointer is hidden over the picture while it is armed and nowhere else.
  **It settles on eight positions and nothing between them** (K-230) — the layer's four edges
  and four corners. A continuously leaning mark was true to the geometry and worse to read: a
  pointer that is never twice the same shape is one the eye re-reads every time.
- **A preview in flight MUST be drawn in flight** (K-230). The picture is previewed at the new
  angle while a turn is being dragged, so the wireframe over it MUST turn with it rather than
  waiting for the document to be written on release.
- A **layer-controls switch** in the Viewer bar (§2.2) hides and shows the boxes, the
  handles and the hover highlight, for judging the picture itself. It governs *drawing* only:
  clicks and drags still select and move, exactly as After Effects' Show Layer Controls does.
- Snapping while dragging: layer edges/centres to comp edges/centres, guides, grid, and
  other layers' anchors/edges. Snapping is on by default; holding `Ctrl` suspends it during
  a drag. Snap matches MUST be indicated visually at the moment of snap.

### 2.3.1 The shape tools and masks (K-222)

- With a layer **selected**, a shape tool draws a **mask** on it. With **nothing** selected it
  makes a **shape layer** at the top of the composition (K-237), holding the art it drew, in the
  toolbar's fill and — when the width is not zero — its stroke. The new layer MUST land where
  the art was drawn and MUST become the selection, so the next drag masks it.
- A shape layer's art lists in its Timeline twirl-down under a **Contents** heading, above Masks
  and Effects: the art is the picture, the masks gate it, the effects process it.
- Each item's **animatable numbers get a row each** under it, as a mask's and a stroke's do
  (K-551): Trim start, Trim end and Trim offset. A row rather than another control on the item's
  own row, because a property without a row has nowhere to put the stopwatch that animates it.
- **Path** is the first of an item's rows (K-606): the shape itself, which has no number, so the
  row carries the stopwatch and its diamonds and no value field. The stopwatch on plants a key
  holding the shape already showing, so nothing moves; off keeps the shape the playhead is over.
  A shape is edited with the drawing tools, and a point drag on a keyed item lands on the key
  under the playhead exactly as a mask's does (K-340) — and the row selects itself when it does,
  so the key is planted on a row the author can see (K-341).
- **Combine** heads the rows of every item **after the first** (K-605): an Apart / Union /
  Subtract / Intersect / Exclude choice saying how this piece of art joins the piece above it in
  the list. The first item has nothing in front of it to join, so it carries no such row. An item
  set to anything but Apart shows **only** that row: it lends its path to the run and nothing
  else, and the run is drawn with the paint and the modifiers of the item that starts it, so the
  rest of its rows would be settings that change nothing. It does not key — a choice has no curve.
- **Fill** and **Gradient** head an item's rows, on any item that has a fill (K-555): a colour
  swatch and a Flat / Linear / Radial choice. **Gradient colour** and the ramp's two points appear
  once the choice is not Flat. Switching a ramp on **aims it at the art's own box** — down it for
  linear, out from its middle for radial — because a ramp that read as one flat colour the moment
  it was chosen would look broken rather than unaimed. None of the three keys: a colour and a
  choice have no curve, so they carry no stopwatch.
- **Offset path** comes before the trim's rows, because it applies first (K-554): one length in
  layer pixels, out of the path or — negative — into it.
- **Dash, Gap and Dash offset** appear under an item that has an **outline**, and only then
  (K-552): three dead rows on a fill-only shape would be three promises the item cannot keep.
  Writing either Dash or Gap on an item with no dash list makes the pair, so there is no separate
  "add dashes" gesture to find.
- **Copies** appears under every item, and the repeater's other nine rows — Copy offset,
  Repeater anchor x and y, Repeater position x and y, Repeater rotation, Repeater scale, Start
  opacity and End opacity — appear only once there is more than one copy to step between
  (K-553). Copies is the row that opens them, so there is no separate "add a repeater" gesture,
  and an item drawn once carries one row rather than ten that describe nothing.
- **All five shape tools drag out** between two opposite corners of the shape's box —
  whichever way round the drag went — with `Shift` keeping the box square. Rectangle and
  rounded rectangle fill the box; ellipse is inscribed in it; polygon and star are the regular
  five-sided and five-pointed figures inscribed in it, first point at the top.
- **The Pen builds a path point by point** (K-223): a click places a corner; a click-and-drag
  places a vertex and pulls a **mirrored** pair of bezier handles out of it, the dragged
  handle leaving the vertex and its reflection entering; holding `Alt` during that drag breaks
  the pair so the entering handle stays where it was. Clicking the **first** vertex closes the
  path, and closing is what applies it. `Escape` abandons the path; `Backspace` takes back the
  last point. The Pen's four siblings — add, delete and convert vertex, and mask feather —
  edit a *finished* path and are not built.
  **The edge to the pointer MUST be previewed as the curve it would be** (K-230), bent by the
  placed vertex's handles: drawing it straight promised one shape and delivered another the
  moment the next point landed. While the *next* vertex's handles are being dragged out, that
  edge MUST run to where that vertex was placed and bend into it by its own incoming handle
  (K-233) — the shape that will exist when the button comes up, drawn as it is aimed.
  **A click that would close the path MUST say so before it is made** (K-233): the first vertex
  and the pointer both take a ring, so "how close do I need to be" is answered on the picture
  rather than by trying.
  **`Ctrl+Z` takes back one point while a path is being built** (K-233), and returns to the
  document's own undo once the path is empty. This is the one place undo means something
  narrower than the last edit, and it must: the points are not in the document yet.
- A mask's path is stored in **layer space**, so it travels with the layer's transform.
- Every selected layer's masks MUST be outlined on the picture, with a mark on each vertex.
- **A mask's points can be edited with the Selection tool** while the layer controls are shown
  (K-224). Each vertex is drawn as a small square, filled when it is selected. A click takes
  the point under the pointer (`Shift` adds to or removes from the set); a **marquee** that
  catches any of the selected layers' vertices takes **those**, leaving the layer selection
  alone, and one that catches none is the layer sweep of §2.3. Dragging a selected point moves
  every selected point, each in **its own layer's** space so a set spanning differently
  transformed layers still travels together on screen. The order a press is resolved in MUST
  be: scale/rotation handle, then mask point of a *selected* layer, then layer body, then
  empty space. The marquee MUST settle the selection on release rather than clearing it on
  press — otherwise the press would drop the layer whose points the sweep is about to gather.
  Bezier **handles** on a finished path are not editable yet, and mask paths cannot be
  keyframed.
- Masks appear in the layer's Timeline twirl-down under a **Masks** heading — above Effects,
  because a mask gates the layer's alpha before its effects run (docs/06 render order) — and
  the heading appears only once the layer has one, exactly as Effects does. The mask's own
  row carries what the mask *is* — its name, its invert switch, and its **mode** under the
  same header a layer's blend mode sits under (K-340) — and its context menu renames and
  deletes it.
- **A mask's values are property rows, and every one of them keyframes** (K-340). Under the
  mask sit **Path**, **Opacity**, **Feather** and **Expansion**, each with the same
  stopwatch, the same ◄ ◆ ► navigator and the same lane diamonds as a transform property,
  and each with its value in the same column an effect parameter's value sits in. Path is
  the shape itself: it has no number, so its row has no field and its lane shows diamonds
  without a curve (K-339).
- **A feather can be a width per point** (K-545), and then a **Point *n* feather** row joins
  them for each point of the shape — the same stopwatch, navigator, diamonds and graph
  channel. The rows appear only once the mask carries the widths, and the mask row's own menu
  is where they are switched on: **Feather per point** gives every point the width the mask
  already had, so turning it on does not move the picture, and **One feather width** drops
  them again. The Viewer draws a dimmer line half a feather either side of the path, which is
  how a varying width is seen before the frame is; a keyframed width has no line, because the
  Viewer does not evaluate one while it paints (K-184).
- **A mask (and a shape item) is renamed in place.** A shape drawn with a tool is named
  after that tool, which is right until two ellipses need telling apart, so the name is
  editable: **double-click** it, or pick **Rename** from the row's menu. `Enter` or a click
  elsewhere commits; `Escape` abandons; an empty or all-space name is refused and the old
  name stands. The whole edit is one write, so it is one undo step. This is not the layer
  rename of K-243 (`Enter` opens that, because a double-click on a *layer* name opens the
  layer) — a mask row has nothing to open, and its single click is already spoken for by
  selection, so the double-click is free. It is counted from two timestamps rather than an
  `onDoubleTap`, because a double-tap recogniser would hold the selecting click back for
  the length of the double-tap window.
- **A mask row is an ordinary property row (K-234).** Clicking its name selects it, with the
  same plain / `Ctrl` / `Shift` gestures every other property row takes (§4.3), and the row and
  the heading over it light up the same way. A **whole opacity drag is one undo step**, not one
  per tick. With a mask row selected, **`Delete` deletes that mask** rather than the layer it
  sits on.

**Implementation status (2026-07-31).** Built: the wireframe, hover, click and Shift-click
selection, the marquee, body-drag move (of a whole multiple selection), the eight scale
handles with `Shift` for uniform, the rotation bar with `Shift` snapping to 45°, and the
bar's switch. Not built: the anchor-point centre handle, snapping of any kind, parent-aware
and 3D gizmos, scale and rotation of a *multiple* selection about a shared box (a multiple
selection moves, and shows a box per layer), and motion paths (§2.4). A layer whose position
is keyframed draws no box: there is no single value for a drag to add to. **Masks can be
drawn, listed, selected, renamed, inverted, faded, deleted (by menu or `Delete`), and their points
selected and moved** (K-224, K-234), and **a shape layer's own art is drawn and edited by the
same gesture** (K-307) — the two hold the same path type, so a point of either is aimed at,
swept up and dragged alike. A shape point is drawn at the art's own coordinates less the
art's bounding-box corner, because the layer's pixels start at that corner; a press within a
point's reach beats a scale handle, the two sitting on the same corners; the drag previews
the picture; and moving the box's edge moves the layer with it so the rest of the art holds
still (K-308). Not built: bezier **handles** on any path, mask or shape (the
model carries no linked/broken tangent flag, so it is a data-model change as well as a
gesture); a **paint stroke's** points, a stroke being a stored gesture rather than a path; and
mask paths cannot be keyframed.

### 2.3.2 The Type tool (K-225)

- With a type tool armed, clicking **empty picture** MUST make a **text layer** where the
  pointer is and begin typing into it; clicking an **existing text layer** MUST edit that one.
  Clicking elsewhere ends the edit and begins the next.
- A layer the tool made that ends its edit with **no text** MUST be deleted: an empty line
  renders nothing, and what would be left is an invisible row in the Timeline.
- The document MUST be written **once**, when the edit ends — one typing session, one undo
  step — with the picture kept in step meanwhile by the text preview path (K-183's family).
  Ending an edit means `Enter`, `Escape`, clicking elsewhere, or putting the tool down.
- **Making the layer is one op, and finishing the edit is one more** (K-230). So the first
  undo takes back the words and the very next removes the layer. It was five ops between them,
  and undo walked back through states nobody had ever seen: an empty box, then the word "Text".
- **`Ctrl+Z` while typing MUST end the edit and then undo** (K-230). The text field swallows
  the chord otherwise, and undo appears to have stopped working.
- The **caret** is drawn by the tool; the text on screen is the engine's own rendering. The
  caret is placed by the same estimate of a line's width the bridge anchors a text layer with
  (half the point size per character), so the two never disagree about where a line ends.
  When true glyph metrics cross the bridge, both sums change together.
- A new layer's **anchor** starts at the left end of its empty line and is recentred on the
  line when the edit ends, with Position compensating so the words do not move (§2.3's
  pan-behind sum).
- New text takes the toolbar's **fill** and **size** (§1.7).
- **The box grows with the words** (K-233): what is being typed is what the wireframe measures,
  even though the document does not hold it until the edit ends.
- **The click is where the words start**: a new layer's anchor begins at the left end of its
  line's baseline, so what is typed runs to the right of the pointer and sits on it rather
  than straddling it — the same relationship the caret is drawn with.
- **Vertical type is not built**: the engine lays out one horizontal line. The member stays on
  the strip and says so.
- Per-character and per-word text animators are a later feature ([TODO.md](TODO.md)).

### 2.3.3 The tools' pointers (K-226)

Every tool MUST say what it is through the pointer, and the ones no platform ships a cursor
for are **drawn**: the system pointer is hidden over the Viewer and the tool paints its own,
as the Rotation, Anchor point and Razor tools already do.

**Windows ships neither a grab nor a magnifier** (K-230). Flutter accepts `grab`, `grabbing`,
`zoomIn` and `zoomOut`; the Windows embedder's table holds none of them and quietly answers
with the ordinary arrow — which is why the Hand and Zoom tools looked like no tool at all. Any
pointer this application needs and a platform lacks MUST be drawn rather than named.

**A drawn pointer MUST follow the pointer whichever button is held, not only the hover**
(K-230). A `MouseRegion` stops reporting a hovering pointer the moment a button goes down, so a
pointer drawn from hover alone freezes where the press landed — inside the very shape being
dragged out. Hover stops for **any** button, including ones the tool does nothing with: taking
the position from the tool's own drag callbacks fixes the left button only, and a right-click
over the Viewer still pins the drawn pointer until the button comes up. The position MUST
therefore come from pointer *move* events, which arrive whatever the button, and the drawn
pointer MUST clear when the pointer leaves the panel. Following the pointer is a drawing rule
only: no tool gains a gesture on a button it did not already handle.

| Tool | Pointer |
|---|---|
| Shape, Pen | The **crosshair** the eyedropper uses, badged with the tool's own icon down and to the right |
| Brush, Clone stamp, Eraser | A **ring** the size of the stroke it would leave, a dot at its centre, badged with the tool's icon |
| Horizontal type | The system **I-beam** |
| Vertical type | A drawn I-beam, **turned a quarter turn** |
| Orbit, Track, Dolly camera | The crosshair badged with the tool's icon (§2.3.5) |
| Rotation | A curved arrow leaning round the anchor (§2.3) |
| Anchor point | The anchor's ring-and-cross (§2.3) |
| Razor (Timeline) | The scissors icon, with the cut line doing the aiming (§4.4, K-235) |
| Razor (Viewer) | The ordinary arrow: it cuts in the Timeline, and a precise pointer here promised a gesture the Viewer does not have (K-230) |
| Hand | A drawn **open hand**, closing while it pans (K-230) |
| Zoom | A drawn **magnifier**, its sign following `Alt` (K-230) |

- A badge MUST be drawn with a halo behind it, so it is legible on a white picture and a black
  one alike, and MUST sit **down and to the right** — above or to the left would cover the
  shape being dragged out.
- The brush ring MUST follow the **magnification**: a width in picture pixels drawn at picture
  scale, clamped so a hairline is still visible and a very wide brush does not fill the window.
- The brush ring MUST be the **brush size** (§2.3.4), so what is under the pointer is the mark
  about to be made.

### 2.3.4 The painting tools (K-227)

- With a painting tool armed, a drag over the picture leaves a **stroke on the selected layer**.
  With nothing selected the tool MUST say so rather than swallowing the press.
- **Brush** lays down the toolbar's **fill** colour; **Eraser** takes the layer's alpha away;
  **Clone stamp** copies from elsewhere on the same layer. The clone stamp MUST refuse to stamp
  until `Alt`-click has set its source, and MUST mark that source on the picture.
- One drag is **one stroke and one undo step**. The stroke is drawn on the overlay while the
  pointer is down and committed once on release. `Escape` abandons a stroke in flight;
  `Backspace` takes the last committed one back.
- The brush's **size, hardness and opacity** sit on the toolbar beside the fill swatch (§1.7)
  and are live. They are the brush's own three, not the shape tools' fill and stroke pair
  (§1.7) — a brush is a different thing that happens to have a width.
- A stroke is stored as the **gesture** in layer coordinates, so it re-stamps at whatever
  resolution the frame is rendered at and every setting stays changeable
  ([03-DATA-MODEL.md](03-DATA-MODEL.md) §7.1).
- Strokes list in the layer's Timeline twirl-down under a **Paint** heading, between Masks and
  Effects — the order the picture is built in — each row named for the tool that made it, with
  its opacity and a menu that deletes it. The heading appears only once there is a stroke.
- Not built: pressure and tilt, non-round brushes, spacing and scatter, write-on (per-stroke
  start and end times), per-stroke blending modes, and painting in Layer view rather than on the
  composite.


### 2.3.5 The camera tools (K-229)

- The three camera tools act on the **active camera** — the topmost visible camera layer whose
  span covers the playhead — regardless of the selection, because the camera is what the
  composition is being looked *through* rather than a thing that has been picked. With no
  camera at all the tool MUST say so.
- Lumit's camera has no separate point of interest: its **position is the point it is looking
  at** (that plane renders 1:1 and centred) and the eye sits its *zoom* — the focal distance —
  behind that, along the camera's forward axis. So:
  - **Orbit** changes the rotations only, swinging the eye round the point being looked at.
    Dragging up MUST lift the camera over the top (tilting it to look down). The pitch MUST be
    clamped just short of the poles rather than wrapped: one pixel past straight down flips the
    picture over.
  - **Track** slides the position along the camera's own right and up axes, *against* the drag,
    so the picture follows the pointer as it does under the Hand tool. The Viewer's
    magnification MUST be undone, so the picture keeps up with the pointer.
  - **Dolly** slides the position along the forward axis by a fraction of the distance already
    in hand, so a wide shot covers ground and a close-up creeps.
- `Shift` locks an orbit or a track to one axis.
- **The pointer MUST be held still for the length of the drag** (K-230) and only its movement
  read. Moving a camera aims at nothing on the picture, so a pointer free to wander leaves the
  Viewer and finally stops in the corner of the screen — ending the drag before the user does.
  Where a platform cannot hold it, the drag MUST fall back to reading the movement between
  events rather than refusing.
- The camera's axes MUST be built the way the compositor builds its matrix (`Ry · Rx · Rz`), or
  a tool sends the camera sideways when it is asked for forward.
- The **gizmo** marks the point the camera is looking at, and the Orbit tool draws the circle it
  swings round. Each tool wears the drawn pointer of §2.3.3, badged with its own icon.
- A camera whose placement is keyframed is left alone — there is no single value for a drag to
  add to, the same rule §2.3's gizmo follows.
- Not built: a **point of interest** (After Effects' two-node camera), the **Unified Camera**
  tool, and depth-of-field handles on the picture.

### 2.3.6 The camera-track point cloud (K-417)

- When a layer carries an enabled **Camera track** whose **Show points** is on and a solve
  exists for its footage, the solved points MUST be drawn over the picture as **depth-cued
  dots**: nearer ones larger and at full strength, further ones smaller and faded, in theme
  colours. Nearness arrives from the engine already normalised over the cloud on the frame
  being shown — the interface draws it, and does not work it out
  ([17-BRIDGE-CONTRACT.md](17-BRIDGE-CONTRACT.md)).
- The cloud is asked for **once per frame change**, never per rebuild — the Levels histogram's
  rule (K-413), and the bridge-call budget is the gate.
- The cloud takes clicks **only while its own layer is the selected one**. Drawn always,
  clickable then: a cloud that always took the pointer would make the whole shot unselectable,
  and clicking the picture is how a layer is selected (§2.3).
- **Selecting**: a click takes the nearest point within reach; `Shift`-click adds (and a second
  `Shift`-click on a picked point takes it away again); a drag on empty picture sweeps a
  **marquee** and takes everything inside it; a click on nothing, and `Escape`, clear. The
  selection is **panel state** — a set of features being worked with, not something the
  document holds — so it does not survive the panel and is not undoable.
- With points picked, a **small floating row** appears under them offering **Create null** and
  **Create solid**. A row rather than a context menu: the gesture that made the selection is a
  drag on the picture, and requiring a second, hidden gesture to act on it would be the
  calmer-looking but slower answer. It MUST stay on the panel when the selection sits near an
  edge.
- Either command adds a **3D layer at the mean solved position** of the picked points, turned
  to face the camera at the frame the selection was made on. Naming nothing that was solved
  MUST be refused rather than putting a layer at the origin.
- A **solve-linked Camera layer** wears its state as a calm badge in the Transform heading of
  the Effect controls panel — *following the solve*, *holding the last solved frame*, or *the
  solve could not be found* — with **Convert to keyframes** beside it. The transform rows stay
  editable: what they hold while linked is a **correction** added to the solved motion (K-578),
  and the badge is what stops that being a surprise. Once one has been made, a small accent
  **dot** appears beside the badge and on the Camera track effect's status row — *edited since
  track* — and **Clear corrections** appears beside the dot, offered only while there is
  something to clear.
- Not built: hiding points behind the shot's geometry, a point count or filter, deleting a
  point from the cloud, and setting the ground plane and origin from a selection.

### 2.4 Motion paths

Position animation MUST draw its motion path in the Viewer for selected layers: keyframe
boxes, spatial bezier handles (editable in place), and per-frame dots so dot spacing shows
speed. Path editing writes to the same keyframe data as the graph editor.

### 2.5 Playback surface

The Viewer shows the render pipeline's current frame; everything about playback belongs to
the transport (§11) and cache system. During scrubs the Viewer shows latest-wins progressive
results (K-017); stale frames MUST never be presented as current without the degradation
indicator lit.

**Preview progress (K-276, moved by K-287).** A frame the user is waiting on — a scrub, a
playhead move, a dragged value — MUST be able to say how far it has got: a slim bar on the
**right-hand end of the Viewer's transport bar**, filling as the engine works through the
frame, labelled with the stage it is in (preparing, reading media, reading the
composition, compositing, showing). It MUST NOT be drawn over the picture: the one thing
the Viewer exists to show is the picture, and covering its bottom edge exactly while a
frame is being waited for covers it when it is being looked at hardest. The bar's place on
the transport is its own — the controls take the space that is left, so the bar arriving
and leaving MUST NOT move any of them. Three further rules make it a help rather than
noise:

- It MUST NOT appear during playback. A frame due in sixteen milliseconds has no use for a
  progress bar, and one blinking per frame would be the busiest thing on screen.
- It MUST NOT appear for a frame that arrives quickly: nothing is drawn until a render has
  been outstanding for ~150 ms, so ordinary work stays silent.
- The fill MUST animate towards each report rather than jumping, and MUST carry a moving
  sheen while it waits, so "working" reads apart from "stuck" at a glance. Both respect the
  animation level (K-092).

The fraction is an estimate built from fixed stage weights and is described as such; a
progress bar's job is "roughly how much longer", and a decimal point would not make that
truer. A frame served from the cache reports nothing at all — there was nothing to wait for.

### 2.6 Viewer locks

Each Viewer MAY be locked to a specific item (padlock on its tab). A locked Viewer MUST NOT
switch items when the user opens another comp — this is how "comp here, precomp there"
two-Viewer workflows survive. Locks are project state (§1.5).

---

## 3. Project panel

The library of assets: footage items, audio items, comps, folders.

### 3.1 Structure

- A folder tree with drag-to-reorganise: dropping rows on a folder row files them there,
  and the row menu's **Move to folder** lists the project's folders for the rows that do not
  drag. Either way the whole selection moves together as one undo step, an item leaves its
  old folder in the same step it joins the new one, and **Move to root** unfiles. A folder
  is never offered — or allowed — inside itself or inside its own descendant. Sorting per
  column; columns: name, type,
  dimensions, frame rate, duration, colour space tag, file path. Column set is configurable
  (workspace state).
- **Under the redesign (K-448, built to the mockup 2026-08-24)**: the panel has a bottom
  bar with **Folder** and **Composition** buttons — dropping an asset on Composition makes
  a comp to match it, exactly as the New composition button does today; asset **colour
  tags** tint the item icon's strokes rather than adding a dot; the **path** column sits at
  the right of the list; and the preview card carries name, size, rate, duration and codec.
  **Shipped, all of it.** The bar carries **Folder**, **Composition** and **Import** —
  Import because it is a command the panel has always had and the mockup gave it no other
  home, which is also why its word is the first the bar sheds as it narrows. A new folder
  is filed inside the picked folder when one is picked, at the root otherwise, and takes
  the next unused "Folder N" when nothing is typed. The **Path** column carries the
  *folder* a footage item points into, hushed to `text_disabled`, empty when the project
  records the file by bare name — the Name column two cells left already says the file
  name, and a column repeating it would not be context. The **`in use`** badge marks any
  item a composition places, hidden switches included. A **colour tag** tints the row's
  glyph rather than adding a dot beside it, is set from the row menu's chip strip, and the
  **swatch filter inside the search well** narrows the tree to one colour — on the colour
  a row is *wearing*, a folder's handed down included (K-634, K-567), so a colour finds
  everything filed under a folder of that colour. The preview card's second
  line names the **codec** and the sound's rate and layout, and a **still** says so where
  a rate and a length would be — which retired the zero-picture-width guess that called a
  silent still an audio file.
- A persistent **search field** filters the tree live (name, type, extension). `Ctrl+F`
  focuses it when the panel has focus.
- Selecting an item shows a header readout: thumbnail, dimensions, fps, duration, codec,
  colour space tag, alpha interpretation.
- **Hover-scrub thumbnails**: hovering a footage item's thumbnail and moving horizontally
  scrubs a low-resolution preview. This MUST be served from proxy/thumbnail data only and
  MUST NOT trigger full decodes. Double-click opens the item in a Viewer (footage mode).
  **Shipped (owner request, 2026-07-28):** selection lands on the pointer's *down* stroke.
  A second click on the lone selected row **opens** it (§4.2); it no longer renames
  (K-321) — `Enter` on the selection does, as it does in every panel. Double-clicking
  empty panel space imports. The footage-Viewer double-click above is deferred until
  footage mode exists; comps front via the Timeline's comp tabs.
- Drag an item into a comp's Timeline or Viewer to create a layer; drag onto the
  **New comp** button to create a comp matching the footage (dimensions, fps, duration).
  **Shipped:** rows multi-select — `Ctrl`/`Cmd`-click adds or removes one, `Shift`-click takes
  the run between the last click and this one, a plain click goes back to one — and a drag from
  any selected row carries the whole selection, so several clips reach the Timeline, or the
  **New composition** button, in one gesture. A drop on that button opens the composition
  settings dialogue (§13.3) prefilled from the media: the size and rate of the first item that
  has a picture, and the length of the longest, because a comp shorter than what was dropped
  into it would clip the very thing that was asked for. Pressing Create makes the comp and
  places every dropped item in it as a layer.

**An image sequence imports as one row** (K-539, [03-DATA-MODEL.md](03-DATA-MODEL.md) §3.1).
The file dialogue offers files, not folders, so picking any single still out of a folder of
numbered ones imports the whole run — named for its span, `frame[0001-0050].png`, so the row
says both that it is a run and where it stops. Picking more of the same run (which is what
selecting a whole folder does) adds nothing further: the item that is already there is the
answer. A numbered still with no numbered neighbours stays a single still, and a folder of
numbered `.mp4`s stays a folder of clips. **Shipped**, apart from the rate control: a
sequence plays at 25 until §3.2's dialogue exists to change it.

### 3.2 Interpretation dialogue

Per footage item (context menu → *Interpret footage…*), stored in the project, never
touching the file (K-024):

- **Frame rate**: use file rate, or override to an exact rate (the 240 → 60 fps workflow is
  first-class; common capture rates offered as one-click choices).
- **Alpha**: ignore / straight / premultiplied (with matte colour), plus a *guess* action.
- **Colour space tag**: the footage's colour space for conversion into the working space.
  Until this dialogue exists as drawn, the row's list is carried by a **Colour space** submenu
  on the item's context menu (K-490, docs/impl/ocio.md §6.5) — the built-in interpretation,
  then the loaded configuration's own names — and that submenu is replaced when the dialogue
  lands.
- **Loop**: loop count for stills/sequences and short loops.
- **Sequence frame rate** (K-539): for an image sequence this is not an *override* — stills
  carry no rate of their own, so the item's rate is the only rate there is. It defaults to 25
  and is where an imported sequence's speed gets corrected. This is the one part of a
  sequence the project stores; the run's start and length are re-read from the folder.
- **Fields/pulldown**: deliberately out of scope for v1 (gaming footage is progressive);
  the dialogue reserves space for it.

### 3.3 Proxies and relinking

- Each footage item shows a **proxy badge** when a proxy exists (glossary §5): states are
  *none / generating (progress) / ready / stale*. A global proxy toggle
  switches all previews between proxy and original.
  **Shipped (K-501):** the row wears a colourless `proxy` badge while its own
  *use proxy* tick is on, and the row menu carries the four commands on footage
  rows — **Set proxy…** (a file picker), **Make proxy** (the transcode), a
  ticked **Use proxy**, and **Clear proxy**, the last two offered only once
  there is a proxy to act on. MAKE-PROXY is background work of the export's own
  shape, so it reports where the export does: the status line, with its
  progress and a Cancel that works from anywhere. The **project-wide switch is
  on the panel's bottom bar**, after the new-item controls and a hairline —
  this panel has no header band of its own (§12A.3a's six bands are the
  mockup's, and the search row's width is pinned to the pixel), and the bottom
  bar is already where its panel-wide controls live. *Generating* and *stale*
  are not badge states: progress is on the status line, and the engine forms no
  opinion about a stale proxy (K-501 has the renderer fall back silently).
- **Missing footage** shows a distinct badge and renders as a placeholder slate in comps
  (never a crash, never a silent black). The **relink flow**: *Relink…* opens a file picker;
  on relinking one item, Lumit MUST scan the chosen folder for the project's other missing
  items by filename and offer to batch-relink the matches. A *Find missing footage* search
  filter lists all missing items in one view.
  **Shipped:** `MediaStatus::Missing` is its own state, distinct from an unreadable file —
  the project row wears a crossed-link glyph in the warning tint and a `missing` badge
  which is itself the relink control — clicking it opens the picker, which is where the
  separate *Relink…* button's job went when the row was rebuilt to the mockup (the mockup
  gives a broken row a badge and no button) — and the layer renders **generated colour bars**
  (`lumit_media::slate`, drawn at comp size, never a bundled image) in preview *and* export,
  so the mistake cannot hide in a delivered file (K-031). Relinking one item batch-relinks
  every other missing item whose name is found beside the chosen file, in a single undo step,
  each rebased relative and re-fingerprinted (K-173). **Find missing footage** is a toggle
  on the panel's bottom bar — the `· n missing` half of its count, shown only while
  something *is* missing — and a right-click entry on any footage row; it narrows *with* the search text rather than
  replacing it, and unlike the plain search it is never widened by a folder whose own name
  matches, so every visible row is something to fix. An empty result reads "Every file is
  where the project expects it" rather than an error (§13's no-punishment rule). Still to
  build: a dedicated slate for a file that is present but **unreadable** — that state shows
  the row's "unreadable" note and no picture.

---

## 4. Timeline

One Timeline panel; one tab per open comp. Left: the layer outline. Right: time ruler and
lane area. The divider is draggable. **Interaction is specified in
[impl/timeline-interaction.md](impl/timeline-interaction.md)** (K-499, K-500): the
selection model (marquee from any ground, additive `Shift`, a property's name selecting
its keys), the cursor and hover vocabulary, the drag rules (one undo step, live preview,
Escape reverts), and the audited gap list — read it before touching any gesture in this
section or §5.

**A comp that stops existing cannot stay fronted.** Deleting the fronted comp, or undoing the
pre-compose that made it while standing inside it, leaves the panels pointed at something the
engine no longer has. The Timeline MUST front something else instead, in this order: the comp
the user was in before this one, if it is still there; else the nearest open tab, looking left
before right; else nothing at all, which is the state the panel already draws a placeholder
for. This has a regression test.

### 4.1 Time ruler region

Top to bottom: the **time ruler** - one band, with its clock near the top, the
**markers ribbon** and the **work area bar** covering the whole of it (K-513), and — on
the band's own row at the ruler's floor — the **cache bar**; then layer lanes.

- **Markers ribbon**: comp markers (point or span) with labels; double-click the ruler's own
  ground creates one and opens its label editor (built, TI-9);
  drag to move; markers snap. **Beat markers** (generated via the Audio panel, §12) render
  in the same ribbon, visually distinct, and behave as first-class snap targets. Layer
  markers render on the layer's own row.
- **Work area**: `B` and `N` set start/end at the playhead; drag the ends; double-click the
  band to reset to the full comp (built, TI-9 — the reset writes the engine's own "not
  narrowed", which is what the whole comp *is*). Work area is the preview range and default export range,
  and playback **loops** it (§10's default loop mode): reaching its end resumes from its
  start. It draws as **one band in `animated`** (K-441,
  [15-DESIGN.md](15-DESIGN.md) §12A.1): the ruler's **second row** — K-513 gave it the whole
  height and K-529 reversed that half after desktop testing, a wash over the clock making
  the numbers harder to read — then on behind the cache bar and down through the lanes,
  behind the bars and the keys. Its **two drag handles keep the ruler's whole height** (the
  half of K-513 that stands) and are **drawn**: a small vertical rounded tab in the band's
  own colour a step stronger than its edge, with another step under the pointer. The
  **double-click that clears it** is the
  one gesture the ruler's waist still divides: below the waist clears the work area, above
  it makes a marker. It has to stay divided — a comp nobody has narrowed has a work area
  of the whole comp, so a band-wide double-click would leave nowhere on the ruler to make
  a marker at all.
  Outside it the lane ground stays a step darker, drawn both under the rows
  and again, lightly, over the layer bars — under them alone it was invisible along every
  row that had a layer in it (K-207).
  A comp that has never had one set **reads as the whole comp** (K-203) — the engine's
  "not narrowed" is null, but the interface has no such state, and a bar with no ends is
  a bar nobody can take hold of. Clearing the work area widens it back to the comp.
- **Cache bar**: a thin stripe showing cached frames per tier — VRAM, RAM, and disk caches
  as three distinguishable states (visual treatment in [15-DESIGN.md](15-DESIGN.md)).
  The bar MUST update live as background rendering fills the cache (K-016).

**Shipped header arrangement (K-188).** The comp tabs span the panel; each tab is an
*open* comp — fronting a comp opens its tab, its × closes only the tab, and closing the
fronted tab fronts its nearest neighbour. The strip is in the user's order, not the
project's: a tab dragged onto another takes its place, and the order rides along in the
session. **Fronting a comp puts you back where you were in it** (K-624): the playhead
returns to the frame it was left on and the lanes to the magnification and scroll they
had, so a comp reopened from the strip is a return rather than a fresh start at frame zero
fully zoomed out. Like the Viewer's exposure (K-314), the preview resolution (K-357) and
the region of interest (K-362), this is session state — it rides in the session blob and
so in the `.lum`'s `ui_state` (K-245), never in an op, so a scrub never lands on the undo
stack or makes the project dirty. **A tab that is not the fronted one outlines under the pointer** (K-640): the value well's own hover edge, one pixel of `hairline_strong`, drawn over the tab rather than inside it so nothing on the strip moves as the pointer travels. The fronted tab is already marked by its seated surface and adds nothing under the pointer. Right-clicking a tab opens **Composition settings…** for that comp, the same
dialog the Project panel's context menu opens, reached from the comp being worked in. Below them the outline carries two header rows
of its own: the **toolbar** (the playhead as `HH:MM:SS:FF` timecode plus a zero-based
frame readout `f72` — both in **fixed-width slots** and both **click-to-type**, per K-287:
a time typed into either moves the playhead, and one outside the composition lands on the
nearest end — the layer search, the master motion-blur button, the shy filter, the
Lane and Graph view buttons, and a ⋯ menu with the layer / razor / work-area / marker /
beat commands) and the **column-group header** (§4.2). The lane side gives those two
rows' height to a taller, labelled time ruler — a bigger playhead grab — with the cache
bar tucked under it. The ruler is **double height and reads as one band** (K-441, K-513):
the time labels, the ticks and the playhead's head near the top, the markers on the floor,
and nothing ruled across the middle. A labelled tick crosses the waist and reaches the
same distance below it; minor ticks subdivide above the waist as the zoom grows, and stop
at one tick per frame. **Both of the outline's chrome rows grew under Regular** (K-512) -
24 for the toolbar, 23 for the column-group header, with every control standing in them
told to be 20 - and the ruler, derived from their sum, grew with them to 47. Compact keeps
18, 18 and 36. A few pixels of padding sit either side of the axis, so a handle on the
first or last frame stays visible and grabbable; the lanes carry the same padding, so both
halves stay lined up. Markers draw on the ruler itself rather than in a ribbon of their
own (K-254): an **upward triangle** standing on the cache bar at the floor of the ruler,
centred on the frame it marks so its point aims at the clock above. What a marker says
rides in a backdrop **pill** that starts at the triangle's point and runs right, half the
triangle inside it and half outside to its left, not as loose text over the ticks. Styling — a grey `marker` token, editable like any other
— is in [15-DESIGN.md](15-DESIGN.md) §6.4.

**One marker per frame**: adding one where a marker already sits replaces it, and so does
dragging one on top of another. Two flags on one moment are two things to click and one
place, and the second hides the first exactly. A flag can be **dragged** along the ruler —
the document hears about the move once, on release, not per frame crossed — and
**right-clicking** one offers *Edit marker…* and *Delete marker*. The separate ribbon, span
markers and double-click-to-create are still to come.

**Layer markers (K-254)** draw on the layer's own bar, in the same flags, and travel with
it when it is moved. A layer's markers are **its own copy**: dropping a composition into
another brings that comp's markers along as the layer's, and from then on the two lists are
unrelated — deleting one on a layer never reaches into the composition it came from, or
into anywhere else that composition is used. Right-clicking a flag on a bar offers *Edit
marker…*, *Delete marker* and *Delete all markers*; the layer's own row menu carries
*Delete all markers* too, and only when there are some. Pre-composing copies the comp's
markers **into the new composition** (shifted with everything else when the dialogue's
*Adjust duration* moves time back to zero, and any falling outside the new span are left
behind) and leaves the Precomp layer with none — those cues are on the ruler above it
already, and drawing them on the layer as well would say the same thing twice.

### 4.2 Layer outline columns

Default column order, all reorderable and hideable per workspace:

**Opening an item (K-191, K-243).** A double-click — or a second click on a row that is
already the whole selection — **opens** what it lands on, and what opening means is the
item's own answer:

- a **composition** fronts in the Timeline, which is what a double-click means everywhere;
- **footage** raises the **New composition** dialogue on the selection, already the media's
  own size, rate and length (the longest item wins when several are selected), with every
  selected item landing in the finished comp as a layer — footage has no window of its own,
  and a comp to put the clip in is what the gesture is asking for;
- a **folder** shows or hides what is in it. A caret on the row says which it is, and a
  search still looks inside a shut folder.

Items are therefore renamed with **`Enter` on the selection** (K-321, §15) or from the row
menu (**Rename**) — and a comp also from its settings dialogue — never by a second click on
the row. Dropping footage on a Timeline with nothing open raises the same **New
composition** dialogue.

**One rename gesture, everywhere (K-321).** No surface renames on a double-click or on a
click on an already-selected thing: both gestures mean *open*, and a rename that shared
them opened editors under people's pointers. `Enter` renames whatever the focused panel has
selected — a layer in the Timeline, an item in the Project panel, an **effect** in Effect
controls (the effect's heading only; parameter rows are not renameable). The binding is
per-panel and live in the focused panel alone, so one press can never open two editors. An
effect's given name is stored on the instance and shown in place of the effect's label
wherever the stack is drawn, in the panel and in the Timeline's fold-out alike; clearing it
falls back to the label.

**Opening a layer (K-243).** Double-clicking a layer in the Timeline outline opens it the
same way: a **Precomp** layer fronts the comp it draws, and every other kind will open in a
Viewer of its own once there is one to open — until then a double-click on one does nothing.
Entering a precomp this way lands the playhead on **the frame that layer is showing**
(K-624): the moment is mapped through the layer's start offset and its Retime, so a
half-speed precomp opens on the frame on screen rather than on the outer ruler's count.
Standing before the layer's span opens the nested comp at its start; at or past its end,
at its end. Its remembered magnification and scroll still come back — only the playhead is
overridden.
It is never a rename; `Enter` is (§15) — and an inline rename commits when the pointer goes
down anywhere else, not only on `Enter`.

**A modal window's keys are its own (K-243).** Panel commands stand down while a dialogue is
open. A dialogue's default action takes focus when the window opens, is drawn with the accent
edge, and `Enter` presses it.

**Escape dismisses (K-319, K-575).** Every modal MUST answer Escape by dismissing — the same
answer a click on the scrim gives. It does so by claiming the **dialogue rung** of the Escape
ladder (§14.1) rather than by adding a key handler of its own, so a drag being abandoned or a
menu being closed inside the window is not also the window shutting.

**Escape cancels an inline editor too, and writes nothing (K-323).** An inline rename or an
open value box is not a modal, so `DismissIntent` reaches nothing above it; each such editor
MUST therefore answer Escape on its **own focus node**, ahead of the shortcut system. Every
other exit commits (§4.3, K-243) — Enter, clicking away, losing focus — so Escape is the one
way out that keeps the old name or value. It applies to all four surfaces that have one: an
effect's name, a layer's name, a Project item's name, and any value box being typed into.

**Every window, not just one (K-319).** That rule is now the shape of *all* of them: each
confirmation or settings window names one **default action** — the affirmative one, or the
safe one where the affirmative is destructive — and that button carries the accent edge and
holds focus from the moment the window opens. `Enter` presses **whatever is focused**, not a
hard-wired button: Tab moves the focus and Enter presses the control it landed on, so a
window is fully operable without the mouse. Every house control is focusable and shows the
accent focus ring while it holds focus (docs/15 §6.5); buttons, checkboxes and radios answer
`Enter` and `Space`, and a focused value box opens its editor on `Enter`. While a house
control holds focus its keys are its own, exactly as a text field's are — a panel command
never fires underneath it.

**Tab order is reading order (K-319).** Focus moves left to right, then top to bottom, by
where controls actually *are* on screen — not by the order the layout code happened to
compose them. A modal is its own focus scope, so Tab cycles within the window and never
wanders into the panels behind it.

1. **Index** (render order; bottom layer renders first).
2. **Name / source toggle**: click the column header to flip between the user-given layer
   name and the source name. Rename with `Enter` (a double-click *opens* the layer,
   K-243).
3. **Switches** (glossary §2): visible, audible, solo, lock, shy, quality (draft/full),
   motion blur, adjustment, 3D, collapse (Precomp layers). One icon each; the comp-level
   shy filter button lives in the Timeline header. `Alt`-click a switch applies it
   exclusively (solo-style) where that makes sense (visible, solo).
4. **Blend mode** dropdown.
5. **Matte** dropdown + pick-whip: choose any layer in the comp as this layer's matte
   (AE 2023 semantics — glossary §6), with alpha/luma and invert toggles. One matte layer
   MAY serve many layers. The menu offers only layers that *have* a picture to gate with —
   never a camera, an audio-only clip, or the layer itself (K-194). Layer-valued **effect
   parameters** (a depth map, a displacement source) are filtered the same way.
6. **Parent** dropdown + pick-whip.
7. Optional columns: in, out, duration, stretch.

**Shipped arrangement (K-188, superseding K-168's; extended by K-276):** the columns sit in
FIVE groups, left to right — 1 visibility · audio · solo · lock · shy · guide; 2 twirl ·
label-colour chip · layer number · name; 3 fx bypass · motion blur · 3D · adjustment · flow
· collapse (K-632, superseding K-483/K-484's ordering: fx leads the column, collapse has
its own cell after 3D rather than sharing K-168's flow-or-collapse slot, and flow stands
immediately left of it);
4 matte · blend · parent (dropdowns; the pick-whips are a follow-up); 5 **render time**. **Dragging a group's header moves the
whole group**, which is how the column order is changed, and **dragging the seam after a
group resizes it** (K-192) — every other group keeps its width, so the outline grows or
shrinks by what the drag moved, and what sits inside a group grows with it: the fold-out's
value cells span the render group, and the compose group's three pickers share theirs. The
header icons are indicators only, and the switches live on the rows. Visibility and audio swap glyph when off (closed
eye, muted speaker) rather than only dimming. **The bottom bar carries a toggle per
group** (K-448, 15-DESIGN §12A.1): a kicker naming each of the A/V, switches and
matte/blend/parent groups, lit while its columns are drawn, so the outline pares down to
names and bars when the columns are not in use. The identity group has no toggle — names
and bars are what "pared down" means. A hidden group takes no header, no cells and no
width, exactly as an unmeasured render-time column does, and whatever lined up with it
(the fold-out's value cells) moves to the outline's own right edge rather than off the
panel. The state is the session's, like the order and the widths. **The switches are drawn
from Lumit's own icon set** (K-440) wherever the set has the mark — visible/hidden,
audio/muted, solo, lock/unlocked, and the twirl; the rest keep their Iconoir glyph until
the set grows one. **Shy** is a real switch on the layer: it
hides the row from this list while the toolbar's shy filter is on, and never changes what
renders. **Guide** is the opposite pair (K-497): the layer draws in the Viewer as it always
did and no file Lumit writes contains it, at any depth and whatever the solos say.
**Shipped:** its cell is the sixth in the switches group, beside shy, drawn on
every layer kind — any layer can be reference-only — with the set's own Guide
glyph lit `text_primary` on and `text_muted` off. The switches group is
therefore `6 × switchCellWidth`. **Lock** holds the layer still where the gestures live — bar move/trim, razor,
rename, reorder and delete all refuse — though property-row edits are not yet guarded
(docs/TODO.md). The flow cell awaits per-layer optical flow in the engine; it is drawn on
footage and **collapse now has a cell of its own** (K-632), drawn on a Precomp — so a
Precomp that is also retimed footage no longer has to choose which of the two switches its
one slot shows. Each is blank on the kinds it does nothing for, keeping the column the same
width on every row. Quality and
preserve-underlying-transparency still await their backing machinery (K-168);
hide-per-workspace and the optional in/out/duration columns remain open. Right-clicking a
layer row opens the **layer menu** — duplicate, reorder, delete, and the ticked **Accepts
lights** entry (K-361), which is where that setting lives now that it has no cell in the
Modes column (K-483). The cell it freed is the **adjustment switch** (K-537): on, the
layer's own picture is set aside and its effect stack runs on the composite beneath it;
off, the layer is itself again with everything — source, masks, transform, effects — where
it was. It is drawn on **every row that shows something in the Viewer**, whatever its own
visibility switch says, and left blank only on the four kinds with no picture to set aside
(camera, light, null, audio). Lit `text_primary` on, `text_muted` off, one undo step each
way, and applied to the whole selection like the switches beside it (K-523). The Modes
group is six cells wide on every row all the same.

**The render-time column (K-276, [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md) §7.1)**
shows what each layer's own picture cost in the frame at the playhead, and — on a layer
twirled open — what each effect in its stack cost, on that effect's heading row and in the
same column — an effect's figure MUST sit in the same column as its layer's, or the two
cannot be read against each other. Measuring is **on by default**, and its switch is the
**clock in the bottom strip**, after the cache meters: the column header MUST be a plain
readout, because a header that says Time over a column of dashes gives no hint that it is a
button, and a switch nobody can find is a feature that does not work. Switched off, the
column MUST disappear from the outline entirely — no header, no cells, no width — and the
per-effect figures in the Effect controls panel with it: a column of blanks is not a column,
and the outline's width is worth more than an indicator nobody has asked for. The header MUST report the whole frame's
cost while measuring — `…` until a measured frame has arrived, the number once one has — so
the three states (not measuring, measuring with nothing back, measured) read differently
rather than all showing a dash; and an engine that refuses the switch MUST say so in the
status line rather than leaving a lit clock over a column that will never fill.

Measuring makes the engine wait for the graphics card at every node: an honest millisecond,
at the price of the overlap a brisk preview lives on. Only a composited frame has per-layer
costs to report — a held frame cost a copy — but a held frame MUST still be served at once
and composited again, measured, when the worker is next idle (K-420): the cache bar's green
is a promise that the frame is ready, and the column filling a moment later is the price of
the numbers, not the picture. Switching measuring on MUST ask for the frame under the
playhead again, so the numbers appear where the user is looking.
Playback MUST never be measured whatever the switch says, and switching off MUST drop
the numbers rather than leave a stale frame's costs on screen. The same per-effect number
appears on the effect's title row in the Effect controls panel (§6), from the same
measurement — the panel shows the numbers, it does not turn them on.

### 4.3 Layer lanes and property twirl-down

- Each layer row twirls open (`click` the caret, or property-reveal shortcuts, §15) into
  **property lanes**: Transform group, Masks, Effects (one group per effect), Audio, and
  Retime when enabled. Property groups nest with indentation. Opening a layer's twirl does
  not auto-open its Transform sub-group — it starts collapsed, so the twirl first shows a
  tidy list of section headings, each in its own subtle full-width bar, and you open only the
  group you want. In Effect Controls (and the Effects group here) an effect's name is a drag
  handle for reordering the stack.
- **A section heading is selected by clicking its name, and twirled by the same click**
  (K-300). A *modified* click — `Ctrl` to toggle, `Shift` to extend the run — only selects, so
  picking several effects does not open every one of them on the way past; the twirl mark
  beside the name always twirls and never selects. An **effect's** heading picked this way is
  what **Copy**, **Cut** and **Copy effect** act on, and the pick is shared with the Effect
  controls panel (§6), so an effect chosen in one place is lit in the other. Several picked
  effects copy as one `.lumfx` document in stack order.
- Each animatable property lane shows: stopwatch (keyframing on/off), value with
  **scrub-drag** and click-to-type numeric entry, expression toggle, and its keyframes as
  diamonds on the lane. Keyframe icons reflect interpolation (hold/linear/bezier), matching
  the graph editor.
- **There is no auto-key** (K-447): the stopwatch is the whole model. A property animates
  exactly while its stopwatch is lit — from then on every edit at the playhead is a
  keyframe — and an edit to an unlit property sets its constant value. There is no record
  mode to leave on by accident, and whether an edit keys is always visible on the row that
  takes it.
- Keyframe interaction on lanes: click to select, box-select, drag to move in time,
  `Alt+drag` a selection's end to scale the group's timing, `Ctrl+click` a lane to add a
  keyframe at that time, right-click for interpolation and *Ease* commands.
- `U` reveals animated properties of selected layers; `UU` reveals all modified properties;
  a third `U` within half a second shuts the layer again (After Effects' own cycle).
  **Shipped (K-199).** The taps are counted in the panel — a multi-tap is a gesture, like a
  double-click — and *which* groups qualify is asked of the engine each time
  (`LayerReference::reveal_groups`), because "holds a keyframe" and "changed from a fresh
  layer" are facts about the document. A reveal starts from the layer shut, so it shows
  exactly what it says rather than adding to what was already open, and a layer with
  nothing to show stays shut rather than opening onto empty headings.

  **The first tap stops at the keys** (K-622). `U` opens down to the keyframed *rows* —
  each heading over one is kept so the row is placed, and nothing else under that heading
  is drawn: effect name → effect → keyed row, Transform → keyed property. It used to open
  the qualifying *groups*, and a group opens whole, so one keyed Intensity unrolled every
  other parameter of that effect and one keyed Position unrolled every transform property
  beside it. `UU` is where a heading still opens whole. The filtering is the Animated
  strip's own (§6.43) asked of the revealed layers rather than of the comp, and a layer
  stops answering the reveal the moment any caret under it is turned by hand — otherwise
  the caret would look broken.

  **Shipped (partial):** the caret on each layer row opens onto the **section headings**, each
  with its own caret, and nothing under them until one is opened — the tidy-list behaviour
  above. Three groups exist, plus the Retime row above them:

  - **Retime**, only on a layer that has been given one with **Ctrl+Alt+T** or
    Composition ▸ Enable Retime (K-197, K-198, K-200): a single
    row, not a group, sitting *above* Transform and outside every group — it decides which
    moment of the source the rest of the fold-out then transforms. Its value is that source
    time in seconds, and it is an ordinary keyframable property: the same stopwatch, the same
    navigator, the same lane diamonds and the same graph lane as Position, with nothing
    Retime-specific attached. Switching it on installs the identity map, so the picture does
    not move; switching it off removes the property rather than flattening it. **A map
    flattened to one constant removes it too** (K-329): turning the stopwatch off, or deleting
    the last key, means "no more retime" on this property, so the layer is re-hung on its
    source and plays at source rate again rather than freezing on a single frame. A freeze is
    still asked for the way After Effects asks — a map with one key holds that moment.
  - **Transform**, always: one row per property group with the stopwatch, the ◄ ◆ ► navigator,
    the label, and a scrub-drag/click-to-type value per axis. **Right-clicking the name of
    Anchor point, Position or Scale offers Separate axes** (K-571, docs/03 §6.5): each axis
    takes a row of its own, with its own stopwatch, its own lane and its own curve, and
    *Combine axes* puts them back — merging the axes' key times as it goes, exactly, so the
    picture does not move. **Scale is linked by default** and draws one box, an edit holding
    the x:y ratio (K-072); *Unlink axes* gives it a box per axis. Each command is one undo
    step. The Viewer's gizmo is unchanged by any of it: it reads the resolved transform,
    which has always been per-axis.
  - **Effects**, only when the layer has any: one row per effect, opening onto that effect's
    parameters — the same rows, with the same controls, that the Effect controls panel shows.
  - **Audio**, only when the layer's source actually carries sound (`LayerReference::has_audio`
    probes the container): the layer's **Volume** in dB, keyable like any other property. Every
    layer has a Volume in the model, but on a solid or a title it can never be heard, and a
    control that cannot do anything is worse than no control.

  The rows are one implementation shared with the Effect controls panel
  (`transform_rows_frb.dart`, `effect_param_row_frb.dart`) rather than a second copy, so a
  parameter behaves the same wherever it is shown. A drag stages the value, previews it through
  the engine's patched clone, and commits once on release: one undo step for the gesture.
  Every scrub-drag obeys the modifier convention After Effects uses, with the study's
  fourth rung under it: holding `Shift` is coarse (×10 per pixel), nothing held is ×1,
  holding `Ctrl` is fine (×0.1) and holding `Alt` finer still (×0.01) — coarse wins where
  two are held at once — and pressing or releasing a modifier mid-drag takes effect at
  once. **All four rungs are shown while the drag runs**, in a small floating chip above
  the field with the one in force boxed (`ScrubLadder`, docs/impl/timeline-interaction.md
  polish 27): a sensitivity nobody can see is a sensitivity nobody finds. The chip is
  summoned by the gesture and gone on release, like every other transient hint (K-439's
  §12A.5 discipline). The
  fold-out is worked out once as a list of rows (`layer_fold_frb.dart`) that *both* halves of
  the table walk — the outline drawing each row, the lane side leaving its height — so bars
  cannot drift away from their names. Each property row leads with its keyframe controls —
  the stopwatch, and once animating the ◄ ◆ ► buttons, the ◆ filled exactly while the
  playhead sits on a key — then the name; the value cells all share the span of column
  group 3 (K-188), aligned to both its edges, so the numbers stack into one column wherever
  the groups are dragged. **An animated value stays editable** (K-189): the field shows the
  value under the playhead, and an edit writes the key sitting there — or plants a linear
  one — never flattening the curve. **A driven parameter is the one exception, in every
  panel that draws a property row** (K-471, K-627): where a driver is wired to it, the
  hollow ring and the word *driven* take the keyframe controls' place — there is no key to
  plant and no neighbour to step to while the wire decides the value — the driver's name
  rides in the mark's tooltip, and the value cell keeps showing the number the parameter
  holds while refusing every gesture on it. A drag on one is **one undo step**, staged in Dart and
  committed on release (K-192). **Clicking a property's name selects it** (K-196): the
  name, not the whole row, so grabbing a value field or a stopwatch never re-aims the
  graph — though *editing* a value or keying the property selects it too. `Ctrl`/`Cmd`-click
  toggles a property in and out of the selection and `Shift`-click takes the visible run of
  rows between, across layers; every selected property is a coloured curve in the graph
  editor and its label text takes its curve's colour. Every row containing a selected
  property — its group heading, its effect, its layer — marks itself a shade dimmer;
  selecting keyframes on a lane selects their properties the same way. **Keyed rows draw
  their keyframes as diamonds on their lanes**, and dragging empty lane space boxes them up
  for selection (the shared marquee the graph editor also uses); the F9 family and the
  bottom bar's easing buttons act on that selection in either view. **Selection lets go**
  (K-203): closing a fold drops the selection inside it, selecting a layer clears the
  property selection, and a click on empty ground in either half of the table selects
  nothing at all — no layer, no properties, no keys. `U`/`UU`/`UUU` reveal what is
  animated / what has been modified / nothing, on the selected layer or — with nothing
  selected — on every layer in the comp. Mask rows are in that same selection (K-234), and
  `Delete` with one picked deletes the mask rather than its layer: a panel holding a finer
  selection than the layer one is asked before the shell's `Delete` removes anything.
  **Keyframes are the finest rung of that ladder** (items 6.24 and 6.6): a drag on any key
  already in the lane selection carries the whole selection, one undo step, and `Delete`
  with keys in hand removes those keys and leaves the layer and its masks alone. Still to
  build here: the expression toggle.

### 4.4 Sequence layers

A Sequence layer's row renders its clips back-to-back (glossary §2):

- Each clip block shows its source name, a **speed readout** (single percentage for constant
  speed, `100→20%` style for ramps), and thumbnail strips when row height allows.
- **Edit points** between clips are draggable (roll). Dragging a clip body slides it and its
  neighbours' edit points never overlap; clips never overlap by definition.
- **Overrun hatching**: when a clip's Retime requests source beyond the media (glossary §4),
  the affected span renders with a hatched overlay and the boundary frame holds. Overrun
  MUST never move edit points (K-022). Context menu offers *Trim to source end* explicitly.
- **Razor** (K-220): with the razor tool (`C`) click a layer to cut it **at the time under
  the pointer** — not at the playhead, which is what `Ctrl+Shift+D` is for. `Shift`-clicking
  cuts **every layer whose span contains that moment**, the way Premiere's razor cuts all
  tracks. Cutting a Footage (or any non-Sequence) layer converts nothing — it **splits the
  layer** in two (AE behaviour): both halves keep the source, effects, masks, parent, label
  and keyframes, they meet exactly at the cut with no gap and no overlap, and each keeps the
  same start offset so neither half's content or keyframes move. Cutting inside a Sequence
  layer creates an **edit point** instead and the layer stays one layer. Each cut is one undo
  step (§4.7).
  The razor is armed from the **toolbar** (§1.7); the Timeline's own menu item is a second
  door into the same state, never a second razor. While it is armed the pointer over the
  lanes is the **scissors icon** and a vertical line MUST follow it across every row at the
  frame it would cut, so the cut can be aimed before it is made. The line does the aiming and
  the pointer only says which tool is in hand (K-235, replacing K-230's hot-spot rule): a drawn
  blade leaning away from the point it cuts at needed a second mark to say where the edge
  actually bit, and the icon on the toolbar says "razor" better than a bespoke drawing of one.
  **A cut only keys a layer that has actually been retimed** (K-236): switching Retime on
  installs the identity map, and putting keys into a map nobody has shaped leaves the user keys
  to notice and remove for a cut they asked nothing else of. A cut at a layer's own end MUST be refused — there is no
  second half there — rather than making a layer of no length.
- Per-clip context menu: frame interpolation mode (nearest / blend / flow), Retime reset,
  reveal in Project panel, replace source (preserves trim and Retime where durations allow).
- **The sequence view (K-248)**: double-clicking a Sequence layer — its outline name (where
  a Precomp opens its comp) or its lane bar — grows the row **in place**: each clip draws a
  start and an end thumbnail, the razor and `Ctrl+Shift+D` cut the clip under the
  pointer/playhead, and a small **speed-envelope strip** (K-247, speed lens only) sits under
  the clips, its points travelling with their clip. Clips may be **reordered and repeated**
  (K-248 dropped K-071's source ordering). The layer's bar always spans first clip start →
  last clip end. Double-clicking again collapses the row.

### 4.5 Snapping

Snapping MUST cover, as sources and targets: edit points, layer in/out points, keyframes,
markers, **beat markers**, the playhead, and work area edges. On by default; a header toggle
plus `Ctrl`-hold to suspend during a drag.

**The switch lives where the snapping does.** The toolbar carried a second one that nothing
read, and it is gone (K-230, §1.7): a global switch belongs there once there is snapping
outside the Timeline for it to govern.

**Shipped (K-190, K-292):** the **magnet** in the lane bottom bar, on by default. With it
off a key may sit *between* frames: the time is quantised to a thousandth of a frame and
built from the comp's exact rate, so it stays rational (docs/14 §2) rather than becoming a
rounded double.

With it on, a keyframe dragged on its lane lands on the nearest **target** within reach —
edit points, layer in/out points, other keyframes, markers (composition and layer, **beat
markers among them**), the playhead, and the work area edges — and on a whole frame when
there is nothing near, which was K-190's original and much narrower behaviour. Beat-marker
snapping is the beat-sync covenant's daily face, and it comes for free because a beat marker
*is* a marker.

Snap distance is measured in **screen pixels**, not time, so zoom level controls precision.
The snapped-to target is indicated at the moment of capture — a line at what caught the drag.
**`Ctrl` held suspends snapping** for as long as it is held, which is the way out when the
wanted place is exactly where a snap will not allow.

**The razor snaps too, and its line says where the edge bites** (owner, 2026-08-06). A cut
was always quantised — it lands on a whole frame — while the blade's line followed the pointer
continuously, so the two disagreed by up to half a frame. Both now read one function: the line
stands exactly where the cut will land, and with the magnet on the cut takes the nearest target
in reach before falling back to the nearest frame. A cut is a clip boundary, so it lands on a
whole frame even when what caught it sits between two.

**Every dragged thing snaps now (TI-9).** The layer **bar** drag takes **both of its ends as
sources** and the nearer capture wins — so a bar can be laid against a marker by either end —
while a trim offers only the end in hand; the **work-area handles** and **marker drags** each
reach for the same shared list, minus themselves (an edge or a flag that can snap to where it
already is is one that never moves); and the **graph's key drags** reach for the timeline's
landmarks in the time axis — markers, beat markers, the playhead, the work-area edges, layer
ends and edit points — but **not for other keyframes**, which are everywhere on that pane and
would make every drag sticky against the very things being rearranged. Each draws the same
capture hairline while a target holds it, and `Ctrl` suspends the reach everywhere. The
arithmetic is the one shared pure module (`panels/timeline_snap.dart`).

### 4.6 Navigation, zoom, and scroll

- Plain wheel scrolls vertically. `Shift+wheel` scrolls horizontally. `Ctrl+wheel` zooms
  time about the pointer. The wheel MUST never zoom without a modifier (no scroll hijack).
- **Zoom flies rather than cutting** (K-293): magnification is a place changing, not a value
  being nudged, so it animates — geometrically, because zoom is a ratio and equal time should
  buy equal ratio. Notches arriving quickly are worth more, so a rolled wheel covers ground
  while a clicked one stays precise; when the hand stops, the flight finishes and settles
  rather than stopping where the last notch fell. The frame under the pointer is held there
  for the whole flight, not merely at its ends.
- **The bottom bar's zoom is a slider** between a small landscape glyph and a large one — the
  pair After Effects flanks its own zoom slider with, painter-drawn so the small end can sit
  under K-209's 16px floor without crunching (K-293). Its left end is the whole composition;
  its right end shows **20 frames** across the lanes, whatever the composition's length — a
  count of frames rather than a magnification, because that is what the number means to a
  person. It runs on the logarithm of the zoom, so equal travel buys equal ratio.
- **A slider zoom holds the playhead still; `Ctrl+wheel` holds the frame under the pointer**
  (K-293). The slider has no pointer to zoom about, and the playhead is where the work is —
  the same thing After Effects zooms its timeline about. A playhead in view keeps the screen
  position it has; a playhead out of view is brought to the middle of the lanes.
- **A dragged slider MUST choose its anchor once, at the start of the gesture, and hold it
  to the end** (K-320). Re-measuring per update reads the scroll offset *before* layout has
  corrected it for the zoom just applied — a fresh zoom against a stale offset — so every
  update anchors somewhere slightly different and the lanes ping about under the finger. It
  MUST also measure the per-frame width from the scroll position's own content extent, the
  same numbers the correction is applied with: a width taken from anywhere else disagrees a
  little at every zoom, and that disagreement grows with magnification.
- **The scroll correction that holds the anchor MUST happen inside layout** (K-293): the
  offset that keeps a frame still is only valid for the width the zoom has just produced, so
  moving it before that width is laid out leaves the view scrolled past its own end for a
  frame — which springs back, and draws the scrollbar's thumb from a position and a length
  that disagree.
- **A dragged zoom control MUST NOT animate** (K-293). The flight fills the gap between zooms
  that arrive in steps — a wheel notch, a tap on the track. A drag is already continuous, so
  it applies at once, and the handle is drawn from the zoom being asked for rather than from
  the flight's current value; animating a drag makes the lanes trail the finger by a flight's
  length and restart before arriving.
- **A trackpad's two-finger scroll MUST scroll the panel** (K-278). It arrives as a pan
  *gesture* rather than as the wheel's signal, so the panel — which otherwise gives drags to
  the keyframe marquee — MUST admit exactly the trackpad as a drag-scroll device, and every
  editing recogniser laid over a scrollable surface MUST exclude it in turn so it cannot be
  taken back in the gesture arena. A click-drag is a pointer drag, not a pan-zoom, so it
  still draws the marquee.
- **The outline and the lanes MUST scroll exactly as far as each other** (K-278): they are
  one table. The lane side's bottom bar is therefore reserved under the outline as well —
  without it the lanes have the shorter viewport, scroll further, and the two halves come
  apart at the bottom of a long stack.
- `=`/`-` zoom time in/out; `Shift+=` zooms to the work area; `\` toggles between full-comp
  zoom and the previous zoom (AE-compatible). `=`/`-`/`\` are built (TI-9), each holding the
  **playhead** still as the slider does — a key press has no pointer to zoom about.
- Dragging in the ruler scrubs the playhead. Scrubbing previews video always; holding
  `Ctrl` while scrubbing also scrubs audio. **Scrubbing while playing stops playback**
  (K-254) and the playhead stays where the drag left it: the engine hands back a frame
  every tick, so a scrub fought against playback could never win, and a playhead that
  returned to where play started would undo the very gesture that stopped it.
- The playhead MUST stay visible during playback via edge-follow scrolling (page-flip or
  smooth per user setting); the timeline MUST NOT recentre while the user is dragging
  anything.

**Shipped (K-189, K-190):** the outline and lanes scroll vertically as one table — one
linked scroll, the visible thumb on the lane side; in graph view each side scrolls alone
with its own thumb. Each thumb lives in a fixed-width **gutter** down the right of its
half, outside the horizontal scroller so it stays pinned to the viewport edge, and the
outline reserves the same gutter with an undraggable block level with its toolbar and
column header — so the columns never shift as the view changes. The lane bottom bar
carries the time-zoom slider, the magnet, and the horizontal scrollbar. **The wheel
scrolls, dragging never does**: a plain wheel moves the rows, `Shift+wheel` scrolls
sideways, `Ctrl+wheel` zooms time about the pointer, and a drag on empty lane space is the
keyframe marquee. A zoom with no pointer to zoom about — the slider — holds the playhead
still instead (§4.6, K-293), so what is being worked on stays on screen. **Edge-follow is
built as a page flip** (TI-9): while the transport runs, a playhead that leaves the viewport
puts the lanes on the next page rather than scrolling under it, so what is being watched
stays still. It never fights a hand, because taking hold of the playhead stops the transport
(K-254). Still to build: the *smooth* alternative and the setting that chooses between the
two, and `Shift+=`.

### 4.7 Editing behaviours

- Layer drag moves in time; vertical drag reorders the stack. `[`/`]` move the selected
  layer's in/out to the playhead; `Alt+[`/`Alt+]` trim in/out at the playhead.

  **The two pairs differ in what happens to the animation** (A8 — the Caddis study's slide
  and trim as two gestures). A **move** carries the layer's content with the bar, and a
  keyframe's time is the layer's own: it reaches the composition's clock through the start
  offset (K-213), which travels with a move — so `[` and `]` slide every keyframe on the
  layer along with it. A **trim** moves one edge over content that has not moved, so
  `Alt+[` and `Alt+]` leave every keyframe exactly where it was. That is what the bar drag
  has always done at each end (`barDragPreview`), and the keys read the bar drag's own
  clamp (`clampBarDelta`), so a key and a drag cannot come to different answers about where
  a layer may end. Both pairs were already bound and already behaved this way; the claim is
  pinned by `shortcuts_frb_test.dart`.

  **Shipped (K-193):** dragging a layer's **bar** moves it in time, and dragging a layer's
  **name** in the outline moves it up or down the stack — drop it on a row and it takes
  that row's place, as one undo step. A locked layer neither drags nor accepts a drop.

  **What the lock means (K-291).** A locked layer refuses every edit to what it *is* — its
  transform, effects, masks, paint, art, text, clips, markers, blend, matte, parent, retime,
  volume, its switches, its span, its place in the stack and its existence. The refusal is in
  the **engine**, so it holds for every caller, not only the gestures the Timeline happens to
  guard; the property rows are also shown read-only, so the interface never offers a gesture
  that would only be refused. A *group* heading in the fold-out stays live: twirling one open
  is navigation, not editing. Three things a locked layer still accepts, because they are the
  Timeline's own bookkeeping rather than the composition: the **lock** itself (or it could
  never be undone), **shy**, and the **label** colour.
  Footage or a comp dragged in from the Project panel lands **where it was dropped** —
  the slot the pointer let go over, by the same midpoint rule — rather than always at the
  top of the stack; a drop past the last layer lands at the bottom.

  **The ends are handles, and the source is the limit (K-211).** Dragging the last few
  pixels of either end of a bar trims that end — the pointer shows the horizontal resize
  arrow there, and the grab zone never takes more than a third of a short bar, so even a
  two-frame bar keeps a middle to move by. A layer whose source has a length of its own —
  Footage (picture or sound) and Precomp — trims **within** it: the in point cannot go
  earlier than the source's first frame, the out point cannot go past its last, and a bar
  already at that limit draws a small triangle in that top corner. Every generated kind
  (Solid, Text, Adjustment, Null, Camera, Sequence) has no such source and trims freely,
  with no corner marks. **Retime removes both limits and both marks**: a retimed layer
  maps its own local time onto source time (docs/04-RETIMING.md), so its length is no
  longer the source's business. Media whose length cannot be read leaves the ends free —
  a missing file must never silently crop a layer. Moving a bar is never limited: the
  start offset travels with it, so what fits its source keeps fitting it.

  **A trimmed layer shows its source's reach (K-212).** A source-backed layer that is not
  retimed and does not fill its source draws a faint outlined rectangle spanning the whole
  source, behind the bar and in the layer's own label colour — so what shows past each end
  is exactly the material trimmed away. **A hairline and nothing inside it** (K-441,
  15-DESIGN §12A.1): a fill behind the bar read as a second, dimmer object rather than as
  this bar's own reach. Absent when the bar already fills its source, on
  the kinds with no source, and under Retime. One vocabulary with the corner triangles: a
  triangle says *this end can go no further*, the outline says *this end could, and this
  is how far*. Both travel with a bar being moved, because the source's reach moves with it.

  **The bar fills desaturated behind a solid leading edge (K-441, 15-DESIGN §12A.1).**
  The fill is the layer's label colour thinned over the lane's ground rather than the
  colour itself — at full strength a stack of layers is a row of bright slabs and there is
  nothing left for the selection or the playhead — with 2px of that colour whole at the
  bar's start, so a bar still lands with a snap. The layer's **name** rides the bar in mono,
  quieted, clear of the edge. Selection brightens the fill rather than outlining it. The
  clips inside a Sequence layer fill and edge the same way, per clip.

  **A shut layer shows what is keyed inside it (K-441).** Its own row draws every keyframe
  anywhere on the layer as diamonds at **half** a property lane's scale, in `animated` — a
  summary, not a target: several properties keyed on one frame are several keys under one
  diamond, and they are dragged on the property lanes, which is where twirling the layer
  open puts them at full size.

  **Switching Retime off re-hangs the layer on its source (K-212).** A retimed layer may be
  any length; when the map goes away it plays at source rate again and needs a length. It
  keeps its in point and the frame showing there, then runs at source rate until either the
  source runs out or its own out point arrives, whichever comes first — it never grows, so
  a layer trimmed short stays short. One undo step covers the removal and the span. Both
  routes to a retime behave the same way, and media with no readable length re-anchors and
  leaves the out point alone.

  **Both halves move (K-208).** While the drag is in flight the stack shows where the drop
  would land: the lifted layer slides towards its slot and the layers it passes slide the
  other way — in the outline **and** in the lane area at once, from one drag state and one
  set of row heights, so a layer's bar never parts company with its name. In graph view,
  where there are no lanes to move, the outline alone animates. Micro-motion per
  [15-DESIGN.md](15-DESIGN.md) §8: transform only, ≤150ms, and it obeys the animation-level
  setting (at *None* the rows arrive without travelling).
- There is **no ripple mode anywhere** (K-022): nothing moves unless the user moves it.
- Multi-selection supports all of the above; relative offsets are preserved.
- Every destructive-feeling action (razor, delete, retime reset) is a single undo step.

---

## 5. Graph editor

The graph editor is a mode of the Timeline's lane area (the mode tabs in the Timeline
header, `Shift+F3`): the lanes are replaced by curves for the selected properties. **The
outline is the Layers outline, identical** (K-529): a property's curves are on the pane
when its row is selected, which is the one way in — the graph's own filtered list, its
per-row *include in graph* tick and its Normalise control were withdrawn with it.

In graph mode the outline scrolls **independently** of the curve (UI-8): it keeps its own
vertical scrollbar at the outline's right edge, and a wheel over the curve pans or zooms the
curve (K-079) without ever scrolling the layer list. The ordinary layers view keeps the single
shared scroll of §4.6, where the outline and lanes move together.

### 5.1 Views

- **Value graph**: value against time, editable bezier tangents per keyframe side, following
  the AE-compatible keyframe maths of K-025 (per-side speed in units/second, influence
  0.1–100 % of the interval).
- **Speed graph**: the first derivative against time; handle height edits speed, horizontal
  handle reach edits influence. Value and speed are views of the same data (glossary §3) —
  editing either MUST round-trip losslessly.
- **Acceleration graph**: the second derivative against time (K-070) — the
  distance/velocity/acceleration analogy taken to its third view. Editing it shapes how
  speed itself ramps; like the others it is a view of the one keyframe/segment store and
  round-trips losslessly. Available for every animatable property, not only motion.
- **Auto view** picks the value graph for scalar properties and the speed graph for spatial
  ones; a per-property override menu offers value / speed / acceleration / stacked, with the
  inactive graphs optionally ghosted as a reference.
- **Lens switch**: value / speed / acceleration are selected by glyph buttons in the
  **bottom-right of the graph editor** (K-070), beside the ease-preset footer (§5.3).

### 5.2 Retime's two lenses

A **retimed layer** exposes its Retime as a channel in the graph editor's left column, beside
the transform properties (K-197). A Sequence layer's clips are retimed in the sequence view
instead (K-248, §4.4).

- The **value lens** plots source position against layer time — the ordinary property graph
  (K-197). It reads in seconds for now; a **frame timecode** readout (`HH:MM:SS:FF` in the
  footage's own timebase — "which source frame is showing here") is still to come.
- The **speed lens** plots speed percentage against time, and its shape follows the Vegas
  preference (K-246): **off**, the ordinary two-sided derivative view every property has;
  **on**, the **envelope** of K-247 — one point per key, whose height *is* the speed, with a
  default vertical range of 100% down to −25% that grows to fit the curve. Either way it is
  drawn **in the graph pane, never overlaid on the clip** in the Timeline (K-021); the bar
  shows only read-only indication.
- **Default lens**: the Vegas preference chooses which lens a Retime channel opens to — on,
  speed; off, value (K-246, realising K-075's preference in the property era).
- The store is the one Retime property (K-249); switching lenses never converts or degrades
  data. Reverse is legal in both lenses and both modes (K-247).

### 5.3 Editing behaviours

- **Box-select** keys by drag; add with `Shift`. A transform box around a multi-selection
  scales the group in time and value — **its four edges**, each about the opposite edge,
  with `Shift` rounding what the scale lands on (K-505 supersedes this line's original
  "corner drag; `Ctrl` tapers": corners stand on the selection's own extreme keys, and
  `Ctrl` already suspends the magnet).
- **Handle editing** with per-side independence; `Alt+drag` breaks tangent continuity;
  a *Continuous* lock keeps in/out speeds equal.
- **Tangent modes** — Auto / Clamp / Free, a run of three on the footer strip, stored per
  key side (K-506). An automatic side takes its speed from the key's neighbours on every
  read (Clamp additionally cannot overshoot them); Free is a side shaped by hand. Shaping
  a handle, an influence field or an ease preset takes its side back to Free, and a side
  returning to Free is given back the exact ease it had before it went automatic. The
  arithmetic is [impl/keyframe-eval.md](impl/keyframe-eval.md) §6.
- **Numeric entry**: double-click a keyframe for exact **frame, value and In/Out
  influence** fields (K-505; a side's speed is what the tangent handle drags and what the
  influence field writes at, so it is not a fifth number).
- **Preset eases**: Ease (`F9`), Ease in (`Shift+F9`), Ease out (`Ctrl+Shift+F9`), hold,
  linear, auto-bezier — buttons along the graph editor footer and in the context menu.
- **Snap-to-beat-markers**: beat markers render as vertical lines in the graph; keyframe
  drags snap to them (same snapping rules as §4.5). This is how speed ramps land on kicks.
- **Auto-zoom fit** (`F`): frame the selected keys, or all keys of shown properties when
  nothing is selected. Manual zoom/pan matches Timeline conventions (§4.6).
- Audio waveforms MAY be ghosted behind curves (toggle) for sync work.

**Shipped (K-196):** the graph editor is one **full-height pane** over the Timeline's own
ruler, zoom and horizontal scroll (`Shift+F3` or the toolbar's Graph toggle), drawing every
selected property as its own coloured curve — a multi-axis property contributes one curve
per axis, and a static property draws as its flat value line. The curves are evaluated by a
Dart port of the engine's cubic (`graph_maths.dart`, pinned to `anim.rs` by
docs/impl/keyframe-eval.md and golden tests), so a paint costs zero bridge calls (K-184).
Landed from the lists above: the **value and speed lenses** (bottom-bar buttons; the speed
lens is the exact derivative, each key an independent in/out dot with one influence handle
each); **per-side tangent handles** with `Alt`-drag breaking and re-joining; **box-select**
with `Shift`/`Ctrl` add; the **preset eases** — F9 / `Shift+F9` / `Ctrl+Shift+F9`, and
Linear / Bezier / Hold buttons in the footer and the key context menu, acting on the lane
selection too; `Ctrl`+click planting a key on the curve under the pointer; **auto-zoom fit**
(`F`, and an Auto fit toggle — off, the wheel pans the value axis and `Alt`+wheel zooms it;
`Ctrl`/`Shift`+wheel stay the Timeline's time bindings); selection-key drags that move a
whole selection in time and value as one write per property; and **keyframe copy/paste**
(`Ctrl+C`/`Ctrl+V`, from the lane view as much as the graph) — full fidelity in-app,
mirrored to the system clipboard as a tab-separated `Lumit <version> Keyframe Data` table
whose per-value easing columns carry the shaping across, and which parses foreign
keyframe tables back in as linear keys. **A shaped ease is drawn once and stamped on many**
(K-348): an **Easing…** button beside the three one-click eases opens a unit box — travel
left to right, two draggable control points, a row of preset shapes, Apply. Apply puts the
shape on every **span** whose two keys are both selected (a lone key names no travel and is
left alone), converting it per span against that span's own chord slope, so one drawn curve
reads the same across a selection whose spans move by different amounts. It is offered in
the **value lens only**, because a shape drawn against value travel would otherwise land on
a graph the user is not looking at. **The box is the Easing panel** (K-349, §5.4): the
button docks it and fronts it, and it stays on screen while the selection changes
underneath — which is the whole point, a shape being worth drawing precisely because it
goes on this run of keys and then that one. Settings ▸ Interface ▸ Editing ▸ *Shape eases
in a popup* puts the same editor in a floating box over the footer instead. **A drag in the graph previews as it goes** (K-329):
every tick renders the values the release will write, through the same patched clone the
value rows use — a key drag, a tangent handle and a Vegas envelope point alike. It covers the
grabbed key's layer, so a selection spanning several layers still shows the rest on release.
**The selection transform box and numeric entry** (K-505): two or more selected keys in
the value lens draw the lanes' own `text_primary` block box, spanning the selection in
time *and* value, with a grab on each of its four edges — left and right scale time about
the opposite edge, top and bottom scale value about the opposite edge, `Shift` rounds what
the scale lands on and a readout pill under the box says live what it reaches. One undo
step, `Escape` abandons it (as it now does the graph's key and tangent drags), and the
badge is the lanes' `n keys · n f`. **Double-clicking a key** opens its exact frame, value
and In/Out influence as fields, the frame bounded by the key's two neighbours.
Still to build:
the acceleration lens and auto view (K-070),
snap-to-beat-markers in the graph, waveform ghosting, and the Retime lenses of §5.2.

### 5.4 The Easing panel (K-349)

A dockable panel holding the shape editor of §5.3 and nothing else: the unit box, the
preset row, the four `cubic-bezier` numbers as text, and **Apply**. No Close — a panel is
closed from its tab or the Window menu, like every other.

- **It never learns what is selected.** The Timeline publishes a callback while it can take
  a shape; the panel presses it and is told nothing about what it landed on. The keyframe
  selection stays the Timeline's.
- **Apply greys when there is nowhere to send a shape** — no Timeline on screen, or a graph
  showing the speed lens — with one line saying so. A popup that vanished could stay silent
  about this; a persistent panel showing a live button that does nothing cannot.
- **The drawn shape survives** a selection change, a lens change and the claim coming and
  going. It resets only when the panel itself is closed and reopened.
- It is in the **Retiming** preset (§1.6) and in no other arrangement; anywhere else the
  Easing… button or Window ▸ Easing puts it there.

---

## 6. Effect Controls

Shows the **effect stack** of the selected layer (tab per recently viewed layer, like AE).

- Effects list top-to-bottom in application order. **Drag to reorder**; reordering re-renders
  live.
- Per effect: enable toggle, **solo** (preview this effect's output alone in the stack),
  delete, reset, rename, and a header twirl. `Ctrl+drag` an effect header onto another layer
  copies it.
- **Parameter widgets** by type: sliders with scrub-drag and click-to-type; **colour
  swatches** opening a picker with an eyedropper that samples the Viewer; **angle dials**;
  **point parameters** with a crosshair button that arms a click-in-Viewer pick (and a
  draggable on-Viewer handle while the effect is selected); dropdowns; checkboxes;
  **track-and-thumb sliders** for a closed range (K-414); curve editors
  where an effect defines one. OFX and LFX parameter types map onto these same widgets
  ([12-PLUGINS.md](12-PLUGINS.md)).
- Every animatable parameter carries the stopwatch and expression toggle inline, mirroring
  the Timeline lanes — the two surfaces edit the same properties.
- **Preset save/load**: save the selected effect (or whole stack) as a preset; presets
  appear in Effects & Presets (§7) and serialise per [10-FILE-FORMAT.md](10-FILE-FORMAT.md)
  for sharing (K-065).

  **Shipped: drag-to-reorder** (K-276) — dragging an effect's heading onto another's moves
  it to that place, the same "take hold of the name" gesture the Timeline and the Project
  panel use; the heading under the pointer marks itself so the place being taken is clear.

  **Shipped: the panel's layout.** The panel is **one list, not a stack of cards** — the same
  reading as the Timeline's twirl-down (§4.3), which is where the same parameters also appear.
  Each section (Source, Transform, and one per effect) is a **heading bar that twirls**, with a
  hairline under every row beneath it. A section arrives open, so an effect shows its
  parameters the moment it is applied.

  Every row is **two columns, undivided**: the property's name left-aligned in a fixed-width
  name column, its control left-aligned in the rest. They read as columns because they line
  up down the panel, not because anything is drawn between them; the name column also reserves
  its keyframe-controls gutter on rows that cannot animate, so labels align whether or not
  the property is animatable (`flutter_ui/lib/panels/fx_section.dart`).

  The **heading row** runs: twirl, the effect's enable switch, the effect's name — all in the
  name column — then **Reset** at the top of the value column, because that is what it acts
  on. Reset writes every parameter's declared default and so drops any curve on it, as one op
  and therefore one undo step. Hard right sit the effect's **render time** (§4.2's column,
  the same measurement) and the close mark, away from Reset: removing an effect is not an
  adjustment to it. **Reordering is a right-click on the heading** (K-276) — move up, move
  down, to the top, to the bottom, remove — rather than the pair of arrows that used to hold
  that space: moving an effect is a handful of acts in a session, and what it costs is read
  continuously while a comp is being made faster. The menu lists only the moves that effect
  can make.

  **An effect's name selects it** (K-300), taking the selection fill across the heading bar —
  plain replaces, `Ctrl` toggles, `Shift` extends the run down the stack. Selecting an effect
  **does not fold it**: the twirl mark is the only thing that opens and shuts a card, because
  a click that did both would take the parameters away at the moment you said which effect you
  meant. It is the same selection the Timeline's fold-out shows (§4.3),
  so an effect picked in either place is lit in both, and it is what **Copy**, **Cut** and the
  heading's **Copy effect** act on. Source and Transform are not part of a stack and so are
  not selectable; their headings twirl as they always did.

  **An effect can be given its own name** (K-321). `Enter` on the selected effect turns its
  heading into an inline editor holding the current name, selected; committing writes the
  name onto the *instance* and it is shown in place of the effect's label wherever the stack
  is drawn — this panel and the Timeline's fold-out — so "Blur the sign" replaces "Gaussian
  blur" for that one instance. Clearing the field falls back to the label. It is a display
  name only: the effect's `match_name`, its schema, its parameters and every lookup by name
  are untouched, and a project saved without a given name is byte-for-byte as it was.
  Parameter rows are **not** renameable — a parameter's name comes from the schema.

  **A parameter can be a button** (K-417). An `Action` row carries no value at all — no
  number, no keyframe, no undo entry — so it draws as a **button in the value column with the
  name column left empty**: the button says its own name, and a label beside it repeating the
  word would be the row said twice. Pressing one is an *event* sent to the engine, not an
  edit. Reset walks past it, because there is nothing to put back. The Camera track's
  **Analyse** and **Cancel** are the first two, and the beat detector is waiting.

  **An effect may draw a status under its rows.** Where an effect owns work that happens
  elsewhere — the Camera track's analysis runs on its own thread, over the media file, while
  editing carries on — a single calm line sits with its buttons: how many frames have been
  followed, that the camera is being solved, and when it is done the number that says whether
  the solve is any good (its point count and mean error), with **Create camera** beside it. A
  refusal is a plain sentence in the same line, and nothing about the shot has changed. The
  line is *sampled* while the job moves and left alone when it does not; it is never a
  progress bar, because the Viewer's is the one progress bar (§2.5).

  **A finished answer that covers part of its clip draws the part it covers** (K-540). Where
  a job's answer is about a *span* of the media rather than the whole of it — today, a camera
  track that stopped where the shot stopped being followable — a thin bar sits **above** that
  line and shows the analysed span against the rest of the clip, in theme colours, with the
  line saying how far it got in words. This is not the progress bar §2.5 reserves and does
  not contradict it: it appears only once the work is *over*, it does not move, and what it
  measures is the answer's extent rather than the work's completeness. A whole answer fills
  it, which is what makes a partial one legible without reading anything.

  **Two parameters can share one row.** Two conventions fold, and both exist because the
  pair is one idea and reads worse split in half: an `_x`/`_y` Float neighbour pair draws as
  one **point row** with a shared stem label and a crosshair pick; and a **Layer** picker
  draws with its `<id>_invert` switch beside it — the uniform **Matte** row of
  [08-EFFECTS.md](08-EFFECTS.md) §2.6 (K-395), labelled "Matte" and "Invert", which every
  effect has and which Depth of field and the Lens flare share rather than keeping private
  synonyms. The folded switch never also gets a row of its own. Both are pairings by
  *convention over the schema*, not a table in the panel naming effects: an effect that
  declares a Layer row and an Invert next to it gets the row, and one that does not, does
  not. (The Timeline's fold-out lists parameters flat and folds neither, because each row
  there is a selectable property path.)

  **Round shape keeps its bubble** (K-092): the same rows, wrapped in floating-card chrome.
  The two shapes differ in chrome, not in layout.

  **Shipped: the closed-range row** (K-414). A **Slider** parameter — one whose whole meaning
  lives inside a range, a wipe's Completion being the catalogue's clearest — draws as a
  **track and thumb with the number beside it**, the same arrangement the angle row uses for
  its dial: the track is a second grip on one value, not a second control, so it sits beside
  the number rather than under it. The two behave identically — a drag on either previews
  live and commits once on release — and the row keeps every affordance a float row has,
  stopwatch and graph editor included, because the value *is* a float. The range is both the
  travel and the hard bound, so neither grip can leave it and neither can typing.

  **Shipped: the curve editor** (K-412). A **Curve** parameter draws as the unit square with
  the spline through its points: **drag a point** to move it, **click the line** to add one
  (up to sixteen), **drag a point well clear of the square** to remove it, and **Reset** puts
  the channel back to the identity diagonal. The two end points move like any other — the
  black and white points slide along their edge, which is how an end is crushed or lifted —
  but are never removed, because a curve needs somewhere to start and finish. A run of
  neighbouring Curve parameters **folds into one editor with a tab each** (Curves' Master,
  Red, Green, Blue and Alpha), the third folding convention alongside the point pair and the
  Matte row; Mix and Matte keep their ordinary rows beneath it. The line drawn is a
  **display-only** evaluation of the same clamped cubic: the engine's baked table is what
  grades the frame, and asking it for one per pointer move would be a bridge call a frame
  (`flutter_ui/lib/widgets/curve_editor.dart` carries the reasoning).

  **Effects with their own display** — Levels' histogram (K-413) — draw a widget **above**
  their rows, through `customEffectDisplay` in `effect_controls_panel_frb.dart`. Levels shows
  the frame's histogram with its input black, gamma and white handles over it and the output
  range as a bar beneath, each handle dragging the Master parameter it marks. It is
  presentation: every number still has its own row underneath, the parameters and their ids
  are untouched, and the picture comes from the trace the Scopes panel already reads (§8),
  asked for once per displayed frame and only while the row is on screen.

  **A control inside this panel uses axis drag recognisers, never a pan.** The panel is a
  list, and a pan needs twice the slop a single-axis drag does, so an enclosing vertical
  scroll wins every upward gesture — the curve editor's points and the Levels handles both
  read as dead until they stopped asking for a pan.

  Still to build here: solo, rename, and the expression toggle.

### 6.1 The colour picker and the dropper (K-210)

**The picker.** A colour swatch opens the house picker: the **R, G and B numbers across the
top**, each drag-scrubbable and typeable, then the saturation/value square, the hue strip, a
was/now pair and a hex field. Every one of those edits every other — type a number and the
square moves; drag in the square and the numbers follow. **Project colours live inside the
picker** (K-448): a project-wide swatch row is the picker's own, never a swatch strip in
the toolbar.

**The numbers are in the scale of the thing being edited**, which is not always 0–255:

- A **display colour** — a theme colour, a solid's swatch — is eight bits a channel, so it
    reads **0–255** and its hex is the same value said another way.
- A **scene-linear colour** in a float working depth (fp16 today, [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §3.1)
    reads **0–1 for black to white, as decimals**, and a channel may go **above 1** or below 0
    as far as the parameter's own declared range allows — several built-ins declare 0–4 for
    exactly this reason ("linear light: HDR tints are legal"), and one declares −1 for a lift.
    A 0–255 dial cannot reach those values at all, which is what this scale is for.
    When the project depth switch lands (§3.1, not built), an 8 bpc project is what puts an
    effect colour on the 0–255 scale.
- **The hex box is display-referred**, so on the float scale it shows the colour **clipped**
    into 0–1, and the picker says so in a line under the swatches whenever a channel is
    outside that. Typing a hex sets exactly those 0–1 values. The alternative — hiding the box
    on the float scale — loses the one notation people actually exchange colours in.

The picker **applies to the document as it changes**. A drag inside it previews continuously
on the picture (the same live tick an Effect controls drag sends) and settles into one
undoable edit when released; a typed number, a hex entry or a preset click is one settled
edit on its own. So there is no state where the picker shows one colour and the composition
shows another, and **clicking away from the picker closes it keeping what is applied** —
nothing is waiting on a button. **Cancel** is the way back: it writes the colour the picker
opened with and closes. **Apply** closes keeping the current one.

**The dropper** is the pipette beside a swatch — and beside anything else that means "a value
at a pixel", which is not only colour: the depth-of-field **focal point** carries one, and it
reads *depth*, not colour. Clicking it arms the tool; clicking it again, pressing Escape, or
pressing away from the picture puts it away. It lights while armed, so a dropper armed and
forgotten is visible from across the panel.

**A pick is a drag, not a click** (K-532). Pressing on the picture writes nothing. It starts a
gesture: every move stages the sample under the pointer and **previews it through the same
staged path a scrub-drag uses** — the colour sweeps, the point slides — and the **release
commits that last sample once**, which is the gesture's one undo step. **Escape** mid-drag puts
back what was being previewed and puts the tool away; nothing was committed, so there is
nothing to undo. A press on a picture no window has been read of stages nothing, so it commits
nothing and leaves the tool armed for the next attempt. Previews are rate-limited exactly as a
value drag's are, and the newest position is never dropped. A pick MUST NOT ask the engine
anything per pointer move beyond the window reads below: a position pick reads the
composition's size **once, when the tool is armed**.

**The armed pick owns the drag.** While any picker is armed the Viewer's own pan stands
down entirely — pressing on the picture starts a pick and nothing else, and the picture
does not move under the pointer while pixels are being read off it. The pan comes back the
moment the tool is put away.

While armed, the Viewer grows a **magnifier** that follows the pointer. It is on screen only
while the pointer is **over the picture** — arming the tool shows nothing until then, and a
fresh arm never opens where the last pick left off — and it keeps **one fixed offset** from the
pointer everywhere on the picture, drawn over whatever sits beside the Viewer rather than
pushed back inside it near an edge (a pick in the bottom-right corner is as ordinary as any
other, and the magnifier must not creep over the pixels being aimed at to make room for
itself). The **window's** edge it does answer to, the way a tooltip does: it **flips to the
other side of the pointer** on whichever axis would run off — above instead of below, left
instead of right, each axis on its own — at the same distance, so it still never covers what
is being read. It shows:

- a **9×9 grid** of the pixels under the pointer, one enlarged square each, with **dashed
  rules between every pair** so pixel boundaries are legible;
- a **solid border** round the pixels that will actually be taken — the **centre pixel alone**
  by default, its corners taking the theme's control radius, so it is rounded under the round
  shape and square under the sharp one;
- **Shift+scroll** steps the sampled region 1×1 → 3×3 → 5×5 → 7×7 → 9×9 and back, never
  wrapping and never exceeding the grid; the region is always odd, so there is always one
  centre pixel. Shift+scroll does not also zoom the picture.
- a **strip under the grid** saying what would be picked. For a colour pick that is the
  averaged colour and its numbers; for a pick that is reading something else it is **the layer
  the numbers are coming from and the value read off it** — a swatch of the composite would be
  a colour nobody is choosing.

Averages are taken in **scene-linear light**, not over display bytes, because that is the
space a Colour parameter stores and what "the average of these pixels" physically means.

A pick that reads a **layer** — the depth-of-field focal point reading its own depth pass —
samples that layer **rendered alone**, not the composite: a depth pass is nearly always
hidden, so what the composite shows at that pixel is not the number the effect uses. The
effect's `depth_invert` is applied at the pick, so the caption and the committed value cannot
disagree.

The pixels themselves come from the engine, a **window** at a time
(`CompositionReference::sample_pixels` → `WorkerResponse::Sampled`,
[17-BRIDGE-CONTRACT.md](17-BRIDGE-CONTRACT.md)): a 129×129 square of the picture, out of which
the magnifier cuts its own 9×9 as the pointer moves. The read is asked for as a **fraction of
the picture**, not as a pixel of the composition, and every pixel is then named in the raster
the reply says it cut from — the picture read is a reduced-resolution preview whenever the
Viewer is showing one, so the two grids are not the same and mixing them shows one repeated
edge pixel where the picture should be. Moving the pointer, and changing the
sample size, therefore cost **nothing** — no bridge call, no render, no message — and a new
read happens only when the pointer nears the window's edge, the playhead moves, an edit lands,
or a different layer is being read. A window is 66 KiB, so this does not reopen the read-back
frame transport K-183 deleted (a 1080p frame is 8 MiB and 8.8 ms in the codec); it is the
answer to a question about a few pixels, not a picture.

Still to build here: the x/y **position** pick for coordinate-valued parameter pairs (the
the T14 viewfinder), and the on-Viewer crosshair handle for point parameters.

### 6.2 Value boxes and text fields (K-319)

Every scrubbable number in Lumit — a parameter row's value, a dialogue's field, the transport
timecode — is one control with two modes: **drag it** sideways to adjust, **click it** to
type. The rules that keep those two from stealing each other's gestures are shared by all of
them, and apply to the plain text fields too.

- **A press that never crosses one increment is a click, not a scrub.** A drag that ends
  without ticking a single step MUST cancel as a drag and then do what the click meant —
  open the editor. Before this rule a click that wandered two pixels did nothing at all, and
  the boxes read as swallowing clicks.
- **The editor opens with the whole value selected.** A value is retyped far more often than
  it is amended, so the first keystroke MUST replace it rather than append to it.
- **Focus lands on the pointer's down stroke, not on the resolved tap.** Someone who presses
  in a text field and slides straight into a drag is selecting text in one motion; the field
  MUST already be theirs when the highlight starts, or the gesture selects nothing.
- **An open editor MUST support real text selection** — press to place the caret, drag to
  highlight, the desktop selection handles — in every field, including the numeric ones and
  the timecode readouts.
- **`Enter` on a focused value box opens its editor**, and while it is focused its keys are
  its own (no panel command fires underneath it). The accent focus ring says which box has
  focus, per docs/15 §6.5.

---

## 7. Effects & Presets

- A searchable tree: **built-in effects** by category ([08-EFFECTS.md](08-EFFECTS.md)),
  **OFX** and **LFX** plugins (labelled with their origin), **user presets**, and imported
  preset packs.
- **Every heading twirls.** Each category — and the Presets group above them — carries the
  set's triangle (right shut, down open) and folds its entries away; the whole heading strip
  is the target, as the Timeline's section headers are. Groups arrive open, and which are
  shut is *view* state: remembered for the session, never written to the document.
- Search is fuzzy, matches names and categories, and filters the tree live. `Ctrl+F`
  focuses search when the panel has focus. **A live search overrides every fold** — matches
  show wherever they sit, because a search that hides what it found is a trap — and clearing
  the field puts the folds back as they were.
- Apply by: double-click (applies to selected layers), drag onto a layer row in the
  Timeline, or drag onto the Viewer (applies to the topmost hit layer, which highlights
  before release).
  - **v1 (K-101)**: the drag-onto-Timeline-row path ships first, scoped to footage and
    adjustment layers (the effect stack's two ordinary homes) — dragging an entry there shows
    an accent hover outline over the row and appends the effect on release, one ordinary undo
    step. The drop target is the **whole row** (the layer outline as readily as the lane, since
    the browser's hit-test ignores whatever bar or switch sits under the cursor) and the
    **Effect Controls panel** (dropping anywhere in it appends to the shown layer). Double-click
    apply, drag onto the Viewer, and every other layer kind (which still gains effects through
    its own row's "Add effect" menu) remain later steps.
- **User presets (K-129)**: a **Presets** group at the top of the tree lists the `.lumfx`
  presets in the preset library — the roaming app-data folder `…/Lumit/data/presets`,
  scanned live so a just-saved preset appears at once. Each entry shows the preset's own
  name (or the file stem when the file can't be read), filters under the same search field,
  and **applies on a click**, appending its whole saved stack (fresh instance ids) to the
  selected layer as one undoable `SetLayerEffects` — the same append the Effect Controls
  → Presets "Load preset…" commits. "Save stack as preset…" defaults its file dialogue to
  this folder (created lazily), so saving and browsing share one home, and it saves **exactly
  the current selection** (K-156): the highlighted effects with their values as set, and — when
  specific keyframes are picked out on the lanes — only those keys. With nothing highlighted it
  falls back to the whole stack. A missing or empty folder shows a hint, never a failure.
  Drag-a-preset-onto-a-layer and preset thumbnails are later steps.
- **Favourites**: star any effect or preset; a Favourites group pins to the top of the tree.
- Hovering an entry SHOULD show a one-line description; presets show a thumbnail where the
  preset carries one.

---

## 8. Scopes

- One Scopes panel type; each instance shows one scope, chosen in its header: **waveform**
  (luma or RGB), **vectorscope**, **histogram** (per-channel/luma). Open several instances
  for side-by-side scopes (the Colour workspace does).
- Scopes are computed from the **Viewer's displayed frame** — after preview resolution
  and channel view, before display transform (so scopes read scene values, not the monitor
  transform). When preview resolution is below Full, the panel MUST show a small "computed
  at Half" style note, because downsampling changes distributions.
- Scopes MUST update live during playback; under load they degrade to a lower update rate
  before they degrade precision (they participate in adaptive degradation and light the
  Viewer's degradation indicator, §2.2).
- **v1 (K-096, extended by K-130)**: scopes are computed on the CPU from the composited
  frame Lumit banks in RAM — that banked frame *is* the Viewer's displayed frame. The panel
  reads the frame **under the playhead** from the cache **every paint** and, while playing,
  requests a repaint at the playback cadence, so the trace **tracks the live frame during
  playback** for every frame the cache holds (a warmed work area — idle fill, playback
  prefetch, or the paused readback — keeps the scope live end to end). When a playback frame
  isn't banked yet (one the frame-budget readback skipped, or one still rendering) the scope
  **holds the last frame it showed** rather than blanking, and catches up the moment the
  current frame is banked — the graceful degradation §8 asks for under load. Guaranteed
  every-frame tracing under all conditions (including a cold, unwarmed comp) still waits on a
  GPU-side scope pass. Banked frames are always specified-resolution, so the "computed at
  Half" note does not fire in v1. Colours come from the theme's fixed scope set (a near-black
  graticule and bright trace in both light and dark chrome, like the neutral Viewer surround,
  §2.1).

---

## 9. Preview panel and transport

A slim transport strip is docked beneath the Viewer bar by default; the same controls exist
as the dockable **Preview panel**.

- **Play/pause** (Space). **Stopping returns the playhead to where play started** — the
  default, because playback is a preview of the moment being worked on and coming back to
  a different frame means finding your place again after every space bar. This holds
  however playback ends, the composition running out included. Settings ▸ Interface ▸
  Editing ▸ *Playhead stays where playback stopped* puts the older behaviour back (K-254).
  The exception is a ruler scrub, which stops playback in order to move the playhead (§4.6).
- **Loop modes**: loop work area (default) / play once / ping-pong.
- **Cache status**: a readout of how much of the work area is preview-ready (backed by the
  cache bar), plus a *fill cache* action that renders the work area ahead of playback while
  idle (K-016). Lumit has no separate "RAM preview" ritual — playback always plays, using
  whatever is cached and rendering the rest, degrading before dropping (K-018); uncached
  playback keeps audio sync by frame-skipping and reports skipped frames in this readout.
- **Audio mute** toggle.
- **Quality toggle**: full / draft preview quality (draft maps to the engine's reduced
  quality mode; independent of preview resolution).
- **Preview mode toggle** (K-030): **Cached** (default) / **Realtime**. Realtime renders
  every frame live, continuously choosing the resolution tier that sustains the comp frame
  rate instead of waiting on cache — the "just play it now" mode for heavy comps. The
  active tier shows in the Viewer's degradation indicator
  ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §6.5). This toggle lives here and in
  Settings → Preview, deliberately **not** in the Viewer bar's resolution dropdown: picking
  a resolution and picking a mode are different decisions, and the resolution picker stays
  the default way to work through a project.

---

## 10. Audio panel (v1)

The v1 sync toolkit (K-050); the Composer workspace is future work specified in
[09-AUDIO.md](09-AUDIO.md) and not here.

- **Waveforms in the Timeline**: every audible layer MAY show its waveform inside its row
  (twirl the Audio group, or a per-layer waveform toggle); the Audio workspace defaults
  them on. Waveform rendering MUST stay responsive at any zoom (mip-mapped peaks).
  **Shipped (K-172, K-280):** the Audio group (Volume + Waveform twirl) in the layer
  outline; the lane draws the layer's own peaks through its live offset each paint. The
  peaks are **mip-mapped and window-fetched** (K-280): the lane asks the engine for the
  stretch of source it is showing at one bucket per pixel column, and asks again when the
  zoom or the scroll moves that window, so the drawn detail follows the zoom instead of
  stretching one fixed summary. Sequence-layer **clips** draw their own waveform inside
  their box, bucketed through the clip's own map so a ramp's transients land where they are
  heard, and carried along when the clip is slid. A **retimed layer's** lane stretches the
  same way (K-436): its buckets are taken in the layer's clock and mapped through its Retime,
  so a slowed passage is drawn wide because it is wide. The lane is drawn across **both** its
  own row and the empty one its Waveform twirl sits in (K-437), so a centred wave sits on the
  divider between them and one from the floor rises through the pair; the rows themselves do
  not grow. Waveforms draw as a three-band
  **multiwave** stack (bass / middle / treble) by default, drawn over one another around one
  centre line rather than in separate lanes (K-284); Settings ▸ Interface ▸ Editing ▸
  *Waveforms show the frequency stack* turns it off for one plain wave, and *Waveforms rise
  from the bottom* stands either of them on the floor of the row rather than centring it
  (K-285).
  The earlier comp-wide strip under the ruler is gone — it was one mixed-down waveform for
  the whole comp, went stale during a drag, and stopped earning its row once every layer
  could carry its own.
- **Beat-marker generation**: pick a source audio layer; controls for **sensitivity**,
  detection **range** (whole layer or work area), and minimum beat spacing; *Generate*
  writes beat markers to the comp's markers ribbon; *Clear beat markers* removes only
  generated ones. Generated markers are ordinary markers thereafter — movable, deletable,
  snap targets everywhere (§4.5, §5.3). Manual beat tapping: pressing `8` during playback
  drops a beat marker at the playhead.
- **Volume keyframes**: each audio-capable layer has a Volume property (dB) with normal
  keyframe/graph-editor behaviour; the Audio panel shows the selected layer's volume and
  pan? — pan is deferred; v1 exposes volume and mute/solo only.
- **Level meters**: output meters for playback, peak-hold, in the panel header.

---

## 11. Export dialogue

`Ctrl+M` (or File → Export…) adds the active comp to the **export queue** and opens the
Export window. Export never blocks editing; the queue runs in the background.

- **Queue list**: each item shows comp name, range, preset, destination, status
  (queued / exporting with progress and time remaining / done / failed with reason /
  cancelled), and a **per-item cancel** button. Items are reorderable; failed items keep
  their settings for retry.
- **Per-item settings**: range (work area / full comp / custom in-out), **preset**, output
  folder, filename (tokenised template: comp name, date, preset) — and, expandable beneath
  the preset, the full custom controls: resolution (comp / half / custom), frame rate
  (comp rate or override), format/container, codec with encoder choice (hardware
  NVENC/AMF/QSV or software), rate control (VBR bitrate / CRF quality), colour output
  (Rec.709 default), audio codec/bitrate, and **resource allocation** — export thread
  count and a background/balanced/fast priority selector governing how much of the
  machine the queue takes while you keep editing. Editing a preset's controls offers
  "save as new preset".
- **Shipped presets**: *YouTube 1080p60* (H.264, high bitrate VBR), *YouTube 1440p60*
  (H.264; the scene's quality trick), *Vertical 1080×1920 60* (Shorts/TikTok/Reels), plus
  *PNG sequence + alpha* and a mezzanine intermediate. Preset details and encoder matrix
  live in the export spec; presets are user-editable and shareable files.
- **The preset store** (engine-backed): the built-ins — *Master* plus the delivery presets —
  are **read-only**, and a preset of one's own is saved from the dialog under a name that is
  not a built-in's. Saving over an existing name replaces it in its own row rather than
  making a second row of the same name; the library is one small JSON file in the
  application's data directory, so it follows the user between projects, and a missing or
  damaged one reads as an empty library rather than an error.
- **When done**: two ticks, not a list (K-485) — *make a noise* and *open folder* — because
  they are independent answers and a long export left running wants both. The noise plays a
  bundled sound file when there is one and is silent when there is not. Both are honoured by
  the **queue** as the item lands, so an export that finishes after its dialogue closed still
  does what it was asked to.
- **The dialogue is one scrolling page** (K-485), and its tab strip — Output / Time /
  Picture / Colour / Audio / Metadata — says *where you are* rather than which page is
  shown: it follows the section last touched or scrolled to, and clicking a tab scrolls
  that section into view when it is not already fully visible and lights its box for a
  moment. The output types are **Video, Image sequence and Audio only**: a still is an
  image sequence of one frame, which the span already says.
- **A control the chosen format cannot honour is disabled, never hidden and never live**
  (K-479's capability table): an `.mp4` has no alpha channel and no sixteenth bit, an image
  sequence has no sound and no bitrate, a `.wav` has no picture at all. The same face
  carries the one row no *subsystem* backs — proxies — with a short reason on hover; *render
  guide layers* is a live tick the day the seam carries it (K-497 built the subsystem). Whatever the engine would refuse is said **in the footer, before
  anything is queued**, and the two actions stay inert until it is answered.
- **Progress**: overall queue progress in the window and on the OS taskbar icon; per-item
  progress bars; completion raises a non-blocking notification with *Reveal in folder*.
- Export uses full quality always — adaptive degradation and preview resolution MUST NOT
  leak into export (glossary §5).

---

## 12. Command palette

`Ctrl+Shift+P` opens the command palette from anywhere.

- Fuzzy search over: **commands** (every menu item and every remappable action, with its
  current shortcut displayed), **effects** (enter applies to the selected layers),
  **comps** (enter opens in the Viewer/Timeline), and **panels** (enter opens/focuses).
- Arrow keys navigate, `Enter` executes, `Esc` closes; the palette MUST be fully
  keyboard-operable and MUST show category badges so an effect is never mistaken for a
  command.
- Recently used entries rank first. Plugins (LFX) MAY contribute commands.
- The palette doubles as the discoverability layer: any command a user cannot find in the
  menus is one palette search away, with its shortcut taught in the result row.

**Shipped (v1, K-102):** the palette exists — Ctrl/Cmd+Shift+P or Window → Command palette…,
fuzzy search (subsequence; a label match outranks a keyword-only one), arrow keys navigate,
Enter/click runs, Esc closes, drawn as a top-anchored modal. v1 covers the
**commands** category (save, undo/redo, new composition, add layers, reset workspace, open
Settings, colour scheme and shape switches, export). The effects/comps/panels categories,
recent-first ranking, category badges and taught shortcuts fill in later.

## 12.1 Composition hierarchy

The **Hierarchy** panel (K-102) shows the active composition as an indented, foldable tree:
its layers, each precomp layer expandable to reveal the layers of the composition it nests,
recursion-guarded. Clicking a row selects that layer and switches to its composition. It is
read-only — the simple tree form of the AE composition flowchart; the full node-graph
flowchart (the deferred node-graph view) grows from it.

---

## 12.2 The FX console (`Ctrl+Space`, K-324; presentation K-325)

A second, narrower command surface, and deliberately not a duplicate of §12: the palette is
every command by name, the console is **effects, fast** — plus the thing you were probably
about to do. Modelled on Video Copilot's FX Console, which is what After Effects users
install first. It supersedes the radial menu K-102 deferred.

**Where it opens (K-325).** The ring MUST open **centred on the pointer** — anywhere in the
window, the Viewer included, pulled in just enough that the whole ring stays on screen — so
the flick can start the instant the chord lands. The search bar floats **above** the ring,
or **below** it when the top of the window would clip it; its dropdown always opens downward
from the bar. The console is not a boxed window: its surfaces are the standard menu float
made slightly translucent, over a **half-strength scrim** — enough darkening that every
slice reads over any frame, never so much that the work it acts on disappears. `Esc` MUST
close it from anywhere, whatever has focus.

**While the console is open, the keyboard is the console's (K-328).** The search field MUST
hold focus for the console's whole life — anything typed lands in the box from the first
keystroke — and every command handler stands down (`lumitModalOpen`), so a keystroke aimed
at the box can never run a shortcut underneath. The only ways out are `Esc` and a click
outside.

**The search bar starts empty and lists nothing** — the ring is the offer. Typing opens a
dropdown below the bar, which MUST rank **effects first and compositions after a divider**,
within each kind and never across it: the reason to open this window is nearly always an
effect, and a comp that happened to score better would be in the way. Matching is §12's
subsequence ranking, so "gau" finds Gaussian blur. `Enter` applies the top match to
**every** selected layer (K-217); a composition fronts; `Enter` on an empty bar closes. The
arrow keys move the highlight. While the query is non-empty the ring steps aside — the
dropdown needs the room, and typing is choosing the other way in. `Esc` retreats **one step
at a time**: clear the text, then pop a sub-ring, then close.

**The snapshot button** sits beside the field and writes the frame on screen to a PNG. It
MUST be a one-frame image-sequence export (§11's `png` codec, K-201) rather than a second
still-writer: same colour and sizing path, and the status line already reports it. It writes
to a `Snapshots` folder beside the saved project, or the user's pictures folder for a project
never saved — never the working directory. It MUST grey out with no composition open.

**The radial menu** follows Blender's:

- A slice MUST be chosen by **angle alone**, not by hit-testing a drawn wedge — a flick in a
  direction picks that slice however far the pointer travelled. That is the whole reason a
  ring beats a list: the direction becomes muscle memory, and a list's third entry moves the
  moment the list grows.
- A **dead zone** in the middle picks nothing, so opening the menu and releasing without
  moving cancels rather than committing to whatever was nearest.
- The first slice is straight up, and they run clockwise.
- Entries MUST follow the selection: a **Project panel item**, while that panel is active,
  offers one slice — **Add to comp**, dimmed when the item cannot be placed (no comp open, a
  folder, a comp into itself) rather than dropped (K-327), and never the new-layer ring; a
  **picked effect** offers what you do to an effect (bypass, copy, remove, add another); a
  **selected layer** what you do to *that* layer — never new-layer commands beside it
  (K-325); a **composition with nothing selected** the new-layer menu, which is what an
  empty timeline is asking for; **nothing open at all** the two ways to get somewhere.
- A slice MAY carry a **ring of its own** (K-325): choosing it expands the menu in place, a
  caret on the slice says it will, and the centre — or `Esc` — steps back out. This is how
  the selected-layer ring reaches creation: a **New ▸** slice opens Layer ▸ New's items, in
  the menu's order, so the two surfaces teach the same thing.
- The selected-layer ring also carries **Keyframe ▸** (K-326): one slice per everyday
  transform row. Choosing one plants a key at the playhead holding the value already there —
  nothing moves — and fronts the Timeline with that row open, so the key just made is on
  screen. A row already keyed there just reveals; a row driven by an expression is dimmed.
- Each ring MUST be at most **six** entries. A ring of twelve is a ring nobody learns, and
  the long tail is the search bar beside it.
- A slice that cannot run right now MUST be drawn dimmed rather than dropped, so a direction
  a hand has learned keeps its meaning.
- The middle of the ring names what it is about (the picked effect, the layer, the comp, the
  sub-ring entered), so the context is never a guess.

The console's lists are declared beside the menu items, as §12's are, so the effects it
applies and the comps it fronts cannot drift from what the menus mean.

## 13. Onboarding and empty states

### 13.1 First-run setup (K-006; v1 ships minimal per K-246)

On the very first launch only, before any project opens, one calm screen asks a single
question: *"Where are you coming from?"* with four cards:

| Choice | What it sets |
|---|---|
| **Vegas for speed ramps and effects** | Graph editor opens Retime in the **speed graph** by default; ramp preset shelf (Linear/Slow/Fast/Smooth/Sharp) pinned in the graph editor; *New Sequence layer* promoted in the Timeline empty-state hints; Vegas-mapping tips enabled (e.g. "velocity envelope → Retime speed graph"). |
| **Vegas for speed ramps, AE for effects** | Speed graph default for Retime, **value graph** default for ordinary properties; AE-alternate keymap offered; both mapping tip sets enabled. The most common montage-scene split. |
| **After Effects for both** | Value graph default everywhere; AE-alternate keymap offered; AE-mapping tips (e.g. "time remap → Retime", "track matte → matte dropdown"). |
| **Neither / just starting** | Lumit defaults; beginner-leaning rich tooltips enabled. |

Rules: one screen, skippable (skip = Lumit defaults), no account, no telemetry, nothing
else asked. Every affected setting is an ordinary visible setting changeable later, and
the chooser can be re-run from the command palette (*First-run setup*). This MUST remain
a single screen — it is a preference primer, not a tour, and does not breach §13's
no-wizard rule below.

The **v1 build** (K-246) ships the minimal form of this screen: two plain choices,
**AE-style** and **Vegas-style**, where Vegas ticks the two K-246 settings (Retime opens to
speed; video arrives as a Sequence layer) and AE ticks neither. Along the bottom sits one
tick, **on by default**, for automatic update checks (K-296) — the same setting as
Settings ▸ General ▸ Updates, asked here because it is a decision about how Lumit behaves
from now on. Skipping the screen leaves it on. The four cards above, with a
small image over each choice, remain the destination (polish tracked in TODO).

### 13.2 Empty states

- **Empty project**: the Viewer area shows a single calm card with three actions —
  *Import footage*, *New composition*, *Open project* — plus recent projects and a note
  that footage can be dropped anywhere in the window. Drag-and-drop import MUST work over
  every panel from first launch.
- **The welcome screen** (K-448, K-464, K-468, superseding the card above once the redesigned
  shell lands): the launch window carries **exactly two** start cards — **New project** and
  **Open** (K-617) — with **Manual** and **What's new** as outlined buttons, and no "free and
  open source" line. With nothing open, the same two cards repeat in the empty Viewer until a
  composition is viewed.
  It is **the window** between the boot splash and the shell rather than a card over
  either, and it is not shown at all when a `.lum` arrived on the command line. Under the
  cards sit the **recent projects** — a thumbnail of the project as it looked when it was
  last saved, its name, its path, and when it was last opened here (K-468: no format
  column; a size and a rate are per-composition, and a project has many) — with a
  **Clear** that empties the list and a **×** on each row that forgets just that one.
  Neither asks first: nothing is deleted and File ▸ Open brings a project back.
  A footer carries the **product** version — `Lumit 0.2.0`, not the boot line's crate name
  (K-480) — and the two links. **New project asks nothing**: it hands the window to the
  editor on the empty project already loaded, and where the file goes is the first save's
  question (K-617, superseding K-480's picker-first card). **Escape closes the screen** with
  nothing open (K-481), and
  **Settings ▸ General ▸ Workspace ▸ Welcome screen on launch** stands it down for every
  launch — off means Lumit opens straight into the shell, where the same two cards are
  waiting in the Viewer, so the setting hides no choice. The shape and every measurement
  are in [15-DESIGN.md](15-DESIGN.md) §12A.3b–c.
- **Comp with no layers**: the Timeline shows one line of hint text (drag footage here, or
  press the new-Sequence-layer / new-Solid shortcuts). Hints disappear at first content
  and never return unprompted.
- **Tooltips policy**: every icon control has a tooltip with its name and current shortcut,
  on a ~500 ms hover delay. **A tooltip is a name, not an explanation: one or two words,
  never more** (K-440, tightening this rule's original "under five words"). *Add keyframe*,
  not *Add a keyframe here*. A
  control whose state changes says the state — *Visible* / *Hidden*, *Locked* / *Lock* —
  rather than narrating the click. **There is no long form and no exception list** (K-482):
  the sentence-length "rich" tooltip is gone, readouts with live figures included — a
  figure is a word, and the sentence around it was never read. Explanation belongs in the
  settings row's own line, in an empty state, or nowhere.
  `flutter_ui/test/l10n/arb_test.dart` walks every `tip*` key and fails any that runs long;
  nothing may be added to it as an allowance.
  Tooltips MUST never block input, auto-play media, or step users through forced tours.
  **The setting is a switch**, on or off, and off means no tooltip anywhere; there is
  nothing to choose between, because there is only one length.
- No multi-step onboarding wizard or forced tour. The single first-run screen (§13.1),
  empty states, tooltips, and command palette are the entire onboarding surface.

### 13.3 The composition settings dialogue (K-180)

One window serves both **New composition** (Create) and **Composition settings** (Save); they
ask the same questions and differ only in what the button does. It is reached from the
Composition menu, the Project panel's footer button, a right-click on a comp row, and a drop of
footage on that button (§3.1). It is built to the shared dialog pattern
([15-DESIGN.md](15-DESIGN.md) §12A.4, K-469): a kicker title strip, rows of 30 in a 110px
label column, two kicker-titled sections, and Cancel beside the single filled action.

- **Name.**
- **Preset**: the whole formats worth one click — a size and a rate together (`HD 1080p · 25`)
  — reading *Custom* whenever the fields say something of their own.
- **Size**: width × height, with an aspect-ratio lock (on by default — editing one side carries
  the other) and the shape shown beside it in its smallest whole numbers (`40 : 17`).
- **Frame rate**: **one number**, in fps. `600` and `23.976` are both typed as they read; a
  **Presets** list offers the common rates including the NTSC family. The denominator is never
  shown. It still crosses the bridge as the exact `num`/`den` pair — 23.976 reaches the engine
  as 24000/1001 — but that pair is derived from what was typed (docs/14 §2 is unchanged).
- **Duration**: reads and edits as `HH:MM:SS:FF` timecode at the frame rate above (the same
  clock face the Viewer shows; the frames field widens with the rate, so 600 fps counts to
  `:599`). What is *written* is still a length of time in exact seconds, converted at the
  typed rate — never a frame count: a count means nothing without the rate it was counted at,
  and writing one back at a *changed* rate is what used to make the comp longer or shorter
  under layers that had not moved (K-180).

- **Background**: the colour behind everything in the comp, chosen from the ordinary colour
  picker and written with the rest when the button is pressed — the same property the Viewer's
  view menu edits (§2.2), reached from the dialog that decides what a composition *is*.
- **Motion blur**: the master shutter's **angle** in degrees and the number of sub-frame
  **samples** a blurred layer is drawn at (K-120). The shutter's phase and the master on/off
  switch are not here: the switch is the Timeline's, and the phase has no drawn home yet.

Every field lands together, as **one undo step**, including the two that need ops of their own
(K-469).

**Changing the frame rate MUST change only the frame rate.** The comp keeps its length, every
layer keeps its timing, and nothing plays faster or slower — the comp is simply shown at more
(or fewer) frames per second. This has a regression test on both sides of the bridge.

**The playhead keeps its moment, not its number** (K-572). A frame count means nothing without
the rate it was counted at, so the time the playhead sits at is read *before* the new rate is
written and the **nearest** frame of the new grid asked for after: one second in at 60 fps is
one second in at 24. Nearest rather than the frame it lands inside, because a moment that no
new frame falls on would otherwise walk backwards every time the rate was touched. Only the
comp being looked at moves — there is one playhead, and changing a background comp's rate is
not its business. Markers and the work area need no conversion at all: both are stored as
rational time rather than as frame numbers, so they keep their moments untouched.

### 13.4 The Pre-compose dialogue

`Ctrl+Shift+C` in the Timeline, or `Layer ▸ Pre-compose…`, packs the selected layers into a
comp of their own and puts that comp back in their place as a Precomp layer. Both commands are
live only with a comp open and something selected in it; the menu item greys out otherwise.

The dialogue asks two questions the engine cannot answer for the user, and one convenience:

- **New composition name**, prefilled from the first selected layer. Blank falls back to the
  engine's own `Pre-comp N`.
- **Where the attributes go**, as an exclusive pair:
  - *Leave all attributes in '\<this comp\>'* — the layer moves into the new comp stripped back
    to its source, and its transform, effects, masks, Retime, blend mode and switches stay
    behind on the Precomp layer, each of them once. Offered only for a single layer: a stack
    has no one layer for its attributes to stay on, so with more than one selected the choice
    is shown disabled and Move is the answer. The engine refuses the combination too.
  - *Move all attributes into the new composition* — the selected layers move whole.
- **Adjust the duration to the span of the selected layers** (default: on). The new comp's
  duration becomes the selection's own span, the packed layers shift back to start at zero
  inside it, and the Precomp layer covers the stretch the selection covered — so the picture
  does not move. Off, the new comp is as long as this one and no layer changes time at all.
- **Open the new composition** (default: off).

The whole move is one undo step, and the new comp auto-files into the Compositions folder like
any other (K-068). The three answers are remembered in the workspace across launches; the
attribute choice is remembered but overridden by a multiple selection, which can only move. A
refusal from the engine leaves the dialogue open saying so, rather than closing on a move that
did not happen.

### 13.5 The Project settings window (K-286)

**File ▸ Project settings…** (`Mod+Alt+Shift+K`), disabled with no project open, holds the
values that belong to the *project* rather than to this machine — saved inside the `.lum`,
undoable like any other edit, and the same when the file is opened somewhere else. It is a
plain form in the same shape as a Settings page (a named section, rows of what-it-is,
what-it-does, control-on-the-right) in a window of its own, and it exists so that §15's
"almost every value there is machine-local" can go back to being simply true.

- **Anti-aliasing** — the number of coverage samples per pixel the composite is drawn with
  (Off / 2 / 4 / 8, default 8). **One value serves the preview and the export**, which is the
  K-031 identity. Where the graphics card cannot manage the count asked for, a second row
  states what is being used instead, in the calm voice; the project keeps the value its author
  chose and nothing is rewritten behind the user's back.

- **Colour** (K-490, docs/impl/ocio.md §6.4) — the project's OCIO configuration: a row holding
  the path with *Choose…* and *Clear*, a line under it saying what was read ("Loaded: 42 colour
  spaces, 3 displays") or, in one calm sentence, why the file named is not in force; and a
  read-only **Working space** row stating the space the compositing arithmetic runs in, which
  v1 fixes at linear Rec. 709. There is no separate relink: *Choose…* is how a configuration
  that moved is pointed at again.

**Export defaults are not here** (K-588, reversing this paragraph's earlier ruling): the
preset, filename template and destination policy the export dialogue opens on are machine-
local — a preference about how this person exports, never a fact about a project — so they
live in Settings ▸ Export and in a JSON file beside the export-preset library, and a `.lum`
sent to someone else says nothing about where their copy of Lumit writes files. The disk
cache's *Applies to* row stays in Settings ▸ Performance (K-215): choosing between the two
scopes is that control's whole job, so it is the one that stands in both.

### 13.6 Floating windows (K-242)

Every window that floats over the shell — Settings, the theme editor, Export, Composition
settings, New composition, Pre-compose — opens centred and **can be dragged anywhere in the
app window** by any part of itself no control has claimed. Where it was left is remembered
in the machine-local workspace store, keyed by the window, and restored on the next open —
the same session and the next launch. The saved place is an offset from the centre, not a
corner, so a window left near the edge of a large monitor still opens on screen on a small
one; the offset is clamped so the middle of a window can never leave the app window.

The **Settings window is resizable** from a grip in its bottom-right corner, and opens at
880×640 rather than a size fixed for the smallest laptop. Its size is remembered with its
position, and is clamped between 560×380 and the app window. Windows that ask one question
(Pre-compose, confirmations) stay at their natural size — there is nothing in them to give
more room to.

These windows remain modal: a click on the dimmed backdrop dismisses. Moving one is for
seeing what is behind it, not for working while it is open.

---

## 14. Interaction and accessibility rules

Binding, from the household mandate; these override convenience everywhere.

- **User controls tempo**: nothing auto-advances, auto-plays, or animates the user's
  viewport. No scroll hijack — the wheel never zooms or navigates without an explicit
  modifier (§4.6). Focus never jumps except as the direct result of a user action.
- **First click selects, always** (K-448): selection lands immediately on the first click,
  and a double-click's action stacks on top of that selection. No surface waits to see
  whether a second click is coming before selecting.
- **Keyboard reachability**: every control MUST be reachable and operable by keyboard.
  `Ctrl+F6`/`Ctrl+Shift+F6` cycle panel focus; `Tab` traverses controls within a panel;
  arrow keys operate lists, trees, and the Timeline outline. All functionality exposed
  through drag interactions MUST have a keyboard or numeric-entry equivalent (keyframes:
  numeric entry §5.3; docking: a *Move panel to…* command; gizmo: arrow-key nudging).
- **Focus visibility**: keyboard focus always shows a visible focus ring (treatment in
  [15-DESIGN.md](15-DESIGN.md)); focus is exposed through AccessKit with names, roles, and
  values for every control.
- **Hit targets**: controls in low-density surfaces (dialogues, transport, onboarding,
  Export window) MUST be at least 44 px in their smaller dimension. In the dense pro
  surfaces (Timeline switches, keyframe diamonds, lane carets) density wins — visual
  targets go as small as legibility allows, and the compensation is mandatory: an invisible
  hit-slop expanding every interactive element's hit area to at least 24×24 px (clamped so
  neighbouring targets never overlap), a UI scale setting (100/125/150 %), adjustable
  Timeline row heights, and a keyboard path to every switch.
- **Reduced motion**: when the OS requests reduced motion, spring animations do not mount —
  panel transitions, drop-zone previews, and palette entrances become instant or simple
  short fades. No parallax, no bounce. Playback of the user's own content is unaffected.
- **No punishment UI**: errors (missing footage, failed export, expression errors) are
  calm, factual, and never alarm-styled; an expression error disables that expression,
  shows a banner with the message, and renders the keyframed value — never a black frame,
  never a modal.
- **Voice**: UI copy is en-GB, sentence case, no exclamation marks (K-005).

### 14.1 The Escape ladder (K-575)

**One press is one step back.** Escape MUST be answered by exactly one thing: the innermost
surface with something to take back. There is one arbiter for the whole application
(`flutter_ui/lib/widgets/escape_ladder.dart`); a surface registers a claim on its rung and
nothing else adds a keyboard handler for Escape. The order is:

1. **A gesture in flight is abandoned.** A drag, a marquee, a dropper pick, a path being drawn
   with the Pen, a stroke being painted, a type edit, a shortcut chord being captured. Nothing
   was written yet, so this writes nothing and there is no undo step (§2.3.1, §2.3.4, §6.1).
2. **The open popup chain closes** — the whole chain, innermost first, as one click on a
   barrier does (§1.8).
3. **The frontmost dialogue or full-window surface closes**: a modal window, the FX console,
   the command palette, the welcome screen. Closing means the same as clicking the scrim.
4. **The finest selection on screen is cleared** — the picked track points, and any selection
   that registers below them.
5. **Nothing.** Escape with none of the above is not an error and is not swallowed: it travels
   on to whatever else wants it.

A rung with nothing to take back stands aside and the press carries on down, so a tool that is
mounted but idle costs nothing. A **focused text editor is not on the ladder**: it answers
Escape on its own focus node (K-323), which runs after every keyboard handler, so it is reached
only when no rung claimed the press.

---

## 15. Default keymap

### Settings inventory (K-031/K-032 anchors)

The Settings window groups, minimum set. Almost every value here is machine-local (never in
the project file, [10-FILE-FORMAT.md](10-FILE-FORMAT.md) §2); the exceptions are the few that
change what a composition *looks like* or where its own frames are parked, which have to
travel in the `.lum` and are marked below:

- **Performance**: RAM budget for Lumit (default 60% of system, slider + absolute),
  VRAM budget (default 70%), CUDA acceleration on/off (per K-014 it is only ever an
  optional per-node accelerator; off = WGSL path, identical output), decoder pool size,
  worker thread cap, background cache fill on/off.
- **Cache**: cache root folder, disk cache size budget, clear-cache actions (per tier),
  proxy generation policy.
- **Preview**: default preview mode (Cached/Realtime, K-030), Realtime tier bounds,
  adaptive-degradation aggressiveness, audio scrubbing on/off.
- **Colour** (K-031): working-space defaults for new comps, display transform selection,
  footage interpretation defaults. The preview–export parity rule is stated in this panel's
  header text so users understand what the app guarantees.
- **Export**: default preset, export priority default (background/balanced/fast), encoder
  preference order, filename template.
- **Rendering** — *not here at all*: it is the project's, not this machine's, so it lives in
  the **Project settings** window instead (§13.5, K-286).
- **Updates** (K-296), under General: *Automatic updates* — look for a new version at
  launch, at most once a day — on by default, plus a readout of the installed version and a
  button driving the same check the Help row does. Checking is all "on" means; the download
  always waits to be asked for.
- **Language** (K-303, K-311), under Interface: which language the interface is written in.
  Defaults to the machine's own and stores nothing until chosen, so an unset Lumit follows
  the operating system for ever rather than freezing whichever language it first opened in.
  The list names each language in its own language — Deutsch, Қазақша, Українська, 简体中文,
  繁體中文 — so somebody who has chosen one they cannot read can find their way back.
- **Keymap**, **Interface** (UI scale, tooltips, whether shortcut hints show throughout
  the UI — the main menu excluded (K-448) — and reduced motion follows OS or override),
  **Autosave** (interval, copies kept), **Plugins** (search paths, disabled list,
  per-plugin overrides).

**The window (K-193, K-194).** It opens from **Window → Settings…** or **Ctrl/Cmd+comma** — a
sidebar of pages, each a stack of named sections, each section a card of rows carrying what
the setting is, a line saying what it does, and its control on the right. Its pages are
**General** (reset workspace, version and build), **Appearance** (colour scheme with an
eight-swatch preview beside it, the theme shelf — Duplicate, Rename…, Delete, Import… and
Export… (K-298) — corners,
interface motion, and the Scopes and Viewer toggles — themed scope colours, themed surround,
and whether the Viewer smooths the picture when it is zoomed past 1:1, all three off by
default: a magnified pixel is a square, because looking at the pixels is what zooming in is
for), **Interface** (UI scale, tooltips, and whether the Effect controls panel
repeats the layer's Source, Transform and Retime rows — off by default, since the Timeline's
fold-out already shows them), and **Performance** (playback mode, quality tier and reset,
and the RAM and VRAM frame-cache budgets with their readouts and Clear buttons). The two
budgets are **typed and draggable numbers capped at what the machine has** — installed RAM
and the adapter's dedicated video memory, asked of the engine — rather than a pick from a
fixed list of sizes (K-194).

**The disk tier's controls landed with K-214**, as a third section on the same page: its
budget (the same typed-and-draggable row), a readout of what is parked and where, and a
**Where** row choosing between *With Lumit* — the application's own cache folder, the default
and the only one that works before a project has been saved — *Beside the project*, which is
the per-project choice, and *A folder I choose*, which offers a folder picker beside the
dropdown. An **Applies to** row beside it chooses the scope (K-215): *Everything*, kept in the settings
file, or *This project*, kept inside the `.lum` so it travels with a copy of the project —
a project's own answer overriding the application's. Switching back to Everything clears the
project's answer rather than copying the application's into it, so the project follows along
afterwards; and because it is an ordinary op, giving a project its own location undoes like any
other edit. Changing any of this moves nothing, so the old folder can be deleted by hand
whenever the user likes. Its **Clear** asks before deleting,
unlike the other two tiers': RAM and VRAM cost a re-render each, while this one destroys files
that may be a night's work and there is nothing to undo. With nothing parked it does not ask —
a question about deleting nothing is only noise. The status line's cache meter grew a matching
third bar (Disk), which asks the same question when clicked. **Autosave** is its own page
(K-587): how often a spare copy is written, in minutes, and how many copies are kept, with
zero minutes meaning off. **Export** is its own page too (K-588), and is the last of the
drawing's nine to arrive: a **default preset** (the named preset the export dialogue opens
on, or *None* for the first built-in), a **filename template** in the tokens the exporter
already substitutes — `{comp}`, `{preset}`, `{date}` (K-119), blank giving each preset's
own suggested name — and a **destination policy** of *Ask every time* / *Beside the
project* / *A folder I choose*, the last offering a folder picker beside the dropdown and
reporting the folder under the row. All three are machine-local, kept in a JSON file beside
the export-preset library and never in a `.lum`; the export dialogue's preset strip carries
a **Set as default** action for the preset alone, which is the one of the three a person
decides while looking at it.

All bindings are remappable in Settings → Keymap (search, conflict detection, per-context
display); the keymap serialises to a shareable file. An "After Effects" alternate preset
ships for muscle-memory cases where Lumit's default deviates. Notable deviations from AE:
`J/K/L` are shuttle transport (the audience's NLE habit, per the layout brief), so keyframe
navigation moves to `,`/`.`; Viewer zoom therefore lives on `Ctrl+=`/`Ctrl+-` and the wheel.
Inside the **Timeline** `L` reveals a layer's Audio instead (K-281) — the panel where you
reach for a layer's sound is the panel where you are least often shuttling — and the
transport keeps it in every other context.

**Shadowing is not a clash (K-281).** A binding scoped to a panel takes a chord over from an
app-wide one while that panel is focused; which action fires is decided by a stated rule (the
focused panel gets first refusal, app-wide is the fallback), so Settings → Keymap reports
those as *shadows* — a quiet note above the table reading "`Ctrl+Z` — Zoom time in in the
Timeline, Undo elsewhere", not a bordered warning — rather than as conflicts to resolve. It is
said at all because the app-wide meaning does stop working in that one panel. Two bindings in
the *same* context, which nothing can tell apart, remain a conflict and keep the banner; a
rebind cannot make one (the previous owner is evicted), so in practice the banner is what an
imported keymap file trips.

**Shipped (K-199).** Settings → Keymap is a table, grouped by the context a binding is live
in, with the action's name on the left and its chords on the right — click a chord cell and
press the keys you want, `Escape` to leave it, `Backspace` to clear it, `Reset` to put the
shipped chord back. Above it: a search box that matches what the table *shows* as well as
the ids underneath, the two presets, and Import / Export for the shareable file. A chord
another action already holds is taken rather than refused (refusing would make swapping two
actions' keys impossible) — within one context the previous owner's row simply goes blank,
and across contexts sharing a chord the panel-scoped one simply wins where it is focused
(K-281, reported as a shadow note rather than a banner). One row, one chord (K-200): no
shipped action carries two, and a user who wants a second spelling of a command binds it
themselves.

The model is `lumit-keymap` and the seam is `crates/lumit-bridge/src/api/keymap.rs`: the
engine decides what a chord means and the frontend only spells the keypress and draws the
answer (K-199). The keymap is stored in the workspace file as the engine's own JSON, so it
survives a restart in the same format it exports in.

One honest gap. The **Tools** context arms the toolbar's
tools (§1.7) and cycles a group on a repeat press, but what most tools then *do* is not built
yet, so the chord lands and the picture stays as it was. The **Project**, **Panels** and
**Effects** contexts, which once had rows and nothing behind them, are built: `Enter` renames
in the Project panel and in Effect controls (K-321), and the Panels context cycles the focus
ring and focuses a panel's search field. Stepping a frame has a second
chord alongside `Page Down`/`Page Up` — `Ctrl`+arrow (K-282); the **bare** arrows do nothing
app-wide, so a list, a field or a canvas is free to use them for moving within itself.

| Context | Key | Action |
|---|---|---|
| Global | `Space` | Play / pause |
| Global | `J` / `K` / `L` | Shuttle reverse / pause / forward (repeat `J`/`L` steps ×2, ×4, ×8) |
| Global | `Page Down` / `Page Up` | Next / previous frame |
| Global | `Ctrl+→` / `Ctrl+←` | Next / previous frame (K-282; `Cmd` on macOS) |
| Global | `Shift+Page Down` / `Shift+Page Up` | ±10 frames |
| Global | `Home` / `End` | Comp start / end |
| Global | `Shift+Home` / `Shift+End` | Work area start / end |
| Global | `I` / `O` | Go to selected layer's in / out point |
| Global | `,` / `.` | Previous / next keyframe on revealed properties |
| Global | `Ctrl+,` / `Ctrl+.` | Previous / next edit point or layer boundary |
| Global | `B` / `N` | Set work area start / end at playhead |
| Global | `*` (numpad or `Shift+8`) / `Shift+M` | Add marker at playhead. `M` keeps Reveal Masks, so the letter form takes Shift (K-254) |
| Global | `Shift+0…9` | Set numbered marker at playhead — pressing it again *moves* that marker, and it replaces whatever is on that frame (K-254) |
| Global | `0…9` | Go to that numbered marker; nothing happens until one has been set (K-254) |
| Global | `Delete` / `Backspace` | Delete the selection — keyframes when any are selected, else the layer (TF-6) |
| Global | `Ctrl+Shift+P` | Command palette |
| Global | `Ctrl+Space` | FX console (K-324) |
| Global | `Ctrl+M` | Add active comp to export queue |
| Global | `Ctrl+K` | Composition settings |
| Global | `Ctrl+Alt+Shift+K` | Project settings (K-286) |
| Global | `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / redo |
| Global | `Ctrl+Alt+N` / `Ctrl+N` | New project / new composition (After Effects' pairing) |
| Global | `Ctrl+O` | Open a project |
| Global | `Ctrl+S` / `Ctrl+Shift+S` | Save / Save as |
| Global | `Ctrl+I` | Import footage |
| Global | `Ctrl+Alt+M` | Export the composition |
| Global | `Ctrl+X` / `Ctrl+C` / `Ctrl+V` | Cut / copy / paste the selection — picked keyframes, else the selected property rows (their keys, or their plain value where they have none, K-301), else the picked effects, else the layer (K-300) |
| Global | `Ctrl+A` / `Ctrl+Shift+A` | Select all layers / deselect all |
| Global | `Ctrl+Alt+;` | Settings (Preferences) |
| Global | `Alt+Shift+1…9` | Switch workspace |
| Global | `` ` `` | Maximise / restore panel under pointer |
| Tools | `V` | Selection tool |
| Tools | `H` | Hand (pan) — also held-`Space` drag in the Viewer |
| Tools | `Z` | Zoom tool (`Alt` to zoom out) |
| Tools | `Y` | Anchor point tool |
| Tools | `C` | Razor tool (Sequence layers and layer splitting) |
| Tools | `Q` | Shape/mask tool cycle |
| Tools | `G` | Pen tool cycle |
| Tools | `W` | Rotation tool |
| Tools | `Ctrl+T` | Type tool cycle |
| Tools | `Ctrl+B` | Brush / clone stamp / eraser cycle |
| Tools | `Alt+W` | Roto brush / refine edge cycle |
| Tools | `Ctrl+P` | Puppet tool cycle |
| Tools | `Shift+C` | Camera tool cycle (AE's `C` is the razor here, §1.7) |
| Timeline | `P` `S` `R` `T` `A` | Reveal position / scale / rotation / opacity / anchor |
| Timeline | `E` / `M` | Reveal effects / masks |
| Timeline | `U` / `UU` | Reveal the keyed rows / every modified property (K-622) |
| Timeline | `L` / `LL` / `LLL` | Reveal Audio / and its waveform / shut again (K-281; `Shift+L` does the same). Inside the Timeline this takes `L` from the shuttle transport, which keeps it everywhere else |
| Timeline | `[` / `]` | Move layer in / out to playhead — the whole bar, keyframes sliding with it (A8, §4.7) |
| Timeline | `Alt+[` / `Alt+]` | Trim layer in / out at playhead — one edge only, keyframes staying where they are |
| Timeline | `Ctrl+Shift+D` | Split layer / cut clip at playhead |
| Timeline | `Ctrl+D` | Duplicate selection |
| Timeline | `Ctrl+Shift+C` | Precompose |
| Global | `Ctrl+Alt+T` | Give the selected layer a Retime, or take it away (K-197, narrowed to this one chord by K-200 — AE's own, and one Windows cannot steal; the Composition-menu route is K-198) |
| Timeline | `=` / `-` | Zoom time in / out (`Ctrl+wheel` at pointer) |
| Timeline | `\` | Toggle full-comp zoom / previous zoom |
| Timeline | `Enter` | Rename selected layer |
| Project panel | `Enter` | Rename the selected item (K-321) |
| Effect controls | `Enter` | Rename the selected effect (K-321; the heading, not a parameter row) |
| Timeline | `X` | Toggle selected layer visible switch |
| Graph editor | `Shift+F3` | Toggle graph editor |
| Graph editor | `F9` / `Shift+F9` / `Ctrl+Shift+F9` | Ease / ease in / ease out |
| Graph editor | `F` | Auto-zoom fit selection |
| Viewer | `Shift+/` | Fit magnification |
| Viewer | `Ctrl+=` / `Ctrl+-` | Zoom in / out |
| Viewer | `Ctrl+J` / `Ctrl+Shift+J` / `Ctrl+Alt+J` | Preview resolution full / half / quarter |
| Viewer | `Ctrl+R` | Toggle rulers |
| Viewer | `Ctrl+'` | Toggle transparency grid |
| Panels | `Ctrl+F6` / `Ctrl+Shift+F6` | Cycle panel focus forward / back |
| Panels | `Ctrl+F` | Focus the panel's search field (Project, Effects & Presets) |

macOS development builds map `Ctrl` to `Cmd` (K-001).

---

## 16. Visual language

Everything visual — colour tokens, dark-native Aizome variant, hairline borders, type stack,
icon set, spacing, cache-bar tier colours, marker styling, focus ring treatment — is
specified in [15-DESIGN.md](15-DESIGN.md) (K-004). This document intentionally contains no
colour or dimension beyond hit-target minima. Where this document names a state that needs
visual distinction (degradation badge, overrun hatching, beat vs ordinary markers, cache
tiers, proxy badges), 15-DESIGN.md MUST define exactly one treatment for it.

---

## Open questions

1. **Graph editor as a detachable panel** — v1 specifies it as a Timeline mode; should it
   also be dockable as a standalone panel locked to a comp (useful on wide monitors)?
2. **Snapshot/compare in the Viewer** — AE's snapshot slots (`Shift+F5…`) are useful for
   grading; deferred from §2.2. Ship in v1 or with the Colour workspace maturation?
3. **Align panel and Properties-style quick panel** — not specced; decide whether v1 needs
   an Align panel or whether gizmo snapping covers the need.
4. **Per-clip thumbnails in Sequence layers** — decode cost versus orientation value at
   small row heights; needs prototyping against the thumbnail cache.
5. **Keyframe navigation keys** — `,`/`.` deviates from AE (which uses them for zoom) and
   from AE's `J`/`K` keyframe navigation; validate with target users before locking the
   shipped default.
6. **Scopes tap point** — specced as pre-display-transform; colour-managed workflows may
   want a post-transform option. Revisit with [15-DESIGN.md](15-DESIGN.md) and the colour
   management spec.
7. **Touchscreen/pen support** — hit-slop rules assume mouse; whether pen scrubbing and
   touch panning are v1 or later is unowned.
8. **Workspace strip overflow behaviour** on narrow windows (menu versus scroll) — trivial
   but undecided.
