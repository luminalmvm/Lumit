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

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../state/dock.dart';
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/curve_editor.dart';
import 'effect_controls_panel_frb.dart' show curveChannelColour;
import 'effect_param_row_frb.dart'
    show
        EffectStackEditor,
        curvePointsOf,
        curveValue,
        defaultEffectValue,
        scalarValueAt;
import 'grade_controls.dart';
import 'grade_wheel.dart';
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'layer_fold_frb.dart' show asOneUndoStep;

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

/// The layers a grade can sit on: the ones that draw a picture.
bool _gradeable(BridgeLayerKind kind) => switch (kind) {
      BridgeLayerKind.camera ||
      BridgeLayerKind.light ||
      BridgeLayerKind.nullLayer ||
      BridgeLayerKind.audio ||
      BridgeLayerKind.bus =>
        false,
      _ => true,
    };

/// How far a wheel's rim reaches, and how far its bar travels, per colour.
/// Lift adds, so a little goes a long way; Gamma and Gain multiply.
const Map<String, (double reach, double span)> _wheelRanges = {
  'lift': (0.5, 0.5),
  'gamma': (1.0, 1.0),
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

  /// Undo groups opened by a gesture and not yet closed.
  int _touches = 0;

  /// Whether the before picture is being held up.
  bool _before = false;

  /// The project a touch opened its group on, so the group can be closed
  /// from `dispose`, where nothing may be looked up.
  ProjectReference? _touched;

  @override
  void dispose() {
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
  BridgeLayerInfo? get _info =>
      _layer == null ? null : ui.model.byId(_layer!.internallayerId)?.info;

  /// The first instance of [fx] on the layer, or null when none is there.
  BridgeEffectInstanceInfo? _instance(GradeFx fx) {
    for (final e in _info?.effects ?? const <BridgeEffectInstanceInfo>[]) {
      if (e.name == fx.matchName) return e;
    }
    return null;
  }

  BridgeEffectValue? _value(GradeFx fx, String id) {
    for (final v in _instance(fx)?.values ?? const <BridgeParamValue>[]) {
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
  bool get _anyPresent => GradeFx.values.any((fx) => _instance(fx) != null);
  bool _bypassed(List<GradeFx> fxs) {
    final here = [for (final fx in fxs) _instance(fx)].nonNulls.toList();
    return here.isNotEmpty && here.every((e) => !e.enabled);
  }

  // --- Writing ---------------------------------------------------------------

  /// A gesture begins: everything it writes, the effect it may add included,
  /// is one undo step until [_touchEnd].
  void _touchStart() {
    final project = app.project;
    if (project == null) return;
    project.beginUndoGroup();
    _touched = project;
    _touches++;
  }

  void _touchEnd() {
    if (_touches == 0) return;
    _touches--;
    _touched?.endUndoGroup();
  }

  void _tell() {
    app.notifyDocumentChanged();
    ui.model.refresh();
  }

  /// The id of [fx] on the layer, adding it in its place when it is not
  /// there yet. Inside a touch, so the add is part of the gesture's step.
  UuidValue? _ensure(GradeFx fx) {
    final layer = _layer;
    if (layer == null) return null;
    final have = _instance(fx);
    if (have != null) return have.id;
    final before = {for (final e in _info?.effects ?? const []) e.id};
    try {
      layer.addEffect(name: fx.matchName);
    } catch (_) {
      return null;
    }
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
      final at = _instance(other);
      if (at != null) target = ids.indexOf(at.id) + 1;
    }
    if (target == null) {
      for (final other in GradeFx.values.skip(fx.index + 1)) {
        final at = _instance(other);
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
    _editor.live(comp, layer, id, param, value,
        frame: ui.playheadFrame.value, scale: ui.viewerScale);
  }

  void _write(GradeFx fx, String param, BridgeEffectValue value) {
    final layer = _layer;
    final id = _ensure(fx);
    if (layer != null && id != null) _editor.write(layer, id, param, value);
    _touchEnd();
    _tell();
  }

  BridgeScalar _scalarFor(GradeFx fx, String id, double v) {
    final comp = ui.selectedComp;
    final was = _value(fx, id);
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

  void _writeFloat(GradeFx fx, String id, double v) =>
      _write(fx, id, BridgeEffectValue.float(_scalarFor(fx, id, v)));

  BridgeEffectValue _colourFor(GradeFx fx, String id, List<double> rgb) {
    final comp = ui.selectedComp;
    final (lo, hi) = switch (_kind(fx, id)) {
      BridgeParamKind_Colour(:final min, :final max) => (min, max),
      _ => (0.0, 1.0),
    };
    final was = _value(fx, id);
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
  /// as one step. Keys go with it: a reset is a clean slate.
  void _reset(List<GradeFx> fxs, {Set<String>? only}) {
    final layer = _layer;
    if (layer == null) return;
    asOneUndoStep(app.project, () {
      for (final fx in fxs) {
        final at = _instance(fx);
        if (at == null) continue;
        final values = <String, BridgeEffectValue>{};
        for (final p in _schema[fx]!.values) {
          if (only != null && !only.contains(p.id)) continue;
          final d = defaultEffectValue(p.kind);
          if (d != null) values[p.id] = d;
        }
        if (values.isNotEmpty) _editor.writeAll(layer, at.id, values);
      }
    });
    _tell();
  }

  void _setBypassed(List<GradeFx> fxs, bool off) {
    final layer = _layer;
    if (layer == null) return;
    final ids = {for (final fx in fxs) _instance(fx)?.id}.nonNulls.toSet();
    asOneUndoStep(app.project, () {
      for (final h in layer.getEffects()) {
        if (ids.contains(h.id())) {
          layer.setEffectEnabled(effect: h, enabled: !off);
        }
      }
    });
    _tell();
  }

  void _remove(List<GradeFx> fxs) {
    final layer = _layer;
    if (layer == null) return;
    final ids = {for (final fx in fxs) _instance(fx)?.id}.nonNulls.toSet();
    asOneUndoStep(app.project, () {
      for (final h in layer.getEffects()) {
        if (ids.contains(h.id())) layer.removeEffect(effect: h);
      }
    });
    _tell();
  }

  /// Pick the section's effects in Effect controls and bring the panel up.
  void _reveal(List<GradeFx> fxs) {
    final layer = _layer;
    if (layer == null) return;
    final ids = [for (final fx in fxs) _instance(fx)?.id].nonNulls.toList();
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
    final ids = {for (final fx in GradeFx.values) _instance(fx)?.id}.nonNulls;
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

  List<Widget> _menuFor(List<GradeFx> fxs, VoidCallback close,
      {required bool changed, String? resetParam, GradeFx? resetFx}) {
    final present = fxs.any((fx) => _instance(fx) != null);
    final off = _bypassed(fxs);
    return [
      MenuRow(
        enabled: changed,
        onPressed: () {
          close();
          resetParam == null
              ? _reset(fxs)
              : _reset([resetFx!], only: {resetParam});
        },
        child: MenuLabel(l10n.reset, ticked: false, enabled: changed),
      ),
      MenuRow(
        enabled: present,
        onPressed: () {
          close();
          _setBypassed(fxs, !off);
        },
        child: MenuLabel(l10n.fxBypass, ticked: off, enabled: present),
      ),
      const MenuDivider(),
      MenuRow(
        enabled: present,
        onPressed: () {
          close();
          _reveal(fxs);
        },
        child: MenuLabel(l10n.gradeRevealInEffectControls,
            ticked: false, enabled: present),
      ),
      const MenuDivider(),
      MenuRow(
        enabled: present,
        onPressed: () {
          close();
          _remove(fxs);
        },
        child: MenuLabel(l10n.delete, ticked: false, enabled: present),
      ),
    ];
  }

  List<Widget> _groundMenu(VoidCallback close) {
    final present = _anyPresent;
    final off = _bypassed(GradeFx.values);
    return [
      MenuRow(
        key: const ValueKey('grade-menu-reset'),
        enabled: _anyChanged,
        onPressed: () {
          close();
          _reset(GradeFx.values);
        },
        child:
            MenuLabel(l10n.gradeResetAll, ticked: false, enabled: _anyChanged),
      ),
      MenuRow(
        key: const ValueKey('grade-menu-bypass'),
        enabled: present,
        onPressed: () {
          close();
          _setBypassed(GradeFx.values, !off);
        },
        child: MenuLabel(l10n.gradeBypassAll, ticked: off, enabled: present),
      ),
      const MenuDivider(),
      MenuRow(
        key: const ValueKey('grade-menu-reveal'),
        enabled: present,
        onPressed: () {
          close();
          _reveal(GradeFx.values);
        },
        child: MenuLabel(l10n.gradeRevealInEffectControls,
            ticked: false, enabled: present),
      ),
      const MenuDivider(),
      MenuRow(
        key: const ValueKey('grade-menu-remove'),
        enabled: present,
        onPressed: () {
          close();
          _remove(GradeFx.values);
        },
        child: MenuLabel(l10n.gradeRemoveAll, ticked: false, enabled: present),
      ),
    ];
  }

  // --- Drawing ---------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final ui = this.ui;
    return ListenableBuilder(
      listenable: Listenable.merge(
          [ui.model, ui.selectedLayer, ui.playheadFrame, ui.selectedEffects]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        final info = _info;
        final enabled =
            info != null && ui.selectedComp != null && _gradeable(info.kind);
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _header(t, info, enabled),
            Expanded(
              child: GradeMenu(
                rows: _groundMenu,
                child: Container(
                  color: t.surface0,
                  child: ListView(
                    padding: const EdgeInsets.only(bottom: 8),
                    children: [
                      _balance(t, enabled),
                      _basics(t, enabled),
                      _curves(t, enabled),
                      _look(t, enabled),
                    ],
                  ),
                ),
              ),
            ),
          ],
        );
      },
    );
  }

  Widget _header(LumitTheme t, BridgeLayerInfo? info, bool enabled) =>
      Container(
        height: 26,
        color: t.surface1,
        padding: const EdgeInsets.only(left: gradeInset, right: 4),
        child: Row(
          children: [
            Expanded(
              child: Text(
                enabled ? info!.name : '',
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
                  glyph: LumitIcons.visible,
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
    Widget wheel(String id, double size) {
      final rgb = _colour(fx, id);
      final neutral = _defaultColour(fx, id)[0];
      final (reach, span) = _wheelRanges[id]!;
      final keyed = _colourKeyed(fx, id);
      final changed = _colourChanged(fx, id);
      final label = engineLabel(_schema[fx]![id]!.label);
      return GradeMenu(
        rows: (close) => _menuFor([fx], close,
            changed: changed, resetParam: id, resetFx: fx),
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
              onCommit: (c) => _write(fx, id, _colourFor(fx, id, c)),
            ),
            const SizedBox(height: 4),
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
            final wheels = [
              for (final id in const ['lift', 'gamma', 'gain']) wheel(id, size)
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

  Widget _basics(LumitTheme t, bool enabled) {
    Widget row(GradeFx fx, String id, {int decimals = 1, bool signed = false}) {
      final (min, max) = _range(fx, id);
      final neutral = _defaultFloat(fx, id);
      final changed = _floatChanged(fx, id);
      return GradeSlider(
        key: ValueKey<String>('grade-row-${fx.matchName}-$id'),
        // Exposure's one number is called Stops on its own row; here, among
        // the other primaries, it is the exposure.
        label: fx == GradeFx.exposure
            ? l10n.fxExposure
            : engineLabel(_schema[fx]![id]!.label),
        value: _float(fx, id),
        min: min,
        max: max,
        neutral: neutral,
        decimals: decimals,
        signed: signed,
        enabled: enabled && !_floatLocked(fx, id),
        keyed: _floatKeyed(fx, id),
        onStart: _touchStart,
        onLive: (v) => _liveFloat(fx, id, v),
        onCommit: (v) => _writeFloat(fx, id, v),
        onCancel: _touchEnd,
        menu: (close) => _menuFor([fx], close,
            changed: changed, resetParam: id, resetFx: fx),
      );
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _heading(_Section.basics, l10n.gradeBasics),
        row(GradeFx.exposure, 'stops', decimals: 2, signed: true),
        row(GradeFx.contrast, 'contrast'),
        row(GradeFx.saturation, 'saturation'),
        row(GradeFx.temperature, 'temperature', signed: true),
        row(GradeFx.temperature, 'tint', signed: true),
      ],
    );
  }

  Widget _curves(LumitTheme t, bool enabled) {
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
              changed: _curveChanged(id), resetParam: id, resetFx: fx),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: gradeInset),
            child: LayoutBuilder(builder: (context, box) {
              final column = channelColumnWidth(labels) + channelColumnGap;
              final plot =
                  (box.maxWidth - column).clamp(48.0, 180.0).toDouble();
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
                          _write(fx, id, curveValue(p));
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
                          const BridgeEffectValue.file(BridgeFileParam(
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
            onCancel: _touchEnd,
            menu: (close) => _menuFor([fx], close,
                changed: _floatChanged(fx, 'mix'),
                resetParam: 'mix',
                resetFx: fx),
          ),
      ],
    );
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
        BridgeEffectValue.file(BridgeFileParam(
            paths: [path], index: const BridgeScalar.static_(0))));
  }
}
