# Build the audio workspace

A tutorial. You type the code; it explains every piece of syntax the first time it
appears. By the end you will have changed the Flutter frontend, added a function to
the Rust engine, run the generator that joins the two, and written tests that fail
if any of it breaks.

Nothing here is in the repository. That is the point: the branch does not carry the
answer, so following the steps is the only way to end up with the feature. If you
would rather read finished code, every file this touches is already open to you —
this page is for building something in it.

**What you already know is the half that matters.** You know what a workspace is,
what a waveform lane is for, and why the Effect controls panel wants to be beside
the Timeline when you are working on sound. None of that is being taught. What is
being taught is where those ideas live in the code, and how to move them.

**Two hours, roughly**, if you read the asides. Less if you only type.

---

## What you are building

Three things, in order:

1. **The Audio arrangement, made real.** Lumit ships five workspaces — Edit,
   Effects, Colour, Audio, Retiming. Audio is currently a stand-in: it makes the
   Timeline taller and leaves everything else where Edit had it. You will give it
   the shape the work actually wants — a tall Timeline for the waveform lanes,
   Effect controls in a column of its own, Scopes to the right.
2. **A number from the engine.** A new function in Rust that answers "how many
   pieces of footage in this project carry sound", surfaced as a quiet line at the
   top of the Project panel. Small on purpose: the value of it is the round trip,
   not the number.
3. **Tests for both**, including one that drives the real engine.

## Before you start

Have the environment ready and know how to run things. That is
[09-DOING-IT-YOURSELF.md](09-DOING-IT-YOURSELF.md), and this page assumes it:

```powershell
. .\scripts\win-dev-env.ps1
```

Once per terminal, before anything that compiles Rust. Then, so you can see what
you are changing:

```powershell
cd flutter_ui
flutter run -d windows
```

Leave it running. While it runs, `r` reloads the Dart side in about a second, `R`
restarts it, `q` quits. Rust changes need `q` and a fresh `flutter run`, because the
compiled engine library is loaded once at startup.

Work on a branch, not on `main`:

```powershell
git switch -c learn-audio-workspace
```

---

# Part 1 — the arrangement

## Where a workspace lives

A workspace in Lumit is not a screen or a mode. It is a **tree**: how the window is
divided, and which panel sits in each division. One file holds the model and all
five shipped arrangements:

`flutter_ui/lib/state/dock.dart`

Open it. The first thing in it, after the comment, is this:

```dart
enum Panel {
  project,
  viewer,
  timeline,
  effectControls,
  effectsAndPresets,
  scopes,
  debug,
  hierarchy,
  easing;
```

**An `enum` is a closed list of possibilities.** `Panel` says: there are nine
dockable panels, these are their names, and there is no tenth. Anywhere the code
handles a `Panel`, the compiler can check that every case is covered — which is why
adding a panel to this list makes the build fail everywhere the new one has not been
thought about. That failure is the feature.

Below it, three small classes describe the tree:

```dart
class DockPane extends DockNode {
  final Panel panel;
  DockPane(this.panel);
```

**A class is a kind of thing, with the data it carries.** `DockPane` is "one panel,
sitting somewhere". It carries exactly one piece of data — which panel.

**`final` means: set once, when the object is made, and never changed after.** Not a
constant across the program — a different `DockPane` can name a different panel —
but *this* pane's `panel` is fixed for as long as it exists. Almost everything in
the Flutter side is `final`, because a thing that cannot change is a thing that
cannot change behind your back. When the arrangement changes, the code does not edit
the tree; it builds a new one.

`DockPane(this.panel);` is the **constructor** — how you make one. The
`this.panel` shorthand means "take one argument and store it in the field of that
name". So `DockPane(Panel.viewer)` is a pane holding the Viewer.

Two more nodes complete the model:

- `DockTabs([...])` — panels stacked behind one another with a tab bar. Only one is
  visible; the rest are one click away.
- `DockSplit(axis, children, shares)` — children laid out side by side
  (`DockAxis.horizontal`) or stacked (`DockAxis.vertical`), each getting a share of
  the space. `[0.22, 0.58, 0.20]` means 22%, 58%, 20% — three children, three
  shares, and the class asserts those two lists are the same length.

That is the whole model. A workspace is a `DockSplit` with other nodes inside it.

## Reading the function you are about to change

Find `presetLayout`. It starts:

```dart
DockSplit presetLayout(WorkspacePreset preset) => switch (preset) {
      // Edit is the default arrangement.
      WorkspacePreset.edit => defaultLayout(),
```

Four pieces of syntax here, and you will meet all four again on every page of this
codebase.

