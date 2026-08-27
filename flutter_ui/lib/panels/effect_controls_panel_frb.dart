// The Effect controls panel, on the flutter_rust_bridge API — the effect stack.
//
// **The shape of it.** One twirl-open section per effect, read the same way the
// Timeline's fold-out reads: a heading bar carrying the effect's enable switch
// and its name, then a row per declared parameter under it, each row separated
// from the next by a hairline. Every row is two columns — the parameter's name
// left-aligned in a fixed-width name column, its control left-aligned in the
// rest — with nothing drawn between them; they read as columns because they line
// up, which is all a column is (`fx_section.dart`). The heading's Reset sits at
// the top of the value column because that is what it acts on; the close mark
// stays hard right, away from it.
//
// Add effect offers every built-in, grouped by category. Above the stack sit the
// Transform rows — anchor, position, scale, rotation, opacity, plus the z and
// x/y-rotation rows when the layer is 3D — in a section of the same shape.
//
// **Effects that want their own display** are the exception this layout
// expects. [customEffectDisplay] is asked for a widget to draw *above* the
// rows, and Levels claims it: a histogram of the frame with its input handles
// over it and the output range beneath (K-413). The rows themselves are
// unchanged — the display writes the same values through the same callbacks.
// Curves' spline is not a display but a **fold**: its five channel curves are
// five declared parameters folded into one tabbed editor, the way an `_x`/`_y`
// pair folds into one point row (K-412).
//
// Every animatable row carries the stopwatch and the ◄ ◆ ► navigator
// (keyframe_controls_frb.dart). An animated row shows "animated" in place of its
// number field: the value there is a curve, and the graph editor is where a
// curve is shaped. The stopwatch turns animation off again, keeping the value
// the curve reads at the playhead — so the row is never a dead end.
//
// **How an edit reaches the document.** `getEffects` hands back a *staged* copy
// of the stack, `setValue` edits that copy, and `LayerReference.setEffects`
// commits the whole list as one `SetLayerEffects` op. So a drag mutates the copy
// and renders it through `renderFrameWithPreview` — which patches a clone of the
// document engine-side — and only the release commits. A drag therefore costs
// one undo entry rather than one per tick, which is the whole reason the staged
// shape exists.

import 'dart:async';

import 'package:flutter/foundation.dart' show mapEquals;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/track.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../widgets/controls.dart';
import '../widgets/curve_editor.dart';
import 'effect_param_row_frb.dart';
import 'graph_panel.dart' show graphNodeKey, graphNoStream;
import 'camera_track_display_frb.dart';
import 'planar_track_display_frb.dart';
import 'levels_display_frb.dart';
import 'fx_section.dart';
import 'transform_rows_frb.dart';
import '../state/clipboard.dart';
import '../theme/theme.dart';
import '../state/drag_payloads.dart';
import 'placeholder.dart';
import 'flow_rows_frb.dart';
import 'source_rows_frb.dart';
import 'text_animator_rows_frb.dart';
import 'timeline_timings.dart';

class EffectControlsPanelFrb extends StatefulWidget {
  const EffectControlsPanelFrb({super.key});

  @override
  State<EffectControlsPanelFrb> createState() => _EffectControlsPanelFrbState();
}

class _EffectControlsPanelFrbState extends State<EffectControlsPanelFrb> {
  /// The drag in progress, and the writes that end it. Shared with the
  /// Timeline's fold-out, which shows the same rows.
  final EffectStackEditor _effects = EffectStackEditor();

  /// Which sections are twirled shut, by their path. Held closed-set rather than
  /// open-set so a newly applied effect arrives open, which is what you want the
  /// moment after applying one.
  final Set<String> _shut = <String>{};

  /// The last layer this panel drew. Deselecting does not empty the panel: the
  /// stack you were just editing stays up, because clicking away in the
  /// Timeline is not a request to lose your place. It is replaced the moment
  /// another layer is selected, and falls back to the placeholder only if that
  /// layer is **gone** — deleted out of the composition it was in.
  LayerReference? _lastLayer;

  /// That layer's row of the read model, and the composition it was read
  /// from — what keeps its stack on the panel when another comp is fronted
  /// (item 6.28).
  ///
  /// Fronting a comp rebinds the model to *its* layers, so the layer this
  /// panel is showing stops being in it while still existing perfectly well
  /// in the comp it belongs to. That used to blank the panel: you stepped
  /// into a pre-comp to look at something and came back to a placeholder,
  /// having lost the stack you were part way through editing. Held here, the
  /// rows stay up until a layer is selected in the new comp — and a layer
  /// missing from the model of its *own* comp is one that has genuinely gone,
  /// which is still the placeholder. No call crosses the bridge for any of it.
  BridgeLayerEntry? _heldEntry;
  UuidValue? _heldComp;

  bool _isOpen(String path) => !_shut.contains(path);
  void _toggle(String path) => setState(() {
        if (!_shut.remove(path)) _shut.add(path);
      });

  /// Twirl one effect — and, when it is one of the **picked** run, all of them
  /// together (item 6.3).
  ///
  /// A selection is a set of things you are treating as one: opening five
  /// stacks one twirl at a time, having just said all five are what you are
  /// working on, is five clicks to reach a state you already asked for. The
  /// clicked effect's new state is what the run takes — never a flip apiece,
  /// which would leave a mixed run mixed the other way round.
  void _toggleEffect(UuidValue id, List<UuidValue> picked) {
    if (!picked.contains(id)) return _toggle('fx-$id');
    final opening = _shut.contains('fx-$id');
    setState(() {
      for (final other in picked) {
        if (opening) {
          _shut.remove('fx-$other');
        } else {
          _shut.add('fx-$other');
        }
      }
    });
  }

  /// The effect whose heading is an inline rename editor, or null (K-321).
  UuidValue? _renamingEffect;

  /// How many Action buttons have been pressed in this panel's life (K-417).
  ///
  /// A press changes nothing in the document — it is an event, not an edit —
  /// so there is no revision for a status line to compare against. This number
  /// is what an effect's own display watches to know a button was pushed.
  int _actionPressed = 0;

  /// Which parameters a driver is wired to (K-471), by `effectId/paramId`,
  /// with the driver's own name and what its wire carries.
  ///
  /// Read from `getGraph` at the two moments the graph can change — the
  /// selection moves, or the document commits — and held, never asked for in a
  /// rebuild: this panel redraws on every playhead frame, and a read in that
  /// path is exactly the traffic `bridge_call_budget_test` guards against.
  /// Empty for every layer that has never been wired, which is nearly all of
  /// them, and empty costs one call that answers "no drivers".
  Map<String, ({String driver, BridgePortType type, bool noStream})> _driven =
      const {};
  LumitUiState? _boundUi;

  @override
  void initState() {
    super.initState();
    // `Enter` renames the selected effect (K-321) — registered on the
    // hardware keyboard like every panel command; stands down for modals,
    // focused fields, and whenever this panel is not the active one.
    HardwareKeyboard.instance.addHandler(_onKey);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (identical(ui, _boundUi)) return;
    _unbindDriven();
    _boundUi = ui;
    ui.selectedLayer.addListener(_readDriven);
    ui.model.addListener(_readDriven);
    // `Ctrl+A` here means every effect on the layer (K-522), not every layer
    // in the composition — the shell routes the chord to the focused panel.
    ui.selectAllRequest.addListener(_onSelectAllRequested);
    // **Delete means the picked effects while this panel is the focused one**
    // (item 6.6, K-234's mechanism). Claimed rather than handled on the
    // keyboard: every hardware-keyboard handler runs on every key, so
    // answering the chord here would remove the effects *and* let the shell
    // remove the layer under them. The shell asks the claim first and stands
    // down when it says yes.
    //
    // Chained onto whatever held the claim before — the Timeline, for its
    // mask rows — because there is one claim and two panels that want it.
    // Ours answers only while this panel is focused and effects are picked;
    // anything else falls through to the claim it displaced.
    if (ui.deleteClaim != _deleteClaim) _priorDeleteClaim = ui.deleteClaim;
    ui.deleteClaim = _deleteClaim;
    // Copy and Paste are claimed the same way, and for the same reason
    // (K-300). They were answered on the keyboard instead for a while, which
    // took the chord here *and* left the shell's own Paste to run: one press
    // put the effects on the layer twice.
    if (ui.copyClaim != _copyClaim) _priorCopyClaim = ui.copyClaim;
    ui.copyClaim = _copyClaim;
    if (ui.pasteClaim != _pasteClaim) _priorPasteClaim = ui.pasteClaim;
    ui.pasteClaim = _pasteClaim;
    _readDriven();
  }

