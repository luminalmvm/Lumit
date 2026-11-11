// The Grade panel: the selected layer's colour grade as wheels, primaries,
// curves and a look, over the ordinary effects that do the work.
//
// In plain terms: a grade is a handful of Colour effects stacked in a sensible
// order. This panel is a view over those effects on the layer that is
// selected. Nothing is added until a control is touched; the first touch adds
// the effect the control needs, in its place in the stack, and that and the
// value are one undo step. Every row in Effect controls and the Timeline
// still works on the same effects, which is where their keyframes are set;
// a keyed value here takes a key at the playhead, as a keyed row does.

import 'dart:async';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/colour.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/presets.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/settings_account.dart' show askBefore;
import '../shell/theme_name_dialog.dart';
import '../state/comp_time.dart';
import '../state/dock.dart';
import '../state/dropper.dart';
import '../state/template_controls.dart' show ControlRow;
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/curve_editor.dart';
import '../widgets/edge_fade.dart';
import 'control_rows.dart' show paramControlRow;
import 'effect_controls_panel_frb.dart' show curveChannelColour;
import 'effect_param_row_frb.dart'
    show
        DropperButton,
        EffectStackEditor,
        curvePointsOf,
        curveValue,
        defaultEffectValue,
        newExpressionLanguage,
        scalarValueAt;
import 'grade_clipboard.dart';
import 'grade_controls.dart';
import 'grade_looks.dart';
import 'grade_wheel.dart';
import 'keyframe_controls_frb.dart'
    show KeyframeControlsFrb, scalarWithValueAt;
import 'layer_fold_frb.dart' show asOneUndoStep, effectPath;
import 'param_row_menu.dart';

/// The effects a grade is made of, in the order they are stacked: light
/// first, then balance, then the shaping, with a look last.
enum GradeFx {
  exposure('exposure'),
  temperature('temperature'),
  colourBalance('colour_balance'),
  contrast('contrast'),
  saturation('saturation'),
  curves('curves'),
  lut('lut');

  const GradeFx(this.matchName);
  final String matchName;
}

/// The panel's four sections, each over some of the effects.
enum _Section {
  balance([GradeFx.colourBalance]),
  basics([
    GradeFx.exposure,
    GradeFx.temperature,
    GradeFx.contrast,
    GradeFx.saturation
  ]),
  curves([GradeFx.curves]),
  look([GradeFx.lut]);

  const _Section(this.effects);
  final List<GradeFx> effects;
}

/// How far a wheel's rim reaches, and how far its bar travels, per colour.
/// Lift adds, so a little goes a long way; Gamma and Gain multiply, and
/// Gamma's rim stops where its floor of 0.1 is, so the dot never jumps back.
const Map<String, (double reach, double span)> _wheelRanges = {
  'lift': (0.5, 0.5),
  'gamma': (0.9, 0.9),
  'gain': (1.0, 1.0),
};

class GradePanelFrb extends StatefulWidget {
  const GradePanelFrb({super.key});

  @override
  State<GradePanelFrb> createState() => _GradePanelFrbState();
}

class _GradePanelFrbState extends State<GradePanelFrb> {
  final EffectStackEditor _editor = EffectStackEditor();

  /// Each effect's declaration, read once: the defaults and ranges every
  /// control takes come from the engine, never from a table here.
  late final Map<GradeFx, Map<String, BridgeParamInfo>> _schema = {
    for (final fx in GradeFx.values)
      fx: {
        for (final p in listParameters(effect: fx.matchName)) p.id: p,
      },
  };

  int _curveChannel = 0;

  /// The height the curve's plot can take with everything else still on
  /// screen, measured after each layout from the pane's own contents.
  double _plotRoom = _plotMax;
  final GlobalKey _content = GlobalKey();
  double _plot = _plotMax;
  static const double _plotMax = 260;
  // Low enough that the whole grade stands in the Colour workspace at 1080 in
  // every look, Lantern's taller rows included.
  static const double _plotMin = 96;
  static const double _listFoot = 8;

  /// From this width the curve takes a column of its own, up to
  /// [_plotWideMax] across.
  static const double _wideAt = 680;
  static const double _plotWideMax = 480;

  /// A section heading's height: its 18 and its padding above and below.
  static const double _headingHeight = 28;

  /// The look each layer is wearing and the grade it brought, by layer. A look
  /// isn't kept in the document, so this lasts until the grade is touched or
  /// its effects change.
  final Map<UuidValue, (String, List<UuidValue>)> _worn = {};

  /// Whether a white balance pick has opened its undo group.
  bool _neutralTouched = false;

  /// Undo groups opened by a gesture and not yet closed.
  int _touches = 0;

  /// Whether the open gesture added an effect, which a cancel takes back.
  bool _touchAdded = false;

  /// The open gesture's newest preview, asked for again after a document
  /// change: the Viewer asks for the document then, and the newer ask would
  /// stand in front of the preview, the scope with it, until the next move.
  VoidCallback? _restage;
  StreamSubscription<void>? _changes;

  @override
  void initState() {
    super.initState();
    _changes = app.onChange.listen((_) {
      if (_restage != null) scheduleMicrotask(() => _restage?.call());
    });
  }

  /// Whether the before picture is being held up.
  bool _before = false;

  /// The project a touch opened its group on, so the group can be closed
  /// from `dispose`, where nothing may be looked up.
  ProjectReference? _touched;

  @override
  void dispose() {
    _changes?.cancel();
    _editor.clear();
    while (_touches > 0) {
      _touches--;
      _touched?.endUndoGroup();
    }
    super.dispose();
  }

  // --- The document as it stands -------------------------------------------

  LumitUiState get ui => Provider.of<LumitUiState>(context, listen: false);
  LumitState get app => Provider.of<LumitState>(context, listen: false);

  LayerReference? get _layer => ui.selectedLayer.value;
  BridgeLayerInfo? get _info => _infoOf(_layer);
  BridgeLayerInfo? _infoOf(LayerReference? layer) =>
      layer == null ? null : ui.model.byId(layer.internallayerId)?.info;

  /// The layers an edit lands on: the one shown first, then every other
  /// picked layer that can wear a grade.
  List<LayerReference> get _targets {
    final shown = _layer;
    if (shown == null) return const [];
    return [
      shown,
      for (final layer in ui.selectedLayers.value)
        if (layer.internallayerId != shown.internallayerId)
          if (_infoOf(layer) case final info? when gradeable(info.kind)) layer,
    ];
  }

  /// The first instance of [fx] on the layer, the shown one unless [on]
  /// says otherwise, or null when none is there.
  BridgeEffectInstanceInfo? _instance(GradeFx fx, [BridgeLayerInfo? on]) {
    for (final e
        in (on ?? _info)?.effects ?? const <BridgeEffectInstanceInfo>[]) {
      if (e.name == fx.matchName) return e;
    }
    return null;
  }

