# Several Viewers at once: the multi-viewer note

**Decision:** a *view* becomes a first-class thing with a stable id of its own; a Viewer
panel holds one to four views; the dock learns pane instances so there can be more than
one Viewer panel; the engine's shared-texture pool is keyed by view as well as size; one
view is active and the document panels follow it. **Related:** the Viewer texture
transport (17), the frame key (06 §5.2), the playback scheduler note, the dock model, the
workspace-versus-project split (07 §1.5), the locks (07 §2.6). This note is the
authoritative *how* for the whole topic: the model, where each piece of state lives, the
engine work, the traps, and the ordered work packages with their tests.

**Where this note and a spec disagree the spec wins and this note is the bug**, except in
the four places named in §4.4, where the spec is amended by the package that reaches it.

## Status, 2026-09-10

Built: MV1 to MV6, MV8, MV9 and MV10, plus MV7's engine half. Two things are outstanding
and both are stated plainly rather than buried.

- **MV11 (3D views) is not built.** It is the one package that reaches into the
  compositor, and nothing above it depends on it.
- **Three tests in `flutter_ui/test/frb/viewer_panel_frb_test.dart` regress**: `play
  advances the playhead, and stopping returns it`, `a playhead move from outside the
  Viewer renders`, and `a still playhead stops asking for renders`. Each passes on its
  own and fails when another test in the file has run first, which is a second render
  worker in the one process. It reproduces in eleven seconds with

  ```
  flutter test test/frb/viewer_panel_frb_test.dart --name 'transport steps|play advances'
  ```

  and the same pair passes on a worktree of the commit this branched from, so it is this
  change and not the machine. In the failing run the second worker publishes no frame at
  all inside the cold-worker budget. Ruled out one at a time, each by disabling it and
  re-running the pair: the `_arrived` active-view gate, the new shape of `requestFrame`,
  the view binding in `setSelectedComp`, `look_at`, the per-view drain and its
  active-view priority, the view key on the frame-name memo, the view key on the
  shared-target pool, and the extra `watch` in the new Viewer layout wrapper. The
  surface's `initState` runs exactly once per test, so it is not the panel being rebuilt.
  What is left to look at is why the **second** renderer in a process is slower to its
  first frame than it was: the pair takes 8 s on the baseline and 11 s here.

## In plain terms

Today Lumit shows one picture. There is one Viewer panel, it shows whichever composition
is fronted, and every other panel shows that same composition because there is only one
answer to "which composition". People who arrive from After Effects and Resolve expect
better: a comp in one picture and its precomp in another, a shot beside the graded
version of it, a piece of footage previewed without making a composition for it, and a
padlock so that opening something else does not steal the picture they were looking at.

The change is one idea repeated. Everywhere the code says "the composition" it has to say
"the composition *this view* is showing", and everywhere it says "the picture" it has to
say "the picture *for this view*". The engine already caches frames by content, so two
compositions on screen at once cache correctly with no new key. What the engine does not
have is any notion of *who asked*, and that is the whole of the engine work: a view id on
the request, a view id on the frame that comes back, and one piece of shared graphics
memory per view instead of one per size.

The trap that makes this more than plumbing is that piece of graphics memory. The engine
keeps a handful of shared textures and picks one by width and height. Two views on two
1920x1080 compositions ask for the same size, get the same texture, and each overwrites
the other. Both pictures flicker between two compositions and nothing in the code looks
wrong. That is the first thing to fix and the first thing to test.

## 1. The model

### 1.1 Three words, kept apart

- **Viewer** is the panel, exactly as the glossary has it. There can be several.
- **View** is one picture surface inside a Viewer panel. A Viewer panel holds one, two or
  four of them in a layout. A view is bound to one **item** and carries its own way of
  looking at it. It has a stable id.
- **Item** is what a view shows: a composition, a footage item, or a layer's source. The
  three display modes of 07 §2.1, unchanged.

"View" is already spoken for twice in the interface (the OCIO *view*, the Viewer's *view
menu*), so in identifiers and prose the thing in this note is a `ViewerView` / `view`
bound to an item, and the OCIO one keeps its qualifier: `colour_view`, as the bridge
already names it. The glossary gains the row (§4.4).

A view is not a panel. It has no tab, it is not in the dock tree by itself, and it cannot
be dragged out on its own. What is in the dock tree is the Viewer panel; what is inside
the panel is the layout and its views.

### 1.2 The view id

Every view has a **uuid minted when it is created** and kept for as long as it exists. It
is the join between the two files that describe a view:

- the workspace says which views exist, how they are laid out inside which Viewer panel,
  and the display preferences that belong to the user rather than to the work;
- the project says, per view id, which item it shows, whether it is locked, and how it is
  being looked at.

