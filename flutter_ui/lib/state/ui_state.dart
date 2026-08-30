// Everything the session holds that is not the document itself: panel focus,
// selection, playback, the Viewer's look, and the saved session. Lifted out of
// main.dart unchanged.
//
// LumitUiState is left whole. It is one ChangeNotifier with one job, and its
// parts share private fields — a class body cannot be split across part files,
// and turning its methods into extensions would change how they dispatch.

import 'dart:async';
import 'dart:convert';
import 'dart:io' show Platform;
import 'dart:ui' show AppExitResponse;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:lumit_flutter/panels/easing_curve.dart' show EasingCurve;
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/panels/viewer_texture_controller.dart';
import 'package:lumit_flutter/shell/about_window_frb.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart' show setAudioDevice;
import 'package:lumit_flutter/src/rust/api/cache.dart';
import 'package:lumit_flutter/src/rust/api/colour.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart'
    show BridgeNodeRef, BridgeNodeRef_Source;
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart' show setAutosave;
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/state/comp_model.dart';
import 'package:lumit_flutter/state/clipboard.dart';
import 'package:lumit_flutter/state/comp_time.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/dropper.dart';
import 'package:lumit_flutter/state/keymap.dart';
import 'package:lumit_flutter/state/animated_mask_paths.dart';
import 'package:lumit_flutter/state/layer_bounds.dart';
import 'package:lumit_flutter/state/playback_loop.dart';
import 'package:lumit_flutter/state/preview_progress.dart';
import 'package:lumit_flutter/state/render_timings.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/state/updates.dart';
import 'package:lumit_flutter/state/viewer_view.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:uuid/uuid.dart';

/// What [LumitUiState.colourSummary] reads before anything has been asked, and
/// whenever the project names no OCIO config: the built-in colour family, which
/// is what every project written before K-490 uses.
const BridgeColourSummary noColourConfig = BridgeColourSummary(
  path: '',
  loaded: false,
  problem: '',
  problemArgs: [],
  problemEnglish: '',
  spaces: [],
  displays: [],
);

/// One entered inner shader graph (K-642, custom-shader.md §4.2): the handle
/// the canvas edits through, and the words the breadcrumb reads — captured on
/// the double-click so drawing them costs no call.
typedef ShaderGraphEntry = ({
  LayerReference layer,
  UuidValue effect,
  String compName,
  String layerName,
  String effectName,
});

class LumitUiState extends ChangeNotifier {
  /// Everything that outlives the session: the panel layout, the appearance,
  /// UI scale, tooltips, autosave and export defaults.
  ///
  /// This is the same [Workspace] the shell has always used, loaded from disk
  /// on construction — the port briefly kept its own copies of the layout and
  /// the colour scheme here instead, which is why arrangements stopped
  /// surviving a restart and the Settings window's scale slider moved nothing.
  final Workspace workspace;

  /// The keyboard map every shortcut is looked up in (docs/07 §15, K-199).
  late final KeymapState keymap;

  /// Whether there is a newer Lumit, and fetching it (K-296).
  ///
  /// One for the session, here, because the Help menu and Settings ▸ General
  /// are two views of the same check and neither owns it. The version is passed
  /// as a function, not a string: it comes over the bridge, and a widget test
  /// that builds this state must not call the engine merely by existing.
  late final UpdateService updates =
      UpdateService(currentVersion: () => versionFromBootLine(lumitVersion()));

  /// How big each layer's content is, for the Viewer's boxes and hit-testing
  /// (K-217). Held here because the answer is the document's, not a panel's,
  /// and probing a clip is disk work that must happen once rather than per
  /// Viewer rebuild.
  final LayerBoundsCache layerBounds = LayerBoundsCache();

  /// Where a keyed mask's shape actually is at the frame on screen (K-342), so
  /// the Viewer's wireframe follows an animated path instead of the still one
  /// the mask still carries. Held against the document and the playhead, so a
  /// hover asks the engine nothing.
  final AnimatedMaskPaths animatedMaskPaths = AnimatedMaskPaths();

  /// Which tool the toolbar has armed (docs/07 §1.7, K-216).
  ///
  /// Session state at the shell level, like the dropper below it and for the
  /// same reason: the tool is picked in one place and read in another, and no
  /// panel should have to be mounted for either.
  final ToolsState tools = ToolsState();

  DockSplit get split => workspace.dock;
  ValueNotifier<Panel?> activePanel = ValueNotifier(null);

  /// Move the focus ring on by [by] panels in the arrangement's own order —
  /// `Ctrl+F6` forwards, `Ctrl+Shift+F6` back (docs/07 §15, "Panels").
  ///
  /// The arrangement's order, not the enum's: what the ring walks is what is on
  /// screen, left to right and top to bottom as the tree visits it, so a panel
  /// dropped from the workspace is simply not in the cycle. A panel sitting
  /// behind a tab is *brought to the front* as it is reached, because a focus
  /// ring on something nobody can see is a keystroke that appears to have done
  /// nothing.
  ///
  /// Answers whether it moved, so an arrangement with nothing in it leaves the
  /// chord to whatever else might want it.
  bool cyclePanelFocus(int by) {
    final panels = panelsIn(split);
    if (panels.isEmpty) return false;
    final current = activePanel.value;
    final at = current == null ? -1 : panels.indexOf(current);
    // Nothing focused yet: the first panel is where a cycle begins, whichever
    // way it was asked to go.
    final next = at < 0 ? panels.first : panels[(at + by) % panels.length];
    activatePanelTab(split, next);
    activePanel.value = next;
    // Which tab a group fronts is part of the arrangement, and the arrangement
    // persists — `touch` both redraws the dock and writes it down.
    workspace.touch();
    return true;
  }

  /// Bring [panel] to the front of whatever tab group holds it (items 6.28,
  /// 6.35) — selecting a layer fronts the Effect controls, adopting a project
  /// fronts the Project panel.
  ///
  /// **The tab, not the focus.** [activePanel] is where the keyboard is
  /// pointed, and moving it here would take Delete and `Ctrl+A` away from the
  /// panel the user is actually working in the instant they clicked a layer
  /// in it. Fronting a tab shows something; it does not claim the keys.
  ///
  /// Which tab a group fronts is part of the arrangement, and the arrangement
  /// persists — `touch` both redraws the dock and writes it down.
  /// **Only when a tab actually moves.** This is asked for on every layer
  /// click, and the tab it fronts is nearly always fronted already; `touch`
  /// notifies the whole shell and saves the workspace, so the quiet case was
  /// repainting every panel and writing a file to front what was in front
  /// (docs/impl/ui-performance.md §4.4).
  void frontPanel(Panel panel) {
    if (!panelVisible(split, panel)) return;
    if (activatePanelTab(split, panel)) workspace.touch();
  }

  /// Bumped when `Ctrl+F` asks the focused panel to put the cursor in its
  /// search box (docs/07 §15).
  ///
  /// A notifier for the same reason as [togglePlayRequest]: the field belongs
  /// to whichever panel is focused, and the shell has no business reaching into
  /// one. Each panel with a search box listens and answers only when it is the
  /// focused one, so one request can never focus two fields.
  final ValueNotifier<int> panelSearchRequest = ValueNotifier(0);

  /// Ask the focused panel for its search box, and say whether there is one to
  /// ask for. Only two panels have one (docs/07 §15); anywhere else the chord
  /// is left alone rather than swallowed.
  bool requestPanelSearch() {
    final panel = activePanel.value;
    if (panel != Panel.project && panel != Panel.effectsAndPresets) {
      return false;
    }
    panelSearchRequest.value++;
    return true;
  }

  /// Whether [panel] is the one a [panelSearchRequest] is meant for.
  bool searchRequestIsFor(Panel panel) => activePanel.value == panel;

  /// Bumped when `Ctrl+A` asks the focused panel to select everything it holds
  /// (K-522).
  ///
  /// **Select all is per panel.** `edit.select.all` used to mean one thing
  /// wherever it was pressed — every layer in the composition — so `Ctrl+A` in
  /// the Project panel selected layers you could not see instead of the items
  /// in front of you. What "everything" is depends on where you are: items in
  /// the Project panel, layers in the Timeline, effects in the Effect controls
  /// panel, nodes in the Node graph.
  ///
  /// Built like [panelSearchRequest] and for the same reason: each panel keeps
  /// its own selection, and the shell has no business reaching into one. A
  /// panel listens, and answers only when it is the focused one — which
  /// [selectAllRequestIsFor] is how it asks.
  final ValueNotifier<int> selectAllRequest = ValueNotifier(0);

  /// The panels that keep a selection of their own, and so answer `Ctrl+A`
  /// themselves rather than letting it mean "every layer".
  ///
  /// The Timeline is deliberately absent: its selection *is* the composition's
  /// layers, which the shell already holds, so the shell answers for it.
  /// The Node graph joined once its pick became a set (K-523): before that it
  /// had a single node and nothing to select *all* of, and listing it would
  /// only have made Ctrl+A a dead key there.
  static const Set<Panel> _selectAllPanels = {
    Panel.project,
    Panel.effectControls,
    Panel.graph,
  };

  /// Ask the focused panel to select everything, and say whether one was asked.
  /// False means the shell should fall back to selecting every layer.
  bool requestSelectAll() {
    if (!_selectAllPanels.contains(activePanel.value)) return false;
    selectAllRequest.value++;
    return true;
  }

  /// Whether [panel] is the one a [selectAllRequest] is meant for.
  bool selectAllRequestIsFor(Panel panel) => activePanel.value == panel;

  /// A finer selection's claim on Delete (K-234), set by the Timeline while it
  /// is mounted and cleared when it goes.
  ///
  /// The shell's Delete removes the selected *layers*, which is only the right
  /// answer when nothing smaller is selected: with a mask row picked, Delete
  /// means that mask, and deleting the layer it sits on instead is the opposite
  /// of what was asked. The shell asks this first and stands down when it
  /// returns true. A callback rather than a race between key handlers: every
  /// hardware-keyboard handler runs on every key, so a panel cannot claim a
  /// chord simply by handling it.
  bool Function()? deleteClaim;

  /// The graph's claim on `Ctrl+Space` (K-673), set by the Graph panel while
  /// it is mounted and — when a Custom shader's inner graph is the panel's
  /// face — by that graph over it, chained exactly as [deleteClaim] is.
  ///
  /// The console is one surface with two answers: over the work it applies an
  /// effect to the selected layers, over the graph it **adds a box to the
  /// canvas**. The shell asks this first and stands down when it returns true,
  /// so the same key opens the same popover wearing whichever list the focused
  /// surface contributes.
  bool Function()? consoleClaim;

  /// The same claim, for Copy and Paste (K-300). The Timeline sets these while
  /// it is mounted: with keyframes selected, `Mod+C` means those keyframes, and
  /// `Mod+V` puts them back — the layer clipboard is what the chord falls
  /// through to. Each returns whether it took the chord.
  bool Function()? copyClaim;
  bool Function()? pasteClaim;