  BridgeEffectValue? _value(GradeFx fx, String id, [BridgeLayerInfo? on]) {
    for (final v in _instance(fx, on)?.values ?? const <BridgeParamValue>[]) {
      if (v.id == id) return v.value;
    }
    return null;
  }

  BridgeParamKind _kind(GradeFx fx, String id) => _schema[fx]![id]!.kind;

  double _defaultFloat(GradeFx fx, String id) => switch (_kind(fx, id)) {
        BridgeParamKind_Float(:final default_) => default_,
        _ => 0,
      };

  (double, double) _range(GradeFx fx, String id) => switch (_kind(fx, id)) {
        BridgeParamKind_Float(:final sliderMin, :final sliderMax) => (
            sliderMin,
            sliderMax
          ),
        _ => (0, 1),
      };

  /// The bounds typing keeps to: the hard ones, or open where the effect
  /// declares none, as an Effect controls field reads them.
  (double, double) _hardRange(GradeFx fx, String id) => switch (_kind(fx, id)) {
        BridgeParamKind_Float(:final hardMin, :final hardMax) => (
            hardMin ?? -1000000,
            hardMax ?? 1000000
          ),
        _ => (0, 1),
      };

  /// A number as it reads at the playhead. An expression reads as its
  /// default and its control stands down: there is no number to drag.
  double _float(GradeFx fx, String id) {
    final comp = ui.selectedComp;
    final v = _value(fx, id);
    if (comp == null || v is! BridgeEffectValue_Float) {
      return _defaultFloat(fx, id);
    }
    return scalarValueAt(v.field0, comp, ui.playheadFrame.value) ??
        _defaultFloat(fx, id);
  }

  bool _floatKeyed(GradeFx fx, String id) {
    final v = _value(fx, id);
    return v is BridgeEffectValue_Float && v.field0 is BridgeScalar_Keyframed;
  }

  bool _floatLocked(GradeFx fx, String id) {
    final v = _value(fx, id);
    return v is BridgeEffectValue_Float && v.field0 is BridgeScalar_Expression;
  }

  List<double> _defaultColour(GradeFx fx, String id) => switch (_kind(fx, id)) {
        BridgeParamKind_Colour(:final default_) => [
            default_[0],
            default_[1],
            default_[2]
          ],
        _ => const [0, 0, 0],
      };

  List<double> _colour(GradeFx fx, String id) {
    final comp = ui.selectedComp;
    final v = _value(fx, id);
    if (comp == null || v is! BridgeEffectValue_Colour) {
      return _defaultColour(fx, id);
    }
    final time = timeOfFrame(comp, ui.playheadFrame.value);
    final c = v.field0;
    return [
      sampledScalar(c.r, time),
      sampledScalar(c.g, time),
      sampledScalar(c.b, time)
    ];
  }

  bool _colourKeyed(GradeFx fx, String id) {
    final v = _value(fx, id);
    if (v is! BridgeEffectValue_Colour) return false;
    final c = v.field0;
    return [c.r, c.g, c.b].any((s) => s is BridgeScalar_Keyframed);
  }

  List<List<double>> _curve(String id) {
    final v = _value(GradeFx.curves, id);
    return v is BridgeEffectValue_Curve
        ? curvePointsOf(v.field0)
        : curveIdentity;
  }

  List<String> get _lutPaths {
    final v = _value(GradeFx.lut, 'file');
    return v is BridgeEffectValue_File ? v.field0.paths : const [];
  }

  static bool _near(double a, double b) => (a - b).abs() < 1e-6;

  bool _floatChanged(GradeFx fx, String id) =>
      _instance(fx) != null && !_near(_float(fx, id), _defaultFloat(fx, id));

  bool _colourChanged(GradeFx fx, String id) {
    if (_instance(fx) == null) return false;
    final now = _colour(fx, id), was = _defaultColour(fx, id);
    for (var i = 0; i < 3; i++) {
      if (!_near(now[i], was[i])) return true;
    }
    return false;
  }

  bool _curveChanged(String id) {
    final c = _curve(id);
    if (c.length != 2) return true;
    return !(_near(c[0][0], 0) &&
        _near(c[0][1], 0) &&
        _near(c[1][0], 1) &&
        _near(c[1][1], 1));
  }

  bool _changed(_Section s) => switch (s) {
        _Section.balance => ['lift', 'gamma', 'gain']
            .any((id) => _colourChanged(GradeFx.colourBalance, id)),
        _Section.basics => _floatChanged(GradeFx.exposure, 'stops') ||
            _floatChanged(GradeFx.temperature, 'temperature') ||
            _floatChanged(GradeFx.temperature, 'tint') ||
            _floatChanged(GradeFx.contrast, 'contrast') ||
            _floatChanged(GradeFx.saturation, 'saturation'),
        _Section.curves => _instance(GradeFx.curves) != null &&
            ['master', 'red', 'green', 'blue'].any(_curveChanged),
        _Section.look =>
          _lutPaths.isNotEmpty || _floatChanged(GradeFx.lut, 'mix'),
      };

  bool get _anyChanged => _Section.values.any(_changed);

  /// The whole grade: every colour effect on the layer, the ones with no control
  /// here as well, since Copy grade and a look carry those too.
  Set<UuidValue> get _gradeIds =>
      _info == null ? const {} : gradeEffectIds(_info!).toSet();

  bool get _anyPresent => _gradeIds.isNotEmpty;

  bool get _gradeOff {
    final ids = _gradeIds;
    return ids.isNotEmpty &&
        _info!.effects.every((e) => !ids.contains(e.id) || !e.enabled);
  }

  bool _bypassed(List<GradeFx> fxs) {
    final here = [for (final fx in fxs) _instance(fx)].nonNulls.toList();
    return here.isNotEmpty && here.every((e) => !e.enabled);
  }

  // --- Writing ---------------------------------------------------------------
  //
  // Every write lands on each of [_targets], in the one undo step. The drag's
  // preview draws the shown layer alone, as the renderer previews one layer;
  // the rest take the value on release.

  /// A gesture begins: everything it writes, the effect it may add included,
  /// is one undo step until [_touchEnd].
  void _touchStart() {
    for (final layer in _targets) {
      _worn.remove(layer.internallayerId);
    }
    final project = app.project;
    if (project == null) return;
    project.beginUndoGroup();
    project.nameUndoGroup(name: 'Grade');
    _touched = project;
    if (_touches == 0) _touchAdded = false;
    _touches++;
  }

  void _touchEnd() {
    if (_touches == 0) return;
    _touches--;
    if (_touches == 0) _restage = null;
    _touched?.endUndoGroup();
  }

  /// A gesture that ends with nothing written: the preview it staged goes,
  /// and the Viewer is asked for the document again. An effect it added to
  /// preview on is undone with its step, so nothing is left in the history.
  void _touchCancel() {
    _editor.clear();
    _touchEnd();
    if (_touches == 0 && _touchAdded) {
      _touchAdded = false;
      _touched?.undo();
      _tell();
    }
    ui.requestFrame();
  }

