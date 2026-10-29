// The Cut timeline's Pan/Crop window, as Vegas has it: the clip's own
// picture with a box over it that marks the part of the picture the clip
// shows. Drag inside the box to pan, a handle to zoom, the knob above it to
// turn. Each gesture is written once, on release, as one undo step.
//
// The box is the clip's own Transform effect. With W by H the clip's picture
// in source pixels, a box centred on c, W/sx wide, H/sy high and turned by a
// writes anchor c, position (W/2, H/2), scale sx and sy in per cent and
// rotation -a, which fills the frame with what the box holds. The box drawn
// is always the true outline of what fills the frame, so with unequal scales
// and a turn it leans. The Crop it adds is left to Effect controls.
//
// A keyed value is written at the playhead, as Effect controls writes it. A
// value an expression drives is shown and not edited.

import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart' show UuidValue;

import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/comp_time.dart';
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'layer_fold_frb.dart' show asOneUndoStep;

/// Give [clip] a Crop and a Transform where it has none, as one undo step,
/// then open the window on it.
Future<void> showCutPanCrop(
  BuildContext context, {
  required CompositionReference comp,
  required LayerReference layer,
  required BridgeClip clip,
  required VoidCallback onChanged,
}) async {
  final project = Provider.of<LumitState>(context, listen: false).project;
  final frame =
      Provider.of<LumitUiState>(context, listen: false).playheadFrame.value;
  final missing = [
    for (final name in const ['crop', 'transform'])
      if (!clip.effects.any((e) => e.name == name)) name,
  ];
  if (missing.isNotEmpty) {
    asOneUndoStep(project, () {
      for (final name in missing) {
        try {
          layer.addClipEffect(clip: clip.id, name: name);
        } catch (_) {}
      }
    });
    onChanged();
  }
  final size = await _pictureSize(project, comp, clip);
  final still = await _still(layer, comp, clip, frame);
  if (!context.mounted) {
    still?.dispose();
    return;
  }
  await showLumitModal<void>(
    context: context,
    builder: (close) => _PanCrop(
      comp: comp,
      layer: layer,
      clip: clip.id,
      frame: frame,
      size: size,
      still: still,
      onChanged: onChanged,
      onClose: () => close(null),
    ),
  );
}

/// How big the clip's picture is in its own pixels, which is the raster its
/// effects run on. Found by the clip's source name, and the comp's size when
/// nothing answers.
///
/// ponytail: a name lookup, so two items of one name give the first. The
/// upgrade is the clip's source item on BridgeClip.
Future<Size> _pictureSize(ProjectReference? project, CompositionReference comp,
    BridgeClip clip) async {
  final own = comp.getSize();
  final fallback = Size(own.width.toDouble(), own.height.toDouble());
  if (project == null) return fallback;
  Iterable<ItemReference> walk(List<ItemReference> items) sync* {
    for (final item in items) {
      yield item;
      if (item is ItemReference_Folder) yield* walk(item.field0.getChildren());
    }
  }

  try {
    for (final item in walk(project.getItems())) {
      if (item.name() != clip.sourceName) continue;
      switch (item) {
        case ItemReference_Footage(:final field0) when !clip.sourceIsComp:
          final info = await field0.mediaInfo();
          if (info != null && info.width > 0 && info.height > 0) {
            return Size(info.width.toDouble(), info.height.toDouble());
          }
        case ItemReference_Composition(:final field0) when clip.sourceIsComp:
          final s = field0.getSize();
          return Size(s.width.toDouble(), s.height.toDouble());
        default:
      }
    }
  } catch (_) {}
  return fallback;
}