  /// Where the Easing panel sends a shape (K-349), published by the Timeline
  /// while it can take one and null when it cannot.
  ///
  /// The same claim idea as the three above, but a notifier rather than a bare
  /// field, because this one is *read to draw with*: the panel is persistent, so
  /// it must grey its Apply the moment there is nowhere to send a shape —
  /// no Timeline on screen, or a graph showing the speed lens, where a curve
  /// drawn against value travel does not belong (K-348). A bare field would
  /// leave the panel showing a live button until something else happened to
  /// rebuild it.
  ///
  /// The keyframe selection itself stays the Timeline's and is never published:
  /// the panel sends a shape and is told nothing about what it landed on.
  final ValueNotifier<ValueChanged<EasingCurve>?> easingApply =
      ValueNotifier(null);

  /// The appearance the shell is drawing in.
  ///
  /// Scheme and shape are held rather than the built theme, because the theme is
  /// derived from them — keeping the composed object as the source of truth
  /// would make "what did the user choose?" a question you answer by comparing
  /// colours.
  LumitColorScheme get scheme => workspace.colorScheme;
  ThemeShape get shape => workspace.themeShape;
  LumitTheme get theme => workspace.theme;

  /// Bumped when something outside the Viewer asks the transport to start or
  /// stop — the space bar, or a command.
  ///
  /// A notifier rather than a direct call because the ticker that runs playback
  /// belongs to the Viewer's own state: the shell should not have to reach into
  /// a panel, and the Viewer should not have to be mounted for the key to be
  /// harmless.
  final ValueNotifier<int> togglePlayRequest = ValueNotifier(0);

  void requestTogglePlay() => togglePlayRequest.value++;

  /// Look for a newer Lumit on launch, if that is switched on and it has been
  /// a day since the last look (K-296).
  ///
  /// Only ever a *look*: what it finds ends up as the wording of the Help menu
  /// row, and downloading anything still waits for a click. Failure is silent —
  /// a machine with no network has not done anything wrong, and an editor that
  /// opened with a complaint about the internet would be insufferable.
  Future<void> maybeCheckForUpdates() async {
    if (!workspace.autoUpdate) return;
    // Never under `flutter test`: a suite that mounts the shell would otherwise
    // reach the network, which is slow, flaky, and none of a test's business.
    if (Platform.environment.containsKey('FLUTTER_TEST')) return;
    if (!updates.dueForCheck(workspace.lastUpdateCheckMs)) return;
    await updates.check();
    workspace.rememberUpdateCheck(DateTime.now().millisecondsSinceEpoch);
  }

  /// Bumped when `Ctrl+Shift+P` asks for the command palette.
  ///
  /// A notifier for the same reason as [togglePlayRequest]: the palette's list
  /// of commands is the menu bar's, declared beside the menu items so the two
  /// cannot drift into different ideas of what "New composition" does. The
  /// shortcut asks for the palette rather than building a second list of its
  /// own — which would be exactly the drift that note warns about.
  final ValueNotifier<int> paletteRequest = ValueNotifier(0);

  void requestPalette() => paletteRequest.value++;

  /// Bumped when `Ctrl+Space` asks for the FX console (K-324). Its effects,
  /// comps and radial entries are the menu bar's, for the same reason the
  /// palette's commands are.
  final ValueNotifier<int> consoleRequest = ValueNotifier(0);

  void requestConsole() => consoleRequest.value++;

  /// A property row the Timeline has been asked to show — the layer and one of
  /// the `reveal.*` actions (docs/07 §4.3's P/S/R/T/A family). Set by the FX
  /// console's Keyframe ring (K-326) after it plants a key, so the key just
  /// made is on screen. The Timeline listens and *ensures* the row is open —
  /// no toggle, unlike the reveal keys, because asking to see a row twice
  /// should never hide it.
  final ValueNotifier<(UuidValue, String)?> revealPropertyRequest =
      ValueNotifier(null);

  void requestRevealProperty(UuidValue layer, String action) =>
      revealPropertyRequest.value = (layer, action);

  /// **Which property rows the Timeline has picked**, as their paths, published
  /// so the Viewer can answer the same question (K-341).
  ///
  /// A property belongs to a layer, so picking one is saying which layer is
  /// being worked on — and the Viewer should outline that layer and its masks
  /// exactly as it does for a layer picked on its own row. It also tells the
  /// Viewer which mask's shape is being edited: with a mask's **Path** row
  /// picked, that mask is the one whose points are offered for dragging.
  final ValueNotifier<List<String>> selectedProperties =
      ValueNotifier(const []);

  /// The other direction: something outside the Timeline asking it to pick a
  /// property row. Set by the Viewer when a mask path with keyframes is
  /// dragged, so the row whose key just moved is the row on screen.
  final ValueNotifier<String?> selectPropertyRequest = ValueNotifier(null);

  void requestSelectProperty(String path) => selectPropertyRequest.value = path;

  /// The Project panel's picked item — its selection anchor, published by the
  /// panel on every click (K-327). The full selection stays the panel's own;
  /// this is the one item the FX console acts on, so a Ctrl+Space over the
  /// Project panel offers "add this to the comp" rather than the new-layer
  /// ring it used to fall through to. Null with nothing picked there.
  final ValueNotifier<ItemReference?> selectedProjectItem = ValueNotifier(null);

  /// Bumped each time a rendered frame reaches the Viewer, on any of the three
  /// transports. Watched by anything that redraws when the picture does — the
  /// Timeline's cache bar, the Scopes panel.
  final ValueNotifier<int> frameArrived = ValueNotifier(0);

  /// Bumped when the engine banks a frame in the background (the idle cache
  /// fill). Its own notifier, not [frameArrived]: the picture did not change,
  /// so nothing that re-renders on a new picture (the Scopes) should stir —
  /// only the cache bar, which listens to both.
  final ValueNotifier<int> cacheChanged = ValueNotifier(0);

  /// Bumped when a Camera track analysis lands a solve (K-430). Its own
  /// notifier because a solve moves neither the document's revision nor the
  /// playhead — and those two are exactly what the Viewer's point cloud is
  /// keyed by, so without this it had no reason to ask again and the dots did
  /// not appear until the frame changed.
  final ValueNotifier<int> solveLanded = ValueNotifier(0);

  /// The preview tier the last frame was made at: 1 Full, 2 Half, 3 Third,
  /// 4 Quarter (K-030/K-171).
  ///
  /// Carried on the frame rather than asked for. The Viewer shows the tier in
  /// two places, and each of them asked the engine in its `build()` — two calls
  /// across the boundary for each frame of playback, ~48 a second at 24 fps,
  /// for a number that only a new frame can change.
  final ValueNotifier<int> previewTier = ValueNotifier(1);

  /// How far the frame the Viewer is waiting for has got, when that is worth
  /// drawing (docs/07 §2.5). Fed from the worker stream below; the Viewer's
  /// progress bar listens to it and nothing else does.
  final PreviewProgressTracker previewProgress = PreviewProgressTracker();

  /// The last measured frame's per-layer and per-effect render times
  /// (docs/13 §7.1). Empty — and the engine not measuring — until a column or
  /// a panel that shows the numbers asks for them.
  ///
  /// Switching it on asks for the frame under the playhead again, because
  /// numbers only exist for a frame the engine actually composites: without
  /// this the column sat empty until something else happened to want a render,
  /// which on a comp the idle fill had already made could be for ever.
  late final RenderTimings renderTimings = RenderTimings(
    onMeasuringStarted: requestFrame,
    // An engine that refuses the switch says so in the status line rather than
    // leaving a lit stopwatch over a column that will never fill.
    onEngineError: (error) => _app.postNotice(
      l10n.couldNotMeasureRenderTimes('$error'),
      error: true,
    ),
  );

  /// Whether the engine is playing.
  ///
  /// Mirrored, not decided: it goes true when [play] is called and false when
  /// the engine says playback ended or the user stops it. The transport reads it
  /// to know which button to draw.
  final ValueNotifier<bool> playing = ValueNotifier(false);

  /// Start playing the fronted composition from the playhead.
  ///
  /// Everything about *how* playback runs — which frame is next, whether the
  /// clock has moved on, when to give up a tier — belongs to the engine
  /// (K-181). This says go, and [_arrived] follows the frames back.
  void play() {
    final comp = selectedComp;
    if (comp == null) return;
    // The work area is the span being worked on, so it is the span playback
    // runs round: reaching its end goes back to its start and carries on,
    // and a comp nobody has narrowed runs round the whole of itself, because
    // that is what its work area is (K-203) — unless the playhead is parked
    // past that end, where looping would mean never showing the frame the user
    // is standing on ([playbackLoop]). Read once here rather than per frame —
    // it cannot change while the transport is running, and
    // [_arrived] fires at the comp's rate.
    final set = comp.getWorkArea();
    _loop = playbackLoop(
      workStart: set == null ? null : comp.frameAtTime(time: set.inPoint),
      workEnd: set == null ? null : comp.frameAtTime(time: set.outPoint),
      playhead: playheadFrame.value,
      lastFrame: comp.durationFrames() - 1,
    );
    _playedFrom = playheadFrame.value;
    // Whatever the scrub before this was waiting for, it is not what the user
    // is watching now: playback draws no progress bar (docs/07 §2.5), and one
    // left standing from the frame that started the run would be the only bar
    // that ever appeared during playback.
    previewProgress.stop();
    _playFrom(comp, playheadFrame.value);
    playing.value = true;
  }

  /// Where the playhead stood when [play] was called, so stopping can put it
  /// back (K-254). Null when nothing is playing.
  ///
  /// Held here rather than read off the comp because it is a fact about *this
  /// run of the transport*, not about the document: it must survive the frames
  /// arriving and moving the playhead, and it must be forgotten the moment
  /// playback ends however it ends.
  int? _playedFrom;

  /// The span playback loops round this run — the work area, or the whole comp
  /// when none is set. Null only when the run cannot loop: the playhead was
  /// parked past the span's end, which makes the run a preview of the tail
  /// rather than a pass round the span ([playbackLoop]).
  ({int start, int end})? _loop;

  void _playFrom(CompositionReference comp, int frame) => comp.play(
        from: BigInt.from(frame),
        scale: viewerScale,
        mode: workspace.performance.playback == PlaybackMode.adaptive
            ? BridgePlaybackMode.adaptive
            : BridgePlaybackMode.everyFrame,
      );

  /// Stop the transport, and — unless the user is taking hold of the playhead
  /// themselves — put the playhead back where play started (K-254).
  ///
  /// Returning is the default because playback is a *preview*: you park the
  /// playhead where you are working, watch it run, and expect to still be where
  /// you were when it stops. Somebody who wants the playhead to stay where the
  /// picture stopped ticks Settings ▸ Interface ▸ Editing.
  ///
  /// `restorePlayhead: false` is for the one case where returning would fight
  /// the user: scrubbing the ruler stops playback *in order to* move the
  /// playhead, so putting it back would undo the very gesture that stopped it.
  void stopPlayback({bool restorePlayhead = true}) {
    playing.value = false;
    _loop = null;
    selectedComp?.stopPlayback();
    _returnPlayhead(restore: restorePlayhead);
  }

  /// The half of stopping that moves the playhead — shared by the user's stop
  /// and by playback running off the end on its own, because "where am I when
  /// it stops" should not depend on *why* it stopped.
  void _returnPlayhead({bool restore = true}) {
    final from = _playedFrom;
    _playedFrom = null;
    if (!restore || from == null) return;
    if (workspace.interface.playheadStaysOnStop) return;
    // Setting the notifier is the whole of it: the Viewer listens and asks the
    // engine for the frame there, exactly as it does for any other move.
    playheadFrame.value = from;
  }