  void _tell() {
    app.notifyDocumentChanged();
    ui.model.refresh();
  }

  /// The id of [fx] on [on], the shown layer by default, adding it in its
  /// place when it is not there yet. Inside a touch, so the add is part of
  /// the gesture's step.
  UuidValue? _ensure(GradeFx fx, [LayerReference? on]) {
    final layer = on ?? _layer;
    if (layer == null) return null;
    final info = _infoOf(layer);
    final have = _instance(fx, info);
    if (have != null) return have.id;
    final before = {for (final e in info?.effects ?? const []) e.id};
    try {
      layer.addEffect(name: fx.matchName);
    } catch (_) {
      return null;
    }
    if (_touches > 0) _touchAdded = true;
    final handles = layer.getEffects();
    BridgeEffectInstance? added;
    for (final h in handles) {
      if (!before.contains(h.id())) added = h;
    }
    if (added == null) return null;
    // Its place: after the last grade effect that comes before it, else
    // before the first that comes after it, else where the engine put it.
    final ids = [for (final h in handles) h.id()];
    int? target;
    for (final other in GradeFx.values) {
      if (other == fx) break;
      final at = _instance(other, info);
      if (at != null) target = ids.indexOf(at.id) + 1;
    }
    if (target == null) {
      for (final other in GradeFx.values.skip(fx.index + 1)) {
        final at = _instance(other, info);
        if (at != null) {
          target = ids.indexOf(at.id);
          break;
        }
      }
    }
    final id = added.id();
    if (target != null && target != ids.indexOf(id)) {
      layer.reorderEffect(effect: added, newIndex: target);
    }
    ui.model.refresh();
    return id;
  }

  void _live(GradeFx fx, String param, BridgeEffectValue value) {
    final layer = _layer;
    final comp = ui.selectedComp;
    final id = _ensure(fx);
    if (layer == null || comp == null || id == null) return;
    final frame = ui.playheadFrame.value;
    final scale = ui.viewerScale;
    _restage = () {
      if (mounted) {
        _editor.live(comp, layer, id, param, value, frame: frame, scale: scale);
      }
    };
    _restage!();
  }

  /// [valueFor] makes each layer's value from what that layer holds, so a
  /// keyed row keeps its keys. [all] false writes the shown layer alone, for
  /// a key or an expression, which belong to the one row they were set on.
  void _write(GradeFx fx, String param,
      BridgeEffectValue Function(BridgeLayerInfo? on) valueFor,
      {bool all = true}) {
    for (final layer in all ? _targets : [if (_layer case final l?) l]) {
      final info = _infoOf(layer);
      final id = _ensure(fx, layer);
      if (id != null) _editor.write(layer, id, param, valueFor(info));
    }
    _touchEnd();
    _tell();
  }

  BridgeScalar _scalarFor(GradeFx fx, String id, double v,
      [BridgeLayerInfo? on]) {
    final comp = ui.selectedComp;
    final was = _value(fx, id, on);
    final (lo, hi) = switch (_kind(fx, id)) {
      BridgeParamKind_Float(:final hardMin, :final hardMax) => (
          hardMin ?? double.negativeInfinity,
          hardMax ?? double.infinity
        ),
      _ => (double.negativeInfinity, double.infinity),
    };
    final clamped = v.clamp(lo, hi).toDouble();
    if (comp != null && was is BridgeEffectValue_Float) {
      return scalarWithValueAt(
          was.field0, clamped, comp, ui.playheadFrame.value);
    }
    return BridgeScalar.static_(clamped);
  }

  void _liveFloat(GradeFx fx, String id, double v) =>
      _live(fx, id, BridgeEffectValue.float(_scalarFor(fx, id, v)));

  void _writeFloat(GradeFx fx, String id, double v) => _write(
      fx, id, (on) => BridgeEffectValue.float(_scalarFor(fx, id, v, on)));

  BridgeEffectValue _colourFor(GradeFx fx, String id, List<double> rgb,
      [BridgeLayerInfo? on]) {
    final comp = ui.selectedComp;
    final (lo, hi) = switch (_kind(fx, id)) {
      BridgeParamKind_Colour(:final min, :final max) => (min, max),
      _ => (0.0, 1.0),
    };
    final was = _value(fx, id, on);
    BridgeScalar at(BridgeScalar? s, double v) {
      final clamped = v.clamp(lo, hi).toDouble();
      return s == null || comp == null
          ? BridgeScalar.static_(clamped)
          : scalarWithValueAt(s, clamped, comp, ui.playheadFrame.value);
    }

    final c = was is BridgeEffectValue_Colour ? was.field0 : null;
    return BridgeEffectValue.colour(BridgeColour(
      r: at(c?.r, rgb[0]),
      g: at(c?.g, rgb[1]),
      b: at(c?.b, rgb[2]),
      a: c?.a ?? const BridgeScalar.static_(1),
    ));
  }

  /// Every parameter of the section's effects back to its declared default,
  /// as one step. A keyed value takes the default at the playhead, as an
  /// Effect controls row's reset does, so the animation stays.
  void _reset(List<GradeFx> fxs, {Set<String>? only}) {
    final targets = _targets;
    if (targets.isEmpty) return;
    asOneUndoStep(app.project, () {
      for (final layer in targets) {
        _worn.remove(layer.internallayerId);
        final info = _infoOf(layer);
        for (final fx in fxs) {
          final at = _instance(fx, info);
          if (at == null) continue;
          final values = <String, BridgeEffectValue>{};
          for (final p in _schema[fx]!.values) {
            if (only != null && !only.contains(p.id)) continue;
            final d = defaultEffectValue(p.kind);
            if (d != null) {
              values[p.id] = _defaultAtPlayhead(_value(fx, p.id, info), d);
            }
          }
          if (values.isNotEmpty) _editor.writeAll(layer, at.id, values);
        }
      }
    });
    _tell();
  }

  /// [d], the declared default, written into [was] at the playhead when
  /// [was] is keyed; [d] itself otherwise.
  BridgeEffectValue _defaultAtPlayhead(
      BridgeEffectValue? was, BridgeEffectValue d) {
    final comp = ui.selectedComp;
    if (comp == null) return d;
    final frame = ui.playheadFrame.value;
    double of(BridgeScalar s) => sampledScalar(s, timeOfFrame(comp, frame));
    return switch ((was, d)) {
      (
        BridgeEffectValue_Float(field0: final BridgeScalar_Keyframed k),
        BridgeEffectValue_Float(:final field0)
      ) =>
        BridgeEffectValue.float(scalarWithValueAt(k, of(field0), comp, frame)),
      (
        BridgeEffectValue_Colour(field0: final c),
        BridgeEffectValue_Colour(field0: final dc)
      )
          when _colourKeyedScalar(c) =>
        BridgeEffectValue.colour(BridgeColour(
          r: scalarWithValueAt(c.r, of(dc.r), comp, frame),
          g: scalarWithValueAt(c.g, of(dc.g), comp, frame),
          b: scalarWithValueAt(c.b, of(dc.b), comp, frame),
          a: scalarWithValueAt(c.a, of(dc.a), comp, frame),
        )),
      _ => d,
    };
  }