/// The clip's picture at the playhead, or at its first frame when the
/// playhead is elsewhere. Null for a comp clip, which has nothing to decode.
Future<ui.Image?> _still(LayerReference layer, CompositionReference comp,
    BridgeClip clip, int frame) async {
  final start = clip.startFrame.toInt();
  double? along;
  if (frame > start && frame < clip.endFrame.toInt()) {
    final at = timeOfFrame(comp, frame);
    final from = timeOfFrame(comp, start);
    along =
        at.num.toInt() / at.den.toInt() - from.num.toInt() / from.den.toInt();
  }
  try {
    final still =
        await layer.clipThumbnail(clip: clip.id, maxEdge: 960, along: along);
    if (still == null || still.width == 0) return null;
    final done = Completer<ui.Image>();
    ui.decodeImageFromPixels(still.rgba, still.width, still.height,
        ui.PixelFormat.rgba8888, done.complete);
    return await done.future;
  } catch (_) {
    return null;
  }
}

/// The box: its centre in source pixels, and the Transform's scale as
/// fractions and its rotation in degrees.
typedef _Fit = ({double cx, double cy, double sx, double sy, double rot});

/// The Transform rows the box writes.
const _rows = [
  'anchor_x',
  'anchor_y',
  'position_x',
  'position_y',
  'scale_x',
  'scale_y',
  'rotation',
];

/// What a drag took hold of.
enum _Grab { pan, scale, turn }

/// Where the source sits in the view: its top-left and pixels per source
/// pixel. A quarter of the picture's size is left round it, so a box zoomed
/// out past the edge can still be seen and caught.
({Offset at, double k}) _place(Size view, Size size) {
  final k = math.min(
      view.width / (size.width * 1.5), view.height / (size.height * 1.5));
  return (
    at: Offset(
        (view.width - size.width * k) / 2, (view.height - size.height * k) / 2),
    k: k,
  );
}

/// The source point the frame's offset [h] from its centre shows.
Offset _back(_Fit f, Offset h) {
  final r = f.rot * math.pi / 180;
  final (s, c) = (math.sin(r), math.cos(r));
  return Offset(f.cx + (c * h.dx + s * h.dy) / f.sx,
      f.cy + (-s * h.dx + c * h.dy) / f.sy);
}

class _PanCrop extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final UuidValue clip;
  final int frame;
  final Size size;
  final ui.Image? still;
  final VoidCallback onChanged;
  final VoidCallback onClose;

  const _PanCrop({
    required this.comp,
    required this.layer,
    required this.clip,
    required this.frame,
    required this.size,
    required this.still,
    required this.onChanged,
    required this.onClose,
  });

  @override
  State<_PanCrop> createState() => _PanCropState();
}

class _PanCropState extends State<_PanCrop> {
  static const _view = Size(640, 380);

  late _Fit _fit;

  /// Off when there is no Transform to write, or an expression drives it.
  bool _live = false;
  bool _lock = true;

  /// The drag in hand: what it caught, which handle, the box and the source
  /// point it started from.
  _Grab? _grab;
  (int, int) _handle = (0, 0);
  _Fit? _from;
  Offset _down = Offset.zero;
  MouseCursor _cursor = MouseCursor.defer;

  double get _w => widget.size.width;
  double get _h => widget.size.height;

  @override
  void initState() {
    super.initState();
    _fit = (cx: _w / 2, cy: _h / 2, sx: 1.0, sy: 1.0, rot: 0.0);
    try {
      final fx = _transform(
          widget.layer.getClipEffects(clip: widget.clip, onCompClock: true));
      if (fx == null) return;
      final s = {for (final id in _rows) id: _scalar(fx, id)};
      if (s.values.any((v) => v == null || v is BridgeScalar_Expression)) {
        return;
      }
      double v(String id) => _at(s[id]!);
      final sx = _nonZero(v('scale_x') / 100);
      final sy = _nonZero(v('scale_y') / 100);
      // The anchor and position as stored, put back as a centre: the source
      // point the middle of the frame shows.
      final c = _back((
        cx: v('anchor_x'),
        cy: v('anchor_y'),
        sx: sx,
        sy: sy,
        rot: v('rotation')
      ), Offset(_w / 2 - v('position_x'), _h / 2 - v('position_y')));
      _fit = (cx: c.dx, cy: c.dy, sx: sx, sy: sy, rot: v('rotation'));
      _live = true;
    } catch (_) {
      // The clip went away; the window shows the picture and edits nothing.
    }
  }