  /// Ask for the frame under the playhead as the document now stands.
  ///
  /// Called when the playhead moves or an edit lands — both are *facts* the
  /// engine is told, not requests the frontend schedules. The worker coalesces
  /// whatever piles up behind a render in flight (`drain_to_newest`), which is
  /// why this can be called freely and needs no in-flight bookkeeping here.
  /// Ignored during playback, where the engine is already choosing frames.
  void requestFrame() {
    if (playing.value) return;
    final comp = selectedComp;
    if (comp == null) return;
    try {
      comp.renderFrame(
        frame: BigInt.from(playheadFrame.value),
        scale: viewerScale,
        mode: workspace.performance.playback == PlaybackMode.adaptive
            ? BridgePlaybackMode.adaptive
            : BridgePlaybackMode.everyFrame,
        // The "at effect" chip (K-528). The engine latches it, so the drags,
        // the playback and the idle fill that follow show the same picture.
        prefix: viewerPrefix,
      );
    } catch (_) {
      // No worker yet, or a composition that has gone away. The next playhead
      // move or edit asks again; there is nothing to recover here.
    }
  }

  /// A frame arrived. While playing, the picture leads and the playhead follows
  /// it — that is what makes the transport show the frame actually on screen
  /// rather than the one the engine was asked for. Paused, the playhead is the
  /// user's and is left alone.
  void _arrived(int frame) {
    frameArrived.value++;
    if (!playing.value) return;
    if (playheadFrame.value != frame) playheadFrame.value = frame;
    // Round the work area: the frame at its end is shown, then playback starts
    // again from its start. Restarted through `play` rather than by moving the
    // playhead, because the sound and the scheduler's clock both take their
    // baseline from the frame play was asked for.
    final loop = _loop;
    final comp = selectedComp;
    if (loop != null && comp != null && frame >= loop.end) {
      playheadFrame.value = loop.start;
      _playFrom(comp, loop.start);
    }
  }

  /// Move the playhead because the user is taking hold of it — a drag on the
  /// time ruler, a click in the lane area (K-254).
  ///
  /// Different from setting [playheadFrame] directly in one way that matters:
  /// it **stops the transport first**. Scrubbing against running playback was
  /// unwinnable — the engine hands back a frame every tick and each one moved
  /// the playhead straight back off the pointer — so taking hold of it means
  /// taking it off the transport. The playhead does *not* return to where play
  /// started here: the point of the gesture is to end up somewhere else.
  void scrubTo(int frame) {
    if (playing.value) stopPlayback(restorePlayhead: false);
    playheadFrame.value = frame;
  }

  /// Move the playhead by `delta` frames, clamped to the fronted composition.
  void stepFrame(int delta) {
    final comp = selectedComp;
    if (comp == null) return;
    final last = comp.durationFrames() - 1;
    playheadFrame.value =
        (playheadFrame.value + delta).clamp(0, last < 0 ? 0 : last);
  }

  /// The locale the interface is currently drawn in — the saved choice, or the
  /// machine's own language when nothing has been chosen.
  Locale get locale {
    final saved = workspace.interface.language;
    return saved == null ? systemLocale() : localeFromTag(saved);
  }

  /// Point `t` at the current language. Cheap and idempotent, which is why it
  /// can hang off every workspace change rather than needing to know which
  /// setting moved.
  void _applyLanguage() => useLocale(locale);

  /// Settings → Interface → Language. Null means follow the machine.
  void setLanguage(String? tag) {
    workspace.interface.language = tag;
    workspace.settingsChanged();
  }

  /// Put the panels back where they started (Window → Reset workspace).
  void resetLayout() => workspace.resetWorkspaceLayout();

  /// Remember a layout the user changed by dragging a panel — app-wide, and
  /// against the open project, which is what makes two projects able to be
  /// arranged differently (K-245).
  void saveLayout() {
    // A workspace of the user's own keeps the arrangement they drag it into
    // (docs/07 §1.4); a preset's factory layout is not theirs to overwrite.
    workspace.rememberActiveWorkspaceLayout();
    workspace.save();
    rememberSession();
  }

  void setScheme(LumitColorScheme next) => workspace.setScheme(next);

  void setShape(ThemeShape next) => workspace.setShape(next);

  CompositionReference? _selectedComp;
  CompositionReference? get selectedComp => _selectedComp;

  /// The fronted comp as the panels draw it (K-184) — refreshed by one bridge
  /// call when the engine reports a change, read by everything else for free.
  final CompModel model = CompModel();

  ViewerTextureController controller = ViewerTextureController();

  /// The platform texture the Viewer draws — the only frame transport (K-183):
  /// every frame arrives as a GPU handle, never as pixels. Null before the
  /// first registration.
  ValueNotifier<int?> viewerFrameid = ValueNotifier(null);

  /// The layer everything single-layer works on: Effect controls, the keyboard
  /// commands, the Timeline's fold-out. The *primary* of the selection below.
  ValueNotifier<LayerReference?> selectedLayer = ValueNotifier(null);

  /// Which box the Graph panel has picked (K-471), or null for none.
  ///
  /// Session state at the shell level for the same reason the armed tool is:
  /// it is set in one panel and read in another — the Node panel draws the
  /// picked box's parameter rows — and neither should have to be mounted for
  /// the other to work. An *effect* box also fronts itself in the ordinary
  /// effect selection (K-300); this notifier is what carries the boxes that
  /// selection cannot name, the drivers among them.
  final ValueNotifier<BridgeNodeRef?> graphNode = ValueNotifier(null);

  /// The Custom shader whose **inner graph** the Graph panel is showing
  /// (K-642, docs/impl/custom-shader.md §4.2), or null when it shows the
  /// layer's own graph.
  ///
  /// Shell-level for the reason [graphNode] is: it is set by a double-click in
  /// one panel (the Graph panel's box, or the Effect controls heading — one
  /// selection, K-300) and read by the Graph panel, and neither should have to
  /// be mounted for the other to work. The names ride along so the breadcrumb
  /// costs no call in a rebuild.
  final ValueNotifier<ShaderGraphEntry?> shaderGraphEntry = ValueNotifier(null);

  /// Where each inner graph's canvas was left **this session**, by effect
  /// instance id: the pan and the zoom come back on re-entry (§4.2 — standing
  /// somewhere in a graph is a way of working on it, not an edit to it), and
  /// they are never in the document.
  ///
  /// ponytail: session memory only; the trigger for persisting it in
  /// `SavedSession` beside `compViews` is somebody missing their place across
  /// a restart.
  final Map<String, ({Offset pan, double zoom})> shaderGraphViews = {};

  /// Enter one Custom shader's inner graph (K-642 — "entering a shader node
  /// works like entering a precomp"). The one funnel both surfaces call, so
  /// the names in the breadcrumb are captured the same way whichever
  /// double-click it was.
  void enterShaderGraph(LayerReference layer, UuidValue effect,
      {required String effectName}) {
    var compName = '';
    var layerName = '';
    try {
      compName = _selectedComp?.getSettings().name ?? '';
      layerName = layer.getInfo().name;
    } catch (_) {
      // The comp or layer has gone under the gesture; the crumbs read blank
      // and the canvas's own reload decides whether there is anything to show.
    }
    shaderGraphEntry.value = (
      layer: layer,
      effect: effect,
      compName: compName,
      layerName: layerName,
      effectName: effectName,
    );
  }

  /// The breadcrumb's way back, and Escape's.
  void exitShaderGraph() => shaderGraphEntry.value = null;

  /// What Copy put down, for Paste to pick up (K-275). One tray for the
  /// session, shared by the Edit menu and the panels.
  ///
  /// Read directly; **written through the two methods below**, because Paste is
  /// greyed out while it is empty and a menu that never hears about the copy
  /// stays greyed until something else happens to repaint it. That is exactly
  /// how it behaved before those methods existed.
  final LumitClipboard clipboard = LumitClipboard();

  /// Copy a layer, and tell the interface so Paste ungreys.
  ///
  /// **Mirrored to the system clipboard** (K-302): a copy that leaves no trace
  /// anywhere the machine can see reads exactly like a copy that did nothing —
  /// paste into a text editor and nothing arrives. The document is the text.
  void copyLayerToClipboard(String text) {
    clipboard.putLayer(text);
    Clipboard.setData(ClipboardData(text: text));
    notifyListeners();
  }

  /// Copy one effect or a whole stack, same repaint, same mirror.
  void copyEffectsToClipboard(String text) {
    clipboard.putEffects(text);
    Clipboard.setData(ClipboardData(text: text));
    notifyListeners();
  }

  /// Take a Lumit document off the **system** clipboard into the tray, if
  /// there is one there and the tray has nothing of its own (K-302).
  ///
  /// This is how a copy made in another Lumit window arrives, and how a paste
  /// still works after something else on the machine has been copied in
  /// between. Ordinary text is left alone — [lumitDocumentKind] only answers
  /// for the two shapes the engine's paste calls accept.
  Future<bool> adoptSystemClipboard() async {
    final text = (await Clipboard.getData(Clipboard.kTextPlain))?.text;
    if (text == null) return false;
    if (text == clipboard.text) return !clipboard.isEmpty;
    switch (lumitDocumentKind(text)) {
      case ClipboardKind.layer:
        clipboard.putLayer(text);
      case ClipboardKind.effects:
        clipboard.putEffects(text);
      case null:
        return false;
    }
    notifyListeners();
    return true;
  }

  /// The whole selection, primary first (K-217).
  ///
  /// Kept beside [selectedLayer] rather than replacing it, because almost
  /// everything in the application acts on one layer and reads it directly —
  /// and a second notifier is cheaper than teaching forty call sites to take
  /// the first element of a list. The two are held in step by [_syncSelection]:
  /// setting [selectedLayer] on its own (which the Timeline and the tests do)
  /// makes that layer the entire selection, which is exactly what clicking one
  /// row means.
  final ValueNotifier<List<LayerReference>> selectedLayers =
      ValueNotifier(const []);

  /// The selection as ids, for a "is this one selected?" test that does not
  /// walk the list per layer per paint.
  Set<UuidValue> get selectedLayerIds =>
      {for (final layer in selectedLayers.value) layer.internallayerId};

  /// **Every layer the Viewer may draw an outline or an editable point on** —
  /// the selection, plus any layer a *property* of which is picked in the
  /// Timeline (K-341: picking a mask's Path row offers that mask's points
  /// without the layer itself ever being clicked).
  ///
  /// One definition rather than two, because two things now depend on it and
  /// they must not drift: the gizmo draws these layers' mask outlines, and the
  /// stage asks the engine for those masks' *interpolated* shapes (K-342). A
  /// layer that fell out of this set while its outline was still drawn would
  /// silently go back to drawing the shape the drawing tools last wrote rather
  /// than the one the picture shows, which is the bug K-342 exists to fix.
  ///
  /// Ids as strings, because half of it arrives as the head of a property path
  /// and parsing it back would be a throw waiting for a path shape nobody
  /// promised.
  Set<String> get outlinedLayerIds => {
        for (final layer in selectedLayers.value)
          layer.internallayerId.toString(),
        for (final path in selectedProperties.value)
          if (path.indexOf('/') > 0) path.substring(0, path.indexOf('/')),
      };

