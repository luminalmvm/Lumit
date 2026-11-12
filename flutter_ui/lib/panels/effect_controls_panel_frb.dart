// The Effect controls panel, on the flutter_rust_bridge API — the effect stack.
//
// **The shape of it.** One twirl-open section per effect, read the same way the
// Timeline's fold-out reads: a heading bar carrying the effect's enable switch
// and its name, then a row per declared parameter under it, each row separated
// from the next by a hairline. Every row is two columns — the parameter's name
// left-aligned in a fixed-width name column, its control left-aligned in the
// rest — with nothing drawn between them; they read as columns because they line
// up, which is all a column is (`fx_section.dart`). What acts on a whole effect,
// Reset and the close mark, comes up on the heading under the pointer, and the
// rest of it is in the heading's menu.
//
// Add effect offers every built-in, grouped by category. Above the stack sit the
// Transform rows — anchor, position, scale, rotation, opacity, plus the z and
// x/y-rotation rows when the layer is 3D — in a section of the same shape.
//
// **Effects that want their own display** are the exception this layout
// expects. [customEffectDisplay] is asked for a widget to draw *above* the
// rows, and Levels claims it: a histogram of the frame with its input handles
// over it and the output range beneath. The rows themselves are
// unchanged — the display writes the same values through the same callbacks.
// Curves' spline is not a display but a **fold**: its five channel curves are
// five declared parameters folded into one tabbed editor, the way an `_x`/`_y`
// pair folds into one point row.
//
// Every animatable row carries the stopwatch, quiet until the pointer is on the
// row or the row is keyed, and a keyed row the ◄ ◆ ► navigator at its far end
// (keyframe_controls_frb.dart). The stopwatch turns animation off again, keeping
// the value the curve reads at the playhead, so the row is never a dead end.
//
// **How an edit reaches the document.** `getEffects` hands back a *staged* copy
// of the stack, `setValue` edits that copy, and `LayerReference.setEffects`
// commits the whole list as one `SetLayerEffects` op. So a drag mutates the copy
// and renders it through `renderFrameWithPreview` — which patches a clone of the
// document engine-side — and only the release commits. A drag therefore costs
// one undo entry rather than one per tick, which is the whole reason the staged
// shape exists.

import 'dart:async';

import 'package:flutter/foundation.dart' show listEquals, mapEquals;
import 'package:lumit_flutter/state/share.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/template/copy.dart' show BridgeCopy;
import 'package:lumit_flutter/src/rust/api/track.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../state/layer_bounds.dart' show isGeometryOptions, meshKindOf;
import '../shell/menu_bar_frb.dart' show removeAllEffects;
import '../shell/menu_layer_frb.dart' show blendModeNames;
import '../state/file_dialogs.dart';
import '../l10n/strings.dart';
import '../widgets/controls.dart';
import '../widgets/curve_editor.dart';
import 'cut_kinds.dart';
import 'effect_param_row_frb.dart';
import 'viewer_effect_points.dart' show pickForPoint;
import 'graph_panel.dart' show DrivenParam, drivenParamsIn, graphCompById;
import 'node_panel.dart' show NodePanelFrb;
import 'camera_track_display_frb.dart';
import 'plane_display_frb.dart';
import 'planar_track_display_frb.dart';
import 'levels_display_frb.dart';
import 'roto_display_frb.dart';
import 'flare_window.dart';
import 'lens_window.dart';
import 'lfx_interface_panel.dart';
import 'shader_editor.dart';
import 'fx_key_strip.dart';
import 'fx_section.dart';
import 'layer_fold_frb.dart'
    show
        asOneUndoStep,
        clipFoldPrefix,
        effectPath,
        groupFoldPrefix,
        inputsPath,
        modelMaterialPath,
        stylePath,
        transformGroupPath,
        transformPath;
import '../widgets/share_marks.dart';
import 'timeline_extras_frb.dart' show DoubleTap, clipRadius, showMenuAt;
import 'timeline_metrics_frb.dart' show dimmedIf;
import 'transform_rows_frb.dart';
import '../state/clipboard.dart';
import '../state/copies.dart';
import '../theme/theme.dart';
import '../state/drag_payloads.dart';
import 'copy_card_frb.dart';
import 'control_rows.dart'
    show copyControlRow, effectControlRow, pointControlRow;