  /// The claims this panel had to displace to take the editing chords.
  bool Function()? _priorDeleteClaim;
  bool Function()? _priorCopyClaim;
  bool Function()? _priorPasteClaim;

  void _unbindDriven() {
    _boundUi?.selectedLayer.removeListener(_readDriven);
    _boundUi?.model.removeListener(_readDriven);
    _boundUi?.selectAllRequest.removeListener(_onSelectAllRequested);
    if (_boundUi?.deleteClaim == _deleteClaim) {
      _boundUi!.deleteClaim = _priorDeleteClaim;
    }
    if (_boundUi?.copyClaim == _copyClaim) {
      _boundUi!.copyClaim = _priorCopyClaim;
    }
    if (_boundUi?.pasteClaim == _pasteClaim) {
      _boundUi!.pasteClaim = _priorPasteClaim;
    }
  }

  bool _copyClaim() {
    final ui = _boundUi;
    if (!mounted ||
        ui == null ||
        ui.activePanel.value != Panel.effectControls) {
      return _priorCopyClaim?.call() ?? false;
    }
    return _copyPickedEffects(ui) || (_priorCopyClaim?.call() ?? false);
  }

  bool _pasteClaim() {
    final ui = _boundUi;
    if (!mounted ||
        ui == null ||
        ui.activePanel.value != Panel.effectControls) {
      return _priorPasteClaim?.call() ?? false;
    }
    return _pastePickedEffects(ui) || (_priorPasteClaim?.call() ?? false);
  }

  bool _deleteClaim() {
    final ui = _boundUi;
    if (!mounted ||
        ui == null ||
        ui.activePanel.value != Panel.effectControls) {
      return _priorDeleteClaim?.call() ?? false;
    }
    return _deletePickedEffects(ui) || (_priorDeleteClaim?.call() ?? false);
  }

  /// Remove every picked effect from the layer this panel is showing.
  ///
  /// Each removal is one `SetLayerEffects` — the stack written whole, which is
  /// the only shape an effect edit has — so a run of three comes back in three
  /// undo steps rather than leaving half a stack behind on the first one. The
  /// handles are read together and are safe to hold: the seam matches them by
  /// id, so removing one does not stale the next.
  ///
  /// Answers whether it took the key. Nothing picked is not this panel's
  /// Delete, and the layer selection is what the shell falls back to.
  bool _deletePickedEffects(LumitUiState ui) {
    final layer =
        ui.selectedEffectsLayer ?? ui.selectedLayer.value ?? _lastLayer;
    final picked = ui.selectedEffects.value.toSet();
    if (layer == null || picked.isEmpty) return false;
    try {
      for (final instance in layer.getEffects()) {
        if (picked.contains(instance.getInfo().id)) {
          layer.removeEffect(effect: instance);
        }
      }
    } catch (_) {
      // The effects went away under the selection; there is nothing here to
      // report and nothing left to remove.
    }
    ui.clearEffectSelection();
    Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
    ui.model.refresh();
    return true;
  }

  /// Pick the whole effect stack of the layer this panel is showing.
  void _onSelectAllRequested() {
    final ui = _boundUi;
    if (!mounted || ui == null) return;
    if (!ui.selectAllRequestIsFor(Panel.effectControls)) return;
    final layer = ui.selectedLayer.value ?? _lastLayer;
    if (layer == null) return;
    final info = ui.model.byId(layer.internallayerId)?.info;
    if (info == null || info.effects.isEmpty) return;
    ui.setEffectSelection(layer, [for (final e in info.effects) e.id]);
  }

  /// The one read behind the *driven* rows. A wire's colour is its **source**
  /// port's type, which is what the parameter is now following.
  void _readDriven() {
    if (!mounted) return;
    final layer = _boundUi?.selectedLayer.value ?? _lastLayer;
    if (layer == null) {
      if (_driven.isNotEmpty) setState(() => _driven = const {});
      return;
    }
    final next =
        <String, ({String driver, BridgePortType type, bool noStream})>{};
    try {
      final graph = layer.getGraph();
      final byRef = {for (final n in graph.nodes) graphNodeKey(n.node): n};
      for (final edge in graph.wiring.edges) {
        if (edge.to case BridgeInputRef_Param(:final node, :final port)) {
          if (node is! BridgeNodeRef_Effect) continue;
          final (fromKey, fromPort) = switch (edge.from) {
            BridgeOutputRef_Driver(node: final d, port: final p) => (
                graphNodeKey(BridgeNodeRef.driver(d)),
                p
              ),
            BridgeOutputRef_SourceMatte() => ('source', 'matte'),
            // A points wire's source is a *stack effect* (K-492), so the
            // row names the effect that hands the data over.
            BridgeOutputRef_EffectData(:final effect, :final port) => (
                graphNodeKey(BridgeNodeRef.effect(effect)),
                port
              ),
          };
          final source = byRef[fromKey];
          if (source == null) continue;
          final socket = source.outputs.where((o) => o.id == fromPort);
          if (socket.isEmpty) continue;
          next['${node.field0}/$port'] = (
            driver: source.customName ?? engineLabel(source.label),
            type: socket.first.portType,
            noStream: graphNoStream(source),
          );
        }
      }
    } catch (_) {
      // The layer has gone; the rows simply draw their own controls again.
    }
    if (!mapEquals(next, _driven)) setState(() => _driven = next);
  }

  @override
  void dispose() {
    _unbindDriven();
    HardwareKeyboard.instance.removeHandler(_onKey);
    super.dispose();
  }