**`DockSplit presetLayout(WorkspacePreset preset)`** — a function named
`presetLayout` that takes one argument called `preset`, of type `WorkspacePreset`,
and gives back a `DockSplit`. Types come first in Dart, before the name.

**`=>`** is "this function is one expression, and here it is". The longer form with
`{ return ...; }` means the same thing; the arrow is used when the body is a single
value, which keeps it readable.

**`switch (preset) { ... }`** used as an *expression*, not a statement. Each arm is
`value => result`, and the whole switch evaluates to whichever arm matched. Because
`WorkspacePreset` is an enum, the compiler knows the full list of arms — leave one
out and it refuses to compile rather than returning nothing at run time.

**A list literal** is square brackets: `[a, b, c]`. The arrangements are built out
of nested list literals, and reading one is exactly like reading the window: the
outer list is top-to-bottom, an inner horizontal split is left-to-right.

Now find the Audio arm and read it as a picture of the screen:

```dart
      WorkspacePreset.audio => DockSplit(
          DockAxis.vertical,
          [
            DockSplit(
              DockAxis.horizontal,
              [
                DockTabs([
                  DockPane(Panel.project),
                  DockPane(Panel.effectControls),
                  DockPane(Panel.effectsAndPresets),
                  DockPane(Panel.hierarchy),
                ]),
                DockPane(Panel.viewer),
                DockTabs([
                  DockPane(Panel.scopes),
                  DockPane(Panel.debug),
                ]),
              ],
              [0.24, 0.56, 0.20],
            ),
            DockPane(Panel.timeline),
          ],
          [0.55, 0.45],
        ),
```

Top level: vertical, two children — an upper band and the Timeline, 55% and 45%. The
upper band: horizontal, three children — a left tab group with four panels stacked
behind Project, the Viewer, and a right tab group of Scopes and Debug.

Read what it does to the work. The Timeline is tall, which is right: the waveform
lanes are there. But **Effect controls is the third tab of the left group**, behind
Project, Effects & presets and Hierarchy. Volume lives in Effect controls. Every
audio effect's parameters live in Effect controls. So the arrangement named after
sound puts the panel you edit sound in behind two tabs of other things.

That is the change.

## The change

Replace the Audio arm with this. The comment above it changes too — a comment that
explains an arrangement you have just rearranged is a comment that lies.

```dart
      // The Timeline taller than Edit with its waveform lanes; the Viewer
      // reduced. Effect controls takes a column of its own rather than tabbing
      // behind Project: a layer's Volume and its audio effects are edited
      // there, and a panel behind a tab is a panel you have to keep fetching.
      // The Audio panel joins this arrangement when it is built.
      WorkspacePreset.audio => DockSplit(
          DockAxis.vertical,
          [
            DockSplit(
              DockAxis.horizontal,
              [
                DockTabs([
                  DockPane(Panel.project),
                  DockPane(Panel.effectsAndPresets),
                  DockPane(Panel.hierarchy),
                ]),
                DockPane(Panel.effectControls),
                DockPane(Panel.viewer),
                DockTabs([
                  DockPane(Panel.scopes),
                  DockPane(Panel.debug),
                ]),
              ],
              [0.18, 0.18, 0.44, 0.20],
            ),
            DockPane(Panel.timeline),
          ],
          [0.55, 0.45],
        ),
```

Three edits, in plain terms: `DockPane(Panel.effectControls)` moved out of the tab
group and became a child of the split in its own right; the upper band therefore has
four children instead of three; and the shares gained a fourth number.

## Why this is the right place

Because there is exactly one description of what the Audio workspace *is*, and this
is it. The widgets that draw panels read this tree; the toolbar strip and
Window ▸ Workspace both call `applyWorkspacePreset`, which calls `presetLayout`; the
saved layout in your settings file is this tree serialised. Change it here and every
one of those follows. There is nowhere else the change could go without being a
second, competing answer to the same question.

## What to notice

**The shares must match the children.** `DockSplit` asserts it —
`assert(children.length == shares.length)` in its constructor — so four children with
three shares fails immediately and loudly rather than drawing something strange. When
you add a child, add a share.

**The numbers do not have to add to 1.** They are weights, normalised when used. It
is a convention here that they read as percentages, and conventions like that are
worth keeping even when nothing enforces them.

**A pane alone renders bare.** `DockPane(Panel.effectControls)` as a direct child of
a split has no tab bar over it. That is deliberate (decision K-086): a tab bar over a
single panel is a control that can do nothing.

**Nothing else needed changing.** No registration, no list of "panels the Audio
workspace uses", no string. That is a sign you found the right place.

## Think in Flutter: the layout is data, drawn by something else

The instinct from most UI toolkits is that a layout is code you run: create panel,
set parent, set size. Here the arrangement is a value — a tree you could print,
compare, save to a JSON file (it is: `toJson` is right there) and read back. Nothing
in `dock.dart` draws a pixel.