import 'effect_commands.dart';
import 'param_row_menu.dart' show menuGroups;
import 'placeholder.dart';
import 'preset_save.dart';
import 'row_filter.dart';
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
  late final Set<String> _shut =
      Provider.of<LumitUiState>(context, listen: false).folds.effectsShut;

  /// The cards that start closed and have been opened since, which keep their
  /// rows from then on.
  final Set<UuidValue> _everOpen = {};

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
        _folds += 1;
        if (!_shut.remove(path)) _shut.add(path);
      });

  /// How many times a twirl has moved in this panel's life.
  ///
  /// A card is handed the fold state of its own parameter groups, so it has to
  /// hear that the sets have changed to draw the change. Counted rather than
  /// compared, because a twirl is one deliberate act and the panel is redrawn
  /// for it whatever this says.
  int _folds = 0;

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
      _folds += 1;
      for (final other in picked) {
        if (opening) {
          _shut.remove('fx-$other');
        } else {
          _shut.add('fx-$other');
        }
      }
    });
  }

  /// Twirl every effect on the stack open or shut.
  void _foldAll(List<BridgeEffectInstanceInfo> effects, bool open) =>
      setState(() {
        _folds += 1;
        for (final fx in effects) {
          if (open) {
            _shut.remove('fx-${fx.id}');
          } else {
            _shut.add('fx-${fx.id}');
          }
        }
      });

  /// The stack's own menu, for the panel's empty ground and its header.
  ///
  /// [layer] is null on a clip's or a group's stack, which takes an add and a
  /// fold and has no clipboard or preset road of its own.
  void _showStackMenu(
    BuildContext context,
    Offset at, {
    required List<BridgeEffectInstanceInfo> effects,
    required ValueChanged<String> onAdd,
    LayerReference? layer,
  }) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final app = Provider.of<LumitState>(context, listen: false);
    final ws = ui.workspace;
    final pastable = ui.clipboard.kind == ClipboardKind.effects;
    void changed() {
      app.notifyDocumentChanged();
      ui.model.refresh();
    }

    showLumitPopup<void>(
      context: context,
      position: at,
      builder: (close) {
        void done() => close(null);
        MenuRow row(String key, Widget label, VoidCallback run,
                {bool enabled = true}) =>
            MenuRow(
              key: ValueKey<String>('fx-ground-$key'),
              enabled: enabled,
              onPressed: () {
                done();
                run();
              },
              child: label,
            );
        return FloatSurface(
          width: 210,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: menuGroups([
              [
                SubmenuRow(
                  key: const ValueKey('fx-ground-add'),
                  closeParent: done,
                  submenu: (dismiss) => FloatSurface(
                    width: 200,
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: _addEffectRows(done, onAdd),
                    ),
                  ),
                  child: Padding(
                    padding: const EdgeInsetsDirectional.only(start: 16),
                    child: Text(l10n.addEffect),
                  ),
                ),
                if (layer != null)
                  row(
                    'paste',
                    MenuLabel(l10n.menuPaste,
                        action: 'edit.paste', ticked: false, enabled: pastable),
                    () => _pastePickedEffects(ui),
                    enabled: pastable,
                  ),
                if (layer != null && effects.isNotEmpty)
                  row(
                    'select-all',
                    MenuLabel(l10n.fxMenuSelectAll,
                        action: 'edit.select.all', ticked: false),
                    () => ui.setEffectSelection(
                        layer, [for (final e in effects) e.id]),
                  ),
              ],
              [
                if (effects.isNotEmpty) ...[
                  row('expand', MenuLabel(l10n.fxMenuExpandAll, ticked: false),
                      () => _foldAll(effects, true)),
                  row('collapse', MenuLabel(l10n.fxMenuCollapseAll, ticked: false),
                      () => _foldAll(effects, false)),
                ],
                // The filter reaches Transform too, so it is there with no
                // effects on the layer.
                if (layer != null)
                  row(
                    'filter',
                    MenuLabel(l10n.filterRows,
                        action: 'panel.search.focus', ticked: false),
                    _filter.show,
                  ),
                row(
                  'strip',
                  MenuLabel(l10n.fxMenuKeyStrip, ticked: ws.fxKeyStrip),
                  () => ws.setFxKeyStrip(!ws.fxKeyStrip),
                ),
              ],
              [
                if (layer != null && effects.isNotEmpty)
                  row('save', MenuLabel(l10n.savePresetEllipsis, ticked: false),
                      () => _saveStackPreset(ui, layer)),
                if (layer != null)
                  row('load', MenuLabel(l10n.loadPresetEllipsis, ticked: false),
                      () async {
                    final path = await pickPresetToOpen();
                    if (path == null) return;
                    try {
                      layer.pasteEffects(
                          text: await readEffectPreset(path: path),
                          atFrame: ui.playheadFrame.value);
                    } catch (_) {
                      // Not a preset: the picker will take any file.
                      app.postNotice(l10n.presetFileNotPreset, error: true);
                      return;
                    }
                    changed();
                  }),
              ],
              [
                if (layer != null && effects.isNotEmpty)
                  row(
                      'remove-all',
                      MenuLabel(l10n.fxMenuRemoveAll,
                          action: 'effect.remove.all', ticked: false), () {
                    removeAllEffects(app, [layer]);
                    ui.clearEffectSelection();
                    changed();
                  }),
              ],
            ]),
          ),
        );
      },
    );
  }

  /// Save the layer's whole stack as a preset in the library.
  Future<void> _saveStackPreset(LumitUiState ui, LayerReference layer) =>
      saveLayerPreset(context, ui, layer);

  /// The effect whose heading is an inline rename editor, or null.
  UuidValue? _renamingEffect;

  /// A double-click counter per Custom shader heading, so two
  /// clicks on two different headings are not one double-click. Counted with
  /// [DoubleTap] rather than `onDoubleTap`, which would make every single
  /// click on a heading wait out the recogniser.
  final Map<UuidValue, DoubleTap> _headingTaps = {};

  /// Front the composition a **Node graph** effect applies
  /// (docs/impl/node-graph-comp.md §4.4), which is its Open graph row.
  ///
  /// The bound comp is asked for here rather than held on the card: pressing
  /// the row is a gesture, and the read model does not carry the binding.
  /// Nothing happens when the comp has been deleted from under the effect,
  /// which renders its input unchanged and has nothing to open, and nothing
  /// when the layer itself has gone: this panel keeps the last one it drew.
  void _openGraphOn(LayerReference layer, UuidValue effect) {
    final ui = context.read<LumitUiState>();
    final project = Provider.of<LumitState>(context, listen: false).project;
    try {
      for (final instance in layer.getEffects()) {
        if (instance.id() != effect) continue;
        final graph = graphCompById(project, instance.nodeGraphCompId());
        if (graph != null) ui.setSelectedComp(graph);
        return;
      }
    } catch (_) {
      // The layer went while its card was up; there is nothing to open.
    }
  }

  /// The name of the graph a **Node graph** instance applies, which its card
  /// wears in place of the effect's label (docs/impl/node-graph-comp.md §4.4).
  ///
  /// Null for every other effect, and for a card whose comp has been deleted
  /// from under it, which then shows the label alone. Read off the cached comp
  /// list rather than asked for: the list is remembered until the item tree
  /// changes, so a rename shows on the next read and a rebuild crosses
  /// nothing.
  String? _graphNameOf(BuildContext context, BridgeEffectInstanceInfo fx) {
    final bound = fx.nodeGraphComp;
    if (bound == null) return null;
    final app = Provider.of<LumitState>(context, listen: false);
    for (final (comp, name) in app.comps()) {
      if (comp.internalid == bound) return name;
    }
    return null;
  }

  /// The layer's stack as a Custom shader's home (docs/impl/custom-shader.md
  /// §3.2, CS3).
  ///
  /// The live preview rides the drag path exactly as a parameter does
  /// (`render_frame_with_preview`'s own words: "the live drag path, which
  /// never touches the document").
  InstanceHome _shaderHome(LayerReference layer) {
    final ui = context.read<LumitUiState>();
    final comp = ui.selectedComp;
    return InstanceHome.layer(
      layer,
      draw: comp == null
          ? null
          : (staged) => comp.renderFrameWithPreview(
                frame: ui.playheadFrame.value,
                scale: ui.viewerScale,
                layer: layer,
                effects: staged,
              ),
    );
  }

  /// How many Action buttons have been pressed in this panel's life.
  ///
  /// A press changes nothing in the document — it is an event, not an edit —
  /// so there is no revision for a status line to compare against. This number
  /// is what an effect's own display watches to know a button was pushed.
  int _actionPressed = 0;

  /// Which parameters a driver is wired to, by `effectId/paramId`,
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

  /// Every Points input on the layer's effects, by `effectId/portId`, with
  /// the port's label and the output it is wired from. Taken from the same
  /// read as [_driven].
  Map<String, ({String label, BridgeOutputRef? from})> _points = const {};
  LumitUiState? _boundUi;

  /// The row filter Ctrl+F opens in the header. Its text narrows every card
  /// to the rows whose names hold it.
  final RowFilter _filter = RowFilter();

  /// `Ctrl+F` over this panel: the filter well comes up with the cursor in it.
  void _onSearchRequested() {
    if (_boundUi?.searchRequestIsFor(Panel.effectControls) ?? false) {
      _filter.show();
    }
  }

  void _onFilter() {
    if (mounted) setState(() {});
  }

  @override
  void initState() {
    super.initState();
    _filter.addListener(_onFilter);
    // `Enter` renames the selected effect — registered on the
    // hardware keyboard like every panel command; stands down for modals,
    // focused fields, and whenever this panel is not the active one.
    HardwareKeyboard.instance.addHandler(_onKey);
  }

  /// The shared project's people, whose rows are marked here as well. Its
  /// roster is all that is listened to, and it only moves while a project
  /// is shared.
  ShareState? _share;

  /// What the others in the shared project have in hand in the comp on
  /// show: by row name, the colour of each person who has it. Gathered at
  /// the top of each build from what is already held, and kept as the same
  /// map while it says the same thing, which is what the rows under
  /// [ShareRows] go by to know whether to redraw. Empty outside a shared
  /// project, and then nothing here is built any other way.
  Map<String, List<int>> _theirs = const {};

  Map<String, List<int>> _gatherTheirs(CompositionReference comp) {
    final share = _share;
    if (share == null || !share.active) return const {};
    final out = <String, List<int>>{};
    for (final person in share.inComp(comp)) {
      for (final path in person.properties) {
        final held = out[path] ??= [];
        if (!held.contains(person.colour)) held.add(person.colour);
      }
    }
    if (out.length == _theirs.length &&
        out.entries.every((e) => listEquals(e.value, _theirs[e.key]))) {
      return _theirs;
    }
    return out.isEmpty ? const {} : out;
  }

  /// Someone came, went or took something else in hand. The rows whose
  /// marks changed are the ones that redraw.
  void _onRoster() {
    if (!mounted) return;
    if ((_share?.active ?? false) || _theirs.isNotEmpty) setState(() {});
  }

  /// This person has pressed on the row called [name], which is how the
  /// others in a shared project see what they are on in a panel with no
  /// selection of its own.
  void _onRowTouched(String name) => _boundUi?.touchProperty(name);

  /// Reveal in Effect controls, from a box on the node graph: the effect's
  /// card is scrolled into view once the selection has drawn it.
  void _onReveal() {
    final ui = _boundUi;
    final id = ui?.revealEffect.value;
    if (ui == null || id == null || !mounted) return;
    ui.revealEffect.value = null;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      final wanted = ValueKey<String>('fx-twirl-$id');
      BuildContext? found;
      void visit(Element el) {
        if (found != null) return;
        if (el.widget.key == wanted) {
          found = el;
          return;
        }
        el.visitChildren(visit);
      }

      context.visitChildElements(visit);
      if (found case final card?) {
        Scrollable.ensureVisible(card,
            alignment: 0.1, duration: const Duration(milliseconds: 160));
      }
    });
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _share ??= Provider.of<LumitState>(context, listen: false).share
      ..roster.addListener(_onRoster);
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (identical(ui, _boundUi)) return;
    _unbindDriven();
    _boundUi = ui;
    ui.selectedLayer.addListener(_readDriven);
    ui.model.addListener(_readDriven);
    ui.revealEffect.addListener(_onReveal);
    ui.panelSearchRequest.addListener(_onSearchRequested);
    // `Ctrl+A` here means every effect on the layer, not every layer
    // in the composition — the shell routes the chord to the focused panel.
    ui.selectAllRequest.addListener(_onSelectAllRequested);
    // **Delete means the picked effects while this panel is the focused one**
    // (item 6.6, the claim mechanism). Claimed rather than handled on the
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
    // Copy and Paste are claimed the same way, and for the same reason.
    // They were answered on the keyboard instead for a while, which
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

  /// Whether the editing keys are this panel's to answer: it is focused and
  /// showing a layer's effects. Over a node graph it shows a box instead, and
  /// the layer it last held is in another comp.
  bool _editsLayers(LumitUiState ui) =>
      ui.activePanel == Panel.effectControls && !ui.model.isNodeGraph;

  void _unbindDriven() {
    _boundUi?.selectedLayer.removeListener(_readDriven);
    _boundUi?.model.removeListener(_readDriven);
    _boundUi?.revealEffect.removeListener(_onReveal);
    _boundUi?.panelSearchRequest.removeListener(_onSearchRequested);
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
    if (!mounted || ui == null || !_editsLayers(ui)) {
      return _priorCopyClaim?.call() ?? false;
    }
    return _copyPickedEffects(ui) || (_priorCopyClaim?.call() ?? false);
  }

  bool _pasteClaim() {
    final ui = _boundUi;
    if (!mounted || ui == null || !_editsLayers(ui)) {
      return _priorPasteClaim?.call() ?? false;
    }
    return _pastePickedEffects(ui) || (_priorPasteClaim?.call() ?? false);
  }

  bool _deleteClaim() {
    final ui = _boundUi;
    if (!mounted || ui == null || !_editsLayers(ui)) {
      return _priorDeleteClaim?.call() ?? false;
    }
    return _deletePickedEffects(ui) || (_priorDeleteClaim?.call() ?? false);
  }

  /// Remove every picked effect from the layer this panel is showing.
  ///
  /// The whole run is one undo step. The handles are read together and are safe
  /// to hold: the seam matches them by id, so removing one does not stale the
  /// next.
  ///
  /// Answers whether it took the key. Nothing picked is not this panel's
  /// Delete, and the layer selection is what the shell falls back to.
  bool _deletePickedEffects(LumitUiState ui) {
    final layer =
        ui.selectedEffectsLayer ?? ui.selectedLayer.value ?? _lastLayer;
    final picked = ui.selectedEffects.value.toSet();
    if (layer == null || picked.isEmpty) return false;
    final names = [
      for (final fx in ui.model.byId(layer.internallayerId)?.info.effects ??
          const <BridgeEffectInstanceInfo>[])
        if (picked.contains(fx.id)) fx.customName ?? effectLabelOf(fx.name),
    ];
    try {
      asOneUndoStep(Provider.of<LumitState>(context, listen: false).project,
          () {
        for (final instance in layer.getEffects()) {
          if (picked.contains(instance.getInfo().id)) {
            layer.removeEffect(effect: instance);
          }
        }
      }, name: deleteStepName(names));
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
    if (!ui.selectAllRequestIsFor(Panel.effectControls) || !_editsLayers(ui)) {
      return;
    }
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
    final picked = _boundUi?.selectedLayer.value;
    final layer = picked ?? _lastLayer;
    BridgeLayerGraph? graph;
    try {
      graph = layer?.getGraph();
    } catch (_) {
      // The layer has gone, so the rows draw their own controls again. One
      // held from another comp has gone with its comp, which an undo can
      // take away, so it is let go rather than drawn from its last read.
      if (picked == null && layer != null) {
        setState(() {
          _lastLayer = null;
          _heldEntry = null;
          _heldComp = null;
        });
      }
    }
    // The Timeline's fold-out takes the same reading from the same helper, so
    // a row cannot say *driven* in one panel and offer a spinner in the other.
    final next =
        graph == null ? const <String, DrivenParam>{} : drivenParamsIn(graph);
    final into = {
      for (final e in graph?.wiring.edges ?? const <BridgeGraphEdge>[])
        e.to: e.from,
    };
    final points = {
      for (final n in graph?.nodes ?? const <BridgeGraphNode>[])
        if (n.node case BridgeNodeRef_Effect(:final field0))
          for (final p in n.inputs)
            if (p.portType == BridgePortType.points)
              '$field0/${p.id}': (
                label: p.label,
                from: into[BridgeInputRef.param(node: n.node, port: p.id)],
              ),
    };
    if (!mapEquals(next, _driven) || !mapEquals(points, _points)) {
      setState(() {
        _driven = next;
        _points = points;
      });
    }
  }

  @override
  void dispose() {
    _share?.roster.removeListener(_onRoster);
    _filter.dispose();
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
    if (ui.activePanel != Panel.effectControls) return false;
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

  /// **`Ctrl+C` on this panel copies the picked effects**.
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
  /// window, which is asynchronous and has no place in a claim that
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
  late final Map<String, bool> _groupOpen =
      Provider.of<LumitUiState>(context, listen: false).folds.paramGroupsOpen;

  bool _isGroupOpen(String path, bool collapsedByDefault) =>
      _groupOpen[path] ?? !collapsedByDefault;

  void _toggleGroup(String path, bool collapsedByDefault) => setState(() {
        _folds += 1;
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
    // A node graph has no layers, so the picked box's rows go here instead.
    if (ui.model.isNodeGraph) return const NodePanelFrb();
    _theirs = _gatherTheirs(comp);

    // The one thing here that is in the tree whether or not the project is
    // shared. While it is not, it tells the rows to be built as they always
    // were.
    return ShareRows(
      marks: _theirs,
      onTouch: (_share?.active ?? false) ? _onRowTouched : null,
      child: _sections(context, ui, comp),
    );
  }

  Widget _sections(
      BuildContext context, LumitUiState ui, CompositionReference comp) {
    return ValueListenableBuilder<UuidValue?>(
      valueListenable: ui.selectedGroupHeader,
      builder: (context, groupId, _) => ValueListenableBuilder<LayerReference?>(
        valueListenable: ui.selectedLayer,
        builder: (context, layer, _) {
          // The layer last drawn was another project's, and went with it.
          if (_lastLayer?.internalprojectId != comp.internalproject) {
            _lastLayer = null;
          }
          // A picked group header is the subject the way a picked layer is:
          // its stack, its Add-effect, its parameter rows. The model
          // no longer holding the group — ungrouped, another comp fronted —
          // falls back to the layer subject rather than a dead card.
          if (groupId != null) {
            final body = _groupBody(context, comp, groupId);
            if (body != null) return body;
          }
          if (layer != null) _lastLayer = layer;
          final shown = layer ?? _lastLayer;
          if (shown == null) {
            return PlaceholderPanel(
              icon: LumitIcon.fx,
              title: l10n.effectControls,
              hint: l10n.effectControlsNoLayer,
            );
          }
          // One clip picked in the Cut timeline is the subject the way a
          // picked group header is: its own stack, its own Add effect.
          return _orOverlap(context, comp, ValueListenableBuilder<
              ({LayerReference layer, UuidValue clip})?>(
            valueListenable: ui.cutClip,
            builder: (context, pick, _) {
              final onClip =
                  pick != null && pick.layer.internalcompId == comp.internalid;
              // Where the editor reads a clip's stack from, set here and not
              // by the clip's own rows: the subject before this one is still
              // in the tree while it fades, and its rows rebuilding would
              // point the editor back at the clip that was left.
              if (onClip) {
                final (layer, clip) = (pick.layer, pick.clip);
                _effects.groupStack = null;
                _effects.clipStack =
                    () => layer.getClipEffects(clip: clip, onCompClock: true);
              }
              // Going between a layer's stack and a clip's is content
              // arriving in a place that was already there, and the stack
              // being left goes at once.
              final reveal = ThemeScope.of(context).motion.reveal;
              return AnimatedSwitcher(
                duration: reveal.duration,
                reverseDuration: Duration.zero,
                switchInCurve: reveal.curve,
                layoutBuilder: (current, previous) => Stack(
                  fit: StackFit.expand,
                  children: [...previous, if (current != null) current],
                ),
                child: KeyedSubtree(
                  key: ValueKey<String>(
                      onClip ? 'fx-subject-${pick.clip}' : 'fx-subject-layer'),
                  child: onClip
                      ? _clipBody(context, comp, pick.layer, pick.clip)
                      : _body(context, comp, shown),
                ),
              );
            },
          ));
        },
      ),
    );
  }

  /// [child], or the rows of the overlap the Cut timeline has fronted.
  Widget _orOverlap(
      BuildContext context, CompositionReference comp, Widget child) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return ValueListenableBuilder<({LayerReference layer, UuidValue clip})?>(
      valueListenable: ui.cutOverlap,
      builder: (context, overlap, _) =>
          overlap == null || overlap.layer.internalcompId != comp.internalid
              ? child
              : _overlapBody(context, comp, overlap.layer, overlap.clip),
    );
  }

  /// The Transition effect at a clip's start, read once per document revision.
  ({int? revision, UuidValue clip, BridgeEffectInstanceInfo? info})? _overlapRead;

  BridgeEffectInstanceInfo? _overlapInfo(
      LumitUiState ui, LayerReference layer, UuidValue clip) {
    final revision = ui.model.revision;
    final held = _overlapRead;
    if (held != null && held.revision == revision && held.clip == clip) {
      return held.info;
    }
    BridgeEffectInstanceInfo? info;
    try {
      info = layer.getClipTransition(clip: clip)?.getInfo();
    } catch (_) {
      // The clip went under the panel.
    }
    _overlapRead = (revision: revision, clip: clip, info: info);
    return info;
  }

  /// The panel with an **overlap** as its subject: its one effect's rows, less
  /// what the overlap decides for it, marked as a transition. When the clip goes
  /// or the overlap stops being one of the kinds, the panel goes back to what it
  /// showed.
  Widget _overlapBody(
    BuildContext context,
    CompositionReference comp,
    LayerReference layer,
    UuidValue clipId,
  ) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return ListenableBuilder(
      listenable: ui.model,
      builder: (context, _) {
        final entry = ui.model.byId(layer.internallayerId);
        BridgeClip? clip;
        for (final c in entry?.info.clips ?? const <BridgeClip>[]) {
          if (c.id == clipId) clip = c;
        }
        final fx = clip?.transition == null
            ? null
            : _overlapInfo(ui, layer, clipId);
        if (entry == null || clip == null || fx == null) {
          WidgetsBinding.instance
              .addPostFrameCallback((_) => ui.cutOverlap.value = null);
          return const SizedBox.shrink();
        }
        _effects.groupStack = null;
        _effects.clipStack = () => [
              if (layer.getClipTransition(clip: clipId) case final t?) t,
            ];
        var swept = const <String>{};
        var name = fx.name;
        for (final kind in transitionKinds) {
          if (kind.matchName == clip.transition) {
            swept = kind.sweptRows.toSet();
            name = engineLabel(kind.label);
          }
        }
        void changed() {
          Provider.of<LumitState>(context, listen: false)
              .notifyDocumentChanged();
          ui.model.refresh();
        }

        final first = clip.startFrame.toInt();
        final length = clip.endFrame.toInt() - first;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _Header(
              layerName: name,
              clipOn: entry.info.name,
              mark: l10n.effectControlsTransition,
              onMenu: (_) {},
            ),
            Expanded(
              child: FxKeyStripHost(
                spanStart: first,
                spanFrames: length < 1 ? 1 : length,
                child: ListView(
                  key: const ValueKey('fx-overlap-ground'),
                  padding: const EdgeInsets.symmetric(vertical: 4),
                  children: [
                    _clipEffectCard(
                        context, ui, comp, layer, clipId, fx, 0, changed,
                        stack: [fx], hideRows: swept),
                  ],
                ),
              ),
            ),
          ],
        );
      },
    );
  }

  /// A clip's effects as the cards draw them, read once per document
  /// revision and kept: on the comp's clock, so a card keys at the playhead
  /// as a layer's does.
  ({int? revision, UuidValue clip, List<BridgeEffectInstanceInfo> stack})?
      _clipRead;

  List<BridgeEffectInstanceInfo> _clipInfos(
      LumitUiState ui, LayerReference layer, UuidValue clip) {
    final revision = ui.model.revision;
    final held = _clipRead;
    if (held != null && held.revision == revision && held.clip == clip) {
      return held.stack;
    }
    var stack = const <BridgeEffectInstanceInfo>[];
    try {
      stack = [
        for (final instance
            in layer.getClipEffects(clip: clip, onCompClock: true))
          instance.getInfo(),
      ];
    } catch (_) {
      // The clip went under the panel, which then has no cards to draw.
    }
    _clipRead = (revision: revision, clip: clip, stack: stack);
    return stack;
  }

  /// The panel with one **clip** of a Sequence layer as its subject: the
  /// clip's own stack drawn with the cards a layer's is, and Add effect
  /// landing on the clip. A clip the model no longer holds falls back to its
  /// layer.
  Widget _clipBody(
    BuildContext context,
    CompositionReference comp,
    LayerReference layer,
    UuidValue clipId,
  ) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return ListenableBuilder(
      listenable: ui.model,
      builder: (context, _) {
        final entry = ui.model.byId(layer.internallayerId);
        BridgeClip? clip;
        for (final c in entry?.info.clips ?? const <BridgeClip>[]) {
          if (c.id == clipId) clip = c;
        }
        // The clip has gone: its layer's own rows. Not through [_body],
        // which would also tell the editor to forget the clip in hand now.
        if (entry == null || clip == null) {
          return _rows(context, comp, layer, ui.playheadFrame.value);
        }
        final stack = _clipInfos(ui, layer, clipId);
        final first = clip.startFrame.toInt();
        final length = clip.endFrame.toInt() - first;
        final span = (first, length < 1 ? 1 : length);
        void changed() {
          Provider.of<LumitState>(context, listen: false)
              .notifyDocumentChanged();
          ui.model.refresh();
        }

        void addToClip(String name) {
          try {
            layer.addClipEffect(clip: clipId, name: name);
          } catch (_) {
            // A driver, which a clip has no graph for: the engine refused
            // calmly and the menu simply did nothing.
          }
          changed();
        }

        _effects.clipInputs = null;
        final copy = clip.copy;
        final copyRows =
            copy == null ? null : _clipCopyRows(ui, layer, clipId);
        final copyCard = copy == null || copyRows == null
            ? null
            : _copyCard(
                context,
                ui,
                comp,
                [
                  CopyTarget(
                      layer: layer, clip: clipId, copy: copy, rows: copyRows),
                ],
                changed);

        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _Header(
              layerName: shownClipName(clip),
              clipOn: entry.info.name,
              onAdd: addToClip,
              onMenu: (at) => _showStackMenu(context, at,
                  effects: stack, onAdd: addToClip),
            ),
            // A picture clip's blend mode, beside the effects it carries.
            if (entry.info.kind != BridgeLayerKind.audio)
              _ClipBlendRow(
                blend: clip.blend,
                onChanged: (index) {
                  try {
                    layer.setClipBlend(clip: clipId, index: index);
                  } catch (_) {
                    // A locked layer refuses, and quietly.
                  }
                  changed();
                },
              ),
            Expanded(
              // The keyframe strip spans the clip, and the keys cross on the
              // comp's clock, so its lanes need no offset.
              child: FxKeyStripHost(
                spanStart: span.$1,
                spanFrames: span.$2,
                child: DragTarget<EffectDragData>(
                onAcceptWithDetails: (details) async {
                  await details.data.landOn(layer,
                      clip: clipId, atFrame: ui.playheadFrame.value);
                  if (mounted) changed();
                },
                builder: (context, candidate, _) => _DropGround(
                  lit: candidate.isNotEmpty,
                  child: GestureDetector(
                    behavior: HitTestBehavior.translucent,
                    onSecondaryTapUp: (d) => _showStackMenu(
                        context, d.globalPosition,
                        effects: stack, onAdd: addToClip),
                    child: ListView(
                      key: const ValueKey('fx-clip-ground'),
                      padding: const EdgeInsets.symmetric(vertical: 4),
                      children: [
                        // A clip of a graphics template: its card, above
                        // the clip's own effects.
                        if (copyCard != null) copyCard,
                        if (stack.isEmpty) _EmptyStack(onAdd: addToClip),
                        for (var i = 0; i < stack.length; i++)
                          _clipEffectCard(
                              context, ui, comp, layer, clipId, stack[i], i,
                              changed),
                      ],
                    ),
                  ),
                ),
              ),
              ),
            ),
          ],
        );
      },
    );
  }

  /// The staged rows of the clips that are copies of a graphics template,
  /// read once per document revision and kept: on the comp's clock, as a
  /// clip's effects are.
  ({int? revision, Map<UuidValue, BridgeEffectInstanceInfo?> rows})? _copyRead;

  BridgeEffectInstanceInfo? _clipCopyRows(
      LumitUiState ui, LayerReference layer, UuidValue clip) {
    final revision = ui.model.revision;
    var held = _copyRead;
    if (held == null || held.revision != revision) {
      held = _copyRead = (revision: revision, rows: {});
    }
    if (held.rows.containsKey(clip)) return held.rows[clip];
    BridgeEffectInstanceInfo? info;
    try {
      info = layer.getClipInputs(clip: clip, onCompClock: true)?.getInfo();
    } catch (_) {
      // The clip went under the panel, which then draws no card for it.
    }
    return held.rows[clip] = info;
  }

  /// [picks] as copies, when every one of them is a copy of one template.
  /// Empty otherwise.
  List<CopyTarget> _clipCopies(LumitUiState ui,
      List<({LayerReference layer, UuidValue clip})> picks) {
    final out = <CopyTarget>[];
    for (final pick in picks) {
      BridgeCopy? copy;
      for (final c in ui.model.byId(pick.layer.internallayerId)?.info.clips ??
          const <BridgeClip>[]) {
        if (c.id == pick.clip) copy = c.copy;
      }
      if (copy == null ||
          (out.isNotEmpty && out.first.copy.comp != copy.comp)) {
        return const [];
      }
      final rows = _clipCopyRows(ui, pick.layer, pick.clip);
      if (rows == null) return const [];
      out.add(CopyTarget(
          layer: pick.layer, clip: pick.clip, copy: copy, rows: rows));
    }
    return out;
  }

  /// The copies the template card edits while a layer is the subject: the
  /// clips picked in the Cut timeline when several are copies of one
  /// template, else the shown layer with every other selected layer that is
  /// a copy of the same template.
  List<CopyTarget> _copiesFor(
      LumitUiState ui, LayerReference layer, BridgeLayerInfo info) {
    final id = layer.internallayerId;
    // The picked clips, while the layer on show is one they are on: a pick
    // left behind by a Cut timeline since shut is not followed.
    final clips = _clipCopies(ui, ui.cutClips.value);
    if (clips.length > 1 &&
        clips.any((c) => c.layer.internallayerId == id)) {
      return clips;
    }
    final copy = info.copy;
    final rows = info.graphInputs;
    if (copy == null || rows == null) return const [];
    final picked = ui.selectedLayerIds;
    return [
      CopyTarget(layer: layer, copy: copy, rows: rows),
      if (picked.contains(id))
        for (final other in ui.model.layers)
          if (other.layer.internallayerId != id &&
              picked.contains(other.layer.internallayerId))
            if ((other.info.copy, other.info.graphInputs)
                case (final theirs?, final staged?)
                when theirs.comp == copy.comp)
              CopyTarget(layer: other.layer, copy: theirs, rows: staged),
    ];
  }

  /// The template card for [targets], the one it shows first, kept while
  /// what it draws from reads the same.
  Widget _copyCard(BuildContext context, LumitUiState ui,
      CompositionReference comp, List<CopyTarget> targets,
      VoidCallback changed) {
    final first = targets.first;
    final id = first.rows.id;
    final layer = first.layer;
    final clip = first.clip;
    // A clip's staged rows are found by the editor here, ahead of the clip's
    // own stack. A layer's are found as a placed graph's Inputs are.
    _effects.clipInputs = clip == null
        ? null
        : () => [
              if (layer.getClipInputs(clip: clip, onCompClock: true)
                  case final rows?)
                rows,
            ];
    // By the copy and not by its rows, whose id changes the first time the
    // copy stores a value: a box being typed in must keep its place.
    final name = 'copy-${clip ?? layer.internallayerId}';
    final open = _isOpen(name);
    final playhead = ui.playheadFrame.value;
    return _SameCard(
      key: ValueKey<String>('fx-card-$name'),
      reading: [
        for (final t in targets) ...[
          t.layer.internallayerId,
          t.clip,
          t.copy.comp,
          t.copy.name,
          [
            for (final c in t.copy.controls)
              [c.id, c.label, c.kind, c.group, c.rows, c.options, c.own, c.fed],
          ],
          t.rows.id,
          t.rows.values,
          t.rows.derivedParams,
          [
            for (final r in t.rows.rowOptions) [r.id, r.options],
          ],
        ],
        open,
        playhead,
        _folds,
        _filter.needle,
        comp.internalid,
        l10n.localeName,
        // What a live drag is holding, which is not in the document yet.
        [
          for (final p in first.rows.derivedParams)
            _effects.stagedValue(id, p.id),
        ],
      ],
      card: () => CopyCardFrb(
        targets: targets,
        comp: comp,
        editor: _effects,
        open: open,
        onToggle: () => _toggle(name),
        isGroupOpen: _isGroupOpen,
        onToggleGroup: _toggleGroup,
        allLayers: ui.model.layers,
        playheadFrame: playhead,
        onSeek: (frame) => ui.playheadFrame.value = frame,
        onLive: (effect, param, value) => setState(() {
          _effects.live(comp, layer, effect, param, value,
              frame: ui.playheadFrame.value, scale: ui.viewerScale);
        }),
        onChanged: changed,
        filter: _filter.needle,
      ),
    );
  }

  /// One clip effect's card: the layer card with the clip as the list its
  /// commands read. Like a group header's, it takes no part in the effect
  /// selection, and its heading only twirls.
  Widget _clipEffectCard(
    BuildContext context,
    LumitUiState ui,
    CompositionReference comp,
    LayerReference layer,
    UuidValue clip,
    BridgeEffectInstanceInfo fx,
    int index,
    VoidCallback changed, {
    List<BridgeEffectInstanceInfo>? stack,
    Set<String> hideRows = const {},
  }) {
    return _EffectSection(
      key: ValueKey<String>('fx-card-clip-$index'),
      hideRows: hideRows,
      info: fx,
      graphName: _graphNameOf(context, fx),
      clip: clip,
      open: _isOpen('fx-${fx.id}'),
      onToggle: () => _toggleEffect(fx.id, ui.selectedEffects.value),
      onFoldAll: (open) =>
          _foldAll(stack ?? _clipInfos(ui, layer, clip), open),
      driven: const {},
      stagedValue: _effects.stagedValue,
      onStackChanged: changed,
      onWrite: (id, param, value) {
        _effects.write(layer, id, param, value);
        changed();
      },
      onWritePair: (id, values) {
        _effects.writeAll(layer, id, values);
        changed();
      },
      onLive: (id, param, value) => setState(() {
        _effects.live(comp, layer, id, param, value,
            frame: ui.playheadFrame.value, scale: ui.viewerScale);
      }),
      onSelect: () {},
      layer: layer,
      allLayers: ui.model.layers,
      comp: comp,
      playheadFrame: ui.playheadFrame.value,
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
              layer: layer,
              effect: effect,
              param: param,
              frame: ui.playheadFrame.value);
        } catch (_) {
          // Refused; the effect's own status line says why.
        }
        setState(() => _actionPressed += 1);
      },
    );
  }

  /// The panel with a **group header** as its subject
  /// (docs/impl/group-effects.md §6): the header's own stack drawn with the
  /// same cards a layer's is, and the Add-effect road targeting the group.
  /// Null when the model no longer holds the group, or it has no member left
  /// to route writes through.
  Widget? _groupBody(
    BuildContext context,
    CompositionReference comp,
    UuidValue groupId,
  ) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return ListenableBuilder(
      listenable: ui.model,
      builder: (context, _) {
        BridgeLayerGroup? group;
        for (final g in ui.model.groups) {
          if (g.id == groupId) group = g;
        }
        // The carrier member is the write road: the engine's shared instance
        // lookup routes a group instance from any of the comp's layers.
        final carrier = group == null || group.members.isEmpty
            ? null
            : ui.model.byId(group.members.first);
        if (group == null || carrier == null) {
          // Fall back to the layer subject on the next frame — clearing here
          // inside build would notify mid-build.
          return _lastLayer == null
              ? PlaceholderPanel(
                  icon: LumitIcon.fx,
                  title: l10n.effectControls,
                  hint: l10n.effectControlsNoLayer,
                )
              : _body(context, comp, _lastLayer!);
        }
        final layer = carrier.layer;
        final g = group;
        // The shared editor's third place to read a stack from, while the
        // header is the subject.
        final gid = g.id;
        _effects.groupStack = () => comp.getGroupEffects(group: gid);
        _effects.clipStack = null;
        void addToGroup(String name) {
          try {
            comp.addGroupEffect(group: gid, name: name);
          } catch (_) {
            // A driver, or a name this build does not know: the engine refused
            // calmly and the menu simply did nothing.
          }
          ui.model.refresh();
        }

        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _Header(
              layerName: g.name,
              onAdd: addToGroup,
              onMenu: (at) => _showStackMenu(context, at,
                  effects: g.effects, onAdd: addToGroup),
            ),
            Expanded(
              child: DragTarget<EffectDragData>(
                onAcceptWithDetails: (details) {
                  try {
                    comp.addGroupEffect(group: gid, name: details.data.name);
                  } catch (_) {}
                  ui.model.refresh();
                },
                builder: (context, candidate, _) => _DropGround(
                  lit: candidate.isNotEmpty,
                  child: GestureDetector(
                    key: const ValueKey('fx-group-ground'),
                    behavior: HitTestBehavior.translucent,
                    onTap: ui.clearEffectSelection,
                    onSecondaryTapUp: (d) => _showStackMenu(
                        context, d.globalPosition,
                        effects: g.effects, onAdd: addToGroup),
                    child: ListView(
                      padding: const EdgeInsets.symmetric(vertical: 4),
                      children: [
                        if (g.effects.isEmpty) _EmptyStack(onAdd: addToGroup),
                        for (var i = 0; i < g.effects.length; i++)
                          _groupEffectCard(context, ui, comp, layer, g, i),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ],
        );
      },
    );
  }

  /// One header effect's card: the layer card with the group as the
  /// list its commands read — like a style's, it takes no part in the effect
  /// selection, and its heading only twirls.
  Widget _groupEffectCard(
    BuildContext context,
    LumitUiState ui,
    CompositionReference comp,
    LayerReference layer,
    BridgeLayerGroup group,
    int index,
  ) {
    final fx = group.effects[index];
    return _EffectSection(
      key: ValueKey<String>('fx-card-group-$index'),
      info: fx,
      graphName: _graphNameOf(context, fx),
      group: group.id,
      open: _isOpen('fx-${fx.id}'),
      onToggle: () => _toggleEffect(fx.id, ui.selectedEffects.value),
      onFoldAll: (open) => _foldAll(group.effects, open),
      driven: const {},
      stagedValue: _effects.stagedValue,
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
            frame: ui.playheadFrame.value, scale: ui.viewerScale);
      }),
      onSelect: () {},
      layer: layer,
      allLayers: ui.model.layers,
      comp: comp,
      playheadFrame: ui.playheadFrame.value,
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
              layer: layer,
              effect: effect,
              param: param,
              frame: ui.playheadFrame.value);
        } catch (_) {
          // Refused; the effect's own status line says why.
        }
        setState(() => _actionPressed += 1);
      },
    );
  }

  Widget _body(
    BuildContext context,
    CompositionReference comp,
    LayerReference layer,
  ) {
    // A layer subject reads no group's stack and no clip's: the editor's
    // third place to look is only armed while one of those is the subject.
    _effects.groupStack = null;
    _effects.clipStack = null;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    // **The panel does not listen to the playhead.** Every row that reads it —
    // a value under the playhead, a diamond that fills on a key — carries its
    // own `ValueListenableBuilder` on it, the way the Timeline's fold rows do,
    // so a scrub redraws those rows and nothing else. Listening here instead
    // rebuilt the whole panel on every frame of a scrub: measured at 312
    // widgets per playhead move on a three-layer project, which is what made
    // the playhead lag the pointer. The frame below is therefore a *snapshot*
    // for the few rows that cannot listen for themselves, and those are wrapped
    // in [_AtPlayhead] where they are built.
    //
    // Which effects are picked is the shell's — the Timeline picks them
    // too — so the headings redraw when that changes, wherever the click
    // happened. The read model repaints the panel when anything commits:
    // an undo, a redo, or the same property dragged in the Timeline.
    return ListenableBuilder(
      listenable: ui.model,
      builder: (context, _) =>
          _rows(context, comp, layer, ui.playheadFrame.value),
    );
  }

  Widget _rows(
    BuildContext context,
    CompositionReference comp,
    LayerReference layer,
    int playhead,
  ) {
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
    // One reading of the comp's other layers for every card on the panel.
    final layersRead = _layersRead(ui.model.layers);
    // An add lands on the layer these rows are for, and no other.
    void addToLayer(String name) => ui.addEffect(layer, name);
    void stackMenu(Offset at) => _showStackMenu(context, at,
        effects: info.effects, onAdd: addToLayer, layer: layer);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _Header(
          layerName: shownLayerName(info),
          onAdd: addToLayer,
          onMenu: stackMenu,
          onFilter: _filter.show,
          filter: _filter.open
              ? rowFilterWell(context, _filter, key: const ValueKey('fx-filter'))
              : null,
        ),
        Expanded(
          // The keyframe strip beside the rows, spanning the layer's own in
          // and out points. The rows' lanes draw into the column it opens.
          child: FxKeyStripHost(
            spanStart: info.inFrame,
            spanFrames: info.outFrame - info.inFrame < 1
                ? 1
                : info.outFrame - info.inFrame,
          // The drop target for an effect dragged from Effects & presets.
          // Nothing else produces an `EffectDragData`; the Timeline's layer
          // rows and the graph panels are the other places that take one.
          child: DragTarget<EffectDragData>(
            onAcceptWithDetails: (details) async {
              await details.data.landOn(layer, atFrame: ui.playheadFrame.value);
              ui.model.refresh();
            },
            builder: (context, candidate, _) => _DropGround(
              // While something is over it, say so: a drop with no feedback is
              // indistinguishable from a drop that did nothing.
              lit: candidate.isNotEmpty,
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
                onSecondaryTapUp: (d) => stackMenu(d.globalPosition),
                child: ListView(
                  padding: const EdgeInsets.symmetric(vertical: 4),
                  children: [
                    // A copy of a graphics template: its card first, above
                    // everything the layer itself has. It follows the
                    // selection, as it edits every selected copy at once.
                    ListenableBuilder(
                      listenable:
                          Listenable.merge([ui.selectedLayers, ui.cutClips]),
                      builder: (context, _) {
                        final copies = _copiesFor(ui, layer, info);
                        if (copies.isEmpty) {
                          _effects.clipInputs = null;
                          return const SizedBox.shrink();
                        }
                        return _copyCard(
                            context, ui, comp, copies, ui.model.refresh);
                      },
                    ),
                    // Source (a text layer's words, a solid's colour) and Retime
                    // ride with Transform behind the same choice: all three are
                    // the *layer*, and this panel is about the effects on it.
                    // Settings → Interface brings them back together. The
                    // filter reaches Transform's rows either way.
                    if (ui.workspace.interface.transformInEffectControls ||
                        _filter.needle.isNotEmpty) ...[
                      // What the layer is made of comes before where it sits: a
                      // text layer's words are the first thing you want when
                      // you select one.
                      // The filter names rows, and Source, Flow and the
                      // Animators have none to name: they step aside while
                      // it holds text.
                      if (_filter.needle.isEmpty)
                      SourceRowsFrb(
                        key: ValueKey<String>(
                            'src-card-${layer.internallayerId}'),
                        layer: layer,
                        onChanged: ui.model.refresh,
                        open: _isOpen('source'),
                        onToggle: () => _toggle('source'),
                      ),
                      // Flow sits between what the layer is made of and where it
                      // sits, because that is what it is: how the source is
                      // *sampled*. It shows itself only when the layer's
                      // flow switch is on.
                      if (_filter.needle.isEmpty)
                      _AtPlayhead(
                        builder: (context, at) => FlowRowsFrb(
                          key: ValueKey<String>(
                              'flow-card-${layer.internallayerId}'),
                          layer: layer,
                          onChanged: ui.model.refresh,
                          comp: comp,
                          playheadFrame: at,
                          onSeek: (frame) => ui.playheadFrame.value = frame,
                          open: _isOpen('flow'),
                          onToggle: () => _toggle('flow'),
                        ),
                      ),
                      _TransformSection(
                        key: ValueKey<String>(
                            'tf-card-${layer.internallayerId}'),
                        layer: layer,
                        comp: comp,
                        transform: info.transform,
                        axisModes: info.axisModes,
                        // A camera is 3D by construction whatever its switch
                        // says — its z and rotation rows must always
                        // draw. Decided here from the model the panel already
                        // holds, not by asking the engine per rebuild.
                        threeD: info.switches.threeD ||
                            info.kind == BridgeLayerKind.camera,
                        kind: meshKindOf(info),
                        twoNode: info.camera?.twoNode ?? false,
                        light: info.light,
                        acceptsLights: info.switches.acceptsLights,
                        material: info.material,
                        corrected: info.trackCorrected,
                        playheadFrame: playhead,
                        onSeek: (frame) => ui.playheadFrame.value = frame,
                        onChanged: ui.model.refresh,
                        open: _isOpen('transform'),
                        onToggle: () => _toggle('transform'),
                        filter: _filter.needle,
                      ),
                    ],
                    // **The letters, one at a time**, and outside the
                    // choice above on purpose. Transform, Source and Retime move
                    // between this panel and the Timeline's fold because they
                    // exist in both; the Animators section has no Timeline home
                    // yet, so hiding it with them would hide the whole feature
                    // from anybody who has not turned the layer cards on. It
                    // shows itself only where there is something to show: a
                    // layer that is not a Text layer has no letters to animate.
                    // Its rows listen to the playhead one at a time, so the card
                    // itself does not have to.
                    if (_filter.needle.isEmpty)
                    TextAnimatorRowsFrb(
                      key: ValueKey<String>(
                          'anim-card-${layer.internallayerId}'),
                      layer: layer,
                      solid: meshKindOf(info) == BridgeLayerKind.model,
                      onChanged: ui.model.refresh,
                      comp: comp,
                      playheadFrame: playhead,
                      onSeek: (frame) => ui.playheadFrame.value = frame,
                      open: _isOpen('animators'),
                      onToggle: () => _toggle('animators'),
                    ),
                    // A placed node graph's Inputs, above the stack because
                    // they make the picture the effects then process.
                    if (info.copy == null &&
                        (info.graphInputs?.derivedParams.isNotEmpty ?? false))
                      _effectCard(context, ui, comp, layer, info, 0,
                          layersRead: layersRead, inputs: true),
                    // A Model layer's materials, before the effects that run
                    // on the picture they make.
                    if (info.materials case final materials?
                        when materials.isNotEmpty) ...[
                      if (!isGeometryOptions(materials.first))
                        _groupHeading(
                            context, 'materials-group', l10n.foldMaterials),
                      if (isGeometryOptions(materials.first) ||
                          _isOpen('materials-group'))
                        for (var i = 0; i < materials.length; i++)
                          _effectCard(context, ui, comp, layer, info, i,
                              layersRead: layersRead,
                              style: true,
                              material: true),
                    ],
                    if (info.effects.isEmpty)
                      _EmptyStack(onAdd: addToLayer)
                    else ...[
                      // The rack sits under its own Audio heading at the foot
                      // of the stack (AP5, "the stack is the rack"): an audio
                      // plugin is an ordinary stack entry, so its card is the
                      // same card, and only where it is *listed* changes. The
                      // indices stay stack indices, which is what keeps a drag
                      // within the group a reorder of the chain.
                      for (final index
                          in _stackIndices(info.effects, audio: false))
                        _effectCard(context, ui, comp, layer, info, index,
                            layersRead: layersRead),
                      if (_stackIndices(info.effects, audio: true)
                          case final rack when rack.isNotEmpty) ...[
                        _groupHeading(
                            context, 'audio-group', l10n.workspaceAudio),
                        if (_isOpen('audio-group'))
                          for (final index in rack)
                            _effectCard(context, ui, comp, layer, info, index,
                                layersRead: layersRead),
                      ],
                    ],
                    // Styles under the stack, because that is where they render
                    // (docs/impl/layer-styles.md §3) — and only once the layer
                    // wears one: an empty heading is a promise the row cannot
                    // keep, and the Layer menu is where the first one is added.
                    if (info.styles.isNotEmpty) ...[
                      _stylesGroupHeading(context, ui, layer, info),
                      if (_isOpen('styles-group'))
                        for (var i = 0; i < info.styles.length; i++)
                          _effectCard(context, ui, comp, layer, info, i,
                              layersRead: layersRead, style: true),
                    ],
                  ],
                ),
              ),
            ),
          ),
          ),
        ),
      ],
    );
  }

  /// The stack positions of the audio-typed entries, or of everything else —
  /// two walks of a list the model already holds, no bridge call.
  List<int> _stackIndices(List<BridgeEffectInstanceInfo> effects,
          {required bool audio}) =>
      [
        for (var i = 0; i < effects.length; i++)
          if (effects[i].audio == audio) i,
      ];

  /// One twirling heading over a group of cards — the Audio rack's and the
  /// Styles fold's, which are the same row: a twirl, a word, and at most one
  /// command at the far end.
  ///
  /// [storageKey] is both what the fold is remembered under and what the row is
  /// keyed by, so a heading cannot be open under one name and keyed by another.
  Widget _groupHeading(
    BuildContext context,
    String storageKey,
    String label, {
    Widget? trailing,
  }) {
    final t = ThemeScope.of(context).theme;
    final open = _isOpen(storageKey);
    return HoverMark(
      builder: (context, lit) {
        final ink = Color.lerp(t.textMuted, t.textPrimary, lit)!;
        return GestureDetector(
          key: ValueKey<String>('fx-$storageKey'),
          behavior: HitTestBehavior.opaque,
          onTap: () => _toggle(storageKey),
          child: Padding(
            padding: const EdgeInsetsDirectional.fromSTEB(4, 8, 10, 2),
            child: Row(
              children: [
                TwirlTurn(
                  open: open,
                  child: lumitIcon(
                    open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                    size: iconSize,
                    color: ink,
                  ),
                ),
                const SizedBox(width: 2),
                Expanded(
                  child: Text(
                    label,
                    style: t.small.copyWith(color: ink),
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                if (trailing != null) trailing,
              ],
            ),
          ),
        );
      },
    );
  }

  /// The **Styles** heading over the layer's styles
  /// (docs/impl/layer-styles.md §6), twirling like the Audio one, with the
  /// add-a-style menu on its own button.
  ///
  /// The menu lists the seven Lumit draws, each greyed once the layer wears it
  /// — the same rule the Layer menu's rows follow, because it is the engine's:
  /// `add_style` refuses a second copy of a style, and a menu that offered one
  /// would be offering a refusal.
  Widget _stylesGroupHeading(
    BuildContext context,
    LumitUiState ui,
    LayerReference layer,
    BridgeLayerInfo info,
  ) {
    final t = ThemeScope.of(context).theme;
    final worn = {for (final s in info.styles) s.name};
    return _groupHeading(
      context,
      'styles-group',
      l10n.foldStyles,
      trailing: LumitTooltip(
        message: l10n.tipAddLayerStyle,
        child: HouseButton(
          key: const ValueKey<String>('fx-add-style'),
          frameless: true,
          small: true,
          padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 2),
          onPressed: worn.length < offeredStyles().length
              ? () => _addStyleMenu(context, ui, layer, worn)
              : null,
          child: glyph.LumitIcon(LumitIcons.addEffect,
              size: 12,
              colour: worn.length < offeredStyles().length
                  ? t.textMuted
                  : t.textDisabled),
        ),
      ),
    );
  }

  /// The add-a-style popup: the ones a menu may offer, in §2's order.
  void _addStyleMenu(BuildContext context, LumitUiState ui,
      LayerReference layer, Set<String> worn) {
    final box = context.findRenderObject() as RenderBox?;
    final at = box == null
        ? Offset.zero
        : box.localToGlobal(box.size.bottomLeft(Offset.zero));
    showLumitPopup<void>(
      context: context,
      position: at,
      builder: (close) => FloatSurface(
        width: 190,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // Only the styles this layer can still take. A menu of dead rows
            // tells you what you cannot do, which is not what a menu is for
            // (docs/15, no punishment UI) — and the Layer menu, whose rows do
            // grey, is where the whole family is always visible.
            for (final style in offeredStyles())
              if (!worn.contains(style.name))
                MenuRow(
                  key: ValueKey<String>('fx-add-style-${style.name}'),
                  onPressed: () {
                    close(null);
                    try {
                      layer.addStyle(name: style.name);
                    } catch (_) {
                      // The style arrived from somewhere else between the menu
                      // opening and the row being chosen; a thrown error about
                      // a style the layer now has helps nobody.
                    }
                    Provider.of<LumitState>(context, listen: false)
                        .notifyDocumentChanged();
                    ui.model.refresh();
                  },
                  child: Text(engineLabel(style.label)),
                ),
          ],
        ),
      ),
    );
  }

  /// One effect's card, wherever it is listed — under the stack or under the
  /// Audio heading. `index` is the effect's **stack** position, which is what
  /// every command on the card acts through.
  ///
  /// With `style` set it draws a **layer style** instead: the same
  /// card, indexing the layer's other list. A style takes no part in the effect
  /// selection — Copy, Paste, Delete and the reorder all act on the stack, and
  /// nine fixed slots can do none of the four — so it is drawn unpicked and its
  /// heading only twirls.
  ///
  /// With `inputs` set it draws a placed graph's **Inputs** the same way, and
  /// `index` is not read: there is only one.
  Widget _effectCard(
    BuildContext context,
    LumitUiState ui,
    CompositionReference comp,
    LayerReference layer,
    BridgeLayerInfo info,
    int index, {
    required List<Object?> layersRead,
    bool style = false,
    bool inputs = false,
    bool material = false,
  }) {
    final playhead = ui.playheadFrame.value;
    final fx = inputs
        ? info.graphInputs!
        : material
            ? info.materials![index]
            : style
                ? info.styles[index]
                : info.effects[index];
    // The Inputs card is headed Inputs: the graph's own name is the layer's.
    // A material its file gives no name is numbered.
    final graphName = inputs
        ? l10n.foldInputs
        : material
            ? l10n.modelMaterialNumbered(index + 1)
            : _graphNameOf(context, fx);
    // Neither a style nor the Inputs takes part in the effect selection.
    final fixed = style || inputs;
    // A model with many materials starts with each closed, so its first
    // look is a list of names rather than a wall of rows. Geometry options
    // start closed on every Text and Shape layer, so selecting a layer never
    // asks the engine for their rows. A card closed from the start draws none
    // until it's first opened, then keeps them for the twirl to close over.
    // A 3D shape's are what it is, so they start open.
    final startsClosed = material &&
        (info.materials!.length > 3 || fx.name == 'geometry_options');
    // A Clone deformer reads the Clone to points above it, which is in the
    // stack rather than on its card.
    final vectorReason =
        !style && !inputs && !material && fx.name == 'clone_deformer'
            ? cloneVectorReason(info.effects, index, ui.model.layers)
            : null;
    final open =
        startsClosed ? _shut.contains('fx-${fx.id}') : _isOpen('fx-${fx.id}');
    if (open) _everOpen.add(fx.id);
    final drawsRows = !startsClosed || _everOpen.contains(fx.id);
    final renaming = _renamingEffect == fx.id;
    // Every row this instance draws, which is also what says whether it lists
    // the comp's layers.
    final params = instanceParameters(fx);
    // The row filter: a heading that holds the text keeps every row, a card
    // with no row that does is left out altogether.
    final needle = _filter.needle;
    var filter = '';
    if (needle.isNotEmpty) {
      // The heading's name is looked up only while filtering: this runs for
      // every card on every selection, and the catalogue walk is not free.
      final heading = (fx.customName ?? graphName ?? effectLabelOf(fx.name))
          .toLowerCase();
      filter = heading.contains(needle) ? '' : needle;
      if (filter.isNotEmpty &&
          !params.any(
              (p) => engineLabel(p.label).toLowerCase().contains(filter))) {
        return const SizedBox.shrink();
      }
    }
    // An LFX effect with a page of its own, which the heading opens.
    final interface =
        !fixed && (ui.extensions.carrying(fx.name)?.$2.interface_ ?? false);
    return _SameCard(
      key: ValueKey<String>(
          'fx-card-${material ? 'material-' : style ? 'style-' : ''}${fx.id}'),
      // Everything the card draws from. Read [_SameCard] before adding to the
      // card below: a fact that is not in here is one the card may not hold.
      reading: [
        fx.id,
        fx.name,
        fx.customName,
        fx.enabled,
        fx.audio,
        fx.values,
        fx.linkedPairs,
        fx.badgeReason,
        fx.badgeDetail,
        fx.derivedParams,
        fx.hiddenRows,
        fx.disabledRows,
        [
          for (final r in fx.rowOptions) [r.id, r.options],
        ],
        fx.nodeGraphComp,
        info.trackCorrected,
        vectorReason,
        graphName,
        open,
        drawsRows,
        renaming,
        style,
        inputs,
        material,
        interface,
        playhead,
        _actionPressed,
        _folds,
        _driven,
        _points,
        filter,
        layer.internallayerId,
        comp.internalid,
        ui.workspace.themedEffectGraphs,
        ui.workspace.curvePlotSize,
        // The language every label on the card is in. It is a global rather
        // than something inherited, so a card that kept its build would keep
        // the words it was built with (l10n/strings.dart).
        l10n.localeName,
        // What a live drag is holding, which is not in the document yet.
        [for (final p in params) _effects.stagedValue(fx.id, p.id)],
        // A row that lists the comp's layers draws names from outside this
        // card, so a card with one follows every re-read of the model.
        if (params.any(_listsLayers)) layersRead,
      ],
      card: () => _EffectSection(
        info: fx,
        vectorReason: vectorReason,
        graphName: graphName,
        style: style,
        inputs: inputs,
        material: material,
        // A card narrowed to the rows that match shows them, shut or not.
        open: open || filter.isNotEmpty,
        filter: filter,
        drawsRows: drawsRows,
        onToggle: () => _toggleEffect(fx.id, ui.selectedEffects.value),
        onFoldAll: (open) => _foldAll(
            ui.model.byId(layer.internallayerId)?.info.effects ?? info.effects,
            open),
        // The selection this heading follows, rather than an answer read here
        // and handed down: a pick recolours the heading and rebuilds nothing
        // above it. A style takes no part in the effect selection, so its
        // heading only twirls.
        pick: fixed
            ? null
            : FxPick(
                selection: ui.pickedEffects,
                layer: layer.internallayerId.toString(),
                effect: fx.id,
              ),
        driven: _driven,
        points: _points,
        renaming: renaming,
        onRenamed: (name) {
          setState(() => _renamingEffect = null);
          EffectCommands(
            info: fx,
            layer: layer,
            comp: comp,
            style: style,
            onStackChanged: ui.model.refresh,
          ).rename(name);
        },
        // Escape: close the editor, write nothing.
        onRenameCancelled: () => setState(() => _renamingEffect = null),
        onStartRename:
            inputs || material
                ? null
                : () => setState(() => _renamingEffect = fx.id),
        onSelect: () {
          if (fixed) return;
          // The run a Shift-click reaches over is read at the click. A card is
          // not rebuilt when a neighbour is removed, so an order held from the
          // last build could name an effect that has gone.
          final stack = ui.model.byId(layer.internallayerId)?.info.effects ??
              info.effects;
          ui.pickEffect(
            layer,
            fx.id,
            order: [for (final e in stack) e.id],
          );
          // **Double-clicking a Custom shader's heading enters its inner
          // graph** — the heading and the Graph panel's box are one
          // selection, so they are one door. The first click still picks,
          // exactly as it did.
          if (fx.name == 'custom_shader' &&
              _headingTaps.putIfAbsent(fx.id, DoubleTap.new).tap()) {
            ui.enterShaderGraph(layer, fx.id,
                effectName: fx.customName ?? effectLabelOf(fx.name));
          }
        },
        onOpenInterface: interface
            ? () => showLfxInterface(context,
                matchName: fx.name, title: effectLabelOf(fx.name))
            : null,
        onOpenShaderGraph: fx.name == 'custom_shader' && !style
            ? () => ui.enterShaderGraph(layer, fx.id,
                effectName: fx.customName ?? effectLabelOf(fx.name))
            : null,
        stagedValue: _effects.stagedValue,
        trackCorrected: info.trackCorrected,
        onStackChanged: ui.model.refresh,
        onWrite: (id, param, value) {
          _effects.write(layer, id, param, value);
          ui.model.refresh();
          if (!fixed) pickForPoint(ui, layer, fx, param);
        },
        onWritePair: (id, values) {
          _effects.writeAll(layer, id, values);
          ui.model.refresh();
          if (!fixed) pickForPoint(ui, layer, fx, values.keys.first);
        },
        onLive: (id, param, value) {
          if (!fixed) pickForPoint(ui, layer, fx, param);
          setState(() {
            _effects.live(comp, layer, id, param, value,
                frame: ui.playheadFrame.value, scale: ui.viewerScale);
          });
        },
        layer: layer,
        // The list a layer-valued row picks from. A card that draws one is
        // rebuilt whenever [_layersRead] moves, so the list it holds says the
        // same as the comp does.
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
          // The Custom shader's two buttons are the frontend's own. Every other
          // Action row goes back to the engine as an event, which is what the
          // kind is.
          if (fx.name == 'custom_shader' &&
              pressShaderButton(
                context: context,
                home: _shaderHome(layer),
                effect: effect,
                param: param,
                onApplied: ui.model.refresh,
              )) {
            return;
          }
          // Edit lens… opens the lens window, shared by both lens effects.
          if (pressLensButton(
            context: context,
            home: _shaderHome(layer),
            effect: effect,
            param: param,
            onApplied: ui.model.refresh,
          )) {
            return;
          }
          // Edit flare… opens the Flare window on a Sprite flare.
          if (pressFlareButton(
            context: context,
            home: _shaderHome(layer),
            effect: effect,
            param: param,
            onApplied: ui.model.refresh,
            comp: ui.selectedComp,
            frame: ui.playheadFrame.value,
          )) {
            return;
          }
          // The Node graph's Open graph row is the frontend's own the same
          // way: fronting a comp is not an event the engine could answer.
          if (fx.name == 'node_graph' && param == 'open') {
            _openGraphOn(layer, effect);
            return;
          }
          try {
            fireEffectAction(
                layer: layer,
                effect: effect,
                param: param,
                frame: ui.playheadFrame.value);
          } catch (_) {
            // Refused — another analysis is already running, or the media
            // cannot be read. The effect's own status line says which; a thrown
            // error here would be a dialogue over a button press.
          }
          setState(() => _actionPressed += 1);
        },
      ),
    );
  }

  /// **What the panel's layer-valued rows can draw from the rest of the
  /// comp**: the names a picker lists, and the masks and clips inside them.
  ///
  /// Read once per rebuild and **held while it reads the same**, so a card
  /// compares one object instead of walking the comp. Every effect carries a
  /// matte row, so without this every card would follow every edit made
  /// anywhere in the comp — which is the whole of what [_SameCard] is for.
  ///
  /// Not in here: whether a layer has a picture, which only a bridge call
  /// answers and which the picker asks for itself when it is opened.
  List<Object?> _layersRead(List<BridgeLayerEntry> layers) {
    final read = <Object?>[
      for (final entry in layers) ...[
        entry.layer.internallayerId,
        entry.info.name,
        for (final mask in entry.info.masks) ...[mask.id, mask.name],
        for (final clip in entry.info.clips) ...[
          clip.id,
          clip.sourceName,
          clip.startFrame,
        ],
      ],
    ];
    final held = _layersHeld;
    if (held != null && _sameReading(held, read)) return held;
    return _layersHeld = read;
  }

  List<Object?>? _layersHeld;

  /// Does this row list the comp's **layers**? A Layer, a Mask path and a
  /// Clip parameter all name something from outside the effect they sit on,
  /// which is why a card holding one cannot be handed back unchanged when the
  /// rest of the comp has moved.
  static bool _listsLayers(BridgeParamInfo param) =>
      param.kind is BridgeParamKind_Layer ||
      param.kind is BridgeParamKind_MaskPath ||
      param.kind is BridgeParamKind_Clip;
}