  bool _onKey(KeyEvent event) {
    if (event is! KeyDownEvent || !mounted) return false;
    if (lumitModalOpen) return false;
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return false;
    }
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (ui.activePanel.value != Panel.effectControls) return false;
    final action = ui.keymap.actionFor(BridgeKeyContext.effects, event);
    if (action == 'effect.rename') {
      final picked = ui.selectedEffects.value;
      if (picked.length != 1 || _renamingEffect != null) return false;
      setState(() => _renamingEffect = picked.first);
      return true;
    }
    // Copy, Paste and Delete are **claimed**, not answered here: see
    // `didChangeDependencies`.
    return false;
  }

  /// **`Ctrl+C` on this panel copies the picked effects** (K-275, K-300).
  ///
  /// The chord had no handler here at all, so it went to the shell — where
  /// `copySelectionFrb` offers it first to whichever panel has *claimed* copy
  /// (the Timeline, for keyframes) and only then looks at the effect
  /// selection. With a property row picked in the Timeline the claim answers
  /// yes, the keyframe clipboard takes the chord, and `Ctrl+C` on an effect
  /// heading in a panel the user is looking at quietly copies something else
  /// — which is a copy that "does nothing", because the paste that follows
  /// puts keyframes back rather than the effect.
  ///
  /// A claim on the chord settles it: while this panel is the active one and
  /// an effect is picked in it, the effect is what the chord means. Nothing is
  /// claimed otherwise — with no effect picked the chord falls through to the
  /// claim this one displaced, and then to the shell, which copies the layer
  /// as it always did.
  bool _copyPickedEffects(LumitUiState ui) {
    final layer = ui.selectedEffectsLayer;
    final picked = ui.selectedEffects.value;
    if (layer == null || picked.isEmpty) return false;
    try {
      ui.copyEffectsToClipboard(layer.copyEffects(effects: picked));
    } catch (_) {
      // The effects went away under the selection; the clipboard keeps what
      // it had, and the layer below is not a silent substitute.
      return false;
    }
    return true;
  }

  /// **`Ctrl+V` puts them on the layer this panel is showing** — the same one
  /// its rows are for, which is not always `selectedLayer` (deselecting keeps
  /// the last stack up, deliberately).
  ///
  /// Claimed only when the tray actually holds effects. A layer on the
  /// clipboard is the shell's business, and an empty tray is too: the shell's
  /// paste reads the *system* clipboard for a document copied in another Lumit
  /// window (K-302), which is asynchronous and has no place in a claim that
  /// must answer yes or no on the spot.
  bool _pastePickedEffects(LumitUiState ui) {
    if (ui.clipboard.kind != ClipboardKind.effects) return false;
    final text = ui.clipboard.text;
    final layer = ui.selectedLayer.value ?? _lastLayer;
    if (text == null || layer == null) return false;
    try {
      layer.pasteEffects(text: text, atFrame: ui.playheadFrame.value);
    } catch (_) {
      // Not a stack this layer can take, or the layer has gone.
      return false;
    }
    Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
    ui.model.refresh();
    return true;
  }

  /// Which parameter twirls the owner has opened or shut, by path.
  ///
  /// A map rather than the closed-set [_shut] because a group carries its own
  /// default: most arrive collapsed (they hold the advanced controls), so
  /// "absent" cannot mean open here the way it does for a section. Absent means
  /// "whatever the schema said", and the entry appears the first time the owner
  /// disagrees.
  final Map<String, bool> _groupOpen = <String, bool>{};

  bool _isGroupOpen(String path, bool collapsedByDefault) =>
      _groupOpen[path] ?? !collapsedByDefault;

  void _toggleGroup(String path, bool collapsedByDefault) => setState(() {
        _groupOpen[path] = !_isGroupOpen(path, collapsedByDefault);
      });

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context);
    final comp = ui.selectedComp;
    if (comp == null) {
      return PlaceholderPanel(
        icon: LumitIcon.fx,
        title: l10n.effectControls,
        hint: l10n.effectControlsNoComp,
      );
    }

    return ValueListenableBuilder<LayerReference?>(
      valueListenable: ui.selectedLayer,
      builder: (context, layer, _) {
        if (layer != null) _lastLayer = layer;
        final shown = layer ?? _lastLayer;
        if (shown == null) {
          return PlaceholderPanel(
            icon: LumitIcon.fx,
            title: l10n.effectControls,
            hint: l10n.effectControlsNoLayer,
          );
        }
        return _body(context, comp, shown);
      },
    );
  }

  Widget _body(
    BuildContext context,
    CompositionReference comp,
    LayerReference layer,
  ) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    // The keyframe controls read the playhead — which key is under it, whether
    // the diamond is filled — so the rows have to redraw when it moves. The
    // read model repaints the panel when anything commits (K-184): an undo, a
    // redo, or the same property dragged in the Timeline's fold-out.
    return ValueListenableBuilder<int>(
      valueListenable: ui.playheadFrame,
      // Which effects are picked is the shell's (K-300) — the Timeline picks
      // them too — so the headings redraw when that changes, wherever the click
      // happened.
      builder: (context, playhead, _) =>
          ValueListenableBuilder<List<UuidValue>>(
        valueListenable: ui.selectedEffects,
        builder: (context, picked, _) => ListenableBuilder(
          listenable: ui.model,
          builder: (context, _) =>
              _rows(context, comp, layer, playhead, picked),
        ),
      ),
    );
  }

  /// Every layer an add from this panel should land on (K-523): the whole
  /// selection when the layer these rows are for is part of it, and that layer
  /// alone when it is not.
  ///
  /// The second case is the one worth naming. This panel deliberately keeps the
  /// last stack up after a deselect, so the rows on screen are not always the
  /// rows of a *selected* layer — and an add made against a stack nobody has
  /// selected means the stack that is being looked at, not nothing.
  ///
  /// Read from the shell in the handler rather than in the build, the way the
  /// Timeline's row menu reads its own targets (K-184).
  List<LayerReference> _addTargets(LumitUiState ui, LayerReference shown) {
    final picked = ui.selectedLayers.value;
    return picked.any((l) => l.internallayerId == shown.internallayerId)
        ? picked
        : [shown];
  }

  Widget _rows(
    BuildContext context,
    CompositionReference comp,
    LayerReference layer,
    int playhead,
    List<UuidValue> picked,
  ) {
    final t = ThemeScope.of(context).theme;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    var entry = ui.model.byId(layer.internallayerId);
    if (entry == null) {
      // Not in the fronted comp's model. Another comp has been fronted and
      // this layer lives in the one before it, so its rows stay up (item
      // 6.28) — held from the last read, since the model no longer carries
      // them. Missing from its OWN comp's model is a layer that has gone.
      entry = _heldComp == comp.internalid ||
              _heldEntry?.layer.internallayerId != layer.internallayerId
          ? null
          : _heldEntry;
    } else {
      _heldEntry = entry;
      _heldComp = comp.internalid;
    }
    if (entry == null) {
      return PlaceholderPanel(
        icon: LumitIcon.fx,
        title: l10n.effectControls,
        hint: l10n.effectControlsNoLayer,
      );
    }
    final info = entry.info;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _Header(
          layerName: info.name,
          onAdd: (name) {
            for (final target in _addTargets(ui, layer)) {
              try {
                target.addEffect(name: name);
              } catch (_) {}
            }
            ui.model.refresh();
          },
        ),
        Expanded(
          // The drop target for an effect dragged from Effects & presets.
          // Nothing else produces an `EffectDragData`, and this is the only
          // thing that accepts one — the same contract `FootageDragData` has
          // with the Timeline.
          child: DragTarget<EffectDragData>(
            onAcceptWithDetails: (details) {
              for (final target in _addTargets(ui, layer)) {
                try {
                  target.addEffect(name: details.data.name);
                } catch (_) {}
              }
              ui.model.refresh();
            },
            builder: (context, candidate, _) => Container(
              // While something is over it, say so: a drop with no feedback is
              // indistinguishable from a drop that did nothing.
              decoration: candidate.isEmpty
                  ? null
                  : BoxDecoration(
                      border: Border.all(color: t.accent),
                      color: t.accent.withValues(alpha: 0.06),
                    ),
              // **A click on nothing clears the pick.** Every other surface
              // in the editor works this way — the Timeline's lane ground,
              // the Project panel's floor, the graph canvas — and an effect
              // left lit after a click somewhere else pointed the next
              // Delete or Copy at something nobody was looking at.
              //
              // Translucent, so it sits *under* everything the rows claim: a
              // heading, a control or a row that answers the tap wins the
              // arena and this never hears it. What is left is the empty
              // spot.
              child: GestureDetector(
                key: const ValueKey('fx-ground'),
                behavior: HitTestBehavior.translucent,
                onTap: ui.clearEffectSelection,
                child: ListView(
                  padding: const EdgeInsets.symmetric(vertical: 4),
                  children: [
                    // Source (a text layer's words, a solid's colour) and Retime
                    // ride with Transform behind the same choice: all three are
                    // the *layer*, and this panel is about the effects on it.
                    // Settings → Interface brings them back together.
                    if (ui.workspace.interface.transformInEffectControls) ...[
                      // What the layer is made of comes before where it sits: a
                      // text layer's words are the first thing you want when
                      // you select one.
                      SourceRowsFrb(
                        key:
                            ValueKey<String>('src-card-${layer.internallayerId}'),
                        layer: layer,
                        onChanged: ui.model.refresh,
                        open: _isOpen('source'),
                        onToggle: () => _toggle('source'),
                      ),
                      // Flow sits between what the layer is made of and where it
                      // sits, because that is what it is: how the source is
                      // *sampled* (K-088). It shows itself only when the layer's
                      // flow switch is on.
                      FlowRowsFrb(
                        key: ValueKey<String>(
                            'flow-card-${layer.internallayerId}'),
                        layer: layer,
                        onChanged: ui.model.refresh,
                        comp: comp,
                        playheadFrame: playhead,
                        onSeek: (frame) => ui.playheadFrame.value = frame,
                        open: _isOpen('flow'),
                        onToggle: () => _toggle('flow'),
                      ),
                      _TransformSection(
                        key: ValueKey<String>('tf-card-${layer.internallayerId}'),
                        layer: layer,
                        comp: comp,
                        transform: info.transform,
                        axisModes: info.axisModes,
                        // A camera is 3D by construction whatever its switch
                        // says (K-023) — its z and rotation rows must always
                        // draw. Decided here from the model the panel already
                        // holds, not by asking the engine per rebuild (K-184).
                        threeD: info.switches.threeD ||
                            info.kind == BridgeLayerKind.camera,
                        isCamera: info.kind == BridgeLayerKind.camera,
                        corrected: info.trackCorrected,
                        playheadFrame: playhead,
                        onSeek: (frame) => ui.playheadFrame.value = frame,
                        onChanged: ui.model.refresh,
                        open: _isOpen('transform'),
                        onToggle: () => _toggle('transform'),
                      ),
                    ],
                    // **The letters, one at a time** (K-609), and outside the
                    // choice above on purpose. Transform, Source and Retime move
                    // between this panel and the Timeline's fold because they
                    // exist in both; the Animators section has no Timeline home
                    // yet, so hiding it with them would hide the whole feature
                    // from anybody who has not turned the layer cards on. It
                    // shows itself only where there is something to show: a
                    // layer that is not a Text layer has no letters to animate.
                    TextAnimatorRowsFrb(
                      key: ValueKey<String>('anim-card-${layer.internallayerId}'),
                      layer: layer,
                      onChanged: ui.model.refresh,
                      comp: comp,
                      playheadFrame: playhead,
                      onSeek: (frame) => ui.playheadFrame.value = frame,
                      open: _isOpen('animators'),
                      onToggle: () => _toggle('animators'),
                    ),
                    // A null layer has no picture, so nothing here changes one
                    // — but the parameters are real, animatable values, which is
                    // exactly what a null is for once expressions can read them
                    // (K-274). Said plainly, once, rather than refusing the drop.
                    if (info.kind == BridgeLayerKind.nullLayer &&
                        info.effects.isNotEmpty)
                      Padding(
                        key: const ValueKey('fx-null-inert'),
                        padding: const EdgeInsets.symmetric(
                            horizontal: 8, vertical: 8),
                        child: Text(
                          'A null layer draws nothing, so an effect here changes '
                          'no picture. Its parameters stay live — a null is '
                          'where a control lives when it is meant to drive other '
                          'layers.',
                          style: t.small.copyWith(color: t.textMuted),
                        ),
                      ),
                    if (info.effects.isEmpty)
                      Padding(
                        padding: const EdgeInsets.symmetric(vertical: 18),
                        child: Text(
                          l10n.noEffectsYet,
                          style: t.small,
                          textAlign: TextAlign.center,
                        ),
                      )
                    else
                      for (var index = 0; index < info.effects.length; index++)
                        _EffectSection(
                          key: ValueKey<String>('fx-card-$index'),
                          info: info.effects[index],
                          open: _isOpen('fx-${info.effects[index].id}'),
                          onToggle: () =>
                              _toggleEffect(info.effects[index].id, picked),
                          selected: picked.contains(info.effects[index].id),
                          driven: _driven,
                          renaming: _renamingEffect == info.effects[index].id,
                          onRenamed: (name) {
                            // Stage the name on a fresh handle and commit the
                            // stack — one SetLayerEffects, one undo step, the
                            // same shape every stack edit has.
                            final stack = layer.getEffects();
                            for (final instance in stack) {
                              if (instance.id() == info.effects[index].id) {
                                instance.setCustomName(name: name);
                                try {
                                  layer.setEffects(effects: stack);
                                } catch (_) {
                                  // The stack changed under us; re-reading is
                                  // the recovery.
                                }
                                break;
                              }
                            }
                            setState(() => _renamingEffect = null);
                            ui.model.refresh();
                          },
                          // Escape: close the editor, write nothing (K-323).
                          onRenameCancelled: () =>
                              setState(() => _renamingEffect = null),
                          onStartRename: () => setState(
                              () => _renamingEffect = info.effects[index].id),
                          onSelect: () => ui.pickEffect(
                            layer,
                            info.effects[index].id,
                            order: [for (final e in info.effects) e.id],
                          ),
                          stagedValue: _effects.stagedValue,
                          trackCorrected: info.trackCorrected,
                          index: index,
                          count: info.effects.length,
                          onStackChanged: ui.model.refresh,
                          onWrite: (id, param, value) {
                            _effects.write(layer, id, param, value);
                            ui.model.refresh();
                          },
                          onWritePair: (id, values) {
                            _effects.writeAll(layer, id, values);
                            ui.model.refresh();
                          },
                          onLive: (id, param, value) => setState(() {
                            _effects.live(comp, layer, id, param, value,
                                frame: ui.playheadFrame.value,
                                scale: ui.viewerScale);
                          }),
                          layer: layer,
                          allLayers: ui.model.layers,
                          comp: comp,
                          playheadFrame: playhead,
                          onSeek: (frame) => ui.playheadFrame.value = frame,
                          isGroupOpen: _isGroupOpen,
                          onToggleGroup: _toggleGroup,
                          pressed: _actionPressed,
                          themedGraphs: ui.workspace.themedEffectGraphs,
                          curvePlotSize: ui.workspace.curvePlotSize,
                          onCurvePlotSize: ui.workspace.setCurvePlotSize,
                          onAction: (effect, param) {
                            try {
                              fireEffectAction(
                                  layer: layer, effect: effect, param: param);
                            } catch (_) {
                              // Refused — another analysis is already running,
                              // or the media cannot be read. The effect's own
                              // status line says which; a thrown error here
                              // would be a dialogue over a button press.
                            }
                            setState(() => _actionPressed += 1);
                          },
                        ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

/// The panel header: which layer is being edited, and Add effect.
class _Header extends StatelessWidget {
  final String layerName;
  final ValueChanged<String> onAdd;
  const _Header({required this.layerName, required this.onAdd});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      color: t.surface1,
      child: Row(
        children: [
          lumitIcon(LumitIcon.fx, size: iconSize, color: t.textMuted),
          const SizedBox(width: 8),
          Expanded(
            child: Text(layerName,
                style: t.bodyPrimary, overflow: TextOverflow.ellipsis),
          ),
          // Its own context, so the menu drops from the *button* rather than
          // from the header row's left edge — which is where it used to land.
          Builder(
            builder: (buttonContext) => HouseButton(
              key: const ValueKey('fx-add'),
              small: true,
              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
              onPressed: () => _showAddMenu(buttonContext, onAdd),
              // Add effect is a container label like every other kicker, so the
              // capitals live here rather than in the arb file.
              child: Text(l10n.addEffect.toUpperCase(),
                  style: t.kicker.copyWith(color: t.textSecondary)),
            ),
          ),
        ],
      ),
    );
  }
}

/// The Add-effect menu: one row per category, each opening onto its effects
/// (K-090, K-194 — Add effect → Stylise → Glow).
///
/// [context] is the *button's*, so the menu drops from it rather than from the
/// panel's left edge. The whole list used to be one 380 px scroller, which is
/// a lot of reading to find one effect.
Future<void> _showAddMenu(
    BuildContext context, ValueChanged<String> onAdd) async {
  final box = context.findRenderObject();
  if (box is! RenderBox) return;
  // Dropped from the button's left edge so a wide menu opens back across the
  // panel rather than off its right side.
  final origin = box.localToGlobal(Offset(0, box.size.height + 4));

  // Grouped in schema order, so the headings come out in the order the engine
  // declares rather than alphabetically by accident.
  final grouped = <String, List<BridgeEffectInfo>>{};
  final headings = <String, String>{};
  for (final e in listEffects()) {
    grouped.putIfAbsent(e.category, () => []).add(e);
    headings[e.category] = engineLabel(e.categoryLabel);
  }

  await showLumitPopup<void>(
    context: context,
    position: origin,
    builder: (close) => FloatSurface(
      width: 200,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final entry in grouped.entries)
            SubmenuRow(
              key: ValueKey<String>('fx-category-${entry.key}'),
              closeParent: () => close(null),
              submenu: (dismiss) => FloatSurface(
                width: 200,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    for (final effect in entry.value)
                      MenuRow(
                        onPressed: () {
                          dismiss();
                          onAdd(effect.name);
                        },
                        child: Text(engineLabel(effect.label)),
                      ),
                  ],
                ),
              ),
              child: Text(headings[entry.key] ?? entry.key),
            ),
        ],
      ),
    ),
  );
}

/// One effect: its heading row and a row per declared parameter.
///
/// Drawn entirely from the read model (K-184) — no bridge calls in build. The
/// heading-row ops need a live instance handle, which is fetched fresh at click
/// time (the model's data is not a handle, deliberately: frb consumes handles
/// passed by value).
class _EffectSection extends StatelessWidget {
  final BridgeEffectInstanceInfo info;
  final bool open;
  final VoidCallback onToggle;

  /// Picked out of the stack, and the click that picks it (K-300). The same
  /// selection the Timeline's fold-out shows, so an effect chosen in one place
  /// is lit in the other — and Copy takes it from either.
  final bool selected;
  final VoidCallback onSelect;

  /// Which of this layer's parameters a driver is wired to, by
  /// `effectId/paramId` (K-471). Read once by the panel and passed down, so a
  /// card costs no question of its own.
  final Map<String, ({String driver, BridgePortType type, bool noStream})>
      driven;

  /// The drag in flight's staged value for (effect, param), or null — overlaid
  /// on the model's value so the number under the pointer is the staged one.
  final BridgeEffectValue? Function(UuidValue effect, String param) stagedValue;
  final int index;
  final int count;
  final LayerReference layer;

  /// Every layer in the comp, from the read model — what a layer-valued
  /// parameter picks from (K-194).
  final List<BridgeLayerEntry> allLayers;
  final CompositionReference comp;
  final int playheadFrame;
  final ValueChanged<int> onSeek;

  /// The stack itself changed (enabled, reordered, removed) — re-read it.
  final VoidCallback onStackChanged;

  /// The heading is an inline rename editor (K-321), its commit, and the
  /// Escape that throws the edit away instead (K-323).
  final bool renaming;
  final ValueChanged<String>? onRenamed;
  final VoidCallback? onRenameCancelled;

  /// Open that editor — the heading menu's Rename. `Enter` on the selected
  /// effect already did this from the keyboard (`effect.rename`); this is the
  /// mouse's way in, and it is the same way in a project item has (K-321's own
  /// pairing, and the reason K-191 took renaming off the second click).
  final VoidCallback? onStartRename;

  /// Write a parameter — a typed value, or the release of a drag. One op.
  final void Function(UuidValue effect, String param, BridgeEffectValue value)
      onWrite;

  /// A drag tick: preview it, do not commit it.
  final void Function(UuidValue effect, String param, BridgeEffectValue value)
      onLive;

  /// Several parameters of one effect in one op — what a chained pair's
  /// proportional write commits through, so it costs one undo step (K-610).
  final void Function(UuidValue effect, Map<String, BridgeEffectValue> values)?
      onWritePair;

  /// Whether a parameter group's twirl is open, and toggling it. Held by the
  /// panel rather than here because this card is rebuilt from the read model on
  /// every change, and a fold that reset itself each time would be unusable.
  /// The schema's `collapsed` is the default until the owner touches it.
  final bool Function(String path, bool collapsedByDefault) isGroupOpen;
  final void Function(String path, bool collapsedByDefault) onToggleGroup;

  /// An Action row's press (K-417), and the panel's running count of them —
  /// what an effect's own display watches to know one happened.
  final void Function(UuidValue effect, String param) onAction;
  final int pressed;

  /// A camera following this layer's Camera track has been nudged (K-578) —
  /// drawn as a dot on the status row. From the read model, like everything
  /// else this card draws.
  final bool trackCorrected;

  /// Settings → "Use theme colours in effect graphs" (owner, desk test). Off
  /// by default, when a channel curve draws in its own R, G or B; on, the
  /// whole graph takes the theme. Read once by the panel and passed down, so a
  /// card asks nothing of its own in a rebuild.
  final bool themedGraphs;

  /// The Curves plot's side and where a change to it is written (item 6.32).
  /// Read from the workspace once here, for the same reason [themedGraphs] is.
  final double curvePlotSize;
  final ValueChanged<double>? onCurvePlotSize;

  const _EffectSection({
    super.key,
    required this.info,
    required this.open,
    required this.onToggle,
    required this.selected,
    required this.onSelect,
    this.driven = const {},
    required this.stagedValue,
    required this.index,
    required this.count,
    required this.layer,
    required this.allLayers,
    required this.comp,
    required this.playheadFrame,
    required this.onSeek,
    required this.onStackChanged,
    required this.onWrite,
    required this.onLive,
    this.onWritePair,
    this.renaming = false,
    this.onRenamed,
    this.onRenameCancelled,
    this.onStartRename,
    required this.isGroupOpen,
    required this.onToggleGroup,
    required this.onAction,
    required this.pressed,
    this.trackCorrected = false,
    this.themedGraphs = false,
    this.curvePlotSize = curvePlotSizeDefault,
    this.onCurvePlotSize,
  });

  /// Freshly read handles for **everything a command on this card acts on**
  /// (K-523): the picked run when this effect is part of one, and this effect
  /// alone when it is not.
  ///
  /// Which is exactly the question `effectsToCopy` already answers for Copy —
  /// so the card's other commands ask it too, rather than each holding a
  /// handle to its own row. Returned in stack order, and the handles are safe
  /// to hold together: the seam matches them by id, so removing one does not
  /// stale the next.
  List<BridgeEffectInstance> _handles(BuildContext context) {
    final ids = Provider.of<LumitUiState>(context, listen: false)
        .effectsToCopy(layer, info.id)
        .toSet();
    return [
      for (final candidate in layer.getEffects())
        if (ids.contains(candidate.getInfo().id)) candidate,
    ];
  }

  /// Run [op] on each of them, in stack order.
  void _withHandle(BuildContext context, void Function(BridgeEffectInstance) op,
      {bool reversed = false}) {
    final handles = _handles(context);
    for (final handle in reversed ? handles.reversed : handles) {
      op(handle);
    }
  }

  /// Switch this effect on or off. One implementation, because the heading's
  /// enlarged hit area and the checkbox mark inside it are two ways at the same
  /// switch and must not drift into two.
  void _setEnabled(BuildContext context, bool on) {
    // All of them take *this* card's new state rather than each flipping its
    // own, so a run of mixed ticks comes out even.
    _withHandle(context, (e) => layer.setEffectEnabled(effect: e, enabled: on));
    onStackChanged();
  }

  /// Chain or unchain a vector pair (K-443).
  ///
  /// Staged onto a fresh handle and committed with the stack, exactly as a
  /// rename is: one `SetLayerEffects`, one undo step, the shape every effect
  /// edit has. The **proportional drag itself never comes here** — that is
  /// arithmetic the row does while a gesture is live, and the document's
  /// business is only which pairs are tied together.
  void _togglePairLink(String stem) {
    final stack = layer.getEffects();
    for (final instance in stack) {
      if (instance.id() != info.id) continue;
      // The engine answers whether anything moved, so a toggle that would
      // undo to itself commits nothing.
      if (!instance.setPairLinked(
          stem: stem, linked: !info.linkedPairs.contains(stem))) {
        return;
      }
      try {
        layer.setEffects(effects: stack);
      } catch (_) {
        // The stack changed under us; re-reading is the recovery.
      }
      break;
    }
    onStackChanged();
  }

  /// Put every parameter back to the value its schema declares, and drop any
  /// curve on it — one op, so one undo step for the whole reset.
  ///
  /// Written straight through the stack rather than through [EffectStackEditor],
  /// which stages exactly one parameter: a reset is every parameter at once, and
  /// staging them one at a time would be one undo entry each.
  void _reset() {
    final stack = layer.getEffects();
    for (final instance in stack) {
      if (instance.id() != info.id) continue;
      for (final param in cachedListParameters(info.name)) {
        // A button has no value to put back (K-417).
        if (defaultEffectValue(param.kind) case final value?) {
          instance.setValue(id: param.id, value: value);
        }
      }
      try {
        layer.setEffects(effects: stack);
      } catch (_) {
        // The stack changed under us; re-reading is the recovery.
      }
      break;
    }
    onStackChanged();
  }

  @override
  Widget build(BuildContext context) {
    final id = info.id;
    final values = {for (final v in info.values) v.id: v.value};

    return FxSection(
      // The user's own name where one is set (K-321); the effect's label
      // otherwise.
      title: info.customName ?? effectLabelOf(info.name),
      open: open,
      onToggle: onToggle,
      selected: selected,
      onSelect: onSelect,
      renaming: renaming,
      onRenamed: onRenamed,
      onRenameCancelled: onRenameCancelled,
      twirlKey: ValueKey<String>('fx-twirl-$id'),
      // Bypassed draws as a dashed outline round the heading (docs/15 §5), not
      // as a faded stack: the values stay readable while the effect is off.
      enabled: info.enabled,
      leading: LumitTooltip(
        message: info.enabled ? l10n.tipDisable : l10n.tipEnable,
        // **A target you can hit** (owner, desk test). The checkbox's own 14px
        // box is what a settings page wants; here it is the control reached
        // for most, and it was being missed. The whole stopwatch column, for
        // the whole height of the heading, switches the effect — and the mark
        // inside is drawn a step larger so it holds its own beside the
        // heading's capitals (see `fxEnableHitWidth`), and a drag off it sets
        // every switch it crosses to what this one just became (item 6.2).
        child: fxEnableSwitch(
          id: '$id',
          on: info.enabled,
          onChanged: (on) => _setEnabled(context, on),
        ),
      ),
      actions: [
        fxTextAction(
          context,
          label: l10n.reset,
          tip: l10n.tipResetParameters,
          keyName: 'fx-reset-$id',
          onPressed: _reset,
        ),
        // What this effect cost in the last measured frame — the same number
        // its row in the Timeline shows, from the same measurement (docs/13
        // §7.1). Blank unless the Timeline's render-time column is measuring,
        // so this panel neither turns the cost on nor shows a stale figure.
        // Expanded rather than a fixed box after a Spacer: the value column is
        // as wide as the panel leaves it, and a readout that insisted on its
        // own width overflowed the heading in a narrow panel. It right-aligns
        // itself and clips rather than pushing anything.
        Expanded(child: TimingsCell(effectId: '$id')),
      ],
      // Drag the heading to move the effect (docs/07 §6): the gesture the rest
      // of the application already uses to reorder a list, and the one the
      // owner asked for.
      dragIndex: index,
      onDropped: (from) {
        final stack = layer.getEffects();
        if (from < 0 || from >= stack.length) return;
        try {
          layer.reorderEffect(effect: stack[from], newIndex: index);
        } catch (_) {
          // The stack changed under the drag; re-reading is the recovery.
        }
        onStackChanged();
      },
      // Right-click is where the rest of the reordering lives (K-276): the two arrows that
      // used to sit here spent permanent space on a rare act, and the render
      // time — read constantly while a comp is being made faster — earns that
      // space instead. Nothing is lost: the menu moves an effect a step, and
      // to either end.
      onContextMenu: (at) => _stackMenu(context, at),
      trailing: _markButton(
        context,
        mark: '×',
        tip: l10n.tipRemove,
        enabled: true,
        key: 'fx-remove-$id',
        onPressed: () {
          _withHandle(context, (e) => layer.removeEffect(effect: e));
          onStackChanged();
        },
      ),
      // An effect with its own display (Levels' histogram, K-413) draws it
      // above its rows; the rows themselves are unchanged.
      rows: [
        // Above everything, because it is the reason the rows beneath it are
        // not doing anything (docs/12 §1, §2.3). An effect that is behaving
        // draws none of this.
        if (effectBadgeRow(context,
                id: '$id',
                reason: info.badgeReason,
                detail: info.badgeDetail)
            case final badge?)
          badge,
        if (customEffectDisplay(
          info.name,
          effectId: id,
          values: {
            for (final p in info.values) p.id: stagedValue(id, p.id) ?? p.value,
          },
          comp: comp,
          layer: layer,
          playheadFrame: playheadFrame,
          onWrite: onWrite,
          onLive: onLive,
          onChanged: onStackChanged,
          pressed: pressed,
          trackCorrected: trackCorrected,
        )
            case final display?)
          display,
        ..._paramRows(id, values),
      ],
    );
  }

  /// The parameter rows, folded through the schema's groups (docs/08 §1.2,
  /// K-145/K-257) and the `_x`/`_y` point-pair convention (docs/07 §6.1):
  ///
  /// - a labelled group renders as a sub-twirl at its first member's
  ///   position, its members inside;
  /// - an empty-label group renders its members in place, headerless — the
  ///   conditional-run shape;
  /// - a group with `visible_when` is skipped entirely (members included)
  ///   while the named sibling Choice holds a different value;
  /// - two adjacent Float params `foo_x`, `foo_y` fold into one point row
  ///   (with the position dropper for the declared %-of-frame pairs).
  List<Widget> _paramRows(UuidValue id, Map<String, BridgeEffectValue> values) {
    final params = cachedListParameters(info.name);
    final groups = cachedListParameterGroups(info.name);
    final byFirstMember = <String, BridgeParamGroup>{};
    final memberOf = <String, BridgeParamGroup>{};
    for (final g in groups) {
      if (g.params.isNotEmpty) byFirstMember[g.params.first] = g;
      for (final m in g.params) {
        memberOf[m] = g;
      }
    }
    bool groupVisible(BridgeParamGroup g) {
      final param = g.visibleWhenParam;
      final want = g.visibleWhenValues;
      if (param == null || want.isEmpty) return true;
      return switch (values[param]) {
        // A group may answer to SEVERAL modes (K-259: the flare's
        // source-colour toggle belongs to Matte and Lights alike).
        BridgeEffectValue_Choice(:final field0) => want.contains(field0),
        _ => false,
      };
    }

    // Which rows another parameter has taken over (`EnabledWhen`, K-313).
    // Judged on what the panel is SHOWING, staged drag included, so ticking a
    // checkbox greys its dependent row on the spot rather than after the commit
    // round-trips.
    final shown = {
      for (final p in params)
        if ((stagedValue(id, p.id) ?? values[p.id]) case final v?) p.id: v,
    };
    final disabled = disabledParams(info.name, shown);

    // **The uniform Matte row** (K-395, K-425) and **the Mix row**. A Layer
    // picker carries its Channel choice and Invert switch beside it on one
    // row, a Mix slider its Blend choice, and none of the riders gets a row of
    // its own. A rider is found by id convention among the parameters the
    // schema places RIGHT AFTER its host — `matte` + `matte_invert` +
    // `matte_channel`, Depth of field's older `depth` + `depth_invert`, whose
    // stored ids K-065 keeps, and `mix` + `blend` — so the injected rows and
    // the effects that predate them fold the same way without a table here
    // naming them, and a channel an effect declares elsewhere for itself
    // (Depth of field's `depth_channel`, three twirls down) stays the row it
    // always was, as does the Lens flare's own `blend`, which sits BEFORE its
    // Mix. Choices draw before switches so the row reads picker, Channel,
    // Invert. It costs no bridge call: `params` is the cached schema this
    // method already read.
    List<BridgeParamInfo> ridersFor(BridgeParamInfo p) {
      final names = switch (p.kind) {
        BridgeParamKind_Layer() => {'${p.id}_invert', '${p.id}_channel'},
        BridgeParamKind_Float() when p.id == 'mix' => {'blend'},
        _ => const <String>{},
      };
      final out = <BridgeParamInfo>[];
      for (var i = params.indexOf(p) + 1;
          i < params.length && names.contains(params[i].id);
          i++) {
        out.add(params[i]);
      }
      out.sort((a, b) =>
          (a.kind is BridgeParamKind_Bool ? 1 : 0) -
          (b.kind is BridgeParamKind_Bool ? 1 : 0));
      return out;
    }

    final folded = <String>{
      for (final p in params)
        for (final r in ridersFor(p)) r.id,
    };

    Widget rowFor(BridgeParamInfo param) {
      return EffectParamRowFrb(
        key: ValueKey<String>('fx-row-$id-${param.id}'),
        effectId: id,
        param: param,
        value: stagedValue(id, param.id) ?? values[param.id],
        comp: comp,
        ownerLayerId: layer.internallayerId,
        ownerLayers: allLayers,
        playheadFrame: playheadFrame,
        onSeek: onSeek,
        onWrite: onWrite,
        onLive: onLive,
        twoColumn: true,
        enabled: !disabled.contains(param.id),
        // The effect's other values, for a control whose behaviour
        // depends on a sibling (the depth-of-field dropper reads the
        // effect's own `depth` layer).
        siblings: values,
        riders: [
          for (final r in ridersFor(param))
            (r, stagedValue(id, r.id) ?? values[r.id]),
        ],
        onAction: onAction,
        driven: driven['$id/${param.id}'],
      );
    }

    // **The curve fold** (K-412, docs/08 §3.30). A run of neighbouring Curve
    // parameters is one editor with a tab each, not one plot per row: five
    // stacked squares would be five times the height and would still make the
    // user compare shapes across them. The same folding the `_x`/`_y` point
    // pair takes, over as many parameters as declare a curve in a row.
    Widget curveEditor(List<BridgeParamInfo> run) => CurveChannelEditor(
          key: ValueKey<String>('fx-curves-$id'),
          keyPrefix: 'fx-curves-$id',
          labels: [for (final p in run) engineLabel(p.label)],
          // A Red tab draws red (owner, desk test) — unless the setting hands
          // the whole graph back to the theme.
          channelColours: themedGraphs
              ? null
              : [for (final p in run) curveChannelColour(p.id)],
          curves: [
            for (final p in run)
              switch (stagedValue(id, p.id) ?? values[p.id]) {
                BridgeEffectValue_Curve(:final field0) => curvePointsOf(field0),
                _ => curveIdentity,
              },
          ],
          resetLabel: l10n.reset,
          resetTip: l10n.tipResetCurve,
          plotSize: curvePlotSize,
          onPlotSize: onCurvePlotSize,
          onLive: (c, points) => onLive(id, run[c].id, curveValue(points)),
          onCommit: (c, points) => onWrite(id, run[c].id, curveValue(points)),
        );

    // Fold a run of params into rows, pairing x/y neighbours and gathering
    // curve runs. Both folds live here rather than only in the outer walk,
    // because a schema is free to put them inside a **group** — Particulate's
    // two over-life curves sit under the Particle kicker, and before this they
    // came out as two stacked plots, which is the shape K-412 exists to
    // prevent.
    List<Widget> foldRows(List<BridgeParamInfo> run) {
      final out = <Widget>[];
      var i = 0;
      while (i < run.length) {
        final param = run[i];
        if (param.kind is BridgeParamKind_Curve) {
          final curves = <BridgeParamInfo>[];
          while (i < run.length && run[i].kind is BridgeParamKind_Curve) {
            curves.add(run[i]);
            i += 1;
          }
          out.add(curveEditor(curves));
          continue;
        }
        // Already drawn, beside its picker or slider on the Matte or Mix row.
        if (folded.contains(param.id)) {
          i += 1;
          continue;
        }
        final next = i + 1 < run.length ? run[i + 1] : null;
        final isPair = next != null &&
            param.id.endsWith('_x') &&
            next.id == '${param.id.substring(0, param.id.length - 2)}_y' &&
            param.kind is BridgeParamKind_Float &&
            next.kind is BridgeParamKind_Float;
        if (isPair) {
          final stem = pairStemOf(info.name, param.id);
          out.add(EffectPointRowFrb(
            key: ValueKey<String>('fx-row-$id-${param.id}-pair'),
            effectId: id,
            xParam: param,
            yParam: next,
            xValue: stagedValue(id, param.id) ?? values[param.id],
            yValue: stagedValue(id, next.id) ?? values[next.id],
            comp: comp,
            playheadFrame: playheadFrame,
            onSeek: onSeek,
            onWrite: onWrite,
            onLive: onLive,
            onWritePair: onWritePair,
            twoColumn: true,
            // A point is one row over two parameters, so it goes quiet only
            // when both halves have been taken over — which is how the schema
            // declares them.
            enabled:
                !disabled.contains(param.id) || !disabled.contains(next.id),
            // The chain (K-443). The stem is the schema's key for the pair,
            // and which pairs are tied is on the instance, so both come out of
            // data the card already holds — no call rides this rebuild.
            linked: stem != null && info.linkedPairs.contains(stem),
            onToggleLink: stem == null ? null : () => _togglePairLink(stem),
          ));
          i += 2;
        } else {
          out.add(rowFor(param));
          i += 1;
        }
      }
      return out;
    }

    final rows = <Widget>[];
    var i = 0;
    while (i < params.length) {
      final param = params[i];

      // The curve fold, for a run the schema left ungrouped. A grouped run
      // takes the same fold inside `foldRows`.
      if (param.kind is BridgeParamKind_Curve) {
        final run = <BridgeParamInfo>[];
        while (i < params.length && params[i].kind is BridgeParamKind_Curve) {
          run.add(params[i]);
          i += 1;
        }
        rows.add(curveEditor(run));
        continue;
      }

      final group = byFirstMember[param.id];
      if (group != null) {
        // Consume the whole contiguous member run.
        final run = <BridgeParamInfo>[];
        while (i < params.length && group.params.contains(params[i].id)) {
          run.add(params[i]);
          i += 1;
        }
        if (!groupVisible(group)) continue;
        if (group.label.isEmpty) {
          rows.addAll(foldRows(run));
        } else {
          rows.add(_ParamGroupSection(
            key: ValueKey<String>('fx-group-$id-${group.label}'),
            label: group.label,
            collapsed: group.collapsed,
            rows: foldRows(run),
          ));
        }
      } else if (memberOf.containsKey(param.id)) {
        // A group member reached out of order (a schema whose group is not
        // contiguous): render it plainly rather than losing it — unless it is
        // a rider already drawn on its picker's or slider's row.
        if (!folded.contains(param.id)) rows.add(rowFor(param));
        i += 1;
      } else {
        // Flat params fold too, so an ungrouped x/y pair still joins.
        final flat = <BridgeParamInfo>[param];
        i += 1;
        // Peek: only extend the flat run while the next is also ungrouped.
        while (i < params.length &&
            !byFirstMember.containsKey(params[i].id) &&
            !memberOf.containsKey(params[i].id) &&
            flat.length < 2 &&
            param.id.endsWith('_x')) {
          flat.add(params[i]);
          i += 1;
        }
        rows.addAll(foldRows(flat));
      }
    }
    return rows;
  }

  /// A small text mark rather than an icon, matching v0's × for Remove — the
  /// icon set has no caret or close glyph, and three marks do not earn three
  /// new ones.
  /// The effect heading's right-click menu: where it sits in the stack, and
  /// removing it. Reordering is a handful of acts in a session, so it lives
  /// here rather than in two buttons on every heading — and unlike the arrows
  /// it can send an effect to the top or the bottom in one go.
  /// Put this effect on the clipboard (K-275) — with the rest of the picked run
  /// when it is part of one (K-300).
  ///
  /// A failure is swallowed the way the neighbouring effect commands' are: the
  /// effect went away between the menu opening and the row being chosen, and an
  /// error about a thing that is no longer there helps nobody.
  void _copyEffect(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    try {
      ui.copyEffectsToClipboard(
        layer.copyEffects(effects: ui.effectsToCopy(layer, info.id)),
      );
    } catch (_) {
      // The effect is gone; the clipboard keeps whatever it had.
    }
  }

  void _stackMenu(BuildContext context, Offset at) {
    final id = info.id;
    void move(int to) {
      // **The order several picked effects land in** (K-523). Each one is
      // taken out of the stack and put back *at* `to`, so the last one moved
      // ends up in front of the others: an upward move therefore takes the
      // bottom-most first and a downward one the topmost, and either way the
      // run arrives together with its own order intact.
      _withHandle(context, (e) => layer.reorderEffect(effect: e, newIndex: to),
          reversed: to <= index);
      onStackChanged();
    }

    showLumitPopup<void>(
      context: context,
      position: at,
      builder: (close) => FloatSurface(
        width: 190,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          // Only the moves this effect can actually make are listed: a menu of
          // dead rows tells you what you cannot do, which is not what a menu
          // is for (docs/15 §no punishment UI).
          children: [
            // **Rename, first** (owner, desk test): the heading had no mouse
            // path to its own editor at all — `Enter` on the selection was the
            // only way in, and a keyboard-only act is one nobody finds. It is
            // the menu's entry rather than a double-click on the name because
            // that is the pattern the application already settled on: K-191
            // took renaming off a list row's second click precisely because a
            // slow double-click and a deliberate click were the same gesture,
            // and K-321 put it on the row menu instead. An effect heading is a
            // list row.
            if (onStartRename case final start?)
              MenuRow(
                key: ValueKey<String>('fx-menu-rename-$id'),
                onPressed: () {
                  close(null);
                  start();
                },
                child: Text(l10n.rename),
              ),
            if (index > 0) ...[
              MenuRow(
                key: ValueKey<String>('fx-menu-up-$id'),
                onPressed: () {
                  close(null);
                  move(index - 1);
                },
                child: Text(l10n.moveUp),
              ),
              MenuRow(
                key: ValueKey<String>('fx-menu-top-$id'),
                onPressed: () {
                  close(null);
                  move(0);
                },
                child: Text(l10n.moveToTop),
              ),
            ],
            if (index < count - 1) ...[
              MenuRow(
                key: ValueKey<String>('fx-menu-down-$id'),
                onPressed: () {
                  close(null);
                  move(index + 1);
                },
                child: Text(l10n.moveDown),
              ),
              MenuRow(
                key: ValueKey<String>('fx-menu-bottom-$id'),
                onPressed: () {
                  close(null);
                  move(count - 1);
                },
                child: Text(l10n.moveToBottom),
              ),
            ],
            // **Copy this one effect** (K-275). The engine has taken one or a
            // whole stack since copy/paste landed — `copy_effects(Some(id))` —
            // and the Edit menu's Copy takes the *layer*, so until now there
            // was no way to pick a single effect and no way to reach the call.
            // It goes on the same clipboard a stack does: both are `.lumfx`, so
            // both paste the same way, and Paste needs no idea which it holds.
            MenuRow(
              key: ValueKey<String>('fx-menu-copy-$id'),
              onPressed: () {
                close(null);
                _copyEffect(context);
              },
              child: Text(l10n.copyEffect),
            ),
            MenuRow(
              key: ValueKey<String>('fx-menu-remove-$id'),
              onPressed: () {
                close(null);
                _withHandle(context, (e) => layer.removeEffect(effect: e));
                onStackChanged();
              },
              child: Text(l10n.removeEffect),
            ),
          ],
        ),
      ),
    );
  }

  Widget _markButton(
    BuildContext context, {
    required String mark,
    required String tip,
    required bool enabled,
    required String key,
    required VoidCallback onPressed,
  }) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: tip,
      child: HouseButton(
        key: ValueKey<String>(key),
        frameless: true,
        small: true,
        padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 2),
        onPressed: enabled ? onPressed : null,
        child: Text(
          mark,
          style:
              t.small.copyWith(color: enabled ? t.textMuted : t.textDisabled),
        ),
      ),
    );
  }
}