That is the whole shape of Flutter, and of this codebase in particular. You describe
what should be on screen; the framework compares your description with the last one
and changes what differs. So building a description is cheap and normal — you will
see whole widget trees created every frame — and *mutating* something the framework
is holding is what causes trouble.

The house version of the rule: **Flutter is a thin view.** Panels display values and
forward calls. Anything that decides something belongs in Rust. When you are unsure
whether a piece of logic belongs on the Dart side, the answer is almost always no.

## Seeing it

Press `r` in the terminal running the app, then Window ▸ Workspace ▸ Audio — or the
Audio button in the strip along the top. Effect controls has its own column beside
the Viewer, the Timeline is tall, and Scopes is one click away on the right.

If the workspace does not change, you are looking at your *saved* layout: applying a
preset is what installs the factory arrangement, so click Audio again after the
reload.

---

# Part 2 — a number from the engine

The arrangement is arranged. Now something that crosses the seam.

## What you are adding, and why it is worth doing

A line at the top of the Project panel: **"3 items with sound"**. It appears when the
project holds footage that carries an audio stream, and stays away when it does not.

As a feature it is small. As an exercise it is the whole round trip: a fact only the
engine can know (it takes opening the file with FFmpeg to find out), asked for from
Dart, answered in Rust, generated code in between, and a test at each end.

## Where the seam is

`crates/lumit-bridge/src/api/` is the entire list of things Flutter is allowed to
call. One file per subject: `audio.rs`, `layer.rs`, `project.rs`, and so on. A
generator reads that folder and writes both halves of the join — Rust glue, and Dart
classes with the same method names.

Two rules about it, both worth more than they look:

- **Never edit a generated file.** Anything under `flutter_ui/lib/src/rust/` and
  `crates/lumit-bridge/src/frb_generated.rs` is written by the generator. CI
  regenerates and compares. A hand edit is undone by the next run.
- **The API folder never panics.** More on that below, when you write the code.

Your function is about sound, so it goes in `crates/lumit-bridge/src/api/audio.rs`.

## Reading Rust before writing it

Open `audio.rs` and look at what is already there.

```rust
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeAudioClock {
    pub seconds: f64,
    pub playing: bool,
    pub loaded: bool,
}
```

**A `struct` is a record: named fields, grouped under a type name.** No methods
inside — Rust keeps the data and the operations apart. `pub` means visible outside
this file; `f64` is a 64-bit floating-point number; `bool` is true or false.

**The lines in `#[...]` are attributes** — notes attached to the item below them,
read by the compiler or by a tool.

`#[derive(Debug, Clone, Copy, PartialEq)]` asks the compiler to write four pieces of
boilerplate for you: how to print it for debugging, how to copy it, that copying is
cheap enough to happen implicitly, and how to compare two of them.

`#[frb(...)]` is the bridge generator's attribute. It is how you tell the generator
what to do with an item. `#[frb(sync)]` means "Dart may call this and get the answer
immediately"; without it, the Dart side gets a `Future` — a promise of an answer
later. `#[frb(ignore)]` means "this is internal; do not offer it to Dart at all".

Now the other shape in the file:

```rust
impl CompositionReference {
    #[frb(sync)]
    pub fn audio_prepare(&self) -> Result<(), BridgeError> {
```

**`impl` means "here are functions belonging to this type".** The struct is declared
in one place; its methods live in `impl` blocks, which may be in a different file
entirely — `CompositionReference` is declared in `composition.rs`, and this block
adds a method to it from `audio.rs`. That is normal and useful: it lets the audio
functions live with the other audio functions.

**`&self`** is the value the method is called on, borrowed rather than handed over.
Borrowing is Rust's central idea: you can lend a value out for a while without giving
up ownership of it. `&self` is a read-only loan.

**`Result` is how Rust reports failure.** There are no exceptions. A function that
might fail returns `Result<T, E>`: either `Ok(value)` or `Err(problem)`, and the
caller cannot use the value without dealing with the possibility of the error, because
the two are the same type. `Result<(), BridgeError>` returns nothing on success —
`()` is the empty type, pronounced "unit" — and a `BridgeError` on failure.

**The engine never panics.** A panic is Rust's abort — the equivalent of an unhandled
exception, and in a library loaded into another process it can take the whole
application down with it. So this workspace forbids the functions that panic:
`unwrap`, `expect`, `panic!`, `todo!` and `unsafe` are denied by the linter, and CI
runs a plain text search over `crates/lumit-bridge/src/api/**` for good measure. In
this folder, an error is a value you return, always.

One more piece of syntax, because you are about to write it:

```rust
let Some(src) = resolve(...) else {
    continue;
};
```

**A `let ... else`** says: match this pattern, bind the names, and if it does not
match, run the `else` block — which must leave the current scope, by `continue`,
`break` or `return`. It is the Rust idiom for "get this or give up here", and it
saves a level of indentation over an `if let`.

## The change, in Rust

At the bottom of `crates/lumit-bridge/src/api/audio.rs`, add:

```rust
impl crate::api::project::ProjectReference {
    /// How many pieces of footage in this project carry sound.
    ///
    /// What the Project panel reports at a glance while the Audio workspace is
    /// up: not every clip in an edit has a soundtrack, and knowing how many do
    /// is the difference between "this comp is silent" and "the mix is not
    /// loaded yet".
    ///
    /// The answer is the container's own — a file with an audio stream — so it
    /// costs a probe per footage item and is deliberately **not**
    /// `#[frb(sync)]`. Probes are cached by path and modification stamp, so the
    /// panel pays once per item and nothing after that. A file that cannot be
    /// resolved is not counted: a missing file makes no sound.
    pub fn audio_item_count(&self) -> Result<u32, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = state.store.snapshot();

        let mut count = 0;
        for item in &snapshot.items {
            let lumit_core::model::ProjectItem::Footage(footage) = item else {
                continue;
            };
            #[cfg(feature = "media")]
            {
                let Some(src) =
                    crate::api::footage::FootageReference::resolve_source(&state, footage)
                else {
                    continue;
                };
                if crate::probe::ensure_probed(&src).is_some_and(|p| p.audio.is_some()) {
                    count += 1;
                }
            }
            // Without a decoder nothing can be probed, so nothing claims to
            // carry sound — the same answer `LayerReference::has_audio` gives.
            #[cfg(not(feature = "media"))]
            let _ = footage;
        }
        Ok(count)
    }
}
```

Line by line, in the order the questions come up.

**`/// ...` is documentation**, attached to the item below it, and it is not
decoration: `cargo doc` publishes it, and in this repo a bridge function without one
is an unfinished bridge function. Write what the caller needs to know, including what
it costs.

**`let state = self.state()?;`** — `state()` finds this project in the engine's
registry and hands back a shared handle. It returns a `Result`, and the `?` is the
whole of Rust's error plumbing: if it is `Ok`, unwrap it and carry on; if it is
`Err`, return that error from *this* function immediately. One character, and no
error goes unchecked.

**`let state = state.read()...`** — the handle wraps the project in a lock. `read()`
takes a shared read lock: many readers at once, no writer while they hold it. The
name `state` is reused deliberately (Rust calls this shadowing) because the old
binding is of no further use.

`.map_err(|_| BridgeError::ReadFailed)?` converts whatever the lock failed with into
this crate's own error type. `|_| ...` is a closure — an anonymous function — whose
argument is ignored, written `_`.

**`let snapshot = state.store.snapshot();`** — the document as it is *right now*, as
an immutable copy. Lumit never edits a document in place: an edit publishes a whole
new one. So a snapshot cannot change under your loop, and holding one blocks nobody.

**`for item in &snapshot.items`** — borrow the list and walk it. The `&` matters:
without it you would be taking ownership of the list out of the snapshot, which the
compiler will not allow, and would not want.

**The `let ... else` line** skips everything that is not footage — comps, folders,
solids. A folder has no sound.

**`#[cfg(feature = "media")]`** is conditional compilation. A *feature* is an
optional part of the build; `media` brings in FFmpeg. On a build without it the
enclosed block does not exist at all, which is why the `#[cfg(not(...))]` arm below
is there to say what happens instead — nothing carries sound, because nothing can be
opened to find out. `let _ = footage;` uses the variable so that build has no
"unused" warning, and warnings are errors here.

**`ensure_probed`** opens the file once and remembers the answer, keyed by path and
modification time. `.is_some_and(|p| p.audio.is_some())` reads as: if there is a
probe, and that probe found an audio stream. `Option` is Rust's "there might be
nothing here" — the language has no null.

**`Ok(count)`** — success, carrying the number. Note what is *not* here: no
`#[frb(sync)]`. Probing files is slow, and the Dart UI thread must never wait on
slow. Leaving the attribute off is what makes the Dart method return a `Future`.

## Check it compiles

```powershell
cargo check -p lumit_bridge
```

The folder is `crates/lumit-bridge` with a hyphen; the package is `lumit_bridge`
with an underscore, and cargo wants the package name.

**Success looks like** `Finished ... in 4.03s`.

## The codegen cycle

Rust that Flutter can see is not enough on its own — the generated Dart has to be
regenerated, and the compiled library rebuilt.

```powershell
.\scripts\codegen.ps1
```