/// One effect's card and **the reading it was built from**, so a rebuild of
/// the panel that says nothing new about this effect costs nothing.
///
/// Every edit is a document revision, so the panel is rebuilt from the read
/// model — and it used to hand the framework a freshly built card for every
/// effect on the layer. Removing one effect from a four-effect stack redrew
/// 2258 widgets, and the figure grew with the stack. The card here is built
/// only when [reading] moves; the rest of the time the *same instance* is
/// handed back, which `Element.updateChild` short-circuits — no rebuild, and
/// no layout either. It is the trick the Timeline's blocks use for a scroll
/// (docs/impl/ui-performance.md §4.3), decided by what was read rather than
/// by identity, because the model hands out fresh objects on every read.
///
/// **What that asks of the card.** The card left on screen is the one built
/// from the *old* reading, closures and all, so nothing inside it may hold a
/// fact that is missing from [reading]. A position in the stack is the one
/// that bites — it moves when a neighbour goes — so every command that needs
/// one reads it at the gesture (`_EffectSection._place`) instead.
class _SameCard extends StatefulWidget {
  const _SameCard({super.key, required this.reading, required this.card});

  final List<Object?> reading;

  /// The card itself, built on demand: a reading that has not moved never
  /// calls this.
  final Widget Function() card;

  @override
  State<_SameCard> createState() => _SameCardState();
}