  static bool _colourKeyedScalar(BridgeColour c) =>
      [c.r, c.g, c.b, c.a].any((s) => s is BridgeScalar_Keyframed);

  /// The section's effects on [on].
  Set<UuidValue> _idsOf(List<GradeFx> fxs, BridgeLayerInfo on) =>
      {for (final fx in fxs) _instance(fx, on)?.id}.nonNulls.toSet();

  static Set<UuidValue> _wholeGrade(BridgeLayerInfo on) =>
      gradeEffectIds(on).toSet();

  void _setBypassed(List<GradeFx> fxs, bool off) =>
      _bypassWhere((on) => _idsOf(fxs, on), off);

  void _bypassWhere(Set<UuidValue> Function(BridgeLayerInfo) idsOf, bool off) {
    _eachTarget((layer, ids) {
      for (final h in layer.getEffects()) {
        if (ids.contains(h.id())) {
          layer.setEffectEnabled(effect: h, enabled: !off);
        }
      }
    }, idsOf);
  }

  void _remove(List<GradeFx> fxs) => _removeWhere((on) => _idsOf(fxs, on));

  void _removeWhere(Set<UuidValue> Function(BridgeLayerInfo) idsOf) {
    _eachTarget((layer, ids) {
      for (final h in layer.getEffects()) {
        if (ids.contains(h.id())) layer.removeEffect(effect: h);
      }
    }, idsOf);
  }

  /// [run] on every target layer with the effects [idsOf] picks there, as one
  /// step.
  void _eachTarget(void Function(LayerReference, Set<UuidValue>) run,
      Set<UuidValue> Function(BridgeLayerInfo) idsOf) {
    final targets = _targets;
    if (targets.isEmpty) return;
    asOneUndoStep(app.project, () {
      for (final layer in targets) {
        if (_infoOf(layer) case final info?) run(layer, idsOf(info));
      }
    });
    _tell();
  }

  /// Pick the section's effects in Effect controls and bring the panel up.
  void _reveal(List<GradeFx> fxs, {bool whole = false}) {
    final layer = _layer;
    if (layer == null) return;
    final ids = whole
        ? _gradeIds.toList()
        : [for (final fx in fxs) _instance(fx)?.id].nonNulls.toList();
    if (ids.isEmpty) return;
    ui.setEffectSelection(layer, ids);
    setPanelVisible(ui.split, Panel.effectControls, true);
    activatePanelTab(ui.split, Panel.effectControls);
    ui.activePane.value = Panel.effectControls.pane();
    ui.workspace.touch();
  }

  /// The picture without the grade, for as long as the button is held: the
  /// same preview path a drag uses, with the grade's effects switched off in
  /// the copy, so the document and its history never see it.
  void _beforeDown() {
    final layer = _layer;
    final comp = ui.selectedComp;
    if (layer == null || comp == null) return;
    final ids = _gradeIds;
    final handles = layer.getEffects();
    for (final h in handles) {
      if (ids.contains(h.id())) h.setEnabled(enabled: false);
    }
    comp.renderFrameWithPreview(
      frame: ui.playheadFrame.value,
      scale: ui.viewerScale,
      layer: layer,
      effects: handles,
    );
    setState(() => _before = true);
  }

  void _beforeUp() {
    if (!_before) return;
    setState(() => _before = false);
    ui.requestFrame();
  }

  // --- Menus -----------------------------------------------------------------

  MenuRow _row(String key, String label, VoidCallback close, VoidCallback run,
          {bool ticked = false, bool enabled = true, String? action}) =>
      MenuRow(
        key: ValueKey<String>(key),
        enabled: enabled,
        onPressed: () {
          close();
          run();
        },
        child:
            MenuLabel(label, ticked: ticked, enabled: enabled, action: action),
      );

  /// A section's or a row's menu: what it can do now, with a row's key and
  /// expression rows between, then the whole grade's rows under a divider.
  /// Nothing greyed: a row that could do nothing is left out, and a section
  /// with no effect yet offers the grade's rows alone. A row with no effect
  /// yet keeps its value and key rows, which add the effect.
  List<Widget> _menuFor(List<GradeFx> fxs, VoidCallback close,
      {required bool changed, GradeFx? rowFx, String? rowParam}) {
    final present = fxs.any((fx) => _instance(fx) != null);
    final param =
        rowFx == null || rowParam == null ? null : _paramMenu(rowFx, rowParam);
    if (!present && param == null) return _groundMenu(close);
    final off = _bypassed(fxs);
    return menuGroups([
      if (present) [
        if (changed)
          _row(
              'grade-section-reset',
              l10n.reset,
              close,
              () => rowParam == null
                  ? _reset(fxs)
                  : _reset([rowFx!], only: {rowParam})),
        _row('grade-section-bypass', l10n.gradeBypassPart(_part(fxs)), close,
            () => _setBypassed(fxs, !off),
            ticked: off),
      ],
      if (param != null) paramCopyRows(context, param, close, tickColumn: true),
      if (param != null) paramKeyRows(context, param, close, tickColumn: true),
      if (present) [
        _row('grade-section-reveal', l10n.gradeRevealInEffectControls, close,
            () => _reveal(fxs)),
      ],
      if (param != null)
        paramLastRows(context, param, close, tickColumn: true),
      if (param != null)
        paramControlRows(context, param, close, tickColumn: true),
      if (present) [
        _row('grade-section-remove', l10n.gradeRemovePart(_part(fxs)), close,
            () => _remove(fxs)),
      ],
      _gradeRows(close),
      _clipboardRows(close),
      _removeRows(close),
    ]);
  }

  /// What a section's or a row's Bypass and Remove act on, for their names: a
  /// heading's whole section, or the one effect behind a row.
  static String _part(List<GradeFx> fxs) => fxs.length > 1
      ? 'primaries'
      : switch (fxs.single) {
          GradeFx.colourBalance => 'balance',
          GradeFx.curves => 'curves',
          GradeFx.lut => 'look',
          GradeFx.exposure => 'exposure',
          GradeFx.contrast => 'contrast',
          GradeFx.saturation => 'saturation',
          GradeFx.temperature => 'temperature',
        };

  /// Copy grade and Paste grade, on any layer that can wear a grade.
  List<Widget> _clipboardRows(VoidCallback close) {
    final ui = this.ui;
    final info = _info;
    if (info == null || !gradeable(info.kind)) return const [];
    return [
      _row('grade-menu-copy', l10n.gradeCopy, close, () => copyGrade(ui),
          enabled: canCopyGrade(ui), action: 'grade.copy'),
      _row('grade-menu-paste', l10n.gradePaste, close,
          () => pasteGrade(app, ui),
          enabled: canPasteGrade(ui), action: 'grade.paste'),
    ];
  }