  /// Replace the selection. The first entry becomes [selectedLayer].
  void setSelection(List<LayerReference> layers) {
    selectedLayers.value = List.unmodifiable(layers);
    selectedLayer.value = layers.isEmpty ? null : layers.first;
    // An effect belongs to a layer, so picking a different layer cannot leave
    // the old layer's effects picked (K-300) — Copy would then act on something
    // no longer on screen.
    clearEffectSelection();
  }

  /// The effects picked out of one layer's stack (K-300), as instance ids in
  /// **stack order** — what Copy and Cut act on when it is not empty.
  ///
  /// Held here rather than in either panel because an effect is picked in two
  /// places — the Effect controls panel's heading and the Timeline fold-out's
  /// row — and one selection shown in both is what makes those two places one
  /// interface rather than two. [selectedEffectsLayer] is the layer they are
  /// on: the effect ids alone name nothing the engine can find.
  final ValueNotifier<List<UuidValue>> selectedEffects =
      ValueNotifier(const []);
  LayerReference? selectedEffectsLayer;

  /// Whether the Viewer is showing the picture **at** the selected effect —
  /// that layer's stack stopping there — rather than the finished composition
  /// (K-528). The "at effect" chip over the picture is what turns it on.
  ///
  /// Session state at the shell level, like the armed tool and the picked
  /// graph box: it is set on the Viewer and read where the render is asked
  /// for, and the two need not be mounted together.
  final ValueNotifier<bool> atSelectedEffect = ValueNotifier(false);

  /// The points the chip has been left engaged at, by node, for the session
  /// (N4).
  ///
  /// **The chip is per box, and it stays as it was left.** Walking back onto a
  /// box you were looking *at* shows it at that box again, without asking
  /// twice; walking back onto one you turned off shows the finished picture.
  /// A box that has never been answered takes the answer the walk arrived with
  /// — which is what makes stepping down a stack with the chip on keep working
  /// (K-528) — and keeps it from then on.
  ///
  /// Session state, not document state: this is a way of *looking*, so it has
  /// no business in the file, on the undo stack, or in what "unsaved changes"
  /// means. It goes when the application does.
  final Map<String, bool> _atNodes = {};

  /// The point on the chain the chip is about: the layer, and the effect the
  /// stack stops **after** — null for the layer's own picture, which is the
  /// Source box (N4). Null altogether when nothing names a single point.
  ///
  /// One picked effect names one, in either panel that picks effects. The
  /// Source box carries no effect id and so cannot ride that selection at all;
  /// the Graph panel's own pick is what names it.
  (LayerReference, UuidValue?)? get viewerPrefixPoint {
    final layer = selectedEffectsLayer;
    final picked = selectedEffects.value;
    if (layer != null && picked.length == 1) return (layer, picked.single);
    final onSource = selectedLayer.value;
    if (graphNode.value is BridgeNodeRef_Source && onSource != null) {
      return (onSource, null);
    }
    return null;
  }

  /// How [_atNodes] remembers one point.
  String? get _chipNodeKey {
    final point = viewerPrefixPoint;
    if (point == null) return null;
    return '${point.$1.internallayerId}/${point.$2 ?? 'source'}';
  }

  /// Where the Viewer is cutting the stack, or null for the picture as the
  /// document has it — read by [requestFrame] and by nothing else.
  ///
  /// **Derived, never stored.** The picked point and the chip engaged is the
  /// whole of it, so the cut cannot drift from the selection it names: there is
  /// no second copy to keep in step. A run of effects picked names no single
  /// point and so cuts nothing.
  BridgePrefixPoint? get viewerPrefix {
    if (!atSelectedEffect.value) return null;
    final point = viewerPrefixPoint;
    if (point == null) return null;
    return BridgePrefixPoint(layer: point.$1, effect: point.$2);
  }

  /// Turn the chip on or off, and show what it asks for.
  ///
  /// The render is the point: nothing else tells the engine, and the frame the
  /// Viewer is already showing is of the other picture. One call, which is the
  /// same one a playhead step makes. The answer is remembered against this box,
  /// so coming back to it comes back to this.
  void setAtSelectedEffect(bool on) {
    if (_chipNodeKey case final key?) _atNodes[key] = on;
    if (atSelectedEffect.value == on) return;
    atSelectedEffect.value = on;
    requestFrame();
  }

  /// Follow the pick: the chip reads how this box was last left (N4).
  ///
  /// Picking a **different** box moves the point and shows that box's own
  /// answer — walking down a stack watching each effect land is what it is for.
  /// Picking a run, or nothing, leaves no single point to stop at, so the chip
  /// goes off: one that outlived its selection would leave the Viewer quietly
  /// showing a truncated composition with nothing on screen saying why.
  ///
  /// Silent unless the answer actually changes, because a box is picked on
  /// every click in three panels and a render apiece would be a render nobody
  /// asked for.
  void _followSelectionWithChip() {
    final key = _chipNodeKey;
    // Nothing names a single point, so the chip has nothing to say and goes
    // quiet. What each box was left at is remembered, so coming back to one is
    // coming back to where you left it.
    final on = key == null ? false : _atNodes[key] ?? atSelectedEffect.value;
    if (key != null) _atNodes[key] = on;
    if (atSelectedEffect.value == on) return;
    atSelectedEffect.value = on;
    requestFrame();
  }

  /// Replace the effect selection outright — what the Timeline hands over,
  /// having already applied the click rules to its own rows.
  void setEffectSelection(LayerReference layer, List<UuidValue> effects) {
    if (effects.isEmpty) {
      clearEffectSelection();
      return;
    }
    selectedEffectsLayer = layer;
    selectedEffects.value = List.unmodifiable(effects);
    _followSelectionWithChip();
  }

  /// Pick [id] by click: plain replaces, Ctrl toggles, Shift extends the run
  /// along [order] (the layer's stack, top to bottom) — the same three rules a
  /// layer row and a property row follow, because a selection that behaved one
  /// way here and another there would be two selections to learn.
  void pickEffect(
    LayerReference layer,
    UuidValue id, {
    required List<UuidValue> order,
  }) {
    final keys = HardwareKeyboard.instance;
    final held = selectedEffectsLayer?.internallayerId == layer.internallayerId
        ? [...selectedEffects.value]
        : <UuidValue>[];
    if (keys.isControlPressed || keys.isMetaPressed) {
      if (!held.remove(id)) held.add(id);
    } else if (keys.isShiftPressed && held.isNotEmpty) {
      final a = order.indexOf(held.last);
      final b = order.indexOf(id);
      if (a < 0 || b < 0) {
        if (!held.contains(id)) held.add(id);
      } else {
        for (var i = a < b ? a : b; i <= (a < b ? b : a); i++) {
          if (!held.contains(order[i])) held.add(order[i]);
        }
      }
    } else {
      held
        ..clear()
        ..add(id);
    }
    setEffectSelection(layer, held);
  }

  /// What **Copy effect** on [id]'s heading takes: the whole picked run when
  /// this effect is part of it, else just this one (K-300). Right-clicking a
  /// heading outside the selection copies what was right-clicked, which is what
  /// every list in the application does.
  List<UuidValue> effectsToCopy(LayerReference layer, UuidValue id) =>
      selectedEffectsLayer?.internallayerId == layer.internallayerId &&
              selectedEffects.value.contains(id)
          ? selectedEffects.value
          : [id];

  /// Nothing picked out of any stack — a layer chosen, a parameter chosen,
  /// empty space clicked.
  void clearEffectSelection() {
    selectedEffectsLayer = null;
    if (selectedEffects.value.isEmpty) return;
    selectedEffects.value = const [];
    _followSelectionWithChip();
  }

  /// Add [layer] to the selection, or take it out again — Shift-click.
  void toggleSelected(LayerReference layer) {
    final id = layer.internallayerId;
    final next = [
      for (final held in selectedLayers.value)
        if (held.internallayerId != id) held,
    ];
    if (next.length == selectedLayers.value.length) next.add(layer);
    setSelection(next);
  }

  void clearSelection() => setSelection(const []);

  /// The turn a Rotation-tool drag is part way through, by layer id (K-230).
  ///
  /// The picture is previewed at the new angle while the drag is in flight, but
  /// the document still holds the old one — so the wireframe drawn from the
  /// document lagged the picture and only caught up on release. The tool that
  /// is turning publishes here and the gizmo that draws the boxes reads it; the
  /// two are different widgets in different layers of the Viewer's stack, and
  /// this is the one value they share. Empty whenever nothing is turning.
  final ValueNotifier<Map<UuidValue, double>> liveRotations =
      ValueNotifier(const {});

  /// The line a Type edit is part way through, by layer id (K-232).
  ///
  /// Published for the same reason as [liveRotations]: what is being typed is
  /// previewed on the picture while the document still holds the old document,
  /// so a box measured from the document does not grow as the words do. Empty
  /// whenever nothing is being typed.
  final ValueNotifier<Map<UuidValue, ({String text, double size})>> liveText =
      ValueNotifier(const {});

  /// The transform a value scrub is part way through, by layer id.
  ///
  /// The third of the same family, and for the reason [liveRotations] gives:
  /// dragging Position or Scale — in the property rows or on a curve in the
  /// graph — previews the *picture* at the new value while the document still
  /// holds the old one, so the box drawn from the document sat still until the
  /// drag was released. The row that is dragging publishes the provisional
  /// transform it already built for the preview, and the boxes read it.
  ///
  /// At most one layer at a time: a gesture is one property of one layer
  /// (see `previewChannelEdits`). Empty whenever nothing is being scrubbed —
  /// and it must be emptied on release, or the box would hold the last
  /// provisional value for ever.
  final ValueNotifier<Map<UuidValue, BridgeTransform>> liveTransforms =
      ValueNotifier(const {});

  /// Forget layers that are no longer in the composition (K-238).
  ///
  /// **Why this is not merely tidy.** A selection is not only a highlight — it
  /// is the answer to "which layer does this tool act on?". Undo a shape layer
  /// and the layer went, but its id stayed selected, so the next shape drag
  /// still believed a layer was selected and tried to draw a *mask* on one that
  /// no longer existed. The engine refused, the refusal was swallowed, and the
  /// drag did nothing: the tool had simply stopped working, with nothing on
  /// screen to say why.
  ///
  /// Undo is only the easiest way to see it. Deleting a layer from the
  /// Timeline, closing a comp, or any edit that removes a layer leaves the same
  /// stale name behind, which is why this is answered once, here, from the
  /// model — rather than at each of the places a layer can vanish.
  ///
  /// An empty model means nothing is loaded yet rather than everything has
  /// gone, so the selection is left alone: clearing it there would drop the
  /// selection on every rebind.
  void _dropVanishedFromSelection() {
    final held = selectedLayers.value;
    if (held.isEmpty) return;
    final live = model.heldLayers;
    if (live.isEmpty) return;
    final alive = {for (final entry in live) entry.layer.internallayerId};
    final kept = [
      for (final layer in held)
        if (alive.contains(layer.internallayerId)) layer,
    ];
    if (kept.length == held.length) return;
    setSelection(kept);
  }