class _SameCardState extends State<_SameCard> {
  Widget? _card;

  @override
  void didUpdateWidget(covariant _SameCard old) {
    super.didUpdateWidget(old);
    if (!_sameReading(widget.reading, old.reading)) _card = null;
  }

  @override
  Widget build(BuildContext context) => _card ??= widget.card();
}

/// Two readings, compared a list deep.
///
/// The read model builds a fresh list for every value it holds, so two
/// readings of an effect nobody touched are equal in content and never the
/// same object. Everything else is compared with `==` — the generated types
/// carry it, and the panel's own state (the driven map) is replaced only when
/// it differs, so identity is the honest answer there.
bool _sameReading(List<Object?> a, List<Object?> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    final one = a[i];
    final other = b[i];
    if (one is List && other is List) {
      if (!_sameReading(one, other)) return false;
    } else if (one != other) {
      return false;
    }
  }
  return true;
}

/// Rebuild just this much of the panel when the playhead moves.
///
/// Most rows here listen to the playhead for themselves (`EffectParamRowFrb`,
/// `KeyframeControlsFrb`, the transform rows). A few cannot: they sample a
/// curve in their own `build`, or — like the levels trace — ask the engine for
/// something new from `didUpdateWidget`, which only runs when the frame arrives
/// as a *property*. Those are built through this, so they keep following a
/// scrub without the panel above them following it too.
class _AtPlayhead extends StatelessWidget {
  const _AtPlayhead({required this.builder});