That runs four commands in the order that matters, and prints each one as it goes:

```powershell
cd flutter_ui
flutter pub get
flutter_rust_bridge_codegen generate
cd ..
cargo build -p lumit_bridge
```

**Success looks like** `Done!` from the generator, then `git status` showing changes
in `flutter_ui/lib/src/rust/api/project.dart` and
`crates/lumit-bridge/src/frb_generated.rs`. Look at the first of those:

```dart
  Future<int> audioItemCount() =>
```

Your `audio_item_count` became `audioItemCount` — Rust names things with
underscores, Dart with capitals, and the generator translates. It landed on the
`ProjectReference` class, because that is the type you wrote the `impl` for, even
though you wrote it in `audio.rs`. `Result<u32, BridgeError>` became `Future<int>`:
the error path becomes a thrown exception on the Dart side, and the future is because
you left `#[frb(sync)]` off.

**The rebuild in the last step is the one everybody forgets.** The Dart tests do not
build the engine — they *load* `target/debug/lumit_bridge.dll` and drive the real
thing. Both sides compare a content hash at startup, so a library that no longer
matches refuses to start, and what you see is every test failing with "found 0
widgets". It reads like a broken interface. It means the library is stale. Rebuild
it.

## The string

Every user-facing word in Lumit lives in one file, and is read through a generated
accessor. Never write text inside a widget.

In `flutter_ui/lib/l10n/app_en.arb`, keeping the file's alphabetical order — the new
key goes just above `"projectInfoAudio"`:

```json
  "projectHasSoundCount": "{count, plural, =1{1 item with sound} other{{count} items with sound}}",
  "@projectHasSoundCount": {
    "description": "How many pieces of footage in the project carry an audio stream. A quiet header line at the top of the Project panel, shown only when there is at least one.",
    "placeholders": {
      "count": {
        "type": "int"
      }
    }
  },
```

Two entries, always: the string, and an `@key` entry describing it. The description
is for a translator who sees the phrase and not the screen, so it says where the
phrase appears and what the number means.

The `{count, plural, ...}` form is ICU message syntax. English needs two cases;
other languages need more, and the translator supplies them. This is why "1 item" is
never built by gluing a number onto a word in code.

Regenerate the Dart accessors:

```powershell
cd flutter_ui
flutter pub get
```

That writes `l10n.projectHasSoundCount(count)` into the generated localisations, and
`flutter analyze` will tell you at once if you mistyped the key.

**The other `app_*.arb` files are not yours to touch.** They come back from Crowdin.
Adding an English key leaves the other languages short, which is expected — a missing
translation falls back to English — but the commit message must say so and name the
new key, or the upload gets forgotten and the string ships English everywhere.

## The Dart that shows it

`flutter_ui/lib/panels/project_panel_frb.dart`. Four small edits.

**One — the import.** With the other `src/rust/api` imports at the top:

```dart
import 'package:lumit_flutter/src/rust/api/project.dart';
```

That is where the generator put `ProjectReference`.

**Two — somewhere to keep the answer.** Beside the `_missing` map, about two thirds
of the way down the state class:

```dart
  /// How many pieces of footage in the project carry sound, or null until the
  /// engine has answered.
  ///
  /// Cached for the same reason [_missing] is: the answer costs a probe of
  /// every footage file, and the panel rebuilds far more often than the item
  /// list changes. Dropped on a document change, and only then.
  int? _soundItems;

  /// Whether the count above has been asked for since the last document
  /// change. Separate from the count itself, so a rebuild between the question
  /// and the answer does not ask again.
  bool _soundAsked = false;
```

**`int?` is a nullable integer** — an `int`, or null. The question mark is the type
saying so, and Dart will not let you use it as a number until you have dealt with the
null. Here null means "not asked yet", which is genuinely different from zero.

A name beginning with an underscore is **private to the file**. That is the whole of
Dart's privacy model: no `private` keyword, just the underscore.

**Three — the asking.** Next to `_documentChanged`, and one line inside it:

```dart
  void _documentChanged() {
    setState(() {
      _epoch++;
      _missing.clear();
      _mediaInfo.clear();
      _soundItems = null;
      _soundAsked = false;
      _dropThumbs();
    });
  }

  /// Ask the engine how much of this project carries sound, once per document
  /// change, off the build.
  void _refreshSoundItems(ProjectReference project) {
    if (_soundAsked) return;
    // Claim the question first, so a rebuild mid-probe does not ask twice.
    _soundAsked = true;
    project.audioItemCount().then((count) {
      if (!mounted) return;
      setState(() => _soundItems = count);
      // A count can outlive its document, the same way a status probe can:
      // opening a project invalidates every reference held from the outgoing
      // one. Nothing carries sound in a document that is gone.
    }).catchError((_) {});
  }
```