  @override
  void dispose() {
    widget.still?.dispose();
    super.dispose();
  }

  static double _nonZero(double v) =>
      v.abs() < 1e-4 ? (v.isNegative ? -1e-4 : 1e-4) : v;

  static BridgeEffectInstance? _transform(List<BridgeEffectInstance> stack) {
    for (final fx in stack) {
      if (fx.name() == 'transform') return fx;
    }
    return null;
  }

  static BridgeScalar? _scalar(BridgeEffectInstance fx, String id) =>
      switch (fx.getValue(id: id)) {
        BridgeEffectValue_Float(:final field0) => field0,
        _ => null,
      };

  double _at(BridgeScalar s) => s is BridgeScalar_Static
      ? s.field0
      : sampledScalar(s, timeOfFrame(widget.comp, widget.frame));

  /// Write the box to the Transform as one op, touching only the rows that
  /// move.
  void _write() {
    if (!_live) return;
    final f = _fit;
    final want = {
      'anchor_x': f.cx,
      'anchor_y': f.cy,
      'position_x': _w / 2,
      'position_y': _h / 2,
      'scale_x': f.sx * 100,
      'scale_y': f.sy * 100,
      'rotation': f.rot,
    };
    try {
      final stack =
          widget.layer.getClipEffects(clip: widget.clip, onCompClock: true);
      final fx = _transform(stack);
      if (fx == null) return;
      var moved = false;
      for (final MapEntry(:key, :value) in want.entries) {
        final was = _scalar(fx, key);
        if (was == null || (_at(was) - value).abs() < 1e-4) continue;
        fx.setValue(
            id: key,
            value: BridgeEffectValue.float(
                scalarWithValueAt(was, value, widget.comp, widget.frame)));
        moved = true;
      }
      if (!moved) return;
      widget.layer.setEffects(effects: stack);
    } catch (_) {
      // A locked layer refuses, and quietly.
      return;
    }
    widget.onChanged();
  }

  void _set(_Fit f, {bool write = false}) {
    setState(() => _fit = f);
    if (write) _write();
  }

  // ------------------------------------------------------------- the view

  Offset _toView(Offset p) {
    final at = _place(_view, widget.size);
    return at.at + p * at.k;
  }

  Offset _toSource(Offset v) {
    final at = _place(_view, widget.size);
    return (v - at.at) / at.k;
  }

  List<(int, int)> get _handles => [
        for (final hx in const [-1, 0, 1])
          for (final hy in const [-1, 0, 1])
            if (hx != 0 || hy != 0) (hx, hy),
      ];

  Offset _handleAt(_Fit f, (int, int) h) =>
      _back(f, Offset(h.$1 * _w / 2, h.$2 * _h / 2));

  /// The turning knob, in view pixels: a little way out past the top edge.
  Offset _knob(_Fit f) {
    final top = _toView(_back(f, Offset(0, -_h / 2)));
    final out = top - _toView(Offset(f.cx, f.cy));
    final d = out.distance;
    return top + (d < 1e-3 ? const Offset(0, -1) : out / d) * 18;
  }

  Path _outline(_Fit f) => Path()
    ..addPolygon([
      for (final h in const [(-1, -1), (1, -1), (1, 1), (-1, 1)])
        _toView(_handleAt(f, h)),
    ], true);

  ({_Grab grab, (int, int) handle})? _hit(Offset v) {
    if ((_knob(_fit) - v).distance <= 7) {
      return (grab: _Grab.turn, handle: (0, 0));
    }
    for (final h in _handles) {
      if ((_toView(_handleAt(_fit, h)) - v).distance <= 7) {
        return (grab: _Grab.scale, handle: h);
      }
    }
    if (_outline(_fit).contains(v)) return (grab: _Grab.pan, handle: (0, 0));
    return null;
  }

  void _start(Offset v) {
    final hit = _hit(v);
    _grab = hit?.grab;
    _handle = hit?.handle ?? (0, 0);
    _from = _fit;
    _down = _toSource(v);
  }