  final Widget Function(BuildContext context, int frame) builder;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<int>(
        valueListenable:
            Provider.of<LumitUiState>(context, listen: false).playheadFrame,
        builder: (context, frame, _) => builder(context, frame),
      );
}

/// The ground an effect is dropped on, lit while one is over it. It lights up
/// as the dock's own drop preview does, and the edge is drawn over [child] so
/// nothing in it shifts.
class _DropGround extends StatelessWidget {
  final bool lit;
  final Widget child;

  const _DropGround({required this.lit, required this.child});

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final accent = scope.theme.accent;
    final step = scope.motion.dropZone;
    return AnimatedContainer(
      duration: step.duration,
      curve: step.curve,
      decoration:
          BoxDecoration(color: accent.withValues(alpha: lit ? 0.06 : 0)),
      foregroundDecoration: BoxDecoration(
          border: Border.all(color: accent.withValues(alpha: lit ? 1 : 0))),
      child: child,
    );
  }
}

/// A clip's own blend mode under the header: **From layer** while it has
/// none, then the engine's list.
class _ClipBlendRow extends StatelessWidget {
  final int? blend;
  final ValueChanged<int?> onChanged;
  const _ClipBlendRow({required this.blend, required this.onChanged});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final modes = blendModeNames();
    return Container(
      color: t.surface1,
      padding: const EdgeInsets.fromLTRB(10, 0, 10, 6),
      child: Row(
        children: [
          Text(l10n.tipBlendMode,
              style: t.small.copyWith(color: t.textSecondary)),
          const SizedBox(width: 8),
          Flexible(
            child: BareDropdown<int>(
              key: const ValueKey('fx-clip-blend'),
              dense: true,
              value: blend == null || blend! >= modes.length ? -1 : blend!,
              options: [for (var i = -1; i < modes.length; i++) i],
              label: (i) =>
                  i < 0 ? l10n.clipBlendFromLayer : engineLabel(modes[i]),
              onChanged: (i) => onChanged(i < 0 ? null : i),
            ),
          ),
        ],
      ),
    );
  }
}

/// The panel header: which layer is being edited, and Add effect.
class _Header extends StatelessWidget {
  final String layerName;

  /// The layer a clip subject is on. Set, the header says the rows are a
  /// clip's and not the layer's own.
  final String? clipOn;

  /// What the mark says for [clipOn]: the clip's, unless it says otherwise.
  final String? mark;

  /// Where the add button adds to. An overlap has no stack to add to.
  final ValueChanged<String>? onAdd;