**`setState` is how a panel says "I have changed; draw me again".** The framework
does not watch your fields. You change them inside `setState`, and it schedules a
rebuild. Changing a field without it is the classic Flutter bug: the value is right
and the screen is stale.

**`.then((count) { ... })`** is what you do with a `Future`. The call returns
immediately; the block runs when the engine answers, which may be several frames
later. `(count) { ... }` is a closure again — Dart's anonymous function.

**`if (!mounted) return;`** — by the time the answer arrives the panel may be gone,
closed with its workspace or replaced when a project was opened. `mounted` is false
then, and calling `setState` on a dead widget throws. Every asynchronous callback in
this codebase checks it. Make it a reflex.

**Claiming the question before asking it** is the pattern to take away. `build` can
run many times a second; `_soundAsked = true` on the way in means the second, third
and fortieth rebuild during a slow probe ask nothing. The panel's `_missing` cache
does exactly the same thing, with the comment "claim the slot first".

**Four — the asking, and the showing.** In `build`, right after `_refreshMissing`:

```dart
    _refreshMissing(roots);
    final project = state.project;
    if (project != null) _refreshSoundItems(project);
```

and in the `Column` that the method returns, between the info header and the missing
header:

```dart
        if ((_soundItems ?? 0) > 0) _SoundHeaderFrb(count: _soundItems!),
```

**`??` is "use the left, unless it is null, then the right".** `_soundItems ?? 0`
reads "the count, or zero while we do not know yet". **`!`** after a nullable value
is the opposite promise: "I know this is not null" — safe here only because the
condition on the same line has just established it.

**`if (...) widget,` inside a list literal** is Dart's collection-if: the element is
in the list when the condition holds, and simply not there when it does not. That is
how a widget is conditionally on screen — not by building it hidden.

Finally the widget itself, just above the `/// The header shown while the project has
missing footage` comment near the bottom of the file:

```dart
/// How much of the project carries sound, above the item list.
///
/// A statement, not a control: nothing is clickable here, which is why it is a
/// [StatelessWidget] holding one number rather than anything with state.
class _SoundHeaderFrb extends StatelessWidget {
  final int count;
  const _SoundHeaderFrb({required this.count});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      key: const ValueKey('project-sound-count'),
      height: 24,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 8),
      alignment: Alignment.centerLeft,
      child: Text(
        l10n.projectHasSoundCount(count),
        style: t.small.copyWith(color: t.textMuted),
      ),
    );
  }
}
```

**A widget's `build` method returns a description of what should be on screen.** It
is called by the framework, never by you, and it may be called at any time — so it
must be cheap, and it must not have side effects. Everything it needs comes from the
widget's own `final` fields and from `context`.

**`extends StatelessWidget`** means this widget has no memory: same inputs, same
picture. The panel around it is a `StatefulWidget` because it remembers things
between builds — the search text, the caches, your count. When something needs no
memory, saying so is worth doing.

**`{required this.count}`** is a named argument, and required. Named arguments are
the house style for widgets, because `_SoundHeaderFrb(count: 3)` says what the 3 is
and `_SoundHeaderFrb(3)` does not.

**`const`** on a constructor means the object can be built at compile time and reused
rather than allocated again. The linter will ask for it wherever it is possible.

**Every colour comes from the theme.** `t.surface1`, `t.textMuted`, `t.small` — a hex
literal in widget code is a defect and CI has a lint that finds them. There are
several themes and they must all work.

**`key: const ValueKey('project-sound-count')`** gives the widget a stable name.
Flutter uses keys to match widgets between builds; tests use them to find things
without depending on the words on screen, which change with the language.

Reload with `r`, import a clip with sound, and the line appears above the project
list.

## Think in Rust: errors are values, and the snapshot is why nothing waits

Two ideas from Part 2 generalise past this function.

**Errors are values, and the compiler counts them.** There is no exception flying
past your code. Every fallible call in `audio_item_count` is visibly fallible — the
`?` marks are the map of everything that can go wrong. When you read a Rust function
here, the `?`s tell you where the exits are, and the absence of `unwrap` tells you
nobody decided to stop caring. That is why the engine can promise not to take the
application down with it: not discipline, but a type that cannot be ignored.

**Immutability is a performance decision, not a purity one.** `store.snapshot()`
hands you a complete document that will never change, and it costs almost nothing
because an edit publishes a *new* document rather than modifying this one. Your loop
therefore holds no lock over its slow work, blocks no editing, and cannot see a
half-applied change. When you find yourself wanting to hold a lock while doing
something slow — decoding, probing, drawing — take a snapshot instead. The rule in
`docs/14-ENGINEERING-RULES.md` is blunter: no locks held across await, GPU work or
FFI.