  void _move(Offset v) {
    final from = _from;
    final grab = _grab;
    if (from == null || grab == null) return;
    final p = _toSource(v);
    final centre = Offset(from.cx, from.cy);
    final d0 = _down - centre;
    final d = p - centre;
    switch (grab) {
      case _Grab.pan:
        final by = p - _down;
        _set((
          cx: from.cx + by.dx,
          cy: from.cy + by.dy,
          sx: from.sx,
          sy: from.sy,
          rot: from.rot,
        ));
      case _Grab.scale:
        // About the centre, as Vegas scales. Unlocked, each axis follows the
        // pointer along the source's own axes, which is the axis the
        // Transform scales along.
        double by(double now, double was, bool held) =>
            held && was.abs() > 1e-3 ? (now / was).abs() : 1;
        final k = d.distance / math.max(d0.distance, 1e-3);
        final (kx, ky) = _lock
            ? (k, k)
            : (
                by(d.dx, d0.dx, _handle.$1 != 0),
                by(d.dy, d0.dy, _handle.$2 != 0),
              );
        _set((
          cx: from.cx,
          cy: from.cy,
          sx: from.sx / math.max(kx, 1e-3),
          sy: from.sy / math.max(ky, 1e-3),
          rot: from.rot,
        ));
      case _Grab.turn:
        final turned =
            (math.atan2(d.dy, d.dx) - math.atan2(d0.dy, d0.dx)) * 180 / math.pi;
        _set((
          cx: from.cx,
          cy: from.cy,
          sx: from.sx,
          sy: from.sy,
          rot: from.rot - turned,
        ));
    }
  }

  void _end() {
    final moved = _grab != null && _from != _fit;
    _grab = null;
    _from = null;
    if (moved) _write();
  }

  void _hover(Offset v) {
    final next = switch (_hit(v)?.grab) {
      _Grab.pan => SystemMouseCursors.move,
      _Grab.scale => SystemMouseCursors.precise,
      _Grab.turn => SystemMouseCursors.grab,
      null => MouseCursor.defer,
    };
    if (next != _cursor) setState(() => _cursor = next);
  }

  // ------------------------------------------------------------ the fields

  void _width(num w, bool write) {
    final sx = _w / math.max(1, w.toDouble());
    final sy = _lock ? _fit.sy * sx / _fit.sx : _fit.sy;
    _set((cx: _fit.cx, cy: _fit.cy, sx: sx, sy: sy, rot: _fit.rot),
        write: write);
  }

  void _height(num h, bool write) {
    final sy = _h / math.max(1, h.toDouble());
    final sx = _lock ? _fit.sx * sy / _fit.sy : _fit.sx;
    _set((cx: _fit.cx, cy: _fit.cy, sx: sx, sy: sy, rot: _fit.rot),
        write: write);
  }