  /// Keep the list honest when something sets the primary on its own.
  void _syncSelection() {
    final primary = selectedLayer.value;
    // Whichever way round the two notifiers were set, the selection has just
    // changed, and it is part of the session (see [rememberSession]).
    rememberSession();
    // **A selected layer fronts its controls** (item 6.28): what you asked for
    // by clicking the layer is what the panel behind the tab is showing, and
    // having to go and find that tab is a step nobody wants to take twice.
    // Not while a project is being restored — the session's own selection is
    // put back there, and the Project panel is what an opened project fronts
    // (item 6.35).
    if (primary != null && !_restoring) frontPanel(Panel.effectControls);
    if (primary == null) {
      if (selectedLayers.value.isNotEmpty) selectedLayers.value = const [];
      return;
    }
    if (selectedLayerIds.contains(primary.internallayerId)) return;
    selectedLayers.value = List.unmodifiable([primary]);
  }

  /// The frame every panel renders and previews at.
  ///
  /// Held here rather than inside the Timeline because it is not the Timeline's
  /// alone: the Effect controls panel previews a drag at the playhead, and its
  /// preview landing on a different frame from the one on screen would show the
  /// wrong picture. A notifier so a scrub redraws only what watches it.
  ValueNotifier<int> playheadFrame = ValueNotifier(0);

  /// What fraction of comp resolution the Viewer is actually showing, which is
  /// the `scale` every render request carries. 1.0 until the Viewer has been laid
  /// out and can measure itself.
  ///
  /// This is why a Viewer in a small panel is cheap: the engine decodes and
  /// composites at the size being displayed rather than always at comp
  /// resolution. It is the frb counterpart of v0's `effectivePreviewScale`, minus
  /// the adaptive quality tier (K-171), which is not ported yet — so this tracks
  /// the panel size only, not measured render cost.
  ///
  /// A getter, not a field, because two separate things decide it: the panel
  /// measures itself ([reportViewerScale]) and the user chooses a preview
  /// resolution ([previewResolution]). Resolving them here means a change to
  /// either is in force on the very next render request, with nothing to keep
  /// in step.
  double get viewerScale => previewResolution.scaleFor(_panelScale);

  /// The scale the *panel* implies, last time the Viewer laid itself out.
  double _panelScale = 1.0;

  /// Called by the Viewer as it lays out. Clamped to (0, 1]: rendering *above*
  /// comp resolution would cost more for no visible gain, and a zero or negative
  /// scale is meaningless.
  ///
  /// **A panel that has changed size asks for the frame again** (K-430). On
  /// Auto the scale is whatever the panel could show at the moment it laid
  /// itself out, and the first layout of a session happens at whatever size the
  /// window opened at — so the first frame stayed at that scale until something
  /// else moved, because growing a panel is neither an edit nor a move of the
  /// playhead and nothing else asks. Compared at 1 % granularity, which is how
  /// the engine keys a frame's scale: a smaller difference would name a frame
  /// already in hand.
  ///
  /// A fixed tier is left alone: it is a raster reduction the user chose, and
  /// the panel's size has no say in it.
  ///
  /// [settled] is false while the Viewer's zoom is in flight. A flight is dozens
  /// of layouts, and a frame asked for at each of them is dozens of frames
  /// nobody ever sees.
  void reportViewerScale(double scale, {bool settled = true}) {
    if (!scale.isFinite || scale <= 0) return;
    final was = _panelScale;
    _panelScale = scale > 1.0 ? 1.0 : scale;
    if (!settled || previewResolution.fraction != null) return;
    if ((_panelScale * 100).round() == (was * 100).round()) return;
    // After this frame, not during it: this runs from the Viewer's layout, and
    // a render request made there would rebuild the tree it is measuring.
    WidgetsBinding.instance.addPostFrameCallback((_) => requestFrame());
  }

  /// The preview resolution of each composition, by id (K-357, docs/07 §2.2
  /// item 2). Per comp because it is a way of *working on* one — a heavy shot
  /// wants Quarter while the title card beside it does not — and it rides the
  /// session blob beside [viewerLooks] rather than the document, because
  /// choosing it is not an edit and must never reach an export (glossary §5).
  final Map<String, PreviewResolution> previewResolutions = {};

  /// How many pixels the engine is asked for, for the fronted comp.
  ///
  /// **Full until something says otherwise** (K-670). Auto renders only what
  /// the panel can show, which is cheap and was the old default — but it means
  /// a picture whose sharpness depends on how wide the panel happens to be,
  /// and a first look at a shot that is soft for a reason nobody can see. A
  /// comp that has been given a tier keeps it; only "never chosen" reads
  /// differently now.
  PreviewResolution get previewResolution =>
      previewResolutions[_selectedComp?.internalid.toString()] ??
      PreviewResolution.full;

  /// Choose the preview resolution for the fronted comp, and ask for the frame
  /// again — the setting changes what the *next* frame is made of, so without
  /// the ask the picture would not change until something else moved.
  void setPreviewResolution(PreviewResolution resolution) {
    final id = _selectedComp?.internalid.toString();
    if (id == null || previewResolution == resolution) return;
    // Every tier is stored, Auto included: with Full the default, an absent
    // entry means "never chosen" and Auto is a choice like any other.
    previewResolutions[id] = resolution;
    // The View menu ticks the one in force, so the bar has to be rebuilt.
    notifyListeners();
    rememberSession();
    requestFrame();
  }

  /// A named magnification the Viewer has been asked to take (docs/07 §2.2).
  ///
  /// A notifier for the same reason as [togglePlayRequest]: the magnification
  /// belongs to the Viewer panel — "fit" cannot be worked out without the
  /// panel's size — and the shell must not have to reach into a panel that may
  /// not be mounted. The serial makes two identical requests in a row two
  /// events rather than one, so pressing Zoom in twice zooms twice.
  final ValueNotifier<(int, ViewerZoomCommand)?> viewerZoomRequest =
      ValueNotifier(null);

  int _viewerZoomRequests = 0;

  void requestViewerZoom(ViewerZoomCommand command) =>
      viewerZoomRequest.value = (++_viewerZoomRequests, command);

  /// The armed dropper, or null when the tool is not armed (docs/07 §7).
  ///
  /// One at a time, and held at the session level rather than inside the Viewer:
  /// the tool is armed from a parameter row in another panel entirely, and the
  /// Viewer must not have to be mounted for that click to be harmless.
  final ValueNotifier<DropperArm?> dropper = ValueNotifier(null);

  /// The window of pixels the last [requestDropperSample] read back, or null
  /// before the first reply. Cleared when the tool disarms, so a fresh arm
  /// never opens on the previous pick's pixels.
  ///
  /// A window, not a pixel: the magnifier cuts its own nine-by-nine out of this
  /// as the pointer moves, and only asks again when the pointer nears its edge
  /// (see `windowCovers`). That is what keeps a sweep across the picture to a
  /// handful of reads instead of one per mouse move.
  final ValueNotifier<BridgeSampledPixels?> dropperPatch = ValueNotifier(null);

  /// Arm the dropper. Replaces whatever was armed — two pending picks would
  /// leave the next click ambiguous.
  void armDropper(DropperArm arm) {
    dropperPatch.value = null;
    dropper.value = arm;
  }

  /// Put the dropper away, picked or not.
  void disarmDropper() {
    dropper.value = null;
    dropperPatch.value = null;
  }

  /// Ask the engine for a window of pixels around the point `(u, v)` of the
  /// picture, each a fraction from 0 to 1. The answer arrives on the worker
  /// stream and lands in [dropperPatch]; nothing here waits for it.
  ///
  /// A fraction rather than a pixel because the frontend cannot know which
  /// raster will be read — a reduced-resolution preview has its own grid, and
  /// the reply is what says which one it used.
  ///
  /// Called only when the window in hand cannot answer — the caller checks
  /// first — so this is a handful of calls per pick, not one per mouse move.
  void requestDropperSample(double u, double v) {
    final comp = selectedComp;
    final arm = dropper.value;
    if (comp == null || arm == null) return;
    try {
      comp.samplePixels(
        frame: BigInt.from(playheadFrame.value),
        u: u,
        v: v,
        window: dropperWindow,
        scale: viewerScale,
        layer: arm.sampleLayer,
      );
    } catch (_) {
      // No worker, or a composition that has gone away. The next pointer move
      // asks again; there is nothing to recover here.
    }
  }

  StreamSubscription? sub;
  StreamSubscription? _changes;

  /// The session's engine-facing state. Held because the comp list it caches
  /// is what says which comps still exist (K-184).
  final LumitState _app;