A view id in the workspace with no entry in the project opens showing the active
composition, unlocked, looking neutral. An entry in the project naming a view id that the
workspace does not have is dropped on load and not written back. That is what makes a
workspace file shareable (07 §1.4: workspaces are files users send each other) while a
lock stays a reference to project content (07 §1.5).

The id must not be an index. Views are added and closed; an index would silently re-point
a lock at a different picture, which is the exact failure 07 §1.5 exists to prevent.

### 1.3 Where each piece of state lives

This is 07 §1.5's table taken down to the view. The middle column is the home; the third
says why, and names the amendment where this disagrees with the spec as written.

| State | Home | Note |
|---|---|---|
| Which Viewer panels exist, their layouts and splits | workspace | frame tree, 07 §1.5 |
| Which views exist and their order in a layout | workspace | as above |
| Share view options across views | workspace | a preference about working, not about the work |
| Always preview this view | workspace | names a view id; about working |
| The item a view shows | project | project content reference, 07 §1.5 |
| Lock | project | 07 §1.5, §2.6 |
| Magnification and pan | project, per **view** | 07 §1.5 says per comp; amended (§4.4) |
| Channel view, transparency board | project, per **view** | 07 §1.5 says per comp; amended (§4.4) |
| Exposure, tone map, OCIO display and view | project, per **view** | 07 §2.2 items 12, 13 say per comp; amended (§4.4) |
| Region of interest | project, per **view** | 07 §2.2 item 7 says per comp; amended (§4.4) |
| 3D view, wireframe, layer-controls switch | project, per **view** | new |
| Preview resolution | project, per **comp** | unchanged, 07 §2.2 item 2: a cost decision about the shot |
| Guides, rulers, grid, safe areas | project, per **comp** | unchanged, 07 §2.2 item 6: marks on the shot |
| Composition background colour | document | unchanged: it is an edit, and goes through an op |
| Snapshot slot | memory, per view | display-side and never stored, 07 §2.2 item 14 |
| The "at effect" chip | memory, per view | it clears itself on any selection change |

The four amendments have one argument behind them. **A view is where you are looking, and
two views exist so they can be looked at differently.** A per-comp magnification makes a
two-up compare impossible; a per-comp exposure makes a before-and-after grade impossible;
a per-comp region means arming one in the left view crops the right. Preview resolution
and the guides go the other way for the same reason: how coarsely a heavy shot previews,
and where its guides are, are facts about the shot rather than about the pane it is in.

The reading rule that keeps this cheap: **a view seeded for the first time takes its look
from the comp's stored one.** Projects written before this change hold looks keyed by
composition id; on load, the first view bound to a composition takes that composition's
stored look, and thereafter the view owns it. No project opens looking different from how
it was left.

### 1.4 The active view, and what follows it

Exactly one view is **active**. It is fronted by a click anywhere in it, by the keymap, by
the menus, and by any action that opens an item into it.

**What follows the active view**: the Timeline, Effect controls, the Graph panel, the Node
panel, the Mixer, the Audio panel, the menu bar's composition-scoped rows, the command
palette's composition context, and the Scopes panel's trace source. In code this is the
whole of the `LumitUiState.selectedComp` surface (31 reads over 20 files); it becomes
`activeView.comp`, and `selectedComp` becomes a getter over it so the change is one
definition and not thirty-one edits.

**Two exceptions, both because the alternative blanks a panel**:

- A view showing **footage** or a **layer source** does not move the Timeline, the Mixer
  or the Audio panel. They keep the composition they had. Double-clicking a piece of
  footage must not empty the panel below it.
- The Effect controls panel follows the layer selection, which is already its own thing,
  and a footage or layer view makes no layer selection.

**What fronting does not do**: it does not re-render anything, it does not touch the
transport, and it does not move any playhead. Fronting is a change of which state the
other panels read, and it is exactly one notification.

**When the active view is locked**, everything above still holds: a locked view is fronted
and followed like any other. The lock governs only what happens when an item is *opened
from somewhere else*, which is §1.5.

### 1.5 Locks, and where an opened item lands

A locked view does not change item. That is the whole of the rule, and the rest is where
an opened item goes instead. Opening a composition (the Project panel, the tab strip, a
precomp layer, the palette, the hierarchy) resolves in this order:

1. the active view, if it is unlocked;
2. otherwise the most recently active unlocked view, in any Viewer panel;
3. otherwise a new view: a new pane in the active Viewer panel if its layout has room,
   and a new Viewer panel if it does not.

Step 3 is After Effects' behaviour and it is the one that makes locks usable: with every
view locked, opening something still shows it rather than doing nothing. The view an item
lands in becomes active.

A lock survives the item being deleted: the view shows the empty state, says what it was
locked to, and the lock is cleared by the user rather than silently by the engine. A
project whose comp has gone must not quietly re-point a padlock at a different picture.

## 2. The engine

### 2.1 A view id on every request