  List<Widget> _removeRows(VoidCallback close) => [
        if (_anyPresent)
          _row('grade-menu-remove', l10n.gradeRemoveAll, close,
              () => _removeWhere(_wholeGrade)),
      ];

  /// The rows that act on the whole grade, each only while it can.
  List<Widget> _gradeRows(VoidCallback close, {bool reveal = false}) {
    final present = _anyPresent;
    final off = _gradeOff;
    return [
      if (_anyChanged)
        _row('grade-menu-reset', l10n.gradeResetAll, close,
            () => _reset(GradeFx.values)),
      if (present)
        _row('grade-menu-bypass', l10n.gradeBypassAll, close,
            () => _bypassWhere(_wholeGrade, !off),
            ticked: off),
      if (present && reveal)
        _row('grade-menu-reveal', l10n.gradeRevealInEffectControls, close,
            () => _reveal(GradeFx.values, whole: true)),
    ];
  }

  /// The panel's own menu: empty, so no menu, on a layer that can't be graded.
  List<Widget> _groundMenu(VoidCallback close) => menuGroups([
        _gradeRows(close, reveal: true),
        _clipboardRows(close),
        _removeRows(close),
      ]);

  /// The key and expression rows of one parameter's menu: the same rows its
  /// Effect controls row has, acting on the same keys and the same curve. A
  /// layer with no grade yet reads the default, and a write adds the effect
  /// as a drag does.
  ParamMenu? _paramMenu(GradeFx fx, String id) {
    final comp = ui.selectedComp;
    final layer = _layer;
    final at = _instance(fx);
    final v = _value(fx, id) ?? defaultEffectValue(_kind(fx, id));
    if (comp == null || layer == null || v == null) return null;
    final scalars = switch (v) {
      BridgeEffectValue_Float(:final field0) => [field0],
      BridgeEffectValue_Colour(:final field0) => [
          field0.r,
          field0.g,
          field0.b,
          field0.a
        ],
      _ => null,
    };
    if (scalars == null) return null;
    final expression = switch (v) {
      BridgeEffectValue_Float(field0: final BridgeScalar_Expression e) => e,
      _ => null,
    };
    BridgeEffectValue rebuilt(List<BridgeScalar> next) =>
        v is BridgeEffectValue_Colour
            ? BridgeEffectValue.colour(
                BridgeColour(r: next[0], g: next[1], b: next[2], a: next[3]))
            : BridgeEffectValue.float(next[0]);
    final colour = v is BridgeEffectValue_Colour;
    // Each row is one Grade step, the effect it may add included.
    void write(GradeFx fx, String id,
        BridgeEffectValue Function(BridgeLayerInfo? on) valueFor,
        {bool all = true}) {
      _touchStart();
      _write(fx, id, valueFor, all: all);
    }

    return ParamMenu(
      // A row of the grade's own effect, once the layer has it.
      control: switch ((at, _schema[fx]?[id])) {
        (final at?, final param?) => switch (
              paramControlRow(layer.internallayerId, at.id, param)) {
            final row? => ControlRow(comp, row),
            null => null,
          },
        _ => null,
      },
      // The value as text, as Effect controls copies it, so it pastes into any
      // other row or field, and onto every picked layer.
      copy: expression != null
          ? null
          : () => copiedAxes(
              colour ? _colour(fx, id).take(3) : [_float(fx, id)]),
      paste: expression != null
          ? null
          : (text) {
              final n = pastedAxes(text, colour ? 3 : 1);
              if (n == null) return;
              write(
                  fx,
                  id,
                  (on) => colour
                      ? _colourFor(fx, id, n, on)
                      : BridgeEffectValue.float(_scalarFor(fx, id, n[0], on)));
            },
      keys: expression != null
          ? null
          : KeyframeControlsFrb(
              scalars: scalars,
              comp: comp,
              onSeek: (frame) => ui.playheadFrame.value = frame,
              rowKey: 'grade-${fx.matchName}-$id',
              onWrite: (next) =>
                  write(fx, id, (_) => rebuilt(next), all: false),
            ),
      setExpression: v is BridgeEffectValue_Float && expression == null
          ? () => write(
              fx,
              id,
              (_) => BridgeEffectValue.float(BridgeScalar.expression(
                  _float(fx, id).toString(), newExpressionLanguage(context))),
              all: false)
          : null,
      removeExpression: expression == null
          ? null
          : () => write(
              fx,
              id,
              (_) => BridgeEffectValue.float(BridgeScalar.static_(
                  sampleScalarWithContext(
                      scalar: expression,
                      time: timeOfFrame(comp, ui.playheadFrame.value),
                      layer: layer))),
              all: false),
      graphPath: at == null
          ? null
          : '${effectPath('${layer.internallayerId}', '${at.id}')}/$id',
    );
  }