  Widget _field(LumitTheme t, String label, num value, String suffix,
          void Function(num v, bool write) set,
          {num min = -100000, int decimals = 1}) =>
      Expanded(
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 3),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(label, style: t.small.copyWith(color: t.textMuted)),
              const SizedBox(height: 2),
              DragValueField(
                value: value,
                min: min,
                max: 100000,
                decimals: decimals,
                suffix: suffix,
                onChangeLive: (v) => set(v, false),
                onChangeEnd: (v) => set(v, true),
                onChanged: (v) => set(v, true),
              ),
            ],
          ),
        ),
      );

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final f = _fit;
    final px = l10n.unitSymbolPx;
    final canvas = MouseRegion(
      cursor: _cursor,
      onHover: (e) => _hover(e.localPosition),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onPanStart: (d) => _start(d.localPosition),
        onPanUpdate: (d) => _move(d.localPosition),
        onPanEnd: (_) => _end(),
        onPanCancel: () {
          final from = _from;
          _grab = null;
          _from = null;
          if (from != null) _set(from);
        },
        child: ClipRect(
          child: CustomPaint(
            size: _view,
            painter: _BoxPainter(
              theme: t,
              still: widget.still,
              source: Rect.fromPoints(
                  _toView(Offset.zero), _toView(Offset(_w, _h))),
              outline: _outline(f),
              handles: [for (final h in _handles) _toView(_handleAt(f, h))],
              top: _toView(_back(f, Offset(0, -_h / 2))),
              knob: _knob(f),
            ),
          ),
        ),
      ),
    );
    final fields = Row(children: [
      _field(
          t,
          l10n.cutPanCropX,
          f.cx,
          px,
          (v, w) => _set(
              (cx: v.toDouble(), cy: f.cy, sx: f.sx, sy: f.sy, rot: f.rot),
              write: w)),
      _field(
          t,
          l10n.cutPanCropY,
          f.cy,
          px,
          (v, w) => _set(
              (cx: f.cx, cy: v.toDouble(), sx: f.sx, sy: f.sy, rot: f.rot),
              write: w)),
      _field(t, l10n.cutPanCropWidth, _w / f.sx, px, _width, min: 1),
      _field(t, l10n.cutPanCropHeight, _h / f.sy, px, _height, min: 1),
      _field(
          t,
          l10n.cutPanCropRotation,
          -f.rot,
          l10n.unitSymbolDegrees,
          (v, w) => _set(
              (cx: f.cx, cy: f.cy, sx: f.sx, sy: f.sy, rot: -v.toDouble()),
              write: w)),
    ]);
    return FloatSurface(
      width: _view.width + 32,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.only(bottom: 10),
            child: Text(l10n.cutPanCropTitle, style: t.bodyPrimary),
          ),
          IgnorePointer(
            ignoring: !_live,
            child: Opacity(
              opacity: _live ? 1 : 0.5,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Center(child: canvas),
                  const SizedBox(height: 10),
                  fields,
                ],
              ),
            ),
          ),
          const SizedBox(height: 14),
          Row(
            children: [
              HouseCheckbox(
                value: _lock,
                label: l10n.cutPanCropLockAspect,
                onChanged: _live ? (v) => setState(() => _lock = v) : null,
              ),
              const Spacer(),
              HouseButton(
                onPressed: _live
                    ? () => _set((cx: _w / 2, cy: _h / 2, sx: 1, sy: 1, rot: 0),
                        write: true)
                    : null,
                child: Text(l10n.cutPanCropFill),
              ),
              const SizedBox(width: 8),
              HouseButton(
                primary: true,
                autofocus: true,
                onPressed: widget.onClose,
                child: Text(l10n.close),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

/// The picture, the frame round it, and the box with its handles and knob.
class _BoxPainter extends CustomPainter {
  final LumitTheme theme;
  final ui.Image? still;
  final Rect source;
  final Path outline;
  final List<Offset> handles;
  final Offset top;
  final Offset knob;

  _BoxPainter({
    required this.theme,
    required this.still,
    required this.source,
    required this.outline,
    required this.handles,
    required this.top,
    required this.knob,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final t = theme;
    canvas.drawRect(Offset.zero & size, Paint()..color = t.viewerSurround);
    final image = still;
    if (image == null) {
      canvas.drawRect(source, Paint()..color = t.surface2);
    } else {
      canvas.drawImageRect(
        image,
        Rect.fromLTWH(0, 0, image.width.toDouble(), image.height.toDouble()),
        source,
        Paint()..filterQuality = FilterQuality.medium,
      );
    }
    canvas.drawRect(
        source,
        Paint()
          ..style = PaintingStyle.stroke
          ..color = t.hairlineStrong);
    final line = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5
      ..color = t.accent;
    canvas.drawPath(outline, line);
    canvas.drawLine(top, knob, line);
    final fill = Paint()..color = t.accent;
    canvas.drawCircle(knob, 4, fill);
    for (final h in handles) {
      canvas.drawRect(Rect.fromCenter(center: h, width: 7, height: 7), fill);
    }
  }

  @override
  bool shouldRepaint(_BoxPainter old) =>
      old.outline != outline || old.still != still || old.theme != theme;
}