`WorkerState` gains a **latched active view** and a per-view map of everything it holds
per Viewer today. Three calls carry the view id, because those three are what decide what
a view is showing:

- `CompositionReference::render_frame(frame, scale, mode, prefix, view)`
- `CompositionReference::play(from, scale, mode, view)`
- `CompositionReference::set_viewer_look(..., view)`

and `sample_pixels` takes it too, so a dropper read names the picture it is reading.
`stop_playback` does **not**: one view plays at a time and the worker is holding its id,
so a stop is unambiguous without one.

The thirteen `render_frame_with_*` staging calls (the drag previews) **do not** grow the
parameter. They inherit the latch, exactly as they already inherit the prefix point.

```rust
// ponytail: the drag-preview renders inherit the latched active view rather than
// naming one. The ceiling is a preview render asked for by a view that is not
// active, which the interface cannot currently produce: a drag on the picture
// fronts the view it is in before it stages anything, and every other stager
// (Effect controls, the Timeline, the graph) already reads the active view. The
// upgrade is the same `view` field on `RenderCompRequestWithPreview` and its
// siblings, which is a mechanical widening if that ever stops being true.
```

`WorkerResponse::RenderedSharedTexture` and `RenderedDMABuf` carry `view: u32` on
`BridgeSharedFrameInfo` / `BridgeSharedFrameInfoLinux`, beside the `tier` that is already
there for the same reason: the frame that changes it brings it. `Sampled`,
`RenderProgress` and `FrameProfile` carry it too, so a progress bar belongs to the view
that is waiting and a measured frame is attributed to the picture it was made for.

The id crosses as a `u32` index rather than the uuid: it is minted per open project by the
frontend when a view first asks for a picture, it is small, and it keeps the frame
response cheap. The uuid stays the frontend's, which is where persistence lives.

### 2.2 The shared target pool: the collision, and its ceiling

`crates/lumit-render/src/headless.rs`, `SHARED_TARGET_POOL = 4`, three platform siblings
(`shared`, `shared_dmabuf`, `shared_iosurface`) with identical shapes. Today an entry is
found by `sh.width == aw && sh.height == ah`. **Two views on same-sized compositions share
one texture and overwrite each other every frame.**

The fix is the key, not the size:

- entries are keyed by **(view, width, height)**;
- `present_prepared` and its two siblings take the view id and look up that triple;
- **every live view keeps at least its most recent entry, always**, so a view can never be
  evicted into re-minting a handle on every frame (which is the "Binding D3D surface
  failed" failure the current comment records, arrived at from the other direction);
- beyond that one-per-view floor, eviction is global least-recently-used against a **byte
  ceiling**, not an entry count.

A byte ceiling because an entry count is a lie at 4K. One 1080p entry is roughly 16 MiB by
the existing comment's reckoning (two textures' worth); one 4K entry is roughly 66 MiB.
Four entries of each is 66 MiB and 265 MiB for the same number. Four views in a 2x2 on a
4K composition, with one spare size each, is 530 MiB of graphics memory doing nothing but
holding handles.

**The ceiling is measured, not guessed.** MV1 measures it with the instruments docs/13
§7.0.1 already names, and records the number here:

- `GpuContext::settle` before every reading, because a CPU that has run ahead of the card
  still holds every frame the card has not reached and a non-blocking poll cannot free
  those;
- `HeadlessRenderer::gpu_allocator_bytes` for bytes on D3D12 and Vulkan;
- `gpu_live_objects` for the count, which is the reading that works on every backend.

Starting point for the measurement, to be confirmed or replaced by it: **256 MiB**, with
the one-per-view floor exempt from it. A view that cannot fit its own current size inside
the ceiling still gets its texture; the ceiling governs the spares.

The pool's bytes are a frame-sized allocation and therefore the governor's business
(docs/13 §3, "every frame-sized allocation is registered"). They are registered, and they
appear in the memory report as their own row.

### 2.3 The look is per view, and the renderer holds one

`HeadlessRenderer` holds exactly one look: `set_display_view`, `set_colour_view`,
`set_transparent_background`, `set_region`, applied through `apply_viewer_look`. With per
view looks (§1.3) the worker applies the asking view's look immediately before its render,
and skips the four setters when the view's look is already the one on the renderer.

This is cheap and it is already correct in the cache, which is the reason it is the right
rung. `HeadlessRenderer::named_under_view` folds the exposure, the tone map, the region,
the OCIO display and view and the colour config's own content hash into the frame name,
under its own tag, returning the name untouched when the look is neutral. So two views
looking at one composition differently name different frames, both bank, and switching
back finds the old frames still there. **No cache key changes for this note.** Same for
`WorkerState::names`: the memo is keyed by revision, comp, frame and quality tag, so it
must gain the view's look identity or be cleared on a look switch, exactly as the prefix
latch already clears it.