  LumitUiState(LumitState state, {Workspace? workspace})
      : _app = state,
        workspace = workspace ?? (Workspace()..load()) {
    // The language, before anything is built: `t` is a plain global
    // (l10n/strings.dart), so it has to hold the right strings by the time the
    // first widget asks for one. Registered ahead of `notifyListeners` below so
    // that a language change has already landed when the rebuild it triggers
    // runs — listeners fire in the order they were added.
    _applyLanguage();
    this.workspace.addListener(_applyLanguage);
    // Appearance and layout live in the workspace, so a change there is a
    // change here as far as any listening widget is concerned.
    this.workspace.addListener(notifyListeners);
    // Floating windows read and write where they were left through this
    // (K-242); the controls file has no other way to reach the store.
    modalPlacementStore = this.workspace;
    selectedLayer.addListener(_syncSelection);
    // A layer that has gone must leave the selection with it (K-238). The
    // model is the one place that knows which layers exist, so the pruning
    // hangs off its refresh rather than off each of the several ways a layer
    // can disappear.
    model.addListener(_dropVanishedFromSelection);
    // The Source box is picked on the canvas and nowhere else, so the effect
    // selection cannot be what tells the chip about it (N4).
    graphNode.addListener(_followSelectionWithChip);
    // And the same for the comp the model itself is bound to: it can be undone
    // out of existence while it is the one being looked at.
    model.addListener(_frontLiveCompIfFrontedOneHasGone);
    // A project being adopted — opened, or made new — is where the saved
    // session is put back. The engine state is the only thing that knows a
    // document has been swapped underneath us. The document loaded *now* is
    // the one this shell starts on, so it does not count as a swap: without
    // this the first edit of the session would read as a new project and
    // clear the fronted comp and the selection.
    _sessionProject = _app.project;
    _app.addListener(_adoptProjectSession);
    // The colour config is a document property, so it can only change when the
    // document does — which is exactly when this fires. Held from here on, so
    // no rebuild path ever asks (K-490, docs/impl/ocio.md §6.1).
    _app.addListener(refreshColourSummary);
    refreshColourSummary();
    // Where the playhead was left is worth keeping, and it moves far too often
    // to write down each time. So it is captured when the user steps away from
    // the window and when they close it, alongside the deliberate acts below.
    _lifecycle = AppLifecycleListener(
      onInactive: rememberSession,
      onExitRequested: () async {
        rememberSession();
        return AppExitResponse.exit;
      },
    );
    // The keymap: restored from the workspace if the user has changed one,
    // otherwise the engine's shipped defaults (K-199). Held here because
    // every keypress goes through it and the settings page edits it, so it
    // wants the same lifetime as the rest of the session's UI state.
    keymap = KeymapState(workspace: this.workspace);
    // The cache budgets: live engine state with no store behind it, so the
    // settings file carries the user's choice and hands it back here (K-194's
    // sizing only picks the *default*). Null means untouched — leave the
    // engine on its own default rather than writing today's default into the
    // file forever.
    final perf = this.workspace.performance;
    final ramBudget = perf.cacheBudgetBytes;
    if (ramBudget != null) setCacheBudget(bytes: BigInt.from(ramBudget));
    final vramBudget = perf.vramBudgetBytes;
    if (vramBudget != null) setVramCacheBudget(bytes: BigInt.from(vramBudget));
    final diskBudget = perf.diskBudgetBytes;
    if (diskBudget != null) setDiskCacheBudget(bytes: BigInt.from(diskBudget));
    // Which output Lumit is heard through. Same arrangement as the budgets: the
    // engine holds the live choice with no store behind it, so the settings
    // file carries it and hands it back here. Null means the system default,
    // which is what the engine is already following — the call is skipped
    // rather than made with an empty id, so nothing happens on a fresh install.
    final device = this.workspace.audioDevice;
    if (device != null) setAudioDevice(id: device);
    // How often a spare copy of the work is written, and how many are kept.
    // Always handed over rather than skipped when it matches the default: this
    // is what starts the engine's timer, so a fresh install with no settings
    // file still autosaves.
    setAutosave(
      minutes: this.workspace.autosaveMinutes,
      keep: this.workspace.autosaveKeep,
    );
    // Where the parked frames go. Restored the same way, and by name rather
    // than by index so a reordered enum cannot silently move a user's cache.
    final where = perf.diskCacheLocation;
    if (where != null) {
      setDiskCacheLocation(
        location: cacheLocationFromName(where),
        folder: perf.diskCacheFolder ?? '',
      );
    }
    // The read model re-reads on every committed change — one bridge call —
    // and every panel that draws layers repaints from it (K-184). **Once per
    // revision** (K-680): the panel that committed the op has usually
    // refreshed already, and a second wave for the same document is a rebuild
    // of every panel that finds nothing new to draw.
    _changes = state.onChange.listen((event) {
      clearCompTimeCache(rate: event.items);
      model.refreshIfMoved();
    });
    sub = state.onWorkerResponse.listen((msg) {
      // Any reply at all means the new project's worker is up and answering,
      // which is what the opening card is waiting on. Not the frame alone: a
      // first render that faults would otherwise leave the shell covered.
      state.previewReady();
      switch (msg) {
        case WorkerResponse_RenderedDMABuf frame:
          previewTier.value = frame.field0.tier;
          _showDmabuf(frame.field0);
        case WorkerResponse_RenderedSharedTexture frame:
          previewTier.value = frame.field0.tier;
          _showSharedTexture(frame.field0);
        // Scope traces ride the same stream; the Scopes panel subscribes to it
        // directly, so there is nothing for the Viewer to do with one.
        case WorkerResponse_Scope():
          break;
        // Playback ran off the end on its own. Stopping because the *user* asked
        // needs no message — `stopPlayback` already set the flag.
        case WorkerResponse_PlaybackEnded():
          playing.value = false;
          // Running off the end returns the playhead too (K-254): where you are
          // when the transport stops should not depend on whether you stopped
          // it or the composition ran out.
          _returnPlayhead();
        case WorkerResponse_CacheFilled():
          cacheChanged.value++;
        // The pixels under the dropper. Held rather than acted on: the
        // magnifier draws whatever the last read said, and the click that
        // picks reads it from here.
        case WorkerResponse_Sampled(:final field0):
          dropperPatch.value = field0;
        // How far the frame being waited on has got. The engine sends these
        // only for a frame somebody is waiting on — never during playback —
        // and the tracker decides whether it is slow enough to draw.
        case WorkerResponse_RenderProgress(:final field0):
          previewProgress.report(field0);
        // What the frame just made cost. Only sent while something is showing
        // the numbers (`RenderTimings.setMeasuring`).
        case WorkerResponse_FrameProfile(:final field0):
          renderTimings.report(field0);
        // The graphics device went and the engine has already built another
        // (K-585). Everything that had to happen has happened by the time this
        // arrives — the frame behind it is being made on the new device — so
        // the only thing left is to say why the picture blinked.
        case WorkerResponse_DeviceReset():
          state.postNotice(l10n.graphicsDeviceReset);
      }
    }, onDone: () {
      // The engine's render worker has finished for good — it could not build
      // a renderer at all, or it faulted on every attempt at one. There will
      // never be another frame on this stream, and *that* is what has to be
      // said out loud: an editor whose preview has quietly stopped looks
      // identical to one that is merely busy, and the opening card waits on the
      // first reply for ever rather than admitting it is not coming.
      state.previewReady();
      state.postNotice(l10n.previewStopped);
    });
  }

  /// Linux zero-copy: register the DMA-BUF and show its texture. The first
  /// positional argument is the controller's identity key, and on this path the
  /// fd serves as that key — a non-null `fd` is also what tells the controller to
  /// send the DMA-BUF argument set rather than the DXGI one.
  void _showDmabuf(BridgeSharedFrameInfoLinux f) {
    controller
        .ensureRegistered(f.fd, f.width, f.height,
            fd: f.fd,
            stride: f.stride,
            offset: f.offset,
            fourcc: f.drmFourcc,
            modifier: f.modifier.toInt())
        .then((id) => _adoptTexture(id, f.frame.toInt()));
  }

  /// Windows and macOS zero-copy: register the surface by the one integer that
  /// names it — an NT handle for the shared D3D12 texture there, an `IOSurfaceID`
  /// here (K-195). One case for both, because the payload is the same shape.
  /// Leaving `fd` null is what selects the handle argument set.
  void _showSharedTexture(BridgeSharedFrameInfo f) {
    controller
        .ensureRegistered(f.handle.toInt(), f.width, f.height)
        .then((id) => _adoptTexture(id, f.frame.toInt()));
  }

  /// A registered texture is now current: mark a frame available and, if the id
  /// changed, point the Viewer at it.
  void _adoptTexture(int? id, int frame) {
    _arrived(frame);
    if (id == null) return;
    controller.frameReady();
    if (viewerFrameid.value != id) viewerFrameid.value = id;
  }

  @override
  void dispose() {
    _app.removeListener(_adoptProjectSession);
    _app.removeListener(refreshColourSummary);
    _lifecycle.dispose();
    sub?.cancel();
    _changes?.cancel();
    tools.dispose();
    layerBounds.dispose();
    // The progress tracker owns a timer — the delay that decides whether a
    // slow frame is slow enough to draw a bar for. Cancelling the subscription
    // above stops new reports, but a report that arrived a moment earlier has
    // already started one, and an uncancelled timer outlives the thing that
    // set it. In the application that is a small leak per project session; in
    // the frb tests it is a failure, and one that lands on whichever test
    // happens to be running when it fires rather than the one that caused it.
    previewProgress.dispose();
    model.dispose();
    cacheChanged.dispose();
    solveLanded.dispose();
    previewTier.dispose();
    viewerFrameid.dispose();
    selectedLayer.removeListener(_syncSelection);
    selectedLayer.dispose();
    selectedLayers.dispose();
    graphNode.dispose();
    shaderGraphEntry.dispose();
    atSelectedEffect.dispose();
    activePanel.dispose();
    paletteRequest.dispose();
    consoleRequest.dispose();
    viewerZoomRequest.dispose();
    panelSearchRequest.dispose();
    selectAllRequest.dispose();
    super.dispose();
  }

  /// The comps open as Timeline tabs (docs/07 §4: one tab per open comp), in
  /// the order first fronted. Fronting a comp opens its tab; closing a tab
  /// only closes the tab — the comp stays in the project.
  final List<UuidValue> openComps = [];

  /// The comp fronted before this one, so a comp that vanishes under the user
  /// can put them back where they came from rather than somewhere arbitrary.
  UuidValue? _previousComp;

  /// Where the user was in each composition, by id (K-624): the playhead, and
  /// the Timeline's magnification and scroll. Not in the document — standing
  /// somewhere in a comp is not an edit to it — so this rides in the session
  /// blob (see [session]) beside the looks, and never in an op.
  ///
  /// Written by two owners, each saying only what it knows: this class holds
  /// the playhead, the Timeline panel holds its own view. See
  /// [rememberCompView].
  final Map<String, CompView> compViews = {};

  /// Write down part of where the user is in a comp. Fields left null keep
  /// whatever was already recorded, so neither owner can wipe the other's half.
  void rememberCompView(String id, {int? frame, double? zoom, double? scroll}) {
    final was = compViews[id] ?? newCompView;
    compViews[id] = (
      frame: frame ?? was.frame,
      zoom: zoom ?? was.zoom,
      scroll: scroll ?? was.scroll,
    );
  }

  /// Front a composition, landing the playhead where the user left it (K-624).
  ///
  /// [atFrame] overrides that: opening a **Precomp layer** enters the nested
  /// comp at the moment that layer is showing, which the engine works out
  /// (`LayerReference.nestedEntryFrame`) because it runs through the layer's
  /// start offset and Retime map. Coming in any other way — the tab strip, the
  /// Project panel, the palette — is a return, and a return goes back to where
  /// you were.
  void setSelectedComp(CompositionReference? reference, {int? atFrame}) {
    if (reference != null && !openComps.contains(reference.internalid)) {
      openComps.add(reference.internalid);
    }
    final leaving = _selectedComp?.internalid;
    final arriving = reference?.internalid;
    final moved = arriving != leaving;
    if (moved) {
      _previousComp = leaving;
      // The comp being left keeps its place. Only the playhead is written
      // here; the Timeline writes its own half when it notices the change.
      if (leaving != null) {
        rememberCompView(leaving.toString(), frame: playheadFrame.value);
      }
    }
    _selectedComp = reference;
    model.bind(reference);
    if (moved && arriving != null) {
      final want = atFrame ?? compViews[arriving.toString()]?.frame ?? 0;
      // A comp shortened since it was last left must not open past its end.
      final last = model.durationFrames - 1;
      playheadFrame.value = last <= 0 ? 0 : want.clamp(0, last);
    }
    // Each comp is looked at its own way (K-314), so fronting one is what puts
    // its exposure and tone map back on the engine's renderer — which holds
    // exactly one view, for whatever the Viewer is showing.
    pushViewerLook();
    rememberSession();
    notifyListeners();
  }

  /// Open the composition a Precomp layer draws, landing on the frame that
  /// layer is showing (K-624).
  ///
  /// The mapping is the engine's (`LayerReference.nestedEntryFrame`): it runs
  /// the playhead through the layer's start offset and Retime map, so a
  /// half-speed precomp opens on the frame that is on screen. Standing off the
  /// layer's span there is no such frame, and it opens at the nested comp's
  /// start or end accordingly.
  ///
  /// Only for a layer in the **fronted** comp: the Hierarchy panel can show a
  /// layer several comps down, whose ruler the playhead is not on at all, and
  /// mapping through it would answer for the wrong clock. Those open where
  /// they were last left, like any other way in.
  void openNestedComp(LayerReference layer, CompositionReference comp) {
    int? at;
    if (layer.internalcompId == _selectedComp?.internalid) {
      try {
        at = layer.nestedEntryFrame(outerFrame: playheadFrame.value);
      } catch (_) {
        // A layer that has gone under us: open it where it was left.
      }
    }
    setSelectedComp(comp, atFrame: at);
  }

  // --- How the Viewer is looking (K-314) -----------------------------------

  /// Exposure and tone map per composition id. Not in the document: a way of
  /// looking is not an edit, so this rides in the session blob (see
  /// [session]) and never in an op.
  final Map<String, ViewerLook> viewerLooks = {};