  /// A right-click anywhere on the header, the add button included.
  final ValueChanged<Offset> onMenu;

  /// The row filter's well, in the name's place while it is up.
  final Widget? filter;

  /// Puts the filter up, from the search mark shown while it is down.
  final VoidCallback? onFilter;
  const _Header({
    required this.layerName,
    this.clipOn,
    this.mark,
    this.onAdd,
    required this.onMenu,
    this.filter,
    this.onFilter,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapUp: (d) => onMenu(d.globalPosition),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        color: t.surface1,
        child: _row(context, t),
      ),
    );
  }

  Widget _row(BuildContext context, LumitTheme t) {
    return Row(
      children: [
        // The same column the switches and stopwatches stand in below.
        SizedBox(
          width: fxStopwatchColumn,
          child: lumitIcon(LumitIcon.fx, size: iconSize, color: t.textMuted),
        ),
        const SizedBox(width: 6),
        if (filter case final well?)
          Expanded(child: well)
        else
        Expanded(
          child: Row(
            children: [
              Flexible(
                child: Text(layerName,
                    style: t.bodyPrimary, overflow: TextOverflow.ellipsis),
              ),
              if (clipOn case final layer?) ...[
                const SizedBox(width: 8),
                // The mark that these are a clip's own effects, with the layer
                // it is on in the tooltip.
                LumitTooltip(
                  message: layer,
                  child: Container(
                    key: const ValueKey('fx-clip-mark'),
                    padding:
                        const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
                    decoration: BoxDecoration(
                      border: Border.all(color: t.accent),
                      borderRadius: BorderRadius.circular(clipRadius(t)),
                    ),
                    child: Text(t.kickerCase(mark ?? l10n.effectControlsClip),
                        style: t.kicker.copyWith(color: t.accent)),
                  ),
                ),
              ],
            ],
          ),
        ),
        if (onFilter case final show? when filter == null)
          rowFilterMark(context, show, key: const ValueKey('fx-filter-mark')),
        // The keyframe strip's switch.
        const FxKeyStripToggle(),
        const SizedBox(width: 4),
        // Its own context, so the menu drops from the *button* rather than
        // from the header row's left edge — which is where it used to land.
        if (onAdd case final add?)
          Builder(
            builder: (buttonContext) => LumitTooltip(
              message: l10n.addEffect,
              child: HouseButton(
                key: const ValueKey('fx-add'),
                small: true,
                frameless: true,
                padding: const EdgeInsets.all(1),
                onPressed: () => showAddEffectMenu(buttonContext, add),
                child: glyph.LumitIcon(LumitIcons.addEffect,
                    size: iconSize, colour: t.textSecondary),
              ),
            ),
          ),
      ],
    );
  }
}

/// What a stack with nothing on it shows: the one thing it can do, on the
/// value column's edge, where Add animator stands.
class _EmptyStack extends StatelessWidget {
  final ValueChanged<String> onAdd;
  const _EmptyStack({required this.onAdd});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Padding(
      padding: const EdgeInsets.fromLTRB(10, 2, 10, 2),
      child: fxTwoColumnRow(
        context: context,
        name: const SizedBox.shrink(),
        control: Builder(
          builder: (button) => HouseButton(
            key: const ValueKey('fx-empty-add'),
            small: true,
            onPressed: () => showAddEffectMenu(button, onAdd),
            child: Text(t.propertyCase(l10n.addEffect)),
          ),
        ),
      ),
    );
  }
}

/// The Add-effect menu: one row per category, each opening onto its effects
/// (Add effect → Stylise → Glow).
///
/// [context] is the *button's*, so the menu drops from it rather than from the
/// panel's left edge. The whole list used to be one 380 px scroller, which is
/// a lot of reading to find one effect.
///
/// [category] narrows the catalogue to one family: the Audio timeline's clip
/// header passes `audio`, so its add button offers everything a clip can
/// actually be heard through and nothing else. The **family**, not the
/// provenance: Lumit's own audio effects and a hosted plugin file under the
/// one Audio key, and a menu that asked where an entry came from would offer
/// half of them.
Future<void> showAddEffectMenu(BuildContext context, ValueChanged<String> onAdd,
    {String? category}) async {
  final box = context.findRenderObject();
  if (box is! RenderBox) return;
  // Dropped from the button's left edge so a wide menu opens back across the
  // panel rather than off its right side.
  final origin = box.localToGlobal(Offset(0, box.size.height + 4));
  await showLumitPopup<void>(
    context: context,
    position: origin,
    builder: (close) => FloatSurface(
      width: 200,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: _addEffectRows(() => close(null), onAdd, category: category),
      ),
    ),
  );
}