One renderer, not one per view. Building a device is serialised on purpose
(`BUILDING_RENDERER`, and the comment at `build_viewer_renderer` says why: twenty at once
is not faster than twenty in turn, and it exhausted the card). Four views must not be four
devices. This note takes no position on ever having a second renderer; it says the four
views share the one, and §2.4 is what makes that liveable.

### 2.4 Scheduling across views

docs/impl/playback-scheduler.md is binding. Renders are serial on the one worker thread,
which is a recorded shortcut with a `ponytail:` comment on `Playback` naming its ceiling:
a stop, a seek or a new play waits for the frame already in flight, up to about 200 ms on
the reference composition at 32 animated layers, against the 15 ms §1 asks for.

**With several views that shortcut stops being about the transport and becomes about
whether the picture you are dragging in responds at all.** Four rules make it liveable,
and all four are in MV6:

1. **A view asks only when its own inputs move.** Its item's document revision, its own
   frame, its own scale, its own look, its own prefix. A locked view on an untouched
   composition costs one render for its lifetime. This is the single largest saving and it
   is a frontend rule, not an engine one.
2. **The drain policy collapses per view.** `drain_to_newest` today collapses every
   picture request to the newest, which with several views would starve every asker but
   the last. It becomes newest-per-view: at most one outstanding picture per view,
   preserving arrival order between views.
3. **The active view is served first.** Within one turn, the active view's outstanding
   picture goes before any other's. A background view is served one frame per turn and
   never inside a burst of scrub requests.
4. **Epochs are per view.** A scrub in the left view cancels the left view's in-flight
   work and leaves the right view's alone. This is the epoch model of the scheduler note
   applied at the granularity the interface now has, and the frontend does not do it: the
   worker bumps a view's epoch when a newer request for that view arrives.

The remaining ceiling, stated so it is not discovered: **a background view's render can
delay the active view's next frame by one render.** Rule 1 makes that rare (a background
view with nothing new asks for nothing) and rule 3 makes it at most one. The upgrade is
the in-render epoch tokens already recorded under docs/TODO.md's playback-scheduler
follow-ups, and it is the same upgrade the single-Viewer ceiling already waits on. The
work package carries the `ponytail:` comment saying so.

### 2.5 Playback, and what the other views show

**One view plays.** It is the "always preview this view" view if one is set, otherwise the
active view. `PlayRequest` carries the view id, and only that view's frames are published
while the transport runs.

The other views **hold the last picture they were given** and are not re-rendered. Not
blanked, not greyed, not degraded: a still picture of the right composition, which is what
those views were showing a moment ago and what they will show again. Nothing on their bars
changes, which is 07 §2.2's rule about the bar not moving while playback runs, applied to
the panels that are not playing.

When playback stops, each other view whose inputs moved while it ran asks once, on the
first idle turn. The idle cache fill (docs/06 §5.5) keeps its anchor on the **playing
view, or the active view when nothing is playing**, and not on all of them: filling four
work areas at once fills none of them.

**The transport belongs to the composition, not to the panel.** Pressing play with a
footage view active plays the composition the Timeline is on, because that is what the
transport has always meant. A footage view has its own source-time strip (07 §2.1) and
that is a separate thing.

## 3. The frontend

### 3.1 The dock learns pane instances

`flutter_ui/lib/state/dock.dart` identifies a pane by its `Panel` enum value. `panelsIn`
returns `List<Panel>`, `movePanel`, `_removePanel`, `_tileOf` and `activatePanelTab` all
match on it, and the invariant they keep is that every panel appears exactly once. Two
Viewer panels break that invariant, so the identity has to widen.

The smallest widening that holds: **`DockPane` gains an instance number**, and a small
value type `PaneId(Panel panel, int instance)` becomes what everything matches on.

- `{'kind': 'pane', 'panel': 'viewer'}` still reads, as instance 0. Old workspaces load
  unchanged, which 07 §1.4 requires because workspaces are files users send each other.
- Instance is written only when it is not 0, so a workspace with one Viewer serialises to
  exactly the bytes it does today.
- `panelMinWidth` and `dockMinWidth` key on `panel`, not on the instance: two Viewers have
  the same floor.
- The Window menu's tick list stays per `Panel` for every panel that is a singleton, and
  the Viewer row gains "New Viewer" beside it. Only the Viewer is instanceable in this
  note. Nothing else asks to be, and making every panel instanceable is exactly the
  speculative generality the ladder refuses.

`LumitUiState.activePanel` becomes a `PaneId?`, and `_contextOf` reads its `panel`.

### 3.2 View layouts inside a Viewer panel

One, two or four views; horizontal or vertical for the two-up; a 2x2 for the four-up. The
layout is a small enum on the Viewer panel and lives in the workspace beside the pane.
Changing it adds or removes views: growing mints new view ids bound to the same item as
the active view (which is what makes the split immediately useful), shrinking closes the
last views and drops their project entries.