  // --- Drawing ---------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final ui = this.ui;
    return ListenableBuilder(
      listenable: Listenable.merge([
        ui.model,
        ui.selectedLayer,
        ui.selectedLayers,
        ui.playheadFrame,
        ui.selectedEffects
      ]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        final info = _info;
        final enabled =
            info != null && ui.selectedComp != null && gradeable(info.kind);
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _header(t, info, enabled),
            Expanded(
              child: GradeMenu(
                rows: _groundMenu,
                child: Container(
                  color: t.surface0,
                  child: LayoutBuilder(builder: (context, viewport) {
                    // A wide pane puts the curve in a column of its own,
                    // as tall as the pane, beside the rest.
                    final wide = viewport.maxWidth >= _wideAt;
                    if (!wide) {
                      WidgetsBinding.instance.addPostFrameCallback(
                          (_) => _fitPlot(viewport.maxHeight));
                    }
                    return EdgeFade(
                      axis: Axis.vertical,
                      child: SingleChildScrollView(
                        primary: false,
                        padding: const EdgeInsets.only(bottom: _listFoot),
                        child: wide
                            ? Row(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: [
                                  Expanded(
                                    flex: 3,
                                    child: Column(
                                      crossAxisAlignment:
                                          CrossAxisAlignment.stretch,
                                      children: [
                                        _balance(t, enabled),
                                        _basics(t, enabled),
                                        _look(t, enabled),
                                      ],
                                    ),
                                  ),
                                  Expanded(
                                    flex: 2,
                                    child: _curves(t, enabled,
                                        room: viewport.maxHeight -
                                            _listFoot -
                                            _headingHeight),
                                  ),
                                ],
                              )
                            : Column(
                                key: _content,
                                crossAxisAlignment: CrossAxisAlignment.stretch,
                                children: [
                                  _balance(t, enabled),
                                  _basics(t, enabled),
                                  _look(t, enabled),
                                  _curves(t, enabled),
                                ],
                              ),
                      ),
                    );
                  }),
                ),
              ),
            ),
          ],
        );
      },
    );
  }

  /// The curve, last, takes what the other sections leave, so it shows whole;
  /// below its least it keeps that and the pane scrolls. Look sits above it,
  /// where a short pane still shows it.
  void _fitPlot(double viewport) {
    final box = _content.currentContext?.findRenderObject() as RenderBox?;
    if (!mounted || box == null || !box.hasSize) return;
    final room = viewport - _listFoot - (box.size.height - _plot);
    if ((room - _plotRoom).abs() > 0.5) setState(() => _plotRoom = room);
  }

  Widget _header(LumitTheme t, BridgeLayerInfo? info, bool enabled) =>
      GradeMenu(
        rows: _groundMenu,
        child: Container(
          height: 26,
          color: t.surface1,
          padding: const EdgeInsets.only(left: gradeInset, right: 4),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  !enabled
                      ? ''
                      : _targets.length > 1
                          ? l10n.gradeLayerCount(_targets.length)
                          : info!.name,
                  style: t.small.copyWith(color: t.textMuted),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              if (enabled && _anyPresent)
                Listener(
                  onPointerDown: (_) => _beforeDown(),
                  onPointerUp: (_) => _beforeUp(),
                  onPointerCancel: (_) => _beforeUp(),
                  child: GradeIconButton(
                    key: const ValueKey('grade-before'),
                    // The effect bypass glyph: held, the grade is bypassed. The
                    // layer's eye one panel down means hide, which this is not.
                    glyph: LumitIcons.bypass,
                    tip: l10n.gradeBefore,
                    active: _before,
                    onPressed: () {},
                  ),
                ),
              if (enabled && _anyChanged)
                GradeIconButton(
                  key: const ValueKey('grade-reset-all'),
                  glyph: LumitIcons.reset,
                  tip: l10n.gradeResetAll,
                  onPressed: () => _reset(GradeFx.values),
                ),
            ],
          ),
        ),
      );

  Widget _heading(_Section s, String label) => GradeMenu(
        rows: (close) => _menuFor(s.effects, close, changed: _changed(s)),
        child: GradeHeading(
          label: label,
          changed: _changed(s),
          onReset: () => _reset(s.effects),
        ),
      );

  Widget _balance(LumitTheme t, bool enabled) {
    const fx = GradeFx.colourBalance;
    Widget wheel(String id, double size, {required bool numbers}) {
      final rgb = _colour(fx, id);
      final neutral = _defaultColour(fx, id)[0];
      final (reach, span) = _wheelRanges[id]!;
      final keyed = _colourKeyed(fx, id);
      final changed = _colourChanged(fx, id);
      final label = engineLabel(_schema[fx]![id]!.label);
      return GradeMenu(
        rows: (close) => _menuFor([fx], close,
            changed: changed, rowFx: fx, rowParam: id),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            GradeWheel(
              key: ValueKey<String>('grade-wheel-$id'),
              rgb: rgb,
              neutral: neutral,
              reach: reach,
              masterSpan: span,
              size: size,
              enabled: enabled,
              keyed: keyed,
              onStart: _touchStart,
              onLive: (c) => _live(fx, id, _colourFor(fx, id, c)),
              onCommit: (c) =>
                  _write(fx, id, (on) => _colourFor(fx, id, c, on)),
              onCancel: _touchCancel,
            ),
            const SizedBox(height: 4),
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  label,
                  style: t.small.copyWith(
                    color: keyed
                        ? t.animated
                        : !enabled
                            ? t.textDisabled
                            : changed
                                ? t.textSecondary
                                : t.textMuted,
                  ),
                ),
                const SizedBox(width: 6),
                _wheelNumber(id, rgb, null, enabled, keyed),
              ],
            ),
            if (numbers)
              Padding(
                padding: const EdgeInsets.only(top: 2),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    for (var i = 0; i < 3; i++) ...[
                      if (i > 0) const SizedBox(width: 6),
                      _wheelNumber(id, rgb, i, enabled, keyed),
                    ],
                  ],
                ),
              ),
          ],
        ),
      );
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _heading(_Section.balance, l10n.gradeBalance),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: gradeInset),
          child: LayoutBuilder(builder: (context, box) {
            // Three across while they have room, one under another when
            // the panel is too narrow for three worth turning.
            final per = (box.maxWidth - 2 * 8) / 3 - 16;
            final across = per >= 56;
            final size = across ? per.clamp(56.0, 88.0) : 72.0;
            // The channel numbers under each wheel, where three fit.
            final numbers = !across || (box.maxWidth - 2 * 8) / 3 >= 104;
            final wheels = [
              for (final id in const ['lift', 'gamma', 'gain'])
                wheel(id, size, numbers: numbers)
            ];
            return across
                ? Row(
                    mainAxisAlignment: MainAxisAlignment.spaceEvenly,
                    children: wheels,
                  )
                : Column(
                    children: [
                      for (final w in wheels)
                        Padding(
                            padding: const EdgeInsets.only(bottom: 8),
                            child: w),
                    ],
                  );
          }),
        ),
      ],
    );
  }

  /// One of a wheel's numbers: a channel, or with [channel] null the master,
  /// which moves the three together as the bar does. Typed or dragged, it
  /// writes the whole colour, as the wheel does.
  Widget _wheelNumber(
      String id, List<double> rgb, int? channel, bool enabled, bool keyed) {
    const fx = GradeFx.colourBalance;
    final (lo, hi) = switch (_kind(fx, id)) {
      BridgeParamKind_Colour(:final min, :final max) => (min, max),
      _ => (0.0, 1.0),
    };
    final neutral = _defaultColour(fx, id)[0];
    final master = wheelMaster(rgb);
    List<double> next(num v) => channel == null
        ? [for (final c in rgb) c + v.toDouble() - master]
        : [for (var i = 0; i < 3; i++) i == channel ? v.toDouble() : rgb[i]];
    void commit(num v) =>
        _write(fx, id, (on) => _colourFor(fx, id, next(v), on));
    final field = DragValueField(
      key: ValueKey<String>('grade-wheel-$id-${channel ?? 'master'}'),
      bare: true,
      value: channel == null ? master : rgb[channel],
      min: lo,
      max: hi,
      speed: _wheelRanges[id]!.$2 / 100,
      decimals: 2,
      signed: neutral == 0,
      keyed: keyed,
      resetTo: neutral,
      onChangeStart: _touchStart,
      onChangeLive: (v) => _live(fx, id, _colourFor(fx, id, next(v))),
      onChangeEnd: commit,
      onDragCancel: _touchCancel,
      onChanged: (v) {
        _touchStart();
        commit(v);
      },
    );
    const channels = ['red', 'green', 'blue'];
    return IgnorePointer(
      ignoring: !enabled,
      child: Opacity(
        opacity: enabled ? 1 : 0.5,
        child: channel == null
            ? field
            // A channel's own colour under its number says which it is.
            : LumitTooltip(
                message: engineLabel(
                    _schema[GradeFx.curves]![channels[channel]]!.label),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    field,
                    Container(
                      width: 12,
                      height: 1.5,
                      color: curveChannelColour(channels[channel]),
                    ),
                  ],
                ),
              ),
      ),
    );
  }

  /// The number a pick reads the picture against is the document's, never a
  /// preview's: the window of pixels the dropper reads is cut from the frame
  /// the document draws, so the picture it sees has the rows as they stand.
  BridgeWhiteBalance _neutralFor(DropperSample s) {
    const fx = GradeFx.temperature;
    return whiteBalanceFor(
      r: s.r,
      g: s.g,
      b: s.b,
      temperature: _float(fx, 'temperature'),
      tint: _float(fx, 'tint'),
    );
  }

  void _neutralLive(DropperSample s) {
    const fx = GradeFx.temperature;
    final solved = _neutralFor(s);
    if (!_neutralTouched) {
      _touchStart();
      _neutralTouched = true;
    }
    _liveFloat(fx, 'temperature', solved.temperature);
    _liveFloat(fx, 'tint', solved.tint);
  }

  /// Both rows as one write, so one undo step takes the whole pick back.
  void _neutralPick(DropperSample s) {
    const fx = GradeFx.temperature;
    final solved = _neutralFor(s);
    if (!_neutralTouched) _touchStart();
    _neutralTouched = false;
    for (final layer in _targets) {
      final info = _infoOf(layer);
      final id = _ensure(fx, layer);
      if (id == null) continue;
      _editor.writeAll(layer, id, {
        'temperature': BridgeEffectValue.float(
            _scalarFor(fx, 'temperature', solved.temperature, info)),
        'tint':
            BridgeEffectValue.float(_scalarFor(fx, 'tint', solved.tint, info)),
      });
    }
    _touchEnd();
    _tell();
  }

  /// Abandoned: the preview goes, and the effect too if the pick brought it.
  void _neutralRevert() {
    if (!_neutralTouched) return;
    _neutralTouched = false;
    _touchCancel();
  }

  Widget _neutralDropper(bool enabled) {
    const fx = GradeFx.temperature;
    if (!enabled ||
        _floatLocked(fx, 'temperature') ||
        _floatLocked(fx, 'tint')) {
      return const SizedBox(width: 18);
    }
    return DropperButton(
      id: 'grade-neutral',
      tip: l10n.gradePickNeutral,
      arm: (ui) => ui.armDropper(DropperArm(
        id: 'grade-neutral',
        reads: DropperReads.colour,
        label: l10n.gradePickNeutral,
        onPreview: _neutralLive,
        onPick: _neutralPick,
        onRevert: _neutralRevert,
      )),
    );
  }

  Widget _basics(LumitTheme t, bool enabled) {
    Widget row(GradeFx fx, String id,
        {int decimals = 1, bool signed = false, Widget? trailing}) {
      final (min, max) = _range(fx, id);
      final (hardMin, hardMax) = _hardRange(fx, id);
      final neutral = _defaultFloat(fx, id);
      final changed = _floatChanged(fx, id);
      // The track sits its neutral value in the middle, however far the
      // effect's own range runs past it one way: an untouched row reads as
      // untouched. The number keeps the whole range.
      final span = math.min(neutral - min, max - neutral);
      return GradeSlider(
        key: ValueKey<String>('grade-row-${fx.matchName}-$id'),
        // Exposure's one number is called Stops on its own row; here, among
        // the other primaries, it is the exposure.
        label: fx == GradeFx.exposure
            ? l10n.fxExposure
            : engineLabel(_schema[fx]![id]!.label),
        value: _float(fx, id),
        min: neutral - span,
        max: neutral + span,
        fieldMin: hardMin,
        fieldMax: hardMax,
        neutral: neutral,
        decimals: decimals,
        signed: signed,
        enabled: enabled && !_floatLocked(fx, id),
        keyed: _floatKeyed(fx, id),
        onStart: _touchStart,
        onLive: (v) => _liveFloat(fx, id, v),
        onCommit: (v) => _writeFloat(fx, id, v),
        onCancel: _touchCancel,
        menu: (close) => _menuFor([fx], close,
            changed: changed, rowFx: fx, rowParam: id),
        trailing: trailing ?? const SizedBox(width: 18),
      );
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _heading(_Section.basics, l10n.gradePrimaries),
        row(GradeFx.exposure, 'stops', decimals: 2, signed: true),
        row(GradeFx.contrast, 'contrast'),
        row(GradeFx.saturation, 'saturation'),
        row(GradeFx.temperature, 'temperature',
            signed: true, trailing: _neutralDropper(enabled)),
        row(GradeFx.temperature, 'tint', signed: true),
      ],
    );
  }

  /// [room] is the height the curve may take in a wide pane's own column;
  /// otherwise it takes what [_fitPlot] measured.
  Widget _curves(LumitTheme t, bool enabled, {double? room}) {
    const fx = GradeFx.curves;
    const ids = ['master', 'red', 'green', 'blue'];
    final labels = [for (final id in ids) engineLabel(_schema[fx]![id]!.label)];
    final channel = _curveChannel.clamp(0, ids.length - 1);
    final id = ids[channel];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _heading(_Section.curves, l10n.fxCurves),
        GradeMenu(
          rows: (close) => _menuFor([fx], close,
              changed: _curveChanged(id), rowFx: fx, rowParam: id),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: gradeInset),
            child: LayoutBuilder(builder: (context, box) {
              final column = channelColumnWidth(labels) + channelColumnGap;
              // Grows with the pane: the curve a colourist draws wants room,
              // and a small square beside long sliders reads as an afterthought.
              final most = room == null ? _plotMax : _plotWideMax;
              final plot = _plot = math.min(box.maxWidth - column,
                      (room ?? _plotRoom).clamp(_plotMin, most).toDouble())
                  .clamp(48.0, most)
                  .toDouble();
              return Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  IgnorePointer(
                    ignoring: !enabled,
                    child: Opacity(
                      opacity: enabled ? 1 : 0.5,
                      child: CurveEditor(
                        key: ValueKey<String>('grade-curve-$id'),
                        points: _curve(id),
                        size: plot,
                        line: curveChannelColour(id),
                        onLive: (p) {
                          if (_touches == 0) _touchStart();
                          _live(fx, id, curveValue(p));
                        },
                        onCommit: (p) {
                          if (_touches == 0) _touchStart();
                          _write(fx, id, (_) => curveValue(p));
                        },
                      ),
                    ),
                  ),
                  const SizedBox(width: channelColumnGap),
                  ChannelButtons(
                    labels: labels,
                    channel: channel,
                    onChannel: (i) => setState(() => _curveChannel = i),
                    keyPrefix: 'grade-curves',
                    colours: [for (final id in ids) curveChannelColour(id)],
                  ),
                ],
              );
            }),
          ),
        ),
      ],
    );
  }

  Widget _look(LumitTheme t, bool enabled) {
    const fx = GradeFx.lut;
    final paths = _lutPaths;
    final set = paths.isNotEmpty;
    final name = set ? paths.first.split(RegExp(r'[/\\]')).last : null;
    final (min, max) = _range(fx, 'mix');
    final filter = switch (_kind(fx, 'file')) {
      BridgeParamKind_File(:final filterName) => engineLabel(filterName),
      _ => l10n.fxLut,
    };
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _heading(_Section.look, l10n.fxLook),
        GradeMenu(
          rows: _groundMenu,
          child: Padding(
            padding:
                const EdgeInsets.symmetric(horizontal: gradeInset, vertical: 1),
            // The picker leads its row: it names the look worn, and the
            // heading above already says Look.
            child: Align(
              alignment: AlignmentDirectional.centerStart,
              child: LookPicker(
                key: const ValueKey('grade-look'),
                enabled: enabled,
                current: _wornLook,
                onSave: enabled && canCopyGrade(ui) ? _saveLook : null,
                onPick: _pickLook,
              ),
            ),
          ),
        ),
        GradeMenu(
          rows: (close) =>
              _menuFor([fx], close, changed: _changed(_Section.look)),
          child: Padding(
            padding:
                const EdgeInsets.symmetric(horizontal: gradeInset, vertical: 1),
            child: Row(
              children: [
                SizedBox(
                  width: 74,
                  child: Text(l10n.fxLut,
                      style: t.small.copyWith(
                          color: enabled ? t.textMuted : t.textDisabled)),
                ),
                // As wide as its name and no wider: a button the width of
                // the panel reads as a field.
                Flexible(
                  child: Align(
                    alignment: AlignmentDirectional.centerStart,
                    child: LumitTooltip(
                      message: set ? paths.first : l10n.chooseA(filter),
                      child: HouseButton(
                        key: const ValueKey('grade-lut-file'),
                        small: true,
                        onPressed: !enabled ? null : _pickLut,
                        child: Text(
                          name ?? l10n.chooseEllipsis,
                          style: t.small,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                    ),
                  ),
                ),
                if (set)
                  GradeIconButton(
                    key: const ValueKey('grade-lut-clear'),
                    glyph: LumitIcons.close,
                    tip: l10n.clear,
                    onPressed: () {
                      _touchStart();
                      _write(
                          fx,
                          'file',
                          (_) => const BridgeEffectValue.file(BridgeFileParam(
                              paths: [], index: BridgeScalar.static_(0))));
                    },
                  ),
              ],
            ),
          ),
        ),
        if (set)
          GradeSlider(
            key: const ValueKey('grade-row-lut-mix'),
            label: engineLabel(_schema[fx]!['mix']!.label),
            value: _float(fx, 'mix'),
            min: min,
            max: max,
            neutral: _defaultFloat(fx, 'mix'),
            decimals: 0,
            enabled: enabled && !_floatLocked(fx, 'mix'),
            keyed: _floatKeyed(fx, 'mix'),
            onStart: _touchStart,
            onLive: (v) => _liveFloat(fx, 'mix', v),
            onCommit: (v) => _writeFloat(fx, 'mix', v),
            onCancel: _touchCancel,
            menu: (close) => _menuFor([fx], close,
                changed: _floatChanged(fx, 'mix'),
                rowFx: fx,
                rowParam: 'mix'),
          ),
      ],
    );
  }

  /// The look on the layer, while its grade is still the one the look brought.
  String? get _wornLook {
    final info = _info;
    final worn = _layer == null ? null : _worn[_layer!.internallayerId];
    if (info == null || worn == null) return null;
    final ids = gradeEffectIds(info);
    final same = ids.length == worn.$2.length &&
        Iterable.generate(ids.length).every((i) => ids[i] == worn.$2[i]);
    return same ? worn.$1 : null;
  }

  /// A look takes the place of the grade on every selected layer.
  Future<void> _pickLook(BridgePresetInfo look) async {
    final String text;
    try {
      text = await readEffectPreset(path: look.path);
    } catch (_) {
      return;
    }
    if (!mounted) return;
    final ui = this.ui;
    layGrade(app, ui, text, name: 'Apply look');
    if (!mounted) return;
    setState(() {
      for (final layer in ui.selectedLayers.value) {
        final info = ui.model.byId(layer.internallayerId)?.info;
        if (info != null) {
          _worn[layer.internallayerId] = (look.name, gradeEffectIds(info));
        }
      }
    });
  }

  /// Write the layer's grade as a preset in the Looks group, which is made on
  /// the first save.
  Future<void> _saveLook() async {
    final layer = _layer;
    final info = _info;
    if (layer == null || info == null) return;
    // The layer's name to start from, less a file's extension.
    final name = await askThemeName(context,
        title: l10n.gradeSaveLookTitle,
        suggested: info.name.replaceFirst(RegExp(r'\.[A-Za-z0-9]{2,4}$'), ''),
        confirm: l10n.save,
        label: l10n.name);
    final library = presetsDirPath();
    if (name == null || library == null || !mounted) return;
    // The name can hold what a file's name can't.
    final stem = name.replaceAll(RegExp(r'[/\\:*?"<>|]'), '-');
    final sep = Platform.pathSeparator;
    final file = File('$library$sep$looksGroup$sep$stem.lumfx');
    if (file.existsSync() &&
        !await askBefore(
            context, l10n.gradeLookReplace(name), l10n.gradeLookReplaceYes)) {
      return;
    }
    if (!mounted) return;
    try {
      makePresetGroup(group: looksGroup);
      final ids = gradeEffectIds(info);
      file.writeAsStringSync(layer.savePresetParts(
          name: name, effects: ids, transformKeys: false, textAnimators: false));
      setState(() => _worn[layer.internallayerId] = (name, ids));
    } catch (_) {
      // A folder that will not take the file. The document is untouched.
      return;
    }
    ui.presetsChanged.value++;
    app.postNotice(l10n.gradeLookSavedNotice);
  }

  Future<void> _pickLut() async {
    final kind = _kind(GradeFx.lut, 'file');
    final (extensions, label) = switch (kind) {
      BridgeParamKind_File(:final filter, :final filterName) => (
          filter,
          filterName
        ),
      _ => (const <String>['cube'], 'LUT'),
    };
    final path = await pickEffectInputFile(extensions, label);
    if (path == null || !mounted) return;
    _touchStart();
    _write(
        GradeFx.lut,
        'file',
        (_) => BridgeEffectValue.file(BridgeFileParam(
            paths: [path], index: const BridgeScalar.static_(0))));
  }
}