/// One collapsible parameter group inside an effect's card (docs/08 §1.2,
/// K-145): a small twirl header, its member rows indented under it. Open
/// state is session-local (a fresh panel starts groups at their declared
/// `collapsed`).
class _ParamGroupSection extends StatefulWidget {
  final String label;
  final bool collapsed;
  final List<Widget> rows;
  const _ParamGroupSection({
    super.key,
    required this.label,
    required this.collapsed,
    required this.rows,
  });

  @override
  State<_ParamGroupSection> createState() => _ParamGroupSectionState();
}

class _ParamGroupSectionState extends State<_ParamGroupSection> {
  late bool _open = !widget.collapsed;

  @override
  Widget build(BuildContext context) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // The shared header row, so a group's twirl sits in the stopwatch
          // column and its label starts where every other label does. The
          // members are NOT indented: K-443's straight label edge runs the
          // whole panel, and a fold says what it is with its kicker.
          fxGroupHeaderRow(
            context,
            label: widget.label,
            open: _open,
            onToggle: () => setState(() => _open = !_open),
          ),
          if (_open)
            Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: widget.rows,
            ),
        ],
      );
}

/// The Transform section: the layer's transform rows, in the panel's section
/// chrome.
///
/// The rows themselves are [TransformRowsFrb], shared with the Timeline's
/// twirl-down — this is the section around them, which is all that is particular
/// to this panel.
class _TransformSection extends StatelessWidget {
  final LayerReference layer;
  final CompositionReference comp;
  final BridgeTransform transform;
  final bool threeD;