**The share view options switch** (07 has no row for this yet; it is item 6 of the
commission and After Effects' own) makes the per-view group of §1.3 follow the active view
instead of being held per view. It is one boolean read at the point the view's state is
resolved, not a second copy of the state: with it on, every view reads the active view's
magnification, channel, board, look and overlays; with it off, its own. Turning it off
leaves every view holding what it was showing, which is the only behaviour that does not
lose work.

### 3.3 The texture controllers

`ViewerTextureController` is already per-instance and the Windows runner keeps
`entries_` keyed by texture id, so the runner half needs nothing. What is single today is
the *ownership*: `LumitUiState` holds one `controller` and one frame handler.

It becomes a map from view id to controller, and the frame handler routes on the view id
the frame now carries (§2.1). A view closing disposes its controller, which unregisters
its texture. A controller is created on a view's first frame, not on its creation: a view
that has never been given a picture has nothing to register.

`viewerPictureKey`, the global that lends the Viewer's `RepaintBoundary` to the project
thumbnail, becomes the **active view's** boundary. A thumbnail is a picture of the
project, and the active view is what the user was looking at.

**No bridge calls in rebuild paths.** The rebuild-budget test expects zero and it will
find any regression here first: a view reading its binding, its look or its lock in
`build()` is a bridge call per view per frame. All of it is Dart state fed by the read
model and by the response stream, per docs/impl/ui-performance.md §4.5.

### 3.4 Compare: wipe and split

Two views, a divider, and nothing crosses the bridge. Both views already publish their own
texture; the compare draws view A's `Texture` clipped to one side of the divider and view
B's to the other, in one stack, with the divider draggable. Split is a hard edge; wipe is
the same edge with the two pictures aligned to the same rect so the seam reads as one
picture.

It is a display affordance in the shape of the snapshot (07 §2.2 item 14): nothing crosses
to the engine, nothing enters a cache, and nothing goes near an export. The two views keep
their own bars and their own state; the compare is a way of stacking them.

Alignment is the trap: two compositions of different sizes cannot share a rect honestly.
The rule is that the compare aligns on the **active view's** picture rectangle and letters
the other into it, and says so in the chrome rather than silently scaling.

### 3.5 Cinema

`panel.maximise` is bound to backtick in `crates/lumit-keymap` and is **not implemented in
the frontend**. Cinema is the same mechanism with a smaller frame: the active view fills
the window, its bars stay (a picture with nothing saying what it is is not a feature), and
Escape comes back, per 07 §14.1's ladder.

Both land in one package because they are one mechanism: a maximised pane id held in
`LumitUiState`, and a dock render that draws that pane alone when it is set. Backtick
maximises the panel under the pointer as the keymap already promises; the cinema chord
maximises the view.

### 3.6 The footage view and the layer view

07 §2.1 already has both display modes. The interim answer, the Project panel's preview
card that hover-scrubs off `FootageReference::thumbnail` and plays sound files, stays: it
is right for a hover and it costs 36 KiB against a frame's 8 MiB.

The full answer needs a real picture down the zero-copy transport, and the engine has one
path to a picture: composite a composition. So:

**A view on an item renders a scratch composition built around it, engine-side, on a clone
of the document.** Exactly the shape `render_frame_with_preview` already uses to stage a
drag: a patched clone, never a commit, no journal entry, no undo step, nothing in the
Project panel. The scratch comp is sized to the item (its natural size for footage; the
layer's source size for a layer view), holds one layer, and its id is **derived
deterministically from the item id** so the frame it names is stable across asks and the
cache works normally.

- **Footage view**: one layer of the item at identity transform, no effects, the item's
  interpretation applied (which is where it already lives). Its own source-time strip for
  setting source in and out, which is 07 §2.1's requirement.
- **Layer view**: the layer's source before transform, with its masks and its anchor point
  drawn over it. After Effects' Render tick (source versus rendered-with-effects) is the
  same clone with the effect list kept or emptied, which is one boolean on the request.

Double-clicking a footage item opens a footage view; double-clicking a layer opens a layer
view. Both land in a view by §1.5's resolution, so a locked view is not stolen.

This is the rung that adds no pixel path, no second transport and no cache semantics. What
it costs is one document clone per render of such a view, which is what a drag preview
already costs per tick.

## 4. Persistence, old files, and the specs

### 4.1 The workspace file

`SavedSession.dock` is raw JSON already, and `DockNode.fromJson` drops a pane it cannot
make rather than throwing (the comment there is explicit that a folded-away panel must not
cost anyone their arrangement). The instance number rides in the same map. A build without
this note reading a workspace written by one with it sees two `viewer` panes and keeps
both, which then both render the one Viewer: wrong but harmless, and it is what the
existing "drop what you cannot make" rule already yields.

The layouts and the per-view display preferences hang off the pane entry, so a workspace
carries them without needing a second structure.

### 4.2 The project

The bindings, the locks and the per-view state go in the block that already rides in the
`.lum`'s `ui_state` and in the per-path copy beside it (`ProjectReference::ui_state` /
`set_ui_state`, `SavedSession`). Keyed by view uuid, as §1.2 says. Every id is validated
against the document that actually loaded before it is used, exactly as the existing
restore does for comps and layers: a view bound to a deleted composition opens on the
empty state, and one bound to a composition that is still there opens on it.

### 4.3 What an old project does

Opens with one view, bound to what `active_comp` says, unlocked, taking the composition's
stored look, resolution, region, guides and overlays. Indistinguishable from today. The
seeding rule of §1.3 is what makes that true, and it has a test of its own in MV3.

### 4.4 The four spec amendments

Each lands in the package that reaches it, in the same commit as the code, per the rule
that a change reversing a rule edits the spec that carries it.

1. **07 §1.5's table**: magnification and channel view move from "per comp" to "per view",
   and the row gains the view bindings and the locks it already implies. (MV3, MV5)
2. **07 §2.2 items 12 and 13**: exposure and tone map persist per view, not per comp; item
   7's region of interest likewise. (MV5)
3. **07 §2.6**: the padlock is per view rather than per Viewer tab, and §1.5's resolution
   order for an opened item is written down there. (MV3)
4. **01-GLOSSARY §7**: a **View** row, distinguishing it from the Viewer panel and from
   the OCIO view. (MV5)

07 §2's opening paragraph already promises additional Viewers placeable anywhere, so no
amendment is owed there; it is a promise this note makes good on.

## 5. Keymap, menus and strings

Every action goes through `crates/lumit-keymap`, gets its English description there, and
gets its entry in `flutter_ui/lib/l10n/engine_labels.dart` in the same commit, or
`engine_labels_test.dart` fails. New actions, all in the `Viewer` context except the two
that open a Viewer:

| Action | What it does |
|---|---|
| `viewer.new` | Open another Viewer panel |
| `viewer.view.next` | Front the next view |
| `viewer.view.prev` | Front the previous view |
| `viewer.lock.toggle` | Lock or unlock the active view |
| `viewer.layout.one` | One view |
| `viewer.layout.two` | Two views |
| `viewer.layout.four` | Four views |
| `viewer.layout.orientation` | Swap a two-up between across and down |
| `viewer.cinema` | Fill the window with the active view |
| `viewer.compare` | Turn the compare on and off |
| `viewer.preview.always` | Make the active view the one that plays |

Default chords are not chosen here: 07 §15 is the inventory and the package that adds an
action adds its row there, checked for clashes by the keymap's own resolution. Every
user-facing string goes through `app_en.arb` with an `@key` description and is read as
`l10n.key`. The other `app_*.arb` files are never hand-edited. New keys are listed in the
pull request draft.

## 6. Real operating-system windows: blocked, and the phase

Re-checked against the installed Flutter on 2026-09-09, which is the check
docs/impl/multi-window.md §1 asks for before anything that needs real windows is planned:

```
Flutter 3.47.1 - channel stable - 2026-08-19
Tools - Dart 3.13.1
```

That is the exact version multi-window.md §1 pins as the blocker, and its finding stands
unchanged: in 3.47.1 `windowingFeature` in `packages/flutter_tools/lib/src/features.dart`
declares `master: FeatureChannelSetting(available: true)` and no stable setting at all, so
`flutter config --enable-windowing` cannot turn it on here. The API is `@internal` with a
documented promise to break in patch versions.

**So item 14 of the commission is not built and no work package below carries it.** The
phase, from multi-window.md §6, is where this note stops:

- **Now**: nothing. Views live inside the single window. Everything in §1 to §5 is
  reachable without the windowing API and none of it takes a dependency on it.
- **The cheap spike, unchanged**: multi-window.md §6 step 2, on a throwaway local branch,
  answering §4's open question of whether a second window's compositor can handle the
  engine's external GPU-surface texture at all. Until that is answered, "a view on a
  second monitor" is a promise nobody can keep.
- **When windowing reaches stable un-flagged**: a Viewer panel is already the unit that
  would move to a `SatelliteWindow`, and a view id already survives the move, because
  neither is tied to the main window's widget tree by anything but the dock. That is the
  whole of the preparation this note does for it, deliberately: multi-window.md §5 is
  explicit that no preparatory abstraction layer is wanted now.

Nothing in the packages below may take a main-channel dependency.

## 7. The traps

1. **The pool collision is silent.** Two views on same-sized comps overwrite each other,
   both pictures flicker between two compositions, and every line of code involved looks
   correct. §2.2. Test it first.
2. **Handles must be stable.** The current pool exists because re-creating a shared
   texture per frame piles up registrations and kills the compositor with "Binding D3D
   surface failed". The one-entry-per-view floor is not an optimisation; it is what stops
   the fix from re-creating that bug from the other side.
3. **`selectedComp` is 31 reads over 20 files.** Making it a getter over the active view
   is the whole of the change; hunting the call sites individually is how half of them end
   up reading a different view from the other half.
4. **The frame-name memo is keyed by revision, comp, frame and quality.** Per view looks
   make that insufficient. It gains the look identity or it is emptied on a look switch,
   which is the rule the prefix latch already follows.
5. **The renderer holds one look.** Applying a view's look before its render is required,
   and skipping the four setters when it has not changed is what stops a two-up costing
   two look switches per frame for nothing.
6. **A view must not ask for a picture it does not need.** Rule 1 of §2.4. Without it,
   four views on one document revision are four renders per edit on a serial worker.
7. **Zero bridge calls in rebuild paths.** Four views multiply any per-frame ask by four
   and the budget test is the thing that will notice.
8. **A lock is a reference to project content.** It cannot live in a workspace file, which
   is 07 §1.5's founding lesson and the reason After Effects drops such locks.
9. **The scratch composition of §3.6 must never reach the document.** A clone, like a drag
   preview: no commit, no journal entry, no undo step, nothing in the Project panel.
10. **Views are not panels.** A view has no tab and is not in the dock tree. Putting one
    there is how the dock's invariants get broken a second time.

## 8. Work packages

One package per pull request, each landing with its tests, in this order. The first three
are the ones that make anything usable; the rest can be reordered among themselves.

### MV1: The pool collision and the view id, engine only

The view id on the three requests and on the published frame (§2.1); the shared target
pool keyed by (view, width, height) with the one-per-view floor and the byte ceiling
(§2.2); the ceiling measured and this note's §2.2 updated with the number. One view still
exists in the interface, so nothing visible changes.

Tests:
- **Two views, one size, two handles.** Present two different comps of the same dimensions
  from two view ids; assert the handles differ and that reading back each shared texture
  gives that view's own pixels. **Fails without the fix**, which is what makes this the
  regression test the fix owes.
- **A view keeps its handle.** Present the same view at the same size ten times; assert
  one handle throughout and one registration.
- **The floor holds under pressure.** Four views, sizes enough to breach the ceiling;
  assert every live view still has its current-size entry and that the evictions came from
  the spares.
- **The measurement.** Four views at 4K, `settle` then `gpu_allocator_bytes` and
  `gpu_live_objects`, twice a batch apart, asserting the second reading does not grow.
  The number goes in this note.
- **Device loss.** A lost device rebuilds and every view's next frame gets a fresh handle
  (extend `a_lost_device_is_rebuilt_and_the_worker_draws_again`).

### MV2: The dock learns pane instances

`PaneId`, the serialisation with instance omitted at 0, `movePanel` / `_removePanel` /
`_tileOf` / `activatePanelTab` / `panelsIn` on pane ids, `activePanel` as a `PaneId?`,
"New Viewer" in the Window menu. Every Viewer still shows the active composition.

Tests:
- An old workspace file (no instance keys) loads and re-serialises byte-identical.
- Two Viewer panes survive add, drag to each of the five drop zones, and close.
- `simplify` keeps both, and closing one leaves the other.
- `panel_width_sweep_test` passes with two Viewers present.
- The Window menu's tick list is unchanged for every singleton panel.

### MV3: A view is bound to an item, and the lock

The view id and its uuid; the per-view controller map and frame routing by view id; the
binding and the lock in the project block; the padlock in the view chrome; §1.5's
resolution order for an opened item. Amendments 1 (partial) and 3.

Tests:
- Two views on two compositions both paint, and each paints its own.
- Opening a composition does not move a locked view, and lands by §1.5's order.
- Every view locked: opening a composition opens a new view.
- A view bound to a deleted composition opens on the empty state with its lock intact.
- An old project opens with one view, bound and looking exactly as it did.
- Round trip: bindings and locks survive save, close and open.

### MV4: The active view, and what follows it

`selectedComp` becomes a getter over the active view; fronting on click and by keymap; the
two exceptions of §1.4; the Timeline, Effect controls, Graph, Node, Mixer, Audio and the
menus following.

Tests:
- Fronting a view moves the Timeline, the Mixer and the Audio panel to its composition.
- Fronting a footage view leaves all three where they were.
- The rebuild budget test still counts zero bridge calls in build with two views up.
- Fronting a view that is already active rebuilds nothing (the rule
  `activatePanelTab` already keeps).

### MV5: View layouts, per-view state, and the share switch

One, two and four views with both orientations; the state split of §1.3 with the seeding
rule; the share view options switch. Amendments 1 (rest), 2 and 4.

Tests:
- Two views on one composition hold different magnifications, channels and exposures.
- Preview resolution and guides are shared by both, being per comp.
- The share switch on makes the second view follow the first; turning it off leaves each
  holding what it was showing.
- Growing a layout mints views bound to the active view's item; shrinking drops their
  project entries and nothing else.
- Layout and per-view display state round-trip through the workspace file.

### MV6: Scheduling across views

The four rules of §2.4: ask-only-when-moved, newest-per-view draining, active-view
priority, per-view epochs. Playback in one view and what the others show (§2.5). The
`ponytail:` comment naming the remaining ceiling.

Tests:
- Four views, one document edit: exactly the views whose inputs moved ask.
- Two views both asking: neither starves, and the active view's frame arrives first.
- A scrub in one view does not cancel the other's in-flight frame (epoch isolation).
- Playing in one view publishes frames for that view only, and the others' last pictures
  are unchanged at the end.
- Stopping refreshes the stale views once each, not once per view per turn.
- The idle fill anchors on the playing view, or the active view when nothing plays.

### MV7: The footage view and the layer view

The scratch-composition render of §3.6, both modes, the layer view's masks and anchor
point, the footage view's source-time strip, double-click routing through §1.5.

Tests:
- Double-clicking footage opens a footage view and paints it.
- Nothing enters the document: assert the revision, the undo depth and the Project panel's
  item list are all unchanged across a hundred footage-view renders.
- The scratch composition's id is stable, so the second render of the same frame is a
  cache hit.
- Double-clicking a layer opens its source before transform, with its masks drawn.
- A footage item that has gone away shows the empty state rather than faulting.

### MV8: Compare: wipe and split

The divider, both modes, the alignment rule of §3.4, the chrome that says which view is
which.

Tests:
- The divider drags and both clip rects follow it.
- Nothing crosses the bridge while the divider is dragged (budget).
- Two differently-sized compositions align on the active view's rectangle and the chrome
  says so.
- Turning the compare off leaves both views exactly as they were.

### MV9: Cinema, and the panel maximise the keymap already promises

`panel.maximise` implemented for panes; the cinema chord for the active view; Escape out
of both, in the §14.1 ladder's position.

Tests:
- Backtick maximises the pane under the pointer and restores it.
- Cinema fills the window with the active view, bars kept.
- Escape leaves cinema before it clears a selection (ladder order).
- The layout is unchanged after both, including split shares.

### MV10: Keymap, menus and strings

The eleven actions of §5 in `lumit-keymap` with their descriptions, their
`engine_labels.dart` entries, their 07 §15 rows, the Window and View menu rows, and the
`app_en.arb` keys.

Tests:
- `engine_labels_test.dart` passes (it fails without the entries, which is the point).
- No chord clashes, by the keymap's own resolution test.
- Every new string is read as `l10n.key` and none is inline (the existing lint).

### MV11: 3D views: active camera, front, top, custom, and the wireframe mode

The one package with real engine work beyond MV1. The render path takes the composition's
active camera from the document (`build.rs`, `draw.rs`); a view showing front, top or a
custom angle needs a **view camera override** threaded from the request into the draw
build, orthographic for front and top. The full wireframe display mode of 07 §2.2 item 5
(outlines only, no raster) belongs here because it is the same override plus a draw mode,
and it is what makes a four-up of a 3D composition usable at all.

Gated last, deliberately: everything above is usable without it, and this is the only
piece that reaches into the compositor.

Tests:
- A composition with one 3D layer renders front, top and active camera, and the three
  differ in the ways the geometry says they must.
- An override is per view: two views, two angles, both correct in one frame.
- The override is in the frame name, so the two views cache separately.
- No override behaves bit-for-bit as today (the determinism rule).
- Wireframe mode draws outlines and no raster, and costs measurably less on a heavy
  composition.

### Not a package: real operating-system windows

Blocked upstream. §6.

## 9. The invariants, across every package

The five things any package may not break, checked by the tests above but stated here
because they are the correctness argument:

1. **One view's picture is only ever that view's pixels.** (MV1)
2. **A workspace written before this note opens unchanged, and a project written before it
   opens looking identical.** (MV2, MV3)
3. **A lock never lets an item be opened into a view that is locked, and never leaves an
   opened item nowhere.** (MV3)
4. **No bridge call happens in a rebuild path, however many views are up.** (MV4, MV8)
5. **A view that has nothing new to show asks for nothing.** (MV6)

## Feeds

07 (§1.4, §1.5, §2 whole, §2.6, §15), 17 (the frame paths and the Viewer texture
transport), 05 (§2's thread roles, §5's one device), 06 (§5.2's key, §5.5's fill), 13
(§3's governor, §7.0.1's memory report), 01 (§7's panel vocabulary),
[multi-window.md](multi-window.md) (§6's phase),
[playback-scheduler.md](playback-scheduler.md) (§1's epochs, §5's pipeline),
[ui-performance.md](ui-performance.md) (§4.5's per-revision rule).