  /// How the fronted comp is being looked at, neutral until something says
  /// otherwise.
  ///
  /// The one place the stored look becomes the look in use, which is why the
  /// tone map setting is honoured here and nowhere else: the Viewer bar, the
  /// engine push and the button all read this, so they cannot disagree. With
  /// the setting off the tone map is false whatever the comp stored — a
  /// session saved while it was engaged would otherwise be stranded with no
  /// button to turn it off. Only the reading is gated, not the store, so
  /// turning the setting back on finds the comp as it was — until the exposure
  /// is moved while the button is away, which writes the pair back as seen.
  ViewerLook get viewerLook {
    final stored =
        viewerLooks[_selectedComp?.internalid.toString()] ?? neutralLook;
    if (workspace.interface.showToneMap) return stored;
    return (stops: stored.stops, toneMap: false);
  }

  /// The whole look the engine was last told — exposure, tone map, the
  /// transparency grid and the colour view — or null when a freshly adopted
  /// worker has been told nothing yet. One record for the four, because they
  /// travel as one message ([pushViewerLook]) and a push can be skipped only
  /// when *all* of it is already in force.
  ({
    double stops,
    bool toneMap,
    bool grid,
    String roi,
    String view
  })? _pushedView;

  // --- The project's colour config (K-490) ----------------------------------

  /// The project's OCIO config as the interface reads it: its path, whether it
  /// is in force, why not when it is not, and every name it puts in a picker
  /// (docs/impl/ocio.md §6.1).
  ///
  /// **Held, never asked for in a build.** `colourSummary()` reads the config
  /// file to see whether it has changed on disk, so a widget that asked for it
  /// while rebuilding would stat a file per frame. It is fetched when the
  /// document changes — which is when it can differ — and every surface reads
  /// this field (K-183, and the bridge-call budget test).
  BridgeColourSummary colourSummary = noColourConfig;

  /// The `[display, view]` the Viewer is showing through, or null for the
  /// built-in transform. Session state rather than the document's: choosing
  /// how to look at a picture is not an edit, and it must never reach an
  /// export (docs/impl/ocio.md §6.1).
  List<String>? _colourView;
  List<String>? get colourView => _colourView;

  /// Show the picture through one of the config's views, or through the
  /// built-in transform (null). The push and the re-render are
  /// [pushViewerLook]'s — the view is part of the one look message.
  void setColourView(List<String>? view) {
    if (_colourView?.join(' ') == view?.join(' ')) return;
    _colourView = view == null ? null : List.of(view);
    notifyListeners();
    pushViewerLook();
  }

  /// Ask the engine what the project's colour config is now.
  ///
  /// Hung off the app state's own notifications, so it happens once per
  /// document change and nowhere else. A view naming a display the new config
  /// has not got is dropped rather than pushed at a renderer that would refuse
  /// it: the names in a picker belong to the config that is loaded.
  void refreshColourSummary() {
    BridgeColourSummary next;
    try {
      next = _app.project?.colourSummary() ?? noColourConfig;
    } catch (_) {
      // No project, or one that has gone. The built-in family is the honest
      // answer either way.
      next = noColourConfig;
    }
    if (next == colourSummary) return;
    colourSummary = next;
    if (!_viewIsOffered(next)) _colourView = null;
    notifyListeners();
    pushViewerLook();
  }

  bool _viewIsOffered(BridgeColourSummary summary) {
    final view = _colourView;
    if (view == null) return true;
    if (!summary.loaded || view.length != 2) return false;
    for (final display in summary.displays) {
      if (display.name == view.first) return display.views.contains(view.last);
    }
    return false;
  }

  /// The Viewer's **region of interest** per comp (K-362): the sub-rectangle
  /// the engine composites, as comp fractions `[u0, v0, u1, v1]`. Rides the
  /// session beside [previewResolutions] rather than the document, for the
  /// same reason — choosing where to look is not an edit, and must never reach
  /// an export.
  final Map<String, List<double>> regionsOfInterest = {};

  /// Whether the next drag on the picture sweeps out a region of interest
  /// (K-362). View state rather than panel state only because two widgets need
  /// it — the bar that arms it and the layer that takes the drag — and
  /// threading a transient flag between them through the stage is more
  /// machinery than the flag is worth. Never saved: arming is a thing you are
  /// in the middle of.
  bool _armingRegion = false;
  bool get armingRegion => _armingRegion;
  set armingRegion(bool on) {
    if (_armingRegion == on) return;
    _armingRegion = on;
    notifyListeners();
  }

  /// The fronted comp's region, or null for the whole frame.
  List<double>? get regionOfInterest {
    final id = _selectedComp?.internalid.toString();
    return id == null ? null : regionsOfInterest[id];
  }

  /// Set (or with null, clear) the fronted comp's region and re-render. A
  /// region that is not four numbers, is inside-out, or covers everything is
  /// no region — the engine says the same, and agreeing here keeps the button's
  /// lit state honest.
  void setRegionOfInterest(List<double>? region) {
    final id = _selectedComp?.internalid.toString();
    if (id == null) return;
    final ok = region != null &&
        region.length == 4 &&
        region.every((v) => v.isFinite) &&
        region[2] - region[0] > 0.001 &&
        region[3] - region[1] > 0.001 &&
        (region[0] > 0 || region[1] > 0 || region[2] < 1 || region[3] < 1);
    if (ok) {
      regionsOfInterest[id] = [
        region[0].clamp(0.0, 1.0),
        region[1].clamp(0.0, 1.0),
        region[2].clamp(0.0, 1.0),
        region[3].clamp(0.0, 1.0),
      ];
    } else {
      regionsOfInterest.remove(id);
    }
    _armingRegion = false;
    notifyListeners();
    rememberSession();
    pushViewerLook();
  }

  /// The Viewer's **overlays** per comp (K-416, docs/07 §2.2 items 5–6): the
  /// proportional grid and the title/action safe rectangles, drawn over the
  /// picture by the display and by nothing else.
  ///
  /// Keyed by comp exactly as [regionsOfInterest] is, and for the same reason —
  /// which marks you want over a shot belong to that shot. Nothing here crosses
  /// the bridge: the engine's picture is untouched, so unlike the region there
  /// is no push and no re-render, only a repaint. Session only for now; keeping
  /// a comp's overlays with the project is owed (docs/TODO.md).
  final Map<String, ({bool grid, bool safeAreas})> viewerOverlaysByComp = {};

  /// The fronted comp's overlays — nothing drawn, until something is asked for.
  ({bool grid, bool safeAreas}) get viewerOverlays =>
      viewerOverlaysByComp[_selectedComp?.internalid.toString()] ??
      (grid: false, safeAreas: false);

  /// Turn one overlay on or off, leaving the other as it is. Two named
  /// arguments rather than a whole record, for [setViewerStops]'s reason: a
  /// caller that rebuilt the pair from what it was *drawn* with would carry a
  /// stale reading for the other half into the write.
  void setViewerOverlays({bool? grid, bool? safeAreas}) {
    final id = _selectedComp?.internalid.toString();
    if (id == null) return;
    final now = viewerOverlays;
    final next =
        (grid: grid ?? now.grid, safeAreas: safeAreas ?? now.safeAreas);
    if (next.grid || next.safeAreas) {
      viewerOverlaysByComp[id] = next;
    } else {
      viewerOverlaysByComp.remove(id);
    }
    notifyListeners();
  }

  /// Whether the Viewer draws its **layer controls** (K-217, K-466): the
  /// wireframe round every layer, the transform handles and the hover
  /// highlight, on and off as one.
  ///
  /// Here rather than in the Viewer panel because two things reach it — the
  /// bar's own view menu and View ▸ Show wireframe (K-244) — and a switch
  /// with one route is a switch that disappears the day something intercepts
  /// that route. Display only: no engine copy, no cache entry, and nothing an
  /// export can see.
  ///
  /// The full wireframe *display mode* the specification also names (docs/07
  /// §2.2 item 5, outlines only and no raster) is a separate thing and is
  /// still owed; until it lands these two rows are the one switch.
  bool viewerLayerControls = true;

  void setViewerLayerControls(bool on) {
    if (viewerLayerControls == on) return;
    viewerLayerControls = on;
    notifyListeners();
  }

  /// Whether the Viewer's transparency grid is up (K-352). While it is, the
  /// engine leaves the comp's background colour out of the composite, so
  /// pixels nothing covers arrive transparent and the grid shows through.
  ///
  /// Held here rather than in the Viewer panel because the engine has to be
  /// told: it rides [pushViewerLook]'s one look message, and project adoption
  /// clears [_pushedView] so a worker just born — which starts opaque — is
  /// told afresh on the first front.
  bool viewerGrid = true;

  /// Flip the transparency grid. The push and the re-render are
  /// [pushViewerLook]'s: the grid is part of the one look message, and the
  /// change of record is what makes it ask for the frame again.
  void setViewerGrid(bool on) {
    if (viewerGrid == on) return;
    viewerGrid = on;
    notifyListeners();
    pushViewerLook();
  }

  /// Set the exposure, leaving the tone map as it is; and the mirror of it.
  ///
  /// Two setters rather than one taking a whole [ViewerLook], because each
  /// control must change only its own half. A control that rebuilt the pair
  /// from the value it was *drawn* with would carry a stale reading for the
  /// other half into the write — two changes between two rebuilds, and the
  /// second undoes the first.
  void setViewerStops(double stops) =>
      setViewerLook((stops: stops, toneMap: viewerLook.toneMap));

  void toggleViewerToneMap() =>
      setViewerLook((stops: viewerLook.stops, toneMap: !viewerLook.toneMap));

  /// Set how the fronted comp is looked at, tell the engine, and write it down.
  void setViewerLook(ViewerLook look) {
    final id = _selectedComp?.internalid.toString();
    if (id == null) return;
    if (look == neutralLook) {
      viewerLooks.remove(id);
    } else {
      viewerLooks[id] = look;
    }
    pushViewerLook();
    rememberSession();
    notifyListeners();
  }

  /// Tell the engine what the Viewer is looking through — exposure, tone map
  /// and the transparency grid, one message carrying the whole look — and ask
  /// for the frame again, because a setting changes what the *next* frame is
  /// made of.
  ///
  /// A look the renderer already holds ([_pushedView]) is nothing to say and
  /// nothing to re-render, so it is skipped — which is every fronting where
  /// nothing changed, and what keeps an exposure drag to one message per
  /// actual movement. The record clears on project adoption, because a new
  /// worker is born neutral-and-opaque and must be told afresh.
  void pushViewerLook() {
    final comp = selectedComp;
    if (comp == null) return;
    final look = viewerLook;
    final roi = regionOfInterest;
    final target = (
      stops: look.stops,
      toneMap: look.toneMap,
      grid: viewerGrid,
      // Compared as text: a record holding a list compares by identity, so two
      // equal regions would look different and re-push on every fronting.
      roi: roi?.join(',') ?? '',
      // And the colour view for the same reason. **It rides every call**: the
      // engine is told the look *whole*, so a message that left the view out
      // would not leave it alone — it would say "no view" and the picture
      // would quietly fall back to the built-in transform.
      view: _colourView?.join(' ') ?? '',
    );
    if (target == _pushedView) return;
    try {
      comp.setViewerLook(
        stops: target.stops,
        toneMap: target.toneMap,
        transparentBackground: target.grid,
        region: roi == null ? null : Float32List.fromList(roi),
        colourView: _colourView,
      );
    } catch (_) {
      // No worker yet, or a comp that has gone. The next change asks again —
      // and _pushedView stays as it was, so the ask is not skipped.
      return;
    }
    _pushedView = target;
    requestFrame();
  }

