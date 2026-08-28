// The application widget, the boot gate, and the shell view that dispatches
// keys and lays the panels out. Lifted out of main.dart unchanged.

import 'package:flutter/gestures.dart' show GestureBinding;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/panels/panels_frb.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/shell/precompose_dialog_frb.dart';
import 'package:lumit_flutter/shell/dock_widget.dart';
import 'package:lumit_flutter/shell/first_run_frb.dart';
import 'package:lumit_flutter/shell/fx_console_frb.dart'
    show lastKnownPointerPosition;
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/shell/project_settings_frb.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/shell/splash.dart';
import 'package:lumit_flutter/shell/status_line_frb.dart';
import 'package:lumit_flutter/shell/tool_bar_frb.dart';
import 'package:lumit_flutter/shell/welcome_frb.dart';
import 'package:lumit_flutter/src/rust/api/state.dart' show OpenProgress;
import 'package:lumit_flutter/src/rust/api/shell.dart' show bootLog;
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/state/viewer_view.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/state/ui_state.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/ui_scale.dart';
import 'package:provider/provider.dart';

class LumitAppNew extends StatelessWidget {
  final LumitState state;
  final LumitUiState uiState;

  /// Whether to open on the welcome screen (K-464). False when a project came
  /// in on the command line: that is already the answer the screen asks for.
  final bool welcome;

  const LumitAppNew(this.state, this.uiState, {super.key, this.welcome = true});