  /// How each two-axis property is shown (K-571).
  final BridgeAxisModes axisModes;

  /// Whether this layer is a Camera — the one kind whose transform can be
  /// **derived** rather than held (K-417), and so the one kind whose heading
  /// carries a link badge.
  final bool isCamera;

  /// This camera's solve link carries a correction (K-578) — the dot beside the
  /// badge, and what makes Clear corrections worth offering.
  final bool corrected;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;
  final bool open;
  final VoidCallback onToggle;

  const _TransformSection({
    super.key,
    required this.layer,
    required this.comp,
    required this.transform,
    required this.threeD,
    required this.axisModes,
    required this.isCamera,
    required this.corrected,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
    required this.open,
    required this.onToggle,
  });

  @override
  Widget build(BuildContext context) => FxSection(
        title: engineLabel('Transform'),
        open: open,
        onToggle: onToggle,
        // A solve-linked camera says so where its numbers are, because that is
        // where the surprise would otherwise be: the rows below are still
        // editable, but what they hold is a correction on top of the solve
        // rather than a pose (K-578). Convert to keyframes
        // sits beside the badge — it is the one command that ends the link,
        // and it belongs next to the thing it ends.
        actions: [
          // Expanded, as the render-time cell beside an effect's Reset is, and
          // for the same reason: a heading's action row lays its children out
          // unbounded, so a badge that carries a sentence has to be told how
          // much room it has before it can clip it.
          if (isCamera)
            Expanded(
              child: CameraLinkBadge(
                key: ValueKey<String>('tf-link-${layer.internallayerId}'),
                camera: layer,
                playheadFrame: playheadFrame,
                corrected: corrected,
                onChanged: onChanged,
              ),
            ),
        ],
        rows: TransformRowsFrb(
          comp: comp,
          layer: layer,
          transform: transform,
          threeD: threeD,
          axisModes: axisModes,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: onChanged,
          twoColumn: true,
        ).rows(context),
      );
}