/// One flyout row per effect category, each opening onto its effects.
List<Widget> _addEffectRows(VoidCallback close, ValueChanged<String> onAdd,
    {String? category}) {
  // Grouped in schema order, so the headings come out in the order the engine
  // declares. Within a heading the effects are sorted by name below.
  final grouped = <String, List<BridgeEffectInfo>>{};
  final headings = <String, String>{};
  for (final e in listEffects()) {
    if (category != null && e.category != category) continue;
    grouped.putIfAbsent(e.category, () => []).add(e);
    // A plugin group can arrive unheaded — audio plugins always do, an OFX
    // one that declared no grouping can — and a submenu with a blank name is
    // a door nobody can find, so the panel words it the way the browser does
    // (AP5). A named label always wins: a built-in audio effect brings the
    // engine's own Audio heading, and the plugins share it.
    if (e.categoryLabel.isNotEmpty) {
      headings[e.category] = engineLabel(e.categoryLabel);
    } else {
      headings.putIfAbsent(
          e.category,
          () => e.namespace == 'audio'
              ? l10n.effectsAudioPlugins
              : l10n.effectsPlugins);
    }
  }
  for (final effects in grouped.values) {
    effects.sort(byEffectLabel);
  }

  return [
    for (final entry in grouped.entries)
      SubmenuRow(
        key: ValueKey<String>('fx-category-${entry.key}'),
        closeParent: close,
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
  ];
}

/// One effect: its heading row and a row per declared parameter.
///
/// Drawn entirely from the read model — no bridge calls in build. The
/// heading-row ops need a live instance handle, which is fetched fresh at click
/// time (the model's data is not a handle, deliberately: frb consumes handles
/// passed by value).
class _EffectSection extends StatelessWidget {
  final BridgeEffectInstanceInfo info;

  /// The bound graph's name, for a Node graph effect: the heading wears it in
  /// place of the effect's label. Null for every other effect, and for one
  /// whose comp has gone. Resolved by the panel, so this card asks nothing.
  final String? graphName;
  final bool open;
  final VoidCallback onToggle;

  /// Twirl every effect on this stack open or shut.
  final ValueChanged<bool>? onFoldAll;

  /// Picked out of the stack, and the click that picks it. The same
  /// selection the Timeline's fold-out shows, so an effect chosen in one place
  /// is lit in the other — and Copy takes it from either.
  ///
  /// The heading follows the selection itself ([FxPick]), so a pick recolours
  /// it without this card being rebuilt. Null on the cards that take no part
  /// in the selection: a layer style, a group's header effect.
  final FxPick? pick;
  final VoidCallback onSelect;

  /// Which of this layer's parameters a driver is wired to, by
  /// `effectId/paramId`. Read once by the panel and passed down, so a
  /// card costs no question of its own.
  final Map<String, ({String driver, BridgePortType type, bool noStream})>
      driven;

  /// The layer's Points inputs, by `effectId/portId`, with each port's label
  /// and what it is wired from. Read once by the panel beside [driven].
  final Map<String, ({String label, BridgeOutputRef? from})> points;

  /// The drag in flight's staged value for (effect, param), or null — overlaid
  /// on the model's value so the number under the pointer is the staged one.
  final BridgeEffectValue? Function(UuidValue effect, String param) stagedValue;

  /// This card is a **layer style** rather than a stack entry
  /// (docs/impl/layer-styles.md §6).
  ///
  /// One bit, and everything the card draws is unchanged by it: a style is an
  /// `EffectInstance`, so the heading, the enable tick, Reset, the removal
  /// cross and every parameter row are the same widgets doing the same thing.
  /// What it changes is which of the layer's two lists a command reads
  /// ([_instances]) and which three commands are absent — reorder, copy and
  /// paste, none of which nine fixed slots in a pinned order can perform.
  final bool style;

  /// This card is a placed graph's **Inputs** (docs/impl/node-graph-comp.md
  /// §5.3). The layer carries exactly one and the graph reads it whatever its
  /// switch says, so the card is its rows, Reset and nothing else.
  final bool inputs;

  /// Whether its rows are built at all, which a closed card that has never
  /// been opened skips.
  final bool drawsRows;

  /// This card is one of a Model layer's materials, which also sets [style]:
  /// fixed in place and taking no part in the selection.
  final bool material;

  /// This card sits on a **group header's** stack
  /// (docs/impl/group-effects.md §6) — the styles bit's pattern, grown its
  /// third arm. Every command still goes through [layer] (the group's first
  /// member): the engine's shared instance lookup routes a group instance to
  /// `SetGroupEffects`, so the card's remove, bypass, reorder, reset and
  /// every parameter row are the same widgets doing the same thing.
  final UuidValue? group;

  /// This card sits on a **clip's** own stack: the group bit's pattern, grown
  /// its fourth arm. Every command goes through [layer], the clip's layer,
  /// and the engine routes the instance to the clip that holds it.
  final UuidValue? clip;
  final LayerReference layer;

  /// For a Clone deformer, why its Vector mode cannot be picked here, or null
  /// where it can ([cloneVectorReason]). Read by the panel, which holds the
  /// stack.
  final ({String tip, String why})? vectorReason;

  /// Rows this subject doesn't show, on top of what the instance itself hides.
  final Set<String> hideRows;

  /// Every layer in the comp, from the read model — what a layer-valued
  /// parameter picks from.
  final List<BridgeLayerEntry> allLayers;
  final CompositionReference comp;
  final int playheadFrame;
  final ValueChanged<int> onSeek;

  /// The stack itself changed (enabled, reordered, removed) — re-read it.
  final VoidCallback onStackChanged;

  /// The heading is an inline rename editor, its commit, and the
  /// Escape that throws the edit away instead.
  final bool renaming;
  final ValueChanged<String>? onRenamed;
  final VoidCallback? onRenameCancelled;

  /// Open that editor — the heading menu's Rename. `Enter` on the selected
  /// effect already did this from the keyboard (`effect.rename`); this is the
  /// mouse's way in, and it is the same way in a project item has (the same
  /// pairing, and the reason renaming came off the second click).
  final VoidCallback? onStartRename;

  /// Write a parameter — a typed value, or the release of a drag. One op.
  final void Function(UuidValue effect, String param, BridgeEffectValue value)
      onWrite;

  /// A drag tick: preview it, do not commit it.
  final void Function(UuidValue effect, String param, BridgeEffectValue value)
      onLive;

  /// Several parameters of one effect in one op — what a chained pair's
  /// proportional write commits through, so it costs one undo step.
  final void Function(UuidValue effect, Map<String, BridgeEffectValue> values)?
      onWritePair;

  /// Whether a parameter group's twirl is open, and toggling it. Held by the
  /// panel rather than here because this card is rebuilt from the read model on
  /// every change, and a fold that reset itself each time would be unusable.
  /// The schema's `collapsed` is the default until the owner touches it.
  final bool Function(String path, bool collapsedByDefault) isGroupOpen;
  final void Function(String path, bool collapsedByDefault) onToggleGroup;

  /// An Action row's press, and the panel's running count of them —
  /// what an effect's own display watches to know one happened.
  final void Function(UuidValue effect, String param) onAction;
  final int pressed;

  /// Open this Custom shader's inner graph. Null on every other effect, which
  /// draws no button for it.
  final VoidCallback? onOpenShaderGraph;

  /// Open this LFX effect's interface. Null on every effect without one.
  final VoidCallback? onOpenInterface;

  /// A camera following this layer's Camera track has been nudged —
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

  /// The panel's row filter, lowercase: only the rows whose names hold it are
  /// drawn, and a group with none is left out. Empty draws every row.
  final String filter;

  const _EffectSection({
    super.key,
    required this.info,
    this.graphName,
    this.filter = '',
    required this.open,
    this.drawsRows = true,
    required this.onToggle,
    this.onFoldAll,
    this.pick,
    required this.onSelect,
    this.driven = const {},
    this.points = const {},
    required this.stagedValue,
    this.style = false,
    this.inputs = false,
    this.material = false,
    this.group,
    this.clip,
    this.hideRows = const {},
    required this.layer,
    required this.allLayers,
    this.vectorReason,
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
    this.onOpenShaderGraph,
    this.onOpenInterface,
    this.trackCorrected = false,
    this.themedGraphs = false,
    this.curvePlotSize = curvePlotSizeDefault,
    this.onCurvePlotSize,
  });

  /// What can be done to this effect, shared with its heading in the Timeline.
  EffectCommands get _commands => EffectCommands(
        info: info,
        layer: layer,
        comp: comp,
        onStackChanged: onStackChanged,
        style: style,
        inputs: inputs,
        material: material,
        group: group,
        clip: clip,
      );

  List<BridgeEffectInstance> _instances() => _commands.instances();

  List<BridgeParamInfo> get _rows => _commands.rows;

  /// Whether an effect dragged from [from] lands **below** this heading, which
  /// it does when it is travelling down the stack. Read as the pointer arrives
  /// over this heading, as a card is not rebuilt when a neighbour goes.
  bool _landsBelow(String from) {
    final stack = _instances();
    final it = stack.indexWhere((e) => '${e.id()}' == from);
    final me = stack.indexWhere((e) => e.id() == info.id);
    return it >= 0 && me >= 0 && it < me;
  }

  /// The Mix and Matte rows nearly every effect ends with: what the Compositing
  /// fold holds. One the schema keeps in a group of its own (the Lens flare's
  /// source matte) stays where it is.
  List<BridgeParamInfo> get _compositingRows {
    final grouped = {
      for (final g in cachedListParameterGroups(info.name)) ...g.params,
    };
    final hidden = info.hiddenRows.toSet();
    return [
      for (final p in _rows)
        if (!hidden.contains(p.id) &&
            !grouped.contains(p.id) &&
            ((p.id == 'mix' && p.kind is BridgeParamKind_Float) ||
                (p.id == 'matte' && p.kind is BridgeParamKind_Layer)))
          p,
    ];
  }

  /// Where the Compositing fold's open state is kept.
  String get _compositingPath => 'fx-compositing-${info.id}';

  /// Put one of this material's rows back to its file's value.
  void _resetMaterialRow(String param) {
    for (final instance in _instances()) {
      if (instance.id() != info.id) continue;
      try {
        layer.resetMaterialRow(material: instance, param: param);
      } catch (_) {
        // The material went away under the press; nothing to put back.
      }
    }
    onStackChanged();
  }

  /// Chain or unchain a vector pair.
  ///
  /// Staged onto a fresh handle and committed with the stack, exactly as a
  /// rename is: one `SetLayerEffects`, one undo step, the shape every effect
  /// edit has. The **proportional drag itself never comes here** — that is
  /// arithmetic the row does while a gesture is live, and the document's
  /// business is only which pairs are tied together.
  void _togglePairLink(String stem) {
    final stack = _instances();
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

  @override
  Widget build(BuildContext context) {
    final id = info.id;
    final values = {for (final v in info.values) v.id: v.value};
    // What this effect and its rows are called in a shared project: the path
    // the Timeline gives them, so a row taken in hand in either panel is
    // marked in both. Null while the project is not shared.
    final share = ShareRows.maybeOf(context);
    final named = share != null && share.active ? share : null;
    final owner = layer.internallayerId.toString();
    final base = named == null
        ? ''
        : inputs
            ? inputsPath(owner)
            : material
                ? modelMaterialPath(owner, '$id')
                : style
                    ? stylePath(owner, '$id')
                    : group != null
                        ? effectPath(groupFoldPrefix(group!), '$id')
                        : clip != null
                            ? effectPath(clipFoldPrefix(clip!), '$id')
                            : effectPath(owner, '$id');

    // The effect's own display, read at `at` — the Levels histogram traces the
    // frame under the playhead, so this one *is* rebuilt on a scrub, alone.
    // Whether there is a display at all is a question about the effect's name,
    // never about the frame, so the `if` below may ask it at any frame.
    Widget? displayAt(int at) => customEffectDisplay(
          info.name,
          effectId: id,
          values: {
            for (final p in info.values) p.id: stagedValue(id, p.id) ?? p.value,
          },
          comp: comp,
          layer: layer,
          playheadFrame: at,
          onWrite: onWrite,
          onLive: onLive,
          onChanged: onStackChanged,
          pressed: pressed,
          trackCorrected: trackCorrected,
          themedGraphs: themedGraphs,
        );

    return FxSection(
      // The user's own name where one is set, then the graph a Node graph
      // effect applies, then the effect's label.
      title: (material && info.customName != null
              ? engineLabel(info.customName!)
              : info.customName) ??
          graphName ??
          effectLabelOf(info.name),
      open: open,
      onToggle: onToggle,
      pick: pick,
      theirs: named == null ? const [] : named.under(base),
      onSelect: onSelect,
      renaming: renaming,
      onRenamed: onRenamed,
      onRenameCancelled: onRenameCancelled,
      twirlKey: ValueKey<String>('fx-twirl-$id'),
      // Bypassed draws as a dashed outline round the heading (docs/15 §5), not
      // as a faded stack: the values stay readable while the effect is off.
      enabled: info.enabled,
      // Every key on the effect, for the strip's summary lane while shut.
      summaryKeys: [
        for (final v in info.values)
          if (v.value case BridgeEffectValue_Float(
              field0: BridgeScalar_Keyframed(:final field0)))
            ...field0,
      ],
      // **A target you can hit** (owner, desk test). The checkbox's own 14px
      // box is what a settings page wants; here it is the control reached
      // for most, and it was being missed. The whole stopwatch column, for
      // the whole height of the heading, switches the effect, and the mark
      // inside is drawn a step larger so it holds its own beside the
      // heading's capitals (see `fxEnableHitWidth`), and a drag off it sets
      // every switch it crosses to what this one just became (item 6.2).
      // No tooltip, as on the node box: the tick is the state, and the menu
      // names it Bypass.
      leading: inputs
          ? null
          : fxEnableSwitch(
              id: '$id',
              on: info.enabled,
              onChanged: (on) => _commands.setEnabled(context, on),
            ),
      actions: [
        if (onOpenInterface case final open?)
          fxTextAction(
            context,
            label: l10n.lfxInterface,
            tip: l10n.tipLfxInterface,
            keyName: 'fx-interface-$id',
            onPressed: () {
              // The panel follows the picked instance, so this one first.
              onSelect();
              open();
            },
          ),
      ],
      // What this effect cost in the last measured frame, the same number
      // its row in the Timeline shows, from the same measurement (docs/13
      // §7.1). Blank unless the Timeline's render-time column is measuring,
      // so this panel neither turns the cost on nor shows a stale figure.
      status: inputs ? null : TimingsCell(effectId: '$id'),
      hoverActions: [
        if (material || !_commands.atDefault)
          _headingButton(
            context,
            mark: LumitIcons.reset,
            tip: l10n.tipResetParameters,
            key: 'fx-reset-$id',
            onPressed: () => _commands.reset(context),
          ),
        if (!inputs && !material)
          _headingButton(
            context,
            mark: LumitIcons.close,
            tip: l10n.tipRemove,
            key: 'fx-remove-$id',
            onPressed: () => _commands.remove(context),
          ),
      ],
      // Drag the heading to move the effect (docs/07 §6): the gesture the rest
      // of the application already uses to reorder a list, and the one the
      // owner asked for.
      // A style is not draggable: its place in the list is Photoshop's, not
      // the user's (docs/impl/layer-styles.md §2).
      dragKey: style || inputs ? null : '$id',
      landsBelow: _landsBelow,
      onDropped: (from) {
        final stack = _instances();
        final it = stack.indexWhere((e) => '${e.id()}' == from);
        final to = stack.indexWhere((e) => e.id() == id);
        if (it < 0 || to < 0) return;
        try {
          layer.reorderEffect(effect: stack[it], newIndex: to);
        } catch (_) {
          // The stack changed under the drag; re-reading is the recovery.
        }
        onStackChanged();
      },
      // Right-click is where everything else an effect can do lives.
      onContextMenu: (at) => _commands.showMenu(
        context,
        at,
        onStartRename: onStartRename,
        onFoldAll: onFoldAll,
        compositing: inputs || _compositingRows.isEmpty
            ? null
            : (
                open: isGroupOpen(_compositingPath, true),
                toggle: () => onToggleGroup(_compositingPath, true),
              ),
      ),
      // An effect with its own display (Levels' histogram) draws it
      // above its rows; the rows themselves are unchanged.
      rows: [
        // Above everything, because it is the reason the rows beneath it are
        // not doing anything (docs/12 §1, §2.3). An effect that is behaving
        // draws none of this.
        if (effectBadgeRow(context,
                id: '$id',
                // A Clone deformer set to Vector that cannot read outlines here
                // says so, and why, as the effects that cannot run do.
                reason: info.badgeReason ??
                    (vectorReason != null &&
                            info.values.any((v) =>
                                v.id == 'mode' &&
                                v.value == const BridgeEffectValue.choice(1))
                        ? 'clone_vector_reads_picture'
                        : null),
                detail: info.badgeDetail ?? vectorReason?.why)
            case final badge?)
          badge,
        if (displayAt(playheadFrame) != null)
          _AtPlayhead(builder: (context, at) => displayAt(at)!),
        // The way into a Custom shader's inner graph, drawn as its other
        // buttons are.
        if (onOpenShaderGraph case final open?)
          fxTwoColumnRow(
            context: context,
            name: const SizedBox.shrink(),
            keyframeControls: null,
            control: HouseButton(
              key: ValueKey<String>('fx-open-graph-$id'),
              small: true,
              onPressed: open,
              child: Text(engineLabel('Open graph'),
                  style: ThemeScope.of(context).theme.body),
            ),
          ),
        // A Points input takes a wire rather than a value, so it has no
        // parameter row of its own.
        for (final input in points.entries)
          if (input.key.startsWith('$id/'))
            _pointsRow(context, input.key.substring('$id/'.length),
                input.value.label, input.value.from),
        if (drawsRows) ..._paramRows(named, base, id, values),
      ],
    );
  }

  /// A Points input's row: what it is wired to, picked from the effects above
  /// this one that hand points out.
  ///
  /// The list is read when the menu opens, as the layer picker's is. Which
  /// effects sit above is a position, and a card can outlive one.
  Widget _pointsRow(BuildContext context, String port, String label,
      BridgeOutputRef? from) {
    final t = ThemeScope.of(context).theme;
    final node = BridgeNodeRef.effect(info.id);
    final to = BridgeInputRef.param(node: node, port: port);
    return fxTwoColumnRow(
      context: context,
      name: Text(t.propertyCase(engineLabel(label)),
          style: t.body, overflow: TextOverflow.ellipsis),
      control: SizedBox(
        width: effectCellWidth + 40,
        child: BareLazyDropdown<BridgeOutputRef?>(
          key: ValueKey<String>('fx-points-${info.id}-$port'),
          // Ticked by the wire itself: two effects can share a name.
          value: from,
          // A wire from a driver in the graph is named here too, though the
          // list only offers the stack.
          label: driven['${info.id}/$port']?.driver ?? l10n.none,
          options: () => [
            (null, l10n.none),
            for (final n
                in layer.getGraph().nodes.takeWhile((n) => n.node != node))
              if (n.node case BridgeNodeRef_Effect(:final field0))
                for (final o in n.outputs)
                  if (o.portType == BridgePortType.points)
                    (
                      BridgeOutputRef.effectData(effect: field0, port: o.id),
                      n.customName ?? engineLabel(n.label)
                    ),
          ],
          onChanged: (from) {
            // The wiring is read, changed and written back whole, so the pick
            // is one undo step.
            try {
              final w = layer.getGraph().wiring;
              layer.setGraph(
                drivers: layer.getGraphDrivers(),
                wiring: BridgeGraphWiring(
                  edges: [
                    for (final e in w.edges)
                      if (e.to != to) e,
                    if (from != null) BridgeGraphEdge(from: from, to: to),
                  ],
                  layout: w.layout,
                  exposed: w.exposed,
                  groups: w.groups,
                  outUnwired: w.outUnwired,
                ),
              );
            } catch (_) {
              // The graph changed under us, or the edit was refused. Reading
              // it again is the recovery.
            }
            onStackChanged();
          },
        ),
      ),
    );
  }

  /// Put [rows] back to what the schema declares, as one write, or null when
  /// they are there already.
  VoidCallback? _resetRows(UuidValue id, List<BridgeParamInfo> rows) {
    final held = {for (final v in info.values) v.id: v.value};
    final values = <String, BridgeEffectValue>{
      for (final p in rows)
        if (defaultEffectValue(p.kind) case final fallback?)
          if (!isDefaultEffectValue(held[p.id], fallback,
              owner: layer.internallayerId))
            p.id: fallback,
    };
    final write = onWritePair;
    if (values.isEmpty || write == null) return null;
    return () => write(id, values);
  }

  /// Where this card's rows sit in the Timeline's selection.
  String? _rowPathBase(UuidValue id) {
    final owner = layer.internallayerId.toString();
    return inputs
        ? inputsPath(owner)
        : material
            ? modelMaterialPath(owner, '$id')
            : style
                ? stylePath(owner, '$id')
                : group != null
                    ? effectPath(groupFoldPrefix(group!), '$id')
                    : clip != null
                        ? effectPath(clipFoldPrefix(clip!), '$id')
                        : null;
  }

  /// The parameter rows, folded through the schema's groups (docs/08 §1.2)
  /// and the `_x`/`_y` point-pair convention (docs/07 §6.1):
  ///
  /// - a labelled group renders as a sub-twirl at its first member's
  ///   position, its members inside;
  /// - an empty-label group renders its members in place, headerless — the
  ///   conditional-run shape;
  /// - a group with `visible_when` is skipped entirely (members included)
  ///   while the named sibling Choice holds a different value;
  /// - two adjacent Float params `foo_x`, `foo_y` fold into one point row
  ///   (with the position dropper for the declared %-of-frame pairs).
  List<Widget> _paramRows(ShareRows? named, String base, UuidValue id,
      Map<String, BridgeEffectValue> values) {
    // In a shared project each row goes by its Timeline path. Outside one
    // the rows come back exactly as they are made.
    Widget marked(String param, String? other, Widget row) => named == null
        ? row
        : named.row('$base/$param', row,
            also: other == null ? null : '$base/$other');

    // A plugin hides and shows its own rows, and the instance says which
    // are hidden right now. A group with nothing left to show is not drawn.
    final hidden = {...info.hiddenRows, ...hideRows};
    final params = [
      for (final p in _rows)
        if (!hidden.contains(p.id) &&
            (filter.isEmpty ||
                engineLabel(p.label).toLowerCase().contains(filter)))
          p,
    ];
    final groups = cachedListParameterGroups(info.name);
    final byFirstMember = <String, BridgeParamGroup>{};
    final memberOf = <String, BridgeParamGroup>{};
    for (final g in groups) {
      final members = [
        for (final m in g.params)
          if (!hidden.contains(m)) m,
      ];
      if (members.isNotEmpty) byFirstMember[members.first] = g;
      for (final m in members) {
        memberOf[m] = g;
      }
    }
    bool groupVisible(BridgeParamGroup g) => paramGroupVisible(g, values);

    // Which rows another parameter has taken over (`EnabledWhen`).
    // Judged on what the panel is SHOWING, staged drag included, so ticking a
    // checkbox greys its dependent row on the spot rather than after the commit
    // round-trips.
    final shown = {
      for (final p in params)
        if ((stagedValue(id, p.id) ?? values[p.id]) case final v?) p.id: v,
    };
    // A plugin greys its own rows as well, and the instance says which.
    final disabled = {
      ...disabledParams(info.name, shown),
      ...info.disabledRows,
    };
    final whyDisabled = disabledReasons(info.name, shown);

    // The uniform Matte row and the Mix row: a Layer picker carries its Channel
    // and Invert beside it, a Mix slider its Blend, and no rider gets a row of
    // its own. [paramRidersFor] holds the rule, so the box on the node graph
    // canvas folds by the same one.
    List<BridgeParamInfo> ridersFor(BridgeParamInfo p) =>
        paramRidersFor(params, p);

    final compositing = _compositingRows;

    final folded = <String>{
      for (final p in params)
        for (final r in ridersFor(p)) r.id,
      for (final p in compositing) p.id,
    };

    // What a control of the composition's graphics template can name here:
    // a row of the layer's own stack or styles, and on a copy's card the
    // control of the template placed there. Nothing on any other card.
    final owner = layer.internallayerId;
    final onStack = !material && group == null && clip == null;
    final copy = !inputs
        ? null
        : allLayers
            .where((e) => e.layer.internallayerId == owner)
            .firstOrNull
            ?.info
            .copy;

    Widget rowFor(BridgeParamInfo param) {
      return marked(
          param.id,
          null,
          EffectParamRowFrb(
        key: ValueKey<String>('fx-row-$id-${param.id}'),
        controlRow: effectControlRow(owner, id, param,
            inputs: inputs, copy: copy, elsewhere: !onStack),
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
        onWritePair: onWritePair,
        twoColumn: true,
        enabled: !disabled.contains(param.id),
        disabledReason: whyDisabled[param.id],
        optionNote: (i) => optionNoteOf(info.name, param.id, i),
        optionReason: info.name == 'clone_deformer' && param.id == 'mode'
            ? (i) => i == 1 ? vectorReason?.tip : null
            : null,
        offBecause: waitsForSimulate(info.name, param.id, shown)
            ? l10n.fxNeedsSimulate
            : null,
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
        onReset: material ? () => _resetMaterialRow(param.id) : null,
        rowPath: base.isEmpty ? _rowPathBase(id) == null ? null : '${_rowPathBase(id)}/${param.id}' : '$base/${param.id}',
      ));
    }

    // **The curve fold** (docs/08 §3.30). A run of neighbouring Curve
    // parameters is one editor with a tab each, not one plot per row: five
    // stacked squares would be five times the height and would still make the
    // user compare shapes across them. The same folding the `_x`/`_y` point
    // pair takes, over as many parameters as declare a curve in a row.
    // Greyed like a row when a switch has taken every curve in it out of
    // play, as Vary points' colour ramp is until it is switched on.
    Widget curveEditor(List<BridgeParamInfo> run) => dimmedIf(
        run.every((p) => disabled.contains(p.id)),
        CurveChannelEditor(
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
          resetTip: l10n.tipResetCurve,
          plotSize: curvePlotSize,
          onPlotSize: onCurvePlotSize,
          onLive: (c, points) => onLive(id, run[c].id, curveValue(points)),
          onCommit: (c, points) => onWrite(id, run[c].id, curveValue(points)),
        ));

    // Fold a run of params into rows, pairing x/y neighbours and gathering
    // curve runs. Both folds live here rather than only in the outer walk,
    // because a schema is free to put them inside a **group** — Particulate's
    // two over-life curves sit under the Particle kicker, and before this they
    // came out as two stacked plots, which is the shape the curve fold
    // exists to prevent.
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
            next.kind is BridgeParamKind_Float &&
            !drivenByExpression(stagedValue(id, param.id) ?? values[param.id]) &&
            !drivenByExpression(stagedValue(id, next.id) ?? values[next.id]);
        if (isPair) {
          final stem = pairStemOf(info.name, param.id);
          out.add(marked(
              param.id,
              next.id,
              EffectPointRowFrb(
            key: ValueKey<String>('fx-row-$id-${param.id}-pair'),
            // Both rows as one. A copy's point is one control already.
            controlRow: !onStack
                ? null
                : inputs
                    ? copyControlRow(owner, copy, param.id)
                    : pointControlRow(owner, id, param, next,
                        engineLabel(pointStem(param.label))),
            ownerLayerId: layer.internallayerId,
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
            // The chain. The stem is the schema's key for the pair,
            // and which pairs are tied is on the instance, so both come out of
            // data the card already holds — no call rides this rebuild.
            linked: stem != null && info.linkedPairs.contains(stem),
            onToggleLink: stem == null ? null : () => _togglePairLink(stem),
            canSetExpression: true,
          )));
          i += 2;
        } else {
          out.add(rowFor(param));
          i += 1;
        }
      }
      return out;
    }

    final rows = <Widget>[];
    // The folds that start shut go after the loose rows, shut or open, so what
    // follows one never reads as what's inside it and its twirl stays put.
    final tucked = <Widget>[];
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
          final path = 'fx-group-$id-${group.label}';
          (group.collapsed ? tucked : rows).add(_ParamGroupSection(
            key: ValueKey<String>(path),
            label: group.label,
            open: filter.isNotEmpty || isGroupOpen(path, group.collapsed),
            onToggle: () => onToggleGroup(path, group.collapsed),
            onReset: _resetRows(id, run),
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
    rows.addAll(tucked);
    if (compositing.isNotEmpty) {
      final path = _compositingPath;
      final open = isGroupOpen(path, true);
      // Shut, what is off its default still shows, so a mix or a matte is never
      // hidden. At its defaults, shut, the fold takes no room at all: the
      // heading's menu opens it.
      final summary =
          open ? null : _compositingSummary(compositing, ridersFor, shown);
      if (open || summary != null) {
        rows.add(_ParamGroupSection(
          headerKey: ValueKey<String>(path),
          label: l10n.fxFoldCompositing,
          open: open,
          onToggle: () => onToggleGroup(path, true),
          onReset: _resetRows(id, [
            for (final p in compositing) ...[p, ...ridersFor(p)],
          ]),
          summary: summary,
          rows: [for (final p in compositing) rowFor(p)],
        ));
      }
    }
    return rows;
  }

  /// What a shut Compositing fold says about its rows: the mix, the blend and
  /// the matte, each only when it is off its default. `50% · Screen · matte:
  /// Title`, or null when there is nothing to say.
  String? _compositingSummary(
    List<BridgeParamInfo> hosts,
    List<BridgeParamInfo> Function(BridgeParamInfo) ridersFor,
    Map<String, BridgeEffectValue> shown,
  ) {
    final parts = <String>[];
    for (final host in hosts) {
      final value = shown[host.id];
      if (value != defaultEffectValue(host.kind)) {
        switch (value) {
          case BridgeEffectValue_Float(
              field0: BridgeScalar_Static(:final field0)
            ):
            final whole = field0 == field0.roundToDouble();
            parts.add('${field0.toStringAsFixed(whole ? 0 : 1)}'
                '${l10n.unitSymbolPercent}');
          // Keyed, or an expression: there is no one number to give.
          case BridgeEffectValue_Float():
            parts.add(l10n.animated);
          case BridgeEffectValue_Layer(:final field0?):
            parts.add(l10n.fxCompositingMatte(allLayers
                    .where((l) => l.layer.internallayerId == field0)
                    .map((l) => l.info.name)
                    .firstOrNull ??
                l10n.missingLayer));
          default:
        }
      }
      // The blend rides on the Mix row, so it reads straight after the mix.
      if (host.id != 'mix') continue;
      for (final rider in ridersFor(host)) {
        if ((rider.kind, shown[rider.id])
            case (
              BridgeParamKind_Choice(:final options, :final default_),
              BridgeEffectValue_Choice(:final field0)
            ) when field0 != default_ && field0 < options.length) {
          parts.add(engineLabel(options[field0]));
        }
      }
    }
    return parts.isEmpty ? null : parts.join(' · ');
  }

  /// One of the marks that come up on the heading under the pointer.
  Widget _headingButton(
    BuildContext context, {
    required String mark,
    required String tip,
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
        padding: const EdgeInsets.all(1),
        onPressed: onPressed,
        child: glyph.LumitIcon(mark, size: 14, colour: t.textMuted),
      ),
    );
  }
}

/// One collapsible parameter group inside an effect's card (docs/08 §1.2):
/// a small twirl header, its member rows indented under it. Whether it is
/// open is the panel's, so it survives the card being rebuilt.
class _ParamGroupSection extends StatelessWidget {
  final String label;
  final bool open;
  final VoidCallback onToggle;
  final List<Widget> rows;

  /// What the heading says about its rows while it is shut, and the key its
  /// twirl row is found by. Both are the Compositing fold's.
  final String? summary;
  final Key? headerKey;

  /// Puts every row inside back to its default, or null when they all are.
  final VoidCallback? onReset;
  const _ParamGroupSection({
    super.key,
    required this.label,
    required this.open,
    required this.onToggle,
    required this.rows,
    this.summary,
    this.headerKey,
    this.onReset,
  });

  @override
  Widget build(BuildContext context) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // The shared header row, so a group's twirl sits in the stopwatch
          // column and its label starts where every other label does. The
          // members are NOT indented: the straight label edge runs the
          // whole panel, and a fold says what it is with its kicker.
          fxGroupHeaderRow(
            context,
            label: label,
            open: open,
            onToggle: onToggle,
            summary: summary,
            key: headerKey,
            onContextMenu: (at) => showMenuAt<void>(
              context: context,
              position: at,
              rows: (close) => [
                MenuRow(
                  onPressed: () {
                    close(null);
                    onToggle();
                  },
                  child: MenuLabel(open ? l10n.fxMenuCollapse : l10n.fxMenuExpand),
                ),
                const MenuDivider(),
                MenuRow(
                  enabled: onReset != null,
                  onPressed: () {
                    close(null);
                    onReset?.call();
                  },
                  child: MenuLabel(l10n.reset, enabled: onReset != null),
                ),
              ],
            ),
          ),
          if (open)
            Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: rows,
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

  /// How each two-axis property is shown.
  final BridgeAxisModes axisModes;

  /// What the layer is, and - on a camera - how it aims: which rows the
  /// section shows, and whether its heading carries a solve-link badge. A
  /// Camera is the one kind whose transform can be **derived** rather than
  /// held, which is what the badge says.
  final BridgeLayerKind kind;
  final bool twoNode;
  final BridgeLightSettings? light;
  final bool acceptsLights;
  final BridgeMaterialSettings? material;

  bool get isCamera => kind == BridgeLayerKind.camera;

  /// This camera's solve link carries a correction — the dot beside the
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
    required this.kind,
    required this.twoNode,
    required this.light,
    required this.acceptsLights,
    required this.material,
    required this.corrected,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
    required this.open,
    required this.onToggle,
    this.filter = '',
  });

  /// The panel's row filter, handed to the rows.
  final String filter;

  @override
  Widget build(BuildContext context) {
    final share = ShareRows.maybeOf(context);
    final named = share != null && share.active ? share : null;
    final owner = layer.internallayerId.toString();
    // A filter the heading's name holds keeps every row, as an effect's does.
    final title = engineLabel('Transform');
    final needle = title.toLowerCase().contains(filter) ? '' : filter;
    final rows = TransformRowsFrb(
      comp: comp,
      layer: layer,
      transform: transform,
      threeD: threeD,
      kind: kind,
      twoNode: twoNode,
      light: light,
      acceptsLights: acceptsLights,
      material: material,
      axisModes: axisModes,
      playheadFrame: playheadFrame,
      onSeek: onSeek,
      onChanged: onChanged,
      twoColumn: true,
      filter: needle,
    ).rows(context);
    if (rows.isEmpty) return const SizedBox.shrink();
    return FxSection(
        title: title,
        open: open || needle.isNotEmpty,
        onToggle: onToggle,
        onContextMenu: (at) =>
            showTransformMenu(context, at, layer, onChanged: onChanged),
        // A solve-linked camera says so where its numbers are, because that is
        // where the surprise would otherwise be: the rows below are still
        // editable, but what they hold is a correction on top of the solve
        // rather than a pose. Convert to keyframes
        // sits beside the badge — it is the one command that ends the link,
        // and it belongs next to the thing it ends.
        actions: [
          // Expanded, as the render-time cell beside an effect's Reset is, and
          // for the same reason: a heading's action row lays its children out
          // unbounded, so a badge that carries a sentence has to be told how
          // much room it has before it can clip it.
          if (isCamera)
            Expanded(
              // Its own listener: the badge reads the solve under the playhead,
              // and the card around it no longer redraws on a scrub.
              child: _AtPlayhead(
                builder: (context, at) => CameraLinkBadge(
                  key: ValueKey<String>('tf-link-${layer.internallayerId}'),
                  camera: layer,
                  playheadFrame: at,
                  corrected: corrected,
                  onChanged: onChanged,
                ),
              ),
            ),
        ],
        theirs: named == null ? const [] : named.under(transformPath(owner)),
        rows: [
          for (final row in rows)
            // Each row by the path its twin in the Timeline has.
            named != null && row is TransformRowFrb
                ? named.row(transformGroupPath(owner, row.group), row)
                : row,
        ],
      );
  }
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