  @override
  Widget build(BuildContext context) {
    // WidgetsApp-level infrastructure only — no Material chrome
    // (docs/archive/flutter-port/04 "Why not Material chrome"). `ThemeData.dark()`
    // was doing real damage here: Material's own greys showed through wherever a
    // panel did not paint, so the shell read as a Material app with Lumit panels
    // in it rather than as Lumit. The backdrop is `surface0` from the theme.
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      // Lumit's own strings come from the `l10n` global rather than from
      // context (l10n/strings.dart); these delegates are still needed for the
      // parts of Flutter that do ask the tree — text selection menus, the
      // Material and Cupertino widgets under a dialogue, and the text direction
      // a right-to-left language would want.
      locale: uiState.locale,
      localizationsDelegates: Strings.localizationsDelegates,
      supportedLocales: Strings.supportedLocales,
      home: ChangeNotifierProvider.value(
        value: state,
        child: ChangeNotifierProvider.value(
          value: uiState,
          // Rebuilt when the workspace changes, so the scale slider and the
          // scheme picker take effect as they are moved.
          child: ListenableBuilder(
            listenable: uiState,
            builder: (context, _) => ThemeScope(
              theme: uiState.theme,
              animationLevel: uiState.workspace.animationLevel,
              showTooltips: uiState.workspace.interface.showTooltips,
              child: Directionality(
                textDirection: TextDirection.ltr,
                child: ColoredBox(
                  color: uiState.theme.surface0,
                  // Settings → Interface → UI scale, the Flutter counterpart of
                  // egui's `set_pixels_per_point`: layout and hit-testing scale
                  // together (see widgets/ui_scale.dart).
                  child: UiScaleView(
                    scale: uiState.workspace.interface.uiScale,
                    child: Overlay(initialEntries: [
                      OverlayEntry(
                          builder: (context) => BootGate(welcome: welcome))
                    ]),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The boot splash, the welcome screen, and the shell behind them once both are
/// done (K-008, K-464).
///
/// **Each of the three is the window in turn**, never a card floating over a
/// half-built application: the shell is not put in the tree until boot has
/// finished and the user has said how they want to start, so nothing of the
/// application shows through, no dialogue — the first-run question above all —
/// can appear underneath, and no panel starts asking the engine for pictures
/// nobody can see yet.
///
/// The lines the splash streams are the engine's own boot log: the library
/// version, the ABI, and what this build was compiled with. That is the only
/// thing the engine can say about starting up — there is no notice stream to
/// subscribe to, only `boot_log` (docs/TODO.md) — so it is what the splash
/// shows, and a build with no bridge at all falls back to the canned list in
/// splash.dart.
class BootGate extends StatefulWidget {
  /// Whether to show the splash at all. False in the tests that drive the
  /// whole shell, which have no boot to wait for and would otherwise spend a
  /// second and a third of simulated time watching one.
  final bool splash;

  /// Whether to show the welcome screen after it. False when a `.lum` came in
  /// on the command line — somebody who double-clicked a project has already
  /// said what they want to open — and in the tests that drive the shell.
  final bool welcome;

  const BootGate({super.key, this.splash = true, this.welcome = true});

  @override
  State<BootGate> createState() => _BootGateState();
}

class _BootGateState extends State<BootGate> {
  late bool _booting = widget.splash;

  /// Whether the welcome screen is the window right now. Both answers have to
  /// agree: a `.lum` on the command line stands the screen down for this launch
  /// ([BootGate.welcome]), and Settings ▸ General stands it down for every
  /// launch (K-481). Read once, here, because the shell behind it is what the
  /// setting sends somebody to.
  late bool _welcoming;

  @override
  void initState() {
    super.initState();
    _welcoming = widget.welcome &&
        context.read<LumitUiState>().workspace.showWelcomeOnLaunch;
  }

  /// The engine's boot log, or empty where there is no engine to ask — a
  /// placeholder build, or a widget test with no library loaded. Read once:
  /// it is a bridge call, and it answers the same thing every time.
  late final List<String> _lines = _readBootLog();

  static List<String> _readBootLog() {
    try {
      return bootLog();
    } catch (_) {
      return const [];
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_booting) {
      return SplashOverlay(
        lines: _lines,
        onDone: () {
          if (mounted) setState(() => _booting = false);
        },
      );
    }
    if (_welcoming) {
      return WelcomeScreenFrb(
        onDone: () {
          if (mounted) setState(() => _welcoming = false);
        },
      );
    }
    return const LumitAppView();
  }
}

class LumitAppView extends StatefulWidget {
  const LumitAppView({super.key});

  @override
  State<LumitAppView> createState() => _LumitAppViewState();
}

class _LumitAppViewState extends State<LumitAppView> {
  @override
  void initState() {
    super.initState();
    // Shortcuts are handled GLOBALLY, not through the focus tree. Every menu,
    // popup and palette lives in the Overlay outside this view's scope, so
    // any of them could walk focus away and never bring it back — and every
    // shortcut died until something was clicked (the space bar's recurring
    // funeral). A hardware-keyboard handler fires wherever focus is; the
    // focused-text-field guard inside _onKey keeps typing safe.
    HardwareKeyboard.instance.addHandler(_handleKey);
    // The pointer is tracked the same way — globally, not through the widget
    // tree. The Ctrl+Space console opens its ring at the mouse (K-325), and a
    // key event carries no position; a widget `Listener` missed everywhere no
    // widget claims the hit (the Viewer's texture, above all), so the console
    // kept opening at wherever the pointer had last crossed a panel. A global
    // route sees every pointer event regardless. One field write per event —
    // no setState, no bridge.
    GestureBinding.instance.pointerRouter.addGlobalRoute(_trackPointer);
    // A Lumit document copied while this window was away — in another Lumit
    // window, most of all — is picked up when the window comes back (K-302), so
    // Paste is live rather than greyed over something that is genuinely there.
    _clipboardWatch = AppLifecycleListener(
      onShow: () => context.read<LumitUiState>().adoptSystemClipboard(),
      onRestart: () => context.read<LumitUiState>().adoptSystemClipboard(),
    );
    // The first-run question (K-246), after the first frame so there is an
    // Overlay to put it in. It asks nothing on any later launch.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      final ui = context.read<LumitUiState>();
      // The update check follows the question rather than racing it: the
      // setup screen is where somebody may have just switched it off (K-296).
      maybeShowFirstRunFrb(context, ui.workspace)
          .then((_) => ui.maybeCheckForUpdates());
    });
  }

  AppLifecycleListener? _clipboardWatch;

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_handleKey);
    GestureBinding.instance.pointerRouter.removeGlobalRoute(_trackPointer);
    _clipboardWatch?.dispose();
    super.dispose();
  }

  void _trackPointer(PointerEvent event) {
    if (event is PointerHoverEvent ||
        event is PointerMoveEvent ||
        event is PointerDownEvent) {
      lastKnownPointerPosition = event.position;
    }
  }

  bool _handleKey(KeyEvent event) {
    if (!mounted) return false;
    // The overlay swallows the pointer; keys are routed globally rather than
    // through the tree, so they have to be swallowed here. A command aimed at
    // the document being replaced has nothing left to run against, and one
    // aimed at a document being worked on would race the job doing it.
    final app = context.read<LumitState>();
    if (app.opening.value || app.busy.value != null) return true;
    return _onKey(
            context.read<LumitState>(), context.read<LumitUiState>(), event) ==
        KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    var uiState = context.watch<LumitUiState>();
    final state = context.watch<LumitState>();

    // The scope stays for the text fields: when one gives focus up, focus
    // falls back to the enclosing scope rather than to nothing.
    return FocusScope(
      autofocus: true,
      child: Stack(children: [
        _shell(uiState, state),
        // Over everything while a document is being read (see OpeningOverlay):
        // the shell behind it is still the previous project and swaps in one go.
        ValueListenableBuilder<bool>(
          valueListenable: state.opening,
          builder: (context, opening, _) => opening
              ? ValueListenableBuilder<OpenProgress?>(
                  valueListenable: state.openProgress,
                  // Null is the sweep: an import says nothing about how far it
                  // has got, and opening a `.lum` says everything (K-628).
                  builder: (context, progress, _) => OpeningOverlay(
                    label: progress == null
                        ? null
                        : openPhaseLabel(progress.phase),
                    fraction: progress?.fraction,
                  ),
                )
              : const SizedBox.shrink(),
        ),
        // The same card for a job working on the document that is already open
        // — beat detection. The two never overlap: nothing can be started
        // against a document that is still being read.
        BusyOverlay(busy: state.busy),
      ]),
    );
  }

  Widget _shell(LumitUiState uiState, LumitState state) => Column(
        children: [
          LumitMenuBarFrb(app: state),
          // The tools, under the menu and above everything else — where a
          // toolbar goes, and where docs/07 §1.7 puts it.
          const LumitToolBarFrb(),
          Expanded(
            child: DockWidget(
              root: uiState.split,
              buildPanel: (context, panel) => buildPanelBodyFrb(context, panel),
              // Persisted, so an arrangement survives a restart.
              onLayoutChanged: uiState.saveLayout,
              activePanel: uiState.activePanel,
            ),
          ),
          // The strip under the dock (docs/07 §1): the running export's
          // progress and Cancel, reachable without the dialogue open.
          const StatusLineFrb(),
        ],
      );

  /// Which keymap context the focused panel is. Panels with no bindings of
  /// their own resolve to `Global`, which is also the fallback for every other
  /// context, so nothing is lost by the mapping being partial.
  BridgeKeyContext _contextOf(Panel? panel) => switch (panel) {
        Panel.project => BridgeKeyContext.project,
        Panel.viewer => BridgeKeyContext.viewer,
        Panel.timeline => BridgeKeyContext.timeline,
        Panel.effectControls => BridgeKeyContext.effects,
        _ => BridgeKeyContext.global,
      };

  /// The keyboard shortcuts, restored from the shell the port replaced.
  ///
  /// Only the ones whose engine calls exist on this bridge; the rest are on the
  /// menus. A field with focus is left alone, or every letter typed into a
  /// layer name would also be a command.
  KeyEventResult _onKey(LumitState state, LumitUiState ui, KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    // A modal surface is up — a dialogue, or the FX console (K-328): its keys
    // are its own, exactly as the panels' handlers already treat it (K-243).
    // Without this, a keystroke aimed at the console's search box also ran
    // whatever shell command it happened to spell.
    if (lumitModalOpen) return KeyEventResult.ignored;
    // A field with focus keeps its keys, or typing a layer name would also run
    // commands. The focused context's own widget is the `Focus` that
    // `EditableText` builds, not the `EditableText` — so the check has to look
    // up the tree, which is what the previous shell's version missed.
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return KeyEventResult.ignored;
    }
    // A focused house control (a dialog's OK button, a tabbed-to checkbox)
    // keeps its keys the same way a text field does: Enter or Space there
    // presses the control, and must not also run a panel command underneath
    // it (K-319).
    if (FocusManager.instance.primaryFocus is ControlFocusNode) {
      return KeyEventResult.ignored;
    }

    final project = state.project;
    final comp = ui.selectedComp;

    // Which action this chord means is the engine's answer, not a ladder of key
    // comparisons here (K-199). Asked in the *focused panel's* context, because
    // a binding scoped to one panel has to beat the app-wide one while that
    // panel is active — the engine falls back to Global itself, so one call
    // answers both. (This handler runs wherever focus is; the active panel is
    // what the dock last fronted, which is what a user would call "where I am".)
    var action = ui.keymap.actionFor(_contextOf(ui.activePanel.value), event);
    if (action == null) {
      // The Tools context is a context no panel *is* (docs/07 §15 scopes it to
      // the toolbar, not to a pane), so it is asked for separately and only
      // once the focused panel and the app-wide table have both declined.
      // That ordering is what keeps a panel free to claim a letter a tool also
      // uses — `C` cuts a clip in the Timeline and arms the razor everywhere
      // else — without either binding having to know about the other.
      final tool = ui.keymap.actionFor(BridgeKeyContext.tools, event);
      if (tool != null && ui.tools.handleAction(tool)) {
        return KeyEventResult.handled;
      }
      // **Panels** is the other one, and for the same reason: its three
      // bindings are about moving *between* panels, so scoping them to one
      // would make them unreachable from every other. Asked last, so a panel
      // that binds `Ctrl+F` for itself one day would still win where it is
      // focused. The engine falls back to Global from here too, which the
      // first lookup has already covered, so nothing can be dispatched twice.
      action = ui.keymap.actionFor(BridgeKeyContext.panels, event);
      if (action == null) return KeyEventResult.ignored;
    }
    // A tool action can also arrive from the primary lookup, if someone rebinds
    // one into a context a panel is. Same handler either way.
    if (ui.tools.handleAction(action)) return KeyEventResult.handled;

    var handled = true;
    switch (action) {
      case 'edit.redo':
        project?.redo();
        state.notifyDocumentChanged();
      case 'edit.undo':
        project?.undo();
        state.notifyDocumentChanged();
      case 'playback.toggle':
        ui.requestTogglePlay();
      // Shuttle is not built, and J/L have always stepped a frame here. Mapping
      // them onto the step keeps today's keyboard exactly as it is rather than
      // taking two keys away until a shuttle exists to give them back.
      case 'playback.frame.prev' || 'playback.shuttle.reverse':
        ui.stepFrame(-1);
      case 'playback.frame.next' || 'playback.shuttle.forward':
        ui.stepFrame(1);
      case 'playback.comp.start':
        ui.playheadFrame.value = 0;
      case 'playback.comp.end':
        final last = (comp?.durationFrames() ?? 1) - 1;
        ui.playheadFrame.value = last < 0 ? 0 : last;
      case 'layer.retime.enable':
        // Give the selected layer a Retime, or take it away again (docs/04
        // §12). On installs the identity map, so the picture does not move —
        // it just gains a row above Transform to key. Ctrl+Alt+T by default
        // (K-200): AE's own Time Remap chord, and one Windows cannot steal.
        // The Composition menu carries the command too (K-198's lesson).
        final layer = ui.selectedLayer.value;
        if (layer == null) {
          handled = false;
        } else {
          state.toggleRetime(layer);
        }
      // The Viewer's own magnification and preview resolution (docs/07 §2.2,
      // §15). Both are asked for rather than done here: the magnification
      // belongs to the Viewer panel, and the resolution is a number every
      // render request already carries.
      case 'viewer.zoom.in':
        ui.requestViewerZoom(ViewerZoomCommand.zoomIn);
      case 'viewer.zoom.out':
        ui.requestViewerZoom(ViewerZoomCommand.zoomOut);
      case 'viewer.zoom.fit':
        ui.requestViewerZoom(ViewerZoomCommand.fit);
      case 'viewer.res.full':
        ui.setPreviewResolution(PreviewResolution.full);
      case 'viewer.res.half':
        ui.setPreviewResolution(PreviewResolution.half);
      case 'viewer.res.quarter':
        ui.setPreviewResolution(PreviewResolution.quarter);
      // Moving between panels without the mouse (docs/07 §15, "Panels").
      case 'panel.focus.next':
        handled = ui.cyclePanelFocus(1);
      case 'panel.focus.prev':
        handled = ui.cyclePanelFocus(-1);
      case 'panel.search.focus':
        handled = ui.requestPanelSearch();
      case 'console.open':
        // The menu bar owns the console's lists too, so the key asks for it
        // rather than assembling a second one (K-324).
        ui.requestConsole();
      case 'palette.open':
        // The menu bar owns the palette's list of commands, so the key asks
        // for it rather than assembling a second one (docs/07 §12).
        ui.requestPalette();
      case 'layer.duplicate':
        final layer = ui.selectedLayer.value;
        if (layer == null) {
          handled = false;
        } else {
          layer.duplicate();
          state.notifyDocumentChanged();
        }
      case 'layer.precompose':
        // Ctrl+Shift+C asks before it packs (docs/07 §13.4): the dialogue is
        // where the two questions live, and the engine call is one line of it.
        final layers = ui.selectedLayers.value;
        if (comp == null || layers.isEmpty) {
          handled = false;
        } else {
          showPrecomposeDialogFrb(
            context: context,
            comp: comp,
            selectedLayers: layers,
            ui: ui,
            workspace: ui.workspace,
          );
        }
      // The rest of the menu bar's own commands (K-244). Each calls the very
      // function its menu row calls, so there is one implementation of "open a
      // project" rather than a keyboard's copy of one.
      case 'file.new':
        state.newProject();
      case 'file.open':
        openProjectFrb(state);
      case 'file.save.as':
        saveProjectFrb(state, ui, forcePicker: true);
      case 'file.import':
        importFootageFrb(state);
      case 'file.export':
        if (comp == null) {
          handled = false;
        } else {
          exportFrb(context);
        }
      case 'comp.new':
        if (project == null) {
          handled = false;
        } else {
          newCompositionFrb(context, state);
        }
      // Cut, copy and paste (K-300). The same three functions the Edit menu's
      // rows call — the chords had no handler at all before, which is why
      // `Ctrl+C` on a selected layer did nothing while the menu row worked.
      case 'edit.copy':
        handled = copySelectionFrb(ui);
      case 'edit.cut':
        handled = cutSelectionFrb(state, ui);
      case 'edit.paste':
        // Reading the system clipboard is asynchronous, so the chord is taken
        // and the paste lands a frame later rather than being declined here.
        pasteSelectionFrb(state, ui, comp, ui.selectedLayer.value);
      case 'edit.select.all':
        // The focused panel answers first (K-522): "everything" is the items
        // in the Project panel, the effects on the layer, the nodes in the
        // graph. Only where no panel claims it does it mean every layer.
        if (ui.requestSelectAll()) {
          handled = true;
        } else if (comp == null) {
          handled = false;
        } else {
          ui.setSelection(comp.getLayers());
        }
      case 'edit.deselect.all':
        ui.clearSelection();
      case 'app.settings':
        showSettingsWindowFrb(context);
      case 'project.settings':
        if (project == null) {
          handled = false;
        } else {
          showProjectSettingsFrb(context, project);
        }
      case 'file.save':
        // Ctrl+S goes through exactly the same call the File menu's Save does
        // (K-203) — a shortcut with its own path to disk is a second save to
        // keep honest. Without a path yet it opens the picker, which is what
        // Save has always meant on a document that has never been written.
        saveProjectFrb(state, ui);
      // The work area is the span the Viewer previews and the export writes
      // (K-037), so setting its ends from the playhead is a two-key job, not a
      // trip to a menu. A comp that has never had one set reads as the whole
      // comp, so B and N always have something to move.
      case 'workarea.set.start' || 'workarea.set.end':
        if (comp == null) {
          handled = false;
        } else {
          comp.setWorkArea(
            span: workAreaWith(
              comp: comp,
              current: comp.getWorkArea(),
              wanted: ui.playheadFrame.value,
              isStart: action == 'workarea.set.start',
            ),
          );
          state.notifyDocumentChanged();
        }
      // Markers (K-254). `Shift+M` (or AE's numpad `*`) drops a plain cue at
      // the playhead; `Ctrl`+digit sets the numbered one and the bare digit
      // returns to it. The numbered pair is the whole point — the key that
      // marks a moment is the key that goes back to it.
      case 'marker.add':
        if (comp == null) {
          handled = false;
        } else {
          addMarkerFrb(comp, frame: ui.playheadFrame.value);
          state.notifyDocumentChanged();
        }
      case final id when id.startsWith('marker.add.'):
        if (comp == null) {
          handled = false;
        } else {
          addMarkerFrb(
            comp,
            frame: ui.playheadFrame.value,
            label: id.substring('marker.add.'.length),
          );
          state.notifyDocumentChanged();
        }
      // `Alt+Shift+1…9` switches workspace by its position on the strip
      // (docs/07 §15) — the shipped presets first, then the user's own. A slot
      // past the end of the strip is left unhandled: it is a key that has not
      // been given a meaning yet, not a failure to report.
      case final id when id.startsWith('workspace.switch.'):
        final slot = int.tryParse(id.substring('workspace.switch.'.length));
        handled = slot != null && ui.workspace.switchToWorkspaceSlot(slot);
      case final id when id.startsWith('marker.goto.'):
        // Nothing bound to that digit yet is not a failure to report — it is a
        // key that has not been given a meaning. Left unhandled so it still
        // reaches whatever else wants it.
        final at = comp == null
            ? null
            : markerFrameFrb(comp, id.substring('marker.goto.'.length));
        if (at == null) {
          handled = false;
        } else {
          // Through the scrub, so a digit pressed mid-playback lands where it
          // says rather than being overwritten by the next frame that arrives.
          ui.scrubTo(at);
        }
      case 'edit.delete.selection':
        // A panel holding a finer selection than the layer one gets the key
        // first (K-234) — a selected mask row is what Delete is about, not the
        // layer under it.
        if (ui.deleteClaim?.call() ?? false) {
          break;
        }
        // The whole selection, not just the primary (K-217): with several
        // layers boxed in the Viewer, Delete taking one of them would be a
        // surprise every time.
        final layers = ui.selectedLayers.value;
        if (layers.isEmpty) {
          handled = false;
        } else {
          for (final layer in layers) {
            layer.delete();
          }
          ui.clearSelection();
          state.notifyDocumentChanged();
        }
      // A bound action this shell has no call for yet — the menus carry those.
      // Ignored rather than swallowed, so the key still reaches whatever else
      // wants it.
      default:
        handled = false;
    }
    return handled ? KeyEventResult.handled : KeyEventResult.ignored;
  }
}