/// The colour a curve channel is drawn in, or null for the ones that take the
/// theme's own (owner, desk test).
///
/// Curves and Levels both declare their channels as `red`, `green` and `blue`
/// beside a `master` — so the schema's own ids answer this, and an effect that
/// grows a fourth channel one day is coloured without a table here being
/// remembered. The three are [ScopeColours.standard]'s, which is where every
/// other channel reading in the application takes its red from: a histogram
/// hump and a Curves tab that disagreed about what red looks like would be two
/// pictures of the same channel.
Color? curveChannelColour(String paramId) => switch (paramId) {
      'red' => ScopeColours.standard.red,
      'green' => ScopeColours.standard.green,
      'blue' => ScopeColours.standard.blue,
      _ => null,
    };

/// The display an effect draws *above* its rows, or null for the effects that
/// draw none — which is nearly all of them.
///
/// **Levels is the one that claims it** (K-413): a histogram of the frame with
/// its input handles over it and the output range beneath, which is not a list
/// of numbered rows and would be the wrong control forced into one. The numbers
/// keep their rows underneath, unchanged; this is presentation, and it writes
/// through the same two callbacks every row writes through.
///
/// Curves does *not* come through here, even though it is the other effect with
/// a shape for a value. Its five channel curves are five declared parameters
/// that fold into one tabbed editor, exactly as an `_x`/`_y` pair folds into one
/// point row — a fold, not a display, and `_paramRows` is where folds live.
Widget? customEffectDisplay(
  String matchName, {
  required UuidValue effectId,
  required Map<String, BridgeEffectValue> values,
  required CompositionReference comp,
  required LayerReference layer,
  required int playheadFrame,
  required void Function(UuidValue, String, BridgeEffectValue) onWrite,
  required void Function(UuidValue, String, BridgeEffectValue) onLive,
  required VoidCallback onChanged,
  required int pressed,
  bool trackCorrected = false,
}) =>
    switch (matchName) {
      'levels' => LevelsDisplayFrb(
          key: ValueKey<String>('fx-levels-display-$effectId'),
          effectId: effectId,
          values: values,
          comp: comp,
          playheadFrame: playheadFrame,
          onWrite: onWrite,
          onLive: onLive,
        ),
      // Camera track's display is a *status*, not a control (K-417): how far
      // an analysis running elsewhere has got, and what its solve came to. It
      // writes no parameter, which is why it takes neither callback.
      'camera_track' => CameraTrackDisplayFrb(
          key: ValueKey<String>('fx-camera-track-display-$effectId'),
          layer: layer,
          onChanged: onChanged,
          pressed: pressed,
          corrected: trackCorrected,
        ),
      // Planar track's display is the same kind of thing and not the same
      // thing (K-579): a status, but about a *surface* rather than a camera,
      // and filed under this instance rather than under the media — which is
      // why it is the one custom display that needs its effect's own id.
      'planar_track' => PlanarTrackDisplayFrb(
          key: ValueKey<String>('fx-planar-track-display-$effectId'),
          layer: layer,
          effectId: effectId,
          onChanged: onChanged,
          pressed: pressed,
        ),
      _ => null,
    };