bool _simulating(Map<String, BridgeEffectValue> values) {
  final on = values['simulate'];
  return on is BridgeEffectValue_Bool && on.field0;
}
bool _namesALayer(BridgeEffectValue? value) =>
    value is BridgeEffectValue_Layer && value.field0 != null;

/// The display an effect draws *above* its rows, or null for the effects that
/// draw none — which is nearly all of them.
///
/// **Levels is the one that claims it**: a histogram of the frame with
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
  bool themedGraphs = false,
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
          // A Red button draws red, on the same setting the curve tabs read.
          channelColours: themedGraphs
              ? null
              : [for (final c in levelsChannels) curveChannelColour(c)],
        ),
      // Camera track's display is a *status*, not a control: how far
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
      // thing: a status, but about a *surface* rather than a camera,
      // and filed under this instance rather than under the media — which is
      // why it is the one custom display that needs its effect's own id.
      // Track points reports through the same status, so it shares the line.
      'planar_track' || 'track_points' => PlanarTrackDisplayFrb(
          key: ValueKey<String>('fx-planar-track-display-$effectId'),
          layer: layer,
          effectId: effectId,
          onChanged: onChanged,
          pressed: pressed,
        ),
      // Bake points' bake is filed and reported the same way, in its own words,
      // and so is a simulation's.
      'bake_points' => PlanarTrackDisplayFrb(
          key: ValueKey<String>('fx-planar-track-display-$effectId'),
          layer: layer,
          effectId: effectId,
          onChanged: onChanged,
          pressed: pressed,
          bake: true,
        ),
      'particulate' || 'smoke' when _simulating(values) => PlanarTrackDisplayFrb(
          key: ValueKey<String>('fx-planar-track-display-$effectId'),
          layer: layer,
          effectId: effectId,
          onChanged: onChanged,
          pressed: pressed,
          bake: true,
        ),
      // The Roto brush's is a status too, with one control in it: the
      // base frame the propagation runs outward from, which is the one thing
      // about a matte that is neither a parameter nor a stroke — it is *which
      // frame* the strokes are read from, and moving it retires the run.
      'roto_brush' => RotoDisplayFrb(
          key: ValueKey<String>('fx-roto-display-$effectId'),
          layer: layer,
          effectId: effectId,
          playheadFrame: playheadFrame,
          onChanged: onChanged,
          pressed: pressed,
        ),
      // Depth's is a status about a shot read by a model: how far the reading
      // covers, and what read it. Filed under this instance, like the Roto
      // brush's and for the same reason: two of them on one clip on different
      // models are two answers, not one.
      'depth' => PlaneDisplayFrb(
          key: ValueKey<String>('fx-depth-display-$effectId'),
          card: PlaneCard.depth,
          layer: layer,
          effectId: effectId,
          onChanged: onChanged,
          pressed: pressed,
        ),
      // Depth points asks for the same reading, unless a Depth layer has taken
      // over from it.
      'depth_points' when !_namesALayer(values['depth_layer']) => PlaneDisplayFrb(
          key: ValueKey<String>('fx-depth-points-display-$effectId'),
          card: PlaneCard.depth,
          layer: layer,
          effectId: effectId,
          onChanged: onChanged,
          pressed: pressed,
        ),
      // Remove background's is the same card, because it is the same job: a
      // model reading a shot a frame at a time and filing what it found. The
      // answer at the end of it is a matte rather than a depth, which is the
      // one word between them.
      'remove_background' => PlaneDisplayFrb(
          key: ValueKey<String>('fx-remove-background-display-$effectId'),
          card: PlaneCard.matte,
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
/// sentence, so the words are the user's own: the plugin failed, the
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
  final sentence = effectBadge(reason, detail);
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
        // A missing LFX effect's sentence names the extension already.
        if (detail != null && detail.isNotEmpty && reason != 'lfx_missing')
          Text(
            detail,
            key: ValueKey<String>('fx-badge-detail-$id'),
            style: t.small.copyWith(color: t.textMuted),
          ),
      ],
    ),
  );
}