---

# Part 3 — the tests

Near-full regression coverage is standing policy here: a feature lands with its
tests, a bug fix lands with the test that fails without the fix. It is not a chore
bolted on at the end — the tests you are about to write are how you find out that the
thing you just built actually does what you think.

## Running what is already there

Rust first:

```powershell
cargo test -p lumit_bridge
```

Then Dart — **one file at a time**:

```powershell
cd flutter_ui
flutter test test\dock_test.dart
flutter test test\frb\project_panel_frb_test.dart
```

**Never run `flutter test` with no file argument on this machine.** It starts a
process per file, in parallel, each of the `frb` ones loading the engine and a
graphics device. It has frozen the machine hard enough to need the power button. CI
runs the full suite on its own hardware, one at a time; locally, name what you
touched.

**Success looks like** `All tests passed!`, and for cargo, `test result: ok`.

If every frb test says "found 0 widgets", the library is stale. `cargo build -p
lumit_bridge` and run again.

## Adding one: the arrangement

`flutter_ui/test/dock_test.dart` is a plain unit test file — no widgets, no engine,
milliseconds to run. It already has a group for the Retiming preset. Add one for
Audio, just above it:

```dart
  group('the Audio preset', () {
    test('gives Effect controls a column of its own, beside the Viewer', () {
      final root = presetLayout(WorkspacePreset.audio);
      final upper = root.children[0] as DockSplit;
      final controls = upper.children[1];
      expect(controls, isA<DockPane>(),
          reason: 'a panel behind a tab is a panel you have to keep fetching');
      expect((controls as DockPane).panel, Panel.effectControls);
      expect(upper.shares.length, upper.children.length);
    });

    test('keeps the Timeline tall, because the waveform lanes live there', () {
      final root = presetLayout(WorkspacePreset.audio);
      expect(root.shares, [0.55, 0.45]);
      expect((root.children[1] as DockPane).panel, Panel.timeline);
    });

    test('holds the Scopes panel and no panel twice', () {
      final panels = panelsIn(presetLayout(WorkspacePreset.audio));
      expect(panels, contains(Panel.scopes));
      expect(panels.toSet().length, panels.length);
    });
  });
```

**`group` and `test`** take a name and a function — that is all a test is here.
**`expect(actual, matcher)`** is the assertion. The matcher can be a plain value
(equality), or something richer: `isA<DockPane>()` checks the type, `contains(...)`
looks inside a collection.

**`as DockSplit`** is a cast: the tree is typed as `DockNode`, and the test says "I
know this one is a split". If it is not, the test fails with a clear message, which
is exactly what you want it to do when someone rearranges the tree.

**`reason:`** is a message shown only on failure. Use it to say *why the rule
exists*, not what the line does. The failing developer in six months is the reader —
"a panel behind a tab is a panel you have to keep fetching" tells them whether they
are breaking a decision or fixing a mistake.

Notice what the three tests pin: the thing you changed (Effect controls is not
tabbed), the thing you deliberately kept (the tall Timeline, which the Retiming test
next door refers to), and the invariant that holds for every arrangement (no panel
twice). Together they are the difference between "the layout is what I typed" and
"the layout still means what I meant".

```powershell
flutter test test\dock_test.dart
```

## Adding one: the panel, against the real engine

The tests under `flutter_ui/test/frb/` are integration tests. They load the real
compiled engine and make real documents — there is nothing to fake, because the
generated classes call straight into the library.

In `flutter_ui/test/frb/project_panel_frb_test.dart`, after the first test:

```dart
    testWidgets('footage that carries no sound raises no sound header',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      await tester.pump();

      expect(
        find.byKey(const ValueKey('project-sound-count')),
        findsNothing,
        reason: 'a file that cannot be resolved makes no sound, so the header '
            'must not appear for every footage item on principle',
      );
    });

    testWidgets('the header counts the footage that carries sound',
        (tester) async {
      final p = freshProject();
      p.state.project!.importFootage(path: _probeableMediaFile('tone.wav'));

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await settleFrb(
        tester,
        until: () =>
            find.byKey(const ValueKey('project-sound-count')).evaluate().isNotEmpty,
      );

      expect(find.byKey(const ValueKey('project-sound-count')), findsOneWidget);
      expect(find.text('1 item with sound'), findsOneWidget);
    });
```

**`testWidgets` gives you a `tester`** that owns a fake screen. `pumpWidget` mounts a
widget tree on it; `pump()` advances one frame and draws.