  // --- The per-project session ---------------------------------------------
  //
  // Where the user had got to in *this* document: the comps on the tab strip,
  // which one was fronted, where the playhead sat, what was selected. It is
  // kept in the workspace store keyed by the project's path rather than in the
  // `.lum`, because none of it is the document — a project file must stay
  // byte-identical between two saves of the same work and must not carry one
  // machine's habits to another (docs/10 §1.1, §2). The panel arrangement and
  // which panel each tab group fronts are already persisted there too, app-wide.

  /// The project whose session is on screen. Compared by identity, so a
  /// document swapped underneath the shell is noticed the moment it is adopted.
  ProjectReference? _sessionProject;

  late final AppLifecycleListener _lifecycle;

  /// Write down where the user is. A project with no file has nowhere to be
  /// written to — the sessions are keyed by path — so this is a no-op until it
  /// has been saved once.
  void rememberSession() {
    // Restoring moves the fronted comp and the selection, and each of those
    // moves would be written back — over the very session being read.
    if (_restoring) return;
    final path = _app.project?.path();
    if (path == null) return;
    workspace.rememberSession(path, session());
  }

  /// Where the user is, as the thing that gets written down: the tab strip, the
  /// fronted comp, the playhead, the selection, and how the panels are arranged.
  SavedSession session() {
    // The fronted comp's own record is only written when it is left, so the
    // live playhead is folded in here: a session written mid-work has to say
    // where the user actually is, not where they last arrived from.
    final front = _selectedComp?.internalid.toString();
    final views = Map.of(compViews);
    if (front != null) {
      views[front] = (
        frame: playheadFrame.value,
        zoom: views[front]?.zoom ?? newCompView.zoom,
        scroll: views[front]?.scroll ?? newCompView.scroll,
      );
    }
    return SavedSession(
        openComps: [for (final id in openComps) id.toString()],
        activeComp: front,
        frame: playheadFrame.value,
        compViews: views,
        selectedLayer: selectedLayer.value?.internallayerId.toString(),
        dock: workspace.dock.toJson(),
        viewerLooks: Map.of(viewerLooks),
        previewResolutions: {
          for (final e in previewResolutions.entries) e.key: e.value.name,
        },
        regionsOfInterest: {
          for (final e in regionsOfInterest.entries) e.key: List.of(e.value),
        });
  }

  /// The same thing as JSON, for the copy that goes inside the `.lum` so it
  /// travels with a project shared with someone else (K-245).
  String sessionJson() => jsonEncode(session().toJson());

  /// Put the saved session back after a project is opened, and start from
  /// nothing when a new one is made.
  ///
  /// Every id is checked against the document that actually loaded before it is
  /// used: a comp or layer deleted since the session was written must leave the
  /// user on a sensible default, never on a reference the engine has never
  /// heard of.
  bool _restoring = false;

  /// The arrangement the project file itself carries, or null when it has none
  /// or none that can be read.
  SavedSession? _embeddedSession(ProjectReference project) {
    try {
      final json = project.uiState();
      if (json == null) return null;
      final decoded = jsonDecode(json);
      if (decoded is! Map) return null;
      return SavedSession.fromJson(decoded.cast<String, dynamic>());
    } catch (_) {
      // A project written by another build may describe an interface this one
      // does not have. Opening it must still work; it simply opens arranged the
      // way this machine already was.
      return null;
    }
  }

  /// Put the panels where the session says, if it says anything about them.
  ///
  /// A layout naming a panel this build does not have loses **that pane** and
  /// keeps the rest ([`DockNode.fromJson`]) — a panel folded away must not cost
  /// anyone their arrangement. Anything else malformed is dropped whole rather
  /// than half-applied: the arrangement is a hint, and the one on screen is a
  /// perfectly good fallback.
  void _applyDock(Map<String, dynamic>? json) {
    if (json == null) return;
    try {
      final parsed = DockNode.fromJson(json);
      if (parsed is! DockSplit) return;
      workspace.dock = parsed;
      workspace.touch();
    } catch (_) {
      // Left as it was.
    }
  }

  void _adoptProjectSession() {
    final project = _app.project;
    if (identical(project, _sessionProject)) return;
    _sessionProject = project;

    var restoredArrangement = false;
    _restoring = true;
    try {
      // Nothing from the previous document may outlive it: its comp ids, its
      // playhead and its selection all belong to a project no longer loaded.
      openComps.clear();
      clearSelection();
      playheadFrame.value = 0;
      viewerLooks.clear();
      previewResolutions.clear();
      // Another project's colour config names another project's views.
      _colourView = null;
      // A new project is a new worker, and a new worker is born knowing
      // nothing of this session's look — the null record is what makes the
      // first front tell it everything, the grid included.
      _pushedView = null;
      setSelectedComp(null);
      // After the unfronting, not before it: letting go of a comp writes down
      // where it was, and that note belongs to the project just closed.
      compViews.clear();

      final path = project?.path();
      if (path == null) return;
      workspace.rememberProject(path);

      // This machine's own record of the project comes first: it is the more
      // recent account of what *this* user was doing, and it is kept up to date
      // between saves. The one in the file is what a project arriving from
      // somebody else brings with it (K-245), so it answers exactly when there
      // is no local record — the first time this project is opened here.
      final session = workspace.sessionFor(path) ?? _embeddedSession(project!);
      if (session == null) return;
      _applyDock(session.dock);
      restoredArrangement = true;

      final known = {
        for (final (comp, _) in _app.comps()) comp.internalid.toString(): comp,
      };
      // Only for comps this document still has: a look kept against a comp id
      // that has gone would be written back out for ever.
      viewerLooks.addEntries(
        session.viewerLooks.entries.where((e) => known.containsKey(e.key)),
      );
      // Same rule for the per-comp resolutions, plus: a name this build does
      // not have (a project written by a newer one) simply reads as the
      // default rather than stopping the project from opening.
      for (final e in session.previewResolutions.entries) {
        if (!known.containsKey(e.key)) continue;
        for (final r in PreviewResolution.values) {
          if (r.name == e.value) previewResolutions[e.key] = r;
        }
      }
      // The regions, checked against the comps that actually loaded — the
      // same rule every other id in a session gets (K-362).
      for (final e in session.regionsOfInterest.entries) {
        if (known.containsKey(e.key)) {
          regionsOfInterest[e.key] = List.of(e.value);
        }
      }
      // Where the user was in each comp (K-624), same rule again.
      compViews.addEntries(
        session.compViews.entries.where((e) => known.containsKey(e.key)),
      );
      for (final id in session.openComps) {
        final comp = known[id];
        if (comp != null) openComps.add(comp.internalid);
      }
      final front = known[session.activeComp] ??
          (openComps.isEmpty ? null : known[openComps.first.toString()]);
      // A session written before the per-comp record existed still says where
      // the fronted comp's playhead was, and that answer is not thrown away.
      final activeComp = session.activeComp;
      if (activeComp != null &&
          known.containsKey(activeComp) &&
          !compViews.containsKey(activeComp)) {
        rememberCompView(activeComp,
            frame: session.frame < 0 ? 0 : session.frame);
      }
      // Which puts the playhead back itself.
      setSelectedComp(front);
      // Unless there is no comp left to be in: no comp remembers the frame,
      // but the frame the user was on is still the frame.
      if (front == null) {
        playheadFrame.value = session.frame < 0 ? 0 : session.frame;
      }

      final wanted = session.selectedLayer;
      if (front == null || wanted == null) return;
      for (final layer in front.getLayers()) {
        if (layer.internallayerId.toString() == wanted) {
          setSelection([layer]);
          break;
        }
      }
    } finally {
      _restoring = false;
      // **A project arriving fronts the Project panel** (item 6.35) — a new
      // one, an opened one, and the empty one that replaces a closed one.
      // What is in the document is where work on it starts, and the panels
      // left fronted belong to the document that has just gone. UNLESS a
      // session restored its own arrangement: the project opens as it was
      // left (docs/07 §1.6, the session-restore promise), and that account
      // includes which tabs were fronted — the more specific rule wins.
      if (!restoredArrangement) frontPanel(Panel.project);
      // A project that opens with no composition fronted has no picture to
      // wait for, so the opening card would stand for ever waiting on a frame
      // nobody is going to ask for. In the `finally` because every early
      // return above is one of those cases.
      if (selectedComp == null) _app.previewReady();
    }
  }

  /// **A comp can be taken out from under the user.** Pre-compose, step into
  /// the new comp, undo: the layer comes back and the comp it pointed at stops
  /// existing, with the Timeline still fronting it. Every panel then reads a
  /// comp the engine has never heard of, which is what put a bridge error on
  /// screen where the timeline should be.
  ///
  /// So the same rule the layer selection follows (K-238) applies to the
  /// fronted comp: what has gone cannot stay fronted. Where to go instead, in
  /// order — the comp the user was in before this one, if it is still there;
  /// else the nearest open tab, left first and then right; else nothing
  /// fronted at all, which is the state the shell starts in and draws fine.
  void _frontLiveCompIfFrontedOneHasGone() {
    if (!model.compGone) return;
    final gone = _selectedComp!.internalid;
    final known = {
      for (final (comp, _) in _app.comps()) comp.internalid: comp,
    };
    final where = openComps.indexOf(gone);
    final at = where < 0 ? 0 : where;
    openComps.remove(gone);

    // Where the user came from, then leftwards from where the gone tab stood,
    // then rightwards — after the removal the tab that stood at `at` is the
    // right-hand neighbour.
    final order = <UuidValue?>[
      _previousComp,
      for (var i = at - 1; i >= 0; i--) openComps[i],
      for (var i = at; i < openComps.length; i++) openComps[i],
    ];
    CompositionReference? next;
    for (final id in order) {
      final candidate = id == null ? null : known[id];
      // Asked of the engine, not of the cached item walk: the walk is only
      // re-read when the change stream says the tree moved, and this runs on
      // the model's refresh, which can be the earlier of the two. Fronting a
      // comp that is *also* gone would land straight back here.
      if (candidate != null && _stillThere(candidate)) {
        next = candidate;
        break;
      }
    }
    _previousComp = null;
    setSelectedComp(next);
  }

  bool _stillThere(CompositionReference comp) {
    try {
      comp.getSettings();
      return true;
    } catch (_) {
      return false;
    }
  }

  /// Drop a comp's tab where [target]'s tab sits. The strip is drawn in this
  /// list's order, so moving the entry is the whole reorder — and it rides
  /// along in the session, like the rest of the tab strip.
  void moveComp(UuidValue id, UuidValue target) {
    final from = openComps.indexOf(id);
    final to = openComps.indexOf(target);
    if (from < 0 || to < 0 || from == to) return;
    openComps.removeAt(from);
    openComps.insert(to, id);
    rememberSession();
    notifyListeners();
  }

  /// Close a comp's Timeline tab. When the closed tab was fronted, [fallback]
  /// — the tab bar's nearest remaining neighbour — fronts instead.
  void closeComp(UuidValue id, {CompositionReference? fallback}) {
    openComps.remove(id);
    if (_selectedComp?.internalid == id) {
      setSelectedComp(fallback);
    } else {
      rememberSession();
      notifyListeners();
    }
  }
}