/// The calm badge an effect wears when the last frame it drew was not its own
/// work, or when this build has never heard of it at all (docs/12 §1, §2.3).
///
/// Four things it can say, and the engine sends a **key** for each rather than a
/// sentence, so the words are the user's own (K-303): the plugin failed, the
/// plugin is switched off, the plugin is not installed on this machine, or the
/// effect came from a newer Lumit. Where the engine or the plugin has words of
/// its own about a failure they go underneath, verbatim — it is somebody else's
/// sentence about somebody else's code and translating it would be inventing.
///
/// Never an alarm and never red: the comp is still compositing, the effect is
/// rendering as identity, and the values below are still readable and still
/// saved. `null` — no badge, no row — is the ordinary case and costs nothing.
///
/// A free function rather than a widget class so a test can pump exactly this
/// and nothing else.
Widget? effectBadgeRow(
  BuildContext context, {
  required String id,
  String? reason,
  String? detail,
}) {
  if (reason == null) return null;
  final sentence = effectBadge(reason);
  if (sentence == null) return null;
  final t = ThemeScope.of(context).theme;
  return Padding(
    key: ValueKey<String>('fx-badge-$id'),
    padding: const EdgeInsets.symmetric(vertical: 2),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          sentence,
          key: ValueKey<String>('fx-badge-reason-$id'),
          style: t.small.copyWith(color: t.accent),
        ),
        if (detail != null && detail.isNotEmpty)
          Text(
            detail,
            key: ValueKey<String>('fx-badge-detail-$id'),
            style: t.small.copyWith(color: t.textMuted),
          ),
      ],
    ),
  );
}