**`_probeableMediaFile('tone.wav')`** is already in that file: it writes a real,
tiny, valid WAV — a RIFF header and a few bytes of samples — to a temporary folder.
FFmpeg opens it and finds an audio stream, so this is a genuine end-to-end check and
not a stub. The path in the first test, `C:/clips/shot.mov`, is deliberately a file
that does not exist.

**`settleFrb`** is the harness's answer to a hard problem. A widget test runs in fake
time, where futures do not complete on their own, but your count comes back across a
real FFI port on a real thread. `settleFrb` alternates: a real event-loop turn so the
answer can land, then a fake-time pump so the `setState` it queued gets drawn. The
`until:` argument lets it stop as soon as the thing you are waiting for appears
rather than always paying the maximum.

**`find.byKey`, `findsNothing`, `findsOneWidget`** — the finders search the mounted
tree. Finding by key is why `_SoundHeaderFrb` has one: the visible text changes with
the language, the key does not.

Take the two tests together. One says the number is honest about silence — it would
fail if the count had been written as "how many footage items are there". The other
says the whole chain works: Rust probed a real file, the future crossed the seam, the
panel called `setState`, the plural string formatted, and the header drew. If
anything in Part 2 breaks, one of these two goes red.

```powershell
flutter test test\frb\project_panel_frb_test.dart
```

## The gates before a commit

Everything CI will run, run locally first:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cd flutter_ui; flutter analyze
```

Or `.\scripts\check.ps1` for the lot, and `-Fix` to let the formatter rewrite rather
than complain. `-D warnings` means a warning ends the run: this workspace treats
"probably fine" as "not yet finished".

## What a real commit of this would owe

You have working code. A commit is more than working code, and the difference is
mostly about the next person:

- **The Crowdin note.** New key `projectHasSoundCount`, named in the commit message
  and in the pull request, because the other languages are now one string short and
  somebody has to upload it.
- **A word in [GUIDE.md](../GUIDE.md)** if the change introduces a concept, not just
  a value. "Workspaces are trees of splits and tabs" belongs there; "Audio's Effect
  controls moved" does not.
- **A decision entry** in `docs/02-DECISIONS.md` if the change reverses one that was
  logged. Rearranging a workspace is not decision-sized. Deciding that a workspace
  may add a panel the others do not have was — that is K-349, and the Retiming tests
  quote it.
- **The commit message in the house voice**: what changed and why it is right,
  British English, sentence case, calm, no exclamation marks and no emoji. Read
  `git log` for a page and the shape is obvious.

---

## Where to go next

You have now touched every layer this codebase has except the GPU. Follow whichever
part you want to understand properly:

- **[GUIDE.md §9, "The Flutter frontend, in plain terms"](../GUIDE.md)** — the panels,
  the shell, the theme and the strings, with no code background assumed. Start here
  if Part 1 was the interesting half.
- **[GUIDE.md §2, "Rust in ten minutes, Lumit edition"](../GUIDE.md)**, then **§3,
  "Threads, in editing terms"** — ownership, snapshots and why the engine is built
  the way it is, in editing language rather than computer-science language.
- **[GUIDE.md §5, "Making a change safely (the recipe)"](../GUIDE.md)** and **§6, "The
  testing philosophy"** — the habits Part 3 was an example of.
- **[FLUTTER.md](FLUTTER.md)** — the same ground as Part 1 and 2's Dart, taught
  properly and in order: notifiers, the theme's inherited widget, custom painting,
  gestures, tests.
- **[RUST.md](RUST.md)** — ownership, borrowing, `Option`, `Result`, traits and
  iterators, every example taken from this engine.
- **[05-BRIDGE.md](05-BRIDGE.md)** — the seam in full: what may cross it, what may
  not, and why some calls are synchronous and most are not.
- **[06-FRONTEND.md](06-FRONTEND.md)** — how the frontend is organised, panel by
  panel, once you want to change one that is bigger than a header line.

And the two you will come back to rather than read once:
[09-DOING-IT-YOURSELF.md](09-DOING-IT-YOURSELF.md) for every command, and
[00-MAP.md](00-MAP.md) for "where do I change X".

## If you want to keep going on this one

Three next steps in the same area, in rising order of difficulty:

1. **Make the count clickable** — filter the project list to items with sound, the
   way the missing-file header filters to missing ones. Everything you need is the
   `_MissingHeaderFrb` widget beside yours.
2. **Open the waveform lanes with the workspace.** Applying Audio could open the
   Audio ▸ Waveform twirl on every layer that has one — the same thing `LL` does on a
   selection, in `timeline_panel_frb.dart`. The work is deciding where that belongs,
   which is a better exercise than the code.
3. **Give the Audio workspace its own panel.** `Panel.audio` does not exist yet; the
   comment in `dock.dart` has been promising it for a while. That one is a feature,
   not an exercise — but you now know every file it would touch.
